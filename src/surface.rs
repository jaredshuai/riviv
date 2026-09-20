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
//! #9's mipmap chain is HISTORY: #81 (ADR 0002 D7) retired it for regular
//! sizes — the GDI arm paints shrinks from the single full-resolution
//! face (`with_face_source`, the old level-0 arm generalized), and the
//! chain's runtime machinery (levels, budget, scratch DC, decode-side
//! pre-generation) is deleted. The pure level math stays in
//! [`crate::mip`] for #82's giant-image tiering; the D2D arm never had
//! mips.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HALFTONE, HBITMAP, HDC, HGDIOBJ, SRCCOPY, SelectObject,
    SetStretchBltMode, StretchBlt,
};

use crate::pixels::{PixelFrame, rotate_bgra_90_cw, rotate_bgra_270_cw};

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

/// The derived GDI half of a frame: the DIB section (a memcpy of the
/// master) selected into a private memory DC, ready for StretchBlt.
/// Built lazily ON DEMAND (#76: "GDI 面降级为按需派生") — the first
/// paint asks for it — so a huge frame's build cost (a 32 MiB-per-
/// megapixel memcpy) lands inside the paint
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

/// The transient intermediate an extreme-source giant's paint draws from
/// (#81, ADR 0002 D7's transition until #82's tiling). A face whose max
/// dimension reaches [`crate::stitch::STRETCH_SOURCE_STITCH_TRIGGER`]
/// cannot be stretched by a single full-rect call NOR by wide slice calls
/// (the #81 smoke census: 2^21 slices render from faces up to 6,291,456
/// wide and fail on 2^23-wide ones — the failure tracks the FACE width;
/// only 512-px slices read such faces, exactly the tiling upstream's mip
/// generation always used for them). So paint builds this relief ONCE per
/// paint: a fresh DIB sized at most `STRETCH_SOURCE_STITCH_TRIGGER / 2 + 1`
/// (2^21 + 1 — the integer-division divisor walk can leave one pixel over;
/// both sit far inside the census-proven single-blit band) on its max axis,
/// filled from the face
/// through [`crate::stitch::stitch_tiles_sized`] 512-px slices in HALFTONE
/// (upstream's generation mode, viv.c:14231), then the scene's ONE blit
/// runs from the relief. Transient by design — no chain, no cache, no
/// budget: the escape-arm doctrine until #82's D2D tiling owns giants.
pub(crate) struct GiantRelief {
    bitmap: HBITMAP,
    /// The relief's DC with the relief DIB selected — the paint blit's
    /// source. Valid for the struct's lifetime.
    pub(crate) memdc: HDC,
    stock: HGDIOBJ,
    /// The relief's dimensions (the blit's source extents).
    pub(crate) wide: i32,
    pub(crate) high: i32,
}

impl Drop for GiantRelief {
    fn drop(&mut self) {
        // SAFETY: we exclusively own the relief; restoring the DC's stock
        // bitmap before deleting it and then deleting the deselected DIB
        // is the documented GDI teardown order.
        unsafe {
            let _ = SelectObject(self.memdc, self.stock);
            let _ = DeleteDC(self.memdc);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        }
    }
}

