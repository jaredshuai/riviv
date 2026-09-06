//! Window shell: window state, the message pump, class registration, and the
//! input/open-action handlers hung off the single wnd_proc.
//!
//! The animation timer wiring (#3), the background decode protocol (#4),
//! the status bar (#5) and the playlist/navigation wiring (#6) are landed:
//! opens spawn a load session whose replies arrive on a private `WM_APP+1`
//! kick and are applied by `on_load_replies` — the display swaps when the
//! first frame replies in, animation frames append while playing, and a
//! stalled prefix waits for the decode (see `loader::apply_reply` and
//! `loadthread.rs`). Drops build playlists (multi-file/folder/Shift,
//! viv.c:3076-3128) and Right/Left/PgUp/PgDn/Home/End navigate them — or,
//! with no playlist, the current file's folder (`playlist.rs`). Zoom & pan
//! (#7): the wheel and the +/- keys step the 16-level preset curve
//! anchored at the cursor or the viewport center, left-drag pans with edge
//! clamping, Ctrl+0 resets to fit and Ctrl+Alt+0 toggles the temporary
//! 1:1 mode (`zoom.rs`). Fullscreen (#8): double-click, Alt+Return or Esc
//! toggles a borderless cover of the current monitor with the pre-toggle
//! rect (and zoomed state) restored on exit; the status bar is destroyed
//! for the cover and recreated after; and an idle cursor hides after 2 s
//! in fullscreen, reappearing on movement (`cursor.rs`).

use std::ffi::{OsStr, OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, GetMonitorInfoW, HBRUSH, InvalidateRect, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow, PtInRect,
    ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_ADD, VK_CONTROL, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT,
    VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SUBTRACT,
};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CreateWindowExW,
    DefWindowProcW, DestroyWindow, DispatchMessageW, GWL_STYLE, GWLP_USERDATA, GetClientRect,
    GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect, HWND_TOP,
    IDC_ARROW, IsZoomed, KillTimer, LoadCursorW, MB_ICONERROR, MINMAXINFO, MSG, MessageBoxW,
    PostQuitMessage, RegisterClassExW, SHOW_WINDOW_CMD, SW_MAXIMIZE, SW_SHOW, SW_SHOWNORMAL,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetProcessDPIAware, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowCursor, ShowWindow, TranslateMessage, USER_TIMER_MINIMUM, WINDOW_EX_STYLE,
    WM_ACTIVATE, WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_SIZE, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_CAPTION,
    WS_EX_ACCEPTFILES, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_THICKFRAME, WS_VISIBLE, WindowFromPoint,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::anim::ANIMATION_TIMER_ID;
use crate::cursor::{self, CursorVisibility};
use crate::fit::fit_shrink;
use crate::loader::{LoadedImage, UiAction, apply_reply, map_reply_frame};
use crate::loadthread::{LoadSession, LoadThread, REPLY_KICK_MESSAGE};
use crate::paint::paint;
use crate::playlist::{self, Playlist, PlaylistEntry};
use crate::status;
use crate::surface::Surface;
use crate::text::{dialog_filter, title_wide, to_wide};
use crate::zoom::{View, Viewport};

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
    /// The navigation playlist (#6; upstream `_viv_playlist_*` globals,
    /// viv.c:656-661). Insertion order; navigation sorts on the fly.
    pub(crate) playlist: Playlist,
    /// The last REQUESTED open — the navigation reference point (upstream
    /// `_viv_current_fd`, set synchronously in `_viv_open` at request time,
    /// viv.c:1574-1579, so it can trail what is on screen while a load is
    /// in flight; a direct open that never got as far as `_viv_open` —
    /// unstatable path — leaves it untouched).
    pub(crate) nav_current: Option<PlaylistEntry>,
    /// Whether the NEXT resolved open is the process's startup open — its
    /// first frame resizes the window to the image (M1's image-sized
    /// window under async startup). Set while parsing the command line's
    /// file arguments (any of their open flavors — direct, folder home,
    /// wildcard home); consumed by the first `request_open`, and cleared
    /// without opening when the command line resolves to nothing.
    pub(crate) startup_open_pending: bool,
    /// Zoom/pan view of the displayed image (#7; upstream's
    /// `_viv_zoom_pos`/`_viv_view_*` globals, viv.c:677-683). Reset on
    /// every display swap and blank, exactly where upstream runs
    /// `_viv_clear` (viv.c:1282-1288).
    pub(crate) view: View,
    /// In-progress left-drag pan: the last cursor point in client pixels
    /// (upstream `_viv_doing == _VIV_DOING_SCROLL` + `_viv_doing_x/y`,
    /// viv.c:14682-14693). `None` = not dragging.
    pub(crate) drag: Option<(i32, i32)>,
    /// Fullscreen state (#8; upstream `_viv_is_fullscreen`,
    /// `_viv_fullscreen_is_maxed`, `_viv_fullscreen_rect`,
    /// `_viv_fullscreen_zoom_offset`, viv.c:704-707). The rect is the
    /// pre-fullscreen normal placement (captured AFTER un-maximizing), so
    /// exit restores onto it — re-maximizing first when the toggle began
    /// from a zoomed window.
    pub(crate) fullscreen: bool,
    pub(crate) fullscreen_was_maxed: bool,
    pub(crate) fullscreen_restore_rect: RECT,
    pub(crate) fullscreen_zoom_offset: i32,
    /// Cursor visibility state machine (#8; upstream `_viv_is_cursor_shown`
    /// + `_viv_is_hide_cursor_timer`, viv.c:709/713) — see `cursor.rs`.
    pub(crate) cursor: CursorVisibility,
    /// Mouse-leave tracking is armed (upstream `_viv_is_tracking_mouse`,
    /// viv.c:781) and the mouse is currently over the window
    /// (`_viv_is_mouseover`, viv.c:782) — the hide-cursor conditions read
    /// the latter.
    pub(crate) tracking_mouse: bool,
    pub(crate) is_mouseover: bool,
    /// The last seen cursor position (upstream `_viv_mousemove_x/y`,
    /// viv.c:714-715) — the movement dedupe behind the cursor
    /// show/restart cycle; (-1, -1) while the mouse is away (reset by
    /// WM_MOUSELEAVE).
    pub(crate) last_cursor_pt: POINT,
    /// Suppress the WM_ACTIVATE deactivate-show during the fullscreen
    /// dummy-window dance (upstream `_viv_prevent_on_deactivate`,
    /// viv.c:784/6712/6780): the momentary deactivate must not force-show
    /// a cursor the cycle had hidden.
    pub(crate) prevent_deactivate_show: bool,
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

/// The zoom/pan geometry inputs from the current state: the render viewport
/// (client area minus the status bar — upstream's `wide`/`high`, e.g.
/// viv.c:13954-13957) and the displayed image's source size. A blank
/// display yields (0, 0), against which the zoom model is inert like
/// upstream's `_viv_get_render_size` no-image early-out (viv.c:6867).
fn viewport_and_src(hwnd: HWND, state: &WindowState) -> (Viewport, (i32, i32)) {
    let mut client = RECT::default();
    // SAFETY: read-only query on the live window; a failed read leaves the
    // zeroed rect and collapses the viewport (the zoom math no-ops).
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let status_h = crate::status::height(state.status);
    let vp = Viewport {
        wide: (client.right - client.left).max(0),
        high: (client.bottom - client.top - status_h).max(0),
    };
    let src = state
        .image
        .as_ref()
        .map(|img| (img.width(), img.height()))
        .unwrap_or((0, 0));
    (vp, src)
}

