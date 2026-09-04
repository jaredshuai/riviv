//! riviv — an unofficial Rust rewrite of voidtools/voidImageViewer (MIT).
//!
//! M1 skeleton: Win32 window + GDI rendering + static image display.
//!
//! Behavior baseline is the upstream C source under `c-original/src/viv.c`
//! (see c-original/PROVENANCE.md). Key alignments in this file:
//! - never upscale (`fill_window = 0` default, `_viv_get_render_size` clamp)
//! - load failure keeps the old image / opens an empty window — never a popup, never an exit
//! - window title is `filename - riviv` (`_viv_update_title` format)
//! - drag & drop of a single file replaces the current image, window size is NOT reset
//!   (`WM_DROPFILES` handler); multi-file drop = M2 playlist, we take the first file
//! - no-arg start = empty window, Ctrl+O opens the file dialog (upstream default keymap)

#![windows_subsystem = "windows"]

use std::ffi::{OsStr, OsString, c_void};
use std::iter::once;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use image::{GenericImageView, ImageDecoder};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, COLOR_BTNFACE, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, FillRect, GetMonitorInfoW,
    GetStockObject, HALFTONE, HBITMAP, HBRUSH, HDC, HGDIOBJ, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT, SRCCOPY, SelectObject,
    SetBrushOrgEx, SetStretchBltMode, StretchBlt, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CreateWindowExW,
    DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetClientRect, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, IDC_ARROW, LoadCursorW, MB_ICONERROR, MINMAXINFO, MSG, MessageBoxW,
    PostQuitMessage, RegisterClassExW, SHOW_WINDOW_CMD, SW_SHOW, SetForegroundWindow,
    SetProcessDPIAware, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage,
    WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WNDCLASSEXW, WS_EX_ACCEPTFILES, WS_OVERLAPPEDWINDOW,
};
use windows::core::{HSTRING, PCWSTR, w};

/// Window class name. Deliberately different from upstream's `VOIDIMAGEVIEWER`
/// (class + mutex) so both viewers can coexist on one machine.
const CLASS_NAME: PCWSTR = w!("riviv");

/// Minimum trackable window size (upstream handles WM_GETMINMAXINFO in viv.c:4424).
const MIN_TRACK: POINT = POINT { x: 160, y: 120 };

// ---------------------------------------------------------------------------
// Pure logic (unit-tested; unsafe shells must sink their math into these)
// ---------------------------------------------------------------------------

/// image crate yields RGBA rows (top-down); GDI 32bpp DIBs want BGRA.
fn rgba8_to_bgra_in_place(buf: &mut [u8]) {
    let (pixels, tail) = buf.as_chunks_mut::<4>();
    debug_assert!(
        tail.is_empty(),
        "RGBA8 buffer length must be a multiple of 4"
    );
    for px in pixels {
        px.swap(0, 2);
    }
}

/// Fit `(src_w, src_h)` inside `(max_w, max_h)` keeping aspect ratio,
/// never upscaling (upstream `fill_window = 0`). The derived side rounds UP
/// with integer math — upstream viv.c:6895-6913 deliberately adds `high - 1`
/// so a 50%-window resize still stretches to the screen edges.
fn fit_shrink(src_w: i32, src_h: i32, max_w: i32, max_h: i32) -> (i32, i32) {
    if src_w <= 0 || src_h <= 0 || max_w <= 0 || max_h <= 0 {
        return (src_w.max(1), src_h.max(1));
    }
    let (w, h) = (i64::from(src_w), i64::from(src_h));
    let (mw, mh) = (i64::from(max_w), i64::from(max_h));
    let (mut rw, mut rh) = if mh * w < mw * h {
        // tall: height binds, width is derived (ceil)
        ((mh * w + h - 1) / h, mh)
    } else {
        // long: width binds, height is derived (ceil)
        (mw, (mw * h + w - 1) / w)
    };
    // never upscale (upstream !fill_window clamp, viv.c:6922-6928)
    if rw > w || rh > h {
        rw = w;
        rh = h;
    }
    (rw as i32, rh as i32)
}

