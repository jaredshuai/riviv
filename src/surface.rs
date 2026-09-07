//! Image frames: decoded pixels held in a top-down 32bpp DIB section.
//!
//! Two types split by thread boundary (#4): [`DibFrame`] is what the
//! background decode produces — the DIB section alone, exactly the payload
//! upstream's replies carry (frame HBITMAPs, viv.c:2900/2989); it owns no
//! DC, so handing it to the UI thread is plain GDI-object transfer.
//! [`Surface`] is the UI-thread wrap that selects the DIB into a private
//! memory DC for StretchBlt (the render path of upstream
//! CreateCompatibleBitmap + SetDIBits -> mem DC -> StretchBlt,
//! viv.c:10263-10271, 4273). Memory DCs stay on the thread that created
//! them; the animation work (#3) holds one surface per displayed frame —
//! each costs a DC + a DIB, which is why the loader caps the frame count.
//!
//! #9 adds the mipmap chain: each frame carries downsampled DDB levels
//! ([`RawMip`], upstream `_viv_mipmap_t`, viv.c:365-384) sized by
//! [`crate::mip::mip_size`]. Levels are bare bitmaps — selected into a DC
//! only transiently (generation source, or the paint-time scratch DC) —
//! which mirrors upstream, where only the paint DC ever holds a mip, and
//! keeps every bitmap selected by at most one DC at a time. The chain is
//! pre-generated on the decode worker (`DibFrame::pregenerate_mips`,
//! upstream viv.c:10302/10316/10717/10749) and can be extended lazily on
//! the UI thread (`Surface::ensure_mips`, upstream's paint-time fill inside
//! `_viv_get_mipmap`); failures truncate the chain — a shallower level
//! still renders, where upstream's NULL propagates into not drawing the
//! image at all (viv.c:14226→14262→4167-4169; deliberate gentler
//! deviation, ADR 0001, README Differences).

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetCurrentObject, GetDC, HALFTONE,
    HBITMAP, HDC, HGDIOBJ, OBJ_BITMAP, ReleaseDC, SRCCOPY, STRETCH_BLT_MODE, SelectObject,
    SetStretchBltMode, StretchBlt,
};

use crate::mip;
use crate::pixels::rgba8_to_bgra_in_place;
use crate::stitch::stitch_tiles;
use crate::zoom::BlitRect;

/// Process-wide cap on live mip GDI objects (one DDB per level, plus each
/// mip-carrying surface's paint-time scratch DC), shared by the worker's
/// pre-generation AND the UI thread's lazy fills. Without a shared cap the
/// lazy path would bypass the loader's worker-side gate: every displayed
/// animation frame extends its own resident chain and a long big-frame
/// animation could push the process past the default 10000-object GDI
/// quota, after which even `CreateDIBSection` fails and the next open dies
/// through FatalSystem (review PR #18, engineering F1). The loader's
/// worker-side gate stays as a waste-avoidance early-out; THIS counter is
/// the real bound. Transient generation DCs (created and deleted within
/// one call) are not counted — the overshoot is at most a couple of
/// short-lived objects and their creation failure degrades, never crashes.
pub(crate) const MIP_GDI_OBJECT_BUDGET: usize = 1000;

static LIVE_MIP_GDI_OBJECTS: AtomicUsize = AtomicUsize::new(0);

/// Whether `needed` more mip objects fit under the shared budget.
fn mip_budget_allows(needed: usize) -> bool {
    LIVE_MIP_GDI_OBJECTS.load(Ordering::Relaxed) + needed <= MIP_GDI_OBJECT_BUDGET
}

/// One mipmap level: a screen-compatible DDB plus its dimensions
/// (upstream `_viv_mipmap_t`, viv.c:365-373 — upstream derives sizes from
/// the frame each time instead of storing them; storing avoids re-deriving
/// at every paint and keeps the level self-describing across threads).
///
/// Like the frame DIB, a DDB is a process-global GDI object with no thread
/// affinity, so worker -> UI handoff and teardown on either side are sound.
pub(crate) struct RawMip {
    bitmap: HBITMAP,
    wide: i32,
    high: i32,
}

// SAFETY: the struct is a GDI bitmap handle plus plain dimensions; DDBs are
// process-global with no thread affinity (the same handoff the frame DIB
// makes, upstream viv.c:10304/10318), so the raw pointer inside HBITMAP
// only makes std conservative about the move.
unsafe impl Send for RawMip {}

