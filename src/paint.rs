//! WM_PAINT rendering: fill the client area, center the fitted image,
//! HALFTONE StretchBlt from the current frame's memory DC.
//!
//! M2 seams left: the 32768-px stitch path and mipmap selection (#9), and
//! zoom/pan-aware source rectangles (#7). Alpha compositing (#3) is landed —
//! transparent pixels are resolved against the windowed background at decode
//! time, so this path blits opaque pixels only.

use windows::Win32::Foundation::{COLORREF, GetLastError, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HALFTONE, HGDIOBJ, PAINTSTRUCT,
    SRCCOPY, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::fit::fit_shrink;
use crate::pixels::WINDOWED_BACKGROUND_RGB;
use crate::window::{fatal, state_of};

pub(crate) fn paint(hwnd: HWND) {
    // SAFETY: all GDI calls are bracketed by BeginPaint/EndPaint on the WM_PAINT
    // DC; handles are valid for the duration of the message.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_invalid() {
            // Already inside this function's outer unsafe block.
            let gle = GetLastError().0;
            // System-level failure (ADR 0001): painting with a null DC would
            // silently produce a blank client.
            fatal(&format!("BeginPaint failed (GLE={gle})"));
        }
        let mut client = RECT::default();
        // Fail-soft on purpose: upstream viv.c:4070/4098 also ignores
        // GetClientRect's return and reads the (zeroed) rect — a failed query
        // yields a degenerate paint, not a dead window (ADR 0001 leaves
        // paint-path diagnostics to the debug-log channel landing in M2).
        let _ = GetClientRect(hwnd, &mut client);
        let cw = (client.right - client.left).max(1);
        let ch = (client.bottom - client.top).max(1);
        // Windowed background fill: a solid brush from the same constant the
        // decode path composites transparent pixels against — the two must
        // never diverge or composited images show a fringe. Upstream creates
        // and deletes this brush per paint too (viv.c:4396-4407).
        let [bg_r, bg_g, bg_b] = WINDOWED_BACKGROUND_RGB;
        // Already inside this function's outer unsafe block.
        let brush = CreateSolidBrush(COLORREF(
            (u32::from(bg_b) << 16) | (u32::from(bg_g) << 8) | u32::from(bg_r),
        ));
        if !brush.is_invalid() {
            let _ = FillRect(hdc, &client, brush);
            // Already inside this function's outer unsafe block; the brush
            // was created above and FillRect does not retain it, so plain
            // DeleteObject is the documented teardown.
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
        // SAFETY (for state_of, nested in this fn's outer unsafe block): the
        // borrow lives only across the GDI draw calls below — none pump
        // messages, so no second `state_of` borrow can be taken while this
        // one is live.
        if let Some(image) = state_of(hwnd).and_then(|s| s.image.as_ref()) {
            let surface = image.surface();
            let (dw, dh) = fit_shrink(surface.width(), surface.height(), cw, ch);
            let dx = client.left + (cw - dw) / 2;
            let dy = client.top + (ch - dh) / 2;
            // Upstream shrink path: HALFTONE + brush-org realignment anchored to
            // the destination image (viv.c:4205-4209 uses -rx,-ry) so the dither
            // pattern does not drift as the centered image moves on resize.
            let _ = SetStretchBltMode(hdc, HALFTONE);
            let _ = SetBrushOrgEx(hdc, -dx, -dy, None);
            // Fail-soft by design: upstream viv.c:4278 also continues past a failed
            // StretchBlt (debug_printf only) — one bad frame must not kill the
            // window. M2 adds a debug-log channel to surface GLE.
            let _ = StretchBlt(
                hdc,
                dx,
                dy,
                dw,
                dh,
                Some(surface.memdc()),
                0,
                0,
                surface.width(),
                surface.height(),
                SRCCOPY,
            );
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
