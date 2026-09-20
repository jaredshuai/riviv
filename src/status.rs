//! Status bar common control (#5): creation, height query, and the
//! measure → SB_SETPARTS → SB_SETTEXT update — the GDI shell over the pure
//! text/width model in `text.rs` (upstream `_viv_status_show`,
//! `_viv_get_status_high`, `_viv_status_update`, viv.c:10932-11440).
//!
//! Layout: `[main text (elastic)] [preload indicator] [POS] [RGB] [frame
//! counter "n / m"] [dimensions "W x H (N KB)"]` — the preload part exists
//! only while its text is non-empty (upstream pushes its SB_SETPARTS
//! boundary inside the non-empty check, viv.c:11312-11316); the POS/RGB
//! parts (#47) ALWAYS exist (upstream's part-array guards test the array
//! pointer, viv.c:11320-11329 — always true), sitting zero-width and
//! invisible until pixel-info shows text in them.

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetTextExtentPoint32W, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::UI::Controls::{
    SB_GETPARTS, SB_GETRECT, SB_SETPARTS, SB_SETTEXTW, SBARS_SIZEGRIP,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, GWL_WNDPROC, GetSystemMetrics, GetWindowRect, HMENU,
    SM_CXBORDER, SM_CXEDGE, SM_CXVSCROLL, SendMessageW, SetWindowLongPtrW, WINDOW_STYLE,
    WM_GETFONT, WM_LBUTTONDOWN, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_COMPOSITED,
    WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::text::{
    min_status_part_wide, status_dimension_text, status_frame_text, status_main_text,
    status_part_edges, status_pixel_pos_text, status_pixel_rgb_text, status_preload_text, to_wide,
};

/// Child-window id for the status bar (upstream `VIV_ID_STATUS`, viv.h:196).
pub(crate) const STATUS_BAR_ID: u16 = 100;

/// The temp-text expiry timer (#47; upstream `VIV_ID_STATUS_TEMP_TEXT_TIMER`
/// — armed at 3000 ms whenever a flash text lands, viv.c:11757).
pub(crate) const TEMP_TEXT_TIMER_ID: usize = 4;

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
    /// A slideshow is running — "Slideshow playing" below every verdict
    /// (#37; upstream viv.c:11374-11377).
    pub(crate) slideshow: bool,
    /// The 3-second flash text — outranks every verdict while it runs
    /// (#47; upstream `_viv_status_temp_text`, viv.c:11351-11353).
    pub(crate) temp_text: Option<String>,
    /// A preload load is decoding its first frame — the "PRELOAD" part
    /// shows and the elastic main part shrinks (upstream viv.c:11210-11214,
    /// #40).
    pub(crate) preload_pending: bool,
    /// The source-pixel coordinate under the cursor, when pixel-info is on
    /// and the cursor is over the image — the POS part's text (#47;
    /// upstream viv.c:11217-11219).
    pub(crate) pixel: Option<(i32, i32)>,
    /// The sampled color of that source pixel — the RGB part's text
    /// (upstream viv.c:11220-11221; kept from the last valid sample while
    /// the coordinate is off-image — unobservable, the part goes empty).
    pub(crate) pixel_rgb: (u8, u8, u8),
    /// 1-based frame position / loaded frame count; `None` when blank.
    pub(crate) frame: Option<(usize, usize)>,
    /// The frames-remaining form (`config_frame_minus`, viv.c:11187-11203).
    pub(crate) frame_remaining: bool,
    /// Canvas size; `None` when blank.
    pub(crate) dimensions: Option<(i32, i32)>,
    /// Byte size of the displayed file, if known (skipped when 0/unknown,
    /// viv.c:11152).
    pub(crate) file_bytes: Option<u64>,
    /// The effective D2D backend label riding the dimension part (#80:
    /// `Some("d2d/hw")` / `Some("d2d/warp")` while a D2D stack renders,
    /// `None` on the gdi baseline — no suffix, byte-identical to upstream).
    pub(crate) backend: Option<&'static str>,
    /// Main-window client width — the part edges are laid out against it.
    pub(crate) client_wide: i32,
}