impl Drop for RawMip {
    fn drop(&mut self) {
        // SAFETY: we exclusively own the bitmap; it is selected into no DC
        // at drop time (generation and paint select transiently and always
        // restore first), so plain DeleteObject is the correct teardown.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        };
        LIVE_MIP_GDI_OBJECTS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// One decoded frame as a top-down 32bpp DIB section — the unit that
/// crosses the decode-worker -> UI thread boundary. GDI bitmaps are
/// process-global with no thread affinity, so creating it on the worker,
/// displaying it on the UI thread, and deleting it on either is sound.
pub(crate) struct DibFrame {
    bitmap: HBITMAP,
    width: i32,
    height: i32,
    /// Pre-generated mipmap levels 1..=k, attached by the worker after the
    /// frame decodes (upstream hangs the chain off each `_viv_frame_t`,
    /// viv.c:380-384). Ownership moves with the frame into its `Surface`.
    pub(crate) mips: Vec<RawMip>,
}

// SAFETY: the struct is a GDI bitmap handle plus plain dimensions. Bitmap
// handles are process-global (upstream ships them across threads the same
// way, viv.c:2900/2989); the raw pointer inside HBITMAP only makes std
// conservative about the move.
unsafe impl Send for DibFrame {}

impl DibFrame {
    /// `rgba` holds exactly `width * height * 4` bytes (converted to BGRA
    /// in place). Errors are plain system-level messages (GDI allocation
    /// failures only); the loader maps them into its two-layer taxonomy.
    pub(crate) fn from_rgba(width: u32, height: u32, rgba: &mut [u8]) -> Result<Self, String> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // negative = top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: `info` is a valid stack BITMAPINFO outliving the call; we own the
        // returned DIB section (no file mapping, no palette with BI_RGB).
        let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(|e| format!("CreateDIBSection failed: {e}"))?;
        if bits.is_null() {
            // SAFETY: bitmap was created above and is owned by us; nothing
            // references it yet, so plain DeleteObject is the correct teardown.
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            return Err("CreateDIBSection returned NULL bits".into());
        }
        rgba8_to_bgra_in_place(rgba);
        let byte_len = width as usize * height as usize * 4;
        debug_assert_eq!(rgba.len(), byte_len);
        // SAFETY: `bits` points to exactly width*height*4 writable bytes of the
        // freshly created section; `rgba` holds the same count (asserted above).
        unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), bits.cast::<u8>(), byte_len) };
        Ok(DibFrame {
            bitmap,
            width: width as i32,
            height: height as i32,
            mips: Vec::new(),
        })
    }

    /// Pre-generate mip levels 1..=`target` for this frame on the calling
    /// (worker) thread — upstream `_viv_get_mipmap` with the request-time
    /// viewport halved, called per frame before the reply is queued
    /// (viv.c:10302/10316/10717/10749). Each level is stitched down from
    /// the PREVIOUS level (viv.c:14229-14236), keeping chain sizes exactly
    /// `mip_size` at every step. A generation failure truncates the chain
    /// instead of failing the frame: the frame itself already decodes
    /// fine, and paint can still extend the chain lazily or render a
    /// shallower level (README deviation; upstream lets a NULL propagate
    /// into not drawing the image, viv.c:4167-4169).
    ///
    /// Thread contract: DCs are created and destroyed on the calling
    /// thread; the frame's bitmap is bare (selected nowhere) until this
    /// returns.
    pub(crate) fn pregenerate_mips(&mut self, target: u32) {
        if target == 0 {
            return;
        }
        // SAFETY: no DC needs to be selected here; None gives a
        // screen-compatible DC, created and destroyed on this thread.
        let src_dc = unsafe { CreateCompatibleDC(None) };
        if src_dc.is_invalid() {
            return;
        }
        // SAFETY: `self.bitmap` is a valid GDI bitmap owned by us and
        // selected nowhere else (the frame is bare on the worker).
        let src_stock = unsafe { SelectObject(src_dc, HGDIOBJ(self.bitmap.0)) };
        if src_stock.is_invalid() {
            // SAFETY: selection failed, so the DC still holds its stock
            // bitmap — plain DeleteDC is the correct teardown.
            unsafe {
                let _ = DeleteDC(src_dc);
            };
            return;
        }
        let (mut src_w, mut src_h) = (self.width, self.height);
        for level in 1..=target {
            let (dst_w, dst_h) = mip::mip_size(self.width, self.height, level);
            match generate_mip(src_dc, src_w, src_h, dst_w, dst_h) {
                Ok(mip_level) => {
                    src_w = dst_w;
                    src_h = dst_h;
                    // SAFETY: the just-created level bitmap is owned by us
                    // and selected nowhere; re-selecting it as the source
                    // for the next level replaces the previous selection.
                    // A failed re-selection must stop the chain: the next
                    // iteration would stretch against the DC's stale
                    // (larger) bitmap using this level's source rect
                    // (review PR #18 F3).
                    // SAFETY (the SelectObject itself): src_dc is valid.
                    let reselected = unsafe { SelectObject(src_dc, HGDIOBJ(mip_level.bitmap.0)) };
                    if reselected.is_invalid() {
                        self.mips.push(mip_level);
                        break;
                    }
                    self.mips.push(mip_level);
                }
                Err(_) => break,
            }
        }
        // SAFETY: restore the stock bitmap before deleting the DC (GDI
        // will not delete a bitmap still selected into a DC — the frame's
        // own DIB must survive this teardown).
        unsafe {
            let _ = SelectObject(src_dc, src_stock);
            let _ = DeleteDC(src_dc);
        }
    }
}

