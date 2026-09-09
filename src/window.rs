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
//! in fullscreen, reappearing on movement (`cursor.rs`). Settings (#19):
//! the window opens at the remembered rect (60% auto-fit on first run)
//! and the windowed position is tracked into the config on every
//! WM_SIZE/WM_MOVE, saved to the ini in WM_DESTROY (`config.rs`).
//! Single instance (#21): a second launch hands its command line to the
//! RIVIV-mutex owner via WM_COPYDATA and exits (`copydata.rs`); the
//! receiver adopts its cwd, re-runs the command-line open path, and
//! appends instead of replacing when the handoffs arrive in a burst.
//! Menu bar (#23): the command table in `menu.rs` walks into a real HMENU
//! at startup (attached per `config_show_menu`, detached/attached with
//! client-anchored frame compensation on the View→Menu toggle), WM_COMMAND
//! dispatches onto the same action functions as the keyboard, and
//! WM_INITMENU refreshes the check marks.

use std::ffi::{OsStr, OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HLOCAL, HWND, LPARAM, LRESULT,
    LocalFree, POINT, RECT, SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, GetMonitorInfoW, HBRUSH, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, MonitorFromRect, MonitorFromWindow, PtInRect, ScreenToClient, UpdateWindow,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, IBindCtx,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Environment::{
    GetCommandLineW, GetCurrentDirectoryW, SetCurrentDirectoryW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    CreateMutexA, GetCurrentThreadId, GetStartupInfoW, STARTF_USESHOWWINDOW, STARTUPINFOW,
};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyNameTextW, GetKeyState, GetKeyboardLayout, MAPVK_VK_TO_VSC, MapVirtualKeyExW,
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ADD, VK_CONTROL,
    VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_OEM_MINUS, VK_OEM_PLUS,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SUBTRACT,
};
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, DragFinish, DragQueryFileW, FILEOPENDIALOGOPTIONS, FOS_NOCHANGEDIR,
    FOS_PICKFOLDERS, FileOpenDialog, HDROP, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, AppendMenuW, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW,
    CheckMenuItem, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, EnableMenuItem, FindWindowA, GWL_STYLE, GWLP_USERDATA,
    GetClientRect, GetCursorPos, GetForegroundWindow, GetMenu, GetMessageW, GetWindowLongPtrW,
    GetWindowRect, HMENU, HWND_TOP, IDC_ARROW, IsIconic, IsZoomed, KillTimer, LoadCursorW,
    MB_ICONERROR, MB_OK, MENU_ITEM_FLAGS, MF_BYCOMMAND, MF_CHECKED, MF_ENABLED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MINMAXINFO, MSG, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SHOW_WINDOW_CMD, SW_MAXIMIZE, SW_RESTORE, SW_SHOW,
    SW_SHOWNORMAL, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetMenu, SetProcessDPIAware, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowCursor, ShowWindow, TPM_CENTERALIGN, TPM_LEFTBUTTON, TPM_VCENTERALIGN,
    TrackPopupMenu, TranslateMessage, USER_TIMER_MINIMUM, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_ACTIVATE, WM_COMMAND, WM_CONTEXTMENU, WM_COPYDATA, WM_DESTROY, WM_DROPFILES, WM_ENDSESSION,
    WM_ERASEBKGND, WM_GETMINMAXINFO, WM_INITMENU, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_MOVE, WM_NCCREATE, WM_NCDESTROY, WM_NULL,
    WM_PAINT, WM_QUERYENDSESSION, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE,
    WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_CAPTION, WS_EX_ACCEPTFILES, WS_OVERLAPPEDWINDOW,
    WS_POPUP, WS_THICKFRAME, WS_VISIBLE, WindowFromPoint,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

use crate::anim::ANIMATION_TIMER_ID;
use crate::config::Config;
use crate::copydata;
use crate::cursor::{self, CursorVisibility};
use crate::loader::{LoadedImage, UiAction, apply_reply, map_reply_frame};
use crate::loadthread::{LoadSession, LoadThread, REPLY_KICK_MESSAGE};
use crate::loc;
use crate::menu;
use crate::paint::paint;
use crate::playlist::{self, Playlist, PlaylistEntry};
use crate::status;
use crate::surface::Surface;
use crate::text::{dialog_filter, title_wide, to_wide};
use crate::zoom::{FitPolicy, View, Viewport};

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
    /// The persisted settings (#19): loaded before the window exists,
    /// updated by the position tracking in WM_SIZE/WM_MOVE, saved in
    /// WM_DESTROY on the way out.
    pub(crate) config: Config,
    /// Whether the animation timer is currently running — edge bookkeeping
    /// so timer reconciliation never resets a live timer's period (which
    /// would starve WM_TIMER under fast frame streams).
    pub(crate) animation_timer_running: bool,
    /// The status-bar child window (#5; upstream `_viv_status_hwnd`).
    /// Created in WM_NCCREATE, destroyed with the parent by Windows.
    pub(crate) status: HWND,
    /// The menu bar (#23; upstream `_viv_hmenu`, viv.c:718/5352) — built
    /// once in `run` before the window exists, attached at creation when
    /// `config_show_menu` is set, and re-attached/detached by the
    /// View→Menu toggle. `HMENU::default()` = creation failed (every menu
    /// call guards on it; the viewer keeps working, like a missing status
    /// bar).
    pub(crate) menu: HMENU,
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
    /// When the previous command line was processed (upstream
    /// `last_process_command_line_tick` + `got_last_process_command_line_tick`,
    /// viv.c:792-793): a handoff arriving within
    /// `config.add_command_line_timeout` of this stamp appends to the
    /// playlist instead of replacing it (Explorer's multi-select launches
    /// forward their command lines in a burst, viv.c:4778-4793). `None` =
    /// no command line processed yet.
    pub(crate) last_cl_tick: Option<u32>,
    /// The last folder the Open Folder dialog picked (upstream
    /// `_viv_last_open_folder`, viv.c:2426-2431 — the dialog reopens there
    /// instead of deriving from the current image, which may sit in a
    /// subfolder of a recursive scan). `None` = never picked; the dialog
    /// then falls back to the current image's parent like riviv's Ctrl+O.
    pub(crate) last_open_folder: Option<OsString>,
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
        frame_remaining: state.config.frame_minus != 0,
        dimensions: state.image.as_ref().map(|i| (i.width(), i.height())),
        file_bytes: state.displayed_file_bytes,
        client_wide: client.right - client.left,
    }
}