/// Upstream title format (`_viv_update_title`): `filename - AppName`,
/// app name only when no image is loaded. Built from raw wide code units so
/// filenames containing unpaired UTF-16 surrogates survive verbatim instead
/// of collapsing into U+FFFD replacement characters.
fn title_wide(path: Option<&OsStr>) -> Vec<u16> {
    let mut title: Vec<u16> = Vec::new();
    if let Some(name) = path.and_then(|p| Path::new(p).file_name()) {
        title.extend(name.encode_wide());
        title.extend(" - ".encode_utf16());
    }
    title.extend("riviv".encode_utf16());
    title
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

/// File-dialog filter as a double-null-terminated wide string.
fn dialog_filter() -> Vec<u16> {
    "Images (*.png;*.jpg;*.jpeg;*.bmp;*.ico;*.tif;*.tiff;*.gif;*.webp)\0\
     *.png;*.jpg;*.jpeg;*.bmp;*.ico;*.tif;*.tiff;*.gif;*.webp\0\
     All files (*.*)\0*.*\0"
        .encode_utf16()
        .chain(once(0))
        .collect()
}

// ---------------------------------------------------------------------------
// Image surface: decoded pixels held in a top-down 32bpp DIB section,
// selected into a private memory DC — the same render path as upstream
// (CreateCompatibleBitmap + SetDIBits -> mem DC -> StretchBlt, viv.c:10263-10271, 4273).
// ---------------------------------------------------------------------------

/// Two failure layers (ADR 0001): user-level keeps the old image / opens an
/// empty window (upstream `_viv_load_failed` behavior); system-level GDI
/// failures are fatal with context.
enum LoadError {
    /// Bad path / undecodable image — user-level, suppressed silently.
    /// The message is unused in M1 and becomes the M2 status-bar text
    /// (upstream "Failed to load image.").
    User(#[allow(dead_code)] String),
    /// DIB/DC infrastructure failure — system-level, fail loud.
    System(String),
}

struct Surface {
    memdc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl Surface {
    fn from_rgba(width: u32, height: u32, rgba: &mut [u8]) -> Result<Self, LoadError> {
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

fn load_surface(path: &OsStr) -> Result<Surface, LoadError> {
    let shown = path.to_string_lossy();
    let user = |msg: String| LoadError::User(format!("{shown}: {msg}"));
    // Sniff the format from file contents (upstream GDI+ behavior): renamed or
    // extensionless files still decode.
    let reader = image::ImageReader::open(Path::new(path)).map_err(|e| user(e.to_string()))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| user(e.to_string()))?;
    let mut decoder = reader.into_decoder().map_err(|e| user(e.to_string()))?;
    // Guard against hostile/corrupt headers declaring huge pixel sizes: the
    // default 512MB allocation cap turns them into user-level load errors
    // instead of an OOM crash (ADR 0001 — a bad file must not kill the viewer).
    decoder
        .set_limits(image::Limits::default())
        .map_err(|e| user(e.to_string()))?;
    // Apply EXIF orientation before reading dimensions (upstream
    // config_orientation = 1, config.c:90) so phone photos are not sideways.
    // Best-effort like upstream os.c:1545-1600: malformed orientation metadata
    // falls back to no rotation instead of rejecting the image.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| user(e.to_string()))?;
    img.apply_orientation(orientation);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(user("empty image".to_string()));
    }
    let mut rgba = img.into_rgba8().into_raw();
    Surface::from_rgba(w, h, &mut rgba)
}

// ---------------------------------------------------------------------------
// Window state, hung off GWLP_USERDATA between WM_NCCREATE and WM_NCDESTROY
// ---------------------------------------------------------------------------

struct WindowState {
    surface: Option<Surface>,
    path: Option<OsString>,
}

/// Window state pointer stored in GWLP_USERDATA between WM_NCCREATE and
/// WM_NCDESTROY — for this single-window M1 skeleton the borrow effectively
/// lives as long as the window itself.
fn state_of(hwnd: HWND) -> Option<&'static mut WindowState> {
    // SAFETY: between WM_NCCREATE and WM_NCDESTROY the slot holds a live Box
    // pointer; before/after it is zero and we return None. Callers run on the
    // window's own thread inside message handlers, so no aliasing occurs.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: pointer was stored once by WM_NCCREATE and only cleared in
    // WM_NCDESTROY, so it is exclusively reachable from this window's handlers.
    Some(unsafe { &mut *ptr })
}

fn open_image(hwnd: HWND, path: &OsStr) {
    let Some(state) = state_of(hwnd) else { return };
    match load_surface(path) {
        Ok(surface) => {
            state.surface = Some(surface); // replacing drops the old surface
            state.path = Some(path.to_os_string());
            let title = HSTRING::from_wide(&title_wide(state.path.as_deref()));
            // SAFETY: hwnd is live; the HSTRING outlives the call.
            // Fail-soft on purpose: upstream viv.c:1249 ignores SetWindowTextW's
            // return too — a stale caption beats killing the viewer (ADR 0001
            // leaves user-visible captions out of the fatal layer).
            let _ = unsafe { SetWindowTextW(hwnd, &title) };
            // SAFETY: repaint request; the return value (whether any region was
            // invalidated) is irrelevant here — WM_PAINT will come.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };
        }
        // User-level failure: keep the current image, no popup, no exit
        // (ADR 0001 / upstream `_viv_load_failed` behavior).
        Err(LoadError::User(_)) => {}
        // System-level GDI failure: fail loud (ADR 0001).
        Err(LoadError::System(msg)) => fatal(&msg),
    }
}

fn open_file_dialog(hwnd: HWND, initial_dir: Option<&OsStr>) -> Option<OsString> {
    let filter = dialog_filter();
    // 32768 code units: long paths must not trip FNERR_BUFFERTOOSMALL.
    let mut file_buf = vec![0u16; 32768];
    let dir = initial_dir.map(HSTRING::from);
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrInitialDir: match &dir {
            Some(h) => PCWSTR(h.as_ptr()),
            None => PCWSTR::null(),
        },
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: `ofn` borrows stack locals (filter/file buffer/dir) that all
    // outlive this modal call.
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        // SAFETY: pure query of this thread's last common-dialog error; zero
        // means a plain user cancel.
        let err = unsafe { CommDlgExtendedError() };
        if err.0 != 0 {
            // System-level failure (ADR 0001): report instead of a dead Ctrl+O.
            fatal(&format!(
                "GetOpenFileNameW failed (CommDlgExtendedError={})",
                err.0
            ));
        }
        return None; // user cancelled
    }
    let len = file_buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(OsString::from_wide(&file_buf[..len]))
}

fn on_keydown(hwnd: HWND, wparam: WPARAM) {
    // Upstream default keymap: Ctrl+O = open file (viv.c:972). The only M1 hotkey.
    if wparam.0 != usize::from(b'O') {
        return;
    }
    // SAFETY: GetKeyState reads thread-local async key state; VK_CONTROL is valid.
    if unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } >= 0 {
        return;
    }
    let initial_dir = state_of(hwnd)
        .and_then(|s| s.path.clone())
        .and_then(|p| Path::new(&p).parent().map(|d| d.as_os_str().to_os_string()));
    if let Some(path) = open_file_dialog(hwnd, initial_dir.as_deref()) {
        open_image(hwnd, &path);
    }
}

