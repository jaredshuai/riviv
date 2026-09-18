//! Pixel-buffer math (pure logic, unit-tested).
//!
//! M2 seam: mipmap generation math (#9) lands here beside the BGRA
//! conversion and the alpha compositing (#3, landed — the composite
//! background itself is a runtime parameter since #24, snapshot from
//! the config at request time).
//!
//! #76 makes the CPU frame the source of truth: [`PixelFrame`] is what
//! the decode worker produces (plain memory, naturally `Send` — the
//! cross-thread boundary carries no GDI objects) and what every
//! consumer reads directly (the RGB status readout, the clipboard image
//! blit's source, the rotate pass, the future D2D upload). The GDI face
//! (DIB section + memory DC + mips) is a UI-thread derivation owned by
//! `surface::Surface`.

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

/// The decoded frame as pure memory (#76, ADR 0002 D3): top-down 32bpp
/// BGRA, exactly `width * height * 4` bytes, alpha forced opaque (the
/// invariant `composite_over_background_in_place` and the clipboard DIB
/// parser's three copy paths pin). This is the source of truth — the
/// GDI face is derived from it on the UI thread; device-loss recovery
/// and the D2D upload (#80) will re-derive from these bytes without
/// re-decoding.
///
/// `mip_target` is the decode-side mip pre-generation decision (the
/// pure `select_mip_level` + budget-gate math, kept verbatim in the
/// loader): how many levels the UI thread pre-generates when it builds
/// the frame's GDI face. Zero means "no pre-generation" (the gate spent
/// the animation's budget, or the viewport asked for none).
#[derive(Debug)]
pub(crate) struct PixelFrame {
    pub(crate) pixels: Box<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mip_target: u32,
}

impl PixelFrame {
    /// Row stride in bytes — the invariant `width * 4` (top-down,
    /// tightly packed), expressed as a method rather than a stored field
    /// so it can never drift from `width`. Test-only until the D2D
    /// upload path (#80) consumes it.
    #[cfg(test)]
    pub(crate) fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// `rgba` holds exactly `width * height * 4` bytes; converted to
    /// BGRA in place and boxed. Pure memory — infallible (a Rust
    /// allocation failure is a process-level abort, not a load failure;
    /// the worker-side GDI allocation errors this replaces were the
    /// system-level failures that could still happen at decode).
    pub(crate) fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Self {
        debug_assert_eq!(rgba.len(), width as usize * height as usize * 4);
        rgba8_to_bgra_in_place(&mut rgba);
        Self::from_bgra(width, height, rgba)
    }

    /// The BGRA-native entry (the clipboard DIB parser already emits
    /// this layout, #66): boxed as-is, no swizzle pass.
    pub(crate) fn from_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Self {
        debug_assert_eq!(bgra.len(), width as usize * height as usize * 4);
        PixelFrame {
            pixels: bgra.into_boxed_slice(),
            width,
            height,
            mip_target: 0,
        }
    }

    /// The pixel dimensions (the load protocol's test surface for frame
    /// payloads; production reads them through the `Surface` wrapper).
    #[cfg(test)]
    pub(crate) fn dims(&self) -> (i32, i32) {
        (self.width as i32, self.height as i32)
    }
}

/// Read one pixel of a top-down tightly-packed BGRA buffer as an
/// (R, G, B) triple (#76; replaces the #47 status readout's `GetPixel`
/// on the frame's memory DC). `width` is the buffer's pixel width (the
/// row stride); `None` for any out-of-bounds coordinate — the caller
/// maps that to the `CLR_INVALID` read-through (255, 255, 255) the
/// unchecked GDI path produced, preserving the failure arm byte for
/// byte.
pub(crate) fn sample_bgra(pixels: &[u8], width: i32, x: i32, y: i32) -> Option<(u8, u8, u8)> {
    if x < 0 || y < 0 || x >= width {
        return None;
    }
    let idx = (y as usize * width as usize + x as usize) * 4;
    let px = pixels.get(idx..idx + 4)?;
    Some((px[2], px[1], px[0]))
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

    // ---- PixelFrame (#76) ----

    #[test]
    fn pixelframe_from_rgba_swizzles_and_boxes() {
        // RGBA [10,20,30,255 | 1,2,3,255] becomes BGRA [30,20,10,255 |
        // 3,2,1,255], owned as a boxed slice.
        let frame = PixelFrame::from_rgba(2, 1, vec![10, 20, 30, 255, 1, 2, 3, 255]);
        assert_eq!(&frame.pixels[..], &[30, 20, 10, 255, 3, 2, 1, 255]);
    }

    #[test]
    fn pixelframe_from_bgra_keeps_bytes_and_defaults_to_no_pregen() {
        let frame = PixelFrame::from_bgra(1, 2, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(&frame.pixels[..], &[1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(frame.mip_target, 0, "the decode side sets it explicitly");
        assert_eq!(frame.dims(), (1, 2));
    }

    #[test]
    fn pixelframe_stride_is_width_times_four() {
        let frame = PixelFrame::from_bgra(7, 3, vec![0u8; 7 * 3 * 4]);
        assert_eq!(frame.stride(), 28);
        assert_eq!(frame.pixels.len(), frame.stride() * 3);
    }

    #[test]
    fn pixelframe_is_send() {
        // The whole point of the CPU frame (#76): the worker -> UI
        // boundary carries plain memory, no `unsafe impl Send` needed —
        // pinned so a future GDI-typed field cannot sneak in silently.
        fn assert_send<T: Send>() {}
        assert_send::<PixelFrame>();
    }

    // ---- sample_bgra (#76) ----

    /// A 3x2 BGRA canvas: pixel (x, y) tagged with its coordinates so
    /// the channel order reads directly off the assertion values.
    fn canvas_3x2() -> Vec<u8> {
        let mut v = vec![0u8; 3 * 2 * 4];
        for (i, px) in v.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 3, i / 3);
            *px = [x as u8, y as u8, (x * 10 + y) as u8, 255];
        }
        v
    }

    #[test]
    fn sample_bgra_reads_rgb_from_the_addressed_pixel() {
        // Pixel (1, 1) is stored as [B=1, G=1, R=11, 255] — the triple
        // comes back ordered (R, G, B), exactly what the COLORREF
        // unpacking of the old GetPixel produced.
        let px = canvas_3x2();
        assert_eq!(sample_bgra(&px, 3, 1, 1), Some((11, 1, 1)));
        assert_eq!(sample_bgra(&px, 3, 2, 0), Some((20, 0, 2)));
        assert_eq!(sample_bgra(&px, 3, 0, 0), Some((0, 0, 0)));
        // The last pixel in the buffer.
        assert_eq!(sample_bgra(&px, 3, 2, 1), Some((21, 1, 2)));
    }

    #[test]
    fn sample_bgra_rejects_every_out_of_bounds_shape() {
        let px = canvas_3x2();
        // x past the width must NOT wrap into the next row.
        assert_eq!(sample_bgra(&px, 3, 3, 0), None);
        // y past the height (the buffer's end guards it).
        assert_eq!(sample_bgra(&px, 3, 0, 2), None);
        // Negative coordinates.
        assert_eq!(sample_bgra(&px, 3, -1, 0), None);
        assert_eq!(sample_bgra(&px, 3, 0, -1), None);
    }
}
