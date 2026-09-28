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
//!
//! #142 (ADR 0003 D2/D6) gives the frame two storage layouts under the
//! content-space mark: the 8-bit era's BGRA bytes (`Srgb`) and the f16
//! master's four little-endian RGBA half quadruples per pixel
//! (`F16Srgb`) — plus the CPU transcoding between the CMM's 16-bit
//! output and that storage.

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

/// The shared rotation core (`bpp` bytes per pixel — the 4-byte BGRA arms
/// and #142's 8-byte f16 arm share one geometry; the pixel moves as an
/// opaque `bpp`-byte block, so the encoding rides along untouched).
fn rotate_90_cw_block(src: &[u8], wide: usize, high: usize, bpp: usize, dst: &mut [u8]) {
    debug_assert_eq!(src.len(), wide * high * bpp);
    debug_assert_eq!(dst.len(), wide * high * bpp);
    for y in 0..wide {
        for x in 0..high {
            let src_idx = (y + (high - x - 1) * wide) * bpp;
            let dst_idx = (x + y * high) * bpp;
            dst[dst_idx..dst_idx + bpp].copy_from_slice(&src[src_idx..src_idx + bpp]);
        }
    }
}

/// The counterclockwise sibling of [`rotate_90_cw_block`] (see the public
/// arms for the mapping's provenance).
fn rotate_270_cw_block(src: &[u8], wide: usize, high: usize, bpp: usize, dst: &mut [u8]) {
    debug_assert_eq!(src.len(), wide * high * bpp);
    debug_assert_eq!(dst.len(), wide * high * bpp);
    for y in 0..wide {
        for x in 0..high {
            let src_idx = ((wide - y - 1) + x * wide) * bpp;
            let dst_idx = (x + y * high) * bpp;
            dst[dst_idx..dst_idx + bpp].copy_from_slice(&src[src_idx..src_idx + bpp]);
        }
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
    rotate_90_cw_block(src, wide, high, 4, dst);
}

/// Rotate 90° counterclockwise — upstream orientation 8 (viv.c:13841-13850,
/// Edit → Rotate Counterclockwise): `new[x + y * high] = old[(wide - 1 - y)
/// + x * wide]`, i.e. old(row r, col c) lands at new(col `r`, row
/// `wide - 1 - c`) — the top-left corner moves to the bottom-left.
pub(crate) fn rotate_bgra_270_cw(src: &[u8], wide: usize, high: usize, dst: &mut [u8]) {
    rotate_270_cw_block(src, wide, high, 4, dst);
}

/// The f16 master's rotation arms (#142): upstream's verbatim geometry at
/// 8 bytes per pixel — each stored half quadruple moves as one opaque
/// block, so halves keep their bits and only their coordinates change.
pub(crate) fn rotate_f16_90_cw(src: &[u8], wide: usize, high: usize, dst: &mut [u8]) {
    rotate_90_cw_block(src, wide, high, 8, dst);
}

/// The f16 counterclockwise sibling of [`rotate_f16_90_cw`].
pub(crate) fn rotate_f16_270_cw(src: &[u8], wide: usize, high: usize, dst: &mut [u8]) {
    rotate_270_cw_block(src, wide, high, 8, dst);
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

/// The f16 master's 8-bit reading — the direct-read seams' ONLY
/// quantize-read point (#141 made it pub(crate); every master-byte
/// consumer routes through it, directly or via
/// [`f16_rgba_halves_to_bgra8`]): sRGB gamma code = round-half-up(v *
/// 255), clamped to [0, 255]. L1's gamma arm keeps values in [0,1], but
/// the read stays defined for ANY half (out-of-range clamps — the same
/// reading the composite and dump channels would make of it).
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

/// The f16 encoding of 1.0 (probe 1's IEEE reference point) — the master's
/// opaque alpha: the composite's skip value, the `BM_16b_RGB` output
/// format's alpha default, and the f16 alpha every opaque deep layout
/// lands on. Pinned by the reference-point test.
pub(crate) const F16_OPAQUE: u16 = 0x3C00;

/// One 16-bit code-value sample (full scale 65535) as an f16 half: the
/// master's own quantization (u16 -> f32 -> f16, ADR 0003 D2).
fn u16_code_to_f16(code: u16) -> u16 {
    f32_to_f16_bits(f32::from(code) / 65535.0)
}

/// One 16-bit code-value sample (full scale 65535) through the f16
/// master encoding: u16 -> f32 -> f16 -> 8-bit reading.
fn u16_code_to_u8_via_f16(code: u16) -> u8 {
    f16_bits_to_u8_code(u16_code_to_f16(code))
}

/// The >8-bit `DynamicImage` sample layouts the direct path converts
/// (#140): 16-bit code-value samples. The 32-bit float (HDR radiance)
/// variants stay out — radiance is linear light, not code values; the
/// crate's tonemapped `into_rgba8()` remains their path.
#[derive(Clone, Copy)]
pub(crate) enum DeepSamples<'a> {
    Rgb16(&'a [u16]),
    Rgba16(&'a [u16]),
    Luma16(&'a [u16]),
    LumaA16(&'a [u16]),
}

/// Convert one frame of 16-bit samples to RGBA8 through the f16 master
/// encoding: u16 -> f32 (code / 65535) -> f16 -> 8-bit reading. Since #142
/// this is the FALLBACK path's source quantizer (ADR 0003 D5): a deep
/// frame whose `BM_16b_RGB` transform failed quantizes through it into
/// the 8-bit chain — the primary deep paths land in the master's own
/// halves via [`deep_to_f16_halves`] instead.
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

/// The deep layouts' own alpha samples, one per pixel (`None` for the
/// opaque layouts) — what the transform path's alpha restore needs after
/// [`deep_to_bgr_u16`] dropped it (the `BM_16b_RGB` format has no alpha
/// channel to carry it through the CMM).
pub(crate) fn deep_source_alpha(samples: &DeepSamples<'_>) -> Option<Vec<u16>> {
    match samples {
        DeepSamples::Rgb16(_) | DeepSamples::Luma16(_) => None,
        DeepSamples::Rgba16(src) => Some(src.as_chunks::<4>().0.iter().map(|p| p[3]).collect()),
        DeepSamples::LumaA16(src) => Some(src.as_chunks::<2>().0.iter().map(|p| p[1]).collect()),
    }
}

/// The direct deep path's landing (#142, ADR 0003 D6): 16-bit code-value
/// samples straight into the f16 master's RGBA half storage — u16 -> f32
/// (code / 65535) -> f16, no 8-bit detour at any point. Alpha: the
/// layouts that carry it (Rgba16/LumaA16) convert their own sample, the
/// opaque ones land at the composite default ([`F16_OPAQUE`]).
pub(crate) fn deep_to_f16_halves(samples: DeepSamples<'_>, dst: &mut [u16]) {
    let (px, tail) = dst.as_chunks_mut::<4>();
    debug_assert!(tail.is_empty(), "dst must hold exactly 4 halves per pixel");
    match samples {
        DeepSamples::Rgb16(src) => {
            debug_assert_eq!(src.len(), px.len() * 3);
            for (p, s) in px.iter_mut().zip(src.as_chunks::<3>().0) {
                *p = [
                    u16_code_to_f16(s[0]),
                    u16_code_to_f16(s[1]),
                    u16_code_to_f16(s[2]),
                    F16_OPAQUE,
                ];
            }
        }
        DeepSamples::Rgba16(src) => {
            debug_assert_eq!(src.len(), px.len() * 4);
            for (p, s) in px.iter_mut().zip(src.as_chunks::<4>().0) {
                *p = [
                    u16_code_to_f16(s[0]),
                    u16_code_to_f16(s[1]),
                    u16_code_to_f16(s[2]),
                    u16_code_to_f16(s[3]),
                ];
            }
        }
        DeepSamples::Luma16(src) => {
            debug_assert_eq!(src.len(), px.len());
            for (p, &s) in px.iter_mut().zip(src) {
                let y = u16_code_to_f16(s);
                *p = [y, y, y, F16_OPAQUE];
            }
        }
        DeepSamples::LumaA16(src) => {
            debug_assert_eq!(src.len(), px.len() * 2);
            for (p, s) in px.iter_mut().zip(src.as_chunks::<2>().0) {
                let y = u16_code_to_f16(s[0]);
                *p = [y, y, y, u16_code_to_f16(s[1])];
            }
        }
    }
}

/// The deep source as the CMM's own input shape (ADR 0003 D6's 16-bit
/// transform path): per-pixel [B, G, R] u16 triplets — the decoder's RGB
/// sample order reversed into `BM_16b_RGB`'s BGR order, values copied
/// verbatim (the CMM consumes code values, not normalized floats). Alpha
/// is dropped here (the format has none); the caller restores it from
/// [`deep_source_alpha`] after the transform.
pub(crate) fn deep_to_bgr_u16(samples: DeepSamples<'_>, dst: &mut [u16]) {
    let (px, tail) = dst.as_chunks_mut::<3>();
    debug_assert!(tail.is_empty(), "dst must hold exactly 3 u16 per pixel");
    match samples {
        DeepSamples::Rgb16(src) => {
            debug_assert_eq!(src.len(), px.len() * 3);
            for (p, s) in px.iter_mut().zip(src.as_chunks::<3>().0) {
                *p = [s[2], s[1], s[0]];
            }
        }
        DeepSamples::Rgba16(src) => {
            debug_assert_eq!(src.len(), px.len() * 4);
            for (p, s) in px.iter_mut().zip(src.as_chunks::<4>().0) {
                *p = [s[2], s[1], s[0]];
            }
        }
        DeepSamples::Luma16(src) => {
            debug_assert_eq!(src.len(), px.len());
            for (p, &y) in px.iter_mut().zip(src) {
                *p = [y, y, y];
            }
        }
        DeepSamples::LumaA16(src) => {
            debug_assert_eq!(src.len(), px.len() * 2);
            for (p, s) in px.iter_mut().zip(src.as_chunks::<2>().0) {
                *p = [s[0], s[0], s[0]];
            }
        }
    }
}

/// The Stage-1 CMM's 16-bit gamma output (`BM_16b_RGB`: [B,G,R] u16
/// triplets, full scale 65535) into the f16 master's RGBA half layout
/// (ADR 0003 D6's CPU transcode — the CMM emits BGR order, the master
/// stores RGBA). One f16 per channel through the master's own encoding;
/// alpha is the format default [`F16_OPAQUE`] — restoring a source's
/// real alpha is the caller's job (the output format carries none).
pub(crate) fn bgr_u16_to_f16_rgba(src: &[u16], dst: &mut [u16]) {
    let (px, tail) = src.as_chunks::<3>();
    debug_assert!(tail.is_empty(), "src must hold exactly 3 u16 per pixel");
    let (out, out_tail) = dst.as_chunks_mut::<4>();
    debug_assert!(
        out_tail.is_empty(),
        "dst must hold exactly 4 halves per pixel"
    );
    debug_assert_eq!(px.len(), out.len(), "one BGR triplet per RGBA quadruple");
    for (d, s) in out.iter_mut().zip(px) {
        *d = [
            u16_code_to_f16(s[2]), // R — the CMM's BGR order reversed
            u16_code_to_f16(s[1]), // G
            u16_code_to_f16(s[0]), // B
            F16_OPAQUE,
        ];
    }
}

/// The f16 master's composite sibling of
/// [`composite_over_background_in_place`]: RGBA half pixels over `bg`
/// ([R, G, B] u8, DecodeEnv's shape — the half layout is RGBA, so no
/// swizzle like the BGRA sibling needs), upstream's blend formula in
/// float: `out = bg + (src - bg) * a`. Identity for a == 1.0 (the master's
/// composite default), applied unconditionally like the 8-bit one; alpha
/// lands at [`F16_OPAQUE`] after the pass (the master's opaque invariant).
pub(crate) fn composite_over_background_f16_in_place(halves: &mut [u16], bg: [u8; 3]) {
    let (px, tail) = halves.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "the f16 master holds exactly 4 halves per pixel"
    );
    let bg_f = bg.map(|b| f32::from(b) / 255.0);
    for px in px {
        let a = f16_bits_to_f32(px[3]);
        if a == 1.0 {
            continue;
        }
        for (c, b) in px[..3].iter_mut().zip(bg_f) {
            *c = f32_to_f16_bits(b + (f16_bits_to_f32(*c) - b) * a);
        }
        px[3] = F16_OPAQUE;
    }
}

/// Restore the source's 8-bit alpha into the f16 master's alpha halves:
/// the `BM_16b_RGB` output format carries no alpha channel, so a
/// transformed frame's halves arrive at the format default — a source
/// with real transparency needs its alpha back before the composite (the
/// same restore the 8-bit `apply` runs on the BGRA quads' x byte). One
/// alpha byte per pixel, `a / 255` through the master's own encoding.
pub(crate) fn restore_alpha_f16_from_u8(halves: &mut [u16], alpha: &[u8]) {
    let (px, tail) = halves.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "the f16 master holds exactly 4 halves per pixel"
    );
    debug_assert_eq!(px.len(), alpha.len(), "one alpha byte per pixel");
    for (px, &a) in px.iter_mut().zip(alpha) {
        px[3] = f32_to_f16_bits(f32::from(a) / 255.0);
    }
}

/// The 16-bit sibling of [`restore_alpha_f16_from_u8`]: one alpha u16
/// (full scale 65535) per pixel.
pub(crate) fn restore_alpha_f16_from_u16(halves: &mut [u16], alpha: &[u16]) {
    let (px, tail) = halves.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "the f16 master holds exactly 4 halves per pixel"
    );
    debug_assert_eq!(px.len(), alpha.len(), "one alpha sample per pixel");
    for (px, &a) in px.iter_mut().zip(alpha) {
        px[3] = u16_code_to_f16(a);
    }
}

/// The RGBA byte rows' alpha channel, one byte per pixel — the shape
/// [`restore_alpha_f16_from_u8`] consumes after an 8-bit decode fed the
/// 16-bit CMM chain (the interleaved rows themselves do not).
pub(crate) fn rgba8_alpha(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect()
}

/// The decoded frame as pure memory (#76, ADR 0002 D3): a top-down,
/// tightly packed pixel buffer whose layout the content space decides
/// (#142) — `Srgb` holds the 8-bit era's 32bpp BGRA (`width * height * 4`
/// bytes, alpha forced opaque), `F16Srgb` holds the f16 master's four
/// little-endian u16 halves per pixel in RGBA order (`width * height * 8`
/// bytes). This is the source of truth — the D2D upload reads it, and
/// device-loss recovery (#80) re-uploads from these bytes without
/// re-decoding (the #76-era GDI face derived from them too, until #90
/// deleted the arm).
#[derive(Debug)]
pub(crate) struct PixelFrame {
    pub(crate) pixels: Box<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Which pipeline owns the frame — and with it the storage convention
    /// of `pixels` (#140/#142, ADR 0003 D1/D2): `Srgb` = the 8-bit era's
    /// BGRA bytes; `F16Srgb` = the f16 master's RGBA halves (RGB carrying
    /// sRGB EOTF encoded values, gamma domain; alpha the composite default
    /// [`F16_OPAQUE`]). The byte buffer is NEVER reinterpreted as an
    /// aligned `&[u16]` — a `Box<[u8]>` is align-1 — every half goes
    /// through `u16::from_le_bytes`/`to_le_bytes`.
    pub(crate) content_space: ContentSpace,
}

impl PixelFrame {
    /// Row stride in bytes — the space's own invariant (`width * 4` for
    /// [`ContentSpace::Srgb`], `width * 8` for [`ContentSpace::F16Srgb`]
    /// and [`ContentSpace::F16P3`] — the wide container shares the f16
    /// halves layout, ADR 0004 D2), expressed as a method rather than a
    /// stored field so it can never drift from `width`. Test-only: the
    /// production seams branch on the content space, never on a stored
    /// stride.
    #[cfg(test)]
    pub(crate) fn stride(&self) -> usize {
        match self.content_space {
            ContentSpace::Srgb => self.width as usize * 4,
            ContentSpace::F16Srgb | ContentSpace::F16P3 => self.width as usize * 8,
        }
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
        debug_assert_eq!(
            content_space,
            ContentSpace::Srgb,
            "the byte constructors carry the 8-bit era's BGRA layout — an f16 master builds through from_f16_halves"
        );
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
        debug_assert_eq!(
            content_space,
            ContentSpace::Srgb,
            "the byte constructors carry the 8-bit era's BGRA layout — an f16 master builds through from_f16_halves"
        );
        PixelFrame {
            pixels: bgra.into_boxed_slice(),
            width,
            height,
            content_space,
        }
    }

    /// The f16 master's narrow entry (#142): `halves` holds exactly
    /// `width * height * 4` u16 values (four per pixel, RGBA order),
    /// packed little-endian into the byte buffer. Infallible like the
    /// byte constructors (an allocation failure is a process-level
    /// abort); the space is always [`ContentSpace::F16Srgb`] — these
    /// ARE the f16 master's own bytes. Wide-gamut halves (a Stage-1 P3
    /// destination, #155) enter through [`PixelFrame::from_f16_halves_wide`]
    /// instead.
    pub(crate) fn from_f16_halves(width: u32, height: u32, halves: Vec<u16>) -> Self {
        Self::from_f16_halves_in_space(width, height, halves, ContentSpace::F16Srgb)
    }

    /// The f16 master's WIDE entry (#155, ADR 0004 D2) — the ONLY
    /// production gate that mints an `F16P3` master: the loader's
    /// transform arm calls it when the ICC transform's destination is
    /// the P3 container, and the stored halves then carry P3-primary
    /// values in the destination gamma domain (8 bytes per pixel
    /// exactly like the narrow f16 master). Every other constructor is
    /// narrow by construction.
    pub(crate) fn from_f16_halves_wide(width: u32, height: u32, halves: Vec<u16>) -> Self {
        Self::from_f16_halves_in_space(width, height, halves, ContentSpace::F16P3)
    }

    /// The shared packing of both f16 entries — one copy of the
    /// little-endian half packing, the content space the only argument
    /// that differs.
    fn from_f16_halves_in_space(
        width: u32,
        height: u32,
        halves: Vec<u16>,
        content_space: ContentSpace,
    ) -> Self {
        debug_assert_eq!(
            halves.len(),
            width as usize * height as usize * 4,
            "four halves per pixel"
        );
        debug_assert_ne!(
            content_space,
            ContentSpace::Srgb,
            "the f16 constructors never mint the 8-bit era's BGRA layout"
        );
        let mut pixels = Vec::with_capacity(halves.len() * 2);
        for h in halves {
            pixels.extend_from_slice(&h.to_le_bytes());
        }
        PixelFrame {
            pixels: pixels.into_boxed_slice(),
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

/// The direct-read seams' single quantize read (#141/#142, ADR 0003 D1):
/// one pixel of the master as the 8-bit BGRA its GDI-era consumers show.
/// The two spaces store different byte widths (`Srgb` a BGRA quad,
/// `F16Srgb` a half quadruple), so each seam samples its own shape and
/// routes the F16Srgb arm through [`f16_rgba_halves_to_bgra8`] — whose
/// only quantize-read point is [`f16_bits_to_u8_code`]. The Srgb arm is
/// the byte passthrough the 8-bit era always was.
pub(crate) fn f16_rgba_halves_to_bgra8(px: [u16; 4]) -> [u8; 4] {
    [
        f16_bits_to_u8_code(px[2]), // B
        f16_bits_to_u8_code(px[1]), // G
        f16_bits_to_u8_code(px[0]), // R
        255,
    ]
}

/// The whole-frame bulk of [`f16_rgba_halves_to_bgra8`] (the clipboard
/// seam's quantize, #142): `pixels` holds the master's 8-byte-per-pixel
/// halves, `dst` the 4-byte-per-pixel BGRA reading. The upload seam this
/// bulk once served quantizes no more — #143 uploads the halves into an
/// `R16G16B16A16_FLOAT` bitmap byte-for-byte — so the remaining callers
/// are the clipboard's GDI-face copy (`master_gdi_bgra`, the one
/// direct-read seam with no f16 surface to hand the data to) and its
/// test.
pub(crate) fn f16_halves_to_bgra8_bulk(pixels: &[u8], dst: &mut [u8]) {
    let (src, src_tail) = pixels.as_chunks::<8>();
    debug_assert!(
        src_tail.is_empty(),
        "the f16 master holds exactly 8 bytes per pixel"
    );
    let (out, dst_tail) = dst.as_chunks_mut::<4>();
    debug_assert!(
        dst_tail.is_empty(),
        "dst must hold exactly 4 bytes per pixel"
    );
    debug_assert_eq!(src.len(), out.len(), "one half quadruple per BGRA quad");
    let halves = |b: &[u8; 8]| {
        [
            u16::from_le_bytes([b[0], b[1]]),
            u16::from_le_bytes([b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]]),
        ]
    };
    for (d, s) in out.iter_mut().zip(src) {
        *d = f16_rgba_halves_to_bgra8(halves(s));
    }
}

/// One pixel of a top-down tightly-packed BGRA buffer as its raw
/// [B, G, R, A] quadruple — the shared shape of [`sample_bgra`] and the
/// Srgb arm of [`sample_master_rgb`]. `None` for any out-of-bounds
/// coordinate (the same bounds [`sample_bgra`]'s doc pins).
fn sample_bgra_pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x >= width {
        return None;
    }
    let idx = (y as usize * width as usize + x as usize) * 4;
    let px = pixels.get(idx..idx + 4)?;
    Some([px[0], px[1], px[2], px[3]])
}

/// One pixel of a top-down tightly-packed f16 master buffer (8 bytes per
/// pixel: four LE u16 halves, RGBA order) as its RGBA half quadruple —
/// the F16Srgb arm's sampling shape. `None` for any out-of-bounds
/// coordinate (the same bounds [`sample_bgra_pixel`] pins).
fn sample_f16_pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> Option<[u16; 4]> {
    if x < 0 || y < 0 || x >= width {
        return None;
    }
    let idx = (y as usize * width as usize + x as usize) * 8;
    let b = pixels.get(idx..idx + 8)?;
    let half = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    Some([half(0), half(2), half(4), half(6)])
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
/// on the master's content space like every direct read — each arm
/// samples its own storage shape (`Srgb` the BGRA quad, `F16Srgb` the
/// half quadruple through [`f16_rgba_halves_to_bgra8`]). `None` for
/// out-of-bounds coordinates — the caller maps it to the (255, 255, 255)
/// `CLR_INVALID` read-through, exactly as the raw [`sample_bgra`] did.
pub(crate) fn sample_master_rgb(frame: &PixelFrame, x: i32, y: i32) -> Option<(u8, u8, u8)> {
    let [b, g, r, _] = match frame.content_space {
        ContentSpace::Srgb => sample_bgra_pixel(&frame.pixels, frame.width as i32, x, y)?,
        // #154 transitional arm: an F16P3 master's P3-encoded halves are
        // read as if they were sRGB code values — chromatically
        // dishonest. The honest P3→sRGB direct-read conversion is the
        // L2 direct-read seam ticket (#156, ADR 0004 impact item 4).
        // Unreachable at runtime before #155 produces an F16P3 master,
        // so there is nothing to observe; no output-byte pin is written
        // against this arm on purpose (never pin a lie).
        ContentSpace::F16Srgb | ContentSpace::F16P3 => {
            f16_rgba_halves_to_bgra8(sample_f16_pixel(&frame.pixels, frame.width as i32, x, y)?)
        }
    };
    Some((r, g, b))
}

/// The clipboard image copy's source bytes (#141; the ADR 0003
/// 后果节 arm of the dispatch — GDI has no f16, so the CF_BITMAP copy
/// chain (`clipboard.rs`'s `set_clipboard_image` feed) reads the whole
/// master as the 8-bit top-down BGRA a GDI bitmap carries, the same
/// shape the 8-bit era produced). The `Srgb` arm is the plain clone the
/// copy always made; the `F16Srgb` arm quantizes every stored half
/// quadruple through the bulk of [`f16_rgba_halves_to_bgra8`].
pub(crate) fn master_gdi_bgra(frame: &PixelFrame) -> Vec<u8> {
    match frame.content_space {
        ContentSpace::Srgb => frame.pixels.to_vec(),
        // #154 transitional arm (same story as [`sample_master_rgb`]):
        // an F16P3 master's P3-encoded halves quantize through the
        // sRGB-shaped arm — chromatically dishonest until #156's honest
        // P3→sRGB direct-read seam lands. Unreachable before #155
        // produces an F16P3 master; no output-byte pin against this arm
        // on purpose (never pin a lie).
        ContentSpace::F16Srgb | ContentSpace::F16P3 => {
            let mut out = vec![0u8; frame.pixels.len() / 2];
            f16_halves_to_bgra8_bulk(&frame.pixels, &mut out);
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
    fn pixelframe_stride_follows_the_content_space_pixel_width() {
        // #142: the stride is the space's own invariant — 4 bytes per
        // pixel for the Srgb BGRA era, 8 for the f16 halves — so a
        // buffer-length consumer can never size an f16 master like an
        // 8-bit one.
        let srgb = PixelFrame::from_bgra(7, 3, vec![0u8; 7 * 3 * 4], ContentSpace::Srgb);
        assert_eq!(srgb.stride(), 28);
        assert_eq!(srgb.pixels.len(), srgb.stride() * 3);
        let f16 = PixelFrame::from_f16_halves(7, 3, vec![F16_OPAQUE; 7 * 3 * 4]);
        assert_eq!(f16.stride(), 56);
        assert_eq!(f16.pixels.len(), f16.stride() * 3);
        assert_eq!(f16.content_space, ContentSpace::F16Srgb);
    }

    #[test]
    fn from_f16_halves_packs_the_halves_little_endian() {
        // The storage contract's byte view: each u16 half lands as its LE
        // byte pair, RGBA halves contiguous per pixel. Read back through
        // the seam's own LE decoder, not an aligned cast (Box<[u8]> is
        // align-1 by contract).
        let frame = PixelFrame::from_f16_halves(1, 1, vec![0x1234, 0xABCD, 0x0001, F16_OPAQUE]);
        assert_eq!(
            &frame.pixels[..],
            &[0x34, 0x12, 0xCD, 0xAB, 0x01, 0x00, 0x00, 0x3C]
        );
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
        // quantization produced — byte-stable on real fixture data.
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

    // ---- direct-read seams (#141/#142, ADR 0003 D1) ----

    /// An F16Srgb frame built from explicit halves: one white pixel
    /// (R=G=B=0x3C00) and one half-grey pixel (0x3800 = 0.5), alpha
    /// opaque. The seams' read-back pins below decode it.
    fn f16_frame_2x1() -> PixelFrame {
        PixelFrame::from_f16_halves(
            2,
            1,
            vec![
                0x3C00, 0x3C00, 0x3C00, F16_OPAQUE, 0x3800, 0x3800, 0x3800, F16_OPAQUE,
            ],
        )
    }

    #[test]
    fn sample_master_rgb_on_an_f16srgb_master_reads_the_stored_halves() {
        // The seam's #142 shape: an F16Srgb master stores half bits, and
        // the status readout reads THEM — pixel 0 through the white half
        // 0x3C00 (255 on every channel), pixel 1 through 0x3800 (0.5 ->
        // 127.5 -> 128, the round-half-up the quantizer pins). Both match
        // the per-channel f16_bits_to_u8_code expectation — the only
        // quantize read there is.
        let frame = f16_frame_2x1();
        assert_eq!(sample_master_rgb(&frame, 0, 0), Some((255, 255, 255)));
        assert_eq!(sample_master_rgb(&frame, 1, 0), Some((128, 128, 128)));
        // The Srgb arm's None-for-out-of-bounds contract holds on the
        // f16 arm too (8-byte pixels, same pixel bounds).
        assert_eq!(sample_master_rgb(&frame, 2, 0), None);
        assert_eq!(sample_master_rgb(&frame, -1, 0), None);
        assert_eq!(sample_master_rgb(&frame, 0, 1), None);
    }

    #[test]
    fn an_f16p3_master_samples_the_seams_like_the_f16srgb_one() {
        // #155's structural pin (NOT a color pin — the chromatic
        // honesty of the read is #156's debt, the transitional arms'
        // comments say so): the wide master shares the f16 storage
        // layout, so its direct-read seams run the same 8-byte stride
        // and the same None-for-out-of-bounds shape.
        let frame = PixelFrame::from_f16_halves_wide(
            2,
            1,
            vec![
                0x3C00, 0x3C00, 0x3C00, F16_OPAQUE, 0x3800, 0x3800, 0x3800, F16_OPAQUE,
            ],
        );
        assert_eq!(frame.content_space, ContentSpace::F16P3);
        assert_eq!(
            frame.stride(),
            2 * 8,
            "8 bytes per pixel, same as the narrow f16"
        );
        assert_eq!(sample_master_rgb(&frame, 0, 0), Some((255, 255, 255)));
        assert_eq!(sample_master_rgb(&frame, 1, 0), Some((128, 128, 128)));
        assert_eq!(sample_master_rgb(&frame, 2, 0), None);
        assert_eq!(sample_master_rgb(&frame, -1, 0), None);
        assert_eq!(sample_master_rgb(&frame, 0, 1), None);
    }

    #[test]
    fn the_two_f16_spaces_read_identically_at_the_seams_for_the_same_halves() {
        // The routing pin: the same batch of halves boxed as F16Srgb and
        // as F16P3 must produce BIT-IDENTICAL readings at both
        // direct-read seams — the wide arm routes through the ONE
        // quantize function (f16_rgba_halves_to_bgra8), never a second
        // quantizer. Deliberately NOT a color-correctness pin: both
        // readings are the transitional sRGB-shaped read (#156's debt).
        let halves = vec![
            0x3C00, 0x3C00, 0x3C00, F16_OPAQUE, 0x3800, 0x3800, 0x3800, F16_OPAQUE,
        ];
        let narrow = PixelFrame::from_f16_halves(2, 1, halves.clone());
        let wide = PixelFrame::from_f16_halves_wide(2, 1, halves);
        for (x, y) in [(0i32, 0i32), (1, 0)] {
            assert_eq!(
                sample_master_rgb(&narrow, x, y),
                sample_master_rgb(&wide, x, y),
                "the status read at ({x},{y}) is the shared quantize arm"
            );
        }
        assert_eq!(
            master_gdi_bgra(&narrow),
            master_gdi_bgra(&wide),
            "the clipboard read is the shared bulk quantize"
        );
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
    fn f16_half_pixels_quantize_through_the_one_read_point() {
        // The seam contract in its smallest form: f16_rgba_halves_to_bgra8
        // reverses the half order into BGRA bytes, each channel through
        // f16_bits_to_u8_code, alpha forced opaque.
        assert_eq!(
            f16_rgba_halves_to_bgra8([0x3C00, 0x3800, 0x0000, 0x3C00]),
            [0, 128, 255, 255]
        );
    }

    #[test]
    fn the_clipboard_source_of_an_f16srgb_master_is_the_bulk_quantize() {
        // The clipboard seam (ADR 0003 后果节: GDI has no f16 — the copy
        // quantizes back to 8-bit, "与现产字节同形"): the whole-frame read
        // equals the per-pixel seam read at every pixel — one quantize
        // path, two granularities.
        let frame = f16_frame_2x1();
        let bulk = master_gdi_bgra(&frame);
        assert_eq!(bulk.len(), frame.pixels.len() / 2, "4 bytes per pixel");
        for (x, px) in bulk.as_chunks::<4>().0.iter().enumerate() {
            let expected = f16_rgba_halves_to_bgra8(
                sample_f16_pixel(&frame.pixels, frame.width as i32, x as i32, 0)
                    .expect("in-bounds pixel"),
            );
            assert_eq!(*px, expected, "pixel {x}");
        }
        // The f16 bulk quantize of a white frame is the 8-bit era's white.
        let white = PixelFrame::from_f16_halves(
            2,
            1,
            vec![
                0x3C00, 0x3C00, 0x3C00, F16_OPAQUE, 0x3C00, 0x3C00, 0x3C00, F16_OPAQUE,
            ],
        );
        assert_eq!(
            master_gdi_bgra(&white),
            vec![255, 255, 255, 255, 255, 255, 255, 255]
        );
    }

    // ---- CPU transcode (#142, ADR 0003 D6) ----

    #[test]
    fn bgr_u16_to_f16_reverses_the_cmm_channel_order() {
        // The CMM's BM_16b_RGB emits [B, G, R]; the master stores [R, G,
        // B, A]. An asymmetric color is the discriminator — a symmetric
        // one (grey) could not tell reversal from a passthrough.
        let mut dst = [0u16; 4];
        bgr_u16_to_f16_rgba(&[0x0A00, 0x3C00, 0xC700], &mut dst);
        // R takes the third triplet slot (0xC700), B the first (0x0A00).
        assert_eq!(
            (dst[0], dst[1], dst[2], dst[3]),
            (
                f32_to_f16_bits(f32::from(0xC700u16) / 65535.0),
                f32_to_f16_bits(f32::from(0x3C00u16) / 65535.0),
                f32_to_f16_bits(f32::from(0x0A00u16) / 65535.0),
                F16_OPAQUE
            )
        );
    }

    #[test]
    fn bgr_u16_endpoints_and_midpoint_land_on_the_reference_halves() {
        // 0 -> black (0x0000), 65535 -> full (1.0 = 0x3C00), and the
        // 32768 case the loader's 16-bit fixture family leans on:
        // 0.5000076 is nearer 0.5 than the next half up, so it rounds to
        // 0x3800 — no code value drifts past the master's quantization.
        for (code, half) in [(0u16, 0x0000u16), (65535, 0x3C00), (32768, 0x3800)] {
            let mut dst = [0u16; 4];
            bgr_u16_to_f16_rgba(&[code, code, code], &mut dst);
            assert_eq!(dst[0], half, "R half of {code}");
            assert_eq!(dst[1], half, "G half of {code}");
            assert_eq!(dst[2], half, "B half of {code}");
        }
    }

    #[test]
    fn bgr_u16_to_f16_sets_alpha_opaque_and_tracks_lengths() {
        // The output format has no alpha channel: every pixel's A half is
        // the composite default, and the 3-to-4 halves-per-pixel ratio is
        // asserted (a caller mixing up u16 and byte counts cannot pass).
        let src = [0xFFFFu16; 9]; // three pixels
        let mut dst = [0u16; 12];
        bgr_u16_to_f16_rgba(&src, &mut dst);
        assert_eq!(dst[3], F16_OPAQUE);
        assert_eq!(dst[7], F16_OPAQUE);
        assert_eq!(dst[11], F16_OPAQUE);
    }

    #[test]
    fn the_f16_composite_lands_a_transparent_pixel_on_the_background() {
        // The M1 bug's f16 sibling: a=0 hides the source RGB entirely.
        let mut px = vec![0x3C00, 0x0000, 0x3800, 0x0000]; // RGBA halves, a=0
        composite_over_background_f16_in_place(&mut px, [10, 20, 30]);
        assert_eq!(
            (px[0], px[1], px[2], px[3]),
            (
                f32_to_f16_bits(10.0 / 255.0),
                f32_to_f16_bits(20.0 / 255.0),
                f32_to_f16_bits(30.0 / 255.0),
                F16_OPAQUE
            )
        );
    }

    #[test]
    fn the_f16_composite_leaves_opaque_pixels_untouched() {
        let src = vec![
            0x1234, 0x5678, 0x9ABC, F16_OPAQUE, 0x0000, 0x3C00, 0x0A00, F16_OPAQUE,
        ];
        let mut px = src.clone();
        composite_over_background_f16_in_place(&mut px, [200, 200, 200]);
        assert_eq!(px, src, "a == 1.0 is the composite's skip value");
    }

    #[test]
    fn the_f16_composite_stays_within_one_code_of_the_8bit_formula() {
        // The two composites must agree where both are defined: half
        // alpha over a midtone background, every channel within one 8-bit
        // code of upstream's integer blend `bg + (src - bg) * a / 255`
        // (viv.c:10166-10168). The f16 arm's extra precision may land one
        // code off the truncating integer result, never further.
        let (r8, g8, b8, a8) = (200u8, 60u8, 10u8, 128u8);
        let bg = [80u8, 80, 80];
        let src8 = [r8, g8, b8];
        let mut px = vec![
            f32_to_f16_bits(f32::from(r8) / 255.0),
            f32_to_f16_bits(f32::from(g8) / 255.0),
            f32_to_f16_bits(f32::from(b8) / 255.0),
            f32_to_f16_bits(f32::from(a8) / 255.0),
        ];
        composite_over_background_f16_in_place(&mut px, bg);
        for (i, &half) in px[..3].iter().enumerate() {
            let expected =
                i32::from(bg[i]) + ((i32::from(src8[i]) - i32::from(bg[i])) * i32::from(a8)) / 255;
            let read = f16_bits_to_u8_code(half);
            assert!(
                (i32::from(read) - expected).abs() <= 1,
                "channel {i}: f16 read {read} vs integer formula {expected}"
            );
        }
        assert_eq!(px[3], F16_OPAQUE);
    }

    #[test]
    fn restoring_8bit_alpha_encodes_the_same_fraction_the_8bit_era_carried() {
        // a/255 through the master's encoding: 128 -> f16(128/255) — read
        // back it is the same fraction, so the composite downstream sees
        // the transparency the decoder produced.
        let mut px = vec![0u16; 8];
        restore_alpha_f16_from_u8(&mut px, &[128, 0]);
        assert_eq!(px[3], f32_to_f16_bits(f32::from(128u8) / 255.0));
        assert_eq!(px[7], 0x0000, "a=0 stays fully transparent");
    }

    #[test]
    fn restoring_16bit_alpha_spans_the_full_scale() {
        // 0xFFFF -> 1.0 (0x3C00), 0x0000 -> 0: the endpoints the deep
        // fixture family leans on.
        let mut px = vec![0u16; 8];
        restore_alpha_f16_from_u16(&mut px, &[0xFFFF, 0x0000]);
        assert_eq!(px[3], F16_OPAQUE);
        assert_eq!(px[7], 0x0000);
    }

    #[test]
    fn deep_samples_land_in_f16_halves_without_an_8bit_detour() {
        // The direct deep path's storage: the fixture triple reads back
        // the same 8-bit codes the old quantization produced (198/60/10),
        // but now as stored HALVES — and 0xC700's half is NOT the f16 of
        // its 8-bit read (198/255), which is the >8-bit information the
        // old truncation destroyed.
        let src = [0xC700u16, 0x3C00, 0x0A00, 0xFFFF];
        let mut dst = [0u16; 4];
        deep_to_f16_halves(DeepSamples::Rgba16(&src), &mut dst);
        assert_eq!(f16_bits_to_u8_code(dst[0]), 198);
        assert_eq!(f16_bits_to_u8_code(dst[1]), 60);
        assert_eq!(f16_bits_to_u8_code(dst[2]), 10);
        assert_eq!(dst[3], F16_OPAQUE, "0xFFFF alpha -> opaque");
        assert_ne!(dst[0], f32_to_f16_bits(198.0 / 255.0));
    }

    #[test]
    fn deep_opaque_layouts_default_alpha_to_the_composite_value() {
        // Rgb16/Luma16 have no alpha samples: their halves land at the
        // format default, and luma replicates across RGB.
        let mut dst = [0u16; 8];
        deep_to_f16_halves(DeepSamples::Luma16(&[0x8000, 0xFFFF]), &mut dst);
        assert_eq!(dst[0], dst[1]);
        assert_eq!(dst[1], dst[2]);
        assert_eq!(dst[3], F16_OPAQUE);
        assert_eq!(dst[7], F16_OPAQUE);
    }

    #[test]
    fn deep_samples_feed_the_cmm_as_bgr_triplets() {
        // deep_to_bgr_u16 is the CMM's input builder: RGB sample order
        // reversed into BM_16b_RGB's BGR, values verbatim, alpha dropped
        // (restored separately), luma triplicated.
        let mut dst = [0u16; 3];
        deep_to_bgr_u16(DeepSamples::Rgb16(&[0x0A00, 0x3C00, 0xC700]), &mut dst);
        assert_eq!(dst, [0xC700, 0x3C00, 0x0A00], "B, G, R");
        let mut dst = [0u16; 3];
        deep_to_bgr_u16(
            DeepSamples::Rgba16(&[0x0A00, 0x3C00, 0xC700, 0x1234]),
            &mut dst,
        );
        assert_eq!(dst, [0xC700, 0x3C00, 0x0A00], "alpha dropped");
        let mut dst = [0u16; 6];
        deep_to_bgr_u16(
            DeepSamples::LumaA16(&[0x8000, 0xFFFF, 0x0000, 0x0000]),
            &mut dst,
        );
        assert_eq!(dst, [0x8000, 0x8000, 0x8000, 0x0000, 0x0000, 0x0000]);
    }

    #[test]
    fn deep_source_alpha_is_pulled_only_from_the_layouts_that_carry_it() {
        // The transform path's restore needs the samples' own alpha back:
        // Rgba16 takes slot 3, LumaA16 slot 1, the opaque layouts none.
        let rgba = [0xFFFFu16, 0, 0, 0x8000, 0, 0xFFFF, 0, 0x4000];
        assert_eq!(
            deep_source_alpha(&DeepSamples::Rgba16(&rgba)),
            Some(vec![0x8000, 0x4000])
        );
        let la = [0x1234u16, 0x8000, 0x5678, 0xFFFF];
        assert_eq!(
            deep_source_alpha(&DeepSamples::LumaA16(&la)),
            Some(vec![0x8000, 0xFFFF])
        );
        assert_eq!(deep_source_alpha(&DeepSamples::Rgb16(&[0; 3])), None);
        assert_eq!(deep_source_alpha(&DeepSamples::Luma16(&[0])), None);
    }

    #[test]
    fn the_f16_bulk_quantize_matches_the_per_pixel_seam_read() {
        // The upload seam's whole-frame staging (#142) must produce
        // exactly the pixels the per-pixel seam would read — one
        // quantize, no second opinion.
        let frame = f16_frame_2x1();
        let mut bulk = vec![0u8; frame.pixels.len() / 2];
        f16_halves_to_bgra8_bulk(&frame.pixels, &mut bulk);
        for x in 0..2i32 {
            let [b, g, r, a] = f16_rgba_halves_to_bgra8(
                sample_f16_pixel(&frame.pixels, frame.width as i32, x, 0).expect("in-bounds"),
            );
            assert_eq!(&bulk[x as usize * 4..x as usize * 4 + 4], &[b, g, r, a]);
        }
    }

    // ---- f16 rotation (#142) ----

    /// The 2x3 f16 twin of the byte ramp below: pixel i carries the half
    /// value 0x0400 + i in every channel (an index tag that survives as
    /// bits), alpha opaque.
    fn f16_ramp_2x3() -> Vec<u8> {
        let mut halves = Vec::with_capacity(2 * 3 * 4);
        for i in 0..6u16 {
            for _ in 0..4 {
                halves.push(0x0400 + i);
            }
        }
        let mut bytes = Vec::with_capacity(halves.len() * 2);
        for h in halves {
            bytes.extend_from_slice(&h.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn f16_rotation_moves_whole_half_quadruples_with_the_8bit_geometry() {
        // Same fixture shape as the BGRA rotation pin: the 2x3 f16 ramp
        // rotates into a 3x2 result with the pixel mapping unchanged —
        // old(row r, col c) -> new(col high-1-r, row c) — and the half
        // BITS ride along untouched (rotation is geometry, not encoding).
        let src = f16_ramp_2x3();
        let mut dst = vec![0u8; src.len()];
        rotate_f16_90_cw(&src, 2, 3, &mut dst);
        let px = |i: usize| -> u16 {
            // Read the moved pixel's first channel half back (LE).
            u16::from_le_bytes([dst[i * 8], dst[i * 8 + 1]])
        };
        // The 8-bit pin's mapping, at half granularity: new row 0 holds
        // old pixels 4, 2, 0; new row 1 holds 5, 3, 1.
        assert_eq!((px(0), px(1), px(2)), (0x0404, 0x0402, 0x0400));
        assert_eq!((px(3), px(4), px(5)), (0x0405, 0x0403, 0x0401));
    }

    #[test]
    fn f16_clockwise_and_counterclockwise_are_inverses() {
        let src = f16_ramp_2x3();
        let mut cw = vec![0u8; src.len()];
        rotate_f16_90_cw(&src, 2, 3, &mut cw);
        let mut back = vec![0u8; src.len()];
        rotate_f16_270_cw(&cw, 3, 2, &mut back);
        assert_eq!(back, src, "90 + 270 restores every half bit for bit");
    }
}
