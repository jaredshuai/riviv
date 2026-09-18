//! ICM Stage 1 (#77): embedded ICC -> system sRGB through the Windows
//! Color System (mscms), on the decode worker — the ADR 0002 D2 pipeline
//! slot `decode(RGBA) -> ICM -> composite over bg -> PixelFrame(BGRA)`.
//!
//! The transform folds the RGBA->BGRA swizzle into its output side:
//! `TranslateBitmapBits` reads the decoder's `[R,G,B,x]` rows as
//! `BM_xBGRQUADS` and emits the master's `[B,G,R,x]` layout as
//! `BM_xRGBQUADS` — probe-verified byte-identical to a manual swizzle
//! plus a same-format transform. The composite then runs on the BGRA
//! output (the sRGB background is never transformed) and
//! `PixelFrame::from_bgra` boxes it.
//!
//! sRGB equivalence is detected BEFORE trusting the transform, because
//! the CMM's own sRGB->sRGB pass drifts a few LSBs (observed max 4/255 —
//! Microsoft's "precision errors" warning is real). Two gates: a
//! byte-identical embedded blob is skipped without any WCS call (the
//! common tagged case), and a *different* profile encoding sRGB is
//! caught by probing the built transform against a 1024-pixel ramp —
//! equivalent sources drift within the noise floor, real profiles move
//! midtones by tens.
//!
//! Every failure downgrades to "decode untagged": breadcrumb on the
//! debug channel, raw pixels, the load itself unaffected (the viewer's
//! user-level failure contract — keep the old image, no dialog, no
//! exit). The ICC header gate runs before any WCS call because
//! `OpenColorProfileW` happily opens non-RGB profiles (a CMYK 'prtr'
//! profile opens and even builds a transform — verified against
//! RSWOP.icm), and applying one to the decoder's RGB output would be
//! wrong.

use std::cell::Cell;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::OnceLock;

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::Win32::UI::ColorSystem::{
    BEST_MODE, BM_xBGRQUADS, BM_xRGBQUADS, CloseColorProfile, CreateMultiProfileTransform,
    DeleteColorTransform, GetStandardColorSpaceProfileW, INDEX_DONT_CARE, INTENT_PERCEPTUAL,
    LCS_sRGB, OpenColorProfileW, PROFILE, PROFILE_MEMBUFFER, PROFILE_READ, TranslateBitmapBits,
    USE_RELATIVE_COLORIMETRIC,
};
use windows::core::{PCWSTR, PWSTR};

/// The identity-probe tolerance: the CMM drifts at most ~4/255 on an
/// sRGB-equivalent source (observed: byte-different system sRGB variants
/// max 4, a byte-identical profile is skipped upstream by the blob
/// compare). A genuinely different profile moves midtones by tens, so
/// 8 sits a full octave above the noise with an order of magnitude of
/// headroom below the real-transform floor.
const SRGB_EQUIVALENT_TOLERANCE: u8 = 8;

/// The ICC header gate — everything checked here runs BEFORE any WCS
/// call. `OpenColorProfileW` accepts non-RGB profiles (a CMYK 'prtr'
/// profile opens and even builds a transform), so the dataColorSpace
/// check is ours to make: the decoder hands us RGB pixels, and only an
/// RGB v2/v4 profile is a legal source for them.
fn icc_declares_rgb_v2v4(blob: &[u8]) -> bool {
    blob.len() >= 128
        && blob[36..40] == *b"acsp"
        && blob[16..20] == *b"RGB "
        && matches!(blob[8], 2 | 4)
}

/// The 1024-pixel fingerprint: gray ramp plus pure-channel ramps. An
/// sRGB-equivalent source maps these to themselves within CMM noise; a
/// foreign profile moves the primaries even when it happens to share
/// sRGB's tone curve (the gray ramp alone cannot distinguish them).
fn probe_pixels() -> Vec<u8> {
    let mut v = Vec::with_capacity(1024 * 4);
    for i in 0..=255u8 {
        v.extend_from_slice(&[i, i, i, 255]);
    }
    for i in 0..=255u8 {
        v.extend_from_slice(&[i, 0, 0, 255]);
    }
    for i in 0..=255u8 {
        v.extend_from_slice(&[0, i, 0, 255]);
    }
    for i in 0..=255u8 {
        v.extend_from_slice(&[0, 0, i, 255]);
    }
    v
}