fn on_drop_files(hwnd: HWND, hdrop: HDROP) {
    // SAFETY: `hdrop` is owned by this message; DragFinish is called exactly once
    // on every path below.
    unsafe {
        // Upstream: single file replaces the current image (viv.c:3119-3124);
        // multi-file / shift-drop build a playlist (M2) — we take the first file.
        if DragQueryFileW(hdrop, u32::MAX, None) > 0 {
            let len = DragQueryFileW(hdrop, 0, None) as usize;
            if len > 0 && len < 32768 {
                let mut buf = vec![0u16; len + 1];
                if DragQueryFileW(hdrop, 0, Some(&mut buf)) as usize == len {
                    let path = OsString::from_wide(&buf[..len]);
                    open_image(hwnd, &path);
                }
            }
        }
        DragFinish(hdrop);
        // Upstream re-activates the viewer after a drop (viv.c:3126) so the
        // drag source window does not stay in front of the result.
        // SAFETY: hwnd is live and owned by this thread.
        let _ = SetForegroundWindow(hwnd);
    }
}

fn paint(hwnd: HWND) {
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
        let _ = GetClientRect(hwnd, &mut client);
        let cw = (client.right - client.left).max(1);
        let ch = (client.bottom - client.top).max(1);
        // Windowed background default is white (config.c:65-67).
        let _ = FillRect(hdc, &client, HBRUSH(GetStockObject(WHITE_BRUSH).0));
        if let Some(surface) = state_of(hwnd).and_then(|s| s.surface.as_ref()) {
            let (dw, dh) = fit_shrink(surface.width, surface.height, cw, ch);
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
                Some(surface.memdc),
                0,
                0,
                surface.width,
                surface.height,
                SRCCOPY,
            );
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            // SAFETY: lparam points to a CREATESTRUCTW for the duration of the
            // message (Win32 contract). We adopt the Box prepared in run() —
            // if creation later fails, WM_NCDESTROY below frees it.
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            // SAFETY: the CREATESTRUCTW field still owns the Box leaked by run();
            // adopting it here is the single hand-off, freed in WM_NCDESTROY.
            let state = unsafe { Box::from_raw(cs.lpCreateParams.cast::<WindowState>()) };
            let state_ptr = Box::into_raw(state);
            // SAFETY: clearing the thread's last error so a zero return from
            // SetWindowLongPtrW is distinguishable from "previous value was 0".
            unsafe { SetLastError(WIN32_ERROR(0)) };
            // SAFETY: hwnd is being created; storing our exclusively-owned pointer.
            let prev = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize) };
            if prev == 0 {
                // SAFETY: reading the thread's last error immediately after the call.
                let gle = unsafe { GetLastError().0 };
                if gle != 0 {
                    // Store failed: the window would run stateless (blank client,
                    // dead Ctrl+O/drops). Reclaim the box and abort creation.
                    // SAFETY: the failed store never published the pointer, so it
                    // is still exclusively ours.
                    drop(unsafe { Box::from_raw(state_ptr) });
                    return LRESULT(0); // FALSE aborts CreateWindowExW
                }
            }
            // SAFETY: hwnd/msg are exactly what this callback received; forwarding
            // to the default procedure must return its verdict untouched.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NCDESTROY => {
            // SAFETY: the slot holds a live Box pointer set in WM_NCCREATE;
            // take it back, clear the slot, then free.
            unsafe {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1), // WM_PAINT fills the whole client
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            // SAFETY: lparam points to a MINMAXINFO for the duration of the message.
            let mmi = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            mmi.ptMinTrackSize = MIN_TRACK;
            LRESULT(0)
        }
        WM_KEYDOWN => {
            on_keydown(hwnd, wparam);
            LRESULT(0)
        }
        WM_DROPFILES => {
            // SAFETY: wparam is the HDROP owned by this message.
            on_drop_files(hwnd, HDROP(wparam.0 as *mut c_void));
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: legal on the owning thread while quitting the message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: hwnd/msg are exactly what this callback received; the default
        // procedure handles everything we do not.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Outer window rect: client = image size shrunk to fit the work area of the
/// monitor under the cursor (upstream centers on the cursor's monitor,
/// viv.c:5359-5387); no image -> 60% auto-fit with a 640x480 floor.
fn initial_window_rect(surface: Option<&Surface>) -> Result<RECT, String> {
    // SAFETY: read-only monitor/geometry queries; AdjustWindowRect only computes.
    unsafe {
        let mut cursor = POINT::default();
        // Fail loud: a zero cursor point would silently center on whichever
        // monitor is nearest (0,0) instead of the user's (ADR 0001).
        GetCursorPos(&mut cursor).map_err(|e| format!("GetCursorPos failed: {e}"))?;
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
            // Already inside this function's outer unsafe block.
            let gle = GetLastError().0;
            return Err(format!("GetMonitorInfoW failed (GLE={gle})"));
        }
        let work = mi.rcWork;
        // Reserve the non-client frame up front so a full-height fit cannot push
        // the outer window past the work area (portrait images used to clip
        // under the taskbar). AdjustWindowRect on a zero rect yields the frame
        // extents; with WS_OVERLAPPEDWINDOW and no menu they are size-independent.
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        AdjustWindowRect(&mut frame, WS_OVERLAPPEDWINDOW, false)
            .map_err(|e| format!("AdjustWindowRect failed: {e}"))?;
        let avail_w = (work.right - work.left - (frame.right - frame.left)).max(1);
        let avail_h = (work.bottom - work.top - (frame.bottom - frame.top)).max(1);
        let (cw, ch) = match surface {
            // Window = image size (upstream Alt+2 semantics); the remembered-rect /
            // 60%-first-run model returns with M3 config persistence.
            Some(s) => fit_shrink(s.width, s.height, avail_w, avail_h),
            None => (
                // floor 640x480, but never beyond the available client area
                // (clamp would panic when min > max on tiny screens/VMs).
                ((work.right - work.left) * 3 / 5).max(640).min(avail_w),
                ((work.bottom - work.top) * 3 / 5).max(480).min(avail_h),
            ),
        };
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: cw,
            bottom: ch,
        };
        AdjustWindowRect(&mut rc, WS_OVERLAPPEDWINDOW, false)
            .map_err(|e| format!("AdjustWindowRect failed: {e}"))?;
        let wide = rc.right - rc.left;
        let high = rc.bottom - rc.top;
        let x = work.left + ((work.right - work.left) - wide).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - high).max(0) / 2;
        Ok(RECT {
            left: x,
            top: y,
            right: x + wide,
            bottom: y + high,
        })
    }
}

