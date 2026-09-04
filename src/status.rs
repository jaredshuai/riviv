//! Status bar common control (#5): creation, height query, and the
//! measure → SB_SETPARTS → SB_SETTEXT update — the GDI shell over the pure
//! text/width model in `text.rs` (upstream `_viv_status_show`,
//! `_viv_get_status_high`, `_viv_status_update`, viv.c:10932-11440).
//!
//! Layout: `[main text (elastic)] [frame counter "n / m"] [dimensions
//! "W x H (N KB)"]` — upstream's preload / pixel-info parts are skipped
//! (riviv has neither feature); the counter and dimension parts keep
//! upstream's trailing-slot semantics.

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetDeviceCaps, GetTextExtentPoint32W, HDC, HGDIOBJ, LOGPIXELSY, ReleaseDC, SelectObject,
};
use windows::Win32::UI::Controls::{SB_SETPARTS, SB_SETTEXTW, SBARS_SIZEGRIP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetSystemMetrics, GetWindowRect, HMENU, SM_CXBORDER, SM_CXEDGE, SM_CXVSCROLL,
    SendMessageW, WINDOW_STYLE, WM_GETFONT, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_EX_COMPOSITED, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::text::{
    min_status_part_wide, status_dimension_text, status_frame_text, status_main_text,
    status_part_edges, to_wide,
};

/// Child-window id for the status bar (upstream `VIV_ID_STATUS`, viv.h:196).
pub(crate) const STATUS_BAR_ID: u16 = 100;

/// Everything the status bar shows, handed to `update` as one snapshot so
/// the pure text model decides what each part says.
pub(crate) struct StatusSnapshot {
    /// A load session is in flight — main part shows "Loading...".
    pub(crate) loading: bool,
    /// The requested path does not exist — "File not found.".
    pub(crate) file_not_found: bool,
    /// The current display's load failed at user level — "Failed to load
    /// image.".
    pub(crate) load_failed: bool,
    /// 1-based frame position / loaded frame count; `None` when blank.
    pub(crate) frame: Option<(usize, usize)>,
    /// Canvas size; `None` when blank.
    pub(crate) dimensions: Option<(i32, i32)>,
    /// Byte size of the displayed file, if known (skipped when 0/unknown,
    /// viv.c:11152).
    pub(crate) file_bytes: Option<u64>,
    /// Main-window client width — the part edges are laid out against it.
    pub(crate) client_wide: i32,
}

/// Create the status bar as a child of `parent` (upstream
/// `_viv_status_show(1)`, viv.c:10932-10963). `WS_EX_COMPOSITED` and the
/// clip styles are upstream's; `SBARS_SIZEGRIP` requests the grip (the
/// common control itself paints it only while the parent is resizable and
/// not maximized).
pub(crate) fn create(parent: HWND, hinstance: HINSTANCE) -> Result<HWND, String> {
    // SAFETY: parent/hinstance are live; the class is the system's status
    // bar (comctl32 — upstream registers no ICC classes either,
    // viv.c:10939-10947).
    unsafe {
        CreateWindowExW(
            WS_EX_COMPOSITED,
            w!("msctls_statusbar32"),
            PCWSTR::null(),
            WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPCHILDREN.0 | WS_CLIPSIBLINGS.0 | SBARS_SIZEGRIP,
            ),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(STATUS_BAR_ID as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| format!("status bar CreateWindowExW failed: {e}"))
}

/// The bar's current height in pixels, 0 without a bar (upstream
/// `_viv_get_status_high`, viv.c:11427-11440) — subtracted from the render
/// area at paint time and added back when sizing the window to an image.
pub(crate) fn height(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    // SAFETY: hwnd is our live child window; a failed query reads the
    // zeroed rect — height 0 degrades paint, never crashes (same fail-soft
    // posture as paint's GetClientRect).
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    rect.bottom - rect.top
}

/// Recompute part widths and refresh all texts (upstream
/// `_viv_status_update`). Texts are only SET when they differ from what
/// the control already shows (upstream `_viv_status_set`'s GETTEXT
/// compare, viv.c:11415-11425) so a steady state never redraws.
pub(crate) fn update(hwnd: HWND, snapshot: &StatusSnapshot) {
    let main = status_main_text(
        snapshot.loading,
        snapshot.file_not_found,
        snapshot.load_failed,
    );
    let frame_text = match snapshot.frame {
        Some((position, total)) => status_frame_text(position, total),
        None => String::new(),
    };
    let dimension_text = match snapshot.dimensions {
        Some((wide, high)) => status_dimension_text(Some(wide), Some(high), snapshot.file_bytes),
        None => String::new(),
    };

    // SAFETY: every call targets our own child window on the UI thread;
    // the DC is borrowed for the measurements and released before the
    // function returns.
    unsafe {
        // System DPI from the desktop DC (the process is system-DPI-aware
        // — SetProcessDPIAware in window::run — so one reading covers the
        // bar; upstream reads LOGPIXELSX from its os DC, os.c:817).
        let screen = GetDC(None);
        let dpi = if screen.is_invalid() {
            96
        } else {
            let d = GetDeviceCaps(Some(screen), LOGPIXELSY);
            let _ = ReleaseDC(None, screen);
            d as u32
        };
        let hdc = GetDC(Some(hwnd));
        if hdc.is_invalid() {
            // Fail-soft: without the control's DC the texts cannot be
            // measured; skip this refresh (the next update retries). Same
            // posture as paint's failed GetClientRect.
            return;
        }
        // Measure with the font the control actually draws with (upstream
        // WM_GETFONT, viv.c:11237-11239); a zero font means the control has
        // not picked one yet — measure with the DC's own font rather than
        // skipping (a skipped update leaves the bar empty until the next
        // state change).
        let font = SendMessageW(hwnd, WM_GETFONT, None, None);
        let sizes = {
            let old = if font.0 != 0 {
                SelectObject(hdc, HGDIOBJ(font.0 as *mut _))
            } else {
                HGDIOBJ::default()
            };
            let sizes = (
                text_extent(hdc, &frame_text),
                text_extent(hdc, &dimension_text),
            );
            if font.0 != 0 {
                SelectObject(hdc, old);
            }
            Some(sizes)
        };
        let _ = ReleaseDC(Some(hwnd), hdc);
        let Some((frame_w, dimension_w)) = sizes else {
            return;
        };

        let margin = GetSystemMetrics(SM_CXEDGE) * 5;
        let grip = GetSystemMetrics(SM_CXVSCROLL) + GetSystemMetrics(SM_CXBORDER);
        let edges = status_part_edges(
            snapshot.client_wide,
            frame_w,
            dimension_w,
            margin,
            grip,
            min_status_part_wide(dpi),
        );
        SendMessageW(
            hwnd,
            SB_SETPARTS,
            Some(WPARAM(edges.len())),
            Some(LPARAM(edges.as_ptr() as isize)),
        );
        set_text(hwnd, 0, main);
        set_text(hwnd, 1, &frame_text);
        set_text(hwnd, 2, &dimension_text);
    }
}

/// Pixel width of a status text with the DC's selected font; empty text
/// measures 0 (upstream only measures non-empty buffers, viv.c:11241).
fn text_extent(hdc: HDC, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let wide = to_wide(text);
    // to_wide appends a NUL that must not be measured.
    let units = &wide[..wide.len() - 1];
    let mut size = windows::Win32::Foundation::SIZE::default();
    // SAFETY: hdc is live with the status font selected; `units` outlives
    // the call. A failed measure yields width 0 — the part collapses and
    // the next update retries.
    let _ = unsafe { GetTextExtentPoint32W(hdc, units, &mut size) };
    size.cx
}

/// Upstream `_viv_status_set` (viv.c:11415-11425) avoids redundant SETs.
/// Cross-thread SB_GETTEXTW reads are unreliable on the v5 status bar (the
/// caller's UIPI boundary), so the last-written text per part is cached in
/// a window property and compared against instead of reading it back.
fn set_text(hwnd: HWND, part: usize, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{GetPropW, SetPropW};
    // Per-part property names ("RivivSt0".."RivivSt2"); the handle is a
    // hash of the last-written text so a repeat write is detectable.
    let name: Vec<u16> = format!("RivivSt{part}\0").encode_utf16().collect();
    let hash = {
        let mut h: usize = 5381;
        for b in text.bytes() {
            h = h.wrapping_mul(33).wrapping_add(usize::from(b));
        }
        h
    };
    // SAFETY: hwnd is our live child; the property names are distinct
    // NUL-terminated literals; GetPropW/SetPropW are plain queries on it.
    let current = unsafe { GetPropW(hwnd, PCWSTR(name.as_ptr())) };
    if current.0 as usize == hash && !text.is_empty() {
        return; // unchanged — skip the redraw
    }
    let new = to_wide(text);
    // SAFETY: `new` is NUL-terminated and outlives the call; SetPropW on a
    // hash-valued property never fails for a live child window (fail-soft:
    // a dropped cache write just re-sends the text next refresh).
    unsafe {
        SendMessageW(
            hwnd,
            SB_SETTEXTW,
            Some(WPARAM(part)),
            Some(LPARAM(new.as_ptr() as isize)),
        );
        let _ = SetPropW(
            hwnd,
            PCWSTR(name.as_ptr()),
            Some(windows::Win32::Foundation::HANDLE(hash as *mut _)),
        );
    }
}