/// Largest per-channel drift between an RGBA source buffer and the
/// BGRA output the transform produced from it — the comparison is
/// channel-semantic (R vs R), so it maps the folded swizzle.
fn probe_max_channel_diff(src_rgba: &[u8], dst_bgra: &[u8]) -> u8 {
    src_rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst_bgra.as_chunks::<4>().0.iter())
        .flat_map(|(i, o)| {
            [
                i[0].abs_diff(o[2]),
                i[1].abs_diff(o[1]),
                i[2].abs_diff(o[0]),
            ]
        })
        .max()
        .unwrap_or(0)
}

/// Resolve the system sRGB profile's bytes once per process. Windows
/// guarantees this profile exists (it is the WCS canonical space and
/// the runtime's own Stage-1 target); a failure here disables ICM for
/// the process rather than retrying per image.
fn system_srgb() -> Option<&'static [u8]> {
    static SRGB: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    SRGB.get_or_init(|| match load_system_srgb() {
        Ok(blob) => Some(blob),
        Err(e) => {
            eprintln!(
                "riviv: icm: system sRGB profile unavailable ({e}) — tagged images decode untagged"
            );
            None
        }
    })
    .as_deref()
}

/// The two-call `GetStandardColorSpaceProfileW` pattern: the first call
/// fails reporting the required path-buffer byte count, the second
/// fills it (the API resolves the localized sRGB filename itself — no
/// hardcoded color-directory path).
fn load_system_srgb() -> Result<Vec<u8>, String> {
    let mut size = 0u32;
    // SAFETY: null machine name and null buffer — `size` is a valid out
    // pointer the failed size query writes the required byte count to.
    let ok = unsafe {
        GetStandardColorSpaceProfileW(PCWSTR::null(), LCS_sRGB.0 as u32, None, &mut size)
    };
    if ok.as_bool() || size == 0 || !size.is_multiple_of(2) {
        return Err(format!(
            "GetStandardColorSpaceProfileW size query returned an unusable size {size}"
        ));
    }
    let mut wide = vec![0u16; (size / 2) as usize];
    let mut used = size;
    // SAFETY: `wide` is a `size`-byte writable PWSTR buffer per the
    // two-call contract; `used` mirrors its capacity.
    let ok = unsafe {
        GetStandardColorSpaceProfileW(
            PCWSTR::null(),
            LCS_sRGB.0 as u32,
            Some(PWSTR(wide.as_mut_ptr())),
            &mut used,
        )
    };
    if !ok.as_bool() {
        // SAFETY: thread error slot read immediately after the failed call.
        let gle = unsafe { GetLastError() }.0;
        return Err(format!("GetStandardColorSpaceProfileW failed (GLE={gle})"));
    }
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    let path = OsString::from_wide(&wide[..len]);
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.to_string_lossy()))
}

/// RAII for an HPROFILE opened from a memory buffer. The blob is owned
/// alongside the handle: the API contract does not promise the profile
/// copy is complete at open time, so the caller's buffer must outlive
/// the handle — owning it makes that unconditional.
struct ProfileHandle {
    h: isize,
    _blob: Vec<u8>,
}

impl ProfileHandle {
    fn open_mem(blob: Vec<u8>) -> Result<Self, u32> {
        let profile = PROFILE {
            dwType: PROFILE_MEMBUFFER,
            pProfileData: blob.as_ptr() as *mut core::ffi::c_void,
            cbDataSize: blob.len() as u32,
        };
        // SAFETY: `profile` points at `blob`, which lives in the returned
        // handle for its whole lifetime; OpenColorProfileW only reads it.
        let h = unsafe { OpenColorProfileW(&profile, PROFILE_READ, 0, OPEN_EXISTING.0) };
        if h == 0 {
            // SAFETY: thread error slot read immediately after the failed call.
            Err(unsafe { GetLastError() }.0)
        } else {
            Ok(ProfileHandle { h, _blob: blob })
        }
    }
}

impl Drop for ProfileHandle {
    fn drop(&mut self) {
        // SAFETY: `h` is a profile handle this value owns; it is closed
        // exactly once here. A close failure is unrecoverable and
        // unreportable from Drop.
        let _ = unsafe { CloseColorProfile(Some(self.h)) };
    }
}