/// Gather `_viv_should_show_cursor`'s live inputs (viv.c:14593-14619): a
/// viewable image is up, we are foreground, the mouse is over us, nothing
/// holds the capture, and (fullscreen OR the `windowed_hide_cursor`
/// config). riviv wires that config input to `false` — the cursor only
/// hides in fullscreen (README deviation; the upstream default is 1).
fn cursor_conditions(hwnd: HWND, state: &WindowState) -> cursor::CursorConditions {
    cursor::CursorConditions {
        has_viewable_image: state.nav_current.is_some()
            && !state.status_file_not_found
            && !state.status_load_failed,
        // SAFETY: read-only query of the foreground window.
        foreground: unsafe { GetForegroundWindow() } == hwnd,
        mouseover: state.is_mouseover,
        // SAFETY: read-only query of this thread's capture window.
        captured: !unsafe { GetCapture() }.is_invalid(),
        fullscreen: state.fullscreen,
        hide_when_windowed: false,
    }
}

/// Perform one cursor-step's Win32 effects — always OUTSIDE any state
/// borrow, in upstream's order (timer dies, polarity flips, a fresh timer
/// may arm; viv.c:14559-14640).
fn apply_cursor(hwnd: HWND, effects: cursor::CursorEffects) {
    if effects.kill_timer {
        // SAFETY: hwnd is live; a failed kill leaves a stale timer whose
        // WM_TIMER guard no-ops.
        let _ = unsafe { KillTimer(Some(hwnd), cursor::HIDE_CURSOR_TIMER_ID) };
    }
    if let Some(show) = effects.show_cursor {
        // SAFETY: adjusts this thread's cursor display count by exactly one.
        unsafe { ShowCursor(show) };
    }
    if effects.start_timer {
        // SAFETY: hwnd is live and owned by this thread. Fail-soft like
        // upstream's unchecked SetTimer (viv.c:14637): a failed timer
        // merely keeps the cursor visible.
        let _ = unsafe {
            SetTimer(
                Some(hwnd),
                cursor::HIDE_CURSOR_TIMER_ID,
                cursor::HIDE_CURSOR_DELAY_MS,
                None,
            )
        };
    }
}

/// `_viv_update_show_cursor` (viv.c:14621-14628): reconcile the cursor
/// with the current conditions — show it, or (re)arm the hide cycle.
fn update_cursor(hwnd: HWND) {
    // SAFETY: the borrow spans the condition gather and the pure state
    // machine; the effects run after it drops.
    let effects = (unsafe { state_of(hwnd) }).map(|state| {
        let conditions = cursor_conditions(hwnd, state);
        state.cursor.update(&conditions)
    });
    if let Some(effects) = effects {
        apply_cursor(hwnd, effects);
    }
}

/// `_viv_show_cursor` (viv.c:14559-14571): force the cursor visible and
/// stop the hide cycle.
fn show_cursor(hwnd: HWND) {
    // SAFETY: the borrow spans the pure state machine.
    let effects = (unsafe { state_of(hwnd) }).map(|state| state.cursor.show());
    if let Some(effects) = effects {
        apply_cursor(hwnd, effects);
    }
}

/// `_viv_show_cursor(); _viv_update_show_cursor();` — the button-press
/// pairing (viv.c:3300-3301/3322-3323).
fn show_and_update_cursor(hwnd: HWND) {
    show_cursor(hwnd);
    update_cursor(hwnd);
}

/// The WM_TIMER hide-cursor arm (upstream viv.c:3161-3169).
fn on_hide_cursor_timer(hwnd: HWND) {
    // SAFETY: the borrow spans the condition gather and the pure machine.
    let effects = (unsafe { state_of(hwnd) }).map(|state| {
        let conditions = cursor_conditions(hwnd, state);
        state.cursor.timer_fired(&conditions)
    });
    if let Some(effects) = effects {
        apply_cursor(hwnd, effects);
    }
}

/// WM_MOUSELEAVE (upstream viv.c:3562-3588): the TME_LEAVE tracking
/// expired — the mouse left the window. Clear the mouseover verdict and
/// the movement dedupe, and make sure the cursor is visible (the hide
/// conditions can no longer hold). The src-pixel part of upstream's
/// handler belongs to the unimplemented pixel-info feature.
fn on_mouse_leave(hwnd: HWND) {
    // SAFETY: the borrow spans the flag resets and the pure cursor step.
    let effects = (unsafe { state_of(hwnd) }).map(|state| {
        state.tracking_mouse = false;
        state.is_mouseover = false;
        state.last_cursor_pt = POINT { x: -1, y: -1 };
        state.cursor.show()
    });
    if let Some(effects) = effects {
        apply_cursor(hwnd, effects);
    }
}

/// The fullscreen dummy window's class — upstream registers
/// "_VIV_FULLSCREEN" per toggle (viv.c:6698-6712); riviv names its own so
/// both viewers coexist like with the main class.
const FULLSCREEN_DUMMY_CLASS: PCWSTR = w!("riviv_fullscreen");

/// The dummy's wnd_proc: the window is created, foregrounded and destroyed
/// within one sweep with no message dispatch in between, so default
/// handling is all it can ever see — except the background ERASE, which
/// upstream suppresses (return 1, viv.c:11730-11731) so the momentary
/// dummy does not flash a gray fill over the screen.
unsafe extern "system" fn fullscreen_dummy_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        // SAFETY: hwnd/msg are exactly what this callback received; the
        // default procedure handles everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// WM_LBUTTONDBLCLK (upstream viv.c:3298-3318): the default left-click
/// action's double-click arm is the fullscreen toggle (action 0 is one of
/// the toggling actions; 3/4 would run the click action instead). The
/// first click of the pair started a drag, but the intervening
/// WM_LBUTTONUP already ended it — the drag is over by the time the DBLCLK
/// arrives. The cursor reappears first, like on every button message.
fn on_double_click(hwnd: HWND) {
    show_and_update_cursor(hwnd);
    toggle_fullscreen(hwnd);
}