/// Refresh the status bar from current state, running the Win32 calls
/// outside any state borrow (SB_SETTEXT redraws synchronously).
pub(crate) fn refresh_status(hwnd: HWND) {
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
pub(crate) fn viewport_and_src(hwnd: HWND, state: &WindowState) -> (Viewport, (i32, i32)) {
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

/// The fit inputs of `_viv_get_render_size` resolved for the CURRENT mode
/// (viv.c:6885-6891): the fill half switches with fullscreen —
/// `fullscreen_fill_window` while fullscreen, `fill_window` while windowed
/// — while keep-aspect is mode-independent. Read live at every size
/// consumer, like upstream's globals.
pub(crate) fn fit_policy(state: &WindowState) -> FitPolicy {
    FitPolicy {
        keep_aspect: state.config.keep_aspect_ratio != 0,
        fill: if state.fullscreen {
            state.config.fullscreen_fill_window != 0
        } else {
            state.config.fill_window != 0
        },
    }
}

/// Gather `_viv_should_show_cursor`'s live inputs (viv.c:14593-14619): a
/// viewable image is up, we are foreground, the mouse is over us, nothing
/// holds the capture, and (fullscreen OR the `windowed_hide_cursor`
/// config, default 1 — #24 wired it to the ini).
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
        hide_when_windowed: state.config.windowed_hide_cursor != 0,
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

/// WM_LBUTTONDBLCLK (upstream viv.c:3298-3327): the scroll-family actions
/// double-click into the fullscreen toggle; the one-shot actions (zoom in,
/// next) repeat themselves instead. The first click of the pair started a
/// drag, but the intervening WM_LBUTTONUP already ended it — the drag is
/// over by the time the DBLCLK arrives. The cursor reappears first, like
/// on every button message.
fn on_double_click(hwnd: HWND, lparam: LPARAM) {
    show_and_update_cursor(hwnd);
    let pt = lparam_point(lparam);
    // SAFETY: the borrow spans only the config read.
    let action = (unsafe { state_of(hwnd) })
        .map(|s| s.config.left_click_action)
        .unwrap_or(0);
    match action {
        // 3 = zoom in: repeat at the click point (upstream's default arm
        // re-runs `_viv_do_left_click_action`).
        3 => zoom_at(hwnd, false, (pt.x, pt.y)),
        // 4 = next image: the second click advances again.
        4 => nav_next(hwnd, false),
        // 0/1/2/5/6 (scroll, slideshow, animation, 1:1 scroll, move
        // window): the double-click toggles fullscreen; unknown values
        // re-run the action, which does nothing (upstream viv.c:3313-3326).
        0 | 1 | 2 | 5 | 6 => toggle_fullscreen(hwnd),
        _ => {}
    }
}

/// WM_RBUTTONDOWN / WM_RBUTTONUP (upstream viv.c:3349-3367): actions 1
/// (zoom out at the click) and 2 (previous image) fire on button-DOWN and
/// swallow both messages so no context menu follows; anything else falls
/// to DefWindowProc (whose RBUTTONUP handling produces WM_CONTEXTMENU).
fn on_right_button(hwnd: HWND, msg: u32, lparam: LPARAM) -> bool {
    // SAFETY: the borrow spans only the config read.
    let action = (unsafe { state_of(hwnd) })
        .map(|s| s.config.right_click_action)
        .unwrap_or(0);
    // Actions fire on press; under CS_DBLCLKS the second press of a rapid
    // pair arrives as WM_RBUTTONDBLCLK — upstream runs the same arm on
    // both (viv.c:3349-3367), so a fast right double-click performs the
    // configured action TWICE, exactly like upstream.
    let press = msg == WM_RBUTTONDOWN || msg == WM_RBUTTONDBLCLK;
    match action {
        1 => {
            if press {
                let pt = lparam_point(lparam);
                zoom_at(hwnd, true, (pt.x, pt.y));
            }
            true
        }
        2 => {
            if press {
                nav_next(hwnd, true);
            }
            true
        }
        _ => false,
    }
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
        let fit = fit_policy(state);
        let old_render = state.view.render_size(src.0, src.1, vp, fit);
        let sizes = state.view.sizes_all_levels(src.0, src.1, vp, fit);
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
        // The menu bar comes back first, per config (upstream viv.c:6635-
        // 6643: SetMenu(_viv_hmenu) when config_show_menu else SetMenu(0) —
        // the enter path below took it off the borderless cover).
        // SAFETY: the borrow spans only the two reads; SetMenu
        // synchronously re-enters wnd_proc, so it runs outside.
        let (show_menu, menu) = (unsafe { state_of(hwnd) })
            .map_or((false, HMENU::default()), |state| {
                (state.config.show_menu != 0, state.menu)
            });
        // SAFETY: `menu` is the state's own bar; None detaches. The bool
        // return is ignored like upstream's unchecked SetMenu.
        let _ = unsafe {
            SetMenu(
                hwnd,
                if show_menu && !menu.is_invalid() {
                    Some(menu)
                } else {
                    None
                },
            )
        };
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
        // The menu bar leaves with the status bar (upstream viv.c:6679:
        // SetMenu(hwnd, 0) before the monitor cover — a borderless cover
        // with a menu bar would donate its strip to non-image UI).
        // SAFETY: detaching our own bar; the bool return is ignored like
        // upstream's unchecked SetMenu.
        let _ = unsafe { SetMenu(hwnd, None) };
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
        // fullscreen viewport with 1:1 already cleared — a fresh sweep now
        // (its fit resolves in the POST-toggle mode, exactly upstream's
        // live `_viv_get_render_size` read), while the fullscreen_fill
        // branch keeps the pre-toggle sweep from the gather above. Both
        // flags flow in from the config (#24): the upstream default
        // `fullscreen_fill_window=1` drops to the largest level that still
        // covers the monitor on entry, exit adds it back.
        // SAFETY: the borrow spans the fullscreen-viewport sweep and the
        // pure offset math; nothing here pumps messages.
        let offset = (unsafe { state_of(hwnd) })
            .map(|state| {
                let (fs_vp, _) = viewport_and_src(hwnd, state);
                let fit_fs = fit_policy(state);
                let sizes_fs = state.view.sizes_all_levels(src.0, src.1, fs_vp, fit_fs);
                crate::zoom::fullscreen_zoom_offset(
                    state.config.fullscreen_fill_window != 0,
                    state.config.fill_window != 0,
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
    // The action pick (upstream viv.c:3673-3677): Ctrl held selects
    // `ctrl_mouse_wheel_action`, else `mouse_wheel_action`.
    // SAFETY: GetKeyState reads this thread's key state.
    let ctrl = (unsafe { GetKeyState(VK_CONTROL.0 as i32) } as u16) & 0x8000 != 0;
    // SAFETY: the borrow spans only the action read.
    let action = (unsafe { state_of(hwnd) })
        .map(|s| {
            if ctrl {
                s.config.ctrl_mouse_wheel_action
            } else {
                s.config.mouse_wheel_action
            }
        })
        .unwrap_or(0);
    match action {
        // Action 1: wheel up = previous, down = next (viv.c:14063-14074).
        1 => {
            if delta > 0 {
                nav_next(hwnd, true);
            } else if delta < 0 {
                nav_next(hwnd, false);
            }
        }
        // Action 2: wheel up = next, down = previous (viv.c:14075-14086).
        2 => {
            if delta > 0 {
                nav_next(hwnd, false);
            } else if delta < 0 {
                nav_next(hwnd, true);
            }
        }
        // Action 0 (and any hand-edited unknown value, which upstream's
        // fall-through also zooms): one zoom level per notch, anchored at
        // the cursor — upstream keys off the delta's sign only, so a zero
        // delta counts as zoom-out.
        _ => {
            // SAFETY: the borrow spans only the pure zoom math — nothing
            // pumps.
            let changed = (unsafe { state_of(hwnd) }).is_some_and(|state| {
                let (vp, src) = viewport_and_src(hwnd, state);
                let fit = fit_policy(state);
                state
                    .view
                    .zoom_step(delta <= 0, (pt.x, pt.y), src.0, src.1, vp, fit)
            });
            if changed {
                repaint(hwnd);
            }
        }
    }
}

/// One zoom step anchored at an explicit client point — the shared body of
/// the wheel, the +/- keys (via the viewport center) and the click-action
/// zooms (upstream `_viv_do_mousewheel_action` action 0 /
/// `_viv_zoom_in(0,1,x,y)`, viv.c:13932+ / 6383-6390).
fn zoom_at(hwnd: HWND, out: bool, cursor: (i32, i32)) {
    // SAFETY: the borrow spans only the pure zoom math.
    let changed = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let (vp, src) = viewport_and_src(hwnd, state);
        let fit = fit_policy(state);
        state.view.zoom_step(out, cursor, src.0, src.1, vp, fit)
    });
    if changed {
        repaint(hwnd);
    }
}

/// A `+`/`-` keypress zoom step — anchored at the viewport center (upstream
/// `_viv_zoom_in` with have_xy=0 feeds the client-area center through the
/// same wheel action, viv.c:11803-11821).
fn zoom_step_centered(hwnd: HWND, out: bool) {
    // SAFETY: the borrow spans only the center read.
    let center = (unsafe { state_of(hwnd) })
        .map(|state| {
            let (vp, _) = viewport_and_src(hwnd, state);
            (vp.wide / 2, vp.high / 2)
        })
        .unwrap_or((0, 0));
    zoom_at(hwnd, out, center);
}

/// The window-size-to-image command (upstream `VIV_ID_VIEW_WINDOW_SIZE_*`,
/// viv.c:2076-2160) — auto_zoom's payload (viv.c:14363-14383): leave
/// fullscreen and the maximized state, size the window around the
/// displayed image (`fit::window_size_client` holds the target math),
/// centered on the window's current center and nudged fully into the
/// monitor's work area. A second pass recenters if the window manager
/// clamped the requested size (upstream viv.c:2140-2160).
fn window_size_to_image(hwnd: HWND, kind: i32) {
    // SAFETY: the borrow spans only the config/image reads — nothing pumps.
    let gathered = (unsafe { state_of(hwnd) }).map(|state| {
        (
            state.fullscreen,
            state
                .image
                .as_ref()
                .map(|img| (img.width(), img.height()))
                .unwrap_or((0, 0)),
            (
                state.config.auto_fit_wide_mul,
                state.config.auto_fit_wide_div,
                state.config.auto_fit_high_mul,
                state.config.auto_fit_high_div,
            ),
            state.config.show_menu != 0 && !state.menu.is_invalid(),
            crate::status::height(state.status),
        )
    });
    let Some((fullscreen, image, auto_fit, has_menu, status_h)) = gathered else {
        return;
    };
    // Get out of fullscreen first (upstream viv.c:2081-2085).
    if fullscreen {
        toggle_fullscreen(hwnd);
    }
    // ...and out of the maximized state (viv.c:2086-2089) — a SetWindowPos
    // with an explicit size already breaks it, but the restore keeps the
    // remembered placement bookkeeping clean.
    // SAFETY: live query on the owning thread.
    if unsafe { IsZoomed(hwnd) }.as_bool() {
        // SAFETY: as above.
        let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    // The percentage kinds need a live image (upstream viv.c:2091).
    if image == (0, 0) && kind != 3 {
        return;
    }
    // The monitors (upstream reads BOTH: the FULL rect for the auto-fit
    // fraction, the WORK rect for the clamp, viv.c:2126/2142).
    // SAFETY: read-only monitor queries on the live window.
    let (work, full) = unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        (mi.rcWork, mi.rcMonitor)
    };
    let client = crate::fit::window_size_client(
        image,
        kind,
        (full.right - full.left, full.bottom - full.top),
        (work.right - work.left, work.bottom - work.top),
        auto_fit,
    );
    // The current center (upstream midx/midy, viv.c:2119-2121).
    let mut rect = RECT::default();
    // SAFETY: read-only rect query.
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    let (midx, midy) = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
    // The outer rect for client + status bar (upstream viv.c:2139-2143:
    // AdjustWindowRect over the client target, then + status height).
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: client.0,
        bottom: client.1 + status_h,
    };
    // SAFETY: live style read; the rect is a valid in/out.
    let style = WINDOW_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32);
    // SAFETY: in/out rect valid; a failure leaves the raw client size and
    // the frame draws tight (upstream never checks either).
    let _ = unsafe { AdjustWindowRect(&mut outer, style, has_menu) };
    let wide = outer.right - outer.left;
    let high = outer.bottom - outer.top;
    // Center + nudge fully visible (upstream viv.c:2145-2153 +
    // os_make_rect_completely_visible; the rect is already in the window's
    // monitor frame, so the re-anchor pass is the identity).
    let place = |wide: i32, high: i32| {
        make_rect_completely_visible_core(
            RECT {
                left: midx - wide / 2,
                top: midy - high / 2,
                right: midx - wide / 2 + wide,
                bottom: midy - high / 2 + high,
            },
            work,
            work,
        )
    };
    let target = place(wide, high);
    // SAFETY: hwnd live and owned here; SWP_NOCOPYBITS like upstream
    // (viv.c:2163) so the old pixels never show through the resize.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            target.left,
            target.top,
            target.right - target.left,
            target.bottom - target.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        )
    };
    // The track-limit second pass (upstream viv.c:2140-2160): if the
    // window came out LARGER than requested, recenter the actual size.
    let mut actual = RECT::default();
    // SAFETY: read-only rect query.
    let _ = unsafe { GetWindowRect(hwnd, &mut actual) };
    let (aw, ah) = (actual.right - actual.left, actual.bottom - actual.top);
    if aw > wide || ah > high {
        let target = place(aw, ah);
        // SAFETY: as the first SetWindowPos.
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                target.left,
                target.top,
                target.right - target.left,
                target.bottom - target.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            )
        };
    }
}

/// Ctrl+0 — back to fit (upstream `VIV_ID_VIEW_ZOOM_RESET`, viv.c:1676-1681;
/// always repaints).
fn zoom_reset(hwnd: HWND) {
    // SAFETY: the borrow spans the pure reset math.
    if let Some(state) = unsafe { state_of(hwnd) } {
        let (vp, src) = viewport_and_src(hwnd, state);
        let fit = fit_policy(state);
        state.view.reset_zoom(src.0, src.1, vp, fit);
    }
    repaint(hwnd);
}

