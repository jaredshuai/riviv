//! Render-fit math (pure logic, unit-tested).
//!
//! M2 seam: the 16-step zoom presets, pan clamping and wheel-anchor math (#7)
//! land here beside the fit-to-window computation (zoom level 0 IS fit).

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

#[cfg(test)]
mod tests {
    use super::*;

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
