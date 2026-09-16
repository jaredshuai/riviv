//! CF_DIB / CF_DIBV5 payload parsing (#66; upstream wishlist viv.c:105 —
//! "paste dib from clipboard CF_DIB"): one clipboard DIB → one top-down
//! BGRA frame, ready for `DibFrame::from_bgra`. Pure logic — the test net
//! synthesizes DIB bytes directly (the acceptance matrix: 24/32bpp,
//! top-down/bottom-up, BI_BITFIELDS masks, V5 headers).
//!
//! Scope is the clipboard's real-world shapes: 24/32bpp BI_RGB plus the
//! bitfield compressions (BI_BITFIELDS 3, BI_ALPHABITFIELDS 6) carrying
//! explicit channel masks, across the INFO/V2/V3/V4/V5 header sizes.
//! Everything else (1/4/8/16bpp, RLE, JPEG/PNG-embedded DIBs) fails
//! user-level — a CF_BITMAP of any depth still displays through the
//! GetDIBits conversion (see clipboard.rs), so old low-depth clipboard
//! bitmaps are not lost.
//!
//! Alpha semantics: the Windows convention leaves the 4th byte of a
//! BI_RGB 32bpp DIB undefined (screenshot tools commonly write 0), so it
//! renders opaque. Alpha is honored ONLY through an explicit alpha mask
//! (BI_ALPHABITFIELDS' fourth mask, or a V4/V5 header's in-header alpha
//! field), flattened against the windowed background exactly like the
//! decode pipeline's `composite_over_background_in_place` — the render
//! path has no alpha channel of its own.

/// One parsed clipboard image: top-down BGRA rows, exactly
/// `width * height * 4` bytes (an honored alpha mask is already flattened
/// against the background — the frame is final).
#[derive(Debug)]
pub(crate) struct DibImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bgra: Vec<u8>,
}

/// The DIB header sizes whose layouts this parser reads: the
/// BITMAPINFOHEADER (40), the V2/V3 mask extensions (52/56 — the three
/// (four) masks appended IN the header), and the V4/V5 headers (108/124)
/// with masks at the same fixed offsets.
const HEADER_SIZES: [u32; 5] = [40, 52, 56, 108, 124];

const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

/// The BITMAPINFOHEADER length — the smallest header this parser accepts
/// (the 12-byte BITMAPCOREHEADER predates CF_DIB producers and is
/// rejected with the other unknown sizes).
const CORE_HEADER_LEN: usize = 40;