fn fatal(message: &str) -> ! {
    let text = to_wide(message);
    // SAFETY: a null owner is allowed for a modal error box (system-level
    // failure path — ADR 0001 fail loud).
    let _ = unsafe { MessageBoxW(None, PCWSTR(text.as_ptr()), CLASS_NAME, MB_ICONERROR) };
    std::process::exit(1)
}

fn run(arg_path: Option<OsString>) -> Result<(), String> {
    let mut state = WindowState {
        surface: None,
        path: None,
    };
    if let Some(path) = arg_path.as_ref() {
        match load_surface(path) {
            Ok(surface) => {
                state.surface = Some(surface);
                state.path = Some(path.clone());
            }
            // Bad path / undecodable file -> empty window, per upstream
            // (ADR 0001 user-level failures).
            Err(LoadError::User(_)) => {}
            // System-level GDI failure -> fatal (ADR 0001).
            Err(LoadError::System(msg)) => return Err(msg),
        }
    }
    // SAFETY: process-wide and must run before any window exists (matches the
    // upstream manifest's dpiAware=true). ERROR_ACCESS_DENIED means the process
    // is already DPI-aware (e.g. a manifest got embedded later) — that is a
    // success state; any other failure undermines the whole 1:1 / work-area
    // geometry model, so fail loud (ADR 0001).
    if !unsafe { SetProcessDPIAware() }.as_bool() {
        // SAFETY: reading the thread's last error immediately after the failed call.
        let gle = unsafe { GetLastError().0 };
        if gle != ERROR_ACCESS_DENIED.0 {
            return Err(format!("SetProcessDPIAware failed (GLE={gle})"));
        }
    }

    // SAFETY: returns the module handle of this exe; no side effects.
    let hinstance =
        unsafe { GetModuleHandleW(None) }.map_err(|e| format!("GetModuleHandleW failed: {e}"))?;

    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS | CS_VREDRAW | CS_HREDRAW, // CS_DBLCLKS now, double-click = M2
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        // SAFETY: IDC_ARROW is a predefined system resource; failing here would
        // register a cursorless class (no pointer over the client), so
        // propagate instead (ADR 0001).
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|e| format!("LoadCursorW failed: {e}"))?,
        hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut c_void), // upstream viv.c:5348
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: wc outlives the call; the returned atom is checked.
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        // SAFETY: reading the thread's last error right after the failed call.
        let gle = unsafe { GetLastError().0 };
        return Err(format!("RegisterClassExW failed (GLE={gle})"));
    }

    let rect = initial_window_rect(state.surface.as_ref())?;
    let title = HSTRING::from_wide(&title_wide(state.path.as_deref()));
    let state_ptr = Box::into_raw(Box::new(state));

    // SAFETY: all parameters are valid for the call; state_ptr ownership moves
    // into the window via WM_NCCREATE. If creation fails BEFORE WM_NCCREATE the
    // pointer leaks into the fatal-exit path (acceptable, ADR 0001); if it fails
    // after, WM_NCDESTROY already freed it.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_ACCEPTFILES,
            CLASS_NAME,
            &title,
            WS_OVERLAPPEDWINDOW,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(hinstance.into()),
            Some(state_ptr as *const c_void),
        )
    }
    .map_err(|e| format!("CreateWindowExW failed: {e}"))?;

    // Honor the launcher's requested show state ("run maximized/minimized"
    // shortcuts) like upstream viv.c:5424-5451; SW_SHOW is the default when
    // the launcher did not ask for anything specific.
    let mut si = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: fills this process's STARTUPINFOW; a read-only query.
    unsafe { GetStartupInfoW(&mut si) };
    let show_cmd = if (si.dwFlags & STARTF_USESHOWWINDOW).0 != 0 {
        SHOW_WINDOW_CMD(i32::from(si.wShowWindow))
    } else {
        SW_SHOW
    };
    // SAFETY: hwnd is live; show per the launcher's request.
    let _ = unsafe { ShowWindow(hwnd, show_cmd) };

    let mut msg = MSG::default();
    loop {
        // SAFETY: standard pump over this thread's queue.
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        match r.0 {
            0 => break, // WM_QUIT
            // Fail loud instead of spinning on a broken pump (ADR 0001); with
            // filter params (None, 0, 0) this is near-unreachable in practice.
            -1 => {
                // SAFETY: reading the thread's last error right after the failed call.
                let gle = unsafe { GetLastError().0 };
                return Err(format!("GetMessageW failed (GLE={gle})"));
            }
            _ => {
                // SAFETY: msg was filled by GetMessageW just above.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    let _ = DispatchMessageW(&msg);
                }
            }
        }
    }
    Ok(())
}

