//! Pixel-buffer math (pure logic, unit-tested).
//!
//! M2 seam: mipmap generation math (#9) lands here beside the BGRA
//! conversion and the alpha compositing (#3, landed).

/// Windowed background color, RGB. Upstream default is white (config.c:65-67)
/// and paint fills the client with the same value; a mismatch between the two
/// would show as a fringe around composited images. Configurable upstream,
/// hardcoded until the M3 settings work.
pub(crate) const WINDOWED_BACKGROUND_RGB: [u8; 3] = [255, 255, 255];

/// image crate yields RGBA rows (top-down); GDI 32bpp DIBs want BGRA.
pub(crate) fn rgba8_to_bgra_in_place(buf: &mut [u8]) {
    let (pixels, tail) = buf.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "RGBA8 buffer length must be a multiple of 4"
    );
    for px in pixels {
        px.swap(0, 2);
    }
}

/// Composite RGBA pixels over `bg`, forcing alpha to opaque: the DIB render
/// path (StretchBlt SRCCOPY) has no alpha channel of its own, so transparent
/// pixels must be resolved against the windowed background at decode time —
/// upstream's WebP integer formula, `out = bg + (src - bg) * a / 255`
/// (viv.c:10166-10168), applied per channel with truncating (i32) division
/// like the C code. The GDI+ path reaches the same result for GIF/PNG by
/// filling the frame bitmap with the background color and drawing with
/// SourceOver (viv.c:10639-10655).
///
/// Identity for fully opaque pixels, so it is applied unconditionally.
pub(crate) fn composite_over_background_in_place(rgba: &mut [u8], bg: [u8; 3]) {
    let (pixels, tail) = rgba.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "RGBA8 buffer length must be a multiple of 4"
    );
    for px in pixels {
        let a = i32::from(px[3]);
        if a == 255 {
            continue;
        }
        for (c, b) in px.iter_mut().zip(bg) {
            let b = i32::from(b);
            // Truncating toward zero matches the C integer division for the
            // (src - bg) term of either sign.
            *c = (b + ((i32::from(*c) - b) * a) / 255) as u8;
        }
        px[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_conversion_swaps_r_and_b_in_place() {
        let mut pixels = vec![10, 20, 30, 255, 1, 2, 3, 128];
        rgba8_to_bgra_in_place(&mut pixels);
        assert_eq!(pixels, vec![30, 20, 10, 255, 3, 2, 1, 128]);
    }

    #[test]
    fn fully_transparent_pixel_becomes_the_background_color() {
        // A transparent pixel shows the window background and nothing of the
        // source RGB underneath it (the M1 bug this replaces).
        let mut px = vec![200, 100, 50, 0];
        composite_over_background_in_place(&mut px, [10, 20, 30]);
        assert_eq!(px, vec![10, 20, 30, 255]);
    }

    #[test]
    fn opaque_pixels_pass_through_unchanged() {
        let src = vec![17, 34, 51, 255, 0, 0, 0, 255];
        let mut px = src.clone();
        composite_over_background_in_place(&mut px, [200, 200, 200]);
        assert_eq!(px, src);
    }

    #[test]
    fn half_alpha_pixel_uses_the_upstream_blend_formula() {
        // viv.c:10166-10168: out = bg + (src - bg) * a / 255, per channel.
        // (240 - 80) * 128 / 255 = 80; (0 - 80) * 128 / 255 = -40 (truncated
        // toward zero); (200 - 80) * 128 / 255 = 60; so 240 -> 160,
        // 0 -> 40 and 200 -> 140 over a flat 80 background.
        let mut px = vec![240, 0, 200, 128];
        composite_over_background_in_place(&mut px, [80, 80, 80]);
        assert_eq!(px, vec![160, 40, 140, 255]);
    }

    #[test]
    fn compositing_forces_alpha_to_opaque() {
        // GDI has no destination alpha; a partially transparent source must
        // not leave a residue alpha in the DIB.
        let mut px = vec![10, 20, 30, 99];
        composite_over_background_in_place(&mut px, [255, 255, 255]);
        assert_eq!(px[3], 255);
    }

    #[test]
    fn windowed_background_default_is_white_like_upstream() {
        // config.c:65-67; also matches the paint fill, which must never
        // diverge from the compositing background.
        assert_eq!(WINDOWED_BACKGROUND_RGB, [255, 255, 255]);
    }
}