/// Parse one clipboard DIB into a top-down BGRA frame. `background` is
/// the windowed background an honored alpha mask flattens against
/// (`[R, G, B]`, the loader's DecodeEnv order); `max_frame_bytes` is the
/// allocation cap (the loader's frame budget — a hostile header
/// declaring a huge canvas fails user-level instead of OOM-killing the
/// viewer, the same posture as the decoder's own limits).
pub(crate) fn parse_dib(
    payload: &[u8],
    background: [u8; 3],
    max_frame_bytes: usize,
) -> Result<DibImage, String> {
    if payload.len() < CORE_HEADER_LEN {
        return Err("truncated DIB header".into());
    }
    let bi_size = le32(payload, 0) as usize;
    if !HEADER_SIZES.contains(&(bi_size as u32)) {
        return Err(format!("unsupported DIB header size {bi_size}"));
    }
    if payload.len() < bi_size {
        return Err("truncated DIB header".into());
    }
    let width = le32(payload, 4) as i32;
    let raw_height = le32(payload, 8) as i32;
    if width <= 0 || raw_height == 0 {
        return Err("zero-size DIB".into());
    }
    if le16(payload, 12) != 1 {
        return Err("unsupported plane count".into());
    }
    let bpp = le16(payload, 14);
    if !matches!(bpp, 24 | 32) {
        return Err(format!("unsupported bit depth {bpp}"));
    }
    let compression = le32(payload, 16);
    // A 24/32bpp DIB has no palette; a nonzero biClrUsed is producer error
    // (GDI never writes one) and would misplace the pixel offset.
    if le32(payload, 32) != 0 {
        return Err("palette present in a 24/32bpp DIB".into());
    }

    // Channel masks. BI_RGB uses the default B/G/R byte layout of a
    // 32bpp DIB (memory order BGRA); the bitfield compressions carry
    // explicit masks — after a 40-byte header, or in-header at the fixed
    // V2+ offsets (40/44/48, alpha at 52).
    let mut r_mask = 0x00FF_0000u32;
    let mut g_mask = 0x0000_FF00u32;
    let mut b_mask = 0x0000_00FFu32;
    let mut a_mask = 0u32;
    let mut pixel_offset = bi_size;
    match compression {
        0 => {} // BI_RGB — defaults above; unused for 24bpp
        BI_BITFIELDS | BI_ALPHABITFIELDS => {
            if bpp != 32 {
                return Err("bitfields compression needs 32bpp".into());
            }
            if bi_size == CORE_HEADER_LEN {
                // The three masks follow the header; BI_ALPHABITFIELDS
                // adds a fourth right behind them.
                let mask_len = if compression == BI_ALPHABITFIELDS {
                    16
                } else {
                    12
                };
                if payload.len() < CORE_HEADER_LEN + mask_len {
                    return Err("truncated DIB masks".into());
                }
                r_mask = le32(payload, 40);
                g_mask = le32(payload, 44);
                b_mask = le32(payload, 48);
                if compression == BI_ALPHABITFIELDS {
                    a_mask = le32(payload, 52);
                }
                pixel_offset = CORE_HEADER_LEN + mask_len;
            } else {
                // V2+ carries the masks in-header; the V4/V5 alpha field
                // (offset 52, headers >= 56) is honored under either
                // bitfields compression when set (BITMAPV5HEADER docs —
                // zero means opaque).
                r_mask = le32(payload, 40);
                g_mask = le32(payload, 44);
                b_mask = le32(payload, 48);
                if bi_size >= 56 {
                    a_mask = le32(payload, 52);
                } else if compression == BI_ALPHABITFIELDS {
                    // V2 (52 bytes) has no alpha field — the fourth mask
                    // follows the header.
                    if payload.len() < 56 {
                        return Err("truncated DIB masks".into());
                    }
                    a_mask = le32(payload, 52);
                    pixel_offset = 56;
                }
            }
        }
        other => return Err(format!("unsupported compression {other}")),
    }
    if r_mask == 0 || g_mask == 0 || b_mask == 0 {
        // A missing COLOR channel is producer error (alpha 0 is legal —
        // it just means opaque).
        return Err("zero color mask".into());
    }

    // Validate every mask BEFORE the row loop (a zero color mask or a
    // non-contiguous one is producer error, not per-pixel data).
    let (r_ch, g_ch, b_ch, a_ch) = (
        Channel::of(r_mask, "color")?,
        Channel::of(g_mask, "color")?,
        Channel::of(b_mask, "color")?,
        Channel::of(a_mask, "alpha")?,
    );

    let top_down = raw_height < 0;
    let height = raw_height.unsigned_abs() as usize;
    let width_u = width as usize;
    let bytes_per_px = usize::from(bpp / 8);
    // DIB rows are DWORD-aligned whatever the pixel width leaves over.
    let stride = (width_u * bytes_per_px).div_ceil(4) * 4;
    let Some(rows_bytes) = stride.checked_mul(height) else {
        return Err("DIB exceeds the frame budget".into());
    };
    if pixel_offset + rows_bytes > payload.len() {
        return Err("truncated DIB pixels".into());
    }
    let Some(frame_bytes) = width_u.checked_mul(height).and_then(|px| px.checked_mul(4)) else {
        return Err("DIB exceeds the frame budget".into());
    };
    if frame_bytes > max_frame_bytes {
        return Err(format!(
            "DIB exceeds the {max_frame_bytes} byte frame budget"
        ));
    }

    let mut bgra = vec![0u8; frame_bytes];
    for row in 0..height {
        // bottom-up memory flips into the top-down frame (DibFrame's
        // negative-biHeight convention).
        let src_row = if top_down { row } else { height - 1 - row };
        let src_start = pixel_offset + src_row * stride;
        let src = &payload[src_start..src_start + width_u * bytes_per_px];
        let dst = &mut bgra[row * width_u * 4..][..width_u * 4];
        match bpp {
            24 => {
                let (pixels, _) = dst.as_chunks_mut::<4>();
                let (tris, _) = src.as_chunks::<3>();
                for (px, tri) in pixels.iter_mut().zip(tris) {
                    px[0] = tri[0]; // B
                    px[1] = tri[1]; // G
                    px[2] = tri[2]; // R
                    px[3] = 255;
                }
            }
            32 => {
                let (pixels, _) = dst.as_chunks_mut::<4>();
                let (quads, _) = src.as_chunks::<4>();
                for (px, q) in pixels.iter_mut().zip(quads) {
                    let v = u32::from_le_bytes([q[0], q[1], q[2], q[3]]);
                    // Masks validated above; the defaults are contiguous
                    // by construction, so every channel exists here.
                    let (b, g, r) = (
                        b_ch.as_ref().unwrap().sample(v),
                        g_ch.as_ref().unwrap().sample(v),
                        r_ch.as_ref().unwrap().sample(v),
                    );
                    match a_ch.as_ref().map(|ch| ch.sample(v)) {
                        Some(a) if a != 255 => {
                            // Flatten against the windowed background —
                            // pixels.rs's integer formula (upstream
                            // viv.c:10166-10168), truncating division
                            // per channel. Output is BGRA; the background
                            // is [R, G, B].
                            let mix = |c: i32, bg: u8| {
                                let bg = i32::from(bg);
                                // pixels.rs's shape: the division binds to
                                // the (c - bg) * a term only (viv.c:10166).
                                (bg + (c - bg) * i32::from(a) / 255) as u8
                            };
                            px[0] = mix(i32::from(b), background[2]);
                            px[1] = mix(i32::from(g), background[1]);
                            px[2] = mix(i32::from(r), background[0]);
                            px[3] = 255;
                        }
                        _ => {
                            px[0] = b;
                            px[1] = g;
                            px[2] = r;
                            px[3] = 255;
                        }
                    }
                }
            }
            _ => unreachable!("bpp matched 24|32 above"),
        }
    }
    Ok(DibImage {
        width: width_u as u32,
        height: height as u32,
        bgra,
    })
}

