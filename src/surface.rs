//! Image frames: the CPU master ([`crate::pixels::PixelFrame`]) plus its
//! UI-thread GDI derivation.
//!
//! #76 (ADR 0002 D3) inverts the old ownership: the decode worker
//! produces pure memory — a top-down BGRA `PixelFrame`, no GDI objects
//! cross the thread boundary — and [`Surface`] is the UI-thread wrap
//! that derives the GDI face from it: a DIB section (a memcpy of the
//! master) selected into a private memory DC for StretchBlt (the render
//! path of upstream CreateCompatibleBitmap + SetDIBits -> mem DC ->
//! StretchBlt, viv.c:10263-10271, 4273). Memory DCs stay on the thread
//! that created them; the animation work (#3) holds one surface per
//! displayed frame — each costs a DC + a DIB, which is why the loader
//! caps the frame count.
//!
//! #9 adds the mipmap chain: each frame carries downsampled DDB levels
//! ([`RawMip`], upstream `_viv_mipmap_t`, viv.c:365-384) sized by
//! [`crate::mip::mip_size`]. Levels are bare bitmaps — selected into a
//! DC only transiently (generation source, or the paint-time scratch
//! DC) — which mirrors upstream, where only the paint DC ever holds a
//! mip, and keeps every bitmap selected by at most one DC at a time.
//! Since #76 the chain lives wholly on the UI thread: the decode-side
//! `mip_target` decision (upstream viv.c:10302/10316/10717/10749) rides
//! the frame to the first `ensure_mips` call, which materializes the
//! pre-generation share AT PAINT (the reply drain stays GDI-free) and
//! extends lazily from there
//! (upstream is itself a paint-time fill inside `_viv_get_mipmap`);
//! failures truncate the chain — a shallower level
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
use crate::pixels::{PixelFrame, rotate_bgra_90_cw, rotate_bgra_270_cw};
use crate::stitch::stitch_tiles;
use crate::zoom::BlitRect;

/// Process-wide cap on live mip GDI objects (one DDB per level, plus each
/// mip-carrying surface's paint-time scratch DC), shared by the UI
/// thread's pre-generation share of the FIRST paint (`ensure_mips`) AND
/// later lazy paint fills (both on the UI thread since #76; the counter
/// stays atomic for the day it is queried cross-thread). Without a shared
/// cap every displayed
/// animation frame extends its own resident chain and a long big-frame
/// animation could push the process past the default 10000-object GDI
/// quota, after which even the frame face's `CreateDIBSection` fails and
/// every image degrades to blank (review PR #18, engineering F1; the
/// failure shape itself went from fail-loud to paint-degrade with #76's
/// lazy faces). The loader's
/// decode-side gate (the `mip_target` each frame carries) stays as a
/// waste-avoidance early-out; THIS counter is the real bound. Transient
/// generation DCs (created and deleted within one call) are not counted —
/// the overshoot is at most a couple of short-lived objects and their
/// creation failure degrades, never crashes.
pub(crate) const MIP_GDI_OBJECT_BUDGET: usize = 1000;

static LIVE_MIP_GDI_OBJECTS: AtomicUsize = AtomicUsize::new(0);

/// Whether `needed` more mip objects fit under the shared budget.
fn mip_budget_allows(needed: usize) -> bool {
    LIVE_MIP_GDI_OBJECTS.load(Ordering::Relaxed) + needed <= MIP_GDI_OBJECT_BUDGET
}

/// One mipmap level: a screen-compatible DDB plus its dimensions
/// (upstream `_viv_mipmap_t`, viv.c:365-373 — upstream derives sizes from
/// the frame each time instead of storing them; storing avoids re-deriving
/// at every paint and keeps the level self-describing).
///
/// Since #76 the chain is UI-thread-only (materialized at the first
/// paint and by lazy fills, never crossing a thread boundary), so no
/// `unsafe impl Send` is needed — the compiler now rejects any accidental
/// cross-thread move.
pub(crate) struct RawMip {
    bitmap: HBITMAP,
    wide: i32,
    high: i32,
}

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