/// `_viv_toggle_fullscreen` (viv.c:6574-6821). Enter: strip caption +
/// thick frame, un-maximize (remembering it), save the window rect,
/// destroy the status bar, cover the current monitor, and run the
/// dummy-window dance so the shell takes the taskbar away. Exit: restore
/// every piece onto the saved rect, re-maximizing when the toggle began
/// zoomed. The zoom level rides the `_viv_fullscreen_zoom_offset` math
/// (both fill modes are off in riviv, so the level is preserved); 1:1 mode
/// is dropped without restoring its saved level (viv.c:6610). WM_SIZE
/// fires naturally through the dance; the explicit `on_size` at the end
/// re-anchors at the FINAL geometry + level — exactly upstream's
/// suppressed-then-manual `_viv_on_size` (viv.c:6604/6783-6785).
fn toggle_fullscreen(hwnd: HWND) {
    // The offset inputs are gathered at the OLD viewport before anything
    // moves (viv.c:6584-6601) — the size sweep inherits the current 1:1
    // flag, so a toggle from 1:1 measures every level as the source size.
    // SAFETY: the borrow spans the pure geometry gather.
    let gathered = (unsafe { state_of(hwnd) }).map(|state| {
        let (vp, src) = viewport_and_src(hwnd, state);
        let old_render = state.view.render_size(src.0, src.1, vp);
        let sizes = state.view.sizes_all_levels(src.0, src.1, vp);
        (state.fullscreen, state.view.level(), old_render, sizes, src)
    });
    let Some((was_fullscreen, level, old_render, sizes, src)) = gathered else {
        return;
    };
    // The monitor is chosen from the WINDOWED position — upstream calls
    // os_MonitorRectFromWindow(hwnd, 1) before any move (viv.c:6667): the
    // FULL rcMonitor (not the work area) of the monitor holding the window,
    // falling back to the primary (os.c:249-276).
    // SAFETY: read-only monitor queries on the live window.
    let monitor_rect = unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // Fail-soft like upstream's os_MonitorRectFromWindow, which never
        // checks GetMonitorInfo either; with DEFAULTTOPRIMARY the handle is
        // never null, so the zeroed rect is theoretical.
        let _ = GetMonitorInfoW(monitor, &mut mi);
        mi.rcMonitor
    };
    {
        // SAFETY: the borrow spans only the flag flips.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        // Before any resize, like upstream (viv.c:6616/6665) — the cursor
        // conditions read it.
        state.fullscreen = !was_fullscreen;
        // viv.c:6610: 1:1 dies here WITHOUT restoring its saved level.
        state.view.leave_one_to_one();
    }
    if was_fullscreen {
        // ---- exit (viv.c:6616-6658) ----
        // The bar is RECREATED first (upstream `_viv_status_show(1)` at
        // 6645 precedes the style commit at 6648; it destroys and recreates
        // rather than hiding, viv.c:10932-10963).
        // SAFETY: returns this exe's module handle; no side effects.
        match unsafe { GetModuleHandleW(None) } {
            Ok(hinstance) => {
                let bar = match status::create(hwnd, hinstance.into()) {
                    Ok(bar) => bar,
                    Err(msg) => {
                        // Same graceful degradation as startup: a NULL bar
                        // no-ops everywhere.
                        eprintln!("status bar unavailable: {msg}");
                        HWND::default()
                    }
                };
                // SAFETY: the borrow spans only the field store.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    state.status = bar;
                }
            }
            Err(e) => eprintln!("GetModuleHandleW failed: {e} (no status bar)"),
        }
        // SAFETY: read-modify-write of the style on the owning thread.
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        // SAFETY: hwnd is live; riviv always shows caption + thick frame
        // (upstream restores per config, whose defaults are both on —
        // config.c:85-86).
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                (style | WS_CAPTION.0 | WS_THICKFRAME.0) as isize,
            );
        }
        // SAFETY: the borrow spans only the copies out.
        let (rect, offset) = (unsafe { state_of(hwnd) })
            .map(|state| (state.fullscreen_restore_rect, state.fullscreen_zoom_offset))
            .unwrap_or_default();
        // SAFETY: hwnd is live; reenters wnd_proc with WM_SIZE — no borrow
        // is live out here (on_size takes its own). A failure leaves the
        // state flag ahead of the real window shape; diagnose, then the
        // next toggle resyncs (upstream viv.c:6650 ignores the result too).
        if let Err(e) = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            )
        } {
            eprintln!("fullscreen restore SetWindowPos failed: {e}");
        }
        // SAFETY: the borrow spans only the copy out.
        let was_maxed = (unsafe { state_of(hwnd) }).is_some_and(|state| state.fullscreen_was_maxed);
        if was_maxed {
            // SAFETY: hwnd is live; SW_MAXIMIZE re-zooms onto the placement
            // restored just above (upstream order, viv.c:6652-6655).
            if !unsafe { ShowWindow(hwnd, SW_MAXIMIZE) }.as_bool() {
                eprintln!("SW_MAXIMIZE after fullscreen failed");
            }
        }
        // The level rides the stored offset back up, AFTER the resize —
        // the final on_size below re-anchors at the final level, like
        // upstream's manual `_viv_on_size` after viv.c:6657-6658.
        // SAFETY: the borrow spans the pure level move.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.view.shift_level(offset);
        }
    } else {
        // ---- enter (viv.c:6659-6781) ----
        // SAFETY: read-modify-write of the style on the owning thread.
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        // SAFETY: hwnd is live.
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                (style & !(WS_CAPTION.0 | WS_THICKFRAME.0)) as isize,
            );
        }
        // SAFETY: read-only zoomed query.
        let was_maxed = unsafe { IsZoomed(hwnd) }.as_bool();
        if was_maxed {
            // Un-maximize FIRST so the saved rect is the normal placement,
            // not the zoomed one (upstream order, viv.c:6671-6675).
            // SAFETY: hwnd is live.
            if !unsafe { ShowWindow(hwnd, SW_SHOWNORMAL) }.as_bool() {
                eprintln!("SW_SHOWNORMAL before fullscreen failed");
            }
        }
        let mut rect = RECT::default();
        // SAFETY: read-only geometry query; fail-soft leaves the zeroed
        // rect like upstream's unchecked GetWindowRect (viv.c:6677).
        let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
        // Take the bar out of the state inside the borrow, tear it down
        // OUTSIDE: DestroyWindow delivers messages (the child's teardown
        // plus a WM_PARENTNOTIFY here) and any future handler arm on those
        // must not alias this borrow.
        let bar = {
            // SAFETY: the borrow spans only field stores.
            let Some(state) = (unsafe { state_of(hwnd) }) else {
                return;
            };
            state.fullscreen_was_maxed = was_maxed;
            state.fullscreen_restore_rect = rect;
            std::mem::take(&mut state.status)
        };
        if !bar.is_invalid() {
            // SAFETY: our live child window, torn down on the owning thread.
            let _ = unsafe { DestroyWindow(bar) };
        }
        // SAFETY: hwnd is live; covers the monitor, reentering wnd_proc
        // with WM_SIZE (no borrow live). Failure diagnosed like the restore
        // path (upstream viv.c:6683 ignores the result too).
        if let Err(e) = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                monitor_rect.left,
                monitor_rect.top,
                monitor_rect.right - monitor_rect.left,
                monitor_rect.bottom - monitor_rect.top,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            )
        } {
            eprintln!("fullscreen cover SetWindowPos failed: {e}");
        }
        // The dummy-window dance (viv.c:6685-6715): a borderless window
        // created fullscreen and immediately destroyed teaches the shell
        // to drop the taskbar ("without this dummy window, sometimes the
        // taskbar will not disappear", upstream comment). Its momentary
        // deactivate must not force-show a hidden cursor (viv.c:6685).
        // SAFETY: the borrow spans only the flag store.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.prevent_deactivate_show = true;
        }
        // SAFETY: returns this exe's module handle; no side effects.
        if let Ok(hinstance) = unsafe { GetModuleHandleW(None) } {
            // Upstream re-registers the class on every toggle and ignores
            // the result — the second registration fails benignly with
            // ERROR_CLASS_ALREADY_EXISTS.
            // SAFETY: wc outlives the call; that failure is benign.
            let _ = unsafe {
                RegisterClassExW(&WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(fullscreen_dummy_proc),
                    hInstance: hinstance.into(),
                    hCursor: {
                        // SAFETY: IDC_ARROW is a predefined shared resource;
                        // a failure degrades to a cursorless momentary dummy.
                        LoadCursorW(None, IDC_ARROW).unwrap_or_default()
                    },
                    hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut c_void),
                    lpszClassName: FULLSCREEN_DUMMY_CLASS,
                    ..Default::default()
                })
            };
            let title = HSTRING::from_wide(&title_wide(None));
            // SAFETY: all parameters are valid for the call; the dummy is
            // created, foregrounded and destroyed in the same sweep.
            let dummy = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    FULLSCREEN_DUMMY_CLASS,
                    &title,
                    WS_POPUP | WS_VISIBLE,
                    monitor_rect.left,
                    monitor_rect.top,
                    monitor_rect.right - monitor_rect.left,
                    monitor_rect.bottom - monitor_rect.top,
                    None,
                    None,
                    Some(hinstance.into()),
                    None,
                )
            };
            if let Ok(dummy) = dummy {
                // SAFETY: dummy is live per its creator; a failed
                // foreground request only risks the taskbar staying put.
                let _ = unsafe { SetForegroundWindow(dummy) };
                // SAFETY: dummy is live and owned by this thread.
                let _ = unsafe { DestroyWindow(dummy) };
            }
        }
        // SAFETY: the borrow spans only the flag store.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.prevent_deactivate_show = false;
        }
        // The zoom offset (viv.c:6718-6778), computed AFTER the cover like
        // upstream: the fill_window branch reads sizes LIVE at the
        // fullscreen viewport with 1:1 already cleared — a fresh sweep now,
        // while the fullscreen_fill branch keeps the pre-toggle sweep from
        // the gather above. Both fill modes are off in riviv, so the offset
        // is always 0 and the level survives — kept wired and pure for the
        // M3 config hookup.
        // SAFETY: the borrow spans the fullscreen-viewport sweep and the
        // pure offset math; nothing here pumps messages.
        let offset = (unsafe { state_of(hwnd) })
            .map(|state| {
                let (fs_vp, _) = viewport_and_src(hwnd, state);
                let sizes_fs = state.view.sizes_all_levels(src.0, src.1, fs_vp);
                crate::zoom::fullscreen_zoom_offset(
                    false,
                    false,
                    &sizes,
                    &sizes_fs,
                    Viewport {
                        wide: monitor_rect.right - monitor_rect.left,
                        high: monitor_rect.bottom - monitor_rect.top,
                    },
                    old_render,
                    level,
                )
            })
            .unwrap_or(0);
        // SAFETY: the borrow spans field stores and the pure level move
        // (viv.c:6777-6778).
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.fullscreen_zoom_offset = offset;
            state.view.shift_level(-offset);
        }
    }
    // The deferred on-size, at the FINAL geometry + level (upstream's
    // manual `_viv_on_size`, viv.c:6785): re-anchors the pan and re-docks
    // the (possibly recreated) bar.
    on_size(hwnd);
    // Refresh the mouseover verdict from where the cursor actually is now
    // that the window moved under it (viv.c:6789-6815).
    let mut pt = POINT::default();
    // SAFETY: read-only cursor query; fail-soft leaves (0, 0).
    let _ = unsafe { GetCursorPos(&mut pt) };
    let mut is_mouseover = false;
    // SAFETY: read-only hit-test queries on the live window.
    unsafe {
        if WindowFromPoint(pt) == hwnd {
            let _ = ScreenToClient(hwnd, &mut pt);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            is_mouseover = PtInRect(&client, pt).as_bool();
        }
    }
    {
        // SAFETY: the borrow spans only the field store.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.is_mouseover = is_mouseover;
        }
    }
    update_cursor(hwnd);
    // A level shift can change the render size without moving the pan
    // offset — always queue the repaint (CS_HREDRAW/VREDRAW already cover
    // the resize itself).
    repaint(hwnd);
}

