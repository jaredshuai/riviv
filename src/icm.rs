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
//! #142 (ADR 0003 D6) promoted the chain to 16-bit output: the same
//! source shapes now translate into `BM_16b_RGB` (16-bit gamma
//! fixed-point [B,G,R] triplets, full scale 65535) and CPU-transcode
//! into the f16 master's RGBA halves — `apply_f16`, the frames' primary
//! path. The 8-bit `apply` stays as the fallback arm (ADR 0003 D5: a
//! refused 16-bit pass downgrades that frame to the 8-bit chain,
//! breadcrumb once, load unaffected); the equivalence probe still runs
//! in the 8-bit domain on `translate`, its tolerance calibrated there.
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
//! #155 (ADR 0004 D2/D5) added the DESTINATION axis: on a hardware
//! session a genuinely-foreign profile's transform is RETARGETED from
//! the system sRGB destination to riviv's own Display-P3-D65 v2
//! matrix-shaper profile (built once, in-process, from fixed literals —
//! [`p3_destination_profile`]), and the frames it touches land as
//! `F16P3` masters whose halves keep the super-sRGB colors inside the
//! P3 container instead of clipping them at the sRGB boundary. A WARP
//! session keeps the sRGB destination (WARP never runs the wide draw
//! arms, so a wide master there would be fake color) — the backend is
//! the caller's one request-time snapshot (`DecodeEnv.backend`). The
//! retarget can fail (profile open or transform build) and then simply
//! keeps the sRGB transform: the ladder's "guarantee correct color"
//! floor (D6), one breadcrumb, nothing else changes. The loader-side
//! fallback ladder lives in loader.rs's `assemble_frame` /
//! `assemble_deep_frame`: a refused wide 16-bit pass retargets back to
//! sRGB and retries the narrow chain.
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
    BEST_MODE, BM_16b_RGB, BM_xBGRQUADS, BM_xRGBQUADS, CloseColorProfile,
    CreateMultiProfileTransform, DeleteColorTransform, GetStandardColorSpaceProfileW,
    INDEX_DONT_CARE, INTENT_RELATIVE_COLORIMETRIC, LCS_sRGB, OpenColorProfileW, PROFILE,
    PROFILE_MEMBUFFER, PROFILE_READ, TranslateBitmapBits, USE_RELATIVE_COLORIMETRIC,
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

