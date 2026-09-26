//! Pixel-buffer math (pure logic, unit-tested).
//!
//! BGRA conversion and the alpha compositing (#3, landed — the composite
//! background itself is a runtime parameter since #24, snapshot from
//! the config at request time).
//!
//! #76 makes the CPU frame the source of truth: [`PixelFrame`] is what
//! the decode worker produces (plain memory, naturally `Send` — the
//! cross-thread boundary carries no GDI objects) and what every
//! consumer reads directly (the RGB status readout, the clipboard image
//! blit's source, the rotate pass, the D2D upload). Through #89
//! `surface::Surface` also derived a UI-thread GDI face from it — that
//! derivation died with the GDI render arm (#90); the Surface is now
//! the master's plain holder.

use crate::transform_stage::ContentSpace;

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

/// The dump channels' swizzle (#80): packed top-down BGRA rows (a DIB
/// section the dump owns, or a D2D CPU-read mapping flattened row by row)
/// into RGBA for `image::RgbaImage`. `src` and `dst` hold the same byte
/// count; alpha passes through (both stacks render opaque frames — the
/// master invariant).
pub(crate) fn bgra_to_rgba(src: &[u8], dst: &mut [u8]) {
    let (src_px, src_tail) = src.as_chunks::<4>();
    let (dst_px, dst_tail) = dst.as_chunks_mut::<4>();
    debug_assert!(src_tail.is_empty() && dst_tail.is_empty());
    debug_assert_eq!(src_px.len(), dst_px.len());
    for (d, s) in dst_px.iter_mut().zip(src_px) {
        *d = [s[2], s[1], s[0], s[3]];
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

/// The BGRA-order sibling of [`composite_over_background_in_place`]: the
/// ICM path's transform already emits the master's BGRA layout (#77 —
/// the swizzle is folded into `TranslateBitmapBits`' output format), so
/// the composite runs directly on it. The caller's background triple
/// stays `[R, G, B]` (DecodeEnv's shape, like the RGBA sibling); it is
/// reversed HERE to match the `[B, G, R]` byte order the BGRA chunks
/// iterate.
pub(crate) fn composite_over_background_bgra_in_place(bgra: &mut [u8], bg: [u8; 3]) {
    composite_over_background_in_place(bgra, [bg[2], bg[1], bg[0]]);
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

// ---------------------------------------------------------------------
// FP16 conversion (L1's master encoding, ADR 0003 D2): the pure math
// of the gamma-sRGB-f16 master. #137 probe 1's hand conversion, landed
// — every reference point and the 256-code round trip are pinned below.
// ---------------------------------------------------------------------

/// Encode one f32 as IEEE 754 binary16 bits, round-to-nearest-even:
/// normals, subnormals, Inf/NaN (quieted) and overflow-to-infinity all
/// handled (probe 1: 1.0 = 0x3C00, 65504 = 0x7BFF, the smallest
/// subnormal 0x0001, 65520 rounds up to infinity).
pub(crate) fn f32_to_f16_bits(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        // Inf / NaN carry over; quiet the NaN bit.
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 };
    }
    if exp == 0 {
        // f32 subnormals (< 2^-126) always round to zero in f16.
        return sign;
    }
    let unbiased = exp - 127;
    if unbiased > 15 {
        // Overflow rounds to infinity.
        return sign | 0x7c00;
    }
    if unbiased >= -14 {
        let e = (unbiased + 15) as u32;
        let m = mant >> 13;
        let mut h = sign | ((e << 10) as u16) | m as u16;
        let round = mant & 0x1fff;
        if round > 0x1000 || (round == 0x1000 && (m & 1) == 1) {
            h += 1; // carry into the exponent field is the encoding
        }
        h
    } else {
        // f16 subnormal: value = m_half * 2^-24, so the implicit-one
        // f32 mantissa shifts right by (-unbiased - 1).
        let shift = (-unbiased - 1) as u32; // 14..=24
        let full = mant | 0x0080_0000;
        let m = full >> shift;
        let round = full & ((1 << shift) - 1);
        let mut h = sign | m as u16;
        let half = 1u32 << (shift - 1);
        if round > half || (round == half && (m & 1) == 1) {
            h += 1;
        }
        h
    }
}

/// Decode IEEE 754 binary16 bits back to f32 — exact for every f16
/// value (all halves are f32-representable); subnormals normalize.
pub(crate) fn f16_bits_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as i32;
    let mant = (h & 0x03ff) as u32;
    if exp == 0x1f {
        f32::from_bits(sign | 0x7f80_0000 | (mant << 13))
    } else if exp == 0 {
        if mant == 0 {
            f32::from_bits(sign)
        } else {
            // Subnormal half: value = mant * 2^-24. Normalize the
            // mantissa into [0x400, 0x800): value = 1.m' * 2^(-14-s),
            // so the f32 exponent field is 127 - 14 - s = 113 - s.
            let mut s = 0u32;
            let mut m = mant;
            while m & 0x0400 == 0 {
                m <<= 1;
                s += 1;
            }
            m &= 0x03ff;
            f32::from_bits(sign | ((113 - s) << 23) | (m << 13))
        }
    } else {
        f32::from_bits(sign | (((exp - 15 + 127) as u32) << 23) | (mant << 13))
    }
}