impl Drop for DibFrame {
    fn drop(&mut self) {
        // SAFETY: we exclusively own the bitmap; no DC has selected it while
        // it is a bare DibFrame (Surfaces unselect before dropping their DIB).
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        };
    }
}

/// Stretch one mip level down from `src_dc` (previous level, already
/// selected) into a fresh DDB of `dst_w x dst_h` — upstream's generation
/// block (viv.c:14216-14257): HALFTONE on the destination DC and
/// [`stitch_tiles`] for the ≥32768 sources huge frames produce ("we will
/// end up with sharp tile edges, but it's better than showing a black
/// image", viv.c:14233-14235).
///
/// Failure tree, every branch restoring what it took: DC creation failure
/// → nothing to clean; bitmap failure → delete the DC; selection failure →
/// delete both (the DC still holds its stock bitmap). A per-tile
/// StretchBlt failure keeps the level — upstream continues past it with a
/// debug print (viv.c:14237-14243) — so one bad tile costs a seam, not the
/// chain. Thread contract: DCs created and destroyed on the calling
/// thread.
fn generate_mip(
    src_dc: HDC,
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
) -> Result<RawMip, String> {
    // Shared process budget first: the DDB this creates is a live mip
    // object until the level drops (review PR #18 F1 — lazy fills must
    // contend with pre-generation for the same cap).
    if !mip_budget_allows(1) {
        return Err("mip GDI object budget exhausted".into());
    }
    // The bitmap MUST be created against the SCREEN DC: a memory DC fresh
    // from CreateCompatibleDC has the 1x1 MONOCHROME stock bitmap selected,
    // and CreateCompatibleBitmap then yields a 1-bit mono bitmap — every
    // level would rasterize to white/black garbage. Upstream passes its
    // GetDC(0) handle here for exactly this reason (viv.c:14216/14226).
    // SAFETY: GetDC(None) is the whole-screen DC, released below on every
    // path out of this block.
    let screen_dc = unsafe { GetDC(None) };
    if screen_dc.is_invalid() {
        // SAFETY: reading the thread's last error immediately after the failed call.
        let gle = unsafe { GetLastError().0 };
        return Err(format!("mip GetDC failed (GLE={gle})"));
    }
    // SAFETY: screen_dc is valid; the bitmap is compatible with the screen
    // (color) and owned by us from creation until the RawMip drops.
    let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, dst_w, dst_h) };
    // SAFETY: the screen DC is borrowed, not owned — released immediately
    // after the creation call (win32 GetDC/ReleaseDC pairing).
    unsafe {
        let _ = ReleaseDC(None, screen_dc);
    }
    if bitmap.is_invalid() {
        // SAFETY: reading the thread's last error immediately after the failed call.
        let gle = unsafe { GetLastError().0 };
        return Err(format!("mip CreateCompatibleBitmap failed (GLE={gle})"));
    }
    // SAFETY: None gives a screen-compatible DC owned by this thread.
    let dst_dc = unsafe { CreateCompatibleDC(None) };
    if dst_dc.is_invalid() {
        // SAFETY: reading the thread's last error immediately after the failed call.
        let gle = unsafe { GetLastError().0 };
        // SAFETY: the bitmap is owned and selected nowhere — plain
        // DeleteObject is the correct teardown.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        };
        return Err(format!("mip CreateCompatibleDC failed (GLE={gle})"));
    }
    // SAFETY: `bitmap` is a valid owned bitmap; on failure the DC still
    // holds the stock bitmap, so both are independently deletable.
    let dst_stock = unsafe { SelectObject(dst_dc, HGDIOBJ(bitmap.0)) };
    if dst_stock.is_invalid() {
        // SAFETY: selection failed; both objects are still individually
        // owned and unselected, so plain teardown is correct.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dst_dc);
        };
        return Err("mip SelectObject failed".into());
    }
    // SAFETY: upstream sets HALFTONE on the destination mem DC (viv.c:14231)
    // and restores the previous mode after the stretch (viv.c:14245).
    let last_mode = unsafe { SetStretchBltMode(dst_dc, HALFTONE) };
    let whole = BlitRect {
        dx: 0,
        dy: 0,
        dw: dst_w,
        dh: dst_h,
        sx: 0,
        sy: 0,
        sw: src_w,
        sh: src_h,
    };
    for tile in stitch_tiles(whole, (0, 0, dst_w, dst_h)) {
        // SAFETY: tiles come from the pure partition of the whole blit;
        // src_dc is a valid DC holding exactly the src_w x src_h previous
        // level. Fail-soft per tile like upstream (viv.c:14237-14243).
        let _ = unsafe {
            StretchBlt(
                dst_dc,
                tile.dx,
                tile.dy,
                tile.dw,
                tile.dh,
                Some(src_dc),
                tile.sx,
                tile.sy,
                tile.sw,
                tile.sh,
                SRCCOPY,
            )
        };
    }
    // SAFETY: restore the mode and stock bitmap before deleting the DC.
    unsafe {
        let _ = SetStretchBltMode(dst_dc, STRETCH_BLT_MODE(last_mode));
        let _ = SelectObject(dst_dc, dst_stock);
        let _ = DeleteDC(dst_dc);
    }
    LIVE_MIP_GDI_OBJECTS.fetch_add(1, Ordering::Relaxed);
    Ok(RawMip {
        bitmap,
        wide: dst_w,
        high: dst_h,
    })
}