fn main() {
    let arg_path = std::env::args_os().nth(1);
    if let Err(err) = run(arg_path) {
        fatal(&format!("riviv: {err}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_conversion_swaps_r_and_b_in_place() {
        let mut pixels = vec![10, 20, 30, 255, 1, 2, 3, 128];
        rgba8_to_bgra_in_place(&mut pixels);
        assert_eq!(pixels, vec![30, 20, 10, 255, 3, 2, 1, 128]);
    }

    #[test]
    fn fit_never_upscales() {
        assert_eq!(fit_shrink(100, 50, 1000, 1000), (100, 50));
    }

    #[test]
    fn fit_caps_to_bounds_and_keeps_aspect() {
        assert_eq!(fit_shrink(2000, 1000, 1000, 1000), (1000, 500));
        assert_eq!(fit_shrink(1000, 2000, 1000, 1000), (500, 1000));
    }

    #[test]
    fn fit_floors_at_one_pixel() {
        assert_eq!(fit_shrink(10000, 10000, 1, 1), (1, 1));
    }

    #[test]
    fn fit_rounds_derived_side_upstream_style() {
        // upstream viv.c:6895-6913: 1000x333 into 400x400 -> 400x134 (not 133),
        // so a 50%-window resize still stretches to the screen edges.
        assert_eq!(fit_shrink(1000, 333, 400, 400), (400, 134));
        assert_eq!(fit_shrink(333, 1000, 400, 400), (134, 400));
    }

    #[test]
    fn title_is_filename_first_then_app_name() {
        let title = title_wide(Some(OsStr::new(r"C:\pics\cat.png")));
        assert_eq!(String::from_utf16_lossy(&title), "cat.png - riviv");
    }

    #[test]
    fn title_preserves_unpaired_surrogate_code_units() {
        // Windows filenames may contain unpaired UTF-16 surrogates; they must
        // reach the title verbatim (upstream SetWindowTextW takes wide strings).
        let name = OsString::from_wide(&[0xD800, u16::from(b'a')]);
        let title = title_wide(Some(name.as_os_str()));
        let expected: Vec<u16> = [0xD800, u16::from(b'a')]
            .into_iter()
            .chain(" - riviv".encode_utf16())
            .collect();
        assert_eq!(title, expected);
    }

    #[test]
    fn title_without_image_is_app_name_only() {
        assert_eq!(String::from_utf16_lossy(&title_wide(None)), "riviv");
    }
}
