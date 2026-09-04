//! Image surface: decoded pixels held in a top-down 32bpp DIB section,
//! selected into a private memory DC — the same render path as upstream
//! (CreateCompatibleBitmap + SetDIBits -> mem DC -> StretchBlt, viv.c:10263-10271, 4273).
//!
//! M2 seam: per-frame surfaces for animation (#3) and mipmap surfaces (#9)
//! land here beside the single-frame surface.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HDC, HGDIOBJ, SelectObject,
};

use crate::loader::LoadError;
use crate::pixels::rgba8_to_bgra_in_place;

pub(crate) struct Surface {
    memdc: HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl Surface {
    pub(crate) fn from_rgba(width: u32, height: u32, rgba: &mut [u8]) -> Result<Self, LoadError> {
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
            .map_err(|e| LoadError::System(format!("CreateDIBSection failed: {e}")))?;
        if bits.is_null() {
            // SAFETY: bitmap was created above and is owned by us; no DC
            // references it yet, so plain DeleteObject is the correct teardown.
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            return Err(LoadError::System(
                "CreateDIBSection returned NULL bits".into(),
            ));
        }
        rgba8_to_bgra_in_place(rgba);
        let byte_len = width as usize * height as usize * 4;
        debug_assert_eq!(rgba.len(), byte_len);
        // SAFETY: `bits` points to exactly width*height*4 writable bytes of the
        // freshly created section; `rgba` holds the same count (asserted above).
        unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), bits.cast::<u8>(), byte_len) };
        // SAFETY: no DC needs to be selected here; None gives a screen-compatible DC.
        let memdc = unsafe { CreateCompatibleDC(None) };
        if memdc.is_invalid() {
            // SAFETY: reading the thread's last error immediately after the failed call.
            let gle = unsafe { GetLastError().0 };
            // SAFETY: bitmap is owned by us and was never selected into a DC.
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            return Err(LoadError::System(format!(
                "CreateCompatibleDC failed (GLE={gle})"
            )));
        }
        // SAFETY: `bitmap` is a valid GDI bitmap handle owned by us.
        let old_bitmap = unsafe { SelectObject(memdc, HGDIOBJ(bitmap.0)) };
        if old_bitmap.is_invalid() {
            // SAFETY: selection failed, so the DC still holds its stock 1x1
            // bitmap — plain DeleteDC then DeleteObject is the correct teardown.
            unsafe {
                let _ = DeleteDC(memdc);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            return Err(LoadError::System(
                "SelectObject failed to select the DIB".to_string(),
            ));
        }
        Ok(Surface {
            memdc,
            bitmap,
            old_bitmap,
            width: width as i32,
            height: height as i32,
        })
    }

    /// Memory DC holding the surface's DIB, for StretchBlt's source parameter.
    pub(crate) fn memdc(&self) -> HDC {
        self.memdc
    }

    pub(crate) fn width(&self) -> i32 {
        self.width
    }

    pub(crate) fn height(&self) -> i32 {
        self.height
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: we exclusively own memdc/bitmap; restoring the old bitmap before
        // deleting both is the documented GDI teardown order.
        unsafe {
            let _ = SelectObject(self.memdc, self.old_bitmap);
            let _ = DeleteDC(self.memdc);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        }
    }
}