/// Ctrl+Alt+0 — toggle the temporary 1:1 pixel-exact mode (upstream
/// `_viv_view_1to1`, viv.c:9318-9339; always repaints).
fn toggle_one_to_one(hwnd: HWND) {
    // SAFETY: the borrow spans the pure toggle math.
    if let Some(state) = unsafe { state_of(hwnd) } {
        let (vp, src) = viewport_and_src(hwnd, state);
        let fit = fit_policy(state);
        state.view.toggle_one_to_one(src.0, src.1, vp, fit);
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
    // The action dispatch (upstream `_viv_do_left_click_action`,
    // viv.c:3319-3325 + 6360-6415): 0 scroll starts the drag; 3 zooms in
    // at the click; 4 advances. Unimplemented values (1/2/5/6) do nothing
    // — upstream's per-value switch has no default-to-scroll arm.
    // SAFETY: the borrow spans only the config read.
    let action = (unsafe { state_of(hwnd) })
        .map(|s| s.config.left_click_action)
        .unwrap_or(0);
    match action {
        3 => {
            zoom_at(hwnd, false, (pt.x, pt.y));
            return;
        }
        4 => {
            nav_next(hwnd, false);
            return;
        }
        // 0 falls through to the drag below; values riviv has no
        // handler for (1 slideshow, 2 animation pause, 5 1:1 scroll,
        // 6 move-window, hand-edited unknowns) do NOTHING, like
        // upstream's per-value switch (viv.c:6360-6415).
        0 => {}
        _ => return,
    }
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
                let fit = fit_policy(state);
                state
                    .view
                    .scroll_by(pt.x - lx, pt.y - ly, src.0, src.1, vp, fit);
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
/// (flags) any in-flight one.
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
        // The request-time render area rides the job for the worker's mip
        // pre-generation (upstream `_viv_load_render_wide/high`,
        // viv.c:1557-1558): the client WIDTH as-is, the height minus the
        // status bar — upstream subtracts the bar from the height only.
        // GetClientRect is a pure window query (no pumping), safe inside
        // this borrow like the stores around it.
        let mut client = RECT::default();
        // SAFETY: a pure window query on the live hwnd — no pumping, safe
        // inside this borrow like the stores around it.
        let _ = unsafe { GetClientRect(hwnd, &mut client) };
        // The startup request can race the bar's first layout (it is
        // created 0x0 and self-sizes) — a zero measured height would make
        // the pre-generation viewport too tall; fall back to the same
        // formula run() sized the initial window with (cubic, PR #18).
        let bar_h = match status::height(state.status) {
            0 => initial_status_height(),
            h => h,
        };
        let render_viewport = (
            client.right - client.left,
            (client.bottom - client.top - bar_h).max(0),
        );
        // The composite background snapshots at request time — a color
        // change mid-load must not flip frames already in flight.
        let background = state.config.windowed_bg();
        let session =
            state
                .load_thread
                .request(hwnd, path.to_os_string(), render_viewport, background);
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
/// next open resets it (viv.c:1447-1458).
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
/// nothing loads.
fn mark_startup_not_found(hwnd: HWND) {
    // SAFETY: the borrow spans only flag stores — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.status_file_not_found = true;
        state.status_load_failed = false;
    }
    refresh_status(hwnd);
}

/// Run one command line's FILE arguments through the open path (upstream
/// `_viv_process_command_line`, viv.c:4744-5148, minus the switch commands
/// riviv does not take — switches were dropped at parse, main.rs): ONE
/// argument keeps single-file semantics (a folder recurses into a
/// playlist, a wildcard expands, a file opens directly); a SECOND argument
/// pulls the first into the playlist too — everything added in argument
/// order — and the first-inserted entry is what opens. When nothing
/// resolves, the "File not found." verdict shows over the kept display
/// (upstream viv.c:5090-5098). Shared by the startup open (viv.c:5445) and
/// the single-instance handoff receive (#21, viv.c:3715).
///
/// `is_add` is upstream's add-mode (viv.c:4778-4793): the file words are
/// APPENDED to the playlist instead of replacing, and the display never
/// changes — the receive-side rapid-handoff window, so a multi-select
/// launch's burst of forwarded command lines builds one playlist instead
/// of replacing each other.
fn process_command_line(hwnd: HWND, args: &[OsString], is_add: bool) {
    // The parse-time clear (viv.c:4998-5006): a REPLACING command line with
    // at least one file word starts a fresh playlist; a switch-only line
    // (empty `args` here, or in add-mode) never clears.
    if !is_add && !args.is_empty() {
        // SAFETY: the borrow spans only the clear.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.playlist.clear();
        }
    }
    // Two-plus words add every word (upstream's loop adds the stashed
    // `single` once it sees the second word, then each next — net: all of
    // them, viv.c:5008-5023); a lone word is NOT added here — below it
    // either opens directly (replace) or is appended (add).
    if args.len() >= 2 {
        // SAFETY: the borrow spans the adds' metadata reads (read_dir /
        // FindFirstFile never pump messages).
        if let Some(state) = unsafe { state_of(hwnd) } {
            for arg in args {
                playlist::add_filename(&mut state.playlist, Path::new(arg));
            }
        }
    }
    if is_add {
        // SAFETY: the borrow spans the adds' metadata reads.
        if let Some(state) = unsafe { state_of(hwnd) } {
            // The bootstrap (viv.c:5028-5036): the current file seeds an
            // EMPTY playlist before the argument lands — after the
            // two-plus-word adds above, a non-empty list skips it, exactly
            // upstream's order.
            if state.playlist.is_empty()
                && let Some(current) = state.nav_current.as_ref()
            {
                let current = current.clone();
                state.playlist.add(current.path, current.modified);
            }
            // A lone word IS appended in add-mode (viv.c:5038-5041).
            if args.len() == 1 {
                playlist::add_filename(&mut state.playlist, Path::new(&args[0]));
            }
        }
    }
    // Show the first image — never in add-mode (viv.c:5046-5098).
    if !is_add && !args.is_empty() {
        let resolved = if args.len() == 1 {
            open_from_filename(hwnd, &args[0])
        } else {
            // SAFETY: the borrow ends at the end of this statement (the
            // entry is cloned out).
            let first = (unsafe { state_of(hwnd) }).and_then(|s| s.playlist.first().cloned());
            first.is_some_and(|entry| open_from_filename(hwnd, &entry.path))
        };
        if !resolved {
            mark_startup_not_found(hwnd);
        }
    }
    // Stamp the add-window anchor (viv.c:5146-5148): the END of every
    // command-line processing, add-mode included — a handoff arriving
    // within add_command_line_timeout of THIS one appends instead of
    // replacing.
    // SAFETY: the borrow spans only the tick store.
    if let Some(state) = unsafe { state_of(hwnd) } {
        // SAFETY: a cheap kernel tick query.
        state.last_cl_tick = Some(unsafe { GetTickCount() });
    }
}

/// WM_COPYDATA / `_VIV_COPYDATA_COMMAND_LINE` (upstream viv.c:3688-3719):
/// the single-instance handoff receive. The second instance's command line
/// is re-run in the first — adopt its working directory, walk its file
/// arguments through the same open path the startup command line took,
/// then show per the second launch's show state. Returns whether the
/// message was ours; a payload under 4 bytes is handled-and-ignored
/// (upstream's `e-p >= sizeof(DWORD)` gate, viv.c:3706).
fn on_copydata(hwnd: HWND, cds: &COPYDATASTRUCT) -> bool {
    if cds.dwData != copydata::COPYDATA_COMMAND_LINE {
        return false;
    }
    // Foreground first (viv.c:3696): the sender also tried right before
    // blocking in SendMessage (viv.c:5302); failing to steal foreground
    // under lock is tolerated — the return is ignored exactly like
    // upstream.
    // SAFETY: hwnd is live and owned by this thread.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let bytes = if cds.cbData >= size_of::<u32>() as u32 && !cds.lpData.is_null() {
        // SAFETY: the WM_COPYDATA contract guarantees lpData addresses
        // cbData readable bytes for the duration of the message.
        Some(unsafe { std::slice::from_raw_parts(cds.lpData.cast::<u8>(), cds.cbData as usize) })
    } else {
        None
    };
    let Some(handoff) = bytes.and_then(copydata::decode) else {
        return true;
    };
    // Adopt the sender's cwd BEFORE parsing (viv.c:3711-3713): its relative
    // file arguments must resolve against ITS working directory. Fail-soft —
    // upstream ignores the return (a vanished directory keeps ours).
    let mut cwd = handoff.cwd.clone();
    cwd.push(0);
    // SAFETY: cwd is NUL-terminated and outlives the call.
    let _ = unsafe { SetCurrentDirectoryW(PCWSTR::from_raw(cwd.as_ptr())) };
    // The same switch/file filter main.rs ran at startup, over the ORIGINAL
    // command line: CommandLineToArgvW is the splitter std itself uses for
    // `args_os`, and the exe word is skipped like upstream's first
    // string_get_word (viv.c:4789-4790).
    let args = handoff_file_args(&handoff.command_line);
    // The add-vs-replace decision (upstream viv.c:4778-4793) — the pure
    // `handoff_add_mode` below, fed the receiver's facts.
    // SAFETY: the read-only borrow ends inside is_some_and.
    let is_add = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        // SAFETY: a cheap kernel tick query.
        handoff_add_mode(
            unsafe { GetTickCount() },
            state.last_cl_tick,
            state.config.add_command_line_timeout,
            state.nav_current.is_some(),
        )
    });
    process_command_line(hwnd, &args, is_add);
    // Show per the second launch's requested state (viv.c:3717): a "run
    // maximized" shortcut forwards SW_MAXIMIZE; a plain launch's
    // SW_SHOWNORMAL restores a minimized window and brings it forward.
    // SAFETY: hwnd is live.
    let _ = unsafe { ShowWindow(hwnd, SHOW_WINDOW_CMD(handoff.show_cmd as i32)) };
    true
}

/// Split a forwarded command line into main.rs's file arguments: drop the
/// exe word, then run the shared switch/file/absolutize filter. An
/// unparseable line yields nothing — the handoff degenerates to a pure
/// bring-to-front, matching upstream's nothing-found word loop.
fn handoff_file_args(cl: &[u16]) -> Vec<OsString> {
    let mut cmd = cl.to_vec();
    cmd.push(0);
    let mut argc = 0i32;
    // SAFETY: cmd is NUL-terminated and outlives the call; on failure the
    // return is null and nothing was allocated. On success argv points at
    // an argc-sized array of NUL-terminated strings in LocalFree-owned
    // memory, read out below and freed exactly once on every path.
    let argv = unsafe { CommandLineToArgvW(PCWSTR::from_raw(cmd.as_ptr()), &mut argc) };
    if argv.is_null() {
        return Vec::new();
    }
    let mut words = Vec::new();
    for i in 0..argc.max(0) as usize {
        // SAFETY: argv[0..argc) are readable NUL-terminated strings per the
        // CommandLineToArgvW contract; the walk reads up to each NUL.
        let word = unsafe {
            let arg = *argv.add(i);
            let mut n = 0usize;
            while *arg.0.add(n) != 0 {
                n += 1;
            }
            OsString::from_wide(std::slice::from_raw_parts(arg.0, n))
        };
        words.push(word);
    }
    // SAFETY: argv came from CommandLineToArgvW and nothing retains it —
    // the strings were all copied out above.
    let _ = unsafe { LocalFree(Some(HLOCAL(argv.cast()))) };
    crate::file_args(words.into_iter().skip(1))
}

