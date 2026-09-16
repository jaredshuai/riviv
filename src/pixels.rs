//! Pixel-buffer math (pure logic, unit-tested).
//!
//! M2 seam: mipmap generation math (#9) lands here beside the BGRA
//! conversion and the alpha compositing (#3, landed — the composite
//! background itself is a runtime parameter since #24, snapshot from
//! the config at request time).

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

/// Rotate a top-down 32bpp BGRA buffer 90° clockwise (#43; upstream
/// `_viv_orientate_hbitmap` orientation 6, viv.c:13810-13819: the Edit →
/// Rotate Clockwise in-memory pass). `src` holds `wide * high` pixels of 4
/// bytes; `dst` receives `high * wide` — the dimensions swap. The pixel
/// mapping is upstream's verbatim: `new[x + y * high] = old[y + (high - 1
/// - x) * wide]`, i.e. old(row r, col c) lands at new(col `high - 1 - r`,
/// row `c`) — the top-left corner moves to the top-right.
pub(crate) fn rotate_bgra_90_cw(src: &[u8], wide: usize, high: usize, dst: &mut [u8]) {
    debug_assert_eq!(src.len(), wide * high * 4);
    debug_assert_eq!(dst.len(), wide * high * 4);
    for y in 0..wide {
        for x in 0..high {
            let src_idx = (y + (high - x - 1) * wide) * 4;
            let dst_idx = (x + y * high) * 4;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
}

/// Rotate 90° counterclockwise — upstream orientation 8 (viv.c:13841-13850,
/// Edit → Rotate Counterclockwise): `new[x + y * high] = old[(wide - 1 - y)
/// + x * wide]`, i.e. old(row r, col c) lands at new(col `r`, row
/// `wide - 1 - c`) — the top-left corner moves to the bottom-left.
pub(crate) fn rotate_bgra_270_cw(src: &[u8], wide: usize, high: usize, dst: &mut [u8]) {
    debug_assert_eq!(src.len(), wide * high * 4);
    debug_assert_eq!(dst.len(), wide * high * 4);
    for y in 0..wide {
        for x in 0..high {
            let src_idx = ((wide - y - 1) + x * wide) * 4;
            let dst_idx = (x + y * high) * 4;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
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

    /// A 2-wide × 3-high ramp; each pixel is its linear index × 4 bytes
    /// (B,G,R,A — the bytes carry an index tag, alpha last).
    fn ramp_2x3() -> Vec<u8> {
        let mut v = vec![0u8; 2 * 3 * 4];
        for (i, px) in v.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *px = [i as u8, (i + 10) as u8, (i + 20) as u8, 255];
        }
        v
    }

    #[test]
    fn clockwise_rotation_moves_the_top_left_to_the_top_right() {
        // viv.c orientation 6: old(row r, col c) -> new(col high-1-r, row c).
        // The 2×3 ramp's four corners: 0->new top-right, 1->new bottom-right,
        // 4->new top-left, 5->new bottom-left; the result is 3 wide × 2 high.
        let src = ramp_2x3();
        let mut dst = vec![0u8; src.len()];
        rotate_bgra_90_cw(&src, 2, 3, &mut dst);
        let px = |i: usize| -> [u8; 4] { dst[i * 4..i * 4 + 4].try_into().unwrap() };
        assert_eq!(px(0), [4, 14, 24, 255]); // new row 0: old rows 2,1,0
        assert_eq!(px(1), [2, 12, 22, 255]);
        assert_eq!(px(2), [0, 10, 20, 255]);
        assert_eq!(px(3), [5, 15, 25, 255]); // new row 1
        assert_eq!(px(4), [3, 13, 23, 255]);
        assert_eq!(px(5), [1, 11, 21, 255]);
    }

    #[test]
    fn counterclockwise_rotation_moves_the_top_left_to_the_bottom_left() {
        // viv.c orientation 8: old(row r, col c) -> new(col r, row wide-1-c).
        // Old top-left (pixel 0) lands at the bottom-left; the result is
        // 3 wide × 2 high with old columns reversed into rows.
        let src = ramp_2x3();
        let mut dst = vec![0u8; src.len()];
        rotate_bgra_270_cw(&src, 2, 3, &mut dst);
        let px = |i: usize| -> [u8; 4] { dst[i * 4..i * 4 + 4].try_into().unwrap() };
        assert_eq!(px(0), [1, 11, 21, 255]); // new row 0: old col 1, rows 0..2
        assert_eq!(px(1), [3, 13, 23, 255]);
        assert_eq!(px(2), [5, 15, 25, 255]);
        assert_eq!(px(3), [0, 10, 20, 255]); // new row 1: old col 0
        assert_eq!(px(4), [2, 12, 22, 255]);
        assert_eq!(px(5), [4, 14, 24, 255]);
    }

    #[test]
    fn four_clockwise_rotations_return_the_original_buffer() {
        let src = ramp_2x3();
        let mut cur = src.clone();
        let mut scratch = vec![0u8; src.len()];
        // 2×3 -> 3×2 -> 2×3 -> 3×2 -> 2×3: alternate the dimension pair.
        for (wide, high) in [(2usize, 3usize), (3, 2), (2, 3), (3, 2)] {
            rotate_bgra_90_cw(&cur, wide, high, &mut scratch);
            std::mem::swap(&mut cur, &mut scratch);
        }
        assert_eq!(cur, src);
    }

    #[test]
    fn clockwise_and_counterclockwise_are_inverses() {
        let src = ramp_2x3();
        let mut cw = vec![0u8; src.len()];
        rotate_bgra_90_cw(&src, 2, 3, &mut cw);
        let mut back = vec![0u8; src.len()];
        rotate_bgra_270_cw(&cw, 3, 2, &mut back);
        assert_eq!(back, src);
    }
}