/// The signed point packed in a mouse-message LPARAM (GET_X_LPARAM /
/// GET_Y_LPARAM semantics — each half-word is a signed coordinate).
fn lparam_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xffff) as u16 as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32,
    }
}

/// Queue a WM_PAINT (erase FALSE — WM_PAINT fills the whole client itself,
/// upstream viv.c:3284).
fn repaint(hwnd: HWND) {
    // SAFETY: queues a WM_PAINT; never pumps messages.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

/// WM_MOUSEWHEEL (upstream viv.c:3673-3677 → `_viv_do_mousewheel_action`
/// action 0: the default config maps BOTH the plain and the Ctrl wheel to
/// zoom). One level per message, anchored at the cursor — upstream keys
/// off the delta's sign only, so a zero delta counts as zoom-out.
fn on_mousewheel(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
    // Unlike other mouse messages, the wheel's lParam holds SCREEN coords.
    let mut pt = lparam_point(lparam);
    // SAFETY: in-place conversion on the live window; a failure leaves the
    // screen point, which merely anchors elsewhere.
    let _ = unsafe { ScreenToClient(hwnd, &mut pt) };
    // SAFETY: the borrow spans only the pure zoom math — nothing pumps.
    let changed = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let (vp, src) = viewport_and_src(hwnd, state);
        state
            .view
            .zoom_step(delta <= 0, (pt.x, pt.y), src.0, src.1, vp)
    });
    if changed {
        repaint(hwnd);
    }
}

/// A `+`/`-` keypress zoom step — anchored at the viewport center (upstream
/// `_viv_zoom_in` with have_xy=0 feeds the client-area center through the
/// same wheel action, viv.c:11803-11821).
fn zoom_step_centered(hwnd: HWND, out: bool) {
    // SAFETY: the borrow spans only the pure zoom math.
    let changed = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let (vp, src) = viewport_and_src(hwnd, state);
        state
            .view
            .zoom_step(out, (vp.wide / 2, vp.high / 2), src.0, src.1, vp)
    });
    if changed {
        repaint(hwnd);
    }
}

/// Ctrl+0 — back to fit (upstream `VIV_ID_VIEW_ZOOM_RESET`, viv.c:1676-1681;
/// always repaints).
fn zoom_reset(hwnd: HWND) {
    // SAFETY: the borrow spans the pure reset math.
    if let Some(state) = unsafe { state_of(hwnd) } {
        let (vp, src) = viewport_and_src(hwnd, state);
        state.view.reset_zoom(src.0, src.1, vp);
    }
    repaint(hwnd);
}

/// Ctrl+Alt+0 — toggle the temporary 1:1 pixel-exact mode (upstream
/// `_viv_view_1to1`, viv.c:9318-9339; always repaints).
fn toggle_one_to_one(hwnd: HWND) {
    // SAFETY: the borrow spans the pure toggle math.
    if let Some(state) = unsafe { state_of(hwnd) } {
        let (vp, src) = viewport_and_src(hwnd, state);
        state.view.toggle_one_to_one(src.0, src.1, vp);
    }
    repaint(hwnd);
}

/// WM_LBUTTONDOWN — show the cursor, restart its cycle, then start a drag
/// pan (upstream's default left-click action 0, viv.c:3320-3325 +
/// 14682-14693: with a caption present the drag always pans; the
/// borderless move-window arm is not riviv's business while it always has
/// a caption outside fullscreen). No image-size gate — panning a fitted
/// image simply clamps to nothing.
fn on_left_button_down(hwnd: HWND, lparam: LPARAM) {
    show_and_update_cursor(hwnd);
    let pt = lparam_point(lparam);
    // SAFETY: the borrow spans only the drag-point store.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.drag = Some((pt.x, pt.y));
    }
    // SAFETY: hwnd is live and owned by this thread. Capture is released in
    // WM_LBUTTONUP; a capture stolen elsewhere just freezes the drag until
    // the next click — upstream never handles WM_CAPTURECHANGED here either.
    let _ = unsafe { SetCapture(hwnd) };
}

/// WM_MOUSEMOVE — mouse-leave tracking, the cursor cycle, then the drag
/// pan (upstream viv.c:3575-3660 + `_viv_mousemove`, viv.c:9151-9170).
fn on_mouse_move(hwnd: HWND, lparam: LPARAM) {
    // Arm TME_LEAVE tracking once. The flag flips BEFORE the call and OUT
    //SIDE any borrow: TrackMouseEvent can deliver WM_MOUSELEAVE
    // SYNCHRONOUSLY (upstream's "ui must come first" comment, viv.c:3593),
    // and that handler takes its own state borrow — ours must be gone.
    // SAFETY: the borrow spans only the flag read.
    let arm_tracking = (unsafe { state_of(hwnd) }).is_some_and(|state| !state.tracking_mouse);
    if arm_tracking {
        // SAFETY: the borrow spans only the flag store.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.tracking_mouse = true;
        }
        let mut tme = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        // SAFETY: tme outlives the call; a failed track only costs the
        // WM_MOUSELEAVE verdict (mouseover then stays true too long,
        // exactly like upstream's unchecked _TrackMouseEvent).
        let _ = unsafe { TrackMouseEvent(&mut tme) };
    }
    let mut screen_pt = POINT::default();
    // SAFETY: read-only cursor query; fail-soft leaves (0, 0), which reads
    // as movement at most once.
    let _ = unsafe { GetCursorPos(&mut screen_pt) };
    // SAFETY: the borrow spans the mouseover verdict, the movement dedupe,
    // the condition gather and the pure cursor step.
    let effects = (unsafe { state_of(hwnd) }).map(|state| {
        // Upstream sets mouseover = 1 unconditionally after the tracking
        // block (viv.c:3609) — even over a synchronous LEAVE — replicated.
        state.is_mouseover = true;
        let moved = screen_pt != state.last_cursor_pt;
        state.last_cursor_pt = screen_pt;
        let conditions = cursor_conditions(hwnd, state);
        state.cursor.mouse_moved(moved, &conditions)
    });
    if let Some(effects) = effects {
        apply_cursor(hwnd, effects);
    }
    // While dragging, pan by the cursor delta (upstream `_VIV_DOING_SCROLL`,
    // viv.c:3617-3644: the image follows the mouse).
    let pt = lparam_point(lparam);
    // SAFETY: the borrow spans the drag bookkeeping and the pure pan math.
    let panned = (unsafe { state_of(hwnd) }).is_some_and(|state| match state.drag {
        Some((lx, ly)) => {
            state.drag = Some((pt.x, pt.y));
            if pt.x == lx && pt.y == ly {
                false
            } else {
                let (vp, src) = viewport_and_src(hwnd, state);
                state.view.scroll_by(pt.x - lx, pt.y - ly, src.0, src.1, vp);
                true
            }
        }
        None => false,
    });
    if panned {
        repaint(hwnd);
    }
}

