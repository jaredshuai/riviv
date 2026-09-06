//! Zoom & pan view model (pure logic, unit-tested) — issue #7.
//!
//! Mirrors upstream's view globals (`_viv_zoom_pos` / `_viv_1to1` /
//! `_viv_old_zoom_pos` / `_viv_view_x..iy`, viv.c:677-683) as one state
//! machine so every piece of geometry math is testable without a window:
//! - the 16-step preset curve lerps the render size from the fit size
//!   toward a 16x per-axis maximum (viv.c:685 + 6995-7017) — level 0 IS
//!   fit (M1 semantics, `fit::fit_shrink`), level 15 is 1600%, NOT 1:1;
//! - 1:1 pixel-exact display is a separate temporary mode (`_viv_1to1`)
//!   whose render size is the source size verbatim (viv.c:6878-6882);
//! - the pan offset keeps the image centered until it outgrows the
//!   viewport, then clamps so an edge never leaves a gap
//!   (`_viv_view_set`, `config_keep_centered = 1`, viv.c:6465-6496);
//! - the resize anchor is the SOURCE pixel under the viewport center
//!   (`_viv_view_ix/iy`), reprojected and re-clamped on WM_SIZE
//!   (viv.c:1643-1651 + 6497-6520);
//! - a wheel notch / zoom key steps one level and re-anchors so the
//!   source pixel under the cursor stays under the cursor
//!   (`_viv_do_mousewheel_action` action 0, viv.c:13932-14097).

use crate::fit::fit_shrink;

/// The 16-step zoom curve (viv.c:685). Index 0 is fit; the value lerps the
/// render size from the fit size toward the 16x maximum per axis.
pub(crate) const ZOOM_PRESETS: [f32; 16] = [
    0.0000, 0.0100, 0.0225, 0.0379, 0.0569, 0.0806, 0.1098, 0.1461, 0.1909, 0.2465, 0.3154, 0.4007,
    0.5063, 0.6372, 0.7993, 1.0000,
];

/// The per-axis zoom maximum: 16x the image (upstream `max_zoom_wide =
/// _viv_image_wide * 16`, viv.c:7004 — with `fill_window = 0` the fit size
/// never exceeds the source, so the `rw * 16` arm is unreachable in riviv).
const ZOOM_LIMIT: i32 = 16;

/// The area an image renders into: the client rect minus the status bar
/// (upstream subtracts `_viv_get_status_high()` everywhere it computes
/// `wide`/`high`, e.g. viv.c:13954-13957).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Viewport {
    pub(crate) wide: i32,
    pub(crate) high: i32,
}

/// Zoom + pan state for the displayed image. The image's top-left renders at
/// `center - render_size/2 - view` (upstream `rx = (wide / 2) - (rw / 2) -
/// _viv_view_x`, viv.c:6460), so `view` is how far the image has been
/// dragged away from the centered position — it only moves off zero while
/// the image is larger than the viewport.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct View {
    /// Current zoom level 0..=15 (0 = fit). Held at 0 while `one_to_one` is
    /// on; the level to restore afterwards lives in `saved_pos`.
    pos: i32,
    /// Temporary 1:1 pixel-exact mode (`_viv_1to1`).
    one_to_one: bool,
    /// The zoom level to restore when leaving 1:1 (`_viv_old_zoom_pos`;
    /// upstream's `_viv_have_old_zoom` guard is always true after the first
    /// entry, so it is omitted).
    saved_pos: i32,
    /// Pan offset, one component per axis (`_viv_view_x/y`).
    pub(crate) view_x: i32,
    pub(crate) view_y: i32,
    /// The source pixel under the viewport center (`_viv_view_ix/iy`) —
    /// the anchor that survives a window resize.
    center_src_x: f64,
    center_src_y: f64,
}

impl Default for View {
    fn default() -> Self {
        View::new()
    }
}

impl View {
    pub(crate) fn new() -> Self {
        View {
            pos: 0,
            one_to_one: false,
            saved_pos: 0,
            view_x: 0,
            view_y: 0,
            center_src_x: 0.0,
            center_src_y: 0.0,
        }
    }

    /// Back to the fresh-image state — runs on every display swap and blank
    /// (upstream `_viv_clear`, viv.c:1282-1288: zoom_pos/view/ix/iy/1to1 all
    /// reset when a new image takes the display).
    pub(crate) fn reset(&mut self) {
        *self = View::new();
    }