/// The two-call `GetStandardColorSpaceProfileW` pattern: the size query
/// with a null buffer reports the required path-buffer byte count, the
/// second call fills it (the API resolves the localized sRGB filename
/// itself — no hardcoded color-directory path). The query's BOOL result
/// is NOT a verdict here (MSDN Parameters documents a TRUE return for
/// the null-buffer call too): only an unusable size — zero, or not a
/// wide-char multiple — makes the query failed.
fn load_system_srgb() -> Result<Vec<u8>, String> {
    let mut size = 0u32;
    // SAFETY: null machine name and null buffer — `size` is a valid out
    // pointer the failed size query writes the required byte count to.
    let _ = unsafe {
        GetStandardColorSpaceProfileW(PCWSTR::null(), LCS_sRGB.0 as u32, None, &mut size)
    };
    if size == 0 || !size.is_multiple_of(2) {
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

// ---- a minimal synthetic ICC v2 profile builder (production) ----
// The CMM needs a real tag set to build a usable transform: a
// bare-bones header-plus-colorants profile opens but produces
// degenerate output. This set (matrix colorants + 1024-entry curv
// TRCs + wtpt/bkpt/lumi/chad/desc/cprt) mirrors the system sRGB
// profile's structure and transforms correctly — verified against
// mscms directly. Lifted from the test fixtures when #155 needed a
// REAL destination profile (the P3 constant below); the fixtures
// delegate so every pre-existing test blob stays byte-identical.

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

/// Assemble a v2 'mntr'/'RGB '/'XYZ ' matrix-shaper profile. Fully
/// deterministic: every byte derives from the arguments (date included),
/// so the same call always produces the same blob.
fn synthetic_icc(
    desc: &str,
    cprt: &str,
    date: [u16; 6],
    primaries: [[f64; 3]; 3],
    trc: Vec<u8>,
) -> Vec<u8> {
    let tags: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"desc", tag_desc(desc)),
        (*b"cprt", tag_text(cprt)),
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
    for (i, v) in date.iter().enumerate() {
        h[24 + i * 2..26 + i * 2].copy_from_slice(&be16(*v));
    }
    h[36..40].copy_from_slice(b"acsp");
    h[40..44].copy_from_slice(b"MSFT");
    // PCS illuminant: D50 (ICC-mandated for v2 profile connection
    // space) — probe P-A verified mscms refuses otherwise.
    h[68..72].copy_from_slice(&s15f16(0.9642));
    h[72..76].copy_from_slice(&s15f16(1.0));
    h[76..80].copy_from_slice(&s15f16(0.8249));
    let mut profile = h;
    profile.extend_from_slice(&table);
    profile.extend_from_slice(&data);
    profile
}

/// The sRGB tone response as a 1024-entry curv table — the TRC both the
/// test fixtures and the P3 destination profile share (the P3 TRC IS an
/// sRGB-shaped curve; parametric forms were rejected by mscms, probe
/// P-A GLE=2011).
fn srgb_trc() -> Vec<u8> {
    tag_curv_table(|x| {
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// Display P3 (D65) primaries stored as ICC PCS (XYZ) colorants: the
/// Bradford-adapted D50 values, probe P-A's verified literals. Stored
/// verbatim — never recomputed at runtime (the adaptation matrix math
/// is a probe concern, not a build concern).
const P3_D50_COLORANTS: [[f64; 3]; 3] = [
    [0.51512, 0.24119, -0.00105],
    [0.29198, 0.69224, 0.04188],
    [0.15711, 0.06657, 0.78408],
];

/// riviv's own Display-P3-D65 destination profile (#155, ADR 0004 D2):
/// the wide container Stage 1 maps genuinely-foreign tagged sources
/// into on hardware sessions, so super-sRGB colors survive to the
/// panel instead of clipping at the sRGB gamut boundary. Built once per
/// process from fixed literals (a v2 matrix-shaper, same shape the
/// equivalence probe trusts in sources) — a pure function of constants,
/// hence the byte-stable hash the test pins.
fn p3_destination_profile() -> &'static [u8] {
    static P3: OnceLock<Vec<u8>> = OnceLock::new();
    P3.get_or_init(|| {
        synthetic_icc(
            "riviv Display P3-D65",
            "public domain",
            [2026, 9, 27, 12, 0, 0],
            P3_D50_COLORANTS,
            srgb_trc(),
        )
    })
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
/// `USE_RELATIVE_COLORIMETRIC` flag forces it, and the intents array
/// now says the same thing): in-gamut colors land byte-accurate,
/// matching how browsers treat tagged images, and keeping the identity
/// probe meaningful — perceptual rescales the gamut and would blur the
/// equivalence line. Failure carries the thread error slot's GLE up to
/// the caller's breadcrumb.
fn create_transform(src: &ProfileHandle, dst: &ProfileHandle) -> Result<isize, u32> {
    let profiles = [src.h, dst.h];
    let intents = [INTENT_RELATIVE_COLORIMETRIC; 2];
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
    if xform != 0 {
        Ok(xform)
    } else {
        // SAFETY: thread error slot read immediately after the failed call.
        Err(unsafe { GetLastError() }.0)
    }
}

/// The source-side bitmap format of one frame for the 16-bit output chain
/// (ADR 0003 D6): the 8-bit decode path hands RGBA byte quads
/// (`BM_xBGRQUADS`, what the decoder produces), the deep path hands the
/// CMM's own `BM_16b_RGB` shape ([B,G,R] u16 triplets — the probe-verified
/// 16→16 form). The destination is always `BM_16b_RGB`.
pub(crate) enum FrameSrc<'a> {
    Rgba8(&'a [u8]),
    Bgr16(&'a [u16]),
}

/// A prepared ICC->sRGB transform: one per decode job, applied to every
/// frame of the image exactly once (upstream applies ICM at load time
/// the same way — `GdipLoadImageFromStreamICM`). Created, used, and
/// dropped on the decode worker; nothing crosses a thread boundary.
///
/// #155 (ADR 0004 D2/D5): the DESTINATION starts as the system sRGB
/// profile, but on a hardware session with a genuinely-foreign source
/// profile `prepare` retargets it to riviv's own P3 profile
/// ([`p3_destination_profile`]) — [`Transform::wide`] then says the
/// frames this transform touches land in the P3 container. A failed
/// retarget simply keeps sRGB (one breadcrumb; the D6 "correct color"
/// floor).
///
/// `Transform`'s own `Drop` deletes the transform handle BEFORE the
/// profile handles close — Rust guarantees the `Drop` impl runs ahead
/// of field destruction, and the CMM may keep referencing its profiles
/// while the transform is alive.
pub(crate) struct Transform {
    xform: isize,
    shown: Box<str>,
    apply_failed: Cell<bool>,
    /// Held only so the profile outlives `xform` (RAII); the leading
    /// underscore marks the drop-only field.
    _src_profile: ProfileHandle,
    /// Same hold for the destination profile (system sRGB, or riviv's
    /// P3 after a successful [`Transform::retarget_wide`]).
    _dst_profile: ProfileHandle,
    /// Whether the destination is the wide P3 container (#155): the
    /// loader's wide arm dispatches on this. Always mirrors the REAL
    /// destination — `true` only after a successful `retarget_wide`,
    /// flipped back by a successful `retarget_srgb`.
    wide: bool,
    /// Test hook for the D5 fallback (#142): force every `translate16`
    /// call to fail so the fallback chains can be exercised without a
    /// CMM that actually refuses `BM_16b_RGB`.
    #[cfg(test)]
    pub(crate) force16_fail: Cell<bool>,
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
    /// BGRA master rows out (the folded swizzle). `Err` carries the
    /// GLE for the caller's breadcrumb. No logging — callers decide
    /// what a failure means.
    fn translate(
        &self,
        width: u32,
        height: u32,
        src_rgba: &[u8],
        dst_bgra: &mut [u8],
    ) -> Result<(), u32> {
        debug_assert_eq!(src_rgba.len(), width as usize * height as usize * 4);
        debug_assert_eq!(dst_bgra.len(), src_rgba.len());
        let stride = width * 4;
        // SAFETY: both buffers span `width*height*4` bytes — debug
        // builds assert it, release builds rely on the callers' own
        // construction (the decode pipeline allocates `dst` from the
        // source buffer's exact length) — and are distinct allocations,
        // matching the API's non-in-place contract; the transform
        // handle is borrowed live via &self.
        let ok = unsafe {
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
        .as_bool();
        if ok {
            Ok(())
        } else {
            // SAFETY: thread error slot read immediately after the failed call.
            Err(unsafe { GetLastError() }.0)
        }
    }

    /// The 16-bit output chain's FFI pass (ADR 0003 D6): whatever the
    /// source shape, the destination is always `BM_16b_RGB` — 16-bit
    /// gamma fixed-point [B,G,R] triplets, full scale 65535, stride
    /// `width * 6` bytes. `Err` carries the GLE for the caller's
    /// breadcrumb; no logging — callers decide what a failure means.
    fn translate16(
        &self,
        width: u32,
        height: u32,
        src: FrameSrc<'_>,
        dst_bgr16: &mut [u16],
    ) -> Result<(), u32> {
        #[cfg(test)]
        if self.force16_fail.get() {
            return Err(50); // ERROR_NOT_SUPPORTED — the injected refusal
        }
        debug_assert_eq!(
            dst_bgr16.len(),
            width as usize * height as usize * 3,
            "three u16 triplets per destination pixel"
        );
        let (src_ptr, src_format, src_stride) = match src {
            FrameSrc::Rgba8(bytes) => {
                debug_assert_eq!(bytes.len(), width as usize * height as usize * 4);
                (
                    bytes.as_ptr().cast(),
                    BM_xBGRQUADS, // the decoder's [R,G,B,x] rows
                    width * 4,
                )
            }
            FrameSrc::Bgr16(samples) => {
                debug_assert_eq!(samples.len(), width as usize * height as usize * 3);
                (
                    samples.as_ptr().cast(),
                    BM_16b_RGB, // the deep path's own [B,G,R] u16 triplets
                    width * 6,
                )
            }
        };
        let dst_stride = width * 6;
        // SAFETY: the source pointer spans exactly the source format's
        // `width*height` pixels (debug asserts above, callers construct
        // the buffers from the same dimensions) and the destination
        // `width*height*3` u16 values; the two are distinct allocations,
        // matching the API's non-in-place contract; the transform handle
        // is borrowed live via &self.
        let ok = unsafe {
            TranslateBitmapBits(
                self.xform,
                src_ptr,
                src_format,
                width,
                height,
                src_stride,
                dst_bgr16.as_mut_ptr().cast(),
                BM_16b_RGB, // the 16-bit gamma output, always
                dst_stride,
                None,
                None,
            )
        }
        .as_bool();
        if ok {
            Ok(())
        } else {
            // SAFETY: thread error slot read immediately after the failed call.
            Err(unsafe { GetLastError() }.0)
        }
    }

    /// One decoded frame: RGBA in, BGRA out. `false` = the CMM refused
    /// the pass — the caller falls back to the untransformed RGBA path
    /// for the frame (breadcrumb once per transform, not per frame).
    ///
    /// The `x` byte of `BM_x*QUADS` is documented *unused*, so the CMM
    /// makes no promise about it — mscms happens to pass it through
    /// byte-for-byte (probe-verified), but the composite downstream
    /// needs the decoder's alpha, so it is restored from the source
    /// here regardless: correctness must not ride on undocumented
    /// behavior.
    pub(crate) fn apply(
        &self,
        width: u32,
        height: u32,
        src_rgba: &[u8],
        dst_bgra: &mut [u8],
    ) -> bool {
        if let Err(gle) = self.translate(width, height, src_rgba, dst_bgra) {
            if !self.apply_failed.replace(true) {
                eprintln!(
                    "riviv: icm: {}: TranslateBitmapBits failed (GLE={gle}) — remaining frames decode untransformed",
                    self.shown
                );
            }
            return false;
        }
        for (dst, src) in dst_bgra
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(src_rgba.as_chunks::<4>().0)
        {
            dst[3] = src[3];
        }
        true
    }

    /// The 16-bit output chain (ADR 0003 D6): `src` through the CMM into
    /// `BM_16b_RGB`, then CPU-transcoded into the f16 master's RGBA half
    /// layout ([`crate::pixels::bgr_u16_to_f16_rgba`] — the CMM emits BGR
    /// order, the master stores RGBA). Alpha is the output format's
    /// default (1.0); a source with real transparency is the CALLER's to
    /// restore before the composite. `None` = the CMM refused the 16-bit
    /// pass — the caller falls back to the 8-bit chain (`apply`) for the
    /// frame (breadcrumb once per transform, not per frame; ADR 0003 D5).
    pub(crate) fn apply_f16(&self, width: u32, height: u32, src: FrameSrc<'_>) -> Option<Vec<u16>> {
        let mut bgr = vec![0u16; width as usize * height as usize * 3];
        if let Err(gle) = self.translate16(width, height, src, &mut bgr) {
            if !self.apply_failed.replace(true) {
                eprintln!(
                    "riviv: icm: {}: TranslateBitmapBits(BM_16b_RGB) failed (GLE={gle}) — frames fall back to the 8-bit path",
                    self.shown
                );
            }
            return None;
        }
        let mut halves = vec![0u16; width as usize * height as usize * 4];
        crate::pixels::bgr_u16_to_f16_rgba(&bgr, &mut halves);
        Some(halves)
    }

    /// Whether this transform's destination is the wide P3 container
    /// (#155) — the loader's wide-arm dispatch bit. `true` only after a
    /// successful [`Transform::retarget_wide`].
    pub(crate) fn destination_is_wide(&self) -> bool {
        self.wide
    }

    /// Swap the destination profile in place (#155): open `blob` as the
    /// new destination, build a fresh transform from the SAME source
    /// profile to it, and — only after that succeeded — delete the old
    /// transform and replace both fields. `Err`/`false` leaves the
    /// transform untouched (the caller keeps its sRGB behavior).
    ///
    /// No new unsafe: the swap composes the existing safe wrappers
    /// ([`ProfileHandle::open_mem`], [`create_transform`]); the one raw
    /// call is the same `DeleteColorTransform` the `Drop` impl makes.
    /// Field-swap order is the liveness contract: the CMM may reference
    /// its profiles while a transform lives, so the OLD destination
    /// handle must outlive the OLD transform — the old xform is deleted
    /// first, then the field replace drops the old `ProfileHandle`.
    fn retarget(&mut self, dst: ProfileHandle, wide: bool, failure_word: &str) -> bool {
        let xform = match create_transform(&self._src_profile, &dst) {
            Ok(xform) => xform,
            Err(gle) => {
                eprintln!(
                    "riviv: icm: {}: {failure_word} unavailable (GLE={gle}) — {}",
                    self.shown,
                    dst_failure_consequence(wide),
                );
                return false;
            }
        };
        // SAFETY: `self.xform` is a transform this value owns and is
        // about to stop owning; it is deleted exactly once here, before
        // the old destination profile handle (field) drops. A delete
        // failure is unrecoverable and unreportable from here.
        let _ = unsafe { DeleteColorTransform(self.xform) };
        self.xform = xform;
        self._dst_profile = dst;
        self.wide = wide;
        true
    }

    /// Retarget the destination to riviv's P3 profile (#155): the
    /// hardware session's wide container, so super-sRGB source colors
    /// land INSIDE the container instead of clipping at sRGB. `false` =
    /// the wide destination could not be built — the transform keeps
    /// its sRGB destination (one breadcrumb; the ladder's floor).
    fn retarget_wide(&mut self) -> bool {
        let dst = match ProfileHandle::open_mem(p3_destination_profile().to_vec()) {
            Ok(dst) => dst,
            Err(gle) => {
                eprintln!(
                    "riviv: icm: {}: wide destination profile unavailable (GLE={gle}) — clipping to sRGB",
                    self.shown
                );
                return false;
            }
        };
        self.retarget(dst, true, "wide destination profile")
    }

    /// Retarget the destination back to the system sRGB profile (#155):
    /// the wide arm's fallback when the CMM refuses the wide 16-bit
    /// pass (loader ladder). `false` = sRGB could not be rebuilt either
    /// — the caller falls to the untransformed tail; the transform
    /// keeps its current destination.
    pub(crate) fn retarget_srgb(&mut self) -> bool {
        let Some(blob) = system_srgb() else {
            eprintln!(
                "riviv: icm: {}: sRGB destination unavailable — decoding untransformed",
                self.shown
            );
            return false;
        };
        let dst = match ProfileHandle::open_mem(blob.to_vec()) {
            Ok(dst) => dst,
            Err(gle) => {
                eprintln!(
                    "riviv: icm: {}: sRGB destination unavailable (GLE={gle}) — decoding untransformed",
                    self.shown
                );
                return false;
            }
        };
        self.retarget(dst, false, "sRGB destination profile")
    }
}

/// The retarget failure's consequence phrase — a wide failure keeps
/// clipping to sRGB, an sRGB failure (the wide arm's own fallback) has
/// nothing left but the raw decode.
fn dst_failure_consequence(wide: bool) -> &'static str {
    if wide {
        "clipping to sRGB"
    } else {
        "decoding untransformed"
    }
}

/// Stage-1 entry point: decide whether `icc` (the decoder's embedded
/// profile bytes) needs an ICC transform and build it. `None`
/// means "decode untagged" — `icm=0`, no profile, a profile already
/// equivalent to sRGB, or any degrade along the chain (each leaves a
/// breadcrumb; the load itself is never failed over color management).
///
/// `backend` is the caller's request-time snapshot of the render
/// backend (#155, ADR 0004 D2): a `Hardware` session retargets a
/// genuinely-foreign profile's transform to the wide P3 destination,
/// `Warp` keeps the sRGB destination (today's production behavior,
/// byte-for-byte). The equivalence-probe path (display_profile.rs)
/// builds sRGB destinations only and never retargets.
pub(crate) fn prepare(
    enabled: bool,
    icc: Option<Vec<u8>>,
    shown: &str,
    backend: crate::transform_stage::Backend,
) -> Option<Transform> {
    if !enabled {
        return None;
    }
    let blob = icc?;
    match probe_equivalence(blob, shown, "decoding untagged") {
        // The ladder's own failures breadcrumb inside; the equivalent
        // verdicts both mean "no transform" for Stage 1.
        None | Some(EquivalenceProbe::Equivalent) => None,
        Some(EquivalenceProbe::Different(mut transform)) => {
            // D2's destination selection: wide only on hardware. A
            // failed retarget keeps the sRGB transform (breadcrumb
            // already emitted) — identical to the WARP shape below.
            if backend == crate::transform_stage::Backend::Hardware {
                transform.retarget_wide();
            }
            Some(transform)
        }
    }
}

/// The shared sRGB-equivalence ladder (#130): Stage 1's skip decision and
/// the display segment's `SrgbEquivalent` classification run the SAME
/// sequence — the RGB v2/v4 gate, the byte-identical short-circuit, then
/// the probe through a transform built for THIS blob. Direction note: the
/// probe always runs blob->sRGB (Stage 1's direction); the max-channel
/// noise floor is direction-agnostic — a same-space pair drifts LSBs
/// either way, a genuinely different space moves midtones either way — so
/// the display side (whose real transform runs sRGB->display on the GPU)
/// reuses this verbatim for classification. `None` = the ladder itself
/// failed; every failure breadcrumbs with `downgrade` as the consequence
/// phrase (Stage 1 passes "decoding untagged"; the display judge passes
/// its own fall-to-Unknown wording).
pub(crate) enum EquivalenceProbe {
    Equivalent,
    Different(Transform),
}

pub(crate) fn probe_equivalence(
    blob: Vec<u8>,
    shown: &str,
    downgrade: &str,
) -> Option<EquivalenceProbe> {
    if !icc_declares_rgb_v2v4(&blob) {
        eprintln!("riviv: icm: {shown}: embedded profile is not RGB ICC v2/v4 — {downgrade}");
        return None;
    }
    let srgb = system_srgb()?;
    if blob.as_slice() == srgb {
        // The common tagged case: byte-identical to the system sRGB
        // profile — skipped without a single WCS call.
        return Some(EquivalenceProbe::Equivalent);
    }
    let src = match ProfileHandle::open_mem(blob) {
        Ok(h) => h,
        Err(gle) => {
            eprintln!(
                "riviv: icm: {shown}: OpenColorProfileW(source) failed (GLE={gle}) — {downgrade}"
            );
            return None;
        }
    };
    let dst = match ProfileHandle::open_mem(srgb.to_vec()) {
        Ok(h) => h,
        Err(gle) => {
            eprintln!(
                "riviv: icm: {shown}: OpenColorProfileW(sRGB) failed (GLE={gle}) — {downgrade}"
            );
            return None;
        }
    };
    let xform = match create_transform(&src, &dst) {
        Ok(xform) => xform,
        Err(gle) => {
            eprintln!(
                "riviv: icm: {shown}: CreateMultiProfileTransform failed (GLE={gle}) — {downgrade}"
            );
            return None;
        }
    };
    let transform = Transform {
        xform,
        shown: shown.into(),
        apply_failed: Cell::new(false),
        _src_profile: src,
        _dst_profile: dst,
        // The equivalence probe always builds an sRGB destination (the
        // probe IS the sRGB comparison); any wide retarget is the
        // caller's decision (#155, prepare's backend arm).
        wide: false,
        #[cfg(test)]
        force16_fail: Cell::new(false),
    };
    // A different blob can still encode the sRGB space (the byte
    // compare above only catches the identical one). Probe the built
    // transform: the CMM drifts only a few LSBs on a same-space pass,
    // so an equivalent source is detected by the noise floor — the
    // decoded bytes stay verbatim instead of taking a shifted round
    // trip (Microsoft's same-profile precision-error caveat).
    let probe = probe_pixels();
    let mut out = vec![0u8; probe.len()];
    if let Err(gle) = transform.translate((probe.len() / 4) as u32, 1, &probe, &mut out) {
        eprintln!("riviv: icm: {shown}: probe transform failed (GLE={gle}) — {downgrade}");
        return None;
    }
    if probe_max_channel_diff(&probe, &out) <= SRGB_EQUIVALENT_TOLERANCE {
        return Some(EquivalenceProbe::Equivalent);
    }
    Some(EquivalenceProbe::Different(transform))
}

// ---- a minimal synthetic ICC v2 profile builder (test fixtures) ----
// The assembly itself is PRODUCTION code now (#155 lifted it for the
// P3 destination profile); this module only picks the fixture
// parameters. The constants below reproduce the fixtures' original
// byte streams exactly (same desc/cprt/date the lifted builder once
// hardcoded), so every pre-existing test blob is unchanged.
#[cfg(test)]
pub(crate) mod test_fixtures {
    /// Assemble a v2 'mntr'/'RGB '/'XYZ ' matrix-shaper profile (the
    /// fixtures' original shape: desc="synthetic", cprt="test",
    /// date 2026-09-18 12:00:00).
    fn synthetic_icc(primaries: [[f64; 3]; 3], trc: Vec<u8>) -> Vec<u8> {
        super::synthetic_icc("synthetic", "test", [2026, 9, 18, 12, 0, 0], primaries, trc)
    }

    /// sRGB primaries + sRGB tone curve: a DIFFERENT blob encoding the
    /// same space (the L2 probe's job to detect).
    pub(crate) fn srgb_like_icc() -> Vec<u8> {
        synthetic_icc(SRGB_PRIMARIES, super::srgb_trc())
    }

    /// AdobeRGB primaries + gamma 2.2: a real foreign profile.
    pub(crate) fn adobe_like_icc() -> Vec<u8> {
        synthetic_icc(ADOBE_PRIMARIES, super::tag_curv_table(|x| x.powf(2.2)))
    }

    /// Display P3 (D65) primaries + the sRGB tone curve: the second
    /// real foreign space the #77 acceptance names.
    pub(crate) fn p3_like_icc() -> Vec<u8> {
        synthetic_icc(P3_D65_PRIMARIES, super::srgb_trc())
    }

    /// sRGB primaries + a linear tone curve: same gamut, wildly
    /// different transfer function — the midtone-mover of the suite
    /// (design pin: `linLike` must prepare a transform).
    pub(crate) fn lin_like_icc() -> Vec<u8> {
        synthetic_icc(SRGB_PRIMARIES, super::tag_curv_table(|x| x))
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
    const P3_D65_PRIMARIES: [[f64; 3]; 3] = [
        [0.4866, 0.2290, 0.0000],
        [0.2657, 0.6918, 0.0451],
        [0.1982, 0.0792, 1.0439],
    ];

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
    use crate::transform_stage::Backend;

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
        assert!(prepare(true, Some(srgb), "t", Backend::Warp).is_none());
    }

    #[test]
    fn a_semantically_srgb_profile_is_detected_by_the_probe() {
        // L2: a DIFFERENT blob encoding the same space — sRGB primaries
        // plus the sRGB tone curve — must not round-trip the pixels
        // (the CMM's own pass drifts a few LSBs; tolerance swallows it).
        let _ = require_srgb();
        assert!(prepare(true, Some(srgb_like_icc()), "t", Backend::Warp).is_none());
    }

    #[test]
    fn a_foreign_profile_transforms_pixels_and_keeps_alpha() {
        let _ = require_srgb();
        let t = prepare(true, Some(adobe_like_icc()), "t", Backend::Warp)
            .expect("an AdobeRGB-like profile transforms");
        // Pure red, an asymmetric color, a midtone gray and a
        // semi-transparent pixel, RGBA in / BGRA out. The first two pin
        // the channel placement: red lands in the BGRA slot's byte 2
        // with B/G near zero (a swapped format would move it), and the
        // CMM moves the source color off its input value.
        let src = [
            255u8, 0, 0, 255, //
            200, 60, 10, 255, //
            128, 128, 128, 255, //
            200, 60, 10, 128,
        ];
        let mut dst = [0u8; 16];
        assert!(t.apply(4, 1, &src, &mut dst));
        // Pure red under a wider gamut maps to a more saturated red:
        // byte 2 (R) stays pinned at full, B (byte 0) stays near zero,
        // and G (byte 1) sits far below its midtone value.
        assert_eq!(dst[2], 255, "pure red keeps a full R channel");
        assert!(dst[0] <= 2, "pure red keeps B near zero, got {}", dst[0]);
        assert!(dst[1] <= 2, "pure red keeps G near zero, got {}", dst[1]);
        assert_ne!(dst[6], 200, "the CMM moved the source red");
        assert!((dst[5] as i32 - 60).abs() <= 8, "green stays near-gamut");
        // Alpha: opaque stays opaque, and the semi-transparent source
        // pixel's alpha is carried through byte-for-byte (design pin —
        // the composite downstream keys on it).
        assert_eq!(dst[3], 255);
        assert_eq!(dst[7], 255);
        assert_eq!(dst[11], 255);
        assert_eq!(dst[15], 128, "alpha passes through the transform");
    }

    #[test]
    fn display_p3_and_linear_gamma_profiles_also_prepare_transforms() {
        // The #77 acceptance names AdobeRGB and Display P3 as the two
        // real foreign spaces, and the design pin adds a linear-gamma
        // variant: every one of them must prepare (none is sRGB).
        let _ = require_srgb();
        for blob in [p3_like_icc(), lin_like_icc()] {
            assert!(
                prepare(true, Some(blob), "t", Backend::Warp).is_some(),
                "a genuinely foreign profile must transform"
            );
        }
    }

    #[test]
    fn a_linear_gamma_profile_moves_midtones_hard() {
        // The linLike discriminator: same primaries as sRGB but a
        // linear transfer function, so the [128,128,128] midtone must
        // land far from where it started (identity would leave it at
        // 128; the sRGB encode of linear 0.5 sits near 188).
        let _ = require_srgb();
        let t =
            prepare(true, Some(lin_like_icc()), "t", Backend::Warp).expect("linLike transforms");
        let src = [128u8, 128, 128, 255];
        let mut dst = [0u8; 4];
        assert!(t.apply(1, 1, &src, &mut dst));
        let moved = (dst[2] as i32 - 128).abs();
        assert!(moved >= 30, "midtone must move nontrivially, got {moved}");
    }

    #[test]
    fn disabled_icm_and_untagged_images_never_build_anything() {
        assert!(
            prepare(false, Some(adobe_like_icc()), "t", Backend::Warp).is_none(),
            "icm=0 bypasses"
        );
        assert!(
            prepare(true, None, "t", Backend::Warp).is_none(),
            "no profile, no cost"
        );
    }

    #[test]
    fn non_rgb_and_malformed_blobs_are_downgraded() {
        assert!(prepare(true, Some(header(b"CMYK", 2, b"acsp")), "t", Backend::Warp).is_none());
        assert!(prepare(true, Some(vec![0u8; 16]), "t", Backend::Warp).is_none());
    }

    // ---- the 16-bit output chain (#142, ADR 0003 D6) ----

    #[test]
    fn the_mixed_8bit_source_to_16bit_destination_pass_transforms_pixels() {
        // The MIXED-format shape the #137 probes never ran (8-bit
        // BM_xBGRQUADS source into the BM_16b_RGB destination): this test
        // IS the empirical verdict — if mscms refuses it, the design
        // reroutes 8-bit sources to the "always widen" form and this pin
        // flips with it. Pure red under the wider gamut reads back full
        // R with B/G near zero (the 8-bit arm's asymmetric-color pin,
        // through the master's own quantizer).
        let _ = require_srgb();
        let t = prepare(true, Some(adobe_like_icc()), "t", Backend::Warp)
            .expect("an AdobeRGB-like profile transforms");
        let src = [255u8, 0, 0, 255, 128, 128, 128, 255];
        let Some(halves) = t.apply_f16(2, 1, FrameSrc::Rgba8(&src)) else {
            panic!("mscms refused the mixed 8-bit-src -> BM_16b_RGB-dst pass");
        };
        let red = crate::pixels::f16_rgba_halves_to_bgra8(
            halves[0..4].try_into().expect("one half quadruple"),
        );
        assert_eq!(red[2], 255, "pure red keeps a full R channel");
        assert!(red[0] <= 2, "pure red keeps B near zero, got {}", red[0]);
        assert!(red[1] <= 2, "pure red keeps G near zero, got {}", red[1]);
        // The grey pixel moved off identity (the CMM ran at all).
        let grey = crate::pixels::f16_rgba_halves_to_bgra8(
            halves[4..8].try_into().expect("one half quadruple"),
        );
        assert_ne!(grey[2], 128, "the CMM moved the midtone");
    }

    #[test]
    fn the_16bit_source_shape_transforms_through_the_verified_form() {
        // The Bgr16 source is the #137 probe's verified 16→16 shape: the
        // deep path's [B,G,R] triplets in, halves out. Pure red as
        // [B=0, G=0, R=0xFFFF] must land R full and B/G near zero — a
        // format that failed to honor the BGR order would put the value
        // in the wrong channel.
        let _ = require_srgb();
        let t = prepare(true, Some(adobe_like_icc()), "t", Backend::Warp)
            .expect("an AdobeRGB-like profile transforms");
        let src = [0u16, 0, 0xFFFF, 0x8000, 0x8000, 0x8000];
        let halves = t
            .apply_f16(2, 1, FrameSrc::Bgr16(&src))
            .expect("the probe-verified 16->16 form must pass");
        let red = crate::pixels::f16_rgba_halves_to_bgra8(
            halves[0..4].try_into().expect("one half quadruple"),
        );
        assert_eq!(red[2], 255, "pure red keeps a full R channel");
        assert!(red[0] <= 2, "pure red keeps B near zero, got {}", red[0]);
        assert!(red[1] <= 2, "pure red keeps G near zero, got {}", red[1]);
    }

    #[test]
    fn the_16bit_chain_moves_midtones_like_the_8bit_arm() {
        // The linLike discriminator through the 16-bit chain, judged in
        // the read-back domain (the same criterion the 8-bit arm pins):
        // a linear-gamma profile's [128,128,128] must land far from
        // where it started.
        let _ = require_srgb();
        let t =
            prepare(true, Some(lin_like_icc()), "t", Backend::Warp).expect("linLike transforms");
        let src = [128u8, 128, 128, 255];
        let halves = t
            .apply_f16(1, 1, FrameSrc::Rgba8(&src))
            .expect("the 16-bit chain takes the mixed shape");
        let read = crate::pixels::f16_rgba_halves_to_bgra8(
            halves[0..4].try_into().expect("one half quadruple"),
        );
        let moved = (read[2] as i32 - 128).abs();
        assert!(moved >= 30, "midtone must move nontrivially, got {moved}");
    }

    #[test]
    fn apply_f16_leaves_alpha_at_the_format_default() {
        // BM_16b_RGB carries no alpha channel: even a semi-transparent
        // source comes out with the halves' default 1.0 — restoring the
        // source's own alpha is the loader's job after the pass (the
        // same division the 8-bit arm's x-byte restore follows).
        let _ = require_srgb();
        let t = prepare(true, Some(adobe_like_icc()), "t", Backend::Warp)
            .expect("an AdobeRGB-like profile transforms");
        let src = [200u8, 60, 10, 128];
        let halves = t
            .apply_f16(1, 1, FrameSrc::Rgba8(&src))
            .expect("the 16-bit chain takes the mixed shape");
        assert_eq!(halves[3], crate::pixels::F16_OPAQUE);
    }

    #[test]
    fn the_injected_16bit_failure_falls_back_to_the_8bit_chain() {
        // The D5 fallback's return-value shape, with the CMM forced to
        // refuse the 16-bit pass: apply_f16 is None (and no panic — the
        // breadcrumb is the eprintln channel), and the SAME transform's
        // 8-bit apply still succeeds on the next call. The one-time
        // apply_failed latch means the fallback adds no second failure
        // breadcrumb of its own kind.
        let _ = require_srgb();
        let t = prepare(true, Some(adobe_like_icc()), "t", Backend::Warp)
            .expect("an AdobeRGB-like profile transforms");
        t.force16_fail.set(true);
        assert!(
            t.apply_f16(1, 1, FrameSrc::Rgba8(&[255u8, 0, 0, 255]))
                .is_none()
        );
        t.force16_fail.set(false);
        let mut dst = [0u8; 4];
        assert!(
            t.apply(1, 1, &[255u8, 0, 0, 255], &mut dst),
            "the 8-bit chain still runs"
        );
        assert_eq!(dst[2], 255, "the 8-bit transform's own pin still holds");
    }

    // ---- the wide P3 destination (#155, ADR 0004 D2/D5) ----

    /// The i-th tag entry's (signature, offset, size) from a synthetic
    /// profile's tag table (the lifted builder's own layout: the table
    /// starts at byte 128 with a 4-byte count).
    fn tag_entry(profile: &[u8], index: usize) -> ([u8; 4], u32, u32) {
        let at = 128 + 4 + index * 12;
        let mut sig = [0u8; 4];
        sig.copy_from_slice(&profile[at..at + 4]);
        let offset = u32::from_be_bytes(profile[at + 4..at + 8].try_into().expect("offset"));
        let size = u32::from_be_bytes(profile[at + 8..at + 12].try_into().expect("size"));
        (sig, offset, size)
    }

    /// The s15Fixed16 triple of an 'XYZ ' tag's data.
    fn tag_xyz_values(profile: &[u8], offset: u32) -> [f64; 3] {
        let at = offset as usize + 8; // sig + reserved
        [0, 1, 2]
            .map(|i| {
                let b: [u8; 4] = profile[at + i * 4..at + i * 4 + 4].try_into().expect("xyz");
                i32::from_be_bytes(b) as f64 / 65536.0
            })
            .map(|v| (v * 65536.0).round() / 65536.0) // the s15f16 round trip
    }

    #[test]
    fn the_p3_destination_profile_is_deterministic_and_hash_pinned() {
        // The destination is minted once from fixed literals — a byte
        // drift would silently re-key every F16P3 output identity (the
        // fingerprint's profile term), so the exact digest is pinned.
        let a = p3_destination_profile();
        let b = p3_destination_profile();
        assert_eq!(a, b, "the profile is a pure function of constants");
        assert_eq!(
            crate::transform_stage::profile_hash(a),
            0x9eac_1480_b05c_ae00,
            "pinned P3 destination digest — a change re-keys output identities"
        );
        assert_eq!(a.len(), 6680, "header + 12-entry tag table + tag data");
    }

    #[test]
    fn the_p3_profile_carries_the_bradford_d50_colorants() {
        // P-A's authoritative literals, read back through the tag table
        // and compared in the s15f16 domain both sides are quantized to
        // (the file format stores 16.16 fixed point; the check is that
        // EXACTLY those literals were stored, not a re-adaptation).
        let profile = p3_destination_profile();
        for (sig, want) in [
            (*b"rXYZ", P3_D50_COLORANTS[0]),
            (*b"gXYZ", P3_D50_COLORANTS[1]),
            (*b"bXYZ", P3_D50_COLORANTS[2]),
        ] {
            let want: [f64; 3] = want.map(|v| (v * 65536.0).round() / 65536.0); // the s15f16 round trip
            let mut found = None;
            for i in 0..12 {
                let (s, offset, _) = tag_entry(profile, i);
                if s == sig {
                    found = Some(tag_xyz_values(profile, offset));
                    break;
                }
            }
            assert_eq!(
                found.expect("the colorant tag exists"),
                want,
                "colorant {sig:?}"
            );
        }
    }

    #[test]
    fn the_p3_profile_passes_the_header_gate_and_opens() {
        // The same two gates a SOURCE profile faces (the RGB v2/v4 gate
        // and OpenColorProfileW) — the destination must clear them too,
        // or mscms would refuse every wide transform this profile feeds.
        let profile = p3_destination_profile();
        assert!(icc_declares_rgb_v2v4(profile));
        ProfileHandle::open_mem(profile.to_vec())
            .expect("mscms opens riviv's P3 destination profile");
    }

    #[test]
    fn the_p3_destination_keeps_super_srgb_colors_inside_the_container() {
        // The container's reason to exist, end to end through mscms: an
        // Adobe-fixture source's pure GREEN is outside sRGB (the sRGB
        // destination clips it to exactly the sRGB green primary,
        // 65535 — today's production clipping, the control) but INSIDE
        // the P3 container, which stores it below full scale with real
        // headroom. The pure RED corner is outside even the P3
        // container (the Adobe fixture's red colorants sit beyond P3's
        // in absolute XYZ — P-A's reference number 62456 was measured
        // with matched-profile sources): there the wide destination
        // GAMUT-MAPS instead of hard-clipping to the primary — the red
        // stays full but carries a G component the sRGB destination
        // erases.
        let _ = require_srgb();
        let wide = prepare(true, Some(adobe_like_icc()), "t", Backend::Hardware)
            .expect("an AdobeRGB-like profile transforms");
        assert!(wide.destination_is_wide(), "hardware prepares the wide arm");
        let read16 = |halves: &[u16], px: usize, ch: usize| {
            (crate::pixels::f16_bits_to_f32(halves[px * 4 + ch]) * 65535.0).round() as i32
        };
        let greens = wide
            .apply_f16(1, 1, FrameSrc::Rgba8(&[0u8, 255, 0, 255]))
            .expect("the wide 16-bit pass takes the mixed shape");
        let g = read16(&greens, 0, 1);
        assert!(
            (65535 - 1000..65535).contains(&g),
            "in-container green keeps headroom below full scale, got G16={g} (measured 65375)"
        );
        let reds = wide
            .apply_f16(1, 1, FrameSrc::Rgba8(&[255u8, 0, 0, 255]))
            .expect("the wide 16-bit pass takes the mixed shape");
        let (r, g) = (read16(&reds, 0, 0), read16(&reds, 0, 1));
        assert_eq!(r, 65535, "the out-of-container red pins at full");
        assert!(
            g > 1000,
            "out-of-container red gamut-maps (keeps its hue), got G16={g} (measured 15704)"
        );
        let srgb = prepare(true, Some(adobe_like_icc()), "t", Backend::Warp)
            .expect("the control transform prepares");
        let greens = srgb
            .apply_f16(1, 1, FrameSrc::Rgba8(&[0u8, 255, 0, 255]))
            .expect("the control 16-bit pass takes the mixed shape");
        assert_eq!(
            read16(&greens, 0, 1),
            65535,
            "the sRGB destination clips the same green to the primary"
        );
        let reds = srgb
            .apply_f16(1, 1, FrameSrc::Rgba8(&[255u8, 0, 0, 255]))
            .expect("the control 16-bit pass takes the mixed shape");
        assert_eq!(
            (read16(&reds, 0, 0), read16(&reds, 0, 1)),
            (65535, 0),
            "the sRGB destination hard-clips red to the bare primary"
        );
    }

    #[test]
    fn p3_to_p3_identity_round_trip_stays_near_lossless() {
        // The destination profile's own acceptance (probe P-A): a P3
        // source through the P3 destination is (near-)identity —
        // full-scale primaries survive essentially intact and in-gamut
        // grays drift only single-digit LSBs of the 16-bit container.
        // Measured on the raw BM_16b_RGB CMM output, before the f16
        // transcode adds its own quantization. The source here is the
        // destination profile ITSELF: identity requires the SAME
        // encoding, and the test fixture's P3 primaries are a different
        // P3 matrix shape (the Lindbloom-style colorants, not these
        // Bradford-D50 literals — a genuinely foreign space to the
        // container, not an identity).
        let _ = require_srgb();
        let t = prepare(
            true,
            Some(p3_destination_profile().to_vec()),
            "t",
            Backend::Hardware,
        )
        .expect("a P3 source transforms");
        // BGR16 source rows: full-scale primaries and white, then
        // in-gamut grays (never at the encoding's rails).
        let mut src: Vec<u16> = Vec::new();
        let mut want: Vec<[u16; 3]> = Vec::new();
        for px in [
            [0u16, 0, 0xFFFF],        // red
            [0, 0xFFFF, 0],           // green
            [0xFFFF, 0, 0],           // blue
            [0xFFFF, 0xFFFF, 0xFFFF], // white
        ] {
            src.extend_from_slice(&px);
            want.push(px);
        }
        for v in [
            0x1999u16, 0x3333, 0x4CCC, 0x6666, 0x8000, 0x9999, 0xB333, 0xCCCC, 0xE666,
        ] {
            src.extend_from_slice(&[v, v, v]);
            want.push([v, v, v]);
        }
        let mut dst = vec![0u16; src.len()];
        t.translate16(want.len() as u32, 1, FrameSrc::Bgr16(&src), &mut dst)
            .expect("the identity probe's 16->16 form");
        let mut max_drift = 0u16;
        for (i, want_px) in want.iter().enumerate() {
            let got = [dst[i * 3], dst[i * 3 + 1], dst[i * 3 + 2]];
            for (ch, (g, w)) in got.iter().zip(want_px).enumerate() {
                let d = g.abs_diff(*w);
                let full_scale_channel = *w == 0xFFFF;
                if full_scale_channel {
                    // The primary's own channel (P-A measured min 65523;
                    // the threshold keeps headroom for CMM versions).
                    assert!(
                        *g >= 65500,
                        "full scale must survive, got {g} on channel {ch} (P-A ref min 65523)"
                    );
                } else if *w != 0 {
                    max_drift = max_drift.max(d);
                }
            }
        }
        assert!(
            max_drift <= 32,
            "in-gamut grays stay near-lossless, max drift {max_drift} (P-A ref 13, repo ramp max 26)"
        );
    }

    #[test]
    fn hardware_sessions_retarget_the_transform_to_the_wide_destination() {
        // D2's destination selection: a genuinely-foreign profile on a
        // HARDWARE session earns the wide container.
        let _ = require_srgb();
        let t = prepare(true, Some(p3_like_icc()), "t", Backend::Hardware)
            .expect("a P3 source transforms");
        assert!(t.destination_is_wide());
    }

    #[test]
    fn warp_sessions_keep_the_srgb_destination() {
        // D5's WARP exclusion: the same source on a WARP session keeps
        // today's sRGB destination — no F16P3 master can exist where no
        // wide draw arm runs.
        let _ = require_srgb();
        let t =
            prepare(true, Some(p3_like_icc()), "t", Backend::Warp).expect("a P3 source transforms");
        assert!(!t.destination_is_wide());
    }

    #[test]
    fn a_refused_wide_pass_falls_back_to_the_srgb_destination() {
        // The loader ladder's fallback, driven directly: the wide
        // 16-bit pass refused (injected), the SAME transform retargets
        // to sRGB and its 16-bit chain works again — destination no
        // longer wide.
        let _ = require_srgb();
        let mut t = prepare(true, Some(adobe_like_icc()), "t", Backend::Hardware)
            .expect("an AdobeRGB-like profile transforms");
        assert!(t.destination_is_wide());
        t.force16_fail.set(true);
        assert!(
            t.apply_f16(1, 1, FrameSrc::Rgba8(&[255u8, 0, 0, 255]))
                .is_none()
        );
        t.force16_fail.set(false);
        assert!(t.retarget_srgb(), "the sRGB destination rebuilds");
        assert!(!t.destination_is_wide());
        assert!(
            t.apply_f16(1, 1, FrameSrc::Rgba8(&[255u8, 0, 0, 255]))
                .is_some(),
            "the 16-bit chain runs again against sRGB"
        );
    }
}
