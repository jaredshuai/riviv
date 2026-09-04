//! Window shell: window state, the message pump, class registration, and the
//! input/open-action handlers hung off the single wnd_proc.
//!
//! The animation timer wiring (#3) and the background decode protocol (#4)
//! are landed: opens spawn a load session whose replies arrive on a private
//! `WM_APP+1` kick and are applied by `on_load_replies` — the display swaps
//! when the first frame replies in, animation frames append while playing,
//! and a stalled prefix waits for the decode (see `loader::apply_reply` and
//! `loadthread.rs`). M2 seams left: fullscreen toggle + cursor-hide timers
//! (#8) and drop→playlist / navigation key wiring (#6).

use std::ffi::{OsStr, OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, GetMonitorInfoW, HBRUSH, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
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
    GetWindowLongPtrW, IDC_ARROW, KillTimer, LoadCursorW, MB_ICONERROR, MINMAXINFO, MSG,
    MessageBoxW, PostQuitMessage, RegisterClassExW, SHOW_WINDOW_CMD, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetProcessDPIAware, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    USER_TIMER_MINIMUM, WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_EX_ACCEPTFILES,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::anim::ANIMATION_TIMER_ID;
use crate::fit::fit_shrink;
use crate::loader::{LoadedImage, UiAction, apply_reply, map_reply_frame};
use crate::loadthread::{LoadSession, LoadThread, REPLY_KICK_MESSAGE};
use crate::paint::paint;
use crate::status;
use crate::surface::Surface;
use crate::text::{dialog_filter, title_wide, to_wide};

/// Window class name. Deliberately different from upstream's `VOIDIMAGEVIEWER`
/// (class + mutex) so both viewers can coexist on one machine.
const CLASS_NAME: PCWSTR = w!("riviv");

/// Minimum trackable window size (upstream handles WM_GETMINMAXINFO in viv.c:4424).
const MIN_TRACK: POINT = POINT { x: 160, y: 120 };

pub(crate) struct WindowState {
    pub(crate) image: Option<LoadedImage>,
    pub(crate) path: Option<OsString>,
    /// Performance-counter frequency (ticks per second) — the unit of the
    /// animation timeline. Constant for the process lifetime.
    pub(crate) timer_freq: u64,
    /// The process-wide decode worker (at most one decode is ever active;
    /// see `loadthread.rs`). Quit+joined at window teardown.
    pub(crate) load_thread: LoadThread,
    /// The in-flight load; `None` while idle. Storing a new session drops
    /// the old one, which flags its job — the worker skips it at the next
    /// check (upstream `_viv_load_image_terminate` chaining).
    pub(crate) session: Option<LoadSession>,
    /// Which load session produced the currently displayed image (`None`
    /// when blank) — the staleness guard for replies and failure handling.
    pub(crate) displayed_from: Option<u64>,
    /// The session whose first frame resizes the window to the image
    /// (M1's image-sized window preserved under async startup — the window
    /// is created before anything is decoded). Tied to the session id so a
    /// superseded or failed startup load cannot leak the resize into a
    /// later open (image switches never resize).
    pub(crate) startup_resize_session: Option<u64>,
    /// Whether the animation timer is currently running — edge bookkeeping
    /// so timer reconciliation never resets a live timer's period (which
    /// would starve WM_TIMER under fast frame streams).
    pub(crate) animation_timer_running: bool,
    /// The status-bar child window (#5; upstream `_viv_status_hwnd`).
    /// Created in WM_NCCREATE, destroyed with the parent by Windows.
    pub(crate) status: HWND,
    /// Status-bar flags (upstream `_viv_file_not_found` /
    /// `_viv_load_failed`, viv.c:779-780): set by the open path and the
    /// reply protocol, reset on every new open. "Loading" is not a flag —
    /// it derives from `session.is_some()` (a session is taken when its
    /// terminal reply drains).
    pub(crate) status_file_not_found: bool,
    pub(crate) status_load_failed: bool,
    /// Byte size of the displayed file, for the status bar's "(N KB)"
    /// (upstream reads it from the load's WIN32_FIND_DATA, viv.c:11152) —
    /// None/0 omits the size clause.
    pub(crate) displayed_file_bytes: Option<u64>,
    /// Byte size of the in-flight load's file, staged at `request_open` and
    /// committed to `displayed_file_bytes` only when its first frame takes
    /// the display — a failed replacement must not clobber the old image's
    /// size in the status bar (cubic PR #13).
    pub(crate) pending_file_bytes: Option<u64>,
}

/// Window state pointer stored in GWLP_USERDATA between WM_NCCREATE and
/// WM_NCDESTROY — for this single-window M1 skeleton the borrow effectively
/// lives as long as the window itself.
///
/// # Safety
///
/// The returned `&'static mut` is an exclusive borrow of the window's state
/// box. Callers must not hold it across anything that pumps messages
/// (modal dialogs, `GetMessageW`, nested `DispatchMessageW`) or call
/// `state_of` again while a previous borrow is still live — that would alias
/// `&mut WindowState` (undefined behavior). Acquire, act, drop.
///
/// (Kept as an explicit `unsafe` contract rather than a safe aliasable API:
/// M2 adds reentry surfaces — animation timers, thread replies — and the
/// compiler cannot see the single-threaded message discipline.)
pub(crate) unsafe fn state_of(hwnd: HWND) -> Option<&'static mut WindowState> {
    // SAFETY: between WM_NCCREATE and WM_NCDESTROY the slot holds a live Box
    // pointer; before/after it is zero and we return None. Callers run on the
    // window's own thread inside message handlers, so no aliasing occurs
    // under the contract above.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: pointer was stored once by WM_NCCREATE and only cleared in
    // WM_NCDESTROY; exclusivity is the caller's obligation per the contract.
    Some(unsafe { &mut *ptr })
}