/// Create the status bar as a child of `parent` (upstream
/// `_viv_status_show(1)`, viv.c:10932-10963). `WS_EX_COMPOSITED` and the
/// clip styles are upstream's; `SBARS_SIZEGRIP` requests the grip (the
/// common control itself paints it only while the parent is resizable and
/// not maximized). The caller must have registered the common-control
/// classes first (upstream init's `InitCommonControlsEx`, viv.c:5236-5242).
/// The bar is then subclassed for the part-0 drag-to-move (#45; upstream
/// `_viv_status_proc`, viv.c:11543-11569 — gated on
/// `config_toolbar_move_window` like the strip and menu-bar drags).
pub(crate) fn create(parent: HWND, hinstance: HINSTANCE) -> Result<HWND, String> {
    // SAFETY: parent/hinstance are live; the class is comctl32's status
    // bar, registered by the caller's InitCommonControlsEx.
    let hwnd = unsafe {
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
    .map_err(|e| format!("status bar CreateWindowExW failed: {e}"))?;
    subclass_for_drag(hwnd);
    Ok(hwnd)
}

/// The bar's original wndproc, stashed by the subclass swap. A process
/// owns at most one bar at a time (fullscreen destroys and recreates);
/// each create re-swaps and overwrites — the previous window is gone by
/// then, its proc value moot.
static OLD_STATUS_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn subclass_for_drag(hwnd: HWND) {
    // SAFETY: hwnd is our fresh child on this thread; the swap hands back
    // the control's own proc, stored for the CallWindowProc forward. The
    // fn-pointer round-trip goes through *const () per the lint's advice.
    let old = unsafe {
        SetWindowLongPtrW(
            hwnd,
            GWL_WNDPROC,
            status_drag_proc as *const () as usize as isize,
        )
    };
    OLD_STATUS_PROC.store(old, std::sync::atomic::Ordering::Relaxed);
}

/// The subclass (upstream `_viv_status_proc`, viv.c:11543-11569): a
/// left-button press on part 0 (the elastic main text) enters the window
/// move loop while `toolbar_move_window` is set; everything else — the
/// size grip included, which lives in the control's own NC handling —
/// reaches the original proc unchanged.
unsafe extern "system" fn status_drag_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_LBUTTONDOWN
        && crate::window::toolbar_move_window_enabled(hwnd)
        && statusbar_index_from_x(hwnd, (lparam.0 & 0xffff) as i16 as i32) == Some(0)
    {
        // SAFETY: the move loop pumps; no borrows are live out here (the
        // gate read its config inside its own short borrow).
        crate::window::start_move_window(hwnd);
        return LRESULT(0);
    }
    let old = OLD_STATUS_PROC.load(std::sync::atomic::Ordering::Relaxed);
    if old == 0 {
        // The swap never happened (theoretical); degrade to the default
        // procedure.
        // SAFETY: the parameters are exactly this callback's own.
        return unsafe {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wparam, lparam)
        };
    }
    // SAFETY: `old` is the status control's own proc captured by the
    // SetWindowLongPtrW swap; CallWindowProc takes it as the fn-pointer
    // flavor of WNDPROC (Option<fn>), so the isize transmutes to the raw
    // fn and rides in Some.
    unsafe {
        CallWindowProcW(
            Some(std::mem::transmute::<
                isize,
                unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
            >(old)),
            hwnd,
            msg,
            wparam,
            lparam,
        )
    }
}

/// `os_statusbar_index_from_x` (os.c:1341-1382): the part whose SB_GETRECT
/// contains `x`; 0 when the click is inside the bar but on no measured
/// part ("simple?"), None outside the bar. Same-process sends only.
fn statusbar_index_from_x(hwnd: HWND, x: i32) -> Option<i32> {
    let mut rect = RECT::default();
    // SAFETY: read-only query on our own child; a failure reads zeroed
    // and the x bounds check fails first.
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect) };
    if x < 0 || x >= rect.right - rect.left {
        return None;
    }
    // SAFETY: hwnd is ours; SB_GETPARTS with a null array only returns
    // the count.
    let count = unsafe { SendMessageW(hwnd, SB_GETPARTS, Some(WPARAM(0)), None) }.0 as i32;
    for i in 0..count {
        // SAFETY: hwnd is ours; the RECT outlives the call.
        let ok = unsafe {
            SendMessageW(
                hwnd,
                SB_GETRECT,
                Some(WPARAM(i as usize)),
                Some(LPARAM(&mut rect as *mut RECT as isize)),
            )
        }
        .0 != 0;
        if ok && x >= rect.left && x < rect.right {
            return Some(i);
        }
    }
    Some(0)
}

/// The bar's current height in pixels, 0 without a bar (upstream
/// `_viv_get_status_high` on a NULL `_viv_status_hwnd`, viv.c:11427-11440)
/// — subtracted from the render area at paint time and added back when
/// sizing the window to an image.
pub(crate) fn height(hwnd: HWND) -> i32 {
    if hwnd.is_invalid() {
        return 0;
    }
    let mut rect = RECT::default();
    // SAFETY: hwnd is our live child window; a failed query reads the
    // zeroed rect — height 0 degrades paint, never crashes (same fail-soft
    // posture as paint's GetClientRect).
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    rect.bottom - rect.top
}

