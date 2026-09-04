//! WM_PAINT rendering: fill the client area, center the fitted image,
//! HALFTONE StretchBlt from the surface's memory DC.
//!
//! M2 seam: alpha-composited transparent drawing (#3), the 32768-px stitch
//! path and mipmap selection (#9), and zoom/pan-aware source rectangles (#7)
//! land here.

use windows::Win32::Foundation::{GetLastError, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GetStockObject, HALFTONE, HBRUSH, PAINTSTRUCT, SRCCOPY,
    SetBrushOrgEx, SetStretchBltMode, StretchBlt, WHITE_BRUSH,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::fit::fit_shrink;
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
        // Windowed background default is white (config.c:65-67).
        let _ = FillRect(hdc, &client, HBRUSH(GetStockObject(WHITE_BRUSH).0));
        // SAFETY (for state_of, nested in this fn's outer unsafe block): the
        // borrow lives only across the GDI draw calls below — none pump
        // messages, so no second `state_of` borrow can be taken while this
        // one is live.
        if let Some(surface) = state_of(hwnd).and_then(|s| s.surface.as_ref()) {
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
