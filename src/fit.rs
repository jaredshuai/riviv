//! Render-fit math (pure logic, unit-tested).
//!
//! This is the zoom curve's level 0; the 16-step presets, wheel anchoring
//! and pan clamping (#7) landed in `zoom.rs` on top of `fit_shrink`.

/// Fit `(src_w, src_h)` inside `(max_w, max_h)` keeping aspect ratio,
/// never upscaling (upstream `fill_window = 0`). The derived side rounds UP
/// with integer math — upstream viv.c:6895-6913 deliberately adds `high - 1`
/// so a 50%-window resize still stretches to the screen edges.
pub(crate) fn fit_shrink(src_w: i32, src_h: i32, max_w: i32, max_h: i32) -> (i32, i32) {
    if src_w <= 0 || src_h <= 0 || max_w <= 0 || max_h <= 0 {
        return (src_w.max(1), src_h.max(1));
    }
    let (w, h) = (i64::from(src_w), i64::from(src_h));
    let (mw, mh) = (i64::from(max_w), i64::from(max_h));
    let (mut rw, mut rh) = if mh * w < mw * h {
        // tall: height binds, width is derived (ceil)
        ((mh * w + h - 1) / h, mh)
    } else {
        // long: width binds, height is derived (ceil)
        (mw, (mw * h + w - 1) / w)
    };
    // never upscale (upstream !fill_window clamp, viv.c:6922-6928)
    if rw > w || rh > h {
        rw = w;
        rh = h;
    }
    (rw as i32, rh as i32)
}

/// The window-size command's client-area target (upstream
/// `VIV_ID_VIEW_WINDOW_SIZE_*`, viv.c:2107-2137 + 2147-2157): the image at
/// 50/100/200% by `kind`, or — `kind` 3 — the auto-fit FRACTION of the
/// full monitor (`os_MonitorRectFromWindow(hwnd, 1)` reads rcMonitor,
/// os.c:193+), each axis independently clamped into the WORK area.
/// auto_zoom fires this at every fresh image (viv.c:14363-14383); an
/// absent image leaves only the auto-fit kind meaningful (upstream
/// gates the percentage kinds on a live image).
pub(crate) fn window_size_client(
    image: (i32, i32),
    kind: i32,
    full_monitor: (i32, i32),
    work: (i32, i32),
    auto_fit: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (wm, wd, hm, hd) = auto_fit;
    let (w, h) = match kind {
        0 => (image.0 / 2, image.1 / 2),
        1 => image,
        2 => (image.0.saturating_mul(2), image.1.saturating_mul(2)),
        // A zero divisor makes upstream skip that axis (rect stays 0);
        // the clamp then keeps whatever the other axis produced.
        _ => (
            if wd != 0 { full_monitor.0 * wm / wd } else { 0 },
            if hd != 0 { full_monitor.1 * hm / hd } else { 0 },
        ),
    };
    (w.clamp(0, work.0), h.clamp(0, work.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_size_client_targets_the_image_percentages() {
        // 100x60 image: 50% -> 50x30, 100% -> 100x60, 200% -> 200x120,
        // each clamped into a 150x110 work area on its own axis (so 200%
        // gives 150x110 here).
        let img = (100, 60);
        let work = (150, 110);
        let fit = (3, 5, 3, 5);
        assert_eq!(
            window_size_client(img, 0, (9999, 9999), work, fit),
            (50, 30)
        );
        assert_eq!(
            window_size_client(img, 1, (9999, 9999), work, fit),
            (100, 60)
        );
        assert_eq!(
            window_size_client(img, 2, (9999, 9999), work, fit),
            (150, 110),
            "each axis clamps to the work area independently"
        );
    }

    #[test]
    fn window_size_client_auto_fit_takes_a_fraction_of_the_full_monitor() {
        // kind 3: 3/5 of the FULL monitor (os_MonitorRectFromWindow flag 1
        // reads rcMonitor, not rcWork) — 1920x1200 full over a 1920x1100
        // work area yields 1152x720 (the fraction of FULL, clamped to
        // WORK).
        assert_eq!(
            window_size_client((0, 0), 3, (1920, 1200), (1920, 1100), (3, 5, 3, 5)),
            (1152, 720)
        );
        // A zero divisor zeroes that axis (upstream leaves the rect 0).
        assert_eq!(
            window_size_client((7, 7), 3, (1920, 1200), (1920, 1100), (3, 0, 3, 5)),
            (0, 720)
        );
    }

    #[test]
    fn fit_never_upscales() {
        assert_eq!(fit_shrink(100, 50, 1000, 1000), (100, 50));
    }

    #[test]
    fn fit_caps_to_bounds_and_keeps_aspect() {
        assert_eq!(fit_shrink(2000, 1000, 1000, 1000), (1000, 500));
        assert_eq!(fit_shrink(1000, 2000, 1000, 1000), (500, 1000));
    }

    #[test]
    fn fit_floors_at_one_pixel() {
        assert_eq!(fit_shrink(10000, 10000, 1, 1), (1, 1));
    }

    #[test]
    fn fit_rounds_derived_side_upstream_style() {
        // upstream viv.c:6895-6913: 1000x333 into 400x400 -> 400x134 (not 133),
        // so a 50%-window resize still stretches to the screen edges.
        assert_eq!(fit_shrink(1000, 333, 400, 400), (400, 134));
        assert_eq!(fit_shrink(333, 1000, 400, 400), (134, 400));
    }
}