/// One sampled channel: a contiguous bit mask's (shift, width). `Err` for
/// a non-contiguous mask; `Ok(None)` for a zero mask (the channel is
/// absent — legal only for alpha, whose absence means opaque).
struct Channel {
    shift: u32,
    bits: u32,
}

impl Channel {
    fn of(mask: u32, kind: &str) -> Result<Option<Channel>, String> {
        if mask == 0 {
            return Ok(None);
        }
        let shift = mask.trailing_zeros();
        let bits = mask.count_ones();
        if mask != (((1u32 << bits) - 1) << shift) {
            return Err(format!("non-contiguous {kind} mask"));
        }
        Ok(Some(Channel { shift, bits }))
    }

    /// Extract and rescale to 8 bits (a 10-bit channel's 1023 becomes
    /// 255, its 512 becomes 128 — round-to-nearest of v*255/max).
    fn sample(&self, pixel: u32) -> u8 {
        let max = (1u64 << self.bits) - 1;
        let v = u64::from((pixel >> self.shift) & (max as u32));
        (((v * 255 + max / 2) / max).min(255)) as u8
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: usize = 1 << 20;

    /// A 40-byte BITMAPINFOHEADER with the given geometry; biClrUsed 0.
    fn core_header(width: i32, height: i32, bpp: u16, compression: u32) -> Vec<u8> {
        let mut h = Vec::with_capacity(40);
        h.extend_from_slice(&40u32.to_le_bytes()); // biSize
        h.extend_from_slice(&width.to_le_bytes());
        h.extend_from_slice(&height.to_le_bytes()); // sign = row order
        h.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        h.extend_from_slice(&bpp.to_le_bytes());
        h.extend_from_slice(&compression.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage (often garbage — ignored)
        h.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
        h.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
        h.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
        h.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
        h
    }

    /// Append `rows` of raw pixel data, each padded out to the DWORD
    /// `stride` the format requires.
    fn with_rows(mut payload: Vec<u8>, rows: &[&[u8]], stride: usize) -> Vec<u8> {
        for r in rows {
            payload.extend_from_slice(r);
            payload.resize(payload.len() + (stride - r.len()), 0);
        }
        payload
    }

    // ---- 24bpp BI_RGB ----

    #[test]
    fn a_24bpp_bottom_up_dib_flips_rows_and_pads_the_stride() {
        // 2x2, row stride 6 -> 8 (2 pad bytes to skip). Memory rows are
        // BOTTOM first; the top-down frame lists them the other way.
        let payload = with_rows(
            core_header(2, 2, 24, 0),
            &[&[1, 2, 3, 4, 5, 6], &[7, 8, 9, 10, 11, 12]],
            8,
        );
        let img = parse_dib(&payload, [0, 0, 0], BUDGET).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        // First OUTPUT row = last memory row (the visual top).
        assert_eq!(&img.bgra[0..8], &[7, 8, 9, 255, 10, 11, 12, 255]);
        assert_eq!(&img.bgra[8..16], &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    // ---- 32bpp BI_RGB ----

    #[test]
    fn a_32bpp_top_down_dib_keeps_row_order_and_forces_opaque() {
        // Negative biHeight = top-down (memory order = visual order); the
        // undefined 4th byte (0 here) renders opaque.
        let payload = with_rows(
            core_header(2, -2, 32, 0),
            &[
                &[10, 20, 30, 0, 40, 50, 60, 0],
                &[70, 80, 90, 99, 100, 110, 120, 255],
            ],
            8,
        );
        let img = parse_dib(&payload, [0, 0, 0], BUDGET).unwrap();
        assert_eq!(&img.bgra[0..8], &[10, 20, 30, 255, 40, 50, 60, 255]);
        assert_eq!(&img.bgra[8..16], &[70, 80, 90, 255, 100, 110, 120, 255]);
    }

    #[test]
    fn a_32bpp_bottom_up_dib_flips_rows() {
        let payload = with_rows(core_header(1, 2, 32, 0), &[&[9, 9, 9, 0], &[1, 1, 1, 0]], 4);
        let img = parse_dib(&payload, [0, 0, 0], BUDGET).unwrap();
        // Memory [9.., 1..] = bottom first; output is [1.., 9..].
        assert_eq!(&img.bgra[0..4], &[1, 1, 1, 255]);
        assert_eq!(&img.bgra[4..8], &[9, 9, 9, 255]);
    }

    #[test]
    fn a_getdibits_shaped_dib_round_trips() {
        // What the CF_BITMAP reader synthesizes (clipboard.rs): a
        // 40-byte header, 32bpp, BI_RGB, top-down — must parse unchanged.
        let payload = with_rows(
            core_header(2, 1, 32, 0),
            &[&[0, 0, 255, 0, 0, 255, 0, 0]],
            8,
        );
        let img = parse_dib(&payload, [0, 0, 0], BUDGET).unwrap();
        assert_eq!(&img.bgra[..], &[0, 0, 255, 255, 0, 255, 0, 255]);
    }

    // ---- BI_BITFIELDS masks ----

    #[test]
    fn bitfields_masks_reorder_the_channels() {
        // Memory RGBA order (R low byte): the masks, not the byte
        // positions, decide where each channel lives.
        let mut payload = core_header(1, 1, 32, BI_BITFIELDS);
        payload.extend_from_slice(&0x0000_00FFu32.to_le_bytes()); // R
        payload.extend_from_slice(&0x0000_FF00u32.to_le_bytes()); // G
        payload.extend_from_slice(&0x00FF_0000u32.to_le_bytes()); // B
        payload.extend_from_slice(&[0x11, 0x22, 0x33, 0x00]);
        let img = parse_dib(&payload, [0, 0, 0], BUDGET).unwrap();
        assert_eq!(&img.bgra[..], &[0x33, 0x22, 0x11, 255]);
    }

    #[test]
    fn ten_bit_masks_scale_to_eight_bits() {
        // 10-bit channels (deep-color clipboard shapes) rescale: 1023 ->
        // 255, 512 -> 128, 0 -> 0.
        let mut payload = core_header(1, 1, 32, BI_BITFIELDS);
        payload.extend_from_slice(&0x0000_03FFu32.to_le_bytes()); // R, bits 0-9
        payload.extend_from_slice(&0x000F_FC00u32.to_le_bytes()); // G, bits 10-19
        payload.extend_from_slice(&0xFFC0_0000u32.to_le_bytes()); // B, bits 22-31
        // v = (512 << 10) | 1023 = 0x803FF; B = 0.
        payload.extend_from_slice(&0x0008_03FFu32.to_le_bytes());
        let img = parse_dib(&payload, [0, 0, 0], BUDGET).unwrap();
        assert_eq!(&img.bgra[..], &[0, 128, 255, 255]);
    }

    #[test]
    fn alpha_bitfields_after_a_core_header_add_the_fourth_mask() {
        // BI_ALPHABITFIELDS with biSize 40: R/G/B at 40..52, alpha at
        // 52..56, pixels from 56. Half-transparent black flattens against
        // the background with pixels.rs's truncating formula.
        let mut payload = core_header(1, 1, 32, BI_ALPHABITFIELDS);
        payload.extend_from_slice(&0x00FF_0000u32.to_le_bytes()); // R
        payload.extend_from_slice(&0x0000_FF00u32.to_le_bytes()); // G
        payload.extend_from_slice(&0x0000_00FFu32.to_le_bytes()); // B
        payload.extend_from_slice(&0xFF00_0000u32.to_le_bytes()); // A
        // Pixel: RGB 0, alpha 128.
        payload.extend_from_slice(&[0, 0, 0, 0x80]);
        let img = parse_dib(&payload, [200, 100, 50], BUDGET).unwrap();
        // bg + (0 - bg) * 128 / 255, per channel, truncating.
        assert_eq!(&img.bgra[..], &[25, 50, 100, 255]);
    }

    #[test]
    fn a_zero_alpha_mask_under_bitfields_stays_opaque() {
        // BI_ALPHABITFIELDS with a zero alpha mask: no alpha channel —
        // the pixel renders as authored.
        let mut payload = core_header(1, 1, 32, BI_ALPHABITFIELDS);
        payload.extend_from_slice(&0x00FF_0000u32.to_le_bytes());
        payload.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
        payload.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // alpha mask 0
        payload.extend_from_slice(&[7, 8, 9, 0x00]);
        let img = parse_dib(&payload, [200, 200, 200], BUDGET).unwrap();
        assert_eq!(&img.bgra[..], &[7, 8, 9, 255]);
    }

    // ---- V4/V5 in-header masks ----

    #[test]
    fn a_v5_header_carries_masks_and_alpha_in_header() {
        // biSize 124: masks at the fixed in-header offsets, alpha honored
        // under BI_BITFIELDS when set; pixels start at 124.
        let mut payload = core_header(2, 1, 32, BI_BITFIELDS);
        payload[0..4].copy_from_slice(&124u32.to_le_bytes());
        payload.resize(124, 0);
        let tail = |payload: &mut Vec<u8>, off: usize, v: u32| {
            payload[off..off + 4].copy_from_slice(&v.to_le_bytes());
        };
        tail(&mut payload, 40, 0x0000_00FF); // bV5RedMask
        tail(&mut payload, 44, 0x0000_FF00); // bV5GreenMask
        tail(&mut payload, 48, 0x00FF_0000); // bV5BlueMask
        tail(&mut payload, 52, 0xFF00_0000); // bV5AlphaMask
        // Pixel 1: white, opaque. Pixel 2: black, half alpha over
        // background [200, 100, 50] -> [25, 50, 100] BGR.
        payload.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0x80]);
        let img = parse_dib(&payload, [200, 100, 50], BUDGET).unwrap();
        assert_eq!(&img.bgra[0..4], &[255, 255, 255, 255]);
        assert_eq!(&img.bgra[4..8], &[25, 50, 100, 255]);
    }

    #[test]
    fn v2_and_v4_header_sizes_parse_with_default_masks() {
        for size in [52u32, 56, 108] {
            let mut payload = core_header(1, 1, 32, 0);
            payload[0..4].copy_from_slice(&size.to_le_bytes());
            payload.resize(size as usize, 0);
            payload.extend_from_slice(&[1, 2, 3, 0]);
            let img = parse_dib(&payload, [0, 0, 0], BUDGET)
                .unwrap_or_else(|e| panic!("biSize {size}: {e}"));
            assert_eq!(&img.bgra[..], &[1, 2, 3, 255]);
        }
    }

    // ---- rejections ----

    #[test]
    fn truncated_headers_masks_and_pixels_are_rejected() {
        // Header cut short.
        assert!(parse_dib(&[0u8; 39], [0; 3], BUDGET).is_err());
        // Declared biSize beyond the payload.
        let mut cut = core_header(1, 1, 32, 0);
        cut.truncate(60);
        cut[0..4].copy_from_slice(&124u32.to_le_bytes());
        assert!(parse_dib(&cut, [0; 3], BUDGET).is_err());
        // Masks cut short (BI_BITFIELDS needs 12 bytes after a 40-byte
        // header).
        let mut short_masks = core_header(1, 1, 32, BI_BITFIELDS);
        short_masks.extend_from_slice(&[0u8; 8]);
        assert!(parse_dib(&short_masks, [0; 3], BUDGET).is_err());
        // Pixels cut short (a full 2-row DIB losing its last row).
        let full = with_rows(core_header(1, 2, 32, 0), &[&[1, 2, 3, 0], &[4, 5, 6, 0]], 4);
        let cut = full[..full.len() - 4].to_vec();
        assert!(parse_dib(&cut, [0; 3], BUDGET).is_err());
    }

    #[test]
    fn unsupported_depths_compressions_and_geometries_are_rejected() {
        let bad = |p: Vec<u8>| parse_dib(&p, [0; 3], BUDGET).is_err();
        // 16bpp / 8bpp.
        assert!(bad(core_header(1, 1, 16, 0)));
        assert!(bad(core_header(1, 1, 8, 0)));
        // RLE8 and the JPEG/PNG-embedded compressions.
        assert!(bad(core_header(1, 1, 32, 1)));
        assert!(bad(core_header(1, 1, 32, 4)));
        assert!(bad(core_header(1, 1, 32, 5)));
        // Bitfields at 24bpp.
        assert!(bad(core_header(1, 1, 24, BI_BITFIELDS)));
        // The 12-byte BITMAPCOREHEADER.
        let mut core = core_header(1, 1, 32, 0);
        core[0..4].copy_from_slice(&12u32.to_le_bytes());
        assert!(bad(core));
        // Planes != 1, zero extents, a palette at 24/32bpp.
        let mut two_planes = core_header(1, 1, 32, 0);
        two_planes[12..14].copy_from_slice(&2u16.to_le_bytes());
        assert!(bad(two_planes));
        assert!(bad(core_header(0, 1, 32, 0)));
        assert!(bad(core_header(1, 0, 32, 0)));
        let mut palette = core_header(1, 1, 32, 0);
        palette[32..36].copy_from_slice(&4u32.to_le_bytes());
        assert!(bad(palette));
    }

    #[test]
    fn zero_or_noncontiguous_masks_are_rejected() {
        // A zero COLOR mask is producer error (alpha 0 is legal, colors
        // must exist).
        let mut zero = core_header(1, 1, 32, BI_BITFIELDS);
        zero.extend_from_slice(&0u32.to_le_bytes());
        zero.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
        zero.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
        zero.extend_from_slice(&[1, 2, 3, 4]);
        assert!(parse_dib(&zero, [0; 3], BUDGET).is_err());
        // Non-contiguous mask.
        let mut gaps = core_header(1, 1, 32, BI_BITFIELDS);
        gaps.extend_from_slice(&0x00FF_00FFu32.to_le_bytes());
        gaps.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
        gaps.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
        gaps.extend_from_slice(&[1, 2, 3, 4]);
        assert!(parse_dib(&gaps, [0; 3], BUDGET).is_err());
    }

    #[test]
    fn a_frame_over_the_budget_is_rejected() {
        // 2x2 needs 16 bytes; a 15-byte cap refuses before allocating.
        let payload = with_rows(core_header(2, 2, 32, 0), &[&[0; 8], &[0; 8]], 8);
        let err = parse_dib(&payload, [0; 3], 15).unwrap_err();
        assert!(err.contains("budget"), "{err}");
    }
}
