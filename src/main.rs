//! riviv — an unofficial Rust rewrite of voidtools/voidImageViewer (MIT).
//!
//! M2: Win32 window + GDI rendering + animated GIF/WebP playback with
//! transparent pixels composited over the windowed background, decoded on
//! a background thread so the window never freezes on large files, plus a
//! keyboard-navigable playlist built from multi-file/folder drops and the
//! command line.
//!
//! Behavior baseline is the upstream C source under `c-original/src/viv.c`
//! (see c-original/PROVENANCE.md). Key alignments across the modules:
//! - never upscale (`fill_window = 0` default, `_viv_get_render_size` clamp)
//! - load failure before the first frame keeps the old display / an empty
//!   window — never a popup, never an exit; a failure after the partial
//!   image reached the screen clears it (upstream `_viv_load_failed`)
//! - window title is `filename - riviv` (`_viv_update_title` format)
//! - drag & drop: one file replaces the current image (window size is NOT
//!   reset); multiple files, a folder, or Shift-drops build the playlist
//!   (`WM_DROPFILES`, viv.c:3076-3128)
//! - navigation Right/PgDn/Left/PgUp/Home/End walks the playlist by the
//!   default upstream sort (date-modified descending, name ascending,
//!   viv.c:5623-5815 + config.c:43-44), falling back to scanning the
//!   current file's folder when no playlist exists (`_viv_next`/`_viv_home`)
//! - command line: one argument opens (folder/wildcard included), several
//!   arguments build the playlist in argument order (viv.c:4990-5100)
//! - no-arg start = empty window, Ctrl+O opens the file dialog (upstream
//!   default keymap)
//! - animations play on a USER_TIMER_MINIMUM timer driven by performance-counter
//!   accumulation (WM_TIMER catch-up/wrap semantics, viv.c:3171-3292); frame
//!   dispose/compositing is the decoder's job, like upstream's GDI+/libwebp frames
//! - decoding runs on a background thread feeding a reply queue
//!   (upstream `_viv_load_image_thread_proc` + `_viv_reply_add`, viv.c:10331/10869):
//!   the display swaps at the first frame, later frames stream in while playing,
//!   and playback waits at the loaded prefix until they arrive (viv.c:3233-3240)
//!
//! Module layout (each module is an M2 seam):
//! - `anim` — animation frame scheduling + delay fallbacks + the streamed-loading
//!   stall branch (#3/#4, landed); M2 seams left: the rate table
//! - `pixels` — BGRA conversion + alpha compositing (#3, landed); M2: mip math (#9)
//! - `fit` — fit-to-window math (the zoom curve's level 0)
//! - `zoom` — 16-step zoom presets, wheel/cursor anchoring, pan clamping,
//!   the temporary 1:1 mode, the resize re-anchor and the fullscreen
//!   toggle's zoom-offset math (#7/#8, landed)
//! - `text` — title & wide-string construction + the status-bar text and
//!   part-width model (#5, landed)
//! - `cursor` — the fullscreen cursor-hide state machine, pure
//!   show/timer/effects decisions (#8, landed)
//! - `surface` — DIB section + memory DC; one per decoded frame (#3, landed);
//!   M2: mipmap surfaces (#9)
//! - `loader` — streaming decode pipeline + the load reply state machine
//!   (#3/#4, landed)
//! - `loadthread` — background decode session: worker thread, reply queue,
//!   kick message (#4, landed)
//! - `paint` — WM_PAINT render; M2: stitch/mip (#9); zoom/pan offsets,
//!   the BitBlt 1:1 path and the COLORONCOLOR magnify filter landed (#7)
//! - `playlist` — playlist model + navigation math + recursive folder/wildcard
//!   entry construction (#6, landed)
//! - `status` — the status-bar common control: creation, height, and the
//!   measure → parts → texts update over `text`'s pure model (#5, landed)
//! - `window` — wnd_proc shell, pump, input/open actions, animation timer,
//!   reply handler, playlist wiring, zoom/pan mouse+key wiring, the
//!   fullscreen toggle and the cursor-hide timers
//!   (#3/#4/#5/#6/#7/#8, landed); M2: stitch/mip (#9)

#![windows_subsystem = "windows"]

mod anim;
mod cursor;
mod fit;
mod loader;
mod loadthread;
mod paint;
mod pixels;
mod playlist;
mod status;
mod surface;
mod text;
mod window;
mod zoom;

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::absolute;

use crate::window::run;

/// Whether a command-line word is a switch, not a file (upstream
/// viv.c:4825: an UNQUOTED '/'- or '-'-prefixed word — where
/// `string_is_dot` (string.c:856-871) means "contains a '.' anywhere", so
/// `-foo.png` is a FILE. `args_os` cannot see the original quoting, so
/// quoted switches are skipped too; noted in README Differences).
fn is_switch(arg: &OsStr) -> bool {
    let wide = arg.encode_wide().collect::<Vec<u16>>();
    match wide.first() {
        Some(&c) if c == u16::from(b'/') || c == u16::from(b'-') => {
            !wide.contains(&u16::from(b'.'))
        }
        _ => false,
    }
}

fn main() {
    // The command line's file words: switches dropped, empties dropped
    // (upstream skips blank words, viv.c:4993), the rest made absolute —
    // upstream cwd-combines relative paths the same way (string_path_combine).
    let args: Vec<OsString> = std::env::args_os()
        .skip(1)
        .filter(|a| !a.is_empty() && !is_switch(a))
        .map(|a| match absolute(&a) {
            Ok(p) => p.into_os_string(),
            Err(_) => a,
        })
        .collect();
    if let Err(err) = run(args) {
        window::fatal(&format!("riviv: {err}"));
    }
}
