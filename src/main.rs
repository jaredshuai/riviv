//! riviv — an unofficial Rust rewrite of voidtools/voidImageViewer (MIT).
//!
//! M3: the M2 feature set (Win32 window + D2D viewport rendering over a
//! GDI chrome, animated
//! GIF/WebP playback, background decoding, playlist navigation, zoom/pan,
//! fullscreen, giant-image handling — the D2D tiling of #82, after the
//! StretchBlt stitching of #9/#81) plus the settings layer
//! (#19): the window rect is remembered across runs in a `[riviv]` ini
//! (`config.rs`/`ini.rs`) with upstream's 60% first-run auto-fit, and the
//! M1 image-sized startup window is gone — windows never resize on load
//! (upstream `auto_zoom = 0`). A second launch hands its command line to
//! the existing window and exits (#21, `copydata.rs`), unless
//! `multiple_instances` is set.
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
//! - `pixels` — BGRA conversion + alpha compositing (#3, landed); the CPU
//!   frame (`PixelFrame`, #76)
//! - `fit` — fit-to-window math (the zoom curve's level 0)
//! - `zoom` — 16-step zoom presets, wheel/cursor anchoring, pan clamping,
//!   the temporary 1:1 mode, the resize re-anchor and the fullscreen
//!   toggle's zoom-offset math (#7/#8, landed)
//! - `text` — title & wide-string construction + the status-bar text and
//!   part-width model (#5, landed)
//! - `cursor` — the fullscreen cursor-hide state machine, pure
//!   show/timer/effects decisions (#8, landed)
//! - `surface` — the UI-thread CPU master holder (`PixelFrame` wrap; #3/#76,
//!   #90 reduced: the GDI face derivation is gone)
//! - `loader` — streaming decode pipeline + the load reply state machine
//!   (#3/#4, landed)
//! - `loadthread` — background decode session: worker thread, reply queue,
//!   kick message (#4, landed)
//! - `loc` — bilingual string tables + one-shot system-language detection
//!   (#20, landed)
//! - `copydata` — the single-instance handoff payload codec (#21, landed)
//! - `assoc` — file associations, the start-menu shortcuts and the
//!   install-family command line (`/install`-family switches, run before
//!   the window exists; #26)
//! - `keys` — per-command keyboard bindings: the default table, the ini
//!   `*_keys` codec, the lookup router and the Controls-page name model
//!   (#25)
//! - `menu` — the menu command table: WM_COMMAND ids,
//!   accelerator labels and the WM_INITMENU check/enable model (#23)
//! - `slideshow` — the slideshow rate model: presets, stepping, custom
//!   compose, the WM_TIMER advance gate and the rate readout (#37)
//! - `custom_rate_dlg` — the Set Custom Rate dialog over that model
//!   (hand-built, the #22 dialog pattern) (#37)
//! - `paint` — the shared view math ([`crate::paint::scene_rect`], the D2D
//!   arm's geometry source) + the dump channel's PNG tail
//! - `gpu` — the D2D/DXGI viewport stack (#80): GpuStack (device chain,
//!   frame upload keyed by frame_gen, WM_SIZE resize, the three-tier
//!   failure ladder, the WM_CLOSE dump readback) behind the `renderer`
//!   ini key; #81: the full filter table, and `auto` is the default;
//!   #90: the GDI render arm is gone — hardware D2D → WARP → fatal
//! - `playlist` — playlist model + navigation math + recursive folder/wildcard
//!   entry construction (#6, landed)
//! - `status` — the status-bar common control: creation, height, and the
//!   measure → parts → texts update over `text`'s pure model (#5, landed)
//! - `window` — wnd_proc shell, pump, input/open actions, animation timer,
//!   reply handler, playlist wiring, zoom/pan mouse+key wiring, the
//!   fullscreen toggle and the cursor-hide timers
//!   (#3/#4/#5/#6/#7/#8, landed); M2: the giant-image gates (#9, #81 shape)

#![windows_subsystem = "windows"]

mod anim;
mod assoc;
mod cli;
mod clipboard;
mod config;
mod copydata;
mod cursor;
mod custom_rate_dlg;
mod dib;
mod everything;
mod filemgmt;
mod fit;
mod frame;
mod gpu;
mod icm;
mod ini;
mod jumpto_dlg;
mod keys;
mod loader;
mod loadthread;
mod loc;
mod menu;
mod mip;
mod options;
mod options_dlg;
mod paint;
mod panscan;
mod pixels;
mod playlist;
mod preload;
mod rename_dlg;
mod shell;
mod slideshow;
mod status;
mod surface;
mod text;
mod tile;
mod toolbar;
#[allow(dead_code)]
// #127 ships the decision table + output-identity keys; consumers land with #126 and the static-gpu_effect phase
mod transform_stage;
mod window;
mod zoom;

use crate::window::run;

fn main() {
    // The whole command line — install pass, then the second pass's
    // config switches and file words — is read raw inside `run` (#48:
    // the second pass needs the quoting `args_os` cannot see).
    if let Err(err) = run() {
        window::fatal(&format!("riviv: {err}"));
    }
}