/// A frame selected into a private memory DC, ready for StretchBlt —
/// built on the UI thread from a worker-produced [`DibFrame`] so the DC
/// never leaves the thread that created it.
pub(crate) struct Surface {
    frame: DibFrame,
    memdc: HDC,
    old_bitmap: HGDIOBJ,
    /// Mipmap levels 1..=k (moved from the frame at wrap time). Bare DDBs:
    /// selected only transiently — into a temp DC while extending the
    /// chain, or into `mip_scratch` for painting.
    mips: Vec<RawMip>,
    /// Lazily created DC that paints select mip bitmaps into (upstream
    /// selects the chosen mip into its per-paint mem DC, viv.c:4173);
    /// invalid until a paint first needs a level > 0, so mip-less frames
    /// cost nothing. `mip_stock` is its original stock bitmap, restored
    /// before the DC is deleted.
    mip_scratch: HDC,
    mip_stock: HGDIOBJ,
    /// The level extension that failed, if any (review PR #18): without
    /// this memory every paint under GDI pressure would retry the full
    /// generation loop. Never retried for this surface's lifetime — the
    /// next image naturally starts fresh.
    mips_stuck: Option<u32>,
}

impl Surface {
    /// Take ownership of `frame`, select it into a fresh memory DC. Must
    /// run on the thread that will render (the UI thread): memory DCs
    /// belong to their creating thread.
    pub(crate) fn from_frame(mut frame: DibFrame) -> Result<Self, String> {
        // SAFETY: no DC needs to be selected here; None gives a screen-compatible DC.
        let memdc = unsafe { CreateCompatibleDC(None) };
        if memdc.is_invalid() {
            // SAFETY: reading the thread's last error immediately after the failed call.
            let gle = unsafe { GetLastError().0 };
            return Err(format!("CreateCompatibleDC failed (GLE={gle})"));
        }
        // SAFETY: `frame.bitmap` is a valid GDI bitmap handle owned by us.
        let old_bitmap = unsafe { SelectObject(memdc, HGDIOBJ(frame.bitmap.0)) };
        if old_bitmap.is_invalid() {
            // SAFETY: selection failed, so the DC still holds its stock 1x1
            // bitmap — plain DeleteDC is the correct teardown; the DibFrame
            // drops itself.
            unsafe {
                let _ = DeleteDC(memdc);
            };
            return Err("SelectObject failed to select the DIB".into());
        }
        // The chain moves with the frame (mem::take: DibFrame implements
        // Drop, so its fields cannot move out directly).
        let mips = std::mem::take(&mut frame.mips);
        Ok(Surface {
            frame,
            memdc,
            old_bitmap,
            mips,
            mip_scratch: HDC::default(),
            mip_stock: HGDIOBJ::default(),
            mips_stuck: None,
        })
    }