/// WM_LBUTTONUP — end the drag (upstream `_viv_doing_cancel`,
/// viv.c:7850-7871: the capture is released iff something was in progress).
fn on_left_button_up(hwnd: HWND) {
    // SAFETY: the borrow spans only the Option take.
    let was_dragging = (unsafe { state_of(hwnd) }).is_some_and(|state| state.drag.take().is_some());
    if was_dragging {
        // SAFETY: we took the capture in WM_LBUTTONDOWN on this thread.
        let _ = unsafe { ReleaseCapture() };
    }
}

/// How an open request came about — whether the navigation reference
/// (`nav_current`) follows it (upstream `_viv_open` copies `_viv_current_fd`
/// synchronously, viv.c:1574-1579).
enum OpenOrigin<'a> {
    /// A direct pick (drop of one file, dialog, CLI argument): the
    /// reference becomes a fresh id-0 entry (upstream zeroes dwReserved for
    /// direct opens, viv.c:1375-1376).
    Direct,
    /// A navigation target: the reference is the entry itself, id and all
    /// (upstream opens the playlist fd including its ids).
    Nav(&'a PlaylistEntry),
}

/// Queue `path` for background decoding (upstream `_viv_open`'s
/// CreateThread arm, viv.c:1569). The current display stays up until this
/// load's first frame replies in; storing a new session supersedes
/// (flags) any in-flight one. The startup flag (see WindowState) ties the
/// one-time resize-to-image to this session's first frame when this is
/// the process's startup open.
fn request_open(hwnd: HWND, path: &OsStr, origin: OpenOrigin<'_>) {
    // Existence check BEFORE queueing a decode (upstream
    // `_viv_open_from_filename`'s GetFileAttributesEx arm, viv.c:1359 —
    // the status bar's "File not found." is a pre-open verdict, not a
    // decode failure, viv.c:5094-5098). The byte size and mtime ride along
    // for the status bar and the navigation reference; directories and
    // unreadable files fall through to the loader as user-level failures
    // like upstream.
    let (not_found, file_bytes, modified) = match std::fs::metadata(Path::new(path)) {
        Ok(meta) => (
            false,
            Some(meta.len()),
            Some(playlist::modified_ticks(&meta)),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, None, None),
        Err(_) => (false, None, None),
    };
    if not_found {
        // A missing file never reaches the loader (no session, no
        // Loading) — the bar shows "File not found." over the kept display
        // until the next open (upstream sets `_viv_file_not_found` without
        // spawning a load, viv.c:5094-5098).
        // SAFETY: the borrow spans only flag stores — nothing pumps.
        if let Some(state) = unsafe { state_of(hwnd) } {
            if let OpenOrigin::Nav(entry) = origin {
                // Navigation opens the fd as-is — the reference follows it
                // even when the file has vanished since the scan (upstream
                // `_viv_open` sets current_fd without an existence check).
                state.nav_current = Some(entry.clone());
            }
            state.status_file_not_found = true;
            state.status_load_failed = false;
            // Supersede any in-flight load so its late replies are inert.
            state.session = None;
            // The not-opened startup verdict must not leak the resize to a
            // later user open.
            state.startup_open_pending = false;
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
        // The navigation reference follows the request (viv.c:1574-1579) —
        // the entry for a navigation, a fresh id-0 entry for a direct pick
        // (mtime 0 in the unstatable corner, where upstream would not have
        // opened at all and riviv's established #5 model proceeds to the
        // loader).
        state.nav_current = Some(match origin {
            OpenOrigin::Nav(entry) => entry.clone(),
            OpenOrigin::Direct => PlaylistEntry {
                path: path.to_os_string(),
                modified: modified.unwrap_or(0),
                id: 0,
            },
        });
        // The byte size is STAGED, not committed: a replacement load that
        // fails before its first frame keeps the old image on screen, and
        // the status bar must keep showing the OLD file's size with it
        // (cubic PR #13). Committed to `displayed_file_bytes` when this
        // session's first frame takes the display.
        state.pending_file_bytes = file_bytes;
        let session = state.load_thread.request(hwnd, path.to_os_string());
        if state.startup_open_pending {
            state.startup_resize_session = Some(session.id());
        }
        state.startup_open_pending = false;
        state.session = Some(session);
    }
    refresh_status(hwnd);
}

/// Upstream `_viv_open_from_filename` (viv.c:1359-1432) minus the cwd
/// combine — every caller passes an absolute path (drops and the dialog
/// natively; CLI arguments are absolutized at parse). Folders recurse into
/// the playlist and home; plain files open directly. ANY attributes
/// failure falls into the FindFirstFile arm (viv.c:1394-1428), which is
/// also the wildcard expander — note Win32 reports a `*`/`?` path as
/// ERROR_INVALID_NAME, not not-found, so gating on io::ErrorKind::NotFound
/// would miss it. Returns whether the path resolved to anything
/// (upstream's ret FALSE; callers turn it into the File-not-found status
/// or ignore it, like the drop path).
fn open_from_filename(hwnd: HWND, path: &OsStr) -> bool {
    let p = Path::new(path);
    match std::fs::metadata(p) {
        // "add subfolders and subsubfolders..." then home (viv.c:1380-1386).
        Ok(md) if md.is_dir() => {
            // SAFETY: the borrow spans the playlist FS scan — read_dir and
            // metadata never pump messages.
            if let Some(state) = unsafe { state_of(hwnd) } {
                playlist::add_path(&mut state.playlist, p);
            }
            home_open(hwnd, false);
            true
        }
        Ok(_) => {
            request_open(hwnd, path, OpenOrigin::Direct);
            true
        }
        // The FindFirstFile fallback: expands wildcards (files added
        // UNFILTERED, viv.c:1413-1417) and, for a plain missing path,
        // matches nothing — both end with an empty playlist arm deciding
        // the return. A matched-but-unreadable file lands in the loader as
        // a user-level failure from there (riviv's established model).
        Err(_) => {
            // SAFETY: the borrow spans the expansion + unfiltered adds.
            let found = (unsafe { state_of(hwnd) })
                .map(|state| playlist::add_expanded(&mut state.playlist, p))
                .unwrap_or(false);
            if found {
                home_open(hwnd, false);
            }
            found
        }
    }
}

/// The folder-scan entry set (upstream `_viv_next`/`_viv_home`'s
/// FindFirstFile arm, viv.c:5999-6076/6184-6238): the valid images of ONE
/// directory (not recursive), every entry id 0 — the scan arm has no node
/// identity, which is why `next` must be called with `from_playlist`
/// false on these.
fn scan_entries(dir: &Path) -> Vec<PlaylistEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new(); // INVALID_HANDLE_VALUE: an empty scan
    };
    let mut entries = Vec::new();
    for entry in read.flatten() {
        // Find-data attributes and mtime, no extra syscall (see add_path).
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            continue; // is_valid_filename's directory gate
        }
        let path = entry.path();
        if playlist::is_valid_path(path.as_os_str()) {
            entries.push(PlaylistEntry {
                path: path.into_os_string(),
                modified: playlist::modified_ticks(&metadata),
                id: 0,
            });
        }
    }
    entries
}