/// The f16 master's 8-bit reading (#140's interim landing; #141 made it
/// pub(crate) as the direct-read seams' ONLY quantize-read point — the
/// dispatch in [`master_pixel_bgra8`] routes every master-byte consumer
/// through it): sRGB gamma code = round-half-up(v * 255), clamped to
/// [0, 255]. L1's gamma arm keeps values in [0,1], but the read stays
/// defined for ANY half (out-of-range clamps — the same reading the
/// composite and dump channels would make of it).
pub(crate) fn f16_bits_to_u8_code(h: u16) -> u8 {
    let scaled = f16_bits_to_f32(h) * 255.0;
    if scaled <= 0.0 {
        0
    } else if scaled >= 255.0 {
        255
    } else {
        (scaled + 0.5) as u8
    }
}

/// One 16-bit code-value sample (full scale 65535) through the f16
/// master encoding: u16 -> f32 -> f16 -> 8-bit reading.
fn u16_code_to_u8_via_f16(code: u16) -> u8 {
    f16_bits_to_u8_code(f32_to_f16_bits(f32::from(code) / 65535.0))
}

/// The >8-bit `DynamicImage` sample layouts the direct path converts
/// (#140): 16-bit code-value samples. The 32-bit float (HDR radiance)
/// variants stay out — radiance is linear light, not code values; the
/// crate's tonemapped `into_rgba8()` remains their path.
pub(crate) enum DeepSamples<'a> {
    Rgb16(&'a [u16]),
    Rgba16(&'a [u16]),
    Luma16(&'a [u16]),
    LumaA16(&'a [u16]),
}

/// Convert one frame of 16-bit samples to RGBA8 through the f16 master
/// encoding (ADR 0003 D6's direct path, no mscms): u16 -> f32 (code /
/// 65535) -> f16 (the master's own quantization, D2) -> 8-bit reading.
/// Until #141 lands the f16 master storage this is the interim 8-bit
/// landing — the conversion semantics are the master's, so the bytes
/// the later tickets carry downstream never change.
pub(crate) fn deep_to_rgba8_via_f16(samples: DeepSamples<'_>, dst: &mut [u8]) {
    let (pixels, tail) = dst.as_chunks_mut::<4>();
    debug_assert!(tail.is_empty(), "dst must hold exactly 4 bytes per pixel");
    match samples {
        DeepSamples::Rgb16(src) => {
            debug_assert_eq!(src.len(), pixels.len() * 3);
            for (px, s) in pixels.iter_mut().zip(src.as_chunks::<3>().0) {
                *px = [
                    u16_code_to_u8_via_f16(s[0]),
                    u16_code_to_u8_via_f16(s[1]),
                    u16_code_to_u8_via_f16(s[2]),
                    255,
                ];
            }
        }
        DeepSamples::Rgba16(src) => {
            debug_assert_eq!(src.len(), pixels.len() * 4);
            for (px, s) in pixels.iter_mut().zip(src.as_chunks::<4>().0) {
                *px = [
                    u16_code_to_u8_via_f16(s[0]),
                    u16_code_to_u8_via_f16(s[1]),
                    u16_code_to_u8_via_f16(s[2]),
                    u16_code_to_u8_via_f16(s[3]),
                ];
            }
        }
        DeepSamples::Luma16(src) => {
            debug_assert_eq!(src.len(), pixels.len());
            for (px, s) in pixels.iter_mut().zip(src) {
                let y = u16_code_to_u8_via_f16(*s);
                *px = [y, y, y, 255];
            }
        }
        DeepSamples::LumaA16(src) => {
            debug_assert_eq!(src.len(), pixels.len() * 2);
            for (px, s) in pixels.iter_mut().zip(src.as_chunks::<2>().0) {
                let y = u16_code_to_u8_via_f16(s[0]);
                *px = [y, y, y, u16_code_to_u8_via_f16(s[1])];
            }
        }
    }
}

/// The decoded frame as pure memory (#76, ADR 0002 D3): top-down 32bpp
/// BGRA, exactly `width * height * 4` bytes, alpha forced opaque (the
/// invariant `composite_over_background_in_place` and the clipboard DIB
/// parser's three copy paths pin). This is the source of truth — the
/// D2D upload reads it, and device-loss recovery (#80) re-uploads from
/// these bytes without re-decoding (the #76-era GDI face derived from
/// them too, until #90 deleted the arm).
#[derive(Debug)]
pub(crate) struct PixelFrame {
    pub(crate) pixels: Box<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Which pipeline owns the frame (#140, ADR 0003 D1): plain 8-bit
    /// sRGB BGRA (`Srgb`, the 8-bit era's only space) or the FP16
    /// master's interim 8-bit reading (`F16Srgb` — the BYTES stay
    /// BGRA8 until #141 lands the f16 storage; the mark is the
    /// master-content gate's verdict, carried so the cache-key and
    /// seam-dispatch consumers wire up without re-deriving it).
    pub(crate) content_space: ContentSpace,
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
    pub(crate) fn from_rgba(
        width: u32,
        height: u32,
        mut rgba: Vec<u8>,
        content_space: ContentSpace,
    ) -> Self {
        debug_assert_eq!(rgba.len(), width as usize * height as usize * 4);
        rgba8_to_bgra_in_place(&mut rgba);
        Self::from_bgra(width, height, rgba, content_space)
    }

    /// The BGRA-native entry (the clipboard DIB parser already emits
    /// this layout, #66): boxed as-is, no swizzle pass.
    pub(crate) fn from_bgra(
        width: u32,
        height: u32,
        bgra: Vec<u8>,
        content_space: ContentSpace,
    ) -> Self {
        debug_assert_eq!(bgra.len(), width as usize * height as usize * 4);
        PixelFrame {
            pixels: bgra.into_boxed_slice(),
            width,
            height,
            content_space,
        }
    }

    /// The pixel dimensions (the load protocol's test surface for frame
    /// payloads; production reads them through the `Surface` wrapper).
    #[cfg(test)]
    pub(crate) fn dims(&self) -> (i32, i32) {
        (self.width as i32, self.height as i32)
    }
}

// ---------------------------------------------------------------------
// Direct-read seams (#141, ADR 0003 D1): the master-byte consumers that
// bypass the GPU pipeline — the status RGB readout, the clipboard image
// copy — all read through ONE dispatch so the f16 era's quantize-read
// semantics live in exactly one place.
// ---------------------------------------------------------------------

/// The direct-read seams' single dispatch (#141, ADR 0003 D1): one BGRA
/// pixel of the master as the 8-bit BGRA its GDI-era consumers show.
///
/// `Srgb` passes the master's own bytes through — the 8-bit era's byte
/// path, untouched. `F16Srgb` reads through the master's own quantizer
/// ([`f16_bits_to_u8_code`]). While the interim seam stands
/// (`loader.rs`'s "Interim seam": #142 has not landed the f16 storage
/// yet), an `F16Srgb` master's bytes ARE the already-quantized 8-bit
/// reading, and re-reading them through the quantizer — f16(k/255) ->
/// round(v * 255) — is the exact identity (probe 1's lossless 256-code
/// round trip, pinned in the tests). Both arms therefore return the same
/// bytes today: the dispatch is STRUCTURALLY in place for #142's stored
/// halves — when they land, only this arm's input changes from the
/// interim byte to the half bits — not yet behaviorally visible.
fn master_pixel_bgra8(space: ContentSpace, px: [u8; 4]) -> [u8; 4] {
    match space {
        ContentSpace::Srgb => px,
        ContentSpace::F16Srgb => {
            px.map(|b| f16_bits_to_u8_code(f32_to_f16_bits(f32::from(b) / 255.0)))
        }
    }
}

/// One pixel of a top-down tightly-packed BGRA buffer as its raw
/// [B, G, R, A] quadruple — the shared shape of [`sample_bgra`] and
/// [`sample_master_rgb`]. `None` for any out-of-bounds coordinate (the
/// same bounds [`sample_bgra`]'s doc pins).
fn sample_bgra_pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x >= width {
        return None;
    }
    let idx = (y as usize * width as usize + x as usize) * 4;
    let px = pixels.get(idx..idx + 4)?;
    Some([px[0], px[1], px[2], px[3]])
}

/// Read one pixel of a top-down tightly-packed BGRA buffer as an
/// (R, G, B) triple (#76; replaces the #47 status readout's `GetPixel`
/// on the frame's memory DC). `width` is the buffer's pixel width (the
/// row stride); `None` for any out-of-bounds coordinate — the caller
/// maps that to the `CLR_INVALID` read-through (255, 255, 255) the
/// unchecked GDI path produced, preserving the failure arm byte for
/// byte. The production read (#141) is [`sample_master_rgb`]'s
/// dispatched entry; this raw form stays the test surface for the
/// sampling bounds and the Srgb arm's byte path.
#[cfg(test)]
pub(crate) fn sample_bgra(pixels: &[u8], width: i32, x: i32, y: i32) -> Option<(u8, u8, u8)> {
    let [b, g, r, _] = sample_bgra_pixel(pixels, width, x, y)?;
    Some((r, g, b))
}

/// The status RGB readout's sampling entry (#141; the R2 contract's
/// "sRGB-normalized reading" — transform_stage.rs's contract block,
/// surface.rs and text.rs all state it the same way): one pixel of the
/// MASTER frame as the (R, G, B) triple the status bar shows, dispatched
/// on the master's content space like every direct read. `None` for
/// out-of-bounds coordinates — the caller maps it to the (255, 255, 255)
/// `CLR_INVALID` read-through, exactly as the raw [`sample_bgra`] did.
pub(crate) fn sample_master_rgb(frame: &PixelFrame, x: i32, y: i32) -> Option<(u8, u8, u8)> {
    let px = sample_bgra_pixel(&frame.pixels, frame.width as i32, x, y)?;
    let [b, g, r, _] = master_pixel_bgra8(frame.content_space, px);
    Some((r, g, b))
}

/// The clipboard image copy's source bytes (#141; the ADR 0003
/// 后果节 arm of the dispatch — GDI has no f16, so the CF_BITMAP copy
/// chain (`clipboard.rs`'s `set_clipboard_image` feed) reads the whole
/// master as the 8-bit top-down BGRA a GDI bitmap carries, the same
/// shape the 8-bit era produced). The `Srgb` arm is the plain clone the
/// copy always made; the `F16Srgb` arm reads every pixel through
/// [`master_pixel_bgra8`], which returns the same bytes on the interim
/// master (see there for why both arms coincide until #142).
pub(crate) fn master_gdi_bgra(frame: &PixelFrame) -> Vec<u8> {
    match frame.content_space {
        ContentSpace::Srgb => frame.pixels.to_vec(),
        ContentSpace::F16Srgb => {
            debug_assert_eq!(
                frame.pixels.len() % 4,
                0,
                "the master holds exactly 4 bytes per pixel"
            );
            let mut out = Vec::with_capacity(frame.pixels.len());
            for px in frame.pixels.as_chunks::<4>().0 {
                out.extend_from_slice(&master_pixel_bgra8(frame.content_space, *px));
            }
            out
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
    fn bgra_to_rgba_swizzles_rows_and_keeps_alpha() {
        // The dump swizzle: [B,G,R,A] rows become [R,G,B,A]; alpha rides
        // along untouched (both dump arms render opaque frames).
        let src = vec![30, 20, 10, 255, 3, 2, 1, 128];
        let mut dst = vec![0u8; src.len()];
        bgra_to_rgba(&src, &mut dst);
        assert_eq!(dst, vec![10, 20, 30, 255, 1, 2, 3, 128]);
    }

    #[test]
    fn bgra_to_rgba_is_the_in_place_swatch_inverse() {
        // Chaining the two swizzles restores any pixel row byte for byte
        // (the golden A/B compare leans on this symmetry).
        let src = vec![7u8, 8, 9, 250, 1, 2, 3, 255];
        let mut mid = vec![0u8; src.len()];
        bgra_to_rgba(&src, &mut mid);
        rgba8_to_bgra_in_place(&mut mid);
        assert_eq!(mid, src);
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
    fn the_bgra_composite_maps_the_background_to_the_swizzled_channels() {
        // Same blend as the RGBA version, but the buffer is [B,G,R,A] and
        // the background must land on the right channels (#77 ICM path).
        let mut px = vec![50, 100, 200, 0]; // BGRA: B=50 G=100 R=200
        composite_over_background_bgra_in_place(&mut px, [10, 20, 30]);
        assert_eq!(px, vec![30, 20, 10, 255], "bg [R,G,B]=[10,20,30]");
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
        let frame = PixelFrame::from_rgba(
            2,
            1,
            vec![10, 20, 30, 255, 1, 2, 3, 255],
            ContentSpace::Srgb,
        );
        assert_eq!(&frame.pixels[..], &[30, 20, 10, 255, 3, 2, 1, 255]);
    }

    #[test]
    fn pixelframe_from_bgra_keeps_bytes_and_dimensions() {
        // The BGRA-native entry boxes the bytes as-is; the dimensions
        // ride along for the load protocol's test surface.
        let frame =
            PixelFrame::from_bgra(1, 2, vec![1, 2, 3, 255, 4, 5, 6, 255], ContentSpace::Srgb);
        assert_eq!(&frame.pixels[..], &[1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(frame.dims(), (1, 2));
    }

    #[test]
    fn pixelframe_stride_is_width_times_four() {
        let frame = PixelFrame::from_bgra(7, 3, vec![0u8; 7 * 3 * 4], ContentSpace::Srgb);
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

    // ---- FP16 conversion (#140, ADR 0003 D2; #137 probe 1 landed) ----

    #[test]
    fn f16_conversion_matches_ieee_reference_points() {
        let cases: &[(f32, u16)] = &[
            (0.0, 0x0000),
            (-0.0, 0x8000),
            (1.0, 0x3c00),
            (-1.0, 0xbc00),
            (0.5, 0x3800),
            (0.25, 0x3400),
            (2.0, 0x4000),
            (65504.0, 0x7bff),            // largest finite half
            (1.0 / 16384.0, 0x0400),      // 2^-14, smallest normal half
            (1.0 / 16_777_216.0, 0x0001), // 2^-24, smallest subnormal half
        ];
        for (v, h) in cases {
            assert_eq!(f32_to_f16_bits(*v), *h, "f32_to_f16_bits({v})");
            assert_eq!(f16_bits_to_f32(*h), *v, "f16_bits_to_f32({h:04x})");
        }
        assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x7c00);
        assert_eq!(f32_to_f16_bits(65520.0), 0x7c00, "rounds up to infinity");
        assert!(f16_bits_to_f32(0x7c00).is_infinite());
        assert!(f16_bits_to_f32(0x7e00).is_nan());
    }

    #[test]
    fn srgb8_codes_round_trip_through_f16_losslessly() {
        // Probe 1's verdict, pinned: every one of the 256 sRGB code
        // values survives f32 -> f16 -> f32 (the whole premise of the
        // gamma-sRGB-f16 master — an 8-bit image loses NOTHING).
        for k in 0u8..=255 {
            let v = f32::from(k) / 255.0;
            let back = f16_bits_to_f32(f32_to_f16_bits(v));
            let k2 = (back * 255.0).round();
            assert_eq!(k2 as u8, k, "code {k} came back as {k2}");
        }
    }

    #[test]
    fn f16_round_trip_error_stays_under_half_an_srgb_step() {
        // The probe measured max 2.432e-4 at code 239 — an eighth of a
        // half-step; the pin is the half-step bound itself, so a future
        // conversion change fails loudly before it can cost a code.
        let mut max_err = 0f32;
        for k in 0u8..=255 {
            let v = f32::from(k) / 255.0;
            let back = f16_bits_to_f32(f32_to_f16_bits(v));
            max_err = max_err.max((back - v).abs());
        }
        assert!(
            max_err < 0.5 / 255.0,
            "max error {max_err:.3e} must stay under half an 8-bit step {:.3e}",
            0.5 / 255.0
        );
    }

    #[test]
    fn sixteen_bit_codes_read_back_through_the_f16_master() {
        // Endpoints and the loader's 16-bit PNG fixture values (the
        // apng_16bit test's 0xC700/0x3C00/0x0A00 triple): the f16 master
        // reads them back as the same 8-bit codes the old direct
        // quantization produced — the interim landing is byte-stable on
        // real fixture data.
        assert_eq!(u16_code_to_u8_via_f16(0x0000), 0);
        assert_eq!(u16_code_to_u8_via_f16(0xFFFF), 255);
        assert_eq!(
            u16_code_to_u8_via_f16(0x8000),
            128,
            "0.5000076 -> f16 0.5 -> 127.5 rounds up"
        );
        assert_eq!(u16_code_to_u8_via_f16(0xC700), 198);
        assert_eq!(u16_code_to_u8_via_f16(0x3C00), 60);
        assert_eq!(u16_code_to_u8_via_f16(0x0A00), 10);
    }

    #[test]
    fn deep_rgb16_expands_with_opaque_alpha() {
        let src = [0x0000u16, 0x8000, 0xFFFF];
        let mut dst = [0u8; 4];
        deep_to_rgba8_via_f16(DeepSamples::Rgb16(&src), &mut dst);
        assert_eq!(dst, [0, 128, 255, 255]);
    }

    #[test]
    fn deep_rgba16_carries_alpha_through_the_f16_master() {
        let src = [0xFFFFu16, 0x0000, 0x8000, 0x4000];
        let mut dst = [0u8; 4];
        deep_to_rgba8_via_f16(DeepSamples::Rgba16(&src), &mut dst);
        // 0x4000 = 16384/65535 = 0.2500076 -> f16 0.25 -> 63.75 -> 64.
        assert_eq!(dst, [255, 0, 128, 64]);
    }

    #[test]
    fn deep_luma16_replicates_the_code_across_rgb() {
        let src = [0xC700u16, 0xFFFF];
        let mut dst = [0u8; 8];
        deep_to_rgba8_via_f16(DeepSamples::Luma16(&src), &mut dst);
        assert_eq!(dst, [198, 198, 198, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn deep_luma_a16_replicates_and_carries_alpha() {
        let src = [0x8000u16, 0xFFFF, 0x0000, 0x0000];
        let mut dst = [0u8; 8];
        deep_to_rgba8_via_f16(DeepSamples::LumaA16(&src), &mut dst);
        assert_eq!(dst, [128, 128, 128, 255, 0, 0, 0, 0]);
    }

    // ---- direct-read seams (#141, ADR 0003 D1) ----

    #[test]
    fn requantizing_the_interim_f16srgb_byte_is_the_exact_identity() {
        // The interim seam's load-bearing fact: an F16Srgb master's bytes
        // are the ALREADY-quantized 8-bit reading (loader.rs's Interim
        // seam — #142 has not landed the f16 storage), and reading one
        // back through the master's own quantizer — f16(k/255) ->
        // round(v * 255), the exact math `master_pixel_bgra8`'s F16Srgb
        // arm runs — gives back the same code for every one of the 256
        // inputs. When #142 swaps the arm's input to stored halves this
        // identity stops being exercised, but until then it is what keeps
        // the two dispatch arms byte-identical.
        for k in 0u8..=255 {
            let read = f16_bits_to_u8_code(f32_to_f16_bits(f32::from(k) / 255.0));
            assert_eq!(read, k, "interim byte {k} must re-read as {k}");
        }
    }

    #[test]
    fn sample_master_rgb_matches_the_direct_sample_on_a_pure_srgb_master() {
        // Zero-regression pin for the Srgb arm: the status readout's
        // dispatched entry reads EXACTLY what the raw sample read before
        // #141 — including the out-of-bounds None the caller maps to the
        // CLR_INVALID white.
        let frame = PixelFrame::from_bgra(
            3,
            2,
            vec![
                1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17,
                18, 255,
            ],
            ContentSpace::Srgb,
        );
        for (x, y) in [(0i32, 0i32), (2, 0), (0, 1), (2, 1), (1, 1)] {
            assert_eq!(
                sample_master_rgb(&frame, x, y),
                sample_bgra(&frame.pixels, frame.width as i32, x, y),
                "({x}, {y}) must not move"
            );
        }
        assert_eq!(sample_master_rgb(&frame, 3, 0), None);
        assert_eq!(sample_master_rgb(&frame, -1, 0), None);
        assert_eq!(sample_master_rgb(&frame, 0, 2), None);
    }

    #[test]
    fn an_f16srgb_marked_master_reads_the_same_pixels_the_interim_bytes_carry() {
        // The dispatch's structural guarantee, observed at the status
        // readout: while the interim seam stands, a frame marked F16Srgb
        // (the #140 loader's mark for deep/transformed sources) reads
        // back the same RGB the bytes carry — the two content spaces'
        // reads coincide until #142's stored halves arrive.
        let bytes = vec![30, 20, 10, 255, 3, 2, 1, 255];
        let srgb = PixelFrame::from_bgra(2, 1, bytes.clone(), ContentSpace::Srgb);
        let f16 = PixelFrame::from_bgra(2, 1, bytes, ContentSpace::F16Srgb);
        for (x, y) in [(0i32, 0i32), (1, 0)] {
            assert_eq!(
                sample_master_rgb(&f16, x, y),
                sample_master_rgb(&srgb, x, y),
                "({x}, {y}): the F16Srgb mark must not change the interim read"
            );
        }
        // And the byte path underneath: every pixel of the marked frame
        // re-reads as its own bytes.
        for px in f16.pixels.as_chunks::<4>().0 {
            assert_eq!(master_pixel_bgra8(ContentSpace::F16Srgb, *px), *px);
        }
    }

    #[test]
    fn the_clipboard_source_of_an_f16srgb_master_matches_its_interim_bytes() {
        // The clipboard seam (ADR 0003 后果节: GDI has no f16 — the copy
        // quantizes back to 8-bit, "与现产字节同形"): on the interim
        // master the quantize read returns the master's own bytes, so the
        // F16Srgb source equals both the bytes and the Srgb clone the
        // 8-bit era produced.
        let bytes = vec![7u8, 8, 9, 255, 200, 150, 100, 255];
        let srgb = PixelFrame::from_bgra(2, 1, bytes.clone(), ContentSpace::Srgb);
        let f16 = PixelFrame::from_bgra(2, 1, bytes, ContentSpace::F16Srgb);
        assert_eq!(master_gdi_bgra(&f16), f16.pixels.to_vec());
        assert_eq!(master_gdi_bgra(&f16), master_gdi_bgra(&srgb));
    }
}