/// The two-profile transform the frames flow through — source first,
/// the system sRGB profile second. Relative colorimetric intent (the
/// `USE_RELATIVE_COLORIMETRIC` flag overrides `padwIntent`): in-gamut
/// colors land byte-accurate, matching how browsers treat tagged
/// images, and keeping the identity probe meaningful — perceptual
/// rescales the gamut and would blur the equivalence line.
fn create_transform(src: &ProfileHandle, dst: &ProfileHandle) -> Option<isize> {
    let profiles = [src.h, dst.h];
    let intents = [INTENT_PERCEPTUAL; 2];
    // SAFETY: both handles are live and owned for the call's duration;
    // the returned transform keeps its own references.
    let xform = unsafe {
        CreateMultiProfileTransform(
            &profiles,
            &intents,
            BEST_MODE | USE_RELATIVE_COLORIMETRIC,
            INDEX_DONT_CARE,
        )
    };
    (xform != 0).then_some(xform)
}

/// A prepared ICC->sRGB transform: one per decode job, applied to every
/// frame of the image exactly once (upstream applies ICM at load time
/// the same way — `GdipLoadImageFromStreamICM`). Created, used, and
/// dropped on the decode worker; nothing crosses a thread boundary.
///
/// Field order matters for drop: `Transform`'s own `Drop` deletes the
/// transform BEFORE the profile handles close (a transform may still
/// reference its profiles while alive).
pub(crate) struct Transform {
    xform: isize,
    shown: Box<str>,
    apply_failed: Cell<bool>,
    _src: ProfileHandle,
    _dst: ProfileHandle,
}

impl Drop for Transform {
    fn drop(&mut self) {
        // SAFETY: `xform` is a transform this value owns; it is deleted
        // exactly once here, before the profile handles (fields) drop.
        // A delete failure is unrecoverable and unreportable from Drop.
        let _ = unsafe { DeleteColorTransform(self.xform) };
    }
}

impl Transform {
    /// `TranslateBitmapBits` over one full frame: RGBA source rows in,
    /// BGRA master rows out (the folded swizzle). No logging — callers
    /// decide what a failure means.
    fn translate(&self, width: u32, height: u32, src_rgba: &[u8], dst_bgra: &mut [u8]) -> bool {
        debug_assert_eq!(src_rgba.len(), width as usize * height as usize * 4);
        debug_assert_eq!(dst_bgra.len(), src_rgba.len());
        let stride = width * 4;
        // SAFETY: both buffers are `width*height*4` bytes (asserted) and
        // distinct allocations, matching the API's non-in-place contract;
        // the transform handle is borrowed live via &self.
        unsafe {
            TranslateBitmapBits(
                self.xform,
                src_rgba.as_ptr().cast(),
                BM_xBGRQUADS, // the decoder's [R,G,B,x] rows
                width,
                height,
                stride,
                dst_bgra.as_mut_ptr().cast(),
                BM_xRGBQUADS, // the master's [B,G,R,x] layout
                stride,
                None,
                None,
            )
        }
        .as_bool()
    }

    /// One decoded frame: RGBA in, BGRA out. `false` = the CMM refused
    /// the pass — the caller falls back to the untransformed RGBA path
    /// for the frame (breadcrumb once per transform, not per frame).
    pub(crate) fn apply(
        &self,
        width: u32,
        height: u32,
        src_rgba: &[u8],
        dst_bgra: &mut [u8],
    ) -> bool {
        let ok = self.translate(width, height, src_rgba, dst_bgra);
        if !ok && !self.apply_failed.replace(true) {
            eprintln!(
                "riviv: icm: {}: TranslateBitmapBits failed — remaining frames decode untransformed",
                self.shown
            );
        }
        ok
    }
}

