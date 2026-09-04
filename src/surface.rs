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
//! M2 seam: mipmap surfaces (#9) land here.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};

use crate::pixels::rgba8_to_bgra_in_place;

/// One decoded frame as a top-down 32bpp DIB section — the unit that
/// crosses the decode-worker -> UI thread boundary. GDI bitmaps are
/// process-global with no thread affinity, so creating it on the worker,
/// displaying it on the UI thread, and deleting it on either is sound.
pub(crate) struct DibFrame {
    bitmap: HBITMAP,
    width: i32,
    height: i32,
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
        })
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

/// A frame selected into a private memory DC, ready for StretchBlt —
/// built on the UI thread from a worker-produced [`DibFrame`] so the DC
/// never leaves the thread that created it.
pub(crate) struct Surface {
    frame: DibFrame,
    memdc: HDC,
    old_bitmap: HGDIOBJ,
}

impl Surface {
    /// Take ownership of `frame`, select it into a fresh memory DC. Must
    /// run on the thread that will render (the UI thread): memory DCs
    /// belong to their creating thread.
    pub(crate) fn from_frame(frame: DibFrame) -> Result<Self, String> {
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
        Ok(Surface {
            frame,
            memdc,
            old_bitmap,
        })
    }

    /// Memory DC holding the surface's DIB, for StretchBlt's source parameter.
    pub(crate) fn memdc(&self) -> HDC {
        self.memdc
    }

    pub(crate) fn width(&self) -> i32 {
        self.frame.width
    }

    pub(crate) fn height(&self) -> i32 {
        self.frame.height
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: we exclusively own memdc/bitmap; restoring the old bitmap before
        // deleting the DC and letting the DibFrame delete the bitmap is the
        // documented GDI teardown order.
        unsafe {
            let _ = SelectObject(self.memdc, self.old_bitmap);
            let _ = DeleteDC(self.memdc);
        }
    }
}
