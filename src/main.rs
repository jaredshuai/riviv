//! riviv — an unofficial Rust rewrite of voidtools/voidImageViewer (MIT).
//!
//! M2: Win32 window + GDI rendering + animated GIF/WebP playback with
//! transparent pixels composited over the windowed background.
//!
//! Behavior baseline is the upstream C source under `c-original/src/viv.c`
//! (see c-original/PROVENANCE.md). Key alignments across the modules:
//! - never upscale (`fill_window = 0` default, `_viv_get_render_size` clamp)
//! - load failure keeps the old image / opens an empty window — never a popup, never an exit
//! - window title is `filename - riviv` (`_viv_update_title` format)
//! - drag & drop of a single file replaces the current image, window size is NOT reset
//!   (`WM_DROPFILES` handler); multi-file drop = M2 playlist, we take the first file
//! - no-arg start = empty window, Ctrl+O opens the file dialog (upstream default keymap)
//! - animations play on a USER_TIMER_MINIMUM timer driven by performance-counter
//!   accumulation (WM_TIMER catch-up/wrap semantics, viv.c:3171-3292); frame
//!   dispose/compositing is the decoder's job, like upstream's GDI+/libwebp frames
//!
//! Module layout (each module is an M2 seam):
//! - `anim` — animation frame scheduling + delay fallbacks (#3, landed);
//!   M2 seams left: the streamed-loading branch and rate table (#4)
//! - `pixels` — BGRA conversion + alpha compositing (#3, landed); M2: mip math (#9)
//! - `fit` — fit-to-window math; M2: zoom presets / pan clamp (#7)
//! - `text` — title & wide-string construction; M2: status-bar text (#5)
//! - `surface` — DIB section + memory DC; one per decoded frame (#3, landed);
//!   M2: mipmap surfaces (#9)
//! - `loader` — decode pipeline incl. animated GIF/WebP frames (#3, landed);
//!   M2: background thread + reply protocol (#4)
//! - `paint` — WM_PAINT render; M2: stitch/mip (#9), zoom (#7)
//! - `window` — wnd_proc shell, pump, input/open actions, animation timer
//!   (#3, landed); M2: fullscreen (#8), playlist wiring (#6)

#![windows_subsystem = "windows"]

mod anim;
mod fit;
mod loader;
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
