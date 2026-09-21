//! The shared view math + the dump channel's PNG tail.
//!
//! #90 retired the GDI render mainline this module used to hold (the
//! WM_PAINT blit bracket, the letterbox strips, the shrink regimes and
//! the GDI dump arm): the viewport paints through the D2D stack
//! ([`crate::gpu`]). What remains is the geometry both the D2D arm and
//! every future consumer share — [`scene_rect`], the pure view-math
//! (upstream `_viv_get_render_size` + the panscan layer) that turns a
//! [`crate::zoom::View`] into the on-screen image rect — and
//! [`save_rgba_png`], the write tail of the `-dump-viewport`
//! automation channel.

use std::path::Path;

use crate::zoom::{FitPolicy, View, Viewport};

/// The shared view math (upstream `_viv_get_render_size` + the panscan
/// layer, viv.c:4136-4154): the image rect in VIEWPORT-RELATIVE client
/// coordinates for a `sw x sh` source. Pure — the D2D arm ([`crate::gpu`])
/// consumes these numbers as the draw plan's scene rect; dx/dy/rw/rh stay
/// exact i32s (the 1:1 five-piece's integer-rect clause). The caller adds
/// the client origin, which is (0,0) for the viewport child.
pub(crate) fn scene_rect(
    view: &View,
    fit: FitPolicy,
    cw: i32,
    ch: i32,
    sw: i32,
    sh: i32,
) -> (i32, i32, i32, i32) {
    let (rw, rh) = view.render_size(sw, sh, Viewport { wide: cw, high: ch }, fit);
    let panscan = view.panscan;
    let rw = crate::panscan::scale(rw, panscan.zoom_x);
    let rh = crate::panscan::scale(rh, panscan.zoom_y);
    let dx = crate::panscan::center_term(cw, panscan.pos_x) - rw / 2 - view.view_x;
    let dy = crate::panscan::center_term(ch, panscan.pos_y) - rh / 2 - view.view_y;
    (dx, dy, rw, rh)
}

/// Write RGBA pixels as a PNG (#80's dump tail; the image crate is already
/// a decode dependency, its png feature on). Failures are plain strings —
/// the WM_CLOSE dump path reports on stderr and exits 2 without a modal.
pub(crate) fn save_rgba_png(
    path: &Path,
    wide: u32,
    high: u32,
    rgba: Vec<u8>,
) -> Result<(), String> {
    let image = image::RgbaImage::from_raw(wide, high, rgba).ok_or("dump buffer size mismatch")?;
    image
        .save(path)
        .map_err(|e| format!("png save failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- scene_rect (#80: the shared view math) ----

    fn one_to_one_view() -> View {
        // The pixel-exact mode (the D2D arm's L0 case): render_size returns
        // the source verbatim while `one_to_one` is on (zoom.rs's own
        // pinned behavior).
        let mut view = View::new();
        view.toggle_one_to_one(
            300,
            300,
            Viewport {
                wide: 1000,
                high: 700,
            },
            FitPolicy::WITHOUT_FILL,
        );
        view
    }

    #[test]
    fn scene_rect_centers_the_render_with_the_default_panscan() {
        // Default panscan (pos 500 = plain center, zoom factor 1.0) and a
        // 300x300 source in 1000x700: dx = 500 - 150 = 350, dy = 350 - 150
        // = 200 — the plain centered rect both the old GDI arm and the D2D
        // arm draw.
        let view = one_to_one_view();
        assert_eq!(
            scene_rect(&view, FitPolicy::WITHOUT_FILL, 1000, 700, 300, 300),
            (350, 200, 300, 300)
        );
    }

    #[test]
    fn scene_rect_subtracts_the_pan_offset() {
        // The drag offset moves the rect opposite to the drag, one-to-one.
        let mut view = one_to_one_view();
        view.view_x = 40;
        view.view_y = 15;
        assert_eq!(
            scene_rect(&view, FitPolicy::WITHOUT_FILL, 1000, 700, 300, 300),
            (310, 185, 300, 300)
        );
    }

    #[test]
    fn scene_rect_follows_the_panscan_center_term() {
        // Pan position 750 (past the default 500 center) moves the center
        // term to (750-250)*2000/1000 = 1000, so the rect shifts right by
        // 500 — the same term the render always adds (viv.c:4153).
        let mut view = one_to_one_view();
        view.panscan.pos_x = 750;
        // center_term(1000, 750) = (750-250)*2000/1000 = 1000.
        assert_eq!(
            scene_rect(&view, FitPolicy::WITHOUT_FILL, 1000, 700, 300, 300),
            (850, 200, 300, 300)
        );
    }
}