/// Build the status-bar snapshot from the window state — the pure text
/// model in `text.rs` decides what each part says from this. Called both
/// inside a state borrow (to collect the snapshot) and the result applied
/// after the borrow drops (`status::update` sends messages).
fn status_snapshot(state: &WindowState, hwnd: HWND) -> status::StatusSnapshot {
    let mut client = RECT::default();
    // SAFETY: hwnd is the state's own live window; a failed query reads the
    // zeroed rect — a 0-wide client collapses the part layout until the
    // next refresh.
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    status::StatusSnapshot {
        loading: state.session.is_some(),
        file_not_found: state.status_file_not_found,
        load_failed: state.status_load_failed,
        frame: state
            .image
            .as_ref()
            .map(|i| (i.frame_position_1based(), i.frame_count())),
        dimensions: state.image.as_ref().map(|i| (i.width(), i.height())),
        file_bytes: state.displayed_file_bytes,
        client_wide: client.right - client.left,
    }
}

/// Refresh the status bar from current state, running the Win32 calls
/// outside any state borrow (SB_SETTEXT redraws synchronously).
fn refresh_status(hwnd: HWND) {
    // SAFETY: the borrow spans only reads into the snapshot struct; the
    // SendMessageW calls in status::update run after it drops. Two
    // sequential borrows, never nested.
    let snapshot = unsafe { state_of(hwnd) }.map(|s| status_snapshot(s, hwnd));
    let Some(snapshot) = snapshot else { return };
    status::update(snapshot_status_bar(hwnd), &snapshot);
}

/// The status-bar child handle (created in WM_NCCREATE).
fn snapshot_status_bar(hwnd: HWND) -> HWND {
    // SAFETY: read-only field copy.
    unsafe { state_of(hwnd) }.map_or(HWND::default(), |s| s.status)
}