/// Recompute part widths and refresh all texts (upstream
/// `_viv_status_update`, which no-ops on a NULL bar). Texts are only SET
/// when they differ from what the control already shows (upstream
/// `_viv_status_set`'s GETTEXT compare, viv.c:11415-11425) so a steady
/// state never redraws.
pub(crate) fn update(hwnd: HWND, snapshot: &StatusSnapshot) {
    if hwnd.is_invalid() {
        return; // no bar (creation failed / not yet created)
    }
    // The flash text outranks every verdict (upstream's temp-text arm sits
    // at the TOP of the main-text chain, viv.c:11351-11353).
    let main = match &snapshot.temp_text {
        Some(temp) => temp.clone(),
        None => status_main_text(
            snapshot.loading,
            snapshot.file_not_found,
            snapshot.load_failed,
            snapshot.slideshow,
        )
        .to_string(),
    };
    let preload_text = status_preload_text(snapshot.preload_pending);
    // Both pixel parts carry text only for a valid coordinate (upstream
    // gates on x/y >= 0, viv.c:11217-11219); the PARTS themselves always
    // exist (see the module header).
    let pixel_pos_text = snapshot
        .pixel
        .map(|(x, y)| status_pixel_pos_text(x, y))
        .unwrap_or_default();
    let pixel_rgb_text = snapshot
        .pixel
        .map(|_| status_pixel_rgb_text(snapshot.pixel_rgb))
        .unwrap_or_default();
    let frame_text = match snapshot.frame {
        Some((position, total)) => status_frame_text(position, total, snapshot.frame_remaining),
        None => String::new(),
    };
    let dimension_text = match snapshot.dimensions {
        Some((wide, high)) => status_dimension_text(
            Some(wide),
            Some(high),
            snapshot.file_bytes,
            snapshot.backend,
        ),
        None => String::new(),
    };

    // SAFETY: every call targets our own child window on the UI thread;
    // the DC is borrowed for the measurements and released before the
    // function returns.
    unsafe {
        // System DPI by design (#79's LOGPIXELS audit): the part-width
        // formulas serve this v5.82 bar, whose font is system-DPI — so the
        // scaling source matches the thing being measured on every
        // monitor (upstream reads the same number from its os DC,
        // os.c:818; the pre-#79 screen-DC read returned this value).
        // SAFETY: resolved in the CALLING THREAD's DPI context — GetDpiForSystem
        // is only "process-wide" for aware threads (an unaware thread would
        // read 96). The UI thread's PMv2 default (checked at startup in
        // window::run) makes this the real system DPI; the 96 floor only
        // guards a failed (0) return.
        let dpi = GetDpiForSystem().max(96);
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
                // Non-empty text owns its part even when the measure fails
                // (a 0 would make status_part_edges DROP the preload part
                // while set_text below still writes it — the frame/dimension
                // texts would shift one part left until the next refresh;
                // upstream keys the boundary push on the text too,
                // viv.c:11312's `if (*preload_buf)`).
                if preload_text.is_empty() {
                    0
                } else {
                    text_extent(hdc, preload_text).max(1)
                },
                if pixel_pos_text.is_empty() {
                    0
                } else {
                    text_extent(hdc, &pixel_pos_text).max(1)
                },
                if pixel_rgb_text.is_empty() {
                    0
                } else {
                    text_extent(hdc, &pixel_rgb_text).max(1)
                },
                text_extent(hdc, &frame_text),
                text_extent(hdc, &dimension_text),
            );
            if font.0 != 0 {
                SelectObject(hdc, old);
            }
            sizes
        };
        let _ = ReleaseDC(Some(hwnd), hdc);
        let (preload_w, pos_w, rgb_w, frame_w, dimension_w) = sizes;

        let margin = GetSystemMetrics(SM_CXEDGE) * 5;
        let grip = GetSystemMetrics(SM_CXVSCROLL) + GetSystemMetrics(SM_CXBORDER);
        let edges = status_part_edges(
            snapshot.client_wide,
            preload_w,
            pos_w,
            rgb_w,
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
        // Part indices follow the edges: the preload part sits at 1 and
        // exists only while its text is non-empty (upstream's dynamic
        // part push, viv.c:11312-11316); the POS/RGB parts ALWAYS take
        // their slots (with empty text when nothing is under the cursor);
        // frame/dimension shift with the pair (viv.c:11382-11412).
        set_text(hwnd, 0, &main);
        let mut part = 1;
        if !preload_text.is_empty() {
            set_text(hwnd, part, preload_text);
            part += 1;
        }
        set_text(hwnd, part, &pixel_pos_text);
        part += 1;
        set_text(hwnd, part, &pixel_rgb_text);
        part += 1;
        set_text(hwnd, part, &frame_text);
        part += 1;
        set_text(hwnd, part, &dimension_text);
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
    if current.0 as usize == hash {
        return; // unchanged — skip the redraw (empty text included)
    }
    let new = to_wide(text);
    // SAFETY: `new` is NUL-terminated and outlives the call.
    let ok = unsafe {
        SendMessageW(
            hwnd,
            SB_SETTEXTW,
            Some(WPARAM(part)),
            Some(LPARAM(new.as_ptr() as isize)),
        )
        .0 != 0
    };
    // Cache the hash ONLY on success: a failed SET leaves the old text on
    // screen, and caching anyway would suppress every retry of this text
    // (a stale bar forever — cubic/Codex PR #13). A dropped cache write
    // just re-sends the text next refresh, which is harmless.
    if ok {
        // SAFETY: hwnd is our live child; SetPropW on a hash-valued
        // property cannot fail for it (fail-soft as above).
        unsafe {
            let _ = SetPropW(
                hwnd,
                PCWSTR(name.as_ptr()),
                Some(windows::Win32::Foundation::HANDLE(hash as *mut _)),
            );
        }
    }
}