/// The directory a folder-scan navigates: the current file's parent, or
/// the process working directory with no current (upstream
/// `string_get_path_part` / GetCurrentDirectory, viv.c:5999/6189-6196).
fn scan_dir(hwnd: HWND) -> std::path::PathBuf {
    // SAFETY: the borrow ends at the end of this statement (the path is
    // cloned out); nothing below pumps.
    let current =
        (unsafe { state_of(hwnd) }).and_then(|s| s.nav_current.as_ref().map(|e| e.path.clone()));
    match current {
        Some(path) => Path::new(&path)
            .parent()
            .map(|d| d.to_path_buf())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default(),
        None => std::env::current_dir().unwrap_or_default(),
    }
}

/// Home/End (upstream `_viv_home`, viv.c:6120-6263): over the playlist,
/// the sort extreme (current included — re-opening it is allowed); with no
/// playlist, the folder scan of `scan_dir` — and a scan that finds
/// NOTHING blanks the display (`_viv_blank`, viv.c:6245-6253; unreachable
/// while the playlist is non-empty, its first node always qualifies).
fn home_open(hwnd: HWND, end: bool) {
    // SAFETY: the borrow ends at the end of this statement (the entry is
    // cloned out); nothing below pumps.
    let playlist_target = (unsafe { state_of(hwnd) }).and_then(|s| {
        if s.playlist.is_empty() {
            None
        } else {
            playlist::home(s.playlist.entries(), end).cloned()
        }
    });
    if let Some(entry) = playlist_target {
        request_open(hwnd, &entry.path, OpenOrigin::Nav(&entry));
        return;
    }
    let entries = scan_entries(&scan_dir(hwnd));
    match playlist::home(&entries, end) {
        Some(entry) => request_open(hwnd, &entry.path, OpenOrigin::Nav(entry)),
        None => blank_display(hwnd),
    }
}

/// Next/Prev for the navigation keys (upstream `_viv_next`,
/// viv.c:5817-6118): playlist arm when a playlist exists (node exclusion
/// by id), folder-scan arm over the current file's parent otherwise, and
/// no current at all becomes home(0) — for prev too (viv.c:6101-6104).
/// next/prev NEVER blanks: no candidate is a no-op (viv.c:6093-6099).
fn nav_next(hwnd: HWND, prev: bool) {
    enum Mode {
        Home,
        Playlist,
        Scan,
    }
    // SAFETY: the borrow ends at the end of this statement (only the mode
    // is taken out); nothing below pumps.
    let mode = match unsafe { state_of(hwnd) } {
        Some(state) => match state.nav_current.as_ref() {
            None => Mode::Home,
            Some(_) if state.playlist.is_empty() => Mode::Scan,
            Some(_) => Mode::Playlist,
        },
        None => return,
    };
    match mode {
        Mode::Home => home_open(hwnd, false),
        Mode::Playlist => {
            // SAFETY: the borrow ends at the end of this statement (the
            // entry is cloned out).
            let target = (unsafe { state_of(hwnd) }).and_then(|s| {
                playlist::next(s.playlist.entries(), s.nav_current.as_ref(), prev, true).cloned()
            });
            if let Some(entry) = target {
                request_open(hwnd, &entry.path, OpenOrigin::Nav(&entry));
            }
        }
        Mode::Scan => {
            let entries = scan_entries(&scan_dir(hwnd));
            // SAFETY: the borrow ends at the end of this statement (the
            // entry is cloned out).
            let current = (unsafe { state_of(hwnd) }).and_then(|s| s.nav_current.clone());
            if let Some(entry) = playlist::next(&entries, current.as_ref(), prev, false) {
                request_open(hwnd, &entry.path, OpenOrigin::Nav(entry));
            }
        }
    }
}

/// Upstream `_viv_blank` (viv.c:7908-7930): clear the display, the
/// navigation reference AND the playlist; the title falls back to the app
/// name. Any in-flight load is superseded (upstream `_viv_clear` stops the
/// load thread). The failure flags are NOT reset — upstream's
/// `_viv_clear`/`_viv_blank` leave `_viv_file_not_found`/`_viv_load_failed`
/// alone (viv.c:1268-1293), so a stale verdict survives the blank until the
/// next open resets it (viv.c:1447-1458). An unresolved startup open also
/// dies here: it must not leak the startup resize into a later user open.
fn blank_display(hwnd: HWND) {
    let stop_timer;
    {
        // SAFETY: the borrow spans only plain field stores — nothing pumps.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        state.image = None;
        state.displayed_from = None;
        state.path = None;
        state.playlist.clear();
        state.nav_current = None;
        state.displayed_file_bytes = None;
        state.pending_file_bytes = None;
        state.session = None;
        state.startup_open_pending = false;
        // The zoom/pan view dies with the display (upstream `_viv_blank` →
        // `_viv_clear`, viv.c:7910 + 1282-1288).
        state.view.reset();
        stop_timer = state.animation_timer_running;
        state.animation_timer_running = false;
    }
    refresh_status(hwnd);
    // A blanked display can never hide the cursor — reconcile it (upstream
    // `_viv_blank` → `_viv_start_first_frame` → `_viv_update_show_cursor`,
    // viv.c:7928 + 14338).
    update_cursor(hwnd);
    if stop_timer {
        // SAFETY: hwnd is live; a failed kill leaves a stale timer that the
        // WM_TIMER guard no-ops on.
        let _ = unsafe { KillTimer(Some(hwnd), ANIMATION_TIMER_ID) };
    }
    // SAFETY: hwnd is live; the HSTRING outlives the call. Fail-soft like
    // every other title update (upstream viv.c:1249 ignores it too).
    let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from_wide(&title_wide(None))) };
    // SAFETY: queues a WM_PAINT; never pumps messages.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

/// Whether an auto-repeated navigation key must wait for the in-flight
/// load (upstream viv.c:5846-5853: load thread active AND still before
/// the streaming phase — its second disjunct is a terminate already
/// pending, which in riviv is the atomic supersede itself and has no
/// observable window). The first press is never blocked: it supersedes,
/// exactly like upstream's terminate-and-chain.
fn nav_repeat_waits_for_load(state: &WindowState) -> bool {
    state.session.as_ref().is_some_and(|session| {
        !(state.displayed_from == Some(session.id())
            && state
                .image
                .as_ref()
                .is_some_and(|i| i.is_animated() && i.frame_count() >= 2))
    })
}

/// The command line's not-found verdict (upstream viv.c:5094-5098): the
/// bar shows "File not found." over the blank window; nothing was queued,
/// nothing loads, and the startup resize must not leak to a later open.
fn mark_startup_not_found(hwnd: HWND) {
    // SAFETY: the borrow spans only flag stores — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.status_file_not_found = true;
        state.status_load_failed = false;
        state.startup_open_pending = false;
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
                        // cleared it): the zoom/pan view resets with it
                        // (upstream `_viv_clear` runs at exactly these
                        // points, viv.c:2804/2835/7910) and the status
                        // bar's "(N KB)" follows the same commit/clear.
                        state.view.reset();
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
            // (M1 behavior under async startup) — but NOT while fullscreen:
            // this resize is riviv's own deviation (upstream never resizes
            // on load), and shrinking the borderless monitor cover to an
            // image-sized rect would visibly break it (cubic PR #16 P1).
            // Skipping is safe: the pending flag is consumed either way.
            if !state.fullscreen {
                // A geometry failure is fatal exactly like run()'s initial
                // rect.
                match initial_window_rect(state.image.as_ref(), status::height(state.status)) {
                    Ok(rect) => resize_rect = Some(rect),
                    Err(msg) => fatal_msg = Some(msg),
                }
            }
        }
    }
    // Borrow dropped — the modal paths and the reentrant resize are safe.
    refresh_status(hwnd);
    // The display may have adopted an image (the hide-cursor conditions
    // just became satisfiable) — reconcile (upstream
    // `_viv_start_first_frame` → `_viv_update_show_cursor`, viv.c:14338).
    update_cursor(hwnd);
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