    /// The rendered size at the current level, in viewport pixels (upstream
    /// `_viv_get_render_size` under the default config: keep-aspect,
    /// allow-shrinking, no fill). A degenerate viewport or source yields
    /// (0, 0) like upstream's `!(wide && high)` early-out (viv.c:6871).
    pub(crate) fn render_size(&self, src_w: i32, src_h: i32, vp: Viewport) -> (i32, i32) {
        self.size_at(self.pos, self.one_to_one, src_w, src_h, vp)
    }

    /// The preset-curve size at `pos` (shared by the wheel handler's 1:1-exit
    /// search, which probes levels the way upstream's loops do).
    fn size_at(
        &self,
        pos: i32,
        one_to_one: bool,
        src_w: i32,
        src_h: i32,
        vp: Viewport,
    ) -> (i32, i32) {
        if src_w <= 0 || src_h <= 0 || vp.wide <= 0 || vp.high <= 0 {
            return (0, 0);
        }
        if one_to_one {
            return (src_w, src_h);
        }
        let (fw, fh) = fit_shrink(src_w, src_h, vp.wide, vp.high);
        if pos <= 0 {
            return (fw, fh);
        }
        let idx = pos.min(ZOOM_LIMIT - 1) as usize;
        // Upstream: rw = rw + (int)((max_zoom_wide - rw) * preset) — float
        // multiply, int truncation, per axis independently (aspect drifts a
        // little between levels; that is the upstream curve, viv.c:7012-7016).
        let rw = fw + ((i64::from(ZOOM_LIMIT * src_w - fw) as f32) * ZOOM_PRESETS[idx]) as i32;
        let rh = fh + ((i64::from(ZOOM_LIMIT * src_h - fh) as f32) * ZOOM_PRESETS[idx]) as i32;
        (rw, rh)
    }

    /// Commit a candidate pan offset, clamped, and refresh the center anchor
    /// (upstream `_viv_view_set`, viv.c:6434-6560 — `config_keep_centered=1`
    /// arm). An image that fits the viewport is re-pinned to the center.
    pub(crate) fn set_view(&mut self, vx: i32, vy: i32, src_w: i32, src_h: i32, vp: Viewport) {
        let (rw, rh) = self.render_size(src_w, src_h, vp);
        let rx = vp.wide / 2 - rw / 2 - vx;
        let ry = vp.high / 2 - rh / 2 - vy;
        self.view_x = clamp_axis(rx, rw, vp.wide, vx);
        self.view_y = clamp_axis(ry, rh, vp.high, vy);
        // The resize anchor: which SOURCE pixel sits at the viewport center
        // now (viv.c:6497-6520; `_viv_dst_pos` is 500 = center, so its term
        // collapses into wide/2 here).
        let rx = vp.wide / 2 - rw / 2 - self.view_x;
        let ry = vp.high / 2 - rh / 2 - self.view_y;
        if rw != 0 {
            self.center_src_x =
                (i64::from(vp.wide / 2 - rx) * i64::from(src_w)) as f64 / f64::from(rw);
        }
        if rh != 0 {
            self.center_src_y =
                (i64::from(vp.high / 2 - ry) * i64::from(src_h)) as f64 / f64::from(rh);
        }
    }

    /// Drag-pan by `(mx, my)` cursor-delta pixels — the image follows the
    /// mouse (upstream `_viv_view_scroll`, viv.c:11931-11947: view decreases
    /// by the delta, then clamps).
    pub(crate) fn scroll_by(&mut self, mx: i32, my: i32, src_w: i32, src_h: i32, vp: Viewport) {
        self.set_view(self.view_x - mx, self.view_y - my, src_w, src_h, vp);
    }