/// Queue `path` for background decoding (upstream `_viv_open`'s
/// CreateThread arm, viv.c:1569). The current display stays up until this
/// load's first frame replies in; storing the new session supersedes
/// (flags) any in-flight one. `is_startup` ties the one-time
/// resize-to-image to this session's first frame.
fn request_open(hwnd: HWND, path: &OsStr, is_startup: bool) {
    // Existence check BEFORE queueing a decode (upstream
    // `_viv_open_from_filename`'s GetFileAttributesEx arm, viv.c:1359 —
    // the status bar's "File not found." is a pre-open verdict, not a
    // decode failure, viv.c:5094-5098). The byte size rides along for the
    // status bar's "(N KB)"; directories and unreadable files fall through
    // to the loader as user-level failures like upstream.
    let (not_found, file_bytes) = match std::fs::metadata(Path::new(path)) {
        Ok(meta) => (false, Some(meta.len())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, None),
        Err(_) => (false, None),
    };
    if not_found {
        // A missing file never reaches the loader (no session, no
        // Loading) — the bar shows "File not found." over the kept display
        // until the next open (upstream sets `_viv_file_not_found` without
        // spawning a load, viv.c:5094-5098).
        // SAFETY: the borrow spans only flag stores — nothing pumps.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.status_file_not_found = true;
            state.status_load_failed = false;
            // Supersede any in-flight load so its late replies are inert.
            state.session = None;
        }
        refresh_status(hwnd);
        return;
    }
    // SAFETY: the borrow spans only the worker request and the session/
    // status stores — the channel send never blocks, the old session's
    // Drop sets an atomic flag (no GDI, no pumping), and kicks posted by
    // the worker are queued messages, never delivered synchronously here.
    if let Some(state) = unsafe { state_of(hwnd) } {
        // Upstream resets the failure flags at the start of every new load
        // (viv.c:1447-1458).
        state.status_file_not_found = false;
        state.status_load_failed = false;
        // The byte size is STAGED, not committed: a replacement load that
        // fails before its first frame keeps the old image on screen, and
        // the status bar must keep showing the OLD file's size with it
        // (cubic PR #13). Committed to `displayed_file_bytes` when this
        // session's first frame takes the display.
        state.pending_file_bytes = file_bytes;
        let session = state.load_thread.request(hwnd, path.to_os_string());
        if is_startup {
            state.startup_resize_session = Some(session.id());
        }
        state.session = Some(session);
    }
    refresh_status(hwnd);
}