    pub(crate) fn width(&self) -> i32 {
        self.frame.width
    }

    pub(crate) fn height(&self) -> i32 {
        self.frame.height
    }

    /// Extend the mip chain to `target` levels if needed, on the calling
    /// (UI) thread — upstream fills missing levels inside `_viv_get_mipmap`
    /// during WM_PAINT the same way (viv.c:14200-14258). Returns the
    /// deepest USABLE level (≤ target; smaller when the chain truncated or
    /// a level failed) — the caller must paint from the returned level,
    /// never the requested one.
    ///
    /// SAFETY-relevant invariant: never fails loud. Generation trouble
    /// truncates the chain (a shallower level still renders); this runs
    /// inside WM_PAINT under the window-state borrow, where the fatal
    /// modal's message pump would alias `&mut` state (PR #10 P1).
    ///
    /// Cost bound: extending by one level reads the current tail, whose
    /// width is at most twice the target level's (the pre-generation
    /// viewport bound plus one step, see `mip.rs` docs) — a cheap stretch
    /// WHEN the chain already starts near the target. A chain built from
    /// empty here (the worker skipped it, or the window shrank a lot)
    /// starts from the full-resolution frame and can take a slow
    /// full-chain pass on the UI thread — same as upstream, which also
    /// generates inside `_viv_get_mipmap` during paint (viv.c:14200-14258).
    pub(crate) fn ensure_mips(&mut self, image_w: i32, image_h: i32, target: u32) -> u32 {
        // 1. Extend the chain if short (and not already failed) — the
        //    early returns below must never skip step 2: a pre-generated
        //    chain skips extension but still needs the scratch DC.
        if self.mips_stuck.is_none() && (self.mips.len() as u32) < target {
            self.extend_mips(image_w, image_h, target);
        }
        // 2. Any level > 0 is only paintable through the scratch DC —
        //    create it lazily here, once mips are actually in play
        //    (mip-less frames never pay for it), under the shared object
        //    budget. Failure drops the whole chain: the frame still
        //    renders from level 0.
        if target > 0 && !self.mips.is_empty() && self.mip_scratch.is_invalid() {
            if !mip_budget_allows(1) {
                self.mips.clear();
                self.mips_stuck = Some(1);
                return 0;
            }
            // SAFETY: None gives a screen-compatible DC; owned by this
            // (UI) thread for the surface's lifetime.
            let scratch = unsafe { CreateCompatibleDC(None) };
            if scratch.is_invalid() {
                self.mips.clear();
                self.mips_stuck = Some(1);
                return 0;
            }
            // Capture the stock bitmap NOW: `SelectObject(.., NULL)` does
            // not restore anything (review PR #18 F2) — without a real
            // handle, with_mip_source's deselect would leave the level
            // selected into the DC until teardown, resting on
            // undocumented delete-while-selected behavior.
            // SAFETY: scratch is valid and holds its stock bitmap; the
            // returned handle stays owned by the DC (never deleted).
            self.mip_stock = unsafe { GetCurrentObject(scratch, OBJ_BITMAP) };
            self.mip_scratch = scratch;
            LIVE_MIP_GDI_OBJECTS.fetch_add(1, Ordering::Relaxed);
        }
        self.mips.len().min(target as usize) as u32
    }