fn on_keydown(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    // Upstream default keymap (viv.c:970-1049): Ctrl+O = open file and
    // Ctrl+Shift+O = add file (viv.c:975); the navigation keys are
    // UNMODIFIED Right/PgDn (next), Left/PgUp (prev), Home/End
    // (viv.c:1040-1045). Up/Down are slideshow rate, not navigation — they
    // stay unwired until the slideshow work.
    if wparam.0 == usize::from(b'O') {
        // SAFETY: GetKeyState reads thread-local async key state; the VKs are valid.
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        // Upstream matches the full modifier mask (viv.c:6396-6402) —
        // Ctrl+Alt+O is NOT Open File.
        // SAFETY: GetKeyState reads thread-local async key state; VK_MENU is valid.
        let alt = unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0;
        if ctrl && !alt {
            // SAFETY: GetKeyState reads thread-local async key state; VK_SHIFT is valid.
            let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
            // SAFETY: the borrow ends at the end of this statement (the path is
            // cloned out); the modal dialog below pumps messages but no borrow is
            // live by then.
            let initial_dir = (unsafe { state_of(hwnd) })
                .and_then(|s| s.path.clone())
                .and_then(|p| Path::new(&p).parent().map(|d| d.as_os_str().to_os_string()));
            if let Some(path) = open_file_dialog(hwnd, initial_dir.as_deref()) {
                // SAFETY: the borrow spans only the playlist mutation —
                // nothing pumps.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    if shift {
                        // Add File appends (viv.c:2396-2402): the current
                        // file becomes the first entry when the list is
                        // empty, then the pick — no clear, no home, the
                        // display stays.
                        if state.playlist.is_empty()
                            && let Some(current) = state.nav_current.as_ref()
                        {
                            let current = current.clone();
                            state.playlist.add(current.path, current.modified);
                        }
                        playlist::add_filename(&mut state.playlist, Path::new(&path));
                    } else {
                        // Open File clears the playlist before opening
                        // (viv.c:2390-2394) — the picked file starts fresh,
                        // and navigation falls back to its folder.
                        state.playlist.clear();
                    }
                }
                if !shift {
                    let _ = open_from_filename(hwnd, &path);
                }
            }
            return;
        }
        return;
    }
    // The zoom keys bind with upstream's exact modifier masks (viv.c:1017-
    // 1024): '+'/'=' and numpad '+' zoom in, '-'/numpad '-' out — Ctrl
    // accelerates ONLY the numpad variants; Ctrl+'0' resets to fit;
    // Ctrl+Alt+'0' toggles temporary 1:1. Key repeat intentionally steps
    // repeatedly (upstream has no repeat gating on zoom commands).
    // SAFETY: GetKeyState reads thread-local async key state; the VKs are valid.
    let (ctrl, shift, alt) = unsafe {
        (
            GetKeyState(i32::from(VK_CONTROL.0)) < 0,
            GetKeyState(i32::from(VK_SHIFT.0)) < 0,
            GetKeyState(i32::from(VK_MENU.0)) < 0,
        )
    };
    let vk = wparam.0 as u16;
    // ESC cancels an in-progress drag, or leaves fullscreen (upstream
    // viv.c:6367-6382: an unmodified ESC with a mouse action active
    // releases the capture FIRST; only otherwise does it exit fullscreen —
    // the slideshow pause arm of that block lands with the slideshow
    // work).
    if vk == VK_ESCAPE.0 && !ctrl && !shift && !alt {
        // SAFETY: the borrow spans only the Option take.
        let was_dragging =
            (unsafe { state_of(hwnd) }).is_some_and(|state| state.drag.take().is_some());
        if was_dragging {
            // SAFETY: the capture was taken on this thread in
            // WM_LBUTTONDOWN.
            let _ = unsafe { ReleaseCapture() };
            return;
        }
        // SAFETY: the read-only borrow ends inside is_some_and.
        let fullscreen = (unsafe { state_of(hwnd) }).is_some_and(|state| state.fullscreen);
        if fullscreen {
            toggle_fullscreen(hwnd);
        }
        return;
    }
    // Alt+Return toggles fullscreen (upstream default keymap, viv.c:995 —
    // an Alt-modified key, so it arrives as WM_SYSKEYDOWN; the exact-mask
    // rule means Ctrl+Alt+Return is NOT the toggle).
    if vk == VK_RETURN.0 && alt && !ctrl && !shift {
        toggle_fullscreen(hwnd);
        return;
    }
    if (vk == VK_OEM_PLUS.0 && !ctrl && !shift && !alt) || (vk == VK_ADD.0 && !shift && !alt) {
        zoom_step_centered(hwnd, false);
        return;
    }
    if (vk == VK_OEM_MINUS.0 && !ctrl && !shift && !alt) || (vk == VK_SUBTRACT.0 && !shift && !alt)
    {
        zoom_step_centered(hwnd, true);
        return;
    }
    if vk == u16::from(b'0') && ctrl && !shift {
        if alt {
            toggle_one_to_one(hwnd);
        } else {
            zoom_reset(hwnd);
        }
        return;
    }
    // The navigation keys bind with no modifiers at all — Ctrl+Left etc.
    // are the animation frame-step commands (M2 later), so any held
    // ctrl/shift/alt disqualifies the key.
    if ctrl || shift || alt {
        return;
    }
    // Auto-repeat (lParam bit 30, upstream viv.c:6403): a repeated
    // next/prev waits for the in-flight load instead of stacking opens.
    let is_repeat = (lparam.0 & 0x4000_0000) != 0;
    let repeat_waits = || {
        // SAFETY: the read-only borrow ends inside is_some_and.
        (unsafe { state_of(hwnd) }).is_some_and(|s| nav_repeat_waits_for_load(s))
    };
    if vk == VK_RIGHT.0 || vk == VK_NEXT.0 {
        if is_repeat && repeat_waits() {
            return;
        }
        nav_next(hwnd, false);
    } else if vk == VK_LEFT.0 || vk == VK_PRIOR.0 {
        if is_repeat && repeat_waits() {
            return;
        }
        nav_next(hwnd, true);
    } else if vk == VK_HOME.0 {
        home_open(hwnd, false);
    } else if vk == VK_END.0 {
        home_open(hwnd, true);
    }
}

