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
    DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
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