/// Build a top-down 32bpp DIB section holding a memcpy of `pixels`
/// (`width * height * 4` BGRA bytes) — the GDI derivation of the CPU
/// master (#76). Errors are plain system-level messages (GDI allocation
/// failures only); callers DEGRADE — `ensure_face` blanks the frame at
/// paint (never a fatal; ADR 0002 D5) and the clipboard copy bails — the
/// fail-loud reply mapping died with the worker-side GDI it described.
/// The returned bitmap is bare
/// (selected into no DC); the caller owns it.
pub(crate) fn create_bgra_dib(width: i32, height: i32, pixels: &[u8]) -> Result<HBITMAP, String> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // negative = top-down rows
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
    debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
    // SAFETY: `bits` points to exactly width*height*4 writable bytes of the
    // freshly created section; `pixels` holds the same count (asserted above).
    unsafe {
        std::ptr::copy_nonoverlapping(
            pixels.as_ptr(),
            bits.cast::<u8>(),
            width as usize * height as usize * 4,
        )
    };
    Ok(bitmap)
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

/// The derived GDI half of a frame: the DIB section (a memcpy of the
/// master) selected into a private memory DC, ready for StretchBlt.
/// Built lazily ON DEMAND (#76: "GDI 面降级为按需派生") — the first
/// paint asks for it — so a huge frame's build cost (a 32 MiB-per-
/// megapixel memcpy plus the mip pre-generation) lands inside the paint
/// that consumes it, AFTER the reply drain has adopted the image and
/// run the window auto-size. Building it eagerly at reply time instead
/// would stall the drain handler for hundreds of milliseconds, wedging
/// every cross-thread window probe (PrintWindow) that lands there
/// between its pre-drain rect read and its post-drain content capture.
struct Face {
    bitmap: HBITMAP,
    memdc: HDC,
    old_bitmap: HGDIOBJ,
}

impl Drop for Face {
    fn drop(&mut self) {
        // SAFETY: we exclusively own the face; restoring the DC's stock
        // bitmap before deleting it and then deleting the deselected DIB
        // is the documented GDI teardown order.
        unsafe {
            let _ = SelectObject(self.memdc, self.old_bitmap);
            let _ = DeleteDC(self.memdc);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        }
    }
}

/// A frame's GDI face: the CPU master ([`PixelFrame`], the source of
/// truth since #76) plus its lazily derived UI-thread face. The master
/// inside is what the RGB readout, the clipboard image blit and the
/// rotate pass read directly; only the paint path ever pays for the
/// DIB/DC derivation.
pub(crate) struct Surface {
    master: PixelFrame,
    /// The derived DIB + DC, built by the first paint (see [`Face`]).
    face: Option<Face>,
    /// Set when a face build failed: never retried for this surface's
    /// lifetime (the same doctrine as `mips_stuck` — a paint-path
    /// failure degrades to a blank frame, it must not fail loud inside
    /// the window-state borrow, PR #10 P1).
    face_stuck: bool,
    /// The decode-side mip pre-generation decision, consumed by the
    /// first `ensure_mips` call (the chain builds to at least this depth
    /// — the worker-side pre-generation's old contract, upstream
    /// viv.c:10302/10316/10717/10749).
    pregen_target: u32,
    /// Mipmap levels 1..=k, generated on this thread. Bare DDBs:
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
    /// Wrap the decode's `master` frame — pure ownership move, no GDI:
    /// the reply drain adopts the image (and runs the window auto-size)
    /// without waiting on any derivation, and the GDI face builds at the
    /// first paint. Infallible by construction.
    pub(crate) fn from_master(mut master: PixelFrame) -> Self {
        let pregen_target = master.mip_target;
        master.mip_target = 0; // consumed by the first ensure_mips
        Surface {
            master,
            face: None,
            face_stuck: false,
            pregen_target,
            mips: Vec::new(),
            mip_scratch: HDC::default(),
            mip_stock: HGDIOBJ::default(),
            mips_stuck: None,
        }
    }