/// WM_DROPFILES (upstream viv.c:3076-3128).
fn on_drop_files(hwnd: HWND, hdrop: HDROP) {
    // SAFETY: `hdrop` is owned by this message; DragFinish is called exactly
    // once on every path below, and nothing here pumps messages (the FS
    // scans and metadata reads inside the playlist helpers cannot).
    unsafe {
        // Upstream branches on shift BEFORE anything else: shift means
        // append (`add_current_if_empty`, viv.c:3090-3094 — the current
        // file becomes the first playlist entry with a FRESH id when the
        // list is empty), no shift means replace (`clearall` runs even for
        // a single dropped file, viv.c:3095-3098).
        let is_shift = GetKeyState(i32::from(VK_SHIFT.0)) < 0;
        // SAFETY: the borrow spans only the playlist mutation.
        if let Some(state) = state_of(hwnd) {
            if is_shift && state.playlist.is_empty() {
                if let Some(current) = state.nav_current.as_ref() {
                    let current = current.clone();
                    state.playlist.add(current.path, current.modified);
                }
            } else if !is_shift {
                state.playlist.clear();
            }
        }
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        // A single unshifted drop keeps the M1 replace semantics — but the
        // playlist was still cleared above, exactly like upstream; a
        // dropped FOLDER still builds its playlist through
        // `open_from_filename` (viv.c:3118-3124).
        if count >= 2 || is_shift {
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                if len == 0 || len >= 32768 {
                    continue;
                }
                let mut buf = vec![0u16; len + 1];
                if DragQueryFileW(hdrop, i, Some(&mut buf)) as usize == len {
                    let path = OsString::from_wide(&buf[..len]);
                    // SAFETY: the borrow spans the add's metadata read.
                    if let Some(state) = state_of(hwnd) {
                        playlist::add_filename(&mut state.playlist, Path::new(&path));
                    }
                }
            }
            // Only the replace flavor homes (viv.c:3113-3116); a shift-append
            // leaves the current image up.
            if !is_shift {
                home_open(hwnd, false);
            }
        } else if count == 1 {
            let len = DragQueryFileW(hdrop, 0, None) as usize;
            if len > 0 && len < 32768 {
                let mut buf = vec![0u16; len + 1];
                if DragQueryFileW(hdrop, 0, Some(&mut buf)) as usize == len {
                    let path = OsString::from_wide(&buf[..len]);
                    let _ = open_from_filename(hwnd, &path);
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
    // WM_SIZE (upstream `_viv_on_size`, viv.c:1615-1620). No bar (creation
    // failed / destroyed for fullscreen): nothing to dock — height()
    // already reported 0 to the viewport math.
    if !bar.is_invalid() {
        // SAFETY: bar is our live child window.
        unsafe {
            SendMessageW(bar, WM_SIZE, None, None);
        }
    }
    // Re-anchor the pan offset for the new viewport (upstream WM_SIZE,
    // viv.c:1643-1651: reproject the center-source anchor onto the new
    // render size and re-clamp). CS_HREDRAW/CS_VREDRAW already repaint
    // resizes; the invalidate only matters for a re-clamped offset.
    // SAFETY: the borrow spans the pure re-anchor math.
    let reclamped = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let (vp, src) = viewport_and_src(hwnd, state);
        state.view.on_resize(src.0, src.1, vp)
    });
    if reclamped {
        repaint(hwnd);
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
            // Give the cursor back before anything else (upstream
            // `_viv_kill`'s first call, viv.c:5459-5461) — the process is
            // exiting and must not leave the display count decremented.
            show_cursor(hwnd);
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
            on_keydown(hwnd, wparam, lparam);
            LRESULT(0)
        }
        // Alt-modified keys arrive as WM_SYSKEYDOWN, not WM_KEYDOWN —
        // upstream dispatches both through the same keymap (viv.c:6347-6348),
        // which is what makes Ctrl+Alt+0 (temporary 1:1) reachable from a
        // real keyboard. Always fall through to DefWindowProc afterwards:
        // it owns Alt+F4 / Alt+Space / F10 even for keys on_keydown used.
        WM_SYSKEYDOWN => {
            on_keydown(hwnd, wparam, lparam);
            // SAFETY: hwnd/msg are exactly what this callback received; the
            // default procedure handles everything we do not.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_MOUSEWHEEL => {
            on_mousewheel(hwnd, wparam, lparam);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            on_left_button_down(hwnd, lparam);
            LRESULT(0)
        }
        // CS_DBLCLKS folds the second press of a double click into this
        // message (after DOWN/UP have already run — see on_double_click).
        WM_LBUTTONDBLCLK => {
            on_double_click(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            on_mouse_move(hwnd, lparam);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            on_mouse_leave(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            on_left_button_up(hwnd);
            LRESULT(0)
        }
        // Upstream viv.c:3547-3555 — the deactivate arm compares the FULL
        // wParam against WA_INACTIVE (0); the guard flag swallows the
        // dummy dance's momentary deactivate so a hidden cursor stays
        // hidden through it.
        WM_ACTIVATE => {
            if wparam.0 == 0 {
                // SAFETY: the read-only borrow ends inside is_some_and.
                let show = (unsafe { state_of(hwnd) }).is_some_and(|s| !s.prevent_deactivate_show);
                if show {
                    show_cursor(hwnd);
                }
            } else {
                update_cursor(hwnd);
            }
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
            } else if wparam.0 == cursor::HIDE_CURSOR_TIMER_ID {
                on_hide_cursor_timer(hwnd);
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

pub(crate) fn run(args: Vec<OsString>) -> Result<(), String> {
    // The animation clock's unit, read once (constant for the process
    // lifetime). Read before any window exists: failure is fatal (ADR 0001).
    let timer_freq = qpc_frequency()?;
    // Register the common-control classes before any window exists — the
    // status bar's `msctls_statusbar32` is only guaranteed registered after
    // this (upstream init does the same, viv.c:5236-5242). Fail-soft like
    // upstream, which ignores the BOOL return: without the classes the
    // status bar degrades away (creation failure handled below), while the
    // viewer itself keeps working.
    let icex = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES | ICC_BAR_CLASSES | ICC_WIN95_CLASSES,
    };
    // SAFETY: `icex` outlives the call; a pure registration query.
    let _ = unsafe { InitCommonControlsEx(&icex) };
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
        playlist: Playlist::new(),
        nav_current: None,
        // The startup resize belongs to the CLI's open only; no file
        // arguments (or none that resolve) leave the default-size window.
        startup_open_pending: !args.is_empty(),
        view: View::new(),
        drag: None,
        fullscreen: false,
        fullscreen_was_maxed: false,
        fullscreen_restore_rect: RECT::default(),
        fullscreen_zoom_offset: 0,
        cursor: CursorVisibility::new(),
        tracking_mouse: false,
        is_mouseover: false,
        last_cursor_pt: POINT { x: -1, y: -1 },
        prevent_deactivate_show: false,
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

    // The status bar does not exist yet (created below, after the main
    // window) — its height is the standard common-control height at the
    // system DPI (upstream reads the live window; the formula matches
    // comctl32's own: border + 3/2 of the system status font's line
    // height).
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

    // Create the status bar child now that the parent window exists (#5;
    // upstream `_viv_status_show(config_show_status)` at init, viv.c:5415).
    // Creation failure degrades gracefully like upstream — its `_viv_status_hwnd`
    // stays NULL and `_viv_status_update` no-ops — the viewer must keep
    // working; the handle stays invalid and every status call guards on it.
    let bar = match status::create(hwnd, hinstance.into()) {
        Ok(bar) => bar,
        Err(msg) => {
            eprintln!("status bar unavailable: {msg}");
            HWND::default()
        }
    };
    // SAFETY: the borrow spans only the field store; the window is created
    // and owned by this thread, nothing below pumps messages.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.status = bar;
    }

    // The command line's file arguments (upstream viv.c:4990-5100; main.rs
    // has already skipped switch-shaped words and absolutized the rest):
    // ONE argument keeps single-file semantics (a folder recurses into a
    // playlist, a wildcard expands, a file opens directly); a SECOND
    // argument pulls the first into the playlist too — everything added in
    // argument order — and the first-inserted entry is what opens. When
    // nothing resolves, the startup "File not found." verdict shows over
    // the blank window (upstream viv.c:5090-5098).
    if !args.is_empty() {
        if args.len() == 1 {
            if !open_from_filename(hwnd, &args[0]) {
                mark_startup_not_found(hwnd);
            }
        } else {
            for arg in &args {
                // SAFETY: the borrow spans the add's metadata reads.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    playlist::add_filename(&mut state.playlist, Path::new(arg));
                }
            }
            // SAFETY: the borrow ends at the end of this statement (the
            // entry is cloned out).
            let first = (unsafe { state_of(hwnd) }).and_then(|s| s.playlist.first().cloned());
            let resolved = first.is_some_and(|entry| open_from_filename(hwnd, &entry.path));
            if !resolved {
                mark_startup_not_found(hwnd);
            }
        }
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