/// Whether a forwarded command line APPENDS to the playlist instead of
/// replacing (upstream viv.c:4778-4793): the previous command line ran
/// within `add_command_line_timeout` ticks AND something is loaded. The
/// tick difference wraps exactly like C's DWORD subtraction (GetTickCount
/// wraps at 2^32 ms), and a negative timeout casts to a huge DWORD the
/// same way C's `(DWORD)config_add_command_line_timeout` does. `timeout`
/// 0 disables the window entirely (the first condition upstream reads).
fn handoff_add_mode(
    now: u32,
    last_cl_tick: Option<u32>,
    timeout: i32,
    current_loaded: bool,
) -> bool {
    timeout != 0
        && last_cl_tick.is_some_and(|tick| now.wrapping_sub(tick) < timeout as u32)
        && current_loaded
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
    // A NEW image adopted the display this drain — the auto-size hook's
    // trigger (upstream `_viv_start_first_frame`'s tail, viv.c:14363-14383).
    let mut adopted_new_image = false;
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
            // Whether THIS session already owned the display before the
            // reply — the auto-size hook must fire on the adoption EDGE
            // only (upstream's `_viv_start_first_frame` runs at the first
            // frame; the completing reply's title refresh must not size
            // the window again).
            let displayed_before_reply = state.displayed_from == Some(session_id);
            let outcome = apply_reply(
                &mut state.image,
                &mut state.displayed_from,
                session_id,
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
                        if state.displayed_from == Some(session_id) && !displayed_before_reply {
                            adopted_new_image = true;
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
    }
    // Borrow dropped — the modal path below is safe.
    refresh_status(hwnd);
    // Auto-size the window to the fresh image (upstream viv.c:14363-14383):
    // windowed only, and only the four known types (a hand-edited type
    // outside 0..=3 makes upstream's switch a no-op too).
    if adopted_new_image {
        // SAFETY: a fresh short borrow for the config/mode read.
        let auto = (unsafe { state_of(hwnd) }).and_then(|s| {
            (!s.fullscreen && s.config.auto_zoom != 0).then_some(s.config.auto_zoom_type)
        });
        if let Some(kind) = auto.filter(|k| (0..=3).contains(k)) {
            window_size_to_image(hwnd, kind);
        }
    }
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
    // Dialog caption from the loc tables (upstream sets it from
    // LOCALIZATION_ID_OPEN_IMAGE_CAPTION, viv.c:2365) — riviv previously
    // left it at the system default ("Open").
    let title = to_wide(loc::get(loc::Id::OpenImageCaption));
    // 32768 code units: long paths must not trip FNERR_BUFFERTOOSMALL.
    let mut file_buf = vec![0u16; 32768];
    let dir = initial_dir.map(HSTRING::from);
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrFile: windows::core::PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrInitialDir: match &dir {
            Some(h) => PCWSTR(h.as_ptr()),
            None => PCWSTR::null(),
        },
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: `ofn` borrows stack locals (filter/title/file buffer/dir)
    // that all outlive this modal call.
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

/// The Open File / Add File action behind both the Ctrl+O /
/// Ctrl+Shift+O keys and the menu's File→Open File (upstream's
/// `VIV_ID_FILE_OPEN_FILE`/`_ADD_FILE` arm, viv.c:2278-2402): a modal
/// pick, then Add appends to the playlist keeping the display while Open
/// clears the list and opens the pick.
fn open_image_via_dialog(hwnd: HWND, add: bool) {
    // SAFETY: the borrow ends at the end of this statement (the path is
    // cloned out); the modal dialog below pumps messages but no borrow is
    // live by then.
    let initial_dir = (unsafe { state_of(hwnd) })
        .and_then(|s| s.path.clone())
        .and_then(|p| Path::new(&p).parent().map(|d| d.as_os_str().to_os_string()));
    let Some(path) = open_file_dialog(hwnd, initial_dir.as_deref()) else {
        return; // user cancelled
    };
    // SAFETY: the borrow spans only the playlist mutation —
    // nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        if add {
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
    if !add {
        let _ = open_from_filename(hwnd, &path);
    }
}

/// File→Open Folder (upstream `VIV_ID_FILE_OPEN_FOLDER`, viv.c:2404-2437):
/// pick a folder, clear the playlist (the OPEN arm, viv.c:2412-2416), then
/// open it — `open_from_filename` scans its images and homes onto the
/// first (upstream `_viv_playlist_add_path` + `_viv_home(0,0)`).
fn open_folder_via_dialog(hwnd: HWND) {
    // SAFETY: the borrow ends at the end of the statement (the initial dir
    // is cloned out); the modal dialog below pumps messages with no borrow
    // live. The last picked folder wins (upstream `_viv_last_open_folder`,
    // viv.c:2418-2431); without one, the current image's parent — riviv's
    // Ctrl+O convention — is the starting point.
    let initial_dir = (unsafe { state_of(hwnd) }).and_then(|s| {
        s.last_open_folder.clone().or_else(|| {
            s.path
                .clone()
                .and_then(|p| Path::new(&p).parent().map(|d| d.as_os_str().to_os_string()))
        })
    });
    let Some(folder) = pick_folder(hwnd, initial_dir.as_deref()) else {
        return; // cancelled / unavailable
    };
    // SAFETY: the borrow spans the playlist clear and the memory store —
    // nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.playlist.clear();
        state.last_open_folder = Some(folder.clone());
    }
    let _ = open_from_filename(hwnd, &folder);
}

/// The folder picker (upstream `os_browse_for_folder`'s IFileOpenDialog
/// arm, os.c: options 0x2028 = PICKFOLDERS | NOCHANGEDIR | HIDEMRUITEMS,
/// starting at a remembered folder when one exists — riviv derives it
/// from the current path like the Ctrl+O dialog's initial dir). None =
/// cancelled or unavailable; COM itself is initialized once in `run`
/// (upstream WinMain, viv.c:5228-5229).
/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)` — the IFileDialog's user-cancel
/// code, the one Show failure that is a normal no-pick exit rather than a
/// system failure.
const HRESULT_FROM_CANCELLED: i32 = 0x8007_0000u32 as i32 | 1223;

fn pick_folder(hwnd: HWND, initial_dir: Option<&OsStr>) -> Option<OsString> {
    // SAFETY: CoCreateInstance on the shell's registered FileOpenDialog
    // class with no outer aggregate; a failure degrades to "no folder
    // opened" (the viewer keeps working — the status-bar posture).
    let dialog: IFileOpenDialog =
        match unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) } {
            Ok(dialog) => dialog,
            Err(e) => {
                eprintln!("folder picker unavailable: {e}");
                return None;
            }
        };
    // SAFETY: live COM interface pointer; the option flags are plain
    // values. 0x2028 is upstream's set (os.c: FOS_PICKFOLDERS |
    // FOS_NOCHANGEDIR | FOS_HIDEMRUITEMS — the last not exposed by
    // windows-rs 0.62, spelled raw).
    if let Err(e) = unsafe {
        dialog.SetOptions(FOS_PICKFOLDERS | FOS_NOCHANGEDIR | FILEOPENDIALOGOPTIONS(0x2000))
    } {
        eprintln!("folder picker options failed: {e}");
        return None;
    }
    if let Some(dir) = initial_dir {
        let mut wide: Vec<u16> = dir.encode_wide().collect();
        wide.push(0);
        // SAFETY: NUL-terminated path owned by `wide` for the call; a
        // failure skips the start folder (the dialog opens at its default,
        // like upstream's NULL shell item).
        let item: Result<IShellItem, _> =
            unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None::<&IBindCtx>) };
        if let Ok(item) = item {
            // SAFETY: live COM pointers on both sides.
            let _ = unsafe { dialog.SetFolder(&item) };
        }
    }
    // SAFETY: the modal Show pumps messages — the caller guarantees no
    // state borrow is live.
    if let Err(e) = unsafe { dialog.Show(Some(hwnd)) } {
        // The plain cancel is the normal no-pick exit; anything else is a
        // system-level failure worth a diagnostic before the degrade to
        // "nothing happened" (ADR 0001 posture; Codex round 2).
        if e.code().0 != HRESULT_FROM_CANCELLED {
            eprintln!("folder picker Show failed: {e}");
        }
        return None; // cancelled (or failed)
    }
    // SAFETY: live COM pointer; the result item is ours.
    let item: IShellItem = match unsafe { dialog.GetResult() } {
        Ok(item) => item,
        Err(e) => {
            eprintln!("folder picker GetResult failed: {e}");
            return None;
        }
    };
    // SAFETY: live COM pointer; the returned buffer is CoTaskMem-allocated
    // and freed exactly once below.
    let name = match unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
        Ok(name) => name,
        Err(e) => {
            eprintln!("folder picker GetDisplayName failed: {e}");
            return None;
        }
    };
    let mut len = 0usize;
    // SAFETY: GetDisplayName's contract returns a NUL-terminated string;
    // the walk reads up to that NUL only.
    unsafe {
        while *name.as_ptr().add(len) != 0 {
            len += 1;
        }
    }
    // SAFETY: the length walk above bounded the readable region.
    let path = OsString::from_wide(unsafe { core::slice::from_raw_parts(name.as_ptr(), len) });
    // SAFETY: the buffer was allocated by the shell on the COM task heap
    // (GetDisplayName's contract); freeing it here releases the only copy.
    unsafe { CoTaskMemFree(Some(name.as_ptr().cast())) };
    Some(path)
}

/// Help→About (upstream `VIV_ID_HELP_ABOUT` → the IDD_ABOUT resource
/// dialog, viv.c:1696/9728-9800). riviv ships no dialog resources — a
/// message box carries the same facts (README Differences).
fn show_about(hwnd: HWND) {
    let text = format!(
        "riviv {}\n\nUnofficial Rust rewrite of voidtools void Image Viewer.\nUpstream (MIT): https://www.voidtools.com/voidimageviewer/\nSource: https://github.com/jaredshuai/riviv",
        env!("CARGO_PKG_VERSION")
    );
    let text_wide = to_wide(&text);
    let caption = to_wide(loc::get(loc::Id::AppName));
    // SAFETY: both buffers outlive the modal call; hwnd is the live owner.
    let _ = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text_wide.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK,
        )
    };
}

/// Build the menu bar from the command table (upstream `_viv_create_menu`,
/// viv.c:12314-12399): walk `menu::ENTRIES`, create each popup on demand
/// keyed by its slot, append separators/items with their accelerator
/// labels. Returns an invalid HMENU on failure (callers degrade to a
/// menu-less window, like a failed status bar).
fn create_menu_bar() -> HMENU {
    // SAFETY: pure menu-object construction; no window involvement. A
    // failure (NULL handle) degrades to a menu-less window.
    let Ok(bar) = (unsafe { CreateMenu() }) else {
        return HMENU::default();
    };
    let mut slots: [HMENU; menu::Slot::COUNT] = [HMENU::default(); menu::Slot::COUNT];
    slots[menu::Slot::Root as usize] = bar;
    for entry in menu::ENTRIES {
        match *entry {
            menu::Entry::Separator { parent } => {
                // SAFETY: the parent slot's menu exists (the table's
                // popup-before-use invariant, tested in menu.rs).
                let _ =
                    unsafe { AppendMenuW(slots[parent as usize], MF_SEPARATOR, 0, PCWSTR::null()) };
            }
            menu::Entry::Popup { loc, parent, slot } => {
                // SAFETY: fresh popup creation; a failure stores the
                // invalid handle and this popup's rows append nowhere
                // (degraded menu, like the failed bar).
                let popup = unsafe { CreatePopupMenu() }.unwrap_or_default();
                slots[slot as usize] = popup;
                let text = to_wide(loc::get(loc));
                // SAFETY: text outlives the append; the popup handle moves
                // into the parent menu here.
                let _ = unsafe {
                    AppendMenuW(
                        slots[parent as usize],
                        MF_STRING | MF_POPUP,
                        popup.0 as usize,
                        PCWSTR(text.as_ptr()),
                    )
                };
            }
            menu::Entry::Item {
                loc,
                parent,
                cmd,
                key,
            } => {
                // The accelerator label: the default key's display name
                // via GetKeyNameTextW (layout-localized like upstream,
                // `_viv_vk_to_text` viv.c:12221-12261), composed by the
                // pure `menu::key_label`.
                let label = key.and_then(|k| vk_text(k.vk).map(|t| menu::key_label(k, &t)));
                let text = to_wide(&menu::item_text(loc::get(loc), label.as_deref()));
                // SAFETY: text outlives the append.
                let _ = unsafe {
                    AppendMenuW(
                        slots[parent as usize],
                        MF_STRING,
                        usize::from(cmd.id()),
                        PCWSTR(text.as_ptr()),
                    )
                };
            }
        }
    }
    bar
}

/// The key-name half of an accelerator label (upstream `_viv_vk_to_text`,
/// viv.c:12221-12261): scan code from the thread's keyboard layout, then
/// `GetKeyNameTextW` with the extended-key bit for the navigation keys
/// riviv registers (upstream's full extended list covers keys riviv has
/// no default binding for). None = no name (the item then shows no
/// accelerator).
fn vk_text(vk: u16) -> Option<String> {
    // SAFETY: read-only layout query for this thread.
    let hkl = unsafe { GetKeyboardLayout(GetCurrentThreadId()) };
    // SAFETY: pure VK→scan-code mapping.
    let scan = unsafe { MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC, Some(hkl)) };
    if scan == 0 {
        return None;
    }
    let mut lparam = (scan as i32) << 16;
    // The extended bit (1 << 24) for keys that live only on the extended
    // cluster (arrows/Home/End) — without it GetKeyNameTextW names the
    // wrong key or fails (viv.c:12243-12254).
    if matches!(vk, v if v == VK_HOME.0 || v == VK_END.0 || v == VK_LEFT.0 || v == VK_RIGHT.0) {
        lparam |= 1 << 24;
    }
    let mut buf = [0u16; 64];
    // SAFETY: buf outlives the call; a zero return means "no name".
    let len = unsafe { GetKeyNameTextW(lparam, &mut buf) };
    if len <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// WM_INITMENU (upstream viv.c:3063-3072: `_viv_check_menus` over
/// `GetMenu(hwnd)` just before the bar opens) — apply the check marks and
/// grays for the state right now.
fn on_initmenu(hwnd: HWND) {
    // The 1:1 check is upstream's render-size == image-size compare
    // (viv.c:7131); blank displays carry no 1:1 state (upstream's raw
    // 0 == 0 compare would check the item on an empty window — riviv
    // guards it, README Differences).
    // SAFETY: the borrow spans only the snapshot reads (window queries
    // inside are pump-free); the menu calls below run outside it.
    let snapshot = (unsafe { state_of(hwnd) }).map(|state| {
        let one_to_one = state.image.as_ref().is_some_and(|image| {
            let (vp, src) = viewport_and_src(hwnd, state);
            let fit = fit_policy(state);
            let (rw, rh) = state.view.render_size(src.0, src.1, vp, fit);
            (rw, rh) == (image.width(), image.height())
        });
        menu::MenuState {
            show_menu: state.config.show_menu != 0,
            fullscreen: state.fullscreen,
            one_to_one,
        }
    });
    let Some(state) = snapshot else {
        return;
    };
    // SAFETY: read-only query of the window's own menu.
    let bar = unsafe { GetMenu(hwnd) };
    if bar.is_invalid() {
        return;
    }
    for cmd in menu::Cmd::ALL {
        let flags: u32 = if menu::checked(cmd, &state) {
            (MF_CHECKED | MF_BYCOMMAND).0
        } else {
            (MF_UNCHECKED | MF_BYCOMMAND).0
        };
        // SAFETY: bar is the window's own live menu.
        let _ = unsafe { CheckMenuItem(bar, u32::from(cmd.id()), flags) };
        let enable: MENU_ITEM_FLAGS = if menu::enabled(cmd) {
            MF_ENABLED | MF_BYCOMMAND
        } else {
            // Grayed, not just disabled: the Options placeholder is a
            // riviv-specific condition (upstream's MF_DISABLED usage,
            // viv.c:7098, gates image-dependent commands that do not exist
            // in riviv's table yet), and the README promises a GREYED
            // placeholder — MF_DISABLED alone renders normal text and
            // leaves a clickable-looking no-op (cubic round 1).
            MF_GRAYED | MF_BYCOMMAND
        };
        // SAFETY: bar is the window's own live menu.
        let _ = unsafe { EnableMenuItem(bar, u32::from(cmd.id()), enable) };
    }
}

/// WM_CONTEXTMENU's recovery slice (upstream builds its full context menu
/// here, viv.c:3376-3550 — navigation, slideshow rate, sort modes…; riviv
/// ships only the one row whose feature exists): upstream puts "Menu" in
/// the context menu GATED to appear only while the bar is hidden
/// (viv.c:3427 + `_viv_context_menu_items`, viv.c:1084) — the in-app way
/// back from View→Menu OFF. Without it the bar could only return by
/// hand-editing the ini (cubic round 1).
fn on_contextmenu(hwnd: HWND, lparam: LPARAM) {
    // The keyboard invocation (-1/-1, Shift+F10 / Menu key) centers the
    // popup on the window instead of skipping (upstream viv.c:3386-3398:
    // GetWindowRect → center → TPM_CENTERALIGN|TPM_VCENTERALIGN) —
    // keyboard-only users get the same recovery row (Codex round 2).
    let (x, y, flags) = if lparam.0 == -1 {
        let mut r = RECT::default();
        // SAFETY: read-only rect query on the live window; a failure reads
        // the zeroed rect and the popup lands at the origin.
        let _ = unsafe { GetWindowRect(hwnd, &mut r) };
        (
            (r.left + r.right) / 2,
            (r.top + r.bottom) / 2,
            TPM_CENTERALIGN | TPM_VCENTERALIGN,
        )
    } else {
        (
            (lparam.0 as u16) as i16 as i32,
            ((lparam.0 as u32 >> 16) as u16) as i16 as i32,
            // Upstream passes tpm_flags = 0 for the mouse path (viv.c:3382).
            TPM_LEFTBUTTON, // TRACK_POPUP_MENU_FLAGS(0)
        )
    };
    // SAFETY: the borrow spans only the three reads — nothing pumps.
    let next = (unsafe { state_of(hwnd) }).and_then(|state| {
        if state.fullscreen || state.config.show_menu != 0 {
            return None; // bar visible (or fullscreen): nothing to recover
        }
        Some(state.menu)
    });
    let Some(menu_bar) = next else {
        return;
    };
    if menu_bar.is_invalid() {
        return;
    }
    // SAFETY: fresh popup creation.
    let Ok(popup) = (unsafe { CreatePopupMenu() }) else {
        return;
    };
    let text = to_wide(loc::get(loc::Id::MenuMenu));
    // SAFETY: text outlives the append; the command id routes back through
    // the shared WM_COMMAND dispatch when picked.
    let _ = unsafe {
        AppendMenuW(
            popup,
            MF_STRING,
            usize::from(menu::Cmd::ViewMenu.id()),
            PCWSTR(text.as_ptr()),
        )
    };
    // The check state the row would carry (unchecked — the bar is hidden;
    // upstream runs `_viv_check_menus` over the popup, viv.c:3530).
    // SAFETY: our fresh popup.
    let _ = unsafe {
        CheckMenuItem(
            popup,
            u32::from(menu::Cmd::ViewMenu.id()),
            (MF_UNCHECKED | MF_BYCOMMAND).0,
        )
    };
    // The MSDN menu-dismissal pattern: foreground the owner before
    // TrackPopupMenu and nudge it with WM_NULL after, so the menu closes
    // when focus leaves (a denial is ignored — the menu still works).
    // SAFETY: our own live window.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    // SAFETY: our popup shown at the message's screen point over our
    // window; blocks until dismissed and posts WM_COMMAND on pick.
    let _ = unsafe { TrackPopupMenu(popup, flags, x, y, None, hwnd, None) };
    // SAFETY: our popup, whose tracking ended above.
    let _ = unsafe { DestroyMenu(popup) };
    // SAFETY: our own window; the empty nudge just wakes the pump.
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
}

/// WM_COMMAND dispatch (upstream `_viv_command`, viv.c:1658-2580: the
/// switch over command ids onto the same action functions the keys use —
/// menu triggers are never key repeats, so the repeat-wait gates of the
/// keyboard path do not apply, matching upstream's is_key_repeat=0).
fn on_command(hwnd: HWND, cmd: menu::Cmd) {
    match cmd {
        menu::Cmd::FileOpenFile => open_image_via_dialog(hwnd, false),
        menu::Cmd::FileOpenFolder => open_folder_via_dialog(hwnd),
        menu::Cmd::FileAddFile => open_image_via_dialog(hwnd, true),
        menu::Cmd::FileExit => {
            // Upstream `_viv_exit` (viv.c:1883-1888) saves the config and
            // quits the pump; riviv's WM_DESTROY does both on the way out.
            // SAFETY: legal on the owning thread; synchronously runs
            // WM_DESTROY/WM_NCDESTROY with no borrow live.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        menu::Cmd::ViewMenu => toggle_menu(hwnd),
        menu::Cmd::ViewFullscreen => toggle_fullscreen(hwnd),
        menu::Cmd::ViewOneToOne => toggle_one_to_one(hwnd),
        // Upstream's two fit commands both collapse the zoom position back
        // to the fit level (`VIV_ID_VIEW_BESTFIT` zeroes the zoom position
        // and refits, `VIV_ID_VIEW_ZOOM_RESET` drops 1:1 and the zoom
        // position — viv.c:1678-1687/2059-2064); riviv's Ctrl+0 reset is
        // that action.
        menu::Cmd::ViewBestFit | menu::Cmd::ViewZoomReset => zoom_reset(hwnd),
        menu::Cmd::ViewZoomIn => zoom_step_centered(hwnd, false),
        menu::Cmd::ViewZoomOut => zoom_step_centered(hwnd, true),
        // The Options dialog (#24) — modal over the viewer; commits into
        // the live config on OK (instant effect + save).
        menu::Cmd::ViewOptions => crate::options_dlg::open(hwnd),
        menu::Cmd::NavNext => nav_next(hwnd, false),
        menu::Cmd::NavPrev => nav_next(hwnd, true),
        menu::Cmd::NavHome => home_open(hwnd, false),
        menu::Cmd::NavEnd => home_open(hwnd, true),
        menu::Cmd::HelpAbout => show_about(hwnd),
    }
}

/// View→Menu (upstream `VIV_ID_VIEW_MENU`, viv.c:1975-1978): flip
/// `config_show_menu` and rebuild the frame around the unchanged client
/// area. Unreachable while fullscreen (the windowed shell carrying the
/// bar is hidden there and the command has no accelerator) — and a strict
/// no-op there rather than a bare config flip, so the flag can never
/// desync from the attached bar (cubic round 1).
fn toggle_menu(hwnd: HWND) {
    // SAFETY: the borrow spans the fullscreen read, the config flip and
    // the handle copy — nothing pumps.
    let next = (unsafe { state_of(hwnd) }).and_then(|state| {
        if state.fullscreen {
            return None;
        }
        state.config.show_menu = i32::from(state.config.show_menu == 0);
        Some((state.config.show_menu != 0, state.menu))
    });
    if let Some((show, menu)) = next {
        update_menu_frame(hwnd, show, menu);
    }
}

/// The frame rebuild around a menu attach/detach (upstream
/// `_viv_update_frame`'s menu arm, viv.c:9840-9925 — riviv keeps caption
/// and thick frame permanently, so only the menu presence changes):
/// re-seat the menu, then shift the outer rect by the AdjustWindowRect
/// delta of the SAME client area so the client — the image — stays
/// exactly where it was when the bar appears or disappears.
fn update_menu_frame(hwnd: HWND, show: bool, menu: HMENU) {
    let mut client = RECT::default();
    // SAFETY: read-only client query on the live window; a failure reads
    // the zeroed rect and the deltas collapse to a no-op shift.
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let mut with_menu = client;
    // SAFETY: in/out rect valid for the call; the BOOL return is ignored
    // like upstream (a failure leaves the frame delta at zero).
    let _ = unsafe { AdjustWindowRect(&mut with_menu, WS_OVERLAPPEDWINDOW, true) };
    let mut without_menu = client;
    // SAFETY: same call with bMenu false.
    let _ = unsafe { AdjustWindowRect(&mut without_menu, WS_OVERLAPPEDWINDOW, false) };
    // Attach/detach FIRST (upstream SetMenu before the rect math,
    // viv.c:9879-9889) so the SWP_FRAMECHANGED below applies the final
    // state in one pass.
    // SAFETY: `menu` is the state's own bar when attaching, None detaches;
    // the BOOL return is ignored like upstream (a failure keeps the old
    // attach state, and the rect shift still matches the menu-less frame).
    let _ = unsafe {
        SetMenu(
            hwnd,
            if show && !menu.is_invalid() {
                Some(menu)
            } else {
                None
            },
        )
    };
    let mut window = RECT::default();
    // SAFETY: read-only outer-rect query; fail-soft like upstream's
    // unchecked GetWindowRect (viv.c:9908).
    let _ = unsafe { GetWindowRect(hwnd, &mut window) };
    // Shift by the frame delta — NEW frame minus OLD (viv.c:9910-9913:
    // windowrect += newrect - oldrect over the same client): attaching
    // moves the top up by the bar, detaching pulls it back down. Wrapping
    // like the rest of riviv's rect math so pathological values cannot
    // panic.
    let (new_frame, old_frame) = if show {
        (&with_menu, &without_menu)
    } else {
        (&without_menu, &with_menu)
    };
    window.left = window
        .left
        .wrapping_add(new_frame.left.wrapping_sub(old_frame.left));
    window.top = window
        .top
        .wrapping_add(new_frame.top.wrapping_sub(old_frame.top));
    window.right = window
        .right
        .wrapping_add(new_frame.right.wrapping_sub(old_frame.right));
    window.bottom = window
        .bottom
        .wrapping_add(new_frame.bottom.wrapping_sub(old_frame.bottom));
    // SAFETY: read-only zoomed query — the restore below needs the
    // pre-change state.
    let was_maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    // SAFETY: live window; re-asserts top like upstream's HWND_TOP
    // (viv.c:9918-9920) and applies the frame change synchronously —
    // WM_SIZE re-docks the status bar with no borrow live out here.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            window.left,
            window.top,
            window.right.wrapping_sub(window.left),
            window.bottom.wrapping_sub(window.top),
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        )
    };
    // A maximized window loses its zoom under the explicit rect — restore
    // it like upstream (viv.c:9921-9924; its caption/thickframe conditions
    // are permanently true in riviv).
    if was_maximized {
        // SAFETY: live window.
        let _ = unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
    }
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
            open_image_via_dialog(hwnd, shift);
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
    // The menu-registered command keys the keyboard path also answers
    // (upstream default keymap, exact masks): Ctrl+B open folder
    // (viv.c:972), Ctrl+Q exit (viv.c:986), Ctrl+F1 about (viv.c:1074).
    if vk == u16::from(b'B') && ctrl && !alt && !shift {
        open_folder_via_dialog(hwnd);
        return;
    }
    if vk == u16::from(b'Q') && ctrl && !alt && !shift {
        // SAFETY: legal on the owning thread; WM_DESTROY saves the config
        // and posts the quit (upstream _viv_exit, viv.c:1883-1888).
        let _ = unsafe { DestroyWindow(hwnd) };
        return;
    }
    if vk == VK_F1.0 && ctrl && !alt && !shift {
        show_about(hwnd);
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
    // Upstream `_viv_on_size`'s first act (viv.c:1583-1615): track the
    // windowed rect for the config. Never iconic, never fullscreen; a
    // maximized window only flips the flag so the normal rect survives
    // for the restore (WM_MOVE below skips maximized, keeping x/y).
    // SAFETY: read-only iconic query on the live window.
    let iconic = unsafe { IsIconic(hwnd) }.as_bool();
    if !iconic
        // SAFETY: the borrow spans only the config field writes and the
        // rect read; nothing pumps messages.
        && let Some(state) = unsafe { state_of(hwnd) }
        && !state.fullscreen
    {
        // Under the !IsIconic guard, upstream's `_viv_is_window_maximized`
        // (viv.c:9637-9657) reduces to IsZoomed.
        // SAFETY: read-only zoomed query on the live window.
        let is_maximized = unsafe { IsZoomed(hwnd) }.as_bool();
        state.config.maximized = i32::from(is_maximized);
        if !is_maximized {
            let mut rect = RECT::default();
            // SAFETY: read-only rect query; fail-soft like upstream's
            // unchecked GetWindowRect (viv.c:1606).
            let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
            state.config.wide = rect.right - rect.left;
            state.config.high = rect.bottom - rect.top;
        }
    }
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
        let fit = fit_policy(state);
        state.view.on_resize(src.0, src.1, vp, fit)
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
        WM_MOVE => {
            // Track the windowed position for the config (viv.c:3955-3972):
            // never iconic, never maximized (the restore placement comes
            // from the normal rect tracked in WM_SIZE), never fullscreen
            // (the monitor cover is not a window position).
            // SAFETY: read-only iconic query on the live window.
            let iconic = unsafe { IsIconic(hwnd) }.as_bool();
            // SAFETY: read-only zoomed query on the live window.
            let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
            if !iconic
                && !zoomed
                // SAFETY: the borrow spans only the config field writes and
                // the rect read; nothing pumps messages.
                && let Some(state) = unsafe { state_of(hwnd) }
                && !state.fullscreen
            {
                let mut rect = RECT::default();
                // SAFETY: read-only rect query; fail-soft like upstream's
                // unchecked GetWindowRect (viv.c:3965).
                let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
                state.config.x = rect.left;
                state.config.y = rect.top;
            }
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
        // Menu dispatch (upstream viv.c:4013-4019: `_viv_command` on the
        // LOWORD of wParam, then break to the default procedure).
        WM_COMMAND => {
            if let Some(cmd) = menu::Cmd::from_id((wparam.0 & 0xffff) as u16) {
                on_command(hwnd, cmd);
            }
            // SAFETY: hwnd/msg are exactly what this callback received; the
            // default procedure handles everything we do not.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // Refresh the bar's checks/grays just before it opens (upstream
        // viv.c:3063-3072 — WM_INITMENU over GetMenu(hwnd), not
        // WM_INITMENUPOPUP), then let the default procedure open it.
        WM_INITMENU => {
            on_initmenu(hwnd);
            // SAFETY: hwnd/msg are exactly what this callback received; the
            // default procedure owns the menu-open default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // Right-click: the bar-recovery slice of upstream's context menu
        // (viv.c:3376-3550 — only its "Menu" row, gated to the hidden
        // state per viv.c:3427); the full context menu lands with its
        // features. Breaks to the default procedure like upstream.
        WM_CONTEXTMENU => {
            on_contextmenu(hwnd, lparam);
            // SAFETY: hwnd/msg are exactly what this callback received; the
            // default procedure handles everything we do not.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
            on_double_click(hwnd, lparam);
            LRESULT(0)
        }
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK | WM_RBUTTONUP => {
            // DBLCLK rides CS_DBLCLKS in place of the second DOWN — upstream
            // handles it in the same arm (viv.c:3349-3355).
            // Right-click actions 1/2 (zoom out / previous) swallow BOTH
            // messages — upstream returns 0 on down and up (viv.c:3349-3367)
            // so DefWindowProc never turns the click into WM_CONTEXTMENU.
            // Action 0 falls through: the click becomes the context menu
            // (riviv's bar-recovery slice).
            if on_right_button(hwnd, msg, lparam) {
                LRESULT(0)
            } else {
                // SAFETY: the parameters are exactly this callback's own;
                // the default procedure owns everything unmatched (the
                // context-menu production for the right click among them).
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
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
        // The single-instance handoff receive (#21; upstream viv.c:3688-3719):
        // only the command-line id is ours — anything else (upstream also
        // multiplexes its Everything-search IPC here) goes to the default.
        // A null lparam is a malformed foreign send no legitimate sender
        // makes — upstream null-derefs straight into an access violation
        // here; riviv routes it to the default instead (the PR #27 rule:
        // where C crashes on pathological input, Rust must degrade
        // gracefully, never UB).
        WM_COPYDATA => {
            // SAFETY: lparam points at the sender-owned COPYDATASTRUCT for
            // the duration of the message (the WM_COPYDATA contract), and
            // the null guard keeps hostile sends out of the cast.
            if lparam.0 != 0 && on_copydata(hwnd, unsafe { &*(lparam.0 as *const COPYDATASTRUCT) })
            {
                LRESULT(1)
            } else {
                // SAFETY: hwnd/msg are exactly what this callback received;
                // the default procedure handles everything we do not.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
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
            // Settings go to disk on the way out (upstream `_viv_exit`'s
            // config_save_settings, viv.c:2577-2580; riviv saves at window
            // destruction — every normal exit path funnels here, and
            // WM_ENDSESSION below covers the session-shutdown exits that
            // never reach it). The window-position tracking in
            // WM_SIZE/WM_MOVE has kept the config current; save only does
            // file I/O, no messages, so the borrow is safe. Failures are
            // logged inside, not fatal.
            // SAFETY: the borrow spans only the save's file I/O.
            if let Some(state) = unsafe { state_of(hwnd) } {
                state.config.save();
            }
            // SAFETY: legal on the owning thread while quitting the message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // Session shutdown (viv.c:2743-2750): agree to end, and persist
        // the settings on the confirmed end-session — the process may be
        // terminated without WM_DESTROY ever arriving.
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                // SAFETY: the borrow spans only the save's file I/O.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    state.config.save();
                }
            }
            LRESULT(0)
        }
        // SAFETY: hwnd/msg are exactly what this callback received; the default
        // procedure handles everything we do not.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The outer (width, height) handed to CreateWindowExW / the startup
/// SetWindowPos — wrapping like C: a remembered rect can hold any i32 pair
/// (the wrap happens in `initial_window_rect`), and C hands the wrapped
/// size to Win32, which rejects it gracefully; a debug-build panic must
/// not get there first (Codex PR #27 round 3).
fn rect_size(rect: RECT) -> (i32, i32) {
    (
        rect.right.wrapping_sub(rect.left),
        rect.bottom.wrapping_sub(rect.top),
    )
}

/// The startup window rect (viv.c:5354-5387): the remembered config rect,
/// or on first run (wide/high == 0) the auto-fit share of the cursor
/// monitor's FULL rect (`rcMonitor`, not the work area — the os_ wrapper's
/// flag is 1 there), centered — in MONITOR-RELATIVE coordinates, an
/// upstream quirk preserved verbatim: the monitor origin is never added
/// (viv.c:5383-5386 computes `width/2 - wide/2` without the origin), so on
/// a secondary monitor the rect lands near the primary's origin and
/// `make_rect_completely_visible` re-anchors it (a net no-op there — the
/// window really does open on the primary's analogous spot). The div=0
/// fallback is the 640x480 default (viv.c:5367-5368).
fn initial_window_rect(config: &Config) -> Result<RECT, String> {
    let mut rect = RECT {
        left: config.x,
        top: config.y,
        // Wrapping like the C build: a hand-edited ini can hold any i32
        // (parse_int mirrors utf8_to_int's silent wrap), and a panicked
        // startup is worse than upstream's garbage rect handed to
        // CreateWindowEx, which fails soft.
        right: config.x.wrapping_add(config.wide),
        bottom: config.y.wrapping_add(config.high),
    };
    if config.wide == 0 || config.high == 0 {
        // SAFETY: read-only cursor + monitor queries.
        unsafe {
            let mut cursor = POINT::default();
            // Fail loud: a zero cursor point would silently center on
            // whichever monitor is nearest (0,0) instead of the user's
            // (ADR 0001).
            GetCursorPos(&mut cursor).map_err(|e| format!("GetCursorPos failed: {e}"))?;
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // Fail loud like the rest of run()'s geometry: a zeroed
            // monitor rect would silently create a 0x0 window (ADR 0001);
            // DEFAULTTOPRIMARY makes the failure near-unreachable, but a
            // stale handle through a display-topology change is exactly
            // the system-level case worth reporting.
            if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
                // Already inside this function's outer unsafe block.
                let gle = GetLastError().0;
                return Err(format!("GetMonitorInfoW failed (GLE={gle})"));
            }
            let full = mi.rcMonitor;
            rect = first_run_window_rect(
                full,
                config.auto_fit_wide_mul,
                config.auto_fit_wide_div,
                config.auto_fit_high_mul,
                config.auto_fit_high_div,
            );
        }
    }
    Ok(rect)
}

/// The first-run centered rect (viv.c:5361-5387) — pure. 60% of the
/// monitor by default, 640x480 when a div is 0, in the monitor's own
/// coordinate frame (see `initial_window_rect` for the no-origin quirk).
/// The arithmetic wraps like the C build: the mul/div come from the ini,
/// where any i32 is possible (`i32::MIN / -1` panics even in release
/// otherwise).
fn first_run_window_rect(
    monitor: RECT,
    wide_mul: i32,
    wide_div: i32,
    high_mul: i32,
    high_div: i32,
) -> RECT {
    let mut wide = 640;
    let mut high = 480;
    let mw = monitor.right - monitor.left;
    let mh = monitor.bottom - monitor.top;
    if wide_div != 0 {
        wide = mw.wrapping_mul(wide_mul).wrapping_div(wide_div);
    }
    if high_div != 0 {
        high = mh.wrapping_mul(high_mul).wrapping_div(high_div);
    }
    let left = (mw / 2).wrapping_sub(wide / 2);
    let top = (mh / 2).wrapping_sub(high / 2);
    RECT {
        left,
        top,
        right: left.wrapping_add(wide),
        bottom: top.wrapping_add(high),
    }
}

/// Pull a startup rect onto a visible monitor (os.c:193-248, the shell
/// gathering the two WORK-area rects — the os_ wrappers' flag 0 — before
/// the pure core runs). The rect is re-anchored from ITS monitor onto the
/// WINDOW's monitor, then clamped fully into view.
fn make_rect_completely_visible(hwnd: HWND, rect: &mut RECT) {
    // SAFETY: read-only monitor queries; both DEFAULTTOPRIMARY handles are
    // never null, and GetMonitorInfo's failure leaves a zeroed rect exactly
    // like upstream's unchecked call (os.c:202-203).
    unsafe {
        let mon_of_window = work_area(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY));
        let mon_of_rect = work_area(MonitorFromRect(rect, MONITOR_DEFAULTTOPRIMARY));
        *rect = make_rect_completely_visible_core(*rect, mon_of_window, mon_of_rect);
    }
}

/// SAFETY contract helper: fills a MONITORINFO's work area for a live
/// monitor handle.
///
/// # Safety
///
/// `monitor` must be a valid HMONITOR (the DEFAULTTOPRIMARY flag above
/// guarantees non-null).
unsafe fn work_area(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> RECT {
    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `monitor` is live per the fn's safety contract; `mi` is a
    // valid out-struct for the call, and the unchecked return matches
    // upstream (a failure leaves the zeroed work rect).
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut mi);
    }
    mi.rcWork
}

/// The os.c:193-248 re-anchor + clamp, pure: offset the rect from its own
/// monitor's frame into the target monitor's frame, cap the size to the
/// target, then push each side that sticks out back in. Wrapping
/// arithmetic throughout like the C build — a pathological remembered
/// rect (any i32 can sit in the ini) must wrap exactly like C, never
/// panic (cubic PR round 2).
fn make_rect_completely_visible_core(rect: RECT, target: RECT, source: RECT) -> RECT {
    let dx = target.left.wrapping_sub(source.left);
    let dy = target.top.wrapping_sub(source.top);
    let mut r = RECT {
        left: rect.left.wrapping_add(dx),
        top: rect.top.wrapping_add(dy),
        right: rect.right.wrapping_add(dx),
        bottom: rect.bottom.wrapping_add(dy),
    };
    let tw = target.right.wrapping_sub(target.left);
    let th = target.bottom.wrapping_sub(target.top);
    let wide = r.right.wrapping_sub(r.left).min(tw);
    let high = r.bottom.wrapping_sub(r.top).min(th);
    if r.right > target.right {
        r.left = target.right.wrapping_sub(wide);
        r.right = target.right;
    }
    if r.bottom > target.bottom {
        r.top = target.bottom.wrapping_sub(high);
        r.bottom = target.bottom;
    }
    if r.left < target.left {
        r.left = target.left;
        r.right = target.left.wrapping_add(wide);
    }
    if r.top < target.top {
        r.top = target.top;
        r.bottom = target.top.wrapping_add(high);
    }
    r
}

/// The status bar's height before its window exists (used only for the
/// startup load's pre-generation viewport fallback — the live bar is
/// measured via `status::height` afterwards). comctl32 sizes a status bar
/// from the system status font and border metrics; we reproduce that
/// formula (border * 2 + font height) at the system DPI.
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
    // SAFETY: process-wide and must run before ANY DPI-sensitive query —
    // a GetMonitorInfo/GetCursorPos-scale call while still unaware LOCKS
    // the process into DPI virtualization and later SetProcessDPIAware
    // calls fail with ERROR_ACCESS_DENIED (the config work's first-run
    // rect queries monitors, so this leads everything; upstream gets the
    // same guarantee from its manifest's dpiAware=true). ERROR_ACCESS_DENIED
    // means the process is already DPI-aware — a success state; any other
    // failure undermines the whole 1:1 / work-area geometry model, so fail
    // loud (ADR 0001).
    if !unsafe { SetProcessDPIAware() }.as_bool() {
        // SAFETY: reading the thread's last error immediately after the failed call.
        let gle = unsafe { GetLastError().0 };
        if gle != ERROR_ACCESS_DENIED.0 {
            return Err(format!("SetProcessDPIAware failed (GLE={gle})"));
        }
    }
    // One-shot language detection (upstream `localization_init`, WinMain's
    // second call after `os_init`, viv.c:5158-5159). Before any window
    // exists so the very first title and status-bar text are in the right
    // language.
    loc::init();
    // COM for the shell folder picker (upstream WinMain's CoInitializeEx,
    // viv.c:5228-5229: apartment-threaded, OLE1DDE disabled — and the
    // return ignored there too; nothing else in riviv needs COM, so a
    // failure only degrades File→Open Folder).
    // SAFETY: process-wide init taking no inputs; the result is
    // deliberately unchecked like upstream.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
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
    // The settings, loaded before the window exists so the remembered
    // rect drives creation (upstream config_load_settings before
    // RegisterClassEx, viv.c:5261-5262 → 5354), the startup rect
    // computed from them while `config` is still owned here — and the
    // remembered maximized flag copied off NOW: the WM_SIZE tracking
    // below overwrites config.maximized with the live zoomed state from
    // the very first show (upstream saves it off before any window
    // exists for exactly this reason, viv.c:5264-5266).
    let config = Config::load();
    // The single-instance gate (#21; upstream viv.c:5278-5341 — config
    // first, viv.c:5261 → 5278, class registration after): with
    // multiple_instances off (the default), a second process hands its
    // command line to the window that owns the RIVIV mutex and exits
    // without ever creating a window. The mutex name and find-class are
    // deliberately not upstream's VOIDIMAGEVIEWER so both viewers run side
    // by side (README Differences). It runs BEFORE the decode worker
    // below: a forwarding launch must not die at a worker-spawn failure
    // before it could hand off (Codex PR #30 P2). The binding is
    // underscore-prefixed on purpose: its Drop at run's scope end is the
    // release.
    let _single_instance_mutex = if config.multiple_instances == 0 {
        // SAFETY: clears the thread's last error so ERROR_ALREADY_EXISTS
        // can only mean this CreateMutexA (upstream SetLastError(0),
        // viv.c:5281).
        unsafe { SetLastError(WIN32_ERROR(0)) };
        // SAFETY: a named-mutex create over a static NUL-terminated name;
        // the returned handle is valid even when the mutex already exists.
        // RAII (see `OwnedMutex`): every exit path — the handoff return,
        // the fail-loud `?`s below, or the pump's end — releases the mutex
        // exactly once.
        let mutex = copydata::OwnedMutex::new(
            unsafe { CreateMutexA(None, false, copydata::MUTEX_NAME) }
                .map_err(|e| format!("CreateMutexA failed: {e}"))?,
        );
        // SAFETY: reading the thread's last error immediately after the call.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // Find the owner's window and hand off (viv.c:5286-5334). No
            // window (the owner is mid-startup, before its class exists) is
            // upstream's accepted race: the handoff is lost and this
            // process still exits (viv.c:5336-5340) rather than show a
            // second window.
            // SAFETY: a pure top-level window search by static class name.
            let other = unsafe { FindWindowA(copydata::FIND_CLASS, PCSTR::null()) }
                .ok()
                .filter(|h| !h.is_invalid());
            if let Some(hwnd) = other {
                // Let this process hand over the foreground (viv.c:5302) —
                // the receiver foregrounds itself too; a lock denial is
                // ignored like upstream.
                // SAFETY: the found window is not ours but is live.
                let _ = unsafe { SetForegroundWindow(hwnd) };
                // The payload: this launch's ORIGINAL command line verbatim,
                // its cwd, and its effective show command (the launcher's
                // wShowWindow when STARTF_USESHOWWINDOW is set, else nCmdShow
                // — SW_SHOWNORMAL for a plain launch — viv.c:5303-5310).
                // SAFETY: GetCommandLineW returns this process's
                // NUL-terminated command line, valid for the process
                // lifetime; the walk reads up to that NUL.
                let cl = unsafe {
                    let cl = GetCommandLineW();
                    let mut n = 0usize;
                    while *cl.0.add(n) != 0 {
                        n += 1;
                    }
                    std::slice::from_raw_parts(cl.0, n)
                };
                let mut cwd_buf = [0u16; copydata::STRING_SIZE];
                // SAFETY: cwd_buf outlives the call; returns the length in
                // u16 units WITHOUT the NUL on success, or the REQUIRED size
                // WITH it when the buffer is too small. Upstream's
                // GetCurrentDirectory(STRING_SIZE, …) truncates identically;
                // a cwd longer than STRING_SIZE-1 is sent empty rather than
                // upstream's uninitialized stack (the receiver's
                // SetCurrentDirectory just fails either way).
                let cwd_len = unsafe { GetCurrentDirectoryW(Some(&mut cwd_buf)) } as usize;
                let cwd: &[u16] = if cwd_len < copydata::STRING_SIZE {
                    &cwd_buf[..cwd_len]
                } else {
                    &[]
                };
                let mut si = STARTUPINFOW {
                    cb: size_of::<STARTUPINFOW>() as u32,
                    ..Default::default()
                };
                // SAFETY: fills this process's STARTUPINFOW; a read-only query.
                unsafe { GetStartupInfoW(&mut si) };
                let show_cmd = if (si.dwFlags & STARTF_USESHOWWINDOW).0 != 0 {
                    u32::from(si.wShowWindow)
                } else {
                    SW_SHOWNORMAL.0 as u32
                };
                let payload = copydata::encode(show_cmd, cl, cwd);
                let cds = COPYDATASTRUCT {
                    dwData: copydata::COPYDATA_COMMAND_LINE,
                    cbData: payload.len() as u32,
                    // The receiver reads its copy during the synchronous
                    // send — the const is a lie the API demands (lpData is
                    // *mut), nothing writes through it.
                    lpData: payload.as_ptr() as *mut c_void,
                };
                // SAFETY: cds outlives the synchronous send; WM_COPYDATA
                // copies the payload into the receiver — the pointer is not
                // retained past the call (and must not be: SendMessage
                // blocks until the receiver returns, viv.c:5332).
                let _ = unsafe {
                    SendMessageW(
                        hwnd,
                        WM_COPYDATA,
                        None,
                        Some(LPARAM(&cds as *const COPYDATASTRUCT as isize)),
                    )
                };
            }
            // Exit without a window (viv.c:5336-5340); the OwnedMutex Drop
            // closes the handle on the way out (upstream _viv_kill,
            // viv.c:5528).
            return Ok(());
        }
        Some(mutex)
    } else {
        None // multiple_instances=1: no mutex is created at all (viv.c:5278)
    };
    // The decode worker, started once per process before any load is
    // requested (at most one decode is ever active — see `loadthread.rs`).
    // A spawn failure is system-level: fail loud (ADR 0001) via run's Err —
    // reached only by the instance that keeps the window.
    let load_thread = LoadThread::start()?;
    let show_maximized = config.maximized != 0;
    // The menu attaches at creation (upstream passes `_viv_hmenu` as
    // CreateWindowExW's menu param when config_show_menu, viv.c:5395-5400)
    // — copied off before the state owns the config, like show_maximized.
    let show_menu = config.show_menu != 0;
    let mut rect = initial_window_rect(&config)?;
    let state = WindowState {
        image: None,
        path: None,
        timer_freq,
        load_thread,
        session: None,
        displayed_from: None,
        config,
        animation_timer_running: false,
        // The status bar is created in WM_NCCREATE (the window handle must
        // exist first) and written into the state there.
        status: HWND::default(),
        menu: HMENU::default(),
        status_file_not_found: false,
        status_load_failed: false,
        displayed_file_bytes: None,
        pending_file_bytes: None,
        playlist: Playlist::new(),
        nav_current: None,
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
        last_cl_tick: None,
        last_open_folder: None,
    };

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

    // The startup window rect (kept from the load above — viv.c:5354-5387).
    let title = HSTRING::from_wide(&title_wide(None));
    let state_ptr = Box::into_raw(Box::new(state));

    // The menu bar (upstream `_viv_create_menu` before CreateWindowExW,
    // viv.c:5352): built once from the command table. A creation failure
    // degrades to a menu-less window — every menu call guards on the
    // invalid handle (the status-bar posture).
    let menu_bar = create_menu_bar();
    if menu_bar.is_invalid() {
        eprintln!("menu bar unavailable: CreateMenu failed");
    }

    let (rect_w, rect_h) = rect_size(rect);
    // SAFETY: all parameters are valid for the call; state_ptr ownership moves
    // into the window via WM_NCCREATE. If creation fails BEFORE WM_NCCREATE the
    // pointer leaks into the fatal-exit path (acceptable, ADR 0001); if it fails
    // after, WM_NCDESTROY already freed it. The size derivation wraps like C
    // (see `rect_size`). The menu param attaches the bar per config_show_menu
    // (upstream viv.c:5399).
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_ACCEPTFILES,
            CLASS_NAME,
            &title,
            WS_OVERLAPPEDWINDOW,
            rect.left,
            rect.top,
            rect_w,
            rect_h,
            None,
            if show_menu && !menu_bar.is_invalid() {
                Some(menu_bar)
            } else {
                None
            },
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

    // Hand the bar to the state for the View→Menu toggle (upstream keeps
    // `_viv_hmenu` in a global, viv.c:718 — the toggle re-attaches this
    // handle after a detach left GetMenu empty).
    // SAFETY: the borrow spans only the field store; nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.menu = menu_bar;
    }

    // Pull the startup rect fully onto a visible monitor and re-seat the
    // window (upstream viv.c:5404-5406, right after creation): a
    // remembered rect can sit off-screen after a monitor layout change,
    // and the first-run rect is monitor-relative (see
    // `initial_window_rect`). Fail-soft like upstream's unchecked
    // SetWindowPos there — the window is live either way.
    make_rect_completely_visible(hwnd, &mut rect);
    let (rect_w, rect_h) = rect_size(rect);
    // SAFETY: hwnd is live. SetWindowPos synchronously reenters wnd_proc
    // with WM_MOVE/WM_SIZE — both handlers take their own borrows, none is
    // live out here. The size derivation wraps like C (see `rect_size`).
    if let Err(e) = unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect_w,
            rect_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    } {
        eprintln!("startup SetWindowPos failed: {e}");
    }

    // The launcher's requested show state ("run maximized/minimized"
    // shortcuts) and the remembered maximized flag, in upstream order
    // (viv.c:5426-5451): anything non-normal shows FIRST, then the
    // remembered SW_MAXIMIZE, then the command line, then SW_SHOW for the
    // normal case. The no-flag default is SW_SHOWNORMAL — upstream falls
    // back to nCmdShow, which is SW_SHOWNORMAL for an ordinary launch, so
    // the early-show branch stays skipped and a remembered-maximized
    // launch never flashes its normal window first (Codex PR round 2).
    let mut si = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: fills this process's STARTUPINFOW; a read-only query.
    unsafe { GetStartupInfoW(&mut si) };
    let show_cmd = if (si.dwFlags & STARTF_USESHOWWINDOW).0 != 0 {
        SHOW_WINDOW_CMD(i32::from(si.wShowWindow))
    } else {
        SW_SHOWNORMAL
    };
    if show_cmd != SW_SHOWNORMAL {
        // SAFETY: hwnd is live; show per the launcher's request.
        let _ = unsafe { ShowWindow(hwnd, show_cmd) };
        // SAFETY: hwnd is live; paints now like upstream's UpdateWindow.
        let _ = unsafe { UpdateWindow(hwnd) };
    }
    // The remembered maximized state, copied off before the window/show
    // sequence could overwrite it through the WM_SIZE tracking (see the
    // config load above; upstream viv.c:5264-5266).
    if show_maximized {
        // SAFETY: hwnd is live.
        let _ = unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
    }

    // The startup command line (upstream viv.c:5445 — the very same
    // _viv_process_command_line the handoff receive re-runs, #21): the
    // first run through never takes add-mode (the tick starts unset), so
    // this is a plain replace/open.
    process_command_line(hwnd, &args, false);

    // If we did not show the window above, make sure it is shown now
    // (upstream viv.c:5444-5451).
    if show_cmd == SW_SHOWNORMAL {
        // SAFETY: hwnd is live.
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
        // SAFETY: hwnd is live.
        let _ = unsafe { UpdateWindow(hwnd) };
    }

    // Populate the status bar (parts + initial texts) now that the window
    // has its final rect — a hidden window's creation sends no WM_SIZE
    // (verified against windows-0.62), so the first one arrives with the
    // first ShowWindow above and the bar is still empty here. Upstream
    // populates it at startup the same way (viv.c:5415).
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
    // The OwnedMutex drops here with run's scope — the explicit mirror of
    // upstream's CloseHandle in _viv_kill (viv.c:5528), covering every
    // return path (the fail-loud `?`s above included, Codex PR #30 P1).
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_handoff_appends_when_something_is_loaded() {
        // The Explorer multi-select burst: the 2nd..Nth command lines
        // arrive within the timeout of the previous one and append
        // (viv.c:4782-4792's three nested gates, all true).
        assert!(handoff_add_mode(1_500, Some(1_100), 500, true)); // 400 < 500
    }

    #[test]
    fn expired_handoff_replaces() {
        // A deliberate later launch replaces the display/playlist.
        assert!(!handoff_add_mode(1_601, Some(1_100), 500, true)); // 501 >= 500
    }

    #[test]
    fn handoff_at_the_exact_timeout_replaces() {
        // The comparison is strict `<` (viv.c:4784) — delta == timeout is
        // already a replace.
        assert!(!handoff_add_mode(1_600, Some(1_100), 500, true));
    }

    #[test]
    fn wrapped_tick_delta_still_measures_rapid() {
        // GetTickCount wraps at 2^32: the C subtraction is DWORD
        // arithmetic, so a delta across the wrap counts normally.
        let last = u32::MAX - 100; // 499 ms before the wrap point
        assert!(handoff_add_mode(300, Some(last), 500, true)); // 401 ms
        assert!(!handoff_add_mode(700, Some(last), 500, true)); // 801 ms
    }

    #[test]
    fn disabled_timeout_never_appends() {
        // add_command_line_timeout = 0 disables the whole gate (the first
        // condition upstream reads, viv.c:4781).
        assert!(!handoff_add_mode(1_100, Some(1_099), 0, true));
    }

    #[test]
    fn first_command_line_never_appends() {
        // No previous stamp (upstream's got_last_process_command_line_tick
        // gate, viv.c:4782) — the startup open is always a replace.
        assert!(!handoff_add_mode(1_000, None, 500, true));
    }

    #[test]
    fn blank_viewer_replaces_even_when_rapid() {
        // `*_viv_current_fd->cFileName` empty — never append onto a blank
        // viewer; the handoff must open something (viv.c:4785-4791).
        assert!(!handoff_add_mode(1_500, Some(1_100), 500, false));
    }

    #[test]
    fn negative_timeout_behaves_like_the_c_dword_cast() {
        // A hand-edited negative ini value reaches C as a huge DWORD —
        // effectively "always rapid"; riviv's `as u32` mirrors the cast.
        assert!(handoff_add_mode(1_000, Some(999), -1, true));
    }

    #[test]
    fn first_run_is_three_fifths_of_the_monitor_centered_in_its_frame() {
        // 1920x1080 primary at the origin: 60% = 1152x648, centered in
        // the FULL monitor rect (rcMonitor, taskbar included — the os_
        // wrapper's flag 1) — and in the monitor's own frame: the origin
        // is never added (viv.c:5383-5386, the no-origin quirk).
        let r = first_run_window_rect(
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            3,
            5,
            3,
            5,
        );
        assert_eq!((r.right - r.left, r.bottom - r.top), (1152, 648));
        assert_eq!((r.left, r.top), ((1920 - 1152) / 2, (1080 - 648) / 2));

        // A secondary monitor keeps its own relative frame too — verbatim
        // upstream, which would land this rect near the primary's origin
        // (see initial_window_rect's doc).
        let r = first_run_window_rect(
            RECT {
                left: 1920,
                top: 0,
                right: 3840,
                bottom: 1080,
            },
            3,
            5,
            3,
            5,
        );
        assert_eq!((r.left, r.top), (384, 216), "origin never added");

        // div = 0 falls back to 640x480 (viv.c:5367-5368).
        let r = first_run_window_rect(
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            3,
            0,
            3,
            0,
        );
        assert_eq!((r.right - r.left, r.bottom - r.top), (640, 480));
    }

    #[test]
    fn pathological_ini_values_wrap_instead_of_panicking() {
        // A hand-edited ini can hold any i32 (parse_int mirrors
        // utf8_to_int's silent wrap); the C build wraps where Rust would
        // panic (debug overflow; `i32::MIN / -1` even in release). The
        // make-visible core runs on the remembered rect too — both paths
        // must survive the same garbage.
        let mon = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let _ = first_run_window_rect(mon, i32::MAX, 1, i32::MAX, 1);
        let _ = first_run_window_rect(mon, i32::MIN, -1, i32::MIN, -1);
        let _ = first_run_window_rect(mon, -i32::MAX, 1, i32::MIN, 2);
        let garbage = RECT {
            left: i32::MAX,
            top: i32::MIN,
            right: i32::MIN,
            bottom: i32::MAX,
        };
        let _ = make_rect_completely_visible_core(garbage, mon, mon);
        let _ = make_rect_completely_visible_core(garbage, garbage, mon);
    }

    #[test]
    fn pathological_remembered_rect_size_wraps_like_c() {
        // x=i32::MAX, wide=1 wraps right to i32::MIN (initial_window_rect);
        // C computes the size as MIN - MAX, which wraps back to 1 — exactly
        // the value CreateWindowEx receives and rejects gracefully. The
        // call sites feed through this helper, so the derivation is pinned
        // (a plain subtraction here panics the debug build, Codex PR #28).
        let rect = RECT {
            left: i32::MAX,
            top: i32::MAX,
            right: i32::MIN,
            bottom: i32::MIN,
        };
        assert_eq!(rect_size(rect), (1, 1));
    }

    #[test]
    fn make_visible_reanchors_between_monitors_and_pushes_each_side_in() {
        let target = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        };
        // Source monitor elsewhere: the rect's position within its own
        // monitor is preserved onto the target monitor (os.c:205-206).
        let source = RECT {
            left: 2000,
            top: 100,
            right: 3000,
            bottom: 900,
        };
        let r = make_rect_completely_visible_core(
            RECT {
                left: 2100,
                top: 200,
                right: 2500,
                bottom: 600,
            },
            target,
            source,
        );
        assert_eq!(
            r,
            RECT {
                left: 100,
                top: 100,
                right: 500,
                bottom: 500
            }
        );

        // Sticking out on every side gets pushed fully into view, and an
        // oversized rect is capped to the monitor (os.c:212-233).
        let r = make_rect_completely_visible_core(
            RECT {
                left: -500,
                top: -500,
                right: 2000,
                bottom: 2000,
            },
            target,
            source,
        );
        assert_eq!(r, target);

        // Already-visible rect: only the re-anchor applies.
        let r = make_rect_completely_visible_core(
            RECT {
                left: 2100,
                top: 200,
                right: 2300,
                bottom: 400,
            },
            source,
            source,
        );
        assert_eq!(
            r,
            RECT {
                left: 2100,
                top: 200,
                right: 2300,
                bottom: 400
            }
        );
    }
}