/// Stage-1 entry point: decide whether `icc` (the decoder's embedded
/// profile bytes) needs an ICC->sRGB transform and build it. `None`
/// means "decode untagged" — `icm=0`, no profile, a profile already
/// equivalent to sRGB, or any degrade along the chain (each leaves a
/// breadcrumb; the load itself is never failed over color management).
pub(crate) fn prepare(enabled: bool, icc: Option<Vec<u8>>, shown: &str) -> Option<Transform> {
    if !enabled {
        return None;
    }
    let blob = icc?;
    if !icc_declares_rgb_v2v4(&blob) {
        eprintln!("riviv: icm: {shown}: embedded profile is not RGB ICC v2/v4 — decoding untagged");
        return None;
    }
    let srgb = system_srgb()?;
    if blob.as_slice() == srgb {
        // The common tagged case: byte-identical to the system sRGB
        // profile — skipped without a single WCS call.
        return None;
    }
    let src = match ProfileHandle::open_mem(blob) {
        Ok(h) => h,
        Err(gle) => {
            eprintln!(
                "riviv: icm: {shown}: OpenColorProfileW(source) failed (GLE={gle}) — decoding untagged"
            );
            return None;
        }
    };
    let dst = match ProfileHandle::open_mem(srgb.to_vec()) {
        Ok(h) => h,
        Err(gle) => {
            eprintln!(
                "riviv: icm: {shown}: OpenColorProfileW(sRGB) failed (GLE={gle}) — decoding untagged"
            );
            return None;
        }
    };
    let Some(xform) = create_transform(&src, &dst) else {
        eprintln!("riviv: icm: {shown}: CreateMultiProfileTransform failed — decoding untagged");
        return None;
    };
    let transform = Transform {
        xform,
        shown: shown.into(),
        apply_failed: Cell::new(false),
        _src: src,
        _dst: dst,
    };
    // A different blob can still encode the sRGB space (the byte
    // compare above only catches the identical one). Probe the built
    // transform: the CMM drifts only a few LSBs on a same-space pass,
    // so an equivalent source is detected by the noise floor — the
    // decoded bytes stay verbatim instead of taking a shifted round
    // trip (Microsoft's same-profile precision-error caveat).
    let probe = probe_pixels();
    let mut out = vec![0u8; probe.len()];
    if !transform.translate((probe.len() / 4) as u32, 1, &probe, &mut out) {
        eprintln!("riviv: icm: {shown}: probe transform failed — decoding untagged");
        return None;
    }
    if probe_max_channel_diff(&probe, &out) <= SRGB_EQUIVALENT_TOLERANCE {
        return None;
    }
    Some(transform)
}