    /// Re-anchor after the viewport resized: put the source pixel that was
    /// at the center back at the center, then re-clamp (upstream WM_SIZE,
    /// viv.c:1643-1651). Returns whether the pan offset moved (a repaint is
    /// owed — upstream `view_set(invalidate = 1)`).
    pub(crate) fn on_resize(&mut self, src_w: i32, src_h: i32, vp: Viewport) -> bool {
        let (rw, rh) = self.render_size(src_w, src_h, vp);
        // Upstream: (int)((ix * rw) / src + 0.5) + dst_pos-term - wide/2 - rw/2
        // — with dst_pos 500 the middle terms cancel to plain -rw/2, and the
        // truncation lands BEFORE the subtraction (viv.c:1648-1649).
        let vx = ((self.center_src_x * f64::from(rw) / f64::from(src_w)) + 0.5) as i32 - rw / 2;
        let vy = ((self.center_src_y * f64::from(rh) / f64::from(src_h)) + 0.5) as i32 - rh / 2;
        let before = (self.view_x, self.view_y);
        self.set_view(vx, vy, src_w, src_h, vp);
        (self.view_x, self.view_y) != before
    }

    /// One zoom step (wheel notch or +/- key) anchored at `cursor` (client
    /// pixels). Returns whether the display changed (a repaint is owed).
    /// Upstream `_viv_do_mousewheel_action` action 0, viv.c:13932-14097:
    /// step one level (or, when leaving 1:1, search for the first level
    /// whose size brackets the 1:1 size), then re-anchor so the source
    /// pixel under the cursor stays under the cursor, then clamp.
    pub(crate) fn zoom_step(
        &mut self,
        out: bool,
        cursor: (i32, i32),
        src_w: i32,
        src_h: i32,
        vp: Viewport,
    ) -> bool {
        let (old_rw, old_rh) = self.render_size(src_w, src_h, vp);
        let (cx, cy) = cursor;
        // The cursor's pixel offset inside the current (scaled) image —
        // unclamped on purpose: upstream's clamping here is commented out
        // (viv.c:14013-14036), an off-image cursor still anchors.
        let rx = vp.wide / 2 - old_rw / 2 - self.view_x;
        let ry = vp.high / 2 - old_rh / 2 - self.view_y;
        let old_px = cx - rx;
        let old_py = cy - ry;
        let old_pos = self.pos;
        if self.one_to_one {
            // Leaving 1:1 by wheel: jump to the first level past the 1:1
            // size (viv.c:14014-14043). The loop's exit value (16 / -1)
            // clamps to the curve ends exactly like upstream.
            self.one_to_one = false;
            self.pos = if out {
                let mut found = -1;
                for pos in (0..ZOOM_LIMIT).rev() {
                    if self.size_at(pos, false, src_w, src_h, vp).0 < old_rw {
                        found = pos;
                        break;
                    }
                }
                found
            } else {
                let mut found = ZOOM_LIMIT;
                for pos in 0..ZOOM_LIMIT {
                    if self.size_at(pos, false, src_w, src_h, vp).0 > old_rw {
                        found = pos;
                        break;
                    }
                }
                found
            };
        } else {
            self.pos += if out { -1 } else { 1 };
        }
        self.pos = self.pos.clamp(0, ZOOM_LIMIT - 1);
        if self.pos == old_pos {
            // Upstream only re-anchors when the level moved (viv.c:14052).
            return false;
        }
        let (rw, rh) = self.render_size(src_w, src_h, vp);
        let new_px = if old_rw != 0 {
            i64::from(old_px) * i64::from(rw) / i64::from(old_rw)
        } else {
            0
        };
        let new_py = if old_rh != 0 {
            i64::from(old_py) * i64::from(rh) / i64::from(old_rh)
        } else {
            0
        };
        // Want: rx_new = cursor - new_px; view = center - rw/2 - rx_new.
        self.set_view(
            vp.wide / 2 - rw / 2 - cx + new_px as i32,
            vp.high / 2 - rh / 2 - cy + new_py as i32,
            src_w,
            src_h,
            vp,
        );
        true
    }

    /// Ctrl+Alt+0 — toggle temporary 1:1 (upstream `_viv_view_1to1`,
    /// viv.c:9318-9339): entering saves the current level and forces 0;
    /// leaving restores it. The view re-clamps for the new size (upstream
    /// re-runs view_set with the current offset and invalidates).
    pub(crate) fn toggle_one_to_one(&mut self, src_w: i32, src_h: i32, vp: Viewport) {
        if self.one_to_one {
            self.one_to_one = false;
            self.pos = self.saved_pos;
        } else {
            self.saved_pos = self.pos;
            self.one_to_one = true;
            self.pos = 0;
        }
        self.set_view(self.view_x, self.view_y, src_w, src_h, vp);
    }