    /// The extension loop proper: generate levels len+1..=target from the
    /// chain's current tail, marking `mips_stuck` on the first failure.
    fn extend_mips(&mut self, image_w: i32, image_h: i32, target: u32) {
        // Transient source DC for levels ≥ 2 (bare DDBs must be selected
        // to blit; the frame's own level-1 source is already `memdc`).
        let mut temp_src: Option<(HDC, HGDIOBJ)> = None;
        while (self.mips.len() as u32) < target {
            let level = self.mips.len() as u32 + 1;
            let (dst_w, dst_h) = mip::mip_size(image_w, image_h, level);
            let (src_dc, src_w, src_h) = if level == 1 {
                (self.memdc, self.frame.width, self.frame.height)
            } else {
                let prev = &self.mips[(level - 2) as usize];
                if temp_src.is_none() {
                    // SAFETY: None gives a screen-compatible DC; created
                    // and destroyed on this thread within this call.
                    let dc = unsafe { CreateCompatibleDC(None) };
                    if dc.is_invalid() {
                        self.mips_stuck = Some(level);
                        break;
                    }
                    // SAFETY: the DC holds its stock bitmap until this
                    // first selection; remember the stock for teardown.
                    let stock = unsafe { SelectObject(dc, HGDIOBJ(prev.bitmap.0)) };
                    if stock.is_invalid() {
                        // SAFETY: selection failed; the DC still holds its
                        // stock bitmap — plain DeleteDC is correct.
                        unsafe {
                            let _ = DeleteDC(dc);
                        };
                        self.mips_stuck = Some(level);
                        break;
                    }
                    temp_src = Some((dc, stock));
                }
                let (dc, _) = temp_src.expect("created or broke above");
                // SAFETY: `prev` is a valid owned bitmap; re-selecting it
                // here replaces the previous level's selection (each level
                // is source exactly once, in order). A failed re-selection
                // must stop the chain — the stale selection would feed the
                // next level a wrong-sized source (review PR #18 F3).
                let reselected = unsafe { SelectObject(dc, HGDIOBJ(prev.bitmap.0)) };
                if reselected.is_invalid() {
                    self.mips_stuck = Some(level);
                    break;
                }
                (dc, prev.wide, prev.high)
            };
            match generate_mip(src_dc, src_w, src_h, dst_w, dst_h) {
                Ok(mip_level) => self.mips.push(mip_level),
                Err(_) => {
                    self.mips_stuck = Some(level);
                    break;
                }
            }
        }
        if let Some((dc, stock)) = temp_src {
            // SAFETY: restore the temp DC's stock bitmap before deleting it
            // so the last-selected level's DDB is deletable by its owner.
            unsafe {
                let _ = SelectObject(dc, stock);
                let _ = DeleteDC(dc);
            }
        }
    }

    /// Run `f` with the source DC and dimensions of mipmap `level`
    /// (0 = the frame itself). Mip bitmaps are selected into the scratch
    /// DC for the call and deselected after — upstream selects the chosen
    /// mip into its paint mem DC the same transient way (viv.c:4173).
    pub(crate) fn with_mip_source<R>(
        &mut self,
        level: u32,
        f: impl FnOnce(HDC, i32, i32) -> R,
    ) -> R {
        if level == 0 {
            return f(self.memdc, self.frame.width, self.frame.height);
        }
        let mip = &self.mips[(level - 1) as usize];
        // SAFETY: the scratch DC exists whenever mips do (ensure_mips
        // created it before any level could be returned); the level bitmap
        // is owned by us and selected nowhere else.
        unsafe {
            let _ = SelectObject(self.mip_scratch, HGDIOBJ(mip.bitmap.0));
        }
        let (wide, high) = (mip.wide, mip.high);
        let result = f(self.mip_scratch, wide, high);
        // SAFETY: restore the stock bitmap so the level's DDB is never
        // selected when it (or the DC) drops.
        unsafe {
            let _ = SelectObject(self.mip_scratch, self.mip_stock);
        }
        result
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: we exclusively own memdc/bitmap; restoring the old bitmap before
        // deleting the DC and letting the DibFrame delete the bitmap is the
        // documented GDI teardown order. The scratch DC (if created) is
        // restored to its captured stock bitmap first for the same reason;
        // the mip DDBs drop themselves afterwards, selected into nothing
        // (each also releasing its slot in the shared object budget).
        unsafe {
            if !self.mip_scratch.is_invalid() {
                let _ = SelectObject(self.mip_scratch, self.mip_stock);
                let _ = DeleteDC(self.mip_scratch);
                LIVE_MIP_GDI_OBJECTS.fetch_sub(1, Ordering::Relaxed);
            }
            let _ = SelectObject(self.memdc, self.old_bitmap);
            let _ = DeleteDC(self.memdc);
        }
    }
}
