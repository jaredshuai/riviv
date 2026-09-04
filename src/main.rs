//! riviv — an unofficial Rust rewrite of voidtools/voidImageViewer (MIT).
//!
//! M2: Win32 window + GDI rendering + animated GIF/WebP playback with
//! transparent pixels composited over the windowed background, decoded on
//! a background thread so the window never freezes on large files.
//!
//! Behavior baseline is the upstream C source under `c-original/src/viv.c`
//! (see c-original/PROVENANCE.md). Key alignments across the modules:
//! - never upscale (`fill_window = 0` default, `_viv_get_render_size` clamp)
//! - load failure before the first frame keeps the old display / an empty
//!   window — never a popup, never an exit; a failure after the partial
//!   image reached the screen clears it (upstream `_viv_load_failed`)
//! - window title is `filename - riviv` (`_viv_update_title` format)
//! - drag & drop of a single file replaces the current image, window size is NOT reset
//!   (`WM_DROPFILES` handler); multi-file drop = M2 playlist, we take the first file
//! - no-arg start = empty window, Ctrl+O opens the file dialog (upstream default keymap)
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
//! - `fit` — fit-to-window math; M2: zoom presets / pan clamp (#7)
//! - `text` — title & wide-string construction; M2: status-bar text (#5)
//! - `surface` — DIB section + memory DC; one per decoded frame (#3, landed);
//!   M2: mipmap surfaces (#9)
//! - `loader` — streaming decode pipeline + the load reply state machine
//!   (#3/#4, landed)
//! - `loadthread` — background decode session: worker thread, reply queue,
//!   kick message (#4, landed)
//! - `paint` — WM_PAINT render; M2: stitch/mip (#9), zoom (#7)
//! - `window` — wnd_proc shell, pump, input/open actions, animation timer,
//!   reply handler (#3/#4, landed); M2: fullscreen (#8), playlist wiring (#6)

#![windows_subsystem = "windows"]

mod anim;
mod fit;
mod loader;
mod loadthread;
mod paint;
mod pixels;
mod surface;
mod text;
mod window;

use crate::window::run;

fn main() {
    let arg_path = std::env::args_os().nth(1);
    if let Err(err) = run(arg_path) {
        window::fatal(&format!("riviv: {err}"));
    }
}