    /// Ctrl+0 — back to fit (upstream `VIV_ID_VIEW_ZOOM_RESET`,
    /// viv.c:1676-1681): leave 1:1, level 0, re-clamp the current offset
    /// (which recenters an image that now fits again).
    pub(crate) fn reset_zoom(&mut self, src_w: i32, src_h: i32, vp: Viewport) {
        self.one_to_one = false;
        self.pos = 0;
        self.set_view(self.view_x, self.view_y, src_w, src_h, vp);
    }
}

/// One axis of the keep-centered clamp (upstream viv.c:6465-6496): while the
/// image exceeds the viewport it may pan freely but never shows a gap past
/// either edge (`rx` in `[0, wide - rw]`); the moment it fits, the offset
/// snaps back to center (0).
fn clamp_axis(r: i32, len: i32, limit: i32, view: i32) -> i32 {
    if len > limit {
        let center_off = limit / 2 - len / 2;
        let mut view = view;
        if r > 0 {
            view = center_off; // image left pulled past the viewport edge
        }
        if r + len < limit {
            view = center_off - (limit - len); // image right pulled past
        }
        view
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: Viewport = Viewport {
        wide: 400,
        high: 300,
    };

    #[test]
    fn level_zero_is_fit_and_never_upscales() {
        let v = View::new();
        // 2000x1500 into 400x300 -> 400x300; 100x50 stays 100x50.
        assert_eq!(v.render_size(2000, 1500, VP), (400, 300));
        assert_eq!(v.render_size(100, 50, VP), (100, 50));
    }

    #[test]
    fn top_level_scales_sixteen_times_per_axis() {
        // A small image zooms to 16x at the top level (max_zoom = image*16,
        // viv.c:7004) — 1600%, not 1:1.
        let mut v = View::new();
        for _ in 0..20 {
            v.zoom_step(false, (200, 150), 100, 60, VP);
        }
        assert_eq!(v.pos, 15);
        assert_eq!(v.render_size(100, 60, VP), (1600, 960));
    }

    #[test]
    fn intermediate_levels_follow_the_preset_curve() {
        // fit(100x60 in 400x300) = 100x60; level 3:
        // 100 + (1600-100)*0.0379 = 155 (0.0379*1500=56.85 truncates to 56)
        let v = View {
            pos: 3,
            ..View::new()
        };
        assert_eq!(v.render_size(100, 60, VP), (156, 94));
        // level 14: 100 + 1500*0.7993 -> 1298; 60 + 900*0.7993 -> 779
        let v = View {
            pos: 14,
            ..View::new()
        };
        assert_eq!(v.render_size(100, 60, VP), (1298, 779));
    }

    #[test]
    fn shrunk_image_zooms_from_its_fit_size() {
        // 800x600 into 400x300 fits to 400x300; level 15 = 16x the SOURCE.
        let v = View {
            pos: 15,
            ..View::new()
        };
        assert_eq!(v.render_size(800, 600, VP), (12800, 9600));
    }

    #[test]
    fn one_to_one_renders_the_source_exactly() {
        let v = View {
            one_to_one: true,
            ..View::new()
        };
        assert_eq!(v.render_size(800, 600, VP), (800, 600));
        // ...even when the fit size would be smaller or larger.
        assert_eq!(v.render_size(100, 50, VP), (100, 50));
    }

    #[test]
    fn wheel_keeps_the_source_pixel_under_the_cursor() {
        // THE anchor assertion: after one zoom step, the source pixel that
        // was under the cursor is still under the cursor. 800x600 in
        // 400x300 fits to 400x300; cursor at (100, 100) sits on source
        // pixel (200, 200); level 1 renders 524x393 and the integer
        // projection divides evenly at this point.
        let mut v = View::new();
        let cursor = (100, 100);
        let src_px: (i64, i64) = (200, 200);
        assert!(v.zoom_step(false, cursor, 800, 600, VP));
        let (rw, rh) = v.render_size(800, 600, VP);
        assert_eq!((rw, rh), (524, 393));
        let rx = 400 / 2 - rw / 2 - v.view_x;
        let ry = 300 / 2 - rh / 2 - v.view_y;
        assert_eq!((cursor.0 - rx) as i64 * 800 / i64::from(rw), src_px.0);
        assert_eq!((cursor.1 - ry) as i64 * 600 / i64::from(rh), src_px.1);
        // A second step keeps it there too (stability across continuous
        // zooming, the acceptance criterion). The projection truncates to
        // whole pixels, so "still under the cursor" is exact when the
        // division lands evenly and within 1 px otherwise.
        assert!(v.zoom_step(false, cursor, 800, 600, VP));
        let (rw, rh) = v.render_size(800, 600, VP);
        let rx = 400 / 2 - rw / 2 - v.view_x;
        let ry = 300 / 2 - rh / 2 - v.view_y;
        let bx = (cursor.0 - rx) as i64 * 800 / i64::from(rw);
        let by = (cursor.1 - ry) as i64 * 600 / i64::from(rh);
        assert!((bx - src_px.0).abs() <= 1);
        assert!((by - src_px.1).abs() <= 1);
    }

    #[test]
    fn wheel_out_below_fit_stays_put() {
        let mut v = View::new();
        assert!(!v.zoom_step(true, (200, 150), 800, 600, VP));
        assert_eq!(v.pos, 0);
    }

    #[test]
    fn wheel_in_at_the_top_stays_put() {
        let mut v = View {
            pos: 15,
            ..View::new()
        };
        assert!(!v.zoom_step(false, (200, 150), 100, 60, VP));
        assert_eq!(v.pos, 15);
    }

    #[test]
    fn wheel_leaving_one_to_one_brackets_the_one_to_one_size() {
        // In 1:1 (source 800x600) a wheel-in must land on the first level
        // whose width exceeds 800; a wheel-out on the first from the top
        // whose width is below 800.
        let mut v = View {
            one_to_one: true,
            ..View::new()
        };
        assert!(v.zoom_step(false, (200, 150), 800, 600, VP));
        assert!(!v.one_to_one);
        assert!(v.render_size(800, 600, VP).0 > 800);

        let mut v = View {
            one_to_one: true,
            ..View::new()
        };
        assert!(v.zoom_step(true, (200, 150), 800, 600, VP));
        assert!(v.render_size(800, 600, VP).0 < 800);
    }

    #[test]
    fn one_to_one_toggle_restores_the_saved_level() {
        let mut v = View {
            pos: 5,
            ..View::new()
        };
        v.toggle_one_to_one(800, 600, VP);
        assert_eq!(v.render_size(800, 600, VP), (800, 600));
        v.toggle_one_to_one(800, 600, VP);
        assert_eq!(v.pos, 5);
    }

    #[test]
    fn zoom_reset_recenters_to_fit() {
        let mut v = View::new();
        v.zoom_step(false, (200, 150), 800, 600, VP);
        v.scroll_by(500, 500, 800, 600, VP); // pan hard against the clamp
        v.reset_zoom(800, 600, VP);
        assert_eq!(v.pos, 0);
        assert_eq!((v.view_x, v.view_y), (0, 0));
        assert_eq!(v.render_size(800, 600, VP), (400, 300));
    }

    #[test]
    fn drag_pan_follows_the_cursor_and_clamps_at_the_edges() {
        // 1280x960 image at level 15 renders 20480x15360 — far beyond the
        // viewport, so panning moves 1:1 with the cursor until an edge
        // reaches the border and pins.
        let mut v = View {
            pos: 15,
            ..View::new()
        };
        let (rw, _) = v.render_size(1280, 960, VP);
        assert!(rw > VP.wide);
        v.scroll_by(10, 0, 1280, 960, VP);
        let rx_before = VP.wide / 2 - rw / 2 - v.view_x;
        v.scroll_by(10, 0, 1280, 960, VP);
        let rx_after = VP.wide / 2 - rw / 2 - v.view_x;
        assert_eq!(rx_after - rx_before, 10); // image followed the cursor
        // Dragging left "forever" walks toward the image's right end: the
        // image's right edge must never leave the viewport's right edge.
        v.scroll_by(-1_000_000, 0, 1280, 960, VP);
        assert_eq!(VP.wide / 2 - rw / 2 - v.view_x, VP.wide - rw);
        // ...and dragging back the other way pins at the left edge instead.
        v.scroll_by(2_000_000, 0, 1280, 960, VP);
        assert_eq!(VP.wide / 2 - rw / 2 - v.view_x, 0);
    }

    #[test]
    fn fitted_image_cannot_be_dragged() {
        // An image that fits stays centered no matter how hard it is
        // dragged (keep_centered, viv.c:6465-6496).
        let mut v = View::new();
        v.scroll_by(100, 100, 100, 60, VP);
        assert_eq!((v.view_x, v.view_y), (0, 0));
    }

    #[test]
    fn resize_keeps_the_center_source_pixel_at_the_center() {
        // Pan a zoomed image a little, then shrink the viewport: the source
        // pixel that sat at the viewport center must still be the one at the
        // center. 100x60 at level 15 renders 1600x960 over 400x300.
        let mut v = View {
            pos: 15,
            ..View::new()
        };
        let (rw, rh) = v.render_size(100, 60, VP);
        assert_eq!((rw, rh), (1600, 960));
        v.scroll_by(-10, -5, 100, 60, VP); // the image follows the cursor
        let rx0 = VP.wide / 2 - rw / 2 - v.view_x;
        let ry0 = VP.high / 2 - rh / 2 - v.view_y;
        let cx0 = (i64::from(VP.wide / 2 - rx0) * 100 / i64::from(rw)) as i32;
        let cy0 = (i64::from(VP.high / 2 - ry0) * 60 / i64::from(rh)) as i32;
        // The pan is small and the render is far larger than either
        // viewport, so the re-anchor target stays inside the clamp.
        let smaller = Viewport {
            wide: 200,
            high: 100,
        };
        v.on_resize(100, 60, smaller);
        let rx = smaller.wide / 2 - rw / 2 - v.view_x;
        let ry = smaller.high / 2 - rh / 2 - v.view_y;
        assert_eq!(
            (i64::from(smaller.wide / 2 - rx) * 100 / i64::from(rw)) as i32,
            cx0
        );
        assert_eq!(
            (i64::from(smaller.high / 2 - ry) * 60 / i64::from(rh)) as i32,
            cy0
        );
    }

    #[test]
    fn resize_growing_window_recenters_a_fitted_image() {
        // A fitted image has no pan; growing the window must keep it at 0
        // (and report no change).
        let mut v = View::new();
        assert!(!v.on_resize(
            100,
            60,
            Viewport {
                wide: 900,
                high: 700
            }
        ));
        assert_eq!((v.view_x, v.view_y), (0, 0));
    }

    #[test]
    fn resize_reclamps_a_panned_view_into_the_smaller_bounds() {
        let mut v = View {
            pos: 15,
            ..View::new()
        };
        let (rw, _) = v.render_size(1280, 960, VP);
        v.scroll_by(2_000_000, 0, 1280, 960, VP); // pinned at the left edge
        assert_eq!(VP.wide / 2 - rw / 2 - v.view_x, 0);
        v.on_resize(
            1280,
            960,
            Viewport {
                wide: 380,
                high: 290,
            },
        );
        let (rw, _) = v.render_size(
            1280,
            960,
            Viewport {
                wide: 380,
                high: 290,
            },
        );
        let rx = 380 / 2 - rw / 2 - v.view_x;
        assert!(rx <= 380 && rx + rw >= 380, "gap after resize: rx={rx}");
    }

    #[test]
    fn reset_restores_the_fresh_view() {
        let mut v = View {
            pos: 9,
            one_to_one: true,
            saved_pos: 9,
            view_x: -40,
            view_y: 55,
            center_src_x: 12.5,
            center_src_y: 90.25,
        };
        v.reset();
        assert_eq!(v, View::new());
    }

    #[test]
    fn degenerate_geometry_renders_nothing() {
        let v = View::new();
        assert_eq!(
            v.render_size(100, 60, Viewport { wide: 0, high: 0 }),
            (0, 0)
        );
        assert_eq!(v.render_size(0, 60, VP), (0, 0));
        // ...and state changes stay inert rather than panicking. The level
        // itself still steps (upstream has no image guard here either,
        // viv.c:14041-14050) but nothing renders or moves.
        let mut v = View::new();
        assert!(v.zoom_step(false, (0, 0), 0, 0, VP));
        v.scroll_by(100, 100, 0, 0, VP);
        v.on_resize(0, 0, VP);
        v.toggle_one_to_one(0, 0, VP);
        v.reset_zoom(0, 0, VP);
        assert_eq!(v.view_x, 0);
        assert_eq!(v.view_y, 0);
    }
}