/// Build the [`GiantRelief`] for a `mw x mh` face selected at `src_dc`.
/// The divisor is the power of two that lands the relief's max axis at or
/// one pixel over `STRETCH_SOURCE_STITCH_TRIGGER / 2` (the integer
/// division can leave a single pixel over) — inside the band a
/// single full-rect blit is proven to render from. Every failure cleans
/// up what it took and returns `None` (the caller degrades to sliced
/// blits — the paint-path doctrine, never a fatal inside the state
/// borrow). Runs on the UI thread; per-call objects only.
pub(crate) fn build_giant_relief(src_dc: HDC, mw: i32, mh: i32) -> Option<GiantRelief> {
    if src_dc.is_invalid() || mw <= 0 || mh <= 0 {
        return None;
    }
    let target = crate::stitch::STRETCH_SOURCE_STITCH_TRIGGER / 2;
    let mut k = 1i32;
    while mw.max(mh) / k > target {
        k <<= 1;
    }
    let wide = ((mw + k - 1) / k).max(1);
    let high = ((mh + k - 1) / k).max(1);
    let zeros = vec![0u8; wide as usize * high as usize * 4];
    let bitmap = create_bgra_dib(wide, high, &zeros).ok()?;
    // SAFETY: None gives a screen-compatible DC owned by this thread for
    // the relief's lifetime.
    let memdc = unsafe { CreateCompatibleDC(None) };
    if memdc.is_invalid() {
        // SAFETY: the DIB is owned and selected nowhere — plain
        // DeleteObject is the correct teardown.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        return None;
    }
    // SAFETY: `bitmap` is a valid owned DIB; the stock handle returned
    // here stays owned by the DC (never deleted by us).
    let stock = unsafe { SelectObject(memdc, HGDIOBJ(bitmap.0)) };
    if stock.is_invalid() {
        // SAFETY: selection failed, so the DC still holds its stock 1x1
        // bitmap — plain DeleteDC is correct; the DIB drops below.
        unsafe {
            let _ = DeleteDC(memdc);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        return None;
    }
    // Upstream's generation posture (viv.c:14231): HALFTONE on the
    // DESTINATION DC for the downscale pass, whatever tier the final
    // paint blit runs in.
    // SAFETY: plain mode setter on the live relief DC.
    unsafe {
        let _ = SetStretchBltMode(memdc, HALFTONE);
    }
    let whole = crate::zoom::BlitRect {
        dx: 0,
        dy: 0,
        dw: wide,
        dh: high,
        sx: 0,
        sy: 0,
        sw: mw,
        sh: mh,
    };
    for tile in crate::stitch::stitch_tiles_sized(
        crate::stitch::STITCH_TILE_SIZE,
        whole,
        (0, 0, wide, high),
    ) {
        // SAFETY: tiles come from the pure partition of the whole blit;
        // `src_dc` holds exactly the mw x mh face. Fail-soft per tile
        // like upstream's generation (viv.c:14237-14243) — one lost tile
        // costs a seam, not the relief.
        let _ = unsafe {
            StretchBlt(
                memdc,
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
    Some(GiantRelief {
        bitmap,
        memdc,
        stock,
        wide,
        high,
    })
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
    /// lifetime (a paint-path
    /// failure degrades to a blank frame, it must not fail loud inside
    /// the window-state borrow, PR #10 P1).
    face_stuck: bool,
}

impl Surface {
    /// Wrap the decode's `master` frame — pure ownership move, no GDI:
    /// the reply drain adopts the image (and runs the window auto-size)
    /// without waiting on any derivation, and the GDI face builds at the
    /// first paint. Infallible by construction.
    pub(crate) fn from_master(master: PixelFrame) -> Self {
        Surface {
            master,
            face: None,
            face_stuck: false,
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
    /// invalidate the GDI face (rebuilt from the new master at the next
    /// paint; there is no chain to invalidate since #81, ADR 0002 D7 —
    /// upstream freed every frame's mips here, viv.c:7742-7749). The
    /// frame's slot in the animation timeline is untouched. Fail-soft like
    /// upstream: a GDI failure leaves the frame exactly as it was
    /// (`_viv_orientate_hbitmap` returns 0 and the caller skips the swap)
    /// — which for the pure pass means only the allocation class, a
    /// process-level abort either way.
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
        };
        // The old face's DIB/DC die with their Drop (deselect-then-delete);
        // a fresh face may succeed even if the old one had failed.
        self.face = None;
        self.face_stuck = false;
        true
    }

    /// Run `f` with the face's source DC and the master's dimensions — the
    /// single-bitmap direct-draw source since #81 retired the mip chain (the
    /// old `with_mip_source` level-0 arm). Builds the face on demand; a
    /// build-failed frame hands `f` a null DC: the caller's blit fails and is
    /// swallowed, degrading to the letterbox only (paint-path doctrine).
    pub(crate) fn with_face_source<R>(&mut self, f: impl FnOnce(HDC, i32, i32) -> R) -> R {
        if !self.ensure_face() {
            return f(
                HDC::default(),
                self.master.width as i32,
                self.master.height as i32,
            );
        }
        let face = self.face.as_ref().expect("ensure_face just built it");
        f(
            face.memdc,
            self.master.width as i32,
            self.master.height as i32,
        )
    }
}