    /// The CPU master this surface derives from — the direct read source
    /// for the RGB status sample (#47), the clipboard image blit (#41)
    /// and anything else that must not pay a GDI roundtrip (#76).
    pub(crate) fn master(&self) -> &PixelFrame {
        &self.master
    }

    pub(crate) fn width(&self) -> i32 {
        self.master.width as i32
    }

    pub(crate) fn height(&self) -> i32 {
        self.master.height as i32
    }

    /// Build the GDI face if it does not exist yet (the on-demand
    /// derivation, #76). Runs on the UI thread — memory DCs belong to
    /// their creating thread. `false` = the build failed (or failed
    /// before and is stuck): the caller degrades (paint skips the blit),
    /// never fails loud (this runs under the paint borrow).
    fn ensure_face(&mut self) -> bool {
        if self.face.is_some() {
            return true;
        }
        if self.face_stuck {
            return false;
        }
        let face = (|| {
            let bitmap = create_bgra_dib(
                self.master.width as i32,
                self.master.height as i32,
                &self.master.pixels,
            )?;
            // SAFETY: no DC needs to be selected here; None gives a
            // screen-compatible DC.
            let memdc = unsafe { CreateCompatibleDC(None) };
            if memdc.is_invalid() {
                // SAFETY: reading the thread's last error immediately
                // after the failed call.
                let gle = unsafe { GetLastError().0 };
                // SAFETY: the DIB is owned by us and selected nowhere —
                // plain DeleteObject is the correct teardown.
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                }
                return Err(format!("CreateCompatibleDC failed (GLE={gle})"));
            }
            // SAFETY: `bitmap` is a valid GDI bitmap handle owned by us.
            let old_bitmap = unsafe { SelectObject(memdc, HGDIOBJ(bitmap.0)) };
            if old_bitmap.is_invalid() {
                // SAFETY: selection failed, so the DC still holds its
                // stock 1x1 bitmap — plain DeleteDC is the correct
                // teardown; the DIB drops with this block's scope.
                unsafe {
                    let _ = DeleteDC(memdc);
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                };
                return Err("SelectObject failed to select the DIB".into());
            }
            Ok(Face {
                bitmap,
                memdc,
                old_bitmap,
            })
        })();
        match face {
            Ok(face) => {
                self.face = Some(face);
                true
            }
            Err(msg) => {
                // A degraded blank frame with zero on-screen trace would be
                // undiagnosable from a user report — leave a stderr
                // breadcrumb (the repo's established degrade-diagnostics
                // channel; review PR #84 N2).
                eprintln!("frame face build failed, degrading to blank: {msg}");
                self.face_stuck = true;
                false
            }
        }
    }

    /// #43 in-place rotation (upstream `_viv_edit_rotate`'s memory pass,
    /// viv.c:7729-7749 over `_viv_orientate_hbitmap` viv.c:13671-13850):
    /// rotate the CPU master with the pure pixels helper (the old
    /// GetDIBits roundtrip is gone — the master IS the pixels, #76) and
    /// invalidate every derivation: the GDI face (rebuilt from the new
    /// master at the next paint) and the mips (upstream frees every
    /// frame's chain, viv.c:7742-7749 — paint lazily regenerates from
    /// the new orientation). The frame's slot in the animation timeline
    /// is untouched. Fail-soft like upstream: a GDI failure leaves the
    /// frame exactly as it was (`_viv_orientate_hbitmap` returns 0 and
    /// the caller skips the swap) — which for the pure pass means only
    /// the allocation class, a process-level abort either way.
    pub(crate) fn rotate(&mut self, clockwise: bool) -> bool {
        let wide = self.master.width as usize;
        let high = self.master.height as usize;
        if wide == 0 || high == 0 {
            return false;
        }
        let mut rotated = vec![0u8; wide * high * 4];
        if clockwise {
            rotate_bgra_90_cw(&self.master.pixels, wide, high, &mut rotated);
        } else {
            rotate_bgra_270_cw(&self.master.pixels, wide, high, &mut rotated);
        }
        self.master = PixelFrame {
            pixels: rotated.into_boxed_slice(),
            width: high as u32,
            height: wide as u32,
            mip_target: 0,
        };
        // The old face's DIB/DC die with their Drop (deselect-then-delete);
        // a fresh face may succeed even if the old one had failed.
        self.face = None;
        self.face_stuck = false;
        // The stale chain and the decode-side pre-generation depth die with
        // the orientation — the new master's first paint builds the chain
        // from the NEW dimensions (an unconsumed pregen_target would feed
        // the OLD orientation's decode-time depth into the new chain;
        // review PR #84 F3).
        self.pregen_target = 0;
        // SAFETY: RawMip's Drop deletes each stale level's DDB — none is
        // selected anywhere (they only ever select transiently).
        self.mips.clear();
        self.mips_stuck = None; // a fresh chain may succeed now
        true
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
        // 0. The on-demand GDI face (#76): level 0 paints from it and
        //    level 1 generates from it — a frame that cannot build its
        //    face degrades to a blank render (the stuck flag prevents a
        //    per-paint retry storm).
        if !self.ensure_face() {
            return 0;
        }
        // 1. Extend the chain if short (and not already failed) — the
        //    early returns below must never skip step 2: a pre-generated
        //    chain skips extension but still needs the scratch DC. The
        //    FIRST call also honors the decode-side pre-generation
        //    decision (`pregen_target`) in TWO shares with different
        //    retry contracts (review PR #84 N1):
        //    - the pregen share: truncation does NOT mark stuck — the
        //      worker-side pre-generation's old contract (a truncated
        //      pregen stays retryable at later paints, where eased GDI
        //      pressure may let a fresh pass fill deeper);
        //    - the render share (the current paint's selected depth):
        //      truncation marks stuck (review PR #18 — no per-paint
        //      retry storm).
        let pregen = std::mem::take(&mut self.pregen_target);
        if pregen > 0 && self.mips_stuck.is_none() && (self.mips.len() as u32) < pregen {
            self.extend_mips(image_w, image_h, pregen);
            self.mips_stuck = None;
        }
        if self.mips_stuck.is_none() && (self.mips.len() as u32) < target {
            self.extend_mips(image_w, image_h, target);
        }
        let target = target.max(pregen);
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
    /// Only callable with a face built (level 1 sources its DC).
    fn extend_mips(&mut self, image_w: i32, image_h: i32, target: u32) {
        // Transient source DC for levels ≥ 2 (bare DDBs must be selected
        // to blit; the frame's own level-1 source is the face's DC).
        let mut temp_src: Option<(HDC, HGDIOBJ)> = None;
        let face_dc = self
            .face
            .as_ref()
            .expect("extend_mips requires the face (ensure_mips built it)")
            .memdc;
        while (self.mips.len() as u32) < target {
            let level = self.mips.len() as u32 + 1;
            let (dst_w, dst_h) = mip::mip_size(image_w, image_h, level);
            let (src_dc, src_w, src_h) = if level == 1 {
                (face_dc, self.master.width as i32, self.master.height as i32)
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
    /// A face-less (build-failed) level 0 hands `f` a null DC: the
    /// caller's blit fails and is swallowed, degrading to the letterbox
    /// only — never a crash, never a fatal (paint-path doctrine).
    pub(crate) fn with_mip_source<R>(
        &mut self,
        level: u32,
        f: impl FnOnce(HDC, i32, i32) -> R,
    ) -> R {
        if level == 0 {
            let dc = self.face.as_ref().map_or(HDC::default(), |face| face.memdc);
            return f(dc, self.master.width as i32, self.master.height as i32);
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
        // SAFETY: the scratch DC (if created) is restored to its captured
        // stock bitmap before deletion so the last-selected level's DDB
        // stays deletable by its owner; the mip DDBs drop themselves
        // afterwards, selected into nothing (each also releasing its slot
        // in the shared object budget). The face and the master drop as
        // plain fields (the face's own Drop does the GDI teardown; the
        // master is plain memory).
        unsafe {
            if !self.mip_scratch.is_invalid() {
                let _ = SelectObject(self.mip_scratch, self.mip_stock);
                let _ = DeleteDC(self.mip_scratch);
                LIVE_MIP_GDI_OBJECTS.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}