/// The reply-kick handler: drain the current load session's queue and apply
/// each reply to the display (upstream `_VIV_WM_REPLY`, viv.c:2762-3060).
/// Protocol decisions live in the pure `loader::apply_reply`; this shell
/// batches the Win32 effects so they run after the state borrow drops —
/// the fatal modal must never run across a borrow (PR #10 P1).
fn on_load_replies(hwnd: HWND) {
    // Read the clock first for the same reason (its failure path is the
    // fatal modal). The anchor is reply-apply time, like upstream's
    // _viv_start_first_frame running inside the reply handler.
    let now = qpc_now();
    let mut fatal_msg: Option<String> = None;
    let start_timer;
    let stop_timer;
    let mut invalidate = false;
    let mut title: Option<HSTRING> = None;
    let mut resize_to_image = false;
    let mut resize_rect: Option<RECT> = None;
    {
        // Copy the session facts out first so the immutable borrow ends
        // before the reply loop mutates the display state.
        // SAFETY: the borrow spans queue draining, the pure reply state
        // machine, and read-only geometry queries — nothing here pumps
        // messages, so no second state_of borrow can alias this one.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        let Some(session) = state.session.as_ref() else {
            return;
        };
        let replies = session.drain();
        let session_id = session.id();
        let session_path = session.path().to_os_string();
        for reply in replies {
            // Frames cross the thread boundary as bare DIBs; the DC-carrying
            // Surface is built here, on the UI thread that renders with it
            // (memory DCs belong to their creating thread). A wrap failure
            // is GDI exhaustion — system-level, fail loud (ADR 0001).
            let reply = map_reply_frame(reply, Surface::from_frame);
            let outcome = apply_reply(
                &mut state.image,
                &mut state.displayed_from,
                session_id,
                &mut state.startup_resize_session,
                now,
                state.timer_freq,
                reply,
            );
            // The status bar's Loading/Failed flags follow the protocol
            // facts (#5): the session ends at its terminal reply (taken so
            // `session.is_some()` stops meaning "loading"), and a
            // user-level failure sticks until the next open.
            if outcome.load_ended && state.session.as_ref().is_some_and(|s| s.id() == session_id) {
                state.session = None;
            }
            if outcome.load_failed {
                state.status_load_failed = true;
            }
            if let Some(msg) = outcome.fatal {
                fatal_msg = Some(msg);
                break;
            }
            for action in outcome.actions {
                match action {
                    UiAction::Invalidate => invalidate = true,
                    UiAction::SetWindowTitle => {
                        // The display adopted this session's image (or
                        // cleared it): the status bar's "(N KB)" follows
                        // the same commit/clear.
                        if state.displayed_from == Some(session_id) {
                            state.path = Some(session_path.clone());
                            title = Some(HSTRING::from_wide(&title_wide(state.path.as_deref())));
                            // Commit the staged size now that THIS session's
                            // image is on screen (a failed replacement never
                            // reaches here, so the old size survives).
                            state.displayed_file_bytes = state.pending_file_bytes.take();
                        } else {
                            state.path = None;
                            title = Some(HSTRING::from_wide(&title_wide(None)));
                            state.displayed_file_bytes = None;
                        }
                    }
                    // Deferred to after the drain: a later reply in the same
                    // batch (mid-stream failure) may clear the image again —
                    // sizing the window to an image that is no longer shown
                    // would be nonsense.
                    UiAction::ResizeWindowToImage => resize_to_image = true,
                }
            }
            if fatal_msg.is_some() {
                break;
            }
        }
        // Timer reconciliation from the drain's NET effect — deriving from
        // the final image state cannot disagree with the protocol (a batch
        // that both starts an animation and clears it again must end with
        // the timer stopped). Edge-managed against the running flag so a
        // live timer is never re-Set (which would reset its period and
        // starve WM_TIMER under fast frame streams).
        let want_timer = state.image.as_ref().is_some_and(LoadedImage::is_animated);
        start_timer = want_timer && !state.animation_timer_running;
        stop_timer = !want_timer && state.animation_timer_running;
        state.animation_timer_running = want_timer;
        if resize_to_image && state.image.is_some() {
            // The startup load's first frame sizes the window to the image
            // (M1 behavior under async startup); a geometry failure is fatal
            // exactly like run()'s initial rect.
            match initial_window_rect(state.image.as_ref(), status::height(state.status)) {
                Ok(rect) => resize_rect = Some(rect),
                Err(msg) => fatal_msg = Some(msg),
            }
        }
    }
    // Borrow dropped — the modal paths and the reentrant resize are safe.
    refresh_status(hwnd);
    if let Some(msg) = fatal_msg {
        fatal(&msg);
    }
    // Edge-managed timer reconciliation (mutually exclusive flags — see
    // the drain loop above).
    if stop_timer {
        // SAFETY: hwnd is live; a failed kill leaves a stale timer that the
        // WM_TIMER guard no-ops on.
        let _ = unsafe { KillTimer(Some(hwnd), ANIMATION_TIMER_ID) };
    }
    if start_timer {
        // SAFETY: hwnd is live and owned by this thread. Fail-soft like
        // upstream viv.c:9144 (unchecked SetTimer): a failed timer merely
        // freezes the animation.
        let _ = unsafe { SetTimer(Some(hwnd), ANIMATION_TIMER_ID, USER_TIMER_MINIMUM, None) };
    }
    if let Some(rect) = resize_rect {
        // SAFETY: hwnd is live. SetWindowPos synchronously reenters wnd_proc
        // with WM_WINDOWPOSCHANGING/WM_SIZE — DefWindowProcW territory (we
        // handle neither), and no state borrow is live out here.
        // SWP_NOZORDER | SWP_NOACTIVATE: resize/move only, like the initial
        // create.
        let placed = unsafe {
            SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if let Err(e) = placed {
            // System-level (ADR 0001): this is the one-time startup resize
            // of a live window — a silent failure would strand the window
            // at the default size with no retry path. Fail loud instead of
            // discarding the error.
            fatal(&format!("SetWindowPos failed: {e}"));
        }
    }
    if let Some(title) = title.as_ref() {
        // SAFETY: hwnd is live; the HSTRING outlives the call. Fail-soft on
        // purpose: upstream viv.c:1249 ignores SetWindowTextW's return too —
        // a stale caption beats killing the viewer.
        let _ = unsafe { SetWindowTextW(hwnd, title) };
    }
    if invalidate {
        // SAFETY: queues a WM_PAINT; never pumps messages. Erase is FALSE
        // like upstream viv.c:3284 — WM_PAINT fills the whole client itself.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// WM_TIMER for the animation timer: advance the animation by the time
/// elapsed since the previous event and repaint when the displayed frame
/// changed (upstream viv.c:3171-3292).
fn on_animation_timer(hwnd: HWND) {
    // Read the clock before any state borrow — its failure path is the fatal
    // modal (see open_image).
    let now = qpc_now();
    // SAFETY: the borrow spans only scheduler/position field updates and a
    // final InvalidateRect; nothing here pumps messages, so no second
    // state_of borrow can alias this one.
    let repaint = match unsafe { state_of(hwnd) } {
        Some(state) => {
            let freq = state.timer_freq;
            match state.image.as_mut() {
                // The timer only runs while an animation is displayed; the
                // guard also makes a stale timer (failed KillTimer) harmless.
                Some(image) if image.is_animated() => image.advance_on_timer(now, freq),
                _ => false,
            }
        }
        None => false,
    };
    if repaint {
        // The frame counter part ("n / m") tracks the displayed frame
        // (upstream refreshes it in the timer body, viv.c:3277).
        refresh_status(hwnd);
        // SAFETY: queues a WM_PAINT; never pumps messages. Erase is FALSE
        // like upstream viv.c:3284 — WM_PAINT fills the whole client itself.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// Performance-counter reading — the animation clock (upstream
/// os_get_tick_count, os.c:1297-1307).
fn qpc_now() -> u64 {
    let mut tick: i64 = 0;
    // SAFETY: `tick` is a valid out-pointer for the duration of the call.
    // Failure is a broken system facility (documented to always succeed on
    // Windows XP+) and the whole frame-timing model rests on it, so fail
    // loud (ADR 0001) instead of animating on a stuck clock.
    if let Err(e) = unsafe { QueryPerformanceCounter(&mut tick) } {
        fatal(&format!("QueryPerformanceCounter failed: {e}"));
    }
    tick as u64
}

/// Performance-counter frequency, read once at startup (constant for the
/// process lifetime) — upstream os_get_tick_freq, os.c:1310-1319.
fn qpc_frequency() -> Result<u64, String> {
    let mut freq: i64 = 0;
    // SAFETY: `freq` is a valid out-pointer for the duration of the call.
    unsafe { QueryPerformanceFrequency(&mut freq) }
        .map_err(|e| format!("QueryPerformanceFrequency failed: {e}"))?;
    Ok(freq as u64)
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
    // SAFETY: the borrow ends at the end of this statement (the path is
    // cloned out); the modal dialog below pumps messages but no borrow is
    // live by then.
    let initial_dir = (unsafe { state_of(hwnd) })
        .and_then(|s| s.path.clone())
        .and_then(|p| Path::new(&p).parent().map(|d| d.as_os_str().to_os_string()));
    if let Some(path) = open_file_dialog(hwnd, initial_dir.as_deref()) {
        request_open(hwnd, &path, false);
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
                    request_open(hwnd, &path, false);
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

fn on_size(hwnd: HWND) {
    // SAFETY: two sequential borrows, never nested — the first only copies
    // the snapshot, the second reads the bar handle.
    let snapshot = unsafe { state_of(hwnd) }.map(|s| status_snapshot(s, hwnd));
    let Some(snapshot) = snapshot else { return };
    let bar = snapshot_status_bar(hwnd);
    status::update(bar, &snapshot);
    // The common control docks itself to the bottom of the client on
    // WM_SIZE (upstream `_viv_on_size`, viv.c:1615-1620).
    // SAFETY: bar is our live child window.
    unsafe {
        SendMessageW(bar, WM_SIZE, None, None);
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
            // The status bar is a child of this window (#5; upstream creates
            // it in `_viv_status_show(config_show_status)`, viv.c:5415). Its
            // creation failure is system-level (a window without the promised
            // status bar is not the app we shipped).
            // SAFETY: the state pointer was just published above; the borrow
            // ends with this block (status::create only creates a child).
            let hinstance = unsafe { GetModuleHandleW(None) };
            match hinstance {
                Ok(h) => match status::create(hwnd, h.into()) {
                    Ok(bar) => {
                        // SAFETY: the pointer is ours (published above).
                        unsafe { (*state_ptr).status = bar };
                    }
                    Err(msg) => {
                        // Reclaim the box and fail creation — run() reports
                        // the reason through its CreateWindowExW error path.
                        // SAFETY: creation is aborted; the pointer was never
                        // visible to a live window.
                        drop(unsafe { Box::from_raw(state_ptr) });
                        // SAFETY: clearing the slot so a later WM_NCDESTROY
                        // does not adopt a dangling pointer.
                        let _ = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
                        // SAFETY: legal on the owning thread during creation.
                        unsafe { SetLastError(WIN32_ERROR(1)) };
                        eprintln!("status bar creation failed: {msg}");
                        return LRESULT(0); // FALSE aborts CreateWindowExW
                    }
                },
                Err(e) => {
                    // SAFETY: creation is aborted; the pointer was never
                    // visible to a live window.
                    drop(unsafe { Box::from_raw(state_ptr) });
                    // SAFETY: clearing the slot so a later WM_NCDESTROY
                    // does not adopt a dangling pointer.
                    let _ = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
                    eprintln!("GetModuleHandleW failed: {e}");
                    return LRESULT(0);
                }
            }
            // SAFETY: hwnd/msg are exactly what this callback received; forwarding
            // to the default procedure must return its verdict untouched.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NCDESTROY => {
            // Stop the background decode and wait for it before the state
            // box is freed — upstream waits INFINITE for the same reason
            // ("it's critical we wait for load image to finish before we
            // kill the main window", viv.c:5470-5479). The session's flag
            // (set by Drop below or here) bounds the wait to the frame
            // currently decoding.
            // SAFETY: the borrow spans only terminate/quit bookkeeping;
            // quit joins the worker, which blocks but never pumps messages.
            if let Some(state) = unsafe { state_of(hwnd) } {
                if let Some(session) = state.session.as_ref() {
                    session.terminate();
                }
                state.load_thread.quit();
            }
            // SAFETY: the slot holds a live Box pointer set in WM_NCCREATE;
            // take it back, clear the slot, then free (the box's Drop flags
            // any remaining session's job — the worker is already gone).
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
        WM_SIZE => {
            on_size(hwnd);
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
        WM_TIMER => {
            if wparam.0 == ANIMATION_TIMER_ID {
                on_animation_timer(hwnd);
                LRESULT(0)
            } else {
                // SAFETY: hwnd/msg are exactly what this callback received;
                // the default procedure handles everything we do not.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        // The background decode's kick: the queue holds the replies, this
        // just wakes the UI thread to drain them (upstream _VIV_WM_REPLY).
        REPLY_KICK_MESSAGE => {
            on_load_replies(hwnd);
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

/// Outer window rect: client = image size shrunk to fit the work area of
/// the monitor under the cursor (upstream centers on the cursor's monitor,
/// viv.c:5359-5387); no image -> 60% auto-fit with a 640x480 floor.
/// `status_h` is the status bar's height, added back so the IMAGE keeps
/// its fitted size above the bar (upstream adds `_viv_get_status_high()`
/// when computing the window rect from the desired client, viv.c:2148).
fn initial_window_rect(image: Option<&LoadedImage>, status_h: i32) -> Result<RECT, String> {
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
        let avail_h = (work.bottom - work.top - (frame.bottom - frame.top) - status_h).max(1);
        let (cw, ch) = match image {
            // Window = image size (upstream Alt+2 semantics); the remembered-rect /
            // 60%-first-run model returns with M3 config persistence.
            Some(img) => fit_shrink(img.width(), img.height(), avail_w, avail_h),
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
            // The image fits above the status bar; the bar's height rides
            // on top of the fitted client (viv.c:2148/2155).
            bottom: ch + status_h,
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

/// The status bar's height before its window exists (used only for the
/// initial rect at startup — the live bar is measured via
/// `status::height` afterwards). comctl32 sizes a status bar from the
/// system status font and border metrics; we reproduce that formula
/// (border * 2 + font height) at the system DPI so the first window rect
/// already accounts for the bar.
fn initial_status_height() -> i32 {
    // SAFETY: desktop DC queries on the calling thread.
    unsafe {
        let hdc = windows::Win32::Graphics::Gdi::GetDC(None);
        if hdc.is_invalid() {
            return 0;
        }
        let dpi = windows::Win32::Graphics::Gdi::GetDeviceCaps(
            Some(hdc),
            windows::Win32::Graphics::Gdi::LOGPIXELSY,
        );
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc);
        // Upstream's bar at 96 DPI is 22 px (SM_CYVTHUMB=20 + borders);
        // scale from there — comctl32's own formula is font-height based
        // and lands on the same value.
        let border = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYBORDER,
        );
        ((20 * dpi) / 96) + border * 2
    }
}

pub(crate) fn fatal(message: &str) -> ! {
    let text = to_wide(message);
    // SAFETY: a null owner is allowed for a modal error box (system-level
    // failure path — ADR 0001 fail loud).
    let _ = unsafe { MessageBoxW(None, PCWSTR(text.as_ptr()), CLASS_NAME, MB_ICONERROR) };
    std::process::exit(1)
}

pub(crate) fn run(arg_path: Option<OsString>) -> Result<(), String> {
    // The animation clock's unit, read once (constant for the process
    // lifetime). Read before any window exists: failure is fatal (ADR 0001).
    let timer_freq = qpc_frequency()?;
    // The decode worker, started once per process before any load is
    // requested (at most one decode is ever active — see `loadthread.rs`).
    // A spawn failure is system-level: fail loud (ADR 0001) via run's Err.
    let load_thread = LoadThread::start()?;
    let state = WindowState {
        image: None,
        path: None,
        timer_freq,
        load_thread,
        session: None,
        displayed_from: None,
        // Bound to the startup session's id when it is queued below: its
        // first frame resizes the window to the image (M1's image-sized
        // window, preserved under async startup — the window must exist
        // before decoding starts); a superseded or failed startup load
        // cannot leak the resize into a later open.
        startup_resize_session: None,
        animation_timer_running: false,
        // The status bar is created in WM_NCCREATE (the window handle must
        // exist first) and written into the state there.
        status: HWND::default(),
        status_file_not_found: false,
        status_load_failed: false,
        displayed_file_bytes: None,
        pending_file_bytes: None,
    };
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

    // The status bar does not exist yet (created in WM_NCCREATE) — its
    // height is the standard common-control height at the system DPI
    // (upstream reads the live window; the formula matches comctl32's own:
    // border + 3/2 of the system status font's line height).
    let status_h = initial_status_height();
    let rect = initial_window_rect(None, status_h)?;
    let title = HSTRING::from_wide(&title_wide(None));
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

    // Kick the startup load off the UI thread: a huge argument file must
    // not freeze the window before it is even shown (issue #4). Replies
    // (first frame, animation frames, terminal) arrive on the kick message
    // and drive everything from there.
    if let Some(path) = arg_path.as_ref() {
        request_open(hwnd, path, true);
    }

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

    // Populate the status bar (parts + initial texts) now that the window
    // has its final rect — the first WM_SIZE fired during creation, before
    // WM_NCCREATE had even stored the state pointer, so the bar is still
    // empty. Upstream populates it at startup the same way (viv.c:5415).
    refresh_status(hwnd);

    let mut msg = MSG::default();
    loop {
        // SAFETY: standard pump over this thread's queue.
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        match r.0 {
            0 => break, // WM_QUIT
            // Fail loud instead of spinning on a broken pump (ADR 0001); with
            // filter params (None, 0, 0) this is near-unreachable in practice.
            -1 => {
                // SAFETY: reading the thread's last error immediately after the failed call.
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