// ---- a minimal synthetic ICC v2 profile builder (test fixtures) ----
// The CMM needs a real tag set to build a usable transform: a
// bare-bones header-plus-colorants profile opens but produces
// degenerate output. This set (matrix colorants + 1024-entry curv
// TRCs + wtpt/bkpt/lumi/chad/desc/cprt) mirrors the system sRGB
// profile's structure and transforms correctly — verified against
// mscms directly. `pub(crate)` so the loader's end-to-end decode tests
// reuse the same profiles.
#[cfg(test)]
pub(crate) mod test_fixtures {
    fn be16(v: u16) -> [u8; 2] {
        v.to_be_bytes()
    }
    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }
    fn s15f16(v: f64) -> [u8; 4] {
        be32((v * 65536.0).round() as i32 as u32)
    }
    fn tag_xyz(x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut t = b"XYZ ".to_vec();
        t.extend_from_slice(&be32(0));
        for v in [x, y, z] {
            t.extend_from_slice(&s15f16(v));
        }
        t
    }
    /// 'curv' with a 1024-entry table (the shape the CMM's transform
    /// builder consumes reliably — single-gamma and parametric forms
    /// produced degenerate transforms in probing).
    fn tag_curv_table(f: impl Fn(f64) -> f64) -> Vec<u8> {
        let mut t = b"curv".to_vec();
        t.extend_from_slice(&be32(0));
        t.extend_from_slice(&be32(1024));
        for i in 0..1024u32 {
            t.extend_from_slice(&be16((f(i as f64 / 1023.0) * 65535.0).round() as u16));
        }
        t
    }
    fn tag_desc(text: &str) -> Vec<u8> {
        let mut t = b"desc".to_vec();
        t.extend_from_slice(&be32(0));
        let a = text.as_bytes();
        t.extend_from_slice(&be32(a.len() as u32 + 1));
        t.extend_from_slice(a);
        t.push(0);
        t
    }
    fn tag_text(text: &str) -> Vec<u8> {
        let mut t = b"text".to_vec();
        t.extend_from_slice(&be32(0));
        t.extend_from_slice(text.as_bytes());
        t.push(0);
        t
    }
    fn tag_sf32_identity() -> Vec<u8> {
        let mut t = b"sf32".to_vec();
        t.extend_from_slice(&be32(0));
        for v in [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            t.extend_from_slice(&s15f16(v));
        }
        t
    }

    /// Assemble a v2 'mntr'/'RGB '/'XYZ ' matrix-shaper profile.
    fn synthetic_icc(primaries: [[f64; 3]; 3], trc: Vec<u8>) -> Vec<u8> {
        let tags: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"desc", tag_desc("synthetic")),
            (*b"cprt", tag_text("test")),
            (*b"wtpt", tag_xyz(0.9642, 1.0, 0.8249)),
            (*b"bkpt", tag_xyz(0.0, 0.0, 0.0)),
            (*b"lumi", tag_xyz(0.7645, 0.8, 0.9216)),
            (*b"chad", tag_sf32_identity()),
            (
                *b"rXYZ",
                tag_xyz(primaries[0][0], primaries[0][1], primaries[0][2]),
            ),
            (
                *b"gXYZ",
                tag_xyz(primaries[1][0], primaries[1][1], primaries[1][2]),
            ),
            (
                *b"bXYZ",
                tag_xyz(primaries[2][0], primaries[2][1], primaries[2][2]),
            ),
            (*b"rTRC", trc.clone()),
            (*b"gTRC", trc.clone()),
            (*b"bTRC", trc),
        ];
        let n = tags.len() as u32;
        let mut table = be32(n).to_vec();
        let mut data = Vec::new();
        let base = 128 + 4 + n as usize * 12;
        for (sig_bytes, bytes) in &tags {
            table.extend_from_slice(sig_bytes);
            table.extend_from_slice(&be32((base + data.len()) as u32));
            table.extend_from_slice(&be32(bytes.len() as u32));
            data.extend_from_slice(bytes);
            while data.len() % 4 != 0 {
                data.push(0);
            }
        }
        let size = (128 + 4 + n as usize * 12 + data.len()) as u32;
        let mut h = vec![0u8; 128];
        h[0..4].copy_from_slice(&be32(size));
        h[8] = 0x02;
        h[9] = 0x10;
        h[12..16].copy_from_slice(b"mntr");
        h[16..20].copy_from_slice(b"RGB ");
        h[20..24].copy_from_slice(b"XYZ ");
        // date-time: 2026-09-18 12:00:00
        let dt = [2026u16, 9, 18, 12, 0, 0];
        for (i, v) in dt.iter().enumerate() {
            h[24 + i * 2..26 + i * 2].copy_from_slice(&be16(*v));
        }
        h[36..40].copy_from_slice(b"acsp");
        h[40..44].copy_from_slice(b"MSFT");
        h[68..72].copy_from_slice(&s15f16(0.9642));
        h[72..76].copy_from_slice(&s15f16(1.0));
        h[76..80].copy_from_slice(&s15f16(0.8249));
        let mut profile = h;
        profile.extend_from_slice(&table);
        profile.extend_from_slice(&data);
        profile
    }

    fn srgb_trc() -> Vec<u8> {
        tag_curv_table(|x| {
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        })
    }
    const SRGB_PRIMARIES: [[f64; 3]; 3] = [
        [0.43607, 0.22249, 0.01392],
        [0.38515, 0.71687, 0.09708],
        [0.14307, 0.06061, 0.71410],
    ];
    const ADOBE_PRIMARIES: [[f64; 3]; 3] = [
        [0.6483, 0.3296, 0.0],
        [0.1882, 0.6407, 0.0307],
        [0.1456, 0.0407, 0.7454],
    ];

    /// sRGB primaries + sRGB tone curve: a DIFFERENT blob encoding the
    /// same space (the L2 probe's job to detect).
    pub(crate) fn srgb_like_icc() -> Vec<u8> {
        synthetic_icc(SRGB_PRIMARIES, srgb_trc())
    }

    /// AdobeRGB primaries + gamma 2.2: a real foreign profile.
    pub(crate) fn adobe_like_icc() -> Vec<u8> {
        synthetic_icc(ADOBE_PRIMARIES, tag_curv_table(|x| x.powf(2.2)))
    }

    /// Require a working system sRGB profile (the same dependency the
    /// runtime transform has — Windows always ships one).
    pub(crate) fn require_srgb() -> &'static [u8] {
        super::system_srgb().expect("this test needs the system sRGB profile")
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;

    // ---- pure header gate ----

    fn header(colorspace: &[u8; 4], major: u8, acsp: &[u8; 4]) -> Vec<u8> {
        let mut h = vec![0u8; 128];
        h[8] = major;
        h[16..20].copy_from_slice(colorspace);
        h[36..40].copy_from_slice(acsp);
        h
    }

    #[test]
    fn rgb_v2_and_v4_headers_pass_the_gate() {
        assert!(icc_declares_rgb_v2v4(&header(b"RGB ", 2, b"acsp")));
        assert!(icc_declares_rgb_v2v4(&header(b"RGB ", 4, b"acsp")));
    }

    #[test]
    fn non_rgb_colorspaces_are_rejected_before_any_wcs_call() {
        // A CMYK profile (e.g. a print-tagged JPEG) would transform the
        // decoder's RGB output through a CMYK space — wrong colors, so
        // the gate rejects it outright.
        assert!(!icc_declares_rgb_v2v4(&header(b"CMYK", 2, b"acsp")));
        assert!(!icc_declares_rgb_v2v4(&header(b"GRAY", 2, b"acsp")));
        assert!(!icc_declares_rgb_v2v4(&header(b"Lab ", 2, b"acsp")));
        assert!(!icc_declares_rgb_v2v4(&header(b"XYZ ", 4, b"acsp")));
    }

    #[test]
    fn malformed_and_out_of_scope_headers_are_rejected() {
        assert!(!icc_declares_rgb_v2v4(&[]));
        assert!(!icc_declares_rgb_v2v4(&[0u8; 64]));
        assert!(!icc_declares_rgb_v2v4(&header(b"RGB ", 2, b"junk")));
        assert!(!icc_declares_rgb_v2v4(&header(b"RGB ", 3, b"acsp")));
        assert!(!icc_declares_rgb_v2v4(&header(b"RGB ", 5, b"acsp")));
    }

    // ---- probe machinery ----

    #[test]
    fn the_probe_compare_is_channel_semantic_across_the_swizzle() {
        // src [R,G,B] vs dst [B,G,R]: equal colors must diff to zero
        // despite the byte reorder; a moved R channel counts.
        let src = [200, 60, 10, 255];
        assert_eq!(probe_max_channel_diff(&src, &[10, 60, 200, 255]), 0);
        assert_eq!(probe_max_channel_diff(&src, &[10, 60, 190, 255]), 10);
        assert_eq!(probe_max_channel_diff(&src, &[3, 60, 200, 255]), 7);
    }

    #[test]
    fn a_blob_byte_identical_to_srgb_is_skipped_without_a_transform() {
        // L1: the common tagged case — same bytes as the system profile.
        let srgb = require_srgb().to_vec();
        assert!(prepare(true, Some(srgb), "t").is_none());
    }

    #[test]
    fn a_semantically_srgb_profile_is_detected_by_the_probe() {
        // L2: a DIFFERENT blob encoding the same space — sRGB primaries
        // plus the sRGB tone curve — must not round-trip the pixels
        // (the CMM's own pass drifts a few LSBs; tolerance swallows it).
        let _ = require_srgb();
        assert!(prepare(true, Some(srgb_like_icc()), "t").is_none());
    }

    #[test]
    fn a_foreign_profile_transforms_pixels_and_keeps_alpha() {
        let _ = require_srgb();
        let t = prepare(true, Some(adobe_like_icc()), "t")
            .expect("an AdobeRGB-like profile transforms");
        // Asymmetric color + a midtone gray, RGBA in / BGRA out.
        let src = [200u8, 60, 10, 255, 128, 128, 128, 255];
        let mut dst = [0u8; 8];
        assert!(t.apply(2, 1, &src, &mut dst));
        // The transform moved the color (foreign profile), R stayed in
        // the BGRA slot's byte 2, and alpha is forced opaque.
        assert_ne!(dst[2], 200, "the CMM moved the source red");
        assert_eq!(dst[3], 255);
        assert_eq!(dst[7], 255);
        assert!((dst[1] as i32 - 60).abs() <= 8, "green stays near-gamut");
    }

    #[test]
    fn disabled_icm_and_untagged_images_never_build_anything() {
        assert!(
            prepare(false, Some(adobe_like_icc()), "t").is_none(),
            "icm=0 bypasses"
        );
        assert!(prepare(true, None, "t").is_none(), "no profile, no cost");
    }

    #[test]
    fn non_rgb_and_malformed_blobs_are_downgraded() {
        assert!(prepare(true, Some(header(b"CMYK", 2, b"acsp")), "t").is_none());
        assert!(prepare(true, Some(vec![0u8; 16]), "t").is_none());
    }
}
