//! The UI-thread CPU master holder ([`crate::pixels::PixelFrame`], #76).
//!
//! [`Surface`] wraps the decode worker's pure memory — a top-down BGRA
//! `PixelFrame`, no GDI objects cross the thread boundary — and hands it
//! to everything that must read pixels on the UI thread. Since #90 the
//! render arm is D2D-only: the GPU stack uploads straight from
//! `master()`'s bytes, so the surface holds no render-side derivation at
//! all — it is the single CPU source of truth for the frame (the RGB
//! readout, the clipboard image copy and the rotate pass read it
//! directly).
//!
//! What lives here besides the wrap is the DIB-section allocation
//! primitive ([`create_bgra_dib_raw`]/[`create_bgra_dib`]), whose sole
//! remaining consumer is the clipboard's `CF_BITMAP`/`CF_DIB` copy
//! (chrome GDI, not rendering).

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
    HGDIOBJ,
};

use crate::pixels::{PixelFrame, rotate_bgra_90_cw, rotate_bgra_270_cw};

/// Create a bare top-down 32bpp BGRA DIB section of `width x height` and
/// hand back the handle plus its bit pointer — the GDI allocation
/// primitive the clipboard's CF_BITMAP/CF_DIB copy builds on
/// (selected into no DC); the caller owns both. The caller must write or
/// zero `width * height * 4` bytes through `bits` before reading the
/// section (the section's initial contents are not guaranteed).
pub(crate) fn create_bgra_dib_raw(
    width: i32,
    height: i32,
) -> Result<(HBITMAP, *mut c_void), String> {
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
    Ok((bitmap, bits))
}

/// [`create_bgra_dib_raw`] + one memcpy of `pixels` into the fresh
/// section. Errors are plain system-level messages (GDI allocation
/// failures only); the clipboard copy bails on them (its degrade is a
/// failed copy operation, never a fatal — ADR 0001's user-level tier).
/// The returned bitmap is bare (selected into no DC); the caller owns it.
pub(crate) fn create_bgra_dib(width: i32, height: i32, pixels: &[u8]) -> Result<HBITMAP, String> {
    let (bitmap, bits) = create_bgra_dib_raw(width, height)?;
    debug_assert_eq!(pixels.len(), width as usize * height as usize * 4);
    // SAFETY: `bits` points at exactly width*height*4 writable bytes of
    // the freshly created section; `pixels` holds the same count
    // (asserted above).
    unsafe {
        std::ptr::copy_nonoverlapping(
            pixels.as_ptr(),
            bits.cast::<u8>(),
            width as usize * height as usize * 4,
        )
    };
    Ok(bitmap)
}

/// The frame's CPU master ([`PixelFrame`], the source of truth since #76)
/// wrapped for UI-thread use. The master inside is what the RGB readout,
/// the clipboard image copy and the rotate pass read directly; the D2D
/// render arm uploads from it without any intermediate copy.
pub(crate) struct Surface {
    master: PixelFrame,
}

impl Surface {
    /// Wrap the decode's `master` frame — pure ownership move, no GDI:
    /// the reply drain adopts the image (and runs the window auto-size)
    /// without paying any derivation. Infallible by construction.
    pub(crate) fn from_master(master: PixelFrame) -> Self {
        Surface { master }
    }

    /// The CPU master — the direct read source for the RGB status sample
    /// (#47), the clipboard image copy (#41), the D2D uploads (#76/#80)
    /// and anything else that must not pay a derivation roundtrip.
    pub(crate) fn master(&self) -> &PixelFrame {
        &self.master
    }

    pub(crate) fn width(&self) -> i32 {
        self.master.width as i32
    }

    pub(crate) fn height(&self) -> i32 {
        self.master.height as i32
    }

    /// #43 in-place rotation (upstream `_viv_edit_rotate`'s memory pass,
    /// viv.c:7729-7749 over `_viv_orientate_hbitmap` viv.c:13671-13850):
    /// rotate the CPU master with the pure pixels helper (the old
    /// GetDIBits roundtrip is gone — the master IS the pixels, #76).
    /// The frame's slot in the animation timeline is untouched. Fail-soft
    /// like upstream: only the allocation class can fail, a process-level
    /// abort either way.
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
            // Rotation moves geometry, not encoding — the rotated frame
            // stays in its master's content space (#140).
            content_space: self.master.content_space,
        };
        true
    }
}
