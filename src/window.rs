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
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HMODULE, HWND, LPARAM, LRESULT, POINT,
    RECT, SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, GetMonitorInfoW, GetPixel, HBRUSH, InvalidateRect, MONITOR_DEFAULTTOPRIMARY,
    MONITORINFO, MonitorFromPoint, MonitorFromRect, MonitorFromWindow, PtInRect, ScreenToClient,
    UpdateWindow,
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
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_ENABLESIZING, OFN_FILEMUSTEXIST,
    OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, NM_CLICK, NMHDR, NMMOUSE, SB_GETPARTS, WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyNameTextW, GetKeyState, GetKeyboardLayout, MAPVK_VK_TO_VSC, MapVirtualKeyExW,
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_ESCAPE,
    VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    DragFinish, DragQueryFileW, FILEOPENDIALOGOPTIONS, FOS_NOCHANGEDIR, FOS_PICKFOLDERS,
    FileOpenDialog, HDROP, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, AdjustWindowRectEx, AppendMenuW, CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW,
    CS_VREDRAW, CheckMenuItem, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DestroyWindow, DispatchMessageW, EnableMenuItem, FindWindowA, GWL_EXSTYLE,
    GWL_STYLE, GWLP_USERDATA, GetClientRect, GetCursorPos, GetForegroundWindow, GetMenu,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, HICON, HMENU, HTCAPTION,
    HTMENU, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, IDC_ARROW, IMAGE_ICON, IsIconic, IsZoomed,
    KillTimer, LR_DEFAULTCOLOR, LoadCursorW, LoadImageW, MB_ICONERROR, MB_ICONQUESTION, MB_OK,
    MENU_ITEM_FLAGS, MF_BYCOMMAND, MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED, MFT_RADIOCHECK, MINMAXINFO, MSG, MenuItemFromPoint, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassExW, SC_MONITORPOWER, SHOW_WINDOW_CMD, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SW_MAXIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SYSTEM_METRICS_INDEX, SendMessageW, SetCursorPos, SetForegroundWindow, SetMenu,
    SetProcessDPIAware, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowCursor,
    ShowWindow, TPM_CENTERALIGN, TPM_LEFTBUTTON, TPM_VCENTERALIGN, TrackPopupMenu,
    TranslateMessage, USER_TIMER_MINIMUM, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_COMMAND,
    WM_CONTEXTMENU, WM_COPYDATA, WM_DESTROY, WM_DROPFILES, WM_ENDSESSION, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_INITMENU, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_MOVE, WM_NCCREATE, WM_NCDESTROY,
    WM_NCLBUTTONDOWN, WM_NCXBUTTONDBLCLK, WM_NCXBUTTONDOWN, WM_NOTIFY, WM_NULL, WM_PAINT, WM_PASTE,
    WM_QUERYENDSESSION, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WM_SYSCOMMAND,
    WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDBLCLK, WM_XBUTTONDOWN, WNDCLASSEXW, WS_CAPTION,
    WS_EX_ACCEPTFILES, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
    WindowFromPoint,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, PWSTR, w};

use crate::anim::{ANIMATION_TIMER_ID, RATE_ONE, rate_step};
use crate::cli;
use crate::clipboard;
use crate::config::Config;
use crate::copydata;
use crate::cursor::{self, CursorEffects, CursorVisibility};
use crate::custom_rate_dlg;
use crate::everything;
use crate::loader::{LoadReply, LoadedImage, UiAction, apply_reply, map_reply_frame};
use crate::loadthread::{LoadSession, LoadThread, REPLY_KICK_MESSAGE};
use crate::loc;
use crate::menu;
use crate::paint::paint;
use crate::playlist::{self, Playlist, PlaylistEntry};
use crate::preload::{self, AdoptDecision, LastCache, PreloadSlot, PreloadState};
use crate::shell;
use crate::slideshow;
use crate::status;
use crate::surface::{DibFrame, Surface};
use crate::text::{
    TitleFormat, dialog_filter, temp_animation_rate_text, temp_pos_zoom_text,
    temp_slideshow_rate_text, title_wide, to_wide,
};
use crate::zoom::{FitPolicy, View, Viewport};
use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

/// Window class name. Deliberately different from upstream's `VOIDIMAGEVIEWER`
/// (class + mutex) so both viewers can coexist on one machine.
const CLASS_NAME: PCWSTR = w!("riviv");

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
    /// The toolbar set (#45; upstream `_viv_rebar_hwnd` /
    /// `_viv_toolbar_hwnd` / `_viv_toolbar_image_list`, viv.c:662-664):
    /// strip + toolbar + image list, created per `config_show_controls`,
    /// destroyed on fullscreen entry and recreated on exit (viv.c:6646/
    /// 6681). Default = absent (creation failed / hidden) — every
    /// consumer guards on it.
    pub(crate) controls: crate::toolbar::ControlsSet,
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
    /// In-progress middle-drag scroll (#44; upstream `_viv_doing ==
    /// _VIV_DOING_MSCROLL` + `_viv_mdoing_x/y`, viv.c:3329-3345): the
    /// SCREEN anchor the cursor is re-pinned to — the scroll delta each
    /// move is anchor minus the live cursor position. `None` = not
    /// middle-dragging.
    pub(crate) mscroll: Option<POINT>,
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
    /// A context menu is tracking (upstream `_viv_in_popup_menu`,
    /// viv.c:716/3534-3539): set across TrackPopupMenu so the WM_TIMER
    /// hide-cursor path sees it (the cursor never hides mid-menu).
    pub(crate) in_popup_menu: bool,
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
    /// The source-pixel coordinate under the cursor (#47; upstream
    /// `_viv_src_pixel_x/y`, viv.c:786-787): (-1, -1) while pixel-info is
    /// off or the cursor is off-image; the POS part shows only for a valid
    /// pair. Resampled on mouse moves and every frame change (the RGB
    /// under a fixed point moves with the frame).
    pub(crate) src_pixel: (i32, i32),
    /// The color sampled at the last valid `src_pixel` (#47; upstream
    /// `_viv_src_pixel_r/g/b`, viv.c:788-790) — upstream leaves stale
    /// values when the coordinate goes invalid (its else-path reads an
    /// uninitialized COLORREF); the pair is unobservable there because the
    /// part empties with the coordinate.
    pub(crate) src_rgb: (u8, u8, u8),
    /// The 3-second status flash text (#47; upstream
    /// `_viv_status_temp_text`, viv.c:735): `Some` while a
    /// panscan/zoom-rate/slideshow-rate readout is showing — it outranks
    /// every main-part verdict until `status::TEMP_TEXT_TIMER_ID` clears
    /// it (viv.c:3135-3137).
    pub(crate) status_temp: Option<String>,
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
    /// Everything-search random mode (#22; upstream `_viv_random` +
    /// `_viv_random_tot_results` + CRT rand, viv.c:749-751): the armed
    /// search term, the result total (0xFFFFFFFF until a reply narrows
    /// it), the stored QUERY2 request flags the reply parse walks
    /// (upstream `_viv_everything_request_flags`, viv.c:751 — stored at
    /// send, never read from the reply echo), and the MSVC-LCG state the
    /// random offsets draw from.
    pub(crate) random_search: Option<Vec<u16>>,
    pub(crate) random_tot_results: u32,
    pub(crate) everything_request_flags: u32,
    pub(crate) random_rand_state: u32,
    /// A slideshow is running (#37; upstream `_viv_is_slideshow`,
    /// viv.c:708) — the timer-advance state, the status bar's playing
    /// line, and both menu checks read it.
    pub(crate) slideshow: bool,
    /// The slideshow timer expired but the current animation has not
    /// looped once yet (upstream `_viv_is_slideshow_timeup`, viv.c:694) —
    /// the held advance fires from the animation's wrap (viv.c:3243-3248).
    pub(crate) slideshow_timeup: bool,
    /// The displayed animation has completed at least one full loop
    /// (upstream `_viv_frame_looped`, viv.c:693 — reset with every new
    /// image like `_viv_clear`, viv.c:1278).
    pub(crate) animation_looped: bool,
    /// The animation plays or is paused (#38; upstream `_viv_animation_
    /// play`, viv.c:673 — default 1, reset per image at `_viv_clear`,
    /// viv.c:1291; Frame Step/Previous/First/Last pause it, Play/Pause
    /// toggles it, the jump commands leave it alone). Read by the timer
    /// gate, the menu check (viv.c:7184) and the prevent-sleep decision
    /// (viv.c:3929-3936).
    pub(crate) animation_playing: bool,
    /// The animation rate table position (#38; upstream
    /// `_viv_animation_rate_pos`, viv.c:672): index into
    /// [`crate::anim::RATE_TABLE`], 10 = 1.0×. Unlike the pause flag it
    /// PERSISTS across images (upstream only the rate commands move it).
    pub(crate) animation_rate_pos: usize,
    /// The prevent-sleep execution-state latch (upstream
    /// `_viv_is_prevent_sleep`, viv.c:789) — SetThreadExecutionState is
    /// only called on a CHANGE of the derived want (viv.c:7319-7347).
    pub(crate) prevent_sleep_active: bool,
    /// The parked next-image preload (#40; upstream `_viv_preload_*`
    /// globals, viv.c:756-764). `None` while idle; a foreground open or a
    /// sort/shuffle cache clear drops it.
    pub(crate) preload: Option<PreloadSlot>,
    /// The previous display parked for instant navigation back (#40;
    /// upstream `_viv_last_fd`/`_viv_last_frames`, one slot).
    pub(crate) last_cache: Option<LastCache>,
    /// The direction of the last MANUAL navigation — the preload follows
    /// it (upstream `_viv_last_is_prev`, viv.c:762: recorded by
    /// `_viv_next` for non-preload calls only, and `_viv_preload_next`
    /// navigates with it, viv.c:14308).
    pub(crate) last_nav_prev: bool,
    /// The navigation entry of the image currently displayed (upstream
    /// `_viv_frame_fd`, "may differ to the current fd because we change
    /// the title before the frames are loaded", viv.c:770 — set at the
    /// first-frame reply, viv.c:2962): what
    /// `viv_copy_current_image_to_last_image` caches (viv.c:14441).
    pub(crate) displayed_entry: Option<PlaylistEntry>,
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
        // The main part's Loading also covers a preload flagged for
        // promotion: the user navigated onto its file and the load is now
        // destined for the display (upstream's should_activate clause in
        // the Loading gate, viv.c:11356).
        loading: state.session.is_some()
            || state.preload.as_ref().is_some_and(|s| s.activate_on_load),
        file_not_found: state.status_file_not_found,
        load_failed: state.status_load_failed,
        slideshow: state.slideshow,
        // The flash text outranks the whole verdict chain (upstream
        // viv.c:11351-11353).
        temp_text: state.status_temp.clone(),
        // The preload part while a preload decodes its first frame (upstream
        // viv.c:11210-11214, #40).
        preload_pending: state.preload.as_ref().is_some_and(|s| {
            preload::indicator_visible(s.state, s.image.is_some(), s.activate_on_load)
        }),
        // The POS/RGB parts show text only for a valid coordinate pair
        // (upstream's x/y >= 0 gate, viv.c:11217-11219).
        pixel: (state.src_pixel.0 >= 0 && state.src_pixel.1 >= 0).then_some(state.src_pixel),
        pixel_rgb: state.src_rgb,
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

/// Rebuild the caption from the current path and `title_bar_format`
/// (#47; upstream `_viv_update_title`, called whenever the format or the
/// path changes). The SetWindowTextW runs OUTSIDE the state borrow like
/// every title update here.
pub(crate) fn refresh_title(hwnd: HWND) {
    // SAFETY: the borrow spans only the path/format read for the title.
    let title = (unsafe { state_of(hwnd) }).map(|state| {
        (HSTRING::from_wide(&title_wide(
            state.path.as_deref(),
            TitleFormat::from_config(state.config.title_bar_format),
        )),)
    });
    if let Some((title,)) = title {
        // SAFETY: hwnd is live; the HSTRING outlives the call. Fail-soft
        // like every other title update (upstream viv.c:1249 ignores the
        // SetWindowTextW return too).
        let _ = unsafe { SetWindowTextW(hwnd, &title) };
    }
}

/// Sample the source pixel under the cursor (#47; upstream
/// `_viv_update_src_pixel`, viv.c:9173-9240). Returns whether the status
/// bar must refresh — upstream performs the refresh inside the same call
/// only when pixel-info is on AND (forced OR the coordinate moved);
/// `update_statusbar=false` leaves the refresh to the caller's explicit
/// status update (every frame-change site pairs the two, viv.c:1901-1902/
/// 9277-9278/14325-14326). With pixel-info off the coordinate resets and
/// no refresh is owed (the parts sit empty either way).
fn update_src_pixel(hwnd: HWND, force: bool, update_statusbar: bool) -> bool {
    // SAFETY: the borrow spans the coordinate math and the read-only
    // GetPixel on the displayed frame's DC — neither pumps messages, so no
    // reentrant state_of borrow can interleave.
    (unsafe { state_of(hwnd) }).is_some_and(|state| {
        if state.config.pixel_info == 0 {
            // Upstream's else-arm resets the coordinate only; the stale rgb
            // stays (unobservable — the POS part empties with it).
            state.src_pixel = (-1, -1);
            return false;
        }
        let mut screen = POINT::default();
        // SAFETY: read-only cursor query; fail-soft leaves (0, 0), which
        // reads as a coordinate change at most once.
        let _ = unsafe { GetCursorPos(&mut screen) };
        let mut client = screen;
        // SAFETY: screen -> client of our own window; pumps nothing.
        let _ = unsafe { ScreenToClient(hwnd, &mut client) };
        // The render rect exactly as paint anchors it (upstream
        // `_viv_get_src_pixel_pos`, viv.c:15020-15057): the panscan-scaled
        // render size, centered by the pan term minus the drag offset,
        // against the viewport (client minus status bar and controls).
        let (vp, src) = viewport_and_src(hwnd, state);
        // Upstream gates the whole walk on a frame being displayed
        // (viv.c:15030's `_viv_frame_count` check).
        let new_pt = if state.image.is_some() {
            (|| {
                if vp.wide <= 0 || vp.high <= 0 || src.0 <= 0 || src.1 <= 0 {
                    return None;
                }
                let fit = fit_policy(state);
                let (rw0, rh0) = state.view.render_size(src.0, src.1, vp, fit);
                let rw = crate::panscan::scale(rw0, state.view.panscan.zoom_x);
                let rh = crate::panscan::scale(rh0, state.view.panscan.zoom_y);
                if rw <= 0 || rh <= 0 {
                    return None;
                }
                let rx = crate::panscan::center_term(vp.wide, state.view.panscan.pos_x)
                    - rw / 2
                    - state.view.view_x;
                let ry = crate::panscan::center_term(vp.high, state.view.panscan.pos_y)
                    - rh / 2
                    - state.view.view_y;
                (client.x >= rx && client.y >= ry && client.x < rx + rw && client.y < ry + rh).then(
                    || {
                        (
                            (((client.x - rx) as i64) * src.0 as i64 / rw as i64) as i32,
                            (((client.y - ry) as i64) * src.1 as i64 / rh as i64) as i32,
                        )
                    },
                )
            })()
        } else {
            None
        }
        .unwrap_or((-1, -1));
        let changed = force || new_pt != state.src_pixel;
        if !changed {
            return false;
        }
        state.src_pixel = new_pt;
        if new_pt.0 >= 0
            && new_pt.1 >= 0
            && let Some(image) = state.image.as_ref()
        {
            // The displayed frame's own DC, read-only (upstream builds a
            // scratch DC for the same GetPixel, viv.c:15063-15109; riviv's
            // persistent mem DC serves the identical read — the #41
            // clipboard blit set the precedent).
            let dc = image.surface().mem_dc();
            // SAFETY: read-only GetPixel on the surface's selected frame;
            // no selection change, no messages. A failed read returns
            // CLR_INVALID (0xFFFFFFFF → 255,255,255) unchecked, like
            // upstream's GetRValue chain (viv.c:9212-9214).
            let cref = unsafe { GetPixel(dc, new_pt.0, new_pt.1) };
            // COLORREF is 0x00BBGGRR (the GetRValue/GValue/BValue macros).
            state.src_rgb = (
                (cref.0 & 0xFF) as u8,
                ((cref.0 >> 8) & 0xFF) as u8,
                ((cref.0 >> 16) & 0xFF) as u8,
            );
        }
        update_statusbar
    })
}

/// The force-resample + refresh pair every frame-change site performs
/// (upstream `_viv_update_src_pixel(1,0)` + the explicit
/// `_viv_status_update()`, viv.c:1901-1902/9277-9278/10097-10098/
/// 14325-14326): the color under a fixed screen point moves with the
/// frame, so stepping/jumping/advancing/adopting resamples unconditionally.
fn resample_pixel_refresh(hwnd: HWND) {
    update_src_pixel(hwnd, true, false);
    refresh_status(hwnd);
}

/// Land a 3-second status flash (#47; upstream `_viv_status_set_temp_text`,
/// viv.c:11737-11758): a showing text's timer dies first, the text replaces
/// whatever was there, the bar refreshes, and a fresh timer arms only when
/// a text landed. `None` clears (the timer's own expiry path).
fn status_set_temp_text(hwnd: HWND, text: Option<String>) {
    // SAFETY: the borrow spans the swap only.
    let had_text = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let had = state.status_temp.is_some();
        state.status_temp = text;
        had
    });
    if had_text {
        // SAFETY: hwnd is live; a failed kill leaves a stale timer whose
        // WM_TIMER handler no-ops on the already-cleared text.
        let _ = unsafe { KillTimer(Some(hwnd), status::TEMP_TEXT_TIMER_ID) };
    }
    refresh_status(hwnd);
    // SAFETY: read-only probe.
    let showing = (unsafe { state_of(hwnd) }).is_some_and(|s| s.status_temp.is_some());
    if showing {
        // SAFETY: hwnd is live and owned by this thread. Fail-soft like
        // upstream's unchecked SetTimer (viv.c:11757): a failed timer only
        // costs the auto-fallback (the text stays until the next verdict).
        let _ = unsafe { SetTimer(Some(hwnd), status::TEMP_TEXT_TIMER_ID, 3000, None) };
    }
}

/// The panscan/zoom flash (`_viv_status_update_temp_pos_zoom` fires from
/// `_viv_dst_pos_set`/`_viv_dst_zoom_set` tails, viv.c:9996/10029 —
/// UNCONDITIONALLY, a clamped no-op step still flashes).
fn flash_pos_zoom(hwnd: HWND) {
    // SAFETY: the borrow spans the read of the panscan state and image
    // dims; the pure text builder runs on the copies.
    let text = (unsafe { state_of(hwnd) }).map(|state| {
        let (w, h) = state
            .image
            .as_ref()
            .map(|i| (i.width(), i.height()))
            .unwrap_or((0, 0));
        temp_pos_zoom_text(
            state.view.panscan.pos_x,
            state.view.panscan.pos_y,
            state.view.panscan.zoom_x,
            state.view.panscan.zoom_y,
            w,
            h,
        )
    });
    if let Some(text) = text {
        status_set_temp_text(hwnd, Some(text));
    }
}

/// The animation-rate flash (`_viv_increase_animation_rate`/
/// `_viv_reset_animation_rate` tails, viv.c:7673/7680).
fn flash_animation_rate(hwnd: HWND) {
    // SAFETY: the borrow spans the rate-position read.
    let text = (unsafe { state_of(hwnd) })
        .map(|s| temp_animation_rate_text(crate::anim::RATE_TABLE[s.animation_rate_pos]));
    if let Some(text) = text {
        status_set_temp_text(hwnd, Some(text));
    }
}

/// The slideshow-rate flash (`_viv_set_rate` tail and the custom-rate path,
/// viv.c:7053/7068).
fn flash_slideshow_rate(hwnd: HWND) {
    // SAFETY: the borrow spans the config read.
    let text =
        (unsafe { state_of(hwnd) }).map(|s| temp_slideshow_rate_text(s.config.slideshow_rate));
    if let Some(text) = text {
        status_set_temp_text(hwnd, Some(text));
    }
}

/// The status bar's NM_CLICK (#47; upstream WM_NOTIFY arm, viv.c:3976-4010):
/// a click that hit nothing (the size grip, or past the last measured part)
/// resolves to the LAST part; part 1 toggles `config_frame_minus`. With the
/// layout [main][preload?][pos][rgb][frame][dimension], part 1 is the
/// PRELOAD slot while its text shows, else the POS part — zero-width when
/// pixel-info is off, making the toggle unclickable in that state.
/// Upstream hardcodes the index; kept bug-for-bug.
fn on_status_nm_click(hwnd: HWND, nm: &NMMOUSE) {
    let bar = snapshot_status_bar(hwnd);
    if bar.is_invalid() {
        return;
    }
    let mut item = nm.dwItemSpec as isize;
    if item < 0 {
        // "if we hit nothing use the last part" (viv.c:3989-3992).
        // SAFETY: count-only SB_GETPARTS on our own child (no buffer).
        item = unsafe { SendMessageW(bar, SB_GETPARTS, Some(WPARAM(0)), None) }.0 as isize - 1;
    }
    if item == 1 {
        // SAFETY: the borrow spans the one-field flip.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.config.frame_minus ^= 1;
        }
        refresh_status(hwnd);
    }
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
    // The strip rides above the status bar (upstream subtracts BOTH from
    // the render area, viv.c:13956-13959/1621-1634) — a hidden/absent
    // strip reports 0 (viv.c:11448-11450).
    let controls_h = state.controls.height();
    let vp = Viewport {
        wide: (client.right - client.left).max(0),
        high: (client.bottom - client.top - status_h - controls_h).max(0),
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
        in_popup_menu: state.in_popup_menu,
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
/// the movement dedupe, make sure the cursor is visible (the hide
/// conditions can no longer hold), and drop the pixel coordinate (the
/// POS/RGB parts empty — the refresh only when pixel-info is on and a
/// coordinate was showing, upstream's conditional `_viv_status_update`,
/// viv.c:3570-3579).
fn on_mouse_leave(hwnd: HWND) {
    // SAFETY: the borrow spans the flag resets, the coordinate clear, and
    // the pure cursor step.
    let (effects, had_pixel) = (unsafe { state_of(hwnd) })
        .map(|state| {
            state.tracking_mouse = false;
            state.is_mouseover = false;
            state.last_cursor_pt = POINT { x: -1, y: -1 };
            let had = state.src_pixel != (-1, -1);
            state.src_pixel = (-1, -1);
            (state.cursor.show(), had)
        })
        .unwrap_or((CursorEffects::default(), false));
    apply_cursor(hwnd, effects);
    if had_pixel {
        refresh_status(hwnd);
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
        // 3/4 = zoom in / next image: the default arm re-runs the click
        // action (upstream viv.c:3313-3326).
        3 => zoom_at(hwnd, false, (pt.x, pt.y)),
        4 => {
            nav_next(hwnd, false, true, false);
        }
        // 0/1/2/5/6 (scroll, slideshow, animation, 1:1 scroll, move
        // window): the double-click toggles FULLSCREEN — upstream's arm
        // switches on exactly these values and never re-runs the action,
        // so action 1's second click presents the run the first click
        // toggled (cubic round 1); unknown values re-run the action,
        // which does nothing.
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
                nav_next(hwnd, true, true, false);
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
        // The bar is RECREATED first — per config (#46; upstream
        // `_viv_status_show(config_show_status)` at 6645 precedes the
        // style commit at 6648; it destroys and recreates rather than
        // hiding, viv.c:10932-10963).
        // SAFETY: read-only config read, then the creation pass.
        if (unsafe { state_of(hwnd) }).is_some_and(|state| state.config.show_status != 0) {
            // SAFETY: returns this exe's module handle; no side effects.
            if let Ok(hinstance) = unsafe { GetModuleHandleW(None) } {
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
        }
        // The strip is recreated right behind the bar, per config (upstream
        // `_viv_controls_show(config_show_controls)` at 6646, second in its
        // exit order). Its own on_size docks it against the restore pass
        // below.
        // SAFETY: read-only config read, then the creation pass.
        if (unsafe { state_of(hwnd) }).is_some_and(|state| state.config.show_controls != 0) {
            controls_show(hwnd, true);
        }
        // SAFETY: read-modify-write of the style on the owning thread.
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        // The caption/frame bits come back per config (#46; upstream
        // rebuilds the style from `config_show_caption`/`thickframe` here
        // — its defaults are both on, config.c:85-86).
        // SAFETY: the borrow spans only the two config reads.
        let (show_caption, show_thickframe) =
            (unsafe { state_of(hwnd) }).map_or((true, true), |state| {
                (
                    state.config.show_caption != 0,
                    state.config.show_thickframe != 0,
                )
            });
        let mut restored = style;
        if show_caption {
            restored |= WS_CAPTION.0 | WS_SYSMENU.0;
        } else {
            restored &= !(WS_CAPTION.0 | WS_SYSMENU.0);
        }
        if show_thickframe {
            restored |= WS_THICKFRAME.0;
        } else {
            restored &= !WS_THICKFRAME.0;
        }
        // SAFETY: hwnd is live.
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, restored as isize) };
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
        // The toolbar strip leaves too (upstream `_viv_controls_show(0)`
        // right after the menu, viv.c:6681 — the monitor goes to the
        // image). Taken out of the state inside a borrow, torn down
        // outside it like the bar above (DestroyWindow delivers messages).
        {
            // SAFETY: the borrow spans only the set take.
            let set = (unsafe { state_of(hwnd) }).map(|state| std::mem::take(&mut state.controls));
            if let Some(mut set) = set {
                crate::toolbar::destroy(&mut set);
            }
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
            let title = HSTRING::from_wide(&title_wide(None, TitleFormat::FilenameOnly));
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
                nav_next(hwnd, true, true, false);
            } else if delta < 0 {
                nav_next(hwnd, false, true, false);
            }
        }
        // Action 2: wheel up = next, down = previous (viv.c:14075-14086).
        2 => {
            if delta > 0 {
                nav_next(hwnd, false, true, false);
            } else if delta < 0 {
                nav_next(hwnd, true, true, false);
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
        // Upstream's `_viv_zoom_in` ends in `_viv_view_set`, which closes
        // with the toolbar refresh (viv.c:6571).
        refresh_toolbar(hwnd);
    }
}

/// One Pan/Scan size/width/height step (#44; upstream
/// `VIV_ID_VIEW_PANSCAN_{INCREASE,DECREASE}_{SIZE,WIDTH,HEIGHT}` →
/// `_viv_dst_zoom_set`, viv.c:2246-2268): the clamped index pair, repaint
/// only when an index actually moved.
fn panscan_step(hwnd: HWND, dx: i32, dy: i32) {
    // SAFETY: the borrow spans only the pure step.
    let changed = (unsafe { state_of(hwnd) }).is_some_and(|state| state.view.panscan.step(dx, dy));
    if changed {
        repaint(hwnd);
    }
    // The readout flashes even when the step clamped to a no-op (upstream's
    // `_viv_dst_zoom_set` calls the temp text outside the changed check,
    // viv.c:10029).
    flash_pos_zoom(hwnd);
}

/// One Pan/Scan Move arrow (#44; upstream the eight MOVE commands →
/// `_viv_dst_pos_set(x ± 5, y ± 5)`, viv.c:2270-2299): clamped, repaint
/// only on a real position change.
fn panscan_pan(hwnd: HWND, dx: i32, dy: i32) {
    // SAFETY: the borrow spans only the pure pan.
    let changed = (unsafe { state_of(hwnd) }).is_some_and(|state| state.view.panscan.pan(dx, dy));
    if changed {
        repaint(hwnd);
    }
    // Unconditional flash (upstream `_viv_dst_pos_set` tail, viv.c:9996).
    flash_pos_zoom(hwnd);
}

/// Move Center / Pan-Scan Reset (#44; upstream viv.c:2302-2313): both write
/// the position directly and invalidate UNCONDITIONALLY (no change check),
/// Reset additionally restores the identity factor indices. Both flash the
/// readout (Center directly, Reset through `_viv_dst_zoom_set`'s tail).
fn panscan_center(hwnd: HWND) {
    // SAFETY: the borrow spans only the pure state write.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.view.panscan.center();
    }
    repaint(hwnd);
    flash_pos_zoom(hwnd);
}

fn panscan_reset(hwnd: HWND) {
    // SAFETY: the borrow spans only the pure state write.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.view.panscan.reset();
    }
    repaint(hwnd);
    flash_pos_zoom(hwnd);
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
            state.controls.height(),
        )
    });
    let Some((fullscreen, image, auto_fit, has_menu, status_h, controls_h)) = gathered else {
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
    // The outer rect for client + status bar + toolbar strip (upstream
    // viv.c:2139-2143/2155: AdjustWindowRect over the client target, then
    // + both heights).
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: client.0,
        bottom: client.1 + status_h + controls_h,
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
    // The Best Fit gray flips here (upstream's ZOOM_RESET ends in
    // `_viv_view_set` → the toolbar refresh, viv.c:6571).
    refresh_toolbar(hwnd);
}

/// The three View fit rows (#46; upstream viv.c:2015-2044): which config a
/// row's toggle lands on.
enum FitInput {
    AllowShrinking,
    KeepAspect,
    FillWindow,
}

/// One View fit-row command (upstream viv.c:2015-2044): 1:1 dies first
/// (`_viv_1to1 = 0` — the toggle's view math re-derives from the fit
/// level), the config flips, then the same size pass + repaint a resize
/// does re-anchors the render at the new fit inputs. The Fill row reads
/// the CURRENT mode: fullscreen toggles `fullscreen_fill_window`,
/// windowed `fill_window` (upstream's own branch, viv.c:2033-2044).
fn toggle_fit_input(hwnd: HWND, input: FitInput) {
    // SAFETY: the borrow spans the 1:1 clear, the config flip and the
    // mode read — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.view.leave_one_to_one();
        let fullscreen = state.fullscreen;
        match input {
            FitInput::AllowShrinking => {
                state.config.allow_shrinking = i32::from(state.config.allow_shrinking == 0);
            }
            FitInput::KeepAspect => {
                state.config.keep_aspect_ratio = i32::from(state.config.keep_aspect_ratio == 0);
            }
            FitInput::FillWindow => {
                if fullscreen {
                    state.config.fullscreen_fill_window =
                        i32::from(state.config.fullscreen_fill_window == 0);
                } else {
                    state.config.fill_window = i32::from(state.config.fill_window == 0);
                }
            }
        }
    }
    on_size(hwnd);
    repaint(hwnd);
}

/// `_viv_refresh` (upstream viv.c:14539-14552) — View→Refresh / F5: drop
/// the last-image cache and the parked preload, blank the display (the
/// current file's name stays — upstream's `_viv_clear` keeps the fd and
/// the title, viv.c:1268-1293), and re-open the same path so the file is
/// re-read from disk. No current file: a no-op (upstream's `_viv_open`
/// with an empty fd does nothing).
fn refresh_current(hwnd: HWND) {
    // SAFETY: the borrow spans the entry clone, the cache drops and the
    // display clear — nothing pumps.
    let entry = (unsafe { state_of(hwnd) }).and_then(|state| {
        let entry = state.nav_current.clone()?;
        state.last_cache = None;
        state.preload = None;
        // The `_viv_clear` body: the frames die, the fd/title stay. The
        // animation marks and the view reset ride along (viv.c:1278-1288).
        state.image = None;
        state.displayed_from = None;
        state.displayed_entry = None;
        state.displayed_file_bytes = None;
        state.pending_file_bytes = None;
        state.session = None;
        state.view.reset();
        state.animation_looped = false;
        state.animation_playing = true;
        state.slideshow_timeup = false;
        let stop_timer = state.animation_timer_running;
        state.animation_timer_running = false;
        Some((entry, stop_timer))
    });
    let Some((entry, stop_timer)) = entry else {
        return;
    };
    if stop_timer {
        // SAFETY: hwnd is live; a failed kill leaves a stale timer that
        // the WM_TIMER guard no-ops on.
        let _ = unsafe { KillTimer(Some(hwnd), ANIMATION_TIMER_ID) };
        // The animation timer stopping is a prevent-sleep transition
        // point (upstream `_viv_timer_stop`, viv.c:9633).
        update_prevent_sleep(hwnd);
        // And an on-top while-playing decision point (riviv superset:
        // upstream re-evaluates only on slideshow events and menu opens).
        update_ontop(hwnd);
    }
    refresh_status(hwnd);
    // A blanked display can never hide the cursor — reconcile it
    // (upstream `_viv_start_first_frame` → `_viv_update_show_cursor`,
    // viv.c:7928 + 14338).
    update_cursor(hwnd);
    // SAFETY: queues a WM_PAINT; never pumps messages.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    // The re-open itself — the display adopts the fresh decode's first
    // frame (upstream `_viv_open(&fd, 0)` re-opens the CURRENT fd, id and
    // all: a navigation onto itself, not a fresh direct entry).
    request_open(hwnd, entry.path.as_os_str(), OpenOrigin::Nav(&entry));
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
    // Both the 1:1 and Best Fit grays flip here (upstream's
    // `_viv_view_1to1` ends in `_viv_view_set` → the toolbar refresh).
    refresh_toolbar(hwnd);
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
    // at the click; 4 advances; 1 toggles the slideshow; 2 toggles the
    // animation pause. Unimplemented values (5/6) do nothing — upstream's
    // per-value switch has no default-to-scroll arm.
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
            nav_next(hwnd, false, true, false);
            return;
        }
        // 1 = play/pause slideshow (upstream's action-1 arm is
        // `_viv_pause`, viv.c:6360-6415) — #37.
        1 => {
            slideshow_toggle(hwnd);
            return;
        }
        // 2 = play/pause animation (upstream's action-2 arm is
        // `_viv_animation_pause`, viv.c:14699-14703) — #38.
        2 => {
            animation_pause(hwnd);
            return;
        }
        // 0 falls through to the drag below; values riviv has no
        // handler for (5 1:1 scroll, 6 move-window, hand-edited
        // unknowns) do NOTHING, like upstream's per-value switch
        // (viv.c:6360-6415).
        0 => {}
        _ => return,
    }
    // Upstream's single `_viv_doing` slot: a left-press mid-middle-drag
    // REPLACES the mscroll silently — no cursor restore, no cleanup
    // (WM_LBUTTONDOWN's scroll arm never checks `_viv_doing`, viv.c:14682-
    // 14693, while WM_MBUTTONDOWN's NOTHING-guard does, viv.c:3330). The
    // unbalanced raw ShowCursor leaves the cursor hidden until some later
    // complete mscroll pair re-balances it — upstream's own quirk, kept
    // bug-for-bug.
    // SAFETY: the borrow spans only the two field stores.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.mscroll = None;
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
        // The drag pans through `_viv_view_set` upstream, which refreshes
        // the strip every tick (viv.c:6571) — the render-size grays track
        // a pan that crosses the fit boundary.
        refresh_toolbar(hwnd);
    }
    // The middle-drag scroll (#44; upstream `_VIV_DOING_MSCROLL` arm,
    // viv.c:3610-3625): the delta is the SCREEN anchor minus the live
    // cursor, the cursor re-pins to the anchor after every move (the
    // unbounded-virtual-mouse pattern), and the view scrolls by that
    // delta — upstream computes it and re-pins but never wired the
    // scroll call; riviv completes the evident intent (MPC-HC panscan
    // drag: mouse right reveals the image's right side), a documented
    // README deviation.
    // SAFETY: the borrow spans the anchor read, the pure scroll, and the
    // cursor re-pin — SetCursorPos pumps nothing (it delivers no
    // WM_MOUSEMOVE), so no reentrant state_of borrow can interleave.
    let mscrolled = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let Some(anchor) = state.mscroll else {
            return false;
        };
        let mut live = POINT::default();
        // SAFETY: read-only cursor query, pumps nothing.
        let _ = unsafe { GetCursorPos(&mut live) };
        let (mx, my) = (anchor.x - live.x, anchor.y - live.y);
        if mx == 0 && my == 0 {
            return false;
        }
        let (vp, src) = viewport_and_src(hwnd, state);
        let fit = fit_policy(state);
        state.view.scroll_by(mx, my, src.0, src.1, vp, fit);
        // SAFETY: raw position write; no messages result.
        let _ = unsafe { SetCursorPos(anchor.x, anchor.y) };
        true
    });
    if mscrolled {
        repaint(hwnd);
    }
    // The pixel-info resample closes the move (upstream's final
    // `_viv_update_src_pixel(0,1)`, viv.c:3662 — after the drag arms, so
    // the POS readout tracks a panning image too).
    if update_src_pixel(hwnd, false, true) {
        refresh_status(hwnd);
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

/// WM_MBUTTONDOWN (#44; upstream viv.c:3329-3345): start the middle-drag
/// scroll — capture, anchor the cursor's SCREEN position, then hide the
/// cursor with a RAW `ShowCursor(FALSE)` the `CursorVisibility` flag never
/// sees (upstream's `_viv_is_cursor_shown` stays 1, so `_viv_show_cursor`
/// no-ops for the whole drag; viv.c:14559-14571 — the hide lasts until the
/// cancel path's raw ShowCursor(TRUE)). Guarded: only when neither drag
/// mode is in progress (upstream's `_viv_doing == _VIV_DOING_NOTHING`).
fn on_middle_button_down(hwnd: HWND) {
    let mut pt = POINT::default();
    // SAFETY: GetCursorPos is a read-only query that pumps nothing.
    let _ = unsafe { GetCursorPos(&mut pt) };
    // SAFETY: the borrow spans the guard and the anchor store.
    let start = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        if state.drag.is_some() || state.mscroll.is_some() {
            return false;
        }
        state.mscroll = Some(pt);
        true
    });
    if !start {
        return;
    }
    // SAFETY: hwnd is live and owned by this thread; the capture is
    // released on every mscroll cancel path (MBUTTONUP / ESC).
    let _ = unsafe { SetCapture(hwnd) };
    // SAFETY: raw display-count hide, upstream viv.c:3344.
    unsafe {
        let _ = ShowCursor(false);
    }
}

/// WM_MBUTTONUP — end the middle-drag scroll (upstream WM_MBUTTONUP →
/// `_viv_doing_cancel`, viv.c:3667-3670 + 7850-7871): release the capture
/// and re-show the cursor, both only if a middle-drag was live.
fn on_middle_button_up(hwnd: HWND) {
    // SAFETY: the borrow spans only the Option take.
    let was_mscrolling =
        (unsafe { state_of(hwnd) }).is_some_and(|state| state.mscroll.take().is_some());
    if was_mscrolling {
        // SAFETY: the capture and the raw cursor hide were both taken on
        // this thread in WM_MBUTTONDOWN.
        unsafe {
            let _ = ReleaseCapture();
            let _ = ShowCursor(true);
        }
    }
}

/// WM_XBUTTONDOWN / WM_XBUTTONDBLCLK (and the NC variants) — the mouse
/// back/forward buttons (#44; upstream filters these pre-dispatch in
/// `_viv_is_msg`, viv.c:6302-6343): `xbutton_action` 1 zooms (back = out,
/// forward = in, anchored at the message point), 2 navigates (back =
/// previous, forward = next); every other value is inert (upstream's
/// switch has only the two arms). The NC variants carry SCREEN
/// coordinates through the same client-coordinate path — upstream feeds
/// `GET_X_LPARAM` straight into `_viv_zoom_in` (ClientToScreen on screen
/// coords, viv.c:6322-6329) — kept bug-for-bug.
fn on_xbutton(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    const XBUTTON1: u16 = 0x0001;
    const XBUTTON2: u16 = 0x0002;
    let button = ((wparam.0 >> 16) & 0xffff) as u16;
    // SAFETY: the borrow spans only the config read.
    let action = (unsafe { state_of(hwnd) }).map_or(0, |s| s.config.xbutton_action);
    let pt = lparam_point(lparam);
    match action {
        1 => match button {
            XBUTTON1 => zoom_at(hwnd, true, (pt.x, pt.y)),
            XBUTTON2 => zoom_at(hwnd, false, (pt.x, pt.y)),
            _ => {}
        },
        2 => match button {
            XBUTTON1 => {
                nav_next(hwnd, true, true, false);
            }
            XBUTTON2 => {
                nav_next(hwnd, false, true, false);
            }
            _ => {}
        },
        _ => {}
    }
}

/// How an open request came about — whether the navigation reference
/// (`nav_current`) follows it (upstream `_viv_open` copies `_viv_current_fd`
/// synchronously, viv.c:1574-1579).
pub(crate) enum OpenOrigin<'a> {
    /// A direct pick (drop of one file, dialog, CLI argument): the
    /// reference becomes a fresh id-0 entry (upstream zeroes dwReserved for
    /// direct opens, viv.c:1375-1376).
    Direct,
    /// A navigation target: the reference is the entry itself, id and all
    /// (upstream opens the playlist fd including its ids).
    Nav(&'a PlaylistEntry),
}

/// The early-return arms ahead of request_open's decode queue (upstream
/// `_viv_open`'s order: last cache, preload, already-loading).
enum Arm {
    LastHit,
    PreloadHit,
    AlreadyLoading,
}

/// Navigate onto the file the last cache holds (upstream `_viv_open`'s
/// arm 2, viv.c:1469-1497, + `_viv_activate_last`, viv.c:14457-14525):
/// the parked image swaps in wholesale and the live display parks in its
/// place — no decode, no disk access, so it works even if the file has
/// vanished since. The swap re-caches the displaced display, making A→B→A
/// a ping-pong of parked images.
fn activate_last_flow(hwnd: HWND) {
    let now = qpc_now();
    let title = {
        // SAFETY: the borrow spans state stores and the (already decoded)
        // image swap — nothing pumps; the Win32 tail runs after the drop.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        let Some(cache) = state.last_cache.take() else {
            return;
        };
        // Arm 2's stop-loading (terminate, viv.c:1481-1484) plus the
        // preload-name clear (viv.c:1487): drop the foreground session and
        // any parked preload (riviv's drop terminates the queued/decoding
        // job — upstream's terminate flag does the same at the next
        // per-frame check).
        state.session = None;
        state.preload = None;
        // The current display parks in last (upstream's old_frames save,
        // viv.c:14467-14485 — only when cacheable).
        move_display_to_last(state);
        // The parked image becomes the display, re-anchored like upstream's
        // `_viv_start_first_frame` (viv.c:14313-14319).
        let mut image = cache.image;
        image.reanchor_at(now);
        state.image = Some(image);
        state.displayed_from = None;
        reset_display_marks(state);
        // current_fd = last_fd + title (viv.c:14499-14501).
        state.nav_current = Some(cache.entry.clone());
        state.path = Some(cache.entry.path.clone());
        state.displayed_entry = Some(cache.entry.clone());
        state.displayed_file_bytes = Some(cache.entry.size);
        state.pending_file_bytes = None;
        HSTRING::from_wide(&title_wide(
            state.path.as_deref(),
            TitleFormat::from_config(state.config.title_bar_format),
        ))
    };
    adopt_display_tail(hwnd, Some(title), true);
    // Chain-preload the next neighbor (upstream viv.c:1493).
    request_preload(hwnd);
}

/// Navigate onto the file a preload slot holds (upstream `_viv_open`'s
/// arm 3 → `_viv_open_preload`, viv.c:1498-1501/15132-15206): the slot's
/// decode state picks the arm — swap the parked frames in (whole or
/// partial), flag the in-flight load for promotion, or show the failed
/// verdict.
fn adopt_preload_flow(hwnd: HWND) {
    let now = qpc_now();
    let mut title: Option<HSTRING> = None;
    let mut fatal_msg: Option<String> = None;
    let mut invalidate = false;
    let mut chain_preload = false;
    {
        // SAFETY: the borrow spans state stores and pure frame mapping;
        // nothing pumps. The Win32 tail and the chained preload run after
        // the drop.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        // Upstream resets the failure flags for every non-preload open
        // before the arms (viv.c:1445-1458).
        state.status_file_not_found = false;
        state.status_load_failed = false;
        // The decision reads the slot up front (the arms below mutate or
        // take it, so the borrow must not span them).
        let Some((slot_state, has_first_frame)) =
            state.preload.as_ref().map(|s| (s.state, s.image.is_some()))
        else {
            return;
        };
        match preload::adopt_decision(slot_state, has_first_frame) {
            AdoptDecision::PromoteOnFirstFrame => {
                // viv.c:15134-15164: the title/current fd follow the
                // preload file NOW (the _viv_open_preload head copies
                // preload_fd into current_fd ONLY), and the in-flight load
                // is flagged — its first frame promotes it to the display
                // (the drain's promotion path). displayed_entry — upstream's
                // frame_fd — stays on the still-shown old image until that
                // adoption (viv.c:2962 copies load_fd into frame_fd at the
                // first-frame reply): the displaced display must park into
                // last under its OWN name (cubic+Codex P1).
                let entry = state
                    .preload
                    .as_ref()
                    .map(|s| s.entry.clone())
                    .expect("slot presence checked above");
                if let Some(slot) = state.preload.as_mut() {
                    slot.activate_on_load = true;
                }
                state.nav_current = Some(entry.clone());
                state.path = Some(entry.path.clone());
                state.pending_file_bytes = Some(entry.size);
                title = Some(HSTRING::from_wide(&title_wide(
                    state.path.as_deref(),
                    TitleFormat::from_config(state.config.title_bar_format),
                )));
            }
            AdoptDecision::AdoptComplete => {
                // viv.c:15176-15182: cache the display, swap the whole
                // parked image in, chain-preload the next neighbor. The
                // finished session drops (nothing replies anymore).
                if let Err(e) = adopt_parked_image(state, now, false) {
                    fatal_msg = Some(e);
                } else {
                    invalidate = true;
                    chain_preload = true;
                    title = Some(HSTRING::from_wide(&title_wide(
                        state.path.as_deref(),
                        TitleFormat::from_config(state.config.title_bar_format),
                    )));
                }
            }
            AdoptDecision::AdoptPartial => {
                // viv.c:15166-15175: switch now — the parked first frame
                // takes the display and the still-running stream finishes
                // as the foreground load.
                if let Err(e) = adopt_parked_image(state, now, true) {
                    fatal_msg = Some(e);
                } else {
                    invalidate = true;
                    title = Some(HSTRING::from_wide(&title_wide(
                        state.path.as_deref(),
                        TitleFormat::from_config(state.config.title_bar_format),
                    )));
                }
            }
            AdoptDecision::AdoptFailed => {
                // viv.c:15184-15202: the display parks in last, then blanks
                // with the failed verdict; the failed file's name stays in
                // the title (the FAILED handler never touches
                // current_fd/title, viv.c:2832-2840).
                let Some(slot) = state.preload.take() else {
                    return;
                };
                state.session = None;
                move_display_to_last(state);
                state.image = None;
                state.displayed_from = None;
                state.status_load_failed = true;
                reset_display_marks(state);
                state.nav_current = Some(slot.entry.clone());
                state.path = Some(slot.entry.path.clone());
                state.displayed_entry = None;
                state.displayed_file_bytes = None;
                state.pending_file_bytes = None;
                invalidate = true;
                chain_preload = true;
                title = Some(HSTRING::from_wide(&title_wide(
                    state.path.as_deref(),
                    TitleFormat::from_config(state.config.title_bar_format),
                )));
            }
        }
    }
    adopt_display_tail(hwnd, title, invalidate);
    if let Some(msg) = fatal_msg {
        fatal(&msg);
    }
    if chain_preload {
        request_preload(hwnd);
    }
}

/// The shared body of the two adopt arms: the current display parks in
/// last, the parked image takes the display (DC-carrying Surfaces built
/// here on the UI thread — a wrap failure is GDI exhaustion, fatal like
/// the drain's reply mapping, ADR 0001), re-anchored like upstream's
/// `_viv_start_first_frame` (viv.c:14313-14319), and the preload file
/// commits as the display's entry (upstream copies preload_fd into
/// current_fd/frame_fd, viv.c:14415/15134). `keep_session` moves the
/// still-running stream into the foreground slot (AdoptPartial, upstream
/// flips `_viv_load_is_preload` to 0, viv.c:15172); dropping it retires a
/// finished stream (AdoptComplete).
fn adopt_parked_image(state: &mut WindowState, now: u64, keep_session: bool) -> Result<(), String> {
    let Some(slot) = state.preload.take() else {
        return Ok(());
    };
    let entry = slot.entry;
    let session = slot.session;
    let image = slot
        .image
        .expect("adopt with no parked image is unreachable: adopt_decision gates on it");
    move_display_to_last(state);
    let mut image = image.map_frames(Surface::from_frame)?;
    image.reanchor_at(now);
    state.image = Some(image);
    state.displayed_from = Some(session.id());
    reset_display_marks(state);
    if keep_session {
        state.session = Some(session);
    }
    state.nav_current = Some(entry.clone());
    state.path = Some(entry.path.clone());
    state.displayed_entry = Some(entry.clone());
    state.displayed_file_bytes = Some(entry.size);
    state.pending_file_bytes = None;
    Ok(())
}

/// Park the current display into the last cache (upstream
/// `viv_copy_current_image_to_last_image`, viv.c:14423-14455).
fn move_display_to_last(state: &mut WindowState) {
    let displaced = state.image.take();
    move_displaced_to_last(state, displaced);
}

/// The shared park: the old cache frees, then the display's frames move
/// wholesale — but only when the feature is on and the whole image has
/// decoded (a partial frame set must keep streaming into the display, and
/// upstream frees the old cache without refilling instead,
/// viv.c:14429-14443). A blank display empties the cache (the vacuous
/// 0==0 count compare copies the empty fd over it).
fn move_displaced_to_last(state: &mut WindowState, displaced: Option<LoadedImage<Surface>>) {
    state.last_cache = None;
    if let Some(image) = displaced
        && preload::cacheable(state.config.cache_last != 0, image.decode_complete())
        && let Some(entry) = state.displayed_entry.take()
    {
        state.last_cache = Some(LastCache { entry, image });
        return;
    }
    state.displayed_entry = None;
}

/// The frame-freeing half of upstream `_viv_clear` is the LoadedImage
/// replacement itself; this is the rest — view and animation marks reset,
/// playback restarted (viv.c:1278-1291).
fn reset_display_marks(state: &mut WindowState) {
    state.view.reset();
    state.animation_looped = false;
    state.slideshow_timeup = false;
    state.animation_playing = true;
}

/// The Win32 tail shared by the adopt flows (and mirroring the drain's
/// post-borrow section): title, status, the auto-size edge (upstream
/// `_viv_start_first_frame`'s tail, viv.c:14363-14383), the cursor
/// reconcile and the repaint.
fn adopt_display_tail(hwnd: HWND, title: Option<HSTRING>, invalidate: bool) {
    // The force-resample pairs with the status refresh at the view-set
    // tail (upstream viv.c:14325-14326) — the first frame lands under a
    // possibly-stationary cursor.
    resample_pixel_refresh(hwnd);
    // The strip's grays track the newly displayed size (upstream's
    // load-reply branch, viv.c:2872).
    refresh_toolbar(hwnd);
    // SAFETY: a fresh short borrow for the config/mode read.
    let auto = (unsafe { state_of(hwnd) }).and_then(|s| {
        (!s.fullscreen && s.config.auto_zoom != 0).then_some(s.config.auto_zoom_type)
    });
    if let Some(kind) = auto.filter(|k| (0..=3).contains(k)) {
        window_size_to_image(hwnd, kind);
    }
    update_cursor(hwnd);
    if let Some(title) = title.as_ref() {
        // SAFETY: hwnd is live; the HSTRING outlives the call. Fail-soft
        // like every other title update (upstream viv.c:1249 ignores the
        // SetWindowTextW return too).
        let _ = unsafe { SetWindowTextW(hwnd, title) };
    }
    if invalidate {
        // SAFETY: queues a WM_PAINT; never pumps messages.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// Kick a background preload of the navigation neighbor (upstream
/// `_viv_preload_next`, viv.c:14302-14310: `_viv_next(_viv_last_is_prev,
/// reset=0, is_preload=1, wait=0)` under the config gate). Fired after a
/// foreground load ends, after cache/preload adoptions, and by the
/// shuffle toggle when idle.
fn request_preload(hwnd: HWND) {
    // SAFETY: reads and a copy out; the navigation re-borrows fresh.
    let Some(state) = (unsafe { state_of(hwnd) }) else {
        return;
    };
    if state.config.preload_next == 0 {
        return;
    }
    if state.random_search.is_some() {
        // Preloading is not supported in random mode (upstream `_viv_next`'s
        // is_preload bail, viv.c:5825-5828) — no query, no preload.
        return;
    }
    let prev = state.last_nav_prev;
    nav_next(hwnd, prev, false, true);
}

/// Queue one entry as a preload job (upstream `_viv_open(fd, 1)`'s
/// fresh-start arm, viv.c:1512-1568). Guards: the same file already
/// parked or already the foreground load is skipped (the latter upstream
/// handles by terminating the foreground and re-chaining the same fd —
/// viv.c:1505-1520 — which the FIFO job queue makes pointless), and the
/// last cache's file is never preloaded (viv.c:1462-1467).
fn queue_preload(hwnd: HWND, entry: &PlaylistEntry) {
    // SAFETY: the borrow spans the guard reads, the viewport query and
    // the worker request — the channel send never blocks, nothing pumps.
    let Some(state) = (unsafe { state_of(hwnd) }) else {
        return;
    };
    if state
        .preload
        .as_ref()
        .is_some_and(|s| s.entry.path == entry.path)
    {
        return; // already preloading this exact file
    }
    if state
        .session
        .as_ref()
        .is_some_and(|s| s.path() == entry.path)
    {
        return; // already loading as the foreground
    }
    if state
        .last_cache
        .as_ref()
        .is_some_and(|c| c.entry.path == entry.path)
    {
        return; // don't preload the last cache's file (viv.c:1462-1467)
    }
    state.preload = None;
    // The request-time render viewport rides the job for the worker's mip
    // pre-generation, exactly like a foreground open (upstream snapshots
    // _viv_load_render_wide/high per load, viv.c:1557-1558).
    let mut client = RECT::default();
    // SAFETY: a pure window query on the live hwnd — no pumping.
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let bar_h = match status::height(state.status) {
        0 => initial_status_height(),
        h => h,
    };
    let render_viewport = (
        client.right - client.left,
        (client.bottom - client.top - bar_h).max(0),
    );
    let background = state.config.windowed_bg();
    let session = state
        .load_thread
        .request(hwnd, entry.path.clone(), render_viewport, background);
    state.preload = Some(PreloadSlot {
        session,
        entry: entry.clone(),
        image: None,
        adopted_from: None,
        state: PreloadState::Loading,
        activate_on_load: false,
    });
    refresh_status(hwnd);
}

/// Queue `path` for background decoding (upstream `_viv_open`'s
/// CreateThread arm, viv.c:1569). The current display stays up until this
/// load's first frame replies in; storing a new session supersedes
/// (flags) any in-flight one.
///
/// The arms ahead of the decode queue (upstream `_viv_open`'s early
/// returns, viv.c:1462-1510): a last-cache hit activates the parked
/// image, a preload hit adopts the parked frames, and a repeat request of
/// the in-flight file is a no-op. They run BEFORE the existence check so
/// a cached/preloaded image still shows from memory after its file
/// vanished (upstream never touches the disk on these paths).
pub(crate) fn request_open(hwnd: HWND, path: &OsStr, origin: OpenOrigin<'_>) {
    // SAFETY: the borrow spans flag reads/stores and the cache hits'
    // dispatch; nothing pumps. The adopt flows re-borrow fresh.
    let arm = (unsafe { state_of(hwnd) }).and_then(|state| {
        // Upstream resets the failure flags at the start of every
        // non-preload open, before the arms (viv.c:1445-1458).
        state.status_file_not_found = false;
        state.status_load_failed = false;
        if state
            .last_cache
            .as_ref()
            .is_some_and(|c| c.entry.path == path)
        {
            return Some(Arm::LastHit);
        }
        if state.preload.as_ref().is_some_and(|s| s.entry.path == path) {
            return Some(Arm::PreloadHit);
        }
        if state.session.as_ref().is_some_and(|s| s.path() == path) {
            // Upstream viv.c:1505-1510: "already loading this one" — a
            // repeat request must not supersede the in-flight decode.
            return Some(Arm::AlreadyLoading);
        }
        None
    });
    match arm {
        Some(Arm::LastHit) => {
            activate_last_flow(hwnd);
            return;
        }
        Some(Arm::PreloadHit) => {
            adopt_preload_flow(hwnd);
            return;
        }
        Some(Arm::AlreadyLoading) => {
            refresh_status(hwnd);
            return;
        }
        None => {}
    }
    // Existence check BEFORE queueing a decode (upstream
    // `_viv_open_from_filename`'s GetFileAttributesEx arm, viv.c:1359 —
    // the status bar's "File not found." is a pre-open verdict, not a
    // decode failure, viv.c:5094-5098). The byte size and the timestamps
    // ride along for the status bar and the navigation reference (#39:
    // Date Created sorts on them); directories and unreadable files fall
    // through to the loader as user-level failures like upstream.
    let (not_found, file_bytes, modified, created) = match std::fs::metadata(Path::new(path)) {
        Ok(meta) => (
            false,
            Some(meta.len()),
            Some(playlist::modified_ticks(&meta)),
            Some(playlist::created_ticks(&meta)),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, None, None, None),
        Err(_) => (false, None, None, None),
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
            // The per-image animation marks die with the open (upstream's
            // nav flavor reaches `_viv_clear`'s resets at the FAILED reply,
            // viv.c:1278-1279). Playback is deliberately NOT reset here:
            // this verdict keeps the old display, and upstream resets play
            // only inside `_viv_clear` — where the old display dies (the
            // command-line not-found runs no `_viv_open`/`_viv_clear` at
            // all, viv.c:5094-5098), so a paused animation stays paused
            // until a new image is actually adopted (cubic round 1).
            state.animation_looped = false;
            state.slideshow_timeup = false;
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
        // The per-image animation marks die with the open (upstream
        // `_viv_open` → `_viv_clear`'s frame_looped/timeup resets,
        // viv.c:1278-1279). Playback resets only at the reply that swaps
        // the display — upstream's `_viv_clear` runs at the first-frame/
        // FAILED replies (viv.c:2951/2804), which the drain applies as
        // UiAction::ResetPlayback — so a paused old animation stays paused
        // while Loading shows (cubic round 1).
        state.animation_looped = false;
        state.slideshow_timeup = false;
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
                created: created.unwrap_or(0),
                size: file_bytes.unwrap_or(0),
                id: 0,
            },
        });
        // The title follows the request too (upstream copies fd into
        // _viv_current_fd and updates the title when the load is queued,
        // viv.c:1574-1578): the name shows while Loading, and a failure
        // never reverts it — the FAILED handler touches neither
        // _viv_current_fd nor the title (viv.c:2832-2840); only blank
        // clears both (viv.c:7919-7923). The actual SetWindowTextW runs
        // after the borrow below.
        state.path = Some(path.to_os_string());
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
        // Clear any existing preload and start fresh (upstream
        // `_viv_clear_preload` inside `_viv_open`'s fresh-start arm,
        // viv.c:1512-1513) — placed after the not-found verdict, which
        // upstream never passes through `_viv_open` (viv.c:5094-5098), so
        // a missing file does not kill a parked preload.
        state.preload = None;
        let session =
            state
                .load_thread
                .request(hwnd, path.to_os_string(), render_viewport, background);
        state.session = Some(session);
    }
    // The request-time title update (upstream viv.c:1577-1578 — see the
    // path store above). The borrow is dropped before the call like every
    // SetWindowTextW here.
    // SAFETY: the short borrow spans only the path read for the title.
    if let Some(state) = unsafe { state_of(hwnd) } {
        let title = HSTRING::from_wide(&title_wide(
            state.path.as_deref(),
            TitleFormat::from_config(state.config.title_bar_format),
        ));
        // SAFETY: hwnd is live; the HSTRING outlives the call. Fail-soft
        // like every other title update (upstream viv.c:1249 ignores the
        // SetWindowTextW return too).
        let _ = unsafe { SetWindowTextW(hwnd, &title) };
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
            home_open(hwnd, false, false);
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
                home_open(hwnd, false, false);
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
                created: playlist::created_ticks(&metadata),
                size: metadata.len(),
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

/// `_viv_do_initial_shuffle` (viv.c:13569-13588) — runs at the top of the
/// shuffle navigation arms only: with shuffle on and no playlist, a live
/// current adopts its whole directory as the playlist (`_viv_add_current_
/// path_to_playlist` — a FLAT scan, no recursion, extension-filtered) and
/// the current then ADOPTS ITS OWN NODE'S ID (the copy-back at
/// viv.c:13511-13517 — the reverse direction of what the comment suggests:
/// `_viv_current_fd` takes the node's id, so the shuffle lookup finds it);
/// with a playlist and no order, the order builds (seeded like upstream's
/// srand(QueryPerformanceCounter), viv.c:12821-12826). FS scans never pump
/// messages, so the whole mutation is safe inside one `state_of` borrow.
fn ensure_shuffle_ready(state: &mut WindowState) {
    if state.config.shuffle == 0 {
        return;
    }
    let current_path = state.nav_current.as_ref().map(|e| e.path.clone());
    if state.playlist.is_empty() {
        let Some(current) = current_path else {
            return;
        };
        let Some(dir) = Path::new(&current).parent().map(|d| d.to_path_buf()) else {
            return;
        };
        let Ok(read) = std::fs::read_dir(&dir) else {
            return; // upstream's INVALID_HANDLE_VALUE arm: nothing added
        };
        for entry in read.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                continue; // _viv_is_valid_filename's directory gate
            }
            let path = entry.path();
            if playlist::is_valid_path(path.as_os_str()) {
                let added = state.playlist.add(
                    path.into_os_string(),
                    playlist::modified_ticks(&metadata),
                    playlist::created_ticks(&metadata),
                    metadata.len(),
                );
                if added.path == current {
                    // The copy-back: the CURRENT adopts the node's id.
                    if let Some(nav) = state.nav_current.as_mut() {
                        nav.id = added.id;
                    }
                }
            }
        }
    }
    state.playlist.ensure_shuffle(perf_counter());
}

/// A QueryPerformanceCounter read as the shuffle seed (upstream
/// `srand(counter.LowPart)`, viv.c:12821-12826 — any time-varying seed
/// matches behaviorally).
fn perf_counter() -> u64 {
    let mut now = i64::default();
    // SAFETY: pure counter query; the out-pointer is a valid local.
    unsafe {
        let _ = QueryPerformanceCounter(&mut now);
    }
    now as u64
}

/// Home/End (upstream `_viv_home`, viv.c:6120-6263): under shuffle the
/// order's first/last slot (after `_viv_do_initial_shuffle`); over the
/// playlist otherwise, the sort extreme (current included — re-opening it
/// is allowed); with no playlist, the folder scan of `scan_dir` — and a
/// scan that finds NOTHING blanks the display (`_viv_blank`, viv.c:6245-
/// 6253; unreachable while the playlist is non-empty, its first node
/// always qualifies) — except for a preload, whose home arm stays silent
/// (viv.c:6247-6252). In random mode (checked first, `end` ignored like
/// upstream) the call just draws one more random image (viv.c:6122-6125).
pub(crate) fn home_open(hwnd: HWND, end: bool, preload: bool) {
    home_open_inner(hwnd, end, preload);
    // Upstream `_viv_home`'s tail resets a running slideshow timer
    // UNCONDITIONALLY — every home path (keys, drop-replace, folder open,
    // Everything replace, the random first-query) re-arms it (viv.c:
    // 6257-6261), including its random arm which falls through like the
    // rest.
    reset_slideshow_timer(hwnd);
}

fn home_open_inner(hwnd: HWND, end: bool, preload: bool) {
    // Random mode first (viv.c:6122): borrow only long enough to decide.
    // SAFETY: the read-only borrow ends inside is_some_and.
    if (unsafe { state_of(hwnd) }).is_some_and(|s| s.random_search.is_some()) {
        if preload {
            // Unreachable via request_preload's own bail (viv.c:5825-5828
            // runs before _viv_home can); kept total anyway.
            return;
        }
        everything::send_random(hwnd);
        return;
    }
    // The live sort config — the scan fallback sorts with it too
    // (upstream's FindFirstFile arm compares through the same
    // `_viv_fd_compare` globals, viv.c:6214-6231).
    // SAFETY: the borrow ends inside the map (plain copies out).
    let (mode, ascending, shuffle_on) = (unsafe { state_of(hwnd) })
        .map(|s| {
            (
                playlist::SortMode::from_config(s.config.nav_sort),
                s.config.nav_sort_ascending != 0,
                s.config.shuffle != 0,
            )
        })
        .unwrap_or((playlist::SortMode::DateModified, false, false));
    // SAFETY: the borrow spans the shuffle bookkeeping (FS scans, no
    // pumping) and ends with the entry cloned out.
    let target = (unsafe { state_of(hwnd) }).and_then(|state| {
        if shuffle_on {
            ensure_shuffle_ready(state);
            if let Some(entry) = state.playlist.shuffle_edge(end) {
                return Some(entry.clone());
            }
            // No playlist even after the initial shuffle (no current, or
            // its directory has nothing): fall to the scan below, the
            // upstream else-branch.
        }
        if state.playlist.is_empty() {
            None
        } else {
            playlist::home(state.playlist.entries(), end, mode, ascending).cloned()
        }
    });
    if let Some(entry) = target {
        if preload {
            queue_preload(hwnd, &entry);
        } else {
            request_open(hwnd, &entry.path, OpenOrigin::Nav(&entry));
        }
        return;
    }
    let entries = scan_entries(&scan_dir(hwnd));
    match playlist::home(&entries, end, mode, ascending) {
        Some(entry) => {
            if preload {
                queue_preload(hwnd, entry);
            } else {
                request_open(hwnd, &entry.path, OpenOrigin::Nav(entry));
            }
        }
        None => {
            if !preload {
                blank_display(hwnd);
            }
        }
    }
}

/// Next/Prev for the navigation keys (upstream `_viv_next`,
/// viv.c:5817-6118): random mode draws one more image (checked after the
/// load-wait gate upstream, riviv's gate lives in the keydown route, so
/// the check sits at the top here); with shuffle on, `_viv_do_initial_
/// shuffle` then the order's neighbor (viv.c:5881-5923 — the shuffle arm
/// replaces the fd_compare scan whenever a playlist exists); otherwise
/// the playlist arm when a playlist exists (node exclusion by id), the
/// folder-scan arm over the current file's parent, and no current at all
/// becomes home(0) — for prev too (viv.c:6101-6104). next/prev NEVER
/// blanks: no candidate is a no-op (viv.c:6093-6099).
///
/// `preload=true` is the background flavor (`_viv_next(..., 1, ...)`,
/// reached only from `request_preload`): the direction comes from the
/// last MANUAL navigation (viv.c:5833-5835 — `last_nav_prev` is recorded
/// by non-preload calls only), random mode bails without querying
/// (viv.c:5825-5828), targets queue as preload jobs, and the home arm
/// never blanks (viv.c:6247-6252).
/// Step to the neighbor image (#6's engine). The bool is upstream
/// `_viv_next`'s return (viv.c:5821 `ret = 1`, `ret = 0` only when no
/// candidate exists — the deleted-file follow-up blanks on false,
/// viv.c:7223-7227): true = an open was initiated (or the random search
/// was sent / the no-current Home arm ran, upstream's ret-stays-1 arms),
/// false = nothing to show.
fn nav_next(hwnd: HWND, prev: bool, reset_slideshow: bool, preload: bool) -> bool {
    enum Action {
        Random,
        Home,
        /// A resolved shuffle target — open directly.
        Open(PlaylistEntry),
        /// The fd_compare scan over the playlist.
        PlaylistSorted,
        /// The fd_compare scan over the current file's directory.
        Scan,
    }
    // One borrow for classification AND the shuffle bookkeeping: the FS
    // scans inside `ensure_shuffle_ready` never pump messages, and only
    // cloned data leaves the borrow.
    let action = {
        // SAFETY: see above; nothing below runs inside this borrow.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return false;
        };
        if state.random_search.is_some() {
            if preload {
                // Preloading is not supported in random mode — request_
                // preload already bailed, this is the belt to its braces
                // (viv.c:5825-5828).
                return false;
            }
            Action::Random
        } else {
            if !preload {
                state.last_nav_prev = prev;
            }
            if state.config.shuffle != 0 {
                ensure_shuffle_ready(state);
                if state.playlist.is_empty() {
                    // The initial shuffle built nothing (no current, or
                    // its directory holds no images): upstream lands
                    // home(0) with no current and the plain
                    // directory-scan arm with one (viv.c:5863-6104's
                    // control flow).
                    if state.nav_current.is_none() {
                        Action::Home
                    } else {
                        Action::Scan
                    }
                } else {
                    // The order neighbor. A pathless current matches no id, so
                    // upstream's lookup returns -1 and the front/back row
                    // serves (viv.c:5908-5918) — `shuffle_edge(!prev)` is the
                    // same read.
                    let target = match state.nav_current.as_ref() {
                        Some(current) => state.playlist.shuffle_target(current, prev),
                        None => state.playlist.shuffle_edge(!prev),
                    };
                    match target {
                        Some(entry) => Action::Open(entry.clone()),
                        // Unreachable with a non-empty playlist (ensure built
                        // the order); the sorted scan keeps it total.
                        None => Action::PlaylistSorted,
                    }
                }
            } else {
                match state.nav_current.as_ref() {
                    None => Action::Home,
                    Some(_) if state.playlist.is_empty() => Action::Scan,
                    Some(_) => Action::PlaylistSorted,
                }
            }
        }
    };
    let mut opened = true;
    match action {
        Action::Random => everything::send_random(hwnd),
        Action::Home => home_open(hwnd, false, preload),
        Action::Open(entry) => {
            if preload {
                queue_preload(hwnd, &entry);
            } else {
                request_open(hwnd, &entry.path, OpenOrigin::Nav(&entry));
            }
        }
        Action::PlaylistSorted => {
            // SAFETY: the borrow ends at the end of the statement (the
            // entry is cloned out); nothing below pumps.
            let target = (unsafe { state_of(hwnd) }).and_then(|s| {
                playlist::next(
                    s.playlist.entries(),
                    s.nav_current.as_ref(),
                    prev,
                    true,
                    playlist::SortMode::from_config(s.config.nav_sort),
                    s.config.nav_sort_ascending != 0,
                )
                .cloned()
            });
            if let Some(entry) = target {
                if preload {
                    queue_preload(hwnd, &entry);
                } else {
                    request_open(hwnd, &entry.path, OpenOrigin::Nav(&entry));
                }
            } else {
                opened = false;
            }
        }
        Action::Scan => {
            // The scan arm sorts by the live config too (upstream's
            // FindFirstFile loop compares through `_viv_fd_compare`'s
            // globals, viv.c:6016-6069).
            // SAFETY: the borrow ends inside the map (plain copies out).
            let sort = (unsafe { state_of(hwnd) }).map(|s| {
                (
                    playlist::SortMode::from_config(s.config.nav_sort),
                    s.config.nav_sort_ascending != 0,
                )
            });
            let entries = scan_entries(&scan_dir(hwnd));
            // SAFETY: the borrow ends at the end of the statement (the
            // entry is cloned out).
            let current = (unsafe { state_of(hwnd) }).and_then(|s| s.nav_current.clone());
            let Some((sort, ascending)) = sort else {
                return false;
            };
            if let Some(entry) =
                playlist::next(&entries, current.as_ref(), prev, false, sort, ascending)
            {
                if preload {
                    queue_preload(hwnd, entry);
                } else {
                    request_open(hwnd, &entry.path, OpenOrigin::Nav(entry));
                }
            } else {
                opened = false;
            }
        }
    }
    // Upstream `_viv_next`'s tail (viv.c:6107-6113): a MANUAL navigation
    // re-arms a running slideshow timer so the new image gets the full
    // rate. The slideshow's own timer tick passes false here (viv.c:3156 —
    // the timer keeps its period); the auto-repeat gate upstream applies
    // with the same flag lives in the keydown route instead (an early
    // return before this function runs).
    if reset_slideshow {
        reset_slideshow_timer(hwnd);
    }
    opened
}

/// Re-arm a RUNNING slideshow timer at the current rate (upstream's
/// kill-then-set pair, viv.c:6109-6112 / 6258-6261 — the restart is what
/// gives the just-navigated image its full rate).
fn reset_slideshow_timer(hwnd: HWND) {
    // SAFETY: the borrow spans the two reads; nothing below it borrows.
    let arm = (unsafe { state_of(hwnd) })
        .filter(|state| state.slideshow)
        .map(|state| state.config.slideshow_rate as u32);
    if let Some(rate) = arm {
        // SAFETY: hwnd is live and owned by this thread; the pair is
        // fail-soft like upstream's unchecked calls — a failed kill leaves
        // the stale timer ticking at the OLD rate until the next re-arm.
        let _ = unsafe { KillTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID) };
        // SAFETY: same pair as the kill above.
        let _ = unsafe { SetTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID, rate, None) };
    }
}

// ---- Slideshow (#37; upstream viv.c:6821-6841 / 7594-7621 / 7582-7601 /
// 7594-7630 / 3143-3159 / 7319-7347) ----

/// `_viv_slideshow` (viv.c:6821-6841): the start-only toggle behind
/// F11/View→Slideshow. A File-not-found verdict blocks the start; a
/// windowed viewer enters fullscreen FIRST (the slideshow presents); an
/// already-running slideshow is a no-op. (Upstream also refreshes the
/// toolbar buttons and the on-top state here — those features are riviv's
/// #45/#46 and plug into this point.)
fn slideshow_start(hwnd: HWND) {
    // SAFETY: the borrow spans the two guards only.
    // Upstream order (viv.c:6821-6841): the File-not-found verdict is the
    // only early return; the FULLSCREEN transition comes next — BEFORE the
    // running check — so F11 over a running WINDOWED slideshow (started
    // via Space/left-click) still presents; only then does an
    // already-running slideshow no-op the start (cubic round 1).
    // SAFETY: the borrow spans only the guard read.
    if (unsafe { state_of(hwnd) }).is_some_and(|state| state.status_file_not_found) {
        return;
    }
    // SAFETY: the read-only borrow ends inside is_some_and.
    if !(unsafe { state_of(hwnd) }).is_some_and(|state| state.fullscreen) {
        toggle_fullscreen(hwnd);
    }
    // SAFETY: the borrow spans the running check, the flag flip and the
    // rate read.
    let arm = (unsafe { state_of(hwnd) }).and_then(|state| {
        if state.slideshow {
            return None; // already running: the start is a no-op (viv.c:6833)
        }
        state.slideshow = true;
        Some(state.config.slideshow_rate as u32)
    });
    let Some(rate) = arm else {
        return;
    };
    // SAFETY: hwnd live; fail-soft like upstream's unchecked SetTimer —
    // the flag is the truth, a failed arm just never advances.
    let _ = unsafe { SetTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID, rate, None) };
    refresh_status(hwnd);
    // The strip's play/pause radio flips here (upstream viv.c:6839).
    refresh_toolbar(hwnd);
    update_prevent_sleep(hwnd);
    // The on-top bit follows the run starting (upstream viv.c:6840).
    update_ontop(hwnd);
}

/// `_viv_pause` (viv.c:7594-7621): the running-state toggle behind Space,
/// the Play/Pause row and left-click action 1 — stop when running, resume
/// when not. Never touches fullscreen (that asymmetry with F11 is
/// upstream's).
fn slideshow_toggle(hwnd: HWND) {
    // SAFETY: the borrow spans the flag flip and the rate read.
    let arm = (unsafe { state_of(hwnd) }).map(|state| {
        state.slideshow = !state.slideshow;
        (state.slideshow, state.config.slideshow_rate as u32)
    });
    let Some((running, rate)) = arm else {
        return;
    };
    // SAFETY: hwnd live; the failed-call story is the same as the start
    // arm (the flag is the truth; a stale timer no-ops in WM_TIMER).
    if running {
        // SAFETY: same contract as the start arm.
        let _ = unsafe { SetTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID, rate, None) };
    } else {
        // SAFETY: same contract as the start arm.
        let _ = unsafe { KillTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID) };
    }
    refresh_status(hwnd);
    // The strip's play/pause radio flips here (upstream viv.c:7617).
    refresh_toolbar(hwnd);
    update_prevent_sleep(hwnd);
    // The on-top bit follows the running-state toggle (upstream
    // viv.c:7618).
    update_ontop(hwnd);
}

/// `_viv_set_rate` (viv.c:7582-7601): store the rate; a running timer is
/// re-armed at it; every rate change flashes the readout (the temp-text
/// call at the tail covers the preset ladder, the custom dialog, and the
/// menu rows — upstream viv.c:7068).
fn slideshow_set_rate(hwnd: HWND, rate_ms: u32) {
    // SAFETY: the borrow spans the config store and the running read.
    let running = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        state.config.slideshow_rate = rate_ms as i32;
        state.slideshow
    });
    if running {
        // Kill before set so the period restarts (viv.c:7587-7592).
        // SAFETY: hwnd live; fail-soft like upstream's unchecked pair.
        let _ = unsafe { KillTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID) };
        // SAFETY: same pair as the kill above.
        let _ = unsafe { SetTimer(Some(hwnd), slideshow::SLIDESHOW_TIMER_ID, rate_ms, None) };
    }
    flash_slideshow_rate(hwnd);
}

/// `_viv_increase_rate` (viv.c:7594-7630): step through the preset table;
/// the clamped ends are no-ops.
fn slideshow_step(hwnd: HWND, decrease: bool) {
    // SAFETY: read-only borrow for the current rate.
    let next = (unsafe { state_of(hwnd) })
        .and_then(|s| slideshow::step_rate(s.config.slideshow_rate, decrease));
    if let Some(ms) = next {
        slideshow_set_rate(hwnd, ms);
    }
}

/// The Custom... row (viv.c:1963 → `_viv_set_custom_rate`,
/// viv.c:7453-7483): the modal collects `(value, unit)`; OK composes and
/// applies, storing BOTH the composed rate and the dialog's own fields.
fn slideshow_open_custom_dialog(hwnd: HWND) {
    // SAFETY: read-only borrow for the dialog seeds.
    let seeds = (unsafe { state_of(hwnd) }).map(|s| {
        (
            s.config.slideshow_custom_rate,
            s.config.slideshow_custom_rate_type,
        )
    });
    if let Some((value, type_value)) = seeds
        && let Some(outcome) = custom_rate_dlg::open(hwnd, value, type_value)
    {
        // SAFETY: the borrow spans the two config stores.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.config.slideshow_custom_rate = outcome.value as i32;
            state.config.slideshow_custom_rate_type = outcome.unit.type_value();
        }
        slideshow_set_rate(hwnd, slideshow::custom_rate(outcome.value, outcome.unit));
    }
}

/// The slideshow WM_TIMER body (viv.c:3143-3159): gate through the pure
/// model, then advance WITHOUT re-arming (the slideshow's own tick is not
/// a manual navigation — upstream's `_viv_next(0,0,0,0)`).
fn on_slideshow_timer(hwnd: HWND) {
    // SAFETY: the borrow spans the reads and the timeup store.
    let advance = (unsafe { state_of(hwnd) }).and_then(|state| {
        if !state.slideshow {
            // A stale timer (failed KillTimer) after a stop — no-op, the
            // flag is the truth.
            return None;
        }
        let is_animation = state.image.as_ref().is_some_and(|i| {
            // Upstream's `_viv_frame_count > 1` reads a PRE-KNOWN total;
            // riviv's frame_count is the loaded prefix, so an animation's
            // first frame still counts as "maybe an animation" until the
            // stream completes — hold those too (cubic round 1). A
            // genuinely static image flips to advance at its Complete, at
            // worst delaying the step by the decode.
            i.is_animated() || !i.decode_complete()
        });
        match slideshow::timer_gate(
            state.config.loop_animations_once != 0,
            is_animation,
            state.animation_looped,
        ) {
            slideshow::Gate::WaitForLoop => {
                // Hold the advance until the animation loops once
                // (viv.c:3146-3153); the wrap performs it.
                state.slideshow_timeup = true;
                None
            }
            slideshow::Gate::Advance => Some(()),
        }
    });
    if advance.is_some() {
        nav_next(hwnd, false, false, false);
    }
}

/// `_viv_update_prevent_sleep` (viv.c:7319-7347): with
/// `config_prevent_sleep` on, a running slideshow or a playing animation
/// holds the display awake; ES_CONTINUOUS makes the requirement sticky
/// until the matching release call, so the latch only calls on a CHANGE.
fn update_prevent_sleep(hwnd: HWND) {
    // SAFETY: the borrow spans the reads and the latch update.
    let flip = (unsafe { state_of(hwnd) }).map(|state| {
        // A paused animation does not hold the display awake (upstream
        // viv.c:3926-3934: `_viv_is_animation_timer && _viv_animation_
        // play`); #38 carries the real pause flag.
        let want = state.config.prevent_sleep != 0
            && (state.slideshow || (state.animation_timer_running && state.animation_playing));
        (
            want,
            std::mem::replace(&mut state.prevent_sleep_active, want) != want,
        )
    });
    let Some((want, changed)) = flip else {
        return;
    };
    if !changed {
        return;
    }
    let flags = if want {
        ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    // SAFETY: adjusts only this thread's execution requirements; the
    // return value is the previous state, unused like upstream's.
    let _ = unsafe { SetThreadExecutionState(flags) };
}

/// Upstream `_viv_blank` (viv.c:7908-7930): clear the display, the
/// navigation reference AND the playlist; the title falls back to the app
/// name. Random mode EXITS here too (viv.c:7912-7917 — upstream frees
/// `_viv_random` right after `_viv_clear`; the result total stays stale,
/// harmlessly, the next randomize resets it). Any in-flight load is
/// superseded (upstream `_viv_clear` stops the load thread). The failure
/// flags are NOT reset — upstream's `_viv_clear`/`_viv_blank` leave
/// `_viv_file_not_found`/`_viv_load_failed` alone (viv.c:1268-1293), so a
/// stale verdict survives the blank until the next open resets it
/// (viv.c:1447-1458).
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
        state.random_search = None;
        state.displayed_file_bytes = None;
        state.pending_file_bytes = None;
        state.session = None;
        // The frame fd empties with the display (upstream `_viv_clear`
        // clears _viv_frame_fd's name, viv.c:1273) — nothing left to cache.
        state.displayed_entry = None;
        // The zoom/pan view dies with the display (upstream `_viv_blank` →
        // `_viv_clear`, viv.c:7910 + 1282-1288) — and so do the per-image
        // animation marks (frame_looped/timeup, viv.c:1278/1279). The
        // slideshow itself keeps running (upstream never stops it here).
        state.view.reset();
        state.animation_looped = false;
        state.animation_playing = true;
        state.slideshow_timeup = false;
        stop_timer = state.animation_timer_running;
        state.animation_timer_running = false;
        // The pixel coordinate dies with the display (upstream's blank
        // force-resamples onto a frame_count of 0, which reads invalid —
        // viv.c:7921-7922).
        state.src_pixel = (-1, -1);
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
        // The animation timer stopping is a prevent-sleep transition point
        // (upstream `_viv_timer_stop` → `_viv_update_prevent_sleep`,
        // viv.c:9633).
        update_prevent_sleep(hwnd);
        // And an on-top while-playing decision point (riviv superset).
        update_ontop(hwnd);
    }
    // SAFETY: hwnd is live; the HSTRING outlives the call. Fail-soft like
    // every other title update (upstream viv.c:1249 ignores it too).
    let _ = unsafe {
        SetWindowTextW(
            hwnd,
            &HSTRING::from_wide(&title_wide(None, TitleFormat::FilenameOnly)),
        )
    };
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

/// Seed an EMPTY playlist with the current image (upstream
/// `add_current_if_empty`, viv.c:9341-9351): the current file becomes the
/// first entry with a FRESH id — the navigation reference is NOT remapped
/// (only `_viv_add_current_path_to_playlist`, viv.c:13511-13517, does
/// that). Shared by the add-mode command line, shift-drops and the
/// Everything ADD reply.
fn playlist_add_current_if_empty(state: &mut WindowState) {
    if state.playlist.is_empty()
        && let Some(current) = state.nav_current.as_ref()
    {
        let current = current.clone();
        state.playlist.add(
            current.path,
            current.modified,
            current.created,
            current.size,
        );
    }
}

/// #43 `_viv_delete` (viv.c:7200-7229): FO_DELETE the current file — the
/// shell's own confirmation dialog appears (upstream passes no
/// FOF_NOCONFIRMATION) — then, only on a clean success, converge the
/// playlist on the deleted node and navigate; a directory with nothing
/// left blanks the viewer (`_viv_next` returning 0, viv.c:7223-7227).
/// Aborted (the user answered No) and outright failure keep everything.
fn delete_current(hwnd: HWND, permanently: bool) {
    // SAFETY: read-only clone out of the borrow; nothing below holds it.
    let current = (unsafe { state_of(hwnd) }).and_then(|s| s.nav_current.clone());
    let Some(current) = current else {
        return;
    };
    if let crate::filemgmt::ShellOutcome::Done =
        crate::filemgmt::shell_delete(hwnd, &current.path, permanently)
    {
        // SAFETY: a short plain-field borrow — the navigation below
        // runs after it drops.
        if let Some(state) = unsafe { state_of(hwnd) } {
            state.playlist.remove_by_id(current.id);
        }
        if !nav_next(hwnd, false, true, false) {
            blank_display(hwnd);
        }
    }
}

/// #43 `_viv_edit_rotate` (viv.c:7715-7768): fire the shell rotate90/
/// rotate270 verb and WAIT — the OS photo handler rewrites the file on
/// disk (upstream's own re-encode route; riviv keeps it verbatim, see
/// README Differences) — then rotate every loaded frame in memory, drop
/// the mips, re-anchor the view at the swapped dimensions, and refresh
/// the POS/RGB sample, the status bar and the paint. The decode-complete
/// gate is upstream's own "wait for the image to load" FIXME
/// (`_viv_frame_loaded_count == _viv_frame_count`, viv.c:7721-7723); a
/// failed verb launch skips the memory pass too (fail-soft).
fn rotate_current(hwnd: HWND, counterclockwise: bool) {
    // SAFETY: read-only clones; the shell call below must run without a
    // borrow (its collision/progress dialogs pump messages).
    let (path, ready) = (unsafe { state_of(hwnd) }).map_or((None, false), |s| {
        (
            s.nav_current.as_ref().map(|e| e.path.clone()),
            s.image.as_ref().is_some_and(|i| i.decode_complete()),
        )
    });
    let Some(path) = ready.then_some(path).flatten() else {
        return;
    };
    // Upstream's verb pair: ROTATE_90 → "rotate90" + orientation 6
    // (clockwise), ROTATE_270 → "rotate270" + orientation 8 (viv.c:2492-
    // 2495 / 7736).
    let verb = if counterclockwise {
        "rotate270"
    } else {
        "rotate90"
    };
    if crate::shell::shell_execute(hwnd, &path, Some(verb), None, true).is_err() {
        return;
    }
    {
        // SAFETY: the borrow spans the frame rotations and the view
        // re-anchor — plain GDI calls and arithmetic, nothing pumps (the
        // shell call has returned; the DC work touches no windows).
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        if let Some(image) = state.image.as_mut() {
            // Per-frame fail-soft, like upstream: a frame whose rotation
            // fails keeps its old bitmap and the loop continues
            // (viv.c:7736-7749 keeps the old HBITMAP on a 0 return).
            for frame in image.frames_mut() {
                frame.rotate(!counterclockwise);
            }
        }
        // `_viv_view_set(_viv_view_x,_viv_view_y,1)` (viv.c:7756): re-run
        // the size pass at the same view coordinates against the swapped
        // dimensions.
        let (vp, src) = viewport_and_src(hwnd, state);
        let fit = fit_policy(state);
        let (vx, vy) = (state.view.view_x, state.view.view_y);
        state.view.set_view(vx, vy, src.0, src.1, vp, fit);
    }
    // `_viv_update_src_pixel(1,0)` + `_viv_status_update` +
    // InvalidateRect (viv.c:7757-7759/7763).
    update_src_pixel(hwnd, true, false);
    refresh_status(hwnd);
    repaint(hwnd);
}

/// #43 Copy To / Move To (viv.c:2496-2568): GetSaveFileName seeded with
/// the current file's full path (the "save as" flavor — OFN_OVERWRITE-
/// PROMPT guards collisions), then FO_COPY/FO_MOVE with FOF_ALLOWUNDO.
/// Fail-soft both ways, and upstream never touches the playlist, the
/// current file or the title afterwards — a Move leaves the display and
/// the navigation reference pointing at the moved-away path until the
/// next navigation slides off it (kept verbatim).
fn copy_move_to(hwnd: HWND, copy: bool) {
    // SAFETY: read-only clone out of the borrow; the save dialog below
    // runs a message pump of its own.
    let path =
        (unsafe { state_of(hwnd) }).and_then(|s| s.nav_current.as_ref().map(|e| e.path.clone()));
    let Some(path) = path else {
        return;
    };
    // The filter (viv.c:2513): "<All Files> (*.*)\0*.*\0" over the
    // localized label, double-NUL terminated like every filter list.
    let label = loc::get(loc::Id::OpenAllFiles);
    let mut filter: Vec<u16> = Vec::new();
    filter.extend(label.encode_utf16());
    filter.extend(" (*.*)".encode_utf16());
    filter.push(0);
    filter.extend("*.*".encode_utf16());
    filter.push(0);
    filter.push(0);
    let title = to_wide(loc::get(if copy {
        loc::Id::CopyToCaption
    } else {
        loc::Id::MoveToCaption
    }));
    // Upstream seeds a STRING_SIZE buffer with the current name and
    // hands its full capacity to the dialog (viv.c:2508/2518).
    let mut file_buf = crate::text::to_wide_os(&path);
    file_buf.resize(1025, 0);
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_ENABLESIZING | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: `ofn` points only at buffers alive in this frame; the
    // dialog writes within nMaxFile; the returned path stays
    // NUL-terminated by the API's contract.
    if unsafe { GetSaveFileNameW(&mut ofn) }.as_bool() {
        crate::filemgmt::shell_copy_move(hwnd, &path, &file_buf, copy);
    }
}

/// Apply one parsed command line (upstream `_viv_process_command_line`,
/// viv.c:4744-5148 — the file-word ladder, the switch arms' side effects,
/// and the show-state tail). Shared by the startup line (viv.c:5445) and
/// the single-instance handoff receive (#21, viv.c:3715). The action
/// stream replays IN WALK ORDER — a `/random` between file words fires
/// its navigation before later words process, and a `/name` after it only
/// then applies (upstream's one-loop ordering, cubic P1); the end blocks
/// (add-mode bootstrap / the open), the usage boxes and the show tail
/// (slideshow, fullscreen, rect, maximized) come last.
///
/// One deliberate timing divergence, documented in README Differences:
/// upstream pops the usage box for EACH unknown word mid-walk; riviv
/// collects them and pops after the whole line is applied — a modal box
/// pumping messages over a half-applied command line is reentrancy
/// upstream only survives by accident.
fn process_parsed_cl(hwnd: HWND, parsed: &cli::Parsed) {
    for action in &parsed.actions {
        match action {
            cli::ClAction::ConfigWrite(write) => {
                // A switch arm's config write (upstream's globals,
                // viv.c:4838-4953). NOTE: unlike the menu's sort/shuffle
                // handlers (viv.c:1750-1816), the CLI path writes the
                // config DIRECTLY — no preload/last cache clear.
                // SAFETY: the borrow spans plain field stores.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    if let Some(mode) = write.nav_sort {
                        state.config.nav_sort = mode as i32;
                    }
                    if let Some(ascending) = write.sort_ascending {
                        state.config.nav_sort_ascending = ascending;
                    }
                    if write.shuffle {
                        state.config.shuffle = 1;
                    }
                    if let Some(rate) = write.slideshow_rate {
                        state.config.slideshow_rate = rate;
                    }
                }
            }
            // `/everything <term>` and `/random <term>` (viv.c:4857-4872 —
            // the send fires mid-walk with the dialog parent 0). send_search
            // takes its own state borrows inside; none is held here.
            cli::ClAction::Everything(term) => {
                everything::send_search(hwnd, HWND::default(), false, false, term);
            }
            cli::ClAction::Random(term) => {
                everything::send_search(hwnd, HWND::default(), false, true, term);
            }
            cli::ClAction::ExitRandom => {
                // SAFETY: the borrow spans one field store.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    state.random_search = None;
                }
            }
            cli::ClAction::ClearPlaylist => {
                // SAFETY: the borrow spans the clear.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    state.playlist.clear();
                }
            }
            cli::ClAction::AddFile(word) => {
                // Relative words resolve against the CWD (upstream's
                // string_path_combine — the handoff adopts the sender's
                // cwd before this runs).
                let path = absolutize(word);
                // SAFETY: the borrow spans the add's metadata read.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    playlist::add_filename(&mut state.playlist, Path::new(&path));
                }
            }
            // `/ontop` / `/minimal` / `/compact` (#46; viv.c:4853-4888):
            // upstream `_viv_command`s them mid-walk — the same handlers
            // the menu rows use, quirks included.
            cli::ClAction::Command(cmd) => on_command(hwnd, *cmd),
        }
    }
    // The add-mode end block (viv.c:5027-5041): the current file seeds an
    // EMPTY playlist before a lone word appends.
    if parsed.is_add {
        // SAFETY: the borrow spans the bootstrap's metadata read and the
        // possible add.
        if let Some(state) = unsafe { state_of(hwnd) } {
            playlist_add_current_if_empty(state);
            if parsed.file_count == 1
                && let Some(word) = &parsed.single
            {
                let path = absolutize(word);
                playlist::add_filename(&mut state.playlist, Path::new(&path));
            }
        }
    }
    // Show the first image — never in add-mode (viv.c:5046-5098).
    if !parsed.is_add && parsed.file_count >= 1 {
        let resolved = if parsed.file_count == 1 {
            let path = absolutize(parsed.single.as_ref().unwrap());
            open_from_filename(hwnd, path.as_os_str())
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
    // The usage boxes — one per unknown switch word (viv.c:4986-4990);
    // popped post-parse (see the doc comment above).
    for _ in &parsed.unknown {
        show_usage(hwnd);
    }
    // The show tail (viv.c:5102-5141).
    if parsed.start_slideshow {
        slideshow_start(hwnd);
    }
    // SAFETY: the read-only borrow ends inside the map.
    let fullscreen = (unsafe { state_of(hwnd) }).is_some_and(|state| state.fullscreen);
    if parsed.start_fullscreen {
        if !fullscreen {
            toggle_fullscreen(hwnd);
            if parsed.start_maximized {
                // `/fullscreen /maximized` — the fullscreen-maximized
                // hybrid restores the maximized state on exit (viv.c:5112).
                // SAFETY: the borrow spans one field store.
                if let Some(state) = unsafe { state_of(hwnd) } {
                    state.fullscreen_was_maxed = true;
                }
            }
        }
    } else {
        if parsed.start_window {
            if fullscreen {
                toggle_fullscreen(hwnd);
            }
        } else if parsed.rect.any() {
            // Unset axes keep the current rect (viv.c:4774 seeds all four
            // from GetWindowRect).
            let mut cur = RECT::default();
            // SAFETY: hwnd is live; cur receives the window rect.
            let _ = unsafe { GetWindowRect(hwnd, &mut cur) };
            let x = parsed.rect.x.unwrap_or(cur.left);
            let y = parsed.rect.y.unwrap_or(cur.top);
            let wide = parsed.rect.wide.unwrap_or(cur.right - cur.left);
            let high = parsed.rect.high.unwrap_or(cur.bottom - cur.top);
            // SAFETY: hwnd is live; upstream's flags exactly (viv.c:5129).
            let _ = unsafe {
                SetWindowPos(hwnd, None, x, y, wide, high, SWP_NOZORDER | SWP_NOACTIVATE)
            };
        }
        if parsed.start_maximized {
            // SAFETY: hwnd is live; a cheap zoomed check.
            if !unsafe { IsZoomed(hwnd) }.as_bool() {
                // SAFETY: hwnd is live.
                let _ = unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
            }
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

/// One file word as an absolute path (main.rs's old `file_args` step:
/// upstream cwd-combines relative paths, string_path_combine).
fn absolutize(word: &[u16]) -> std::ffi::OsString {
    let s = std::ffi::OsString::from_wide(word);
    match std::path::absolute(&s) {
        Ok(p) => p.into_os_string(),
        Err(_) => s,
    }
}

/// The command-line usage box (`_viv_command_line_options`, viv.c:11856+):
/// MB_OK|MB_ICONQUESTION over the caller — the question icon is what
/// avoids the message beep — with the app name as caption. Reached for
/// each unknown switch word AND from Help → Command Line Options
/// (viv.c:1687-1688).
fn show_usage(hwnd: HWND) {
    // SAFETY: a modal box over the live window; no state borrow is live.
    unsafe {
        MessageBoxW(
            Some(hwnd),
            &HSTRING::from(loc::get(loc::Id::UsageText)),
            &HSTRING::from(loc::get(loc::Id::AppName)),
            MB_OK | MB_ICONQUESTION,
        );
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
    // The second pass over the ORIGINAL command line (#48): the raw
    // tokenizer — quotes, switches, parameter words — exactly like the
    // startup line (upstream runs the same `_viv_process_command_line`,
    // viv.c:3715).
    // The add-vs-replace decision + the `/add` arm's current-file input
    // (upstream viv.c:4778-4793/4963-4969) — fed to the pure parse.
    // SAFETY: the read-only borrow ends inside the let.
    let (initial_is_add, has_current) = (unsafe { state_of(hwnd) })
        .map(|state| {
            // SAFETY: a cheap kernel tick query.
            let add = handoff_add_mode(
                unsafe { GetTickCount() },
                state.last_cl_tick,
                state.config.add_command_line_timeout,
                state.nav_current.is_some(),
            );
            (add, state.nav_current.is_some())
        })
        .unwrap_or((false, false));
    let parsed = cli::parse(&handoff.command_line, initial_is_add, has_current);
    process_parsed_cl(hwnd, &parsed);
    // Show per the second launch's requested state (viv.c:3717): a "run
    // maximized" shortcut forwards SW_MAXIMIZE; a plain launch's
    // SW_SHOWNORMAL restores a minimized window and brings it forward.
    // SAFETY: hwnd is live.
    let _ = unsafe { ShowWindow(hwnd, SHOW_WINDOW_CMD(handoff.show_cmd as i32)) };
    true
}

/// An Everything OPEN/ADD reply (upstream viv.c:3808-3904): exit random
/// mode, OPEN clears the playlist and ADD seeds an empty one with the
/// current image (`add_current_if_empty`), then every parseable item
/// appends — and OPEN homes onto the sort extreme (which is the MTIME
/// NEWEST result under the default sort, not the first result; the list
/// itself keeps Everything's result order). The parse walks the flags
/// stored at SEND time, never the reply's echo.
fn on_everything_reply(hwnd: HWND, cds: &COPYDATASTRUCT, add: bool) {
    // The forged-reply gate (cubic P1): a null lpData or a payload too
    // short for the LIST2 header is not ours to parse — `from_raw_parts`
    // over a null/short pointer is UB even at length zero, and a foreign
    // sender owes us nothing. Handled-and-ignored like the command-line
    // arm's size gate.
    if cds.lpData.is_null() || cds.cbData < 20 {
        return;
    }
    // SAFETY: the WM_COPYDATA contract guarantees lpData addresses cbData
    // readable bytes for the duration of the message; the gate above keeps
    // null/short pointers out of the slice.
    let bytes = unsafe { std::slice::from_raw_parts(cds.lpData.cast::<u8>(), cds.cbData as usize) };
    // SAFETY: the borrow spans the random exit, the playlist ops and the
    // flags read — the metadata-free adds never pump.
    let parsed = (unsafe { state_of(hwnd) }).map(|state| {
        state.random_search = None;
        if add {
            playlist_add_current_if_empty(state);
        } else {
            state.playlist.clear();
        }
        let flags = state.everything_request_flags;
        let reply = everything::parse_list2(bytes, flags);
        for item in &reply.items {
            let path = OsString::from_wide(&item.path);
            state.playlist.add(
                path,
                item.modified_ticks.unwrap_or(0),
                item.created_ticks.unwrap_or(0),
                item.size.unwrap_or(0),
            );
        }
        reply
    });
    if !add && parsed.is_some() {
        home_open(hwnd, false, false);
    }
}

/// An Everything RANDOM reply (upstream viv.c:3724-3804) — the exact
/// three-way split: no items with a positive total stores the total and
/// retries (the random index was past the end); otherwise the FIRST item
/// opens directly when it survives the filters (folder/length/extension —
/// all inside `parse_list2`); anything else is upstream's silent hard stop,
/// no retry.
fn on_random_reply(hwnd: HWND, cds: &COPYDATASTRUCT) {
    enum Decision {
        Open(everything::List2Item),
        Retry,
        Stop,
    }
    // Same forged-reply gate as on_everything_reply (cubic P1).
    if cds.lpData.is_null() || cds.cbData < 20 {
        return;
    }
    // SAFETY: same WM_COPYDATA contract + gate as on_everything_reply.
    let bytes = unsafe { std::slice::from_raw_parts(cds.lpData.cast::<u8>(), cds.cbData as usize) };
    // SAFETY: the borrow spans the flags read and the total store — nothing
    // pumps.
    let decision = (unsafe { state_of(hwnd) }).map(|state| {
        let reply = everything::parse_list2(bytes, state.everything_request_flags);
        if reply.numitems == 0 {
            if reply.totitems > 0 {
                state.random_tot_results = reply.totitems;
                Decision::Retry
            } else {
                Decision::Stop
            }
        } else {
            reply
                .items
                .first()
                .cloned()
                .map_or(Decision::Stop, Decision::Open)
        }
    });
    match decision {
        Some(Decision::Open(item)) => {
            let path = OsString::from_wide(&item.path);
            // The direct open (upstream `_viv_open(fd,0)` with zeroed
            // reserved fields, viv.c:3790 — a fresh id-0 navigation
            // reference).
            request_open(hwnd, &path, OpenOrigin::Direct);
        }
        // numitems == 0 && totitems > 0: re-query with the narrowed total
        // (viv.c:3799-3802 posts the retry; send_random re-checks
        // random-search arming itself).
        Some(Decision::Retry) => {
            // SAFETY: queues a message; never pumps.
            let _ = unsafe {
                PostMessageW(
                    Some(hwnd),
                    everything::RETRY_RANDOM_MESSAGE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
        _ => {}
    }
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
    // The foreground load ended while still the active session — chain a
    // background preload (upstream's allow_preload_next →
    // _viv_preload_next, viv.c:2874-2879).
    let mut kick_preload = false;
    {
        // Copy the session facts out first so the immutable borrow ends
        // before the reply loop mutates the display state.
        // SAFETY: the borrow spans queue draining, the pure reply state
        // machines, and read-only geometry queries — nothing here pumps
        // messages, so no second state_of borrow can alias this one.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        if let Some(session) = state.session.as_ref() {
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
                let playback = anim_playback(state);
                // A first frame displaces the current display: upstream
                // parks it in the last cache at EVERY first-frame reply
                // (viv.c:2949) — take it out before apply_reply replaces
                // it. A blank display displaces nothing (upstream's vacuous
                // count compare empties the cache instead).
                let is_first_frame = matches!(reply, LoadReply::FirstFrame { .. });
                let displaced = if is_first_frame {
                    state.image.take()
                } else {
                    None
                };
                let outcome = apply_reply(
                    &mut state.image,
                    &mut state.displayed_from,
                    session_id,
                    now,
                    state.timer_freq,
                    playback,
                    reply,
                );
                // The displaced display parks BEFORE the adoption edge
                // below re-points displayed_entry at the new session's
                // entry (upstream: copy_current_to_last runs at
                // viv.c:2949, _viv_clear at 2951).
                if is_first_frame {
                    move_displaced_to_last(state, displaced);
                }
                // The status bar's Loading/Failed flags follow the protocol
                // facts (#5): the session ends at its terminal reply (taken so
                // `session.is_some()` stops meaning "loading"), and a
                // user-level failure sticks until the next open.
                if outcome.load_ended
                    && state.session.as_ref().is_some_and(|s| s.id() == session_id)
                {
                    state.session = None;
                    kick_preload = true;
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
                        UiAction::ResetPlayback => {
                            // Upstream's `_viv_clear` sets
                            // `_viv_animation_play = 1` (viv.c:1291): every
                            // display swap starts playing. The open request
                            // deliberately leaves the old pause alone until
                            // this moment (cubic round 1).
                            state.animation_playing = true;
                        }
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
                                title = Some(HSTRING::from_wide(&title_wide(
                                    state.path.as_deref(),
                                    TitleFormat::from_config(state.config.title_bar_format),
                                )));
                                // The navigation facts follow the adopted
                                // image (upstream copies _viv_load_fd into
                                // _viv_frame_fd at the first-frame reply,
                                // viv.c:2962 — the last-cache source).
                                state.displayed_entry = state.nav_current.clone();
                                // Commit the staged size now that THIS session's
                                // image is on screen (a failed replacement never
                                // reaches here, so the old size survives).
                                state.displayed_file_bytes = state.pending_file_bytes.take();
                            } else {
                                // The mid-stream FAILED clear: the display goes
                                // blank but the title KEEPS the failed file's
                                // name — upstream's FAILED handler runs
                                // _viv_clear on the frame data only, leaving
                                // _viv_current_fd and the title untouched
                                // (viv.c:2832-2840); only blank clears both
                                // (viv.c:7919-7923). The status bar's "(N KB)"
                                // still clears with the display, and the frame
                                // fd empties with it (viv.c:1273) — nothing to
                                // cache.
                                state.displayed_file_bytes = None;
                                state.displayed_entry = None;
                            }
                        }
                    }
                }
                if fatal_msg.is_some() {
                    break;
                }
            }
        }
        // ---- preload slot drain (#40) ----
        // The parked preload's replies apply against the slot's own state;
        // the display never hears about them (upstream's preload branches
        // stash into _viv_preload_frames only, viv.c:2926-2943).
        let mut adoption = preload::DrainAdoption::None;
        let playback = anim_playback(state);
        if let Some(slot) = state.preload.as_mut() {
            let slot_id = slot.session.id();
            let replies = slot.session.drain();
            for reply in replies {
                // DIBs stay bare — the DC wrap happens only if/when the
                // image takes the display (the same split upstream makes
                // between _viv_preload_frames and _viv_frames).
                let reply = map_reply_frame(reply, Ok::<DibFrame, String>);
                let outcome = apply_reply(
                    &mut slot.image,
                    &mut slot.adopted_from,
                    slot_id,
                    now,
                    state.timer_freq,
                    playback,
                    reply,
                );
                if let Some(_msg) = outcome.fatal {
                    // A system-level decode failure in a PARKED preload
                    // degrades to a failed slot (upstream's load thread
                    // reports GDI exhaustion as a user-level load failure
                    // and the preload arm just marks state 2,
                    // viv.c:2808-2812); killing the viewer over a
                    // background prefetch would be worse. The foreground
                    // path keeps its fail-loud posture (README note).
                    slot.state = PreloadState::Failed;
                    slot.image = None;
                    continue;
                }
                if outcome.load_failed {
                    slot.state = PreloadState::Failed;
                } else if outcome.load_ended {
                    slot.state = PreloadState::Complete;
                }
            }
            // The batch's terminal net state decides what a waiting
            // navigation gets: promote on a first frame, adopt-and-chain on
            // a same-drain completion, blank on a failure (upstream's
            // should_activate arms, viv.c:2808-2829/2916-2924) — the pure
            // decision lives in preload::drain_adoption.
            adoption =
                preload::drain_adoption(slot.activate_on_load, slot.state, slot.image.is_some());
        }
        match adoption {
            preload::DrainAdoption::FailActivation => {
                // The load failed while a navigation waited on it (upstream
                // viv.c:2808-2819): the old display drops UNCACHED (only the
                // already-failed adopt arm copies to last, viv.c:15187), the
                // failed verdict shows, and the next preload chains.
                // nav/path/title already name the failed file — the FAILED
                // handler never touches current_fd (viv.c:2832-2840).
                state.preload = None;
                state.session = None;
                state.image = None;
                state.displayed_from = None;
                state.status_load_failed = true;
                reset_display_marks(state);
                state.displayed_entry = None;
                state.displayed_file_bytes = None;
                state.pending_file_bytes = None;
                invalidate = true;
                kick_preload = true;
            }
            preload::DrainAdoption::Promote { keep_session } => {
                match adopt_parked_image(state, now, keep_session) {
                    Ok(()) => {
                        invalidate = true;
                        adopted_new_image = true;
                        title = Some(HSTRING::from_wide(&title_wide(
                            state.path.as_deref(),
                            TitleFormat::from_config(state.config.title_bar_format),
                        )));
                        // A stream that finished inside this drain has no
                        // reply left to chain from — the completion arm
                        // fires the next preload itself (viv.c:2824-2826/
                        // 2877-2879); a kept stream chains at its own load
                        // end instead.
                        if !keep_session {
                            kick_preload = true;
                        }
                    }
                    Err(e) => fatal_msg = Some(e),
                }
            }
            preload::DrainAdoption::None => {}
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
        // The animation timer stopping is a prevent-sleep transition point
        // (upstream `_viv_timer_stop` → `_viv_update_prevent_sleep`,
        // viv.c:9633).
        update_prevent_sleep(hwnd);
        // And an on-top while-playing decision point (riviv superset).
        update_ontop(hwnd);
    }
    if start_timer {
        // SAFETY: hwnd is live and owned by this thread. Fail-soft like
        // upstream viv.c:9144 (unchecked SetTimer): a failed timer merely
        // freezes the animation.
        let _ = unsafe { SetTimer(Some(hwnd), ANIMATION_TIMER_ID, USER_TIMER_MINIMUM, None) };
        // The animation timer starting is a prevent-sleep transition point
        // (upstream `_viv_timer_start` → `_viv_update_prevent_sleep`,
        // viv.c:9147).
        update_prevent_sleep(hwnd);
        // And an on-top while-playing decision point (riviv superset).
        update_ontop(hwnd);
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
    // Chain a background preload after the load ended (upstream fires
    // _viv_preload_next inside the completion reply, viv.c:2874-2879 —
    // after the next_fd pickup, which the FIFO job queue makes
    // unnecessary).
    if kick_preload {
        request_preload(hwnd);
    }
}

/// The playback knobs snapshot from the window state (#38) — the pause
/// flag and the rate table position the timer loop and the streaming
/// re-anchor read (upstream's globals `_viv_animation_play` /
/// `_viv_animation_rate_pos`, viv.c:672-673).
fn anim_playback(state: &WindowState) -> crate::anim::Playback {
    crate::anim::Playback {
        playing: state.animation_playing,
        rate_pos: state.animation_rate_pos,
    }
}

/// WM_TIMER for the animation timer: advance the animation by the time
/// elapsed since the previous event and repaint when the displayed frame
/// changed (upstream viv.c:3171-3292).
fn on_animation_timer(hwnd: HWND) {
    // Read the clock before any state borrow — its failure path is the fatal
    // modal (see open_image).
    let now = qpc_now();
    // SAFETY: the borrow spans only scheduler/position field updates; the
    // nav_next call below runs after it drops (its own paths re-borrow).
    let (repaint, held_advance) = match unsafe { state_of(hwnd) } {
        Some(state) => {
            let freq = state.timer_freq;
            // The held-advance gate (upstream viv.c:3243-3248: loop-once on
            // + the slideshow timer already expired). Deliberately WITHOUT
            // a running check: upstream's `_viv_pause` does not clear
            // `_viv_is_slideshow_timeup` (viv.c:7594-7621 — only
            // `_viv_clear` resets it, viv.c:1279), so pausing between the
            // timer expiry and the loop still lets the wrap's ONE advance
            // through — at most a single stray step per pause, exactly the
            // upstream quirk (cubic round 1, declined).
            let gate = state.slideshow_timeup && state.config.loop_animations_once != 0;
            let playback = anim_playback(state);
            match state.image.as_mut() {
                // The timer only runs while an animation is displayed; the
                // guard also makes a stale timer (failed KillTimer) harmless.
                Some(image) if image.is_animated() => {
                    let adv = image.advance_on_timer(now, freq, gate, playback);
                    if adv.looped {
                        // Upstream raises _viv_frame_looped on the wrap
                        // (viv.c:3243) — the slideshow gate's wait ends
                        // with it, and the loop-once STOP behavior (#38)
                        // will read it too.
                        state.animation_looped = true;
                    }
                    (adv.repaint, adv.looped && gate)
                }
                _ => (false, false),
            }
        }
        None => (false, false),
    };
    if held_advance {
        // The gate held the slideshow's advance until this animation
        // looped once — fire it now, as a MANUAL advance (the timer
        // re-arms, viv.c:3245's `_viv_next(0,1,0,0)`).
        nav_next(hwnd, false, true, false);
    }
    if repaint {
        // The frame counter part ("n / m") tracks the displayed frame,
        // and the RGB under the cursor moves with it (upstream pairs the
        // force-resample with the status refresh in the timer body,
        // viv.c:3276-3277).
        resample_pixel_refresh(hwnd);
        // SAFETY: queues a WM_PAINT; never pumps messages. Erase is FALSE
        // like upstream viv.c:3284 — WM_PAINT fills the whole client itself.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// Animation → Play/Pause (#38; upstream `_viv_animation_pause`, viv.c:
/// 9250-9253): flip the playing flag and nothing else — the timer keeps
/// running (paused events discard their time), the menu check follows the
/// flag (viv.c:7184), and the prevent-sleep hold drops with it (viv.c:
/// 3929-3936).
fn animation_pause(hwnd: HWND) {
    // SAFETY: the borrow spans the flag flip.
    let flipped = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        state.animation_playing = !state.animation_playing;
        true
    });
    if flipped {
        // A playing⇄paused transition is a prevent-sleep decision point.
        update_prevent_sleep(hwnd);
        // And an on-top while-playing one (riviv superset).
        update_ontop(hwnd);
    }
}

/// The Frame Step / Previous Frame / First Frame / Last Frame commands'
/// shared shape (#38; upstream `_viv_frame_step`/`_viv_frame_prev` and the
/// inline FRAME_HOME/FRAME_END cases, viv.c:9255-9315/1887-1932): the
/// looped-mark reset and the pause run UNCONDITIONALLY — even for a static
/// image, where the walk then does nothing (the upstream guards wrap only
/// the position walk). A successful walk repaints and refreshes the frame
/// counter.
fn frame_command(hwnd: HWND, walk: impl Fn(&mut LoadedImage, u64) -> bool) {
    // Read the clock before any state borrow — the same discipline as the
    // timer path.
    let now = qpc_now();
    // SAFETY: the borrow spans the flag resets and the position walk; the
    // GDI calls below run after it drops.
    let walked = (unsafe { state_of(hwnd) }).and_then(|state| {
        state.animation_looped = false;
        state.animation_playing = false;
        state.image.as_mut().map(|image| walk(image, now))
    });
    // The unconditional pause is a prevent-sleep transition (viv.c:
    // 3929-3936) whether or not the walk moved.
    update_prevent_sleep(hwnd);
    // And an on-top while-playing one (riviv superset).
    update_ontop(hwnd);
    if walked.unwrap_or(false) {
        // The frame counter ("n / m") tracks the walk, and the RGB under
        // the cursor moves with the frame (upstream pairs the
        // force-resample with the status update in the handler body,
        // viv.c:9277-9278).
        resample_pixel_refresh(hwnd);
        // SAFETY: queues a WM_PAINT; never pumps messages.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

/// The Animation rate commands (#38; upstream
/// `_viv_increase_animation_rate`/`_viv_reset_animation_rate`, viv.c:
/// 7656-7681): step the table position (clamped, no wrap) or return to
/// 1.0×. The position persists across images; the status-bar temp-text
/// readout flashes on every command (#47, viv.c:7673/7680).
fn animation_rate_step(hwnd: HWND, decrease: bool) {
    // SAFETY: the borrow spans the one-field update.
    let _ = (unsafe { state_of(hwnd) }).map(|state| {
        state.animation_rate_pos = rate_step(state.animation_rate_pos, decrease);
    });
    flash_animation_rate(hwnd);
}

fn animation_rate_reset(hwnd: HWND) {
    // SAFETY: the borrow spans the one-field update.
    let _ = (unsafe { state_of(hwnd) }).map(|state| {
        state.animation_rate_pos = RATE_ONE;
    });
    flash_animation_rate(hwnd);
}

/// The jump budget a command spends (upstream `config_short_jump` /
/// `config_medium_jump` / `config_long_jump`, config.c:72-74 — 500/1000/
/// 2000 ms defaults; ini-overridable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpKind {
    Short,
    Medium,
    Long,
}

impl JumpKind {
    fn budget_ms(self, config: &crate::config::Config) -> i32 {
        match self {
            Self::Short => config.short_jump,
            Self::Medium => config.medium_jump,
            Self::Long => config.long_jump,
        }
    }
}

/// The Animation jump commands (#38; upstream `_viv_frame_skip`, viv.c:
/// 10056-10103): walk `budget` milliseconds of RAW frame delays forward or
/// backward. Unlike the step family this neither pauses playback nor
/// resets the looped mark (the upstream handler touches neither), and it
/// re-anchors the timeline per step (the walk's own bookkeeping,
/// viv.c:10072-10073).
fn animation_jump(hwnd: HWND, kind: JumpKind, backward: bool) {
    let now = qpc_now();
    // SAFETY: the borrow spans the config read and the position walk; the
    // GDI calls below run after it drops.
    let walked = (unsafe { state_of(hwnd) }).and_then(|state| {
        let budget = kind.budget_ms(&state.config);
        let direction = if backward { -budget } else { budget };
        state
            .image
            .as_mut()
            .map(|image| image.frame_skip(now, direction))
    });
    if walked.unwrap_or(false) {
        refresh_status(hwnd);
        // SAFETY: queues a WM_PAINT; never pumps messages.
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
    // SAFETY: the borrow spans the playlist mutation and one field store —
    // nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        // Any picked file exits random mode (viv.c:2383-2388 — upstream
        // clears right after GetOpenFileName succeeds, before the
        // open/add branch).
        state.random_search = None;
        if add {
            // Add File appends (viv.c:2396-2402): the current
            // file becomes the first entry when the list is
            // empty, then the pick — no clear, no home, the
            // display stays.
            playlist_add_current_if_empty(state);
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
    // SAFETY: the borrow spans the playlist clear, the memory store and
    // the random exit — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        // A picked folder exits random mode (viv.c:2424-2429).
        state.random_search = None;
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
/// The menu bar from the command table (upstream `_viv_create_menu`,
/// viv.c:12314-12399). The accelerator label on each item is the
/// command's FIRST registered binding from `keys` (upstream
/// `_viv_key_list->start[...]`, viv.c:12365-12367 — the live map, so
/// custom bindings relabel the menu after an Options OK rebuilds it).
fn create_menu_bar(keys: &crate::keys::KeyMap) -> HMENU {
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
            menu::Entry::Item { loc, parent, cmd } => {
                // The accelerator label: the hint command's first binding
                // (the visible Delete row reads the RECYCLE-delete key,
                // upstream's own remap, viv.c:12356-12361) via
                // GetKeyNameTextW (layout-localized like upstream,
                // `_viv_vk_to_text` viv.c:12221-12261), composed by the
                // pure `menu::key_label`.
                let hint = menu::hint_cmd(cmd);
                let label = keys
                    .first(hint)
                    .and_then(|k| vk_text(k.vk).map(|t| menu::key_label(k, &t)));
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
            // MF_OWNERDRAW rows never reach the menu bar (upstream's
            // build filter, viv.c:12328-12377) — the command stays
            // dispatchable and shortcut-bindable, just invisible here.
            menu::Entry::HiddenItem { .. } => {}
        }
    }
    bar
}

/// Rebuild the menu bar after the bindings changed (upstream's Options OK:
/// `_viv_key_list_copy` then `_viv_create_menu` + reattach + destroy-old,
/// viv.c:8779-8800). A new bar is built from the live key map so custom
/// bindings relabel their items; the swap attaches only when a bar is
/// currently attached (upstream's GetMenu guard — fullscreen detaches it,
/// so the rebuilt bar waits in the state for the reattach).
pub(crate) fn rebuild_menu_bar(hwnd: HWND) {
    // SAFETY: the borrow spans only the KeyMap clone.
    let keys = (unsafe { state_of(hwnd) }).map(|s| s.config.keys.clone());
    let Some(keys) = keys else { return };
    let bar = create_menu_bar(&keys);
    // SAFETY: fresh borrow for the swap; nothing below pumps.
    unsafe {
        if let Some(state) = state_of(hwnd) {
            // SAFETY: read-only menu query on the live window.
            let attached = GetMenu(hwnd);
            if !attached.is_invalid() && !bar.is_invalid() {
                // SAFETY: the new bar is freshly built; the window takes
                // ownership here (the old one is destroyed after).
                let _ = SetMenu(hwnd, Some(bar));
            }
            if !state.menu.is_invalid() {
                // SAFETY: the state owns the old bar and nothing else
                // references it — SetMenu above already replaced it when
                // it was attached, and a detached bar has no window user.
                let _ = DestroyMenu(state.menu);
            }
            state.menu = bar;
        }
    }
}

/// The key-name half of an accelerator label (upstream `_viv_vk_to_text`,
/// viv.c:12221-12261): scan code from the thread's keyboard layout, then
/// `GetKeyNameTextW` with the extended-key bit for the navigation keys
/// riviv registers (upstream's full extended list covers keys riviv has
/// no default binding for). None = no name (the item then shows no
/// accelerator).
pub(crate) fn vk_text(vk: u16) -> Option<String> {
    // SAFETY: read-only layout query for this thread.
    let hkl = unsafe { GetKeyboardLayout(GetCurrentThreadId()) };
    // SAFETY: pure VK→scan-code mapping.
    let scan = unsafe { MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC, Some(hkl)) };
    if scan == 0 {
        return None;
    }
    let mut lparam = (scan as i32) << 16;
    // The extended bit (1 << 24) for keys that live only on the extended
    // cluster, and the "don't care" bit (1 << 25) for the modifiers —
    // upstream's full lists (viv.c:12243-12254). Custom bindings (#25)
    // can name any VK, so the whole table ships.
    const EXTENDED: &[u16] = &[
        0x21, // VK_PRIOR
        0x22, // VK_NEXT
        0x23, // VK_END
        0x24, // VK_HOME
        0x25, // VK_LEFT
        0x26, // VK_UP
        0x27, // VK_RIGHT
        0x28, // VK_DOWN
        0x2d, // VK_INSERT
        0x2e, // VK_DELETE
        0x90, // VK_NUMLOCK
        0x6f, // VK_DIVIDE
    ];
    const DONT_CARE: &[u16] = &[
        0x11, // VK_CONTROL
        0x10, // VK_SHIFT
        0x12, // VK_MENU
        0x5b, // VK_LWIN
        0x5c, // VK_RWIN
    ];
    if EXTENDED.contains(&vk) {
        lparam |= 1 << 24;
    } else if DONT_CARE.contains(&vk) {
        lparam |= 1 << 25;
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
/// grays for the state right now. The bar comes from GetMenu, NOT the
/// wParam menu: TrackPopupMenu delivers its own WM_INITMENU with the
/// popup's handle, and both upstream and the cross-process probes address
/// the bar.
fn on_initmenu(hwnd: HWND) {
    // SAFETY: read-only query of the window's own menu.
    let bar = unsafe { GetMenu(hwnd) };
    refresh_menu_state(hwnd, bar);
}

/// The shared menu-state application (upstream `_viv_check_menus`,
/// viv.c:7071-7198 — ONE function serves the bar's WM_INITMENU and the
/// context menu's build, viv.c:3071/3530): snapshot the live state, run
/// the fullscreen-slideshow side refreshes (viv.c:7081-7090 — BEFORE any
/// menu-validity concern; upstream never validates the HMENU, and the
/// side effects must survive a missing bar), then apply every command's
/// check/gray to `target` by command id — the MF_BYCOMMAND operations walk
/// the whole menu tree, submenus included.
fn refresh_menu_state(hwnd: HWND, target: HMENU) {
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
            show_status: state.config.show_status != 0,
            show_controls: state.config.show_controls != 0,
            fullscreen: state.fullscreen,
            one_to_one,
            slideshow: state.slideshow,
            slideshow_rate_ms: state.config.slideshow_rate as u32,
            animation_playing: state.animation_playing,
            nav_sort: playlist::SortMode::from_config(state.config.nav_sort),
            nav_sort_ascending: state.config.nav_sort_ascending != 0,
            shuffle: state.config.shuffle != 0,
            // The clipboard quartet's gray (#41): a current file with no
            // not-found / failed verdict standing (upstream's
            // is_image_enabled, viv.c:7103).
            image_enabled: clipboard::image_gate(
                state.nav_current.is_some(),
                state.status_file_not_found,
                state.status_load_failed,
            ),
            // The #46 fit trio and on-top radios (viv.c:7127-7136): the
            // Fill row reads the CURRENT mode's fill config (upstream's
            // `_viv_check_menus` branch, viv.c:7098-7100).
            allow_shrinking: state.config.allow_shrinking != 0,
            keep_aspect: state.config.keep_aspect_ratio != 0,
            fill_window: if state.fullscreen {
                state.config.fullscreen_fill_window != 0
            } else {
                state.config.fill_window != 0
            },
            ontop: state.config.ontop,
        }
    });
    let Some(state) = snapshot else {
        return;
    };
    // Upstream refreshes the status bar, the strip and the on-top state
    // right here when a slideshow runs fullscreen (viv.c:7081-7090) — a
    // slideshow started while fullscreen never passes through the normal
    // refresh points' windowed paths before the menu opens. This runs
    // BEFORE the target-validity guard: a detached bar (fullscreen) must
    // not skip the side effects.
    if state.fullscreen && state.slideshow {
        refresh_status(hwnd);
        refresh_toolbar(hwnd);
        update_ontop(hwnd);
    }
    if target.is_invalid() {
        return;
    }
    for cmd in menu::Cmd::ALL {
        // The Rate submenu's checks render as radio dots (upstream passes
        // MFT_RADIOCHECK in the flags for that family, viv.c:7165-7189).
        let radio: u32 = if menu::radio(cmd) {
            MFT_RADIOCHECK.0
        } else {
            0
        };
        let flags: u32 = if menu::checked(cmd, &state) {
            (MF_CHECKED | MF_BYCOMMAND).0 | radio
        } else {
            (MF_UNCHECKED | MF_BYCOMMAND).0 | radio
        };
        // SAFETY: target is the caller's live menu (the bar or the context
        // popup) — a by-command op on an id the menu lacks just fails.
        let _ = unsafe { CheckMenuItem(target, u32::from(cmd.id()), flags) };
        let enable: MENU_ITEM_FLAGS = if menu::enabled(cmd, &state) {
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
        // SAFETY: target is the caller's live menu.
        let _ = unsafe { EnableMenuItem(target, u32::from(cmd.id()), enable) };
    }
}

/// WM_CONTEXTMENU (upstream viv.c:3376-3545, #49): the FULL context menu
/// from `menu::CONTEXT_TABLE` — navigation, the Rate submenu, the fit
/// rows, the Sort submenu, the shell verbs, rotations, clipboard, file
/// management, Properties/Options/Exit — with NO fullscreen or bar gate
/// (upstream has none); the only gated row is the Menu recovery entry,
/// which appears while the bar hides (viv.c:3427). The keyboard invocation
/// (-1/-1, Shift+F10 / Menu key) centers the popup on the window
/// (viv.c:3386-3398) — keyboard-only users get the same menu.
fn on_contextmenu(hwnd: HWND, lparam: LPARAM) {
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
    // SAFETY: the borrow spans only the two reads — nothing pumps (the
    // rebuild_menu_bar pattern for the KeyMap clone).
    let setup = (unsafe { state_of(hwnd) })
        .map(|state| (state.config.show_menu != 0, state.config.keys.clone()));
    let Some((show_menu, keys)) = setup else {
        return;
    };
    let popup = create_context_menu(show_menu, &keys);
    if popup.is_invalid() {
        return;
    }
    // The check/gray state the rows carry (upstream runs `_viv_check_menus`
    // over the popup BEFORE tracking, viv.c:3530 — same shared walk as the
    // bar, side effects included).
    refresh_menu_state(hwnd, popup);
    // The cursor shows for the menu and the popup flag keeps the idle
    // timer from hiding it mid-tracking (viv.c:3532-3534 + 14595). The
    // effects run outside the borrow (the apply_cursor pattern).
    // SAFETY: short borrow — the flag write and the pure cursor step.
    let show = (unsafe { state_of(hwnd) }).map(|state| {
        state.in_popup_menu = true;
        state.cursor.show()
    });
    if let Some(effects) = show {
        apply_cursor(hwnd, effects);
    }
    // The MSDN menu-dismissal pattern: foreground the owner before
    // TrackPopupMenu and nudge it with WM_NULL after, so the menu closes
    // when focus leaves (a denial is ignored — the menu still works; a
    // riviv recovery-slice behavior kept — upstream tracks bare,
    // viv.c:3536).
    // SAFETY: our own live window.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    // SAFETY: our popup shown at the message's screen point over our
    // window; blocks until dismissed and posts WM_COMMAND on pick. The
    // pick's handler runs INSIDE this modal loop — including Exit, which
    // destroys the window and frees the state — so nothing below may
    // assume either lives.
    let _ = unsafe { TrackPopupMenu(popup, flags, x, y, None, hwnd, None) };
    // viv.c:3539-3540: drop the flag and reconcile the cursor — every
    // access None-tolerant (an Exit pick leaves state_of reading None).
    // SAFETY: short borrow on a window that may already be gone.
    let after = (unsafe { state_of(hwnd) }).map(|state| {
        state.in_popup_menu = false;
        let conditions = cursor_conditions(hwnd, state);
        state.cursor.update(&conditions)
    });
    if let Some(effects) = after {
        apply_cursor(hwnd, effects);
    }
    // SAFETY: our popup, whose tracking ended above — standalone menus
    // stay destroyable after the owner window is gone.
    let _ = unsafe { DestroyMenu(popup) };
    // SAFETY: empty nudge on our own window; a dead window just fails it.
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
}

/// Build the right-click popup from the context table (upstream's build
/// loop, viv.c:3400-3527): replay `menu::context_rows` appending into the
/// root popup, switching to a fresh popup on Push rows and back on Pop
/// rows (upstream's curmenu — the nesting is one level deep, so Pop lands
/// on the root). The accelerator on each command row is the HINT command's
/// first binding (the visible Delete row reads the recycle-delete key,
/// viv.c:3454-3472). Returns an invalid HMENU on failure (the caller
/// skips the popup, like a failed bar).
fn create_context_menu(show_menu: bool, keys: &crate::keys::KeyMap) -> HMENU {
    // SAFETY: pure menu-object construction; no window involvement.
    let Ok(root) = (unsafe { CreatePopupMenu() }) else {
        return HMENU::default();
    };
    let mut cur = root;
    for row in menu::context_rows(show_menu) {
        match row {
            menu::ContextRow::Command { cmd, label } => {
                let hint = menu::hint_cmd(cmd);
                let key = keys
                    .first(hint)
                    .and_then(|k| vk_text(k.vk).map(|t| menu::key_label(k, &t)));
                let text = to_wide(&menu::item_text(loc::get(label), key.as_deref()));
                // SAFETY: text outlives the append.
                let _ = unsafe {
                    AppendMenuW(cur, MF_STRING, usize::from(cmd.id()), PCWSTR(text.as_ptr()))
                };
            }
            menu::ContextRow::Popup { label, .. } => {
                // SAFETY: fresh popup creation; a failure stores an invalid
                // handle whose rows append nowhere (degraded, like the bar
                // build's failed popups).
                let popup = unsafe { CreatePopupMenu() }.unwrap_or_default();
                let text = to_wide(loc::get(label));
                // SAFETY: text outlives the append; the popup handle moves
                // into its parent menu here.
                let _ = unsafe {
                    AppendMenuW(
                        cur,
                        MF_STRING | MF_POPUP,
                        popup.0 as usize,
                        PCWSTR(text.as_ptr()),
                    )
                };
                cur = popup;
            }
            // Upstream's pop arm (viv.c:3482-3487): back to the top level.
            menu::ContextRow::Pop => cur = root,
            menu::ContextRow::Separator => {
                // SAFETY: a separator append carries no text.
                let _ = unsafe { AppendMenuW(cur, MF_SEPARATOR, 0, PCWSTR::null()) };
            }
        }
    }
    root
}

/// WM_COMMAND dispatch (upstream `_viv_command`, viv.c:1658-2580: the
/// switch over command ids onto the same action functions the keys use —
/// menu triggers are never key repeats, so the repeat-wait gates of the
/// keyboard path do not apply, matching upstream's is_key_repeat=0).
fn on_command(hwnd: HWND, cmd: menu::Cmd) {
    match cmd {
        menu::Cmd::FileOpenFile => open_image_via_dialog(hwnd, false),
        menu::Cmd::FileOpenFolder => open_folder_via_dialog(hwnd),
        // The Everything search dialog (#22; upstream viv.c:2510-2516):
        // modal over the viewer; the Open flavor clears the playlist via
        // the query reply, the Add flavor appends.
        menu::Cmd::FileOpenEverythingSearch => everything::open_search_dialog(hwnd, false),
        menu::Cmd::FileAddEverythingSearch => everything::open_search_dialog(hwnd, true),
        menu::Cmd::FileAddFile => open_image_via_dialog(hwnd, true),
        // The #42 shell family (upstream viv.c:2477-2519: Preview/Print/
        // Wallpaper/Close then Edit/Location/Properties) — every handler
        // is the bare current-file gate plus one shell call, all
        // fail-soft.
        cmd @ (menu::Cmd::FileEdit
        | menu::Cmd::FilePreview
        | menu::Cmd::FilePrint
        | menu::Cmd::FileProperties) => shell::run_verb(hwnd, cmd),
        menu::Cmd::FileOpenFileLocation => shell::open_file_location(hwnd),
        menu::Cmd::FileSetDesktopWallpaper => shell::set_desktop_wallpaper(hwnd),
        // Upstream `_viv_blank` (viv.c:7908): this just clears the image —
        // riviv's blank path IS that port.
        menu::Cmd::FileClose => blank_display(hwnd),
        // The #43 file-management family. The visible Delete row probes
        // Shift AT DISPATCH (viv.c:2458-2460) — its menu identity doubles
        // as both deletes; the two hidden rows are the fixed flavors the
        // Del / Shift+Del keys land on.
        menu::Cmd::FileDelete => {
            // SAFETY: GetKeyState reads this thread's key state (the
            // command runs on the window's thread; #44's sandbox lesson
            // is about cross-process reads, not this).
            let permanently = (unsafe { GetKeyState(VK_SHIFT.0 as i32) } as u16) & 0x8000 != 0;
            delete_current(hwnd, permanently);
        }
        menu::Cmd::FileDeleteRecycle => delete_current(hwnd, false),
        menu::Cmd::FileDeletePermanently => delete_current(hwnd, true),
        // `_viv_rename` (viv.c:7365-7371): the bare current-file gate,
        // then the modal (its OK arm owns the shell call and the state
        // follow-up).
        menu::Cmd::FileRename => {
            // SAFETY: read-only clone; the modal below pumps messages.
            let path = (unsafe { state_of(hwnd) })
                .and_then(|s| s.nav_current.as_ref().map(|e| e.path.clone()));
            if let Some(path) = path {
                crate::rename_dlg::open(hwnd, &path);
            }
        }
        // `_viv_edit_rotate` pair (viv.c:2490-2495): the waited shell
        // verb + the in-memory rotation.
        menu::Cmd::EditRotate90 => rotate_current(hwnd, false),
        menu::Cmd::EditRotate270 => rotate_current(hwnd, true),
        // Copy To / Move To (viv.c:2496-2568): the save dialog + the file
        // operation, no state follow-up.
        menu::Cmd::EditCopyTo => copy_move_to(hwnd, true),
        menu::Cmd::EditMoveTo => copy_move_to(hwnd, false),
        menu::Cmd::FileExit => {
            // Upstream `_viv_exit` (viv.c:1883-1888) saves the config and
            // quits the pump; riviv's WM_DESTROY does both on the way out.
            // SAFETY: legal on the owning thread; synchronously runs
            // WM_DESTROY/WM_NCDESTROY with no borrow live.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        // The Edit → clipboard family (#41; upstream viv.c:2335-2353, the
        // same order).
        menu::Cmd::EditCopy => clipboard::copy_current(hwnd, false),
        menu::Cmd::EditCopyFilename => clipboard::copy_filename(hwnd),
        menu::Cmd::EditCopyImage => clipboard::copy_image(hwnd),
        // Upstream routes the command through WM_PASTE (viv.c:2347-2349)
        // so anything else that posts the message lands on the same path.
        menu::Cmd::EditPaste => {
            // SAFETY: hwnd is live and owned by this thread; WM_PASTE runs
            // inline (the clipboard session pumps nothing).
            unsafe {
                let _ = SendMessageW(hwnd, WM_PASTE, Some(WPARAM(0)), Some(LPARAM(0)));
            }
        }
        menu::Cmd::EditCut => clipboard::copy_current(hwnd, true),
        menu::Cmd::ViewCaption => toggle_caption(hwnd),
        menu::Cmd::ViewThickFrame => toggle_thickframe(hwnd),
        menu::Cmd::ViewMenu => toggle_menu(hwnd),
        menu::Cmd::ViewStatus => toggle_status(hwnd),
        menu::Cmd::ViewControls => toggle_controls(hwnd),
        // The Preset trio (#46; viv.c:1990-2013 — assign all five configs,
        // one frame rebuild).
        menu::Cmd::ViewPreset1 => apply_preset(hwnd, crate::frame::Preset::Minimal),
        menu::Cmd::ViewPreset2 => apply_preset(hwnd, crate::frame::Preset::Compact),
        menu::Cmd::ViewPreset3 => apply_preset(hwnd, crate::frame::Preset::Normal),
        menu::Cmd::ViewFullscreen => toggle_fullscreen(hwnd),
        menu::Cmd::ViewOneToOne => toggle_one_to_one(hwnd),
        // The Window Size quartet (#46; viv.c:2076-2240) — the #24 sizing
        // engine behind the menu rows.
        cmd @ (menu::Cmd::ViewWindowSize50
        | menu::Cmd::ViewWindowSize100
        | menu::Cmd::ViewWindowSize200
        | menu::Cmd::ViewWindowSizeAutoFit) => {
            if let Some(kind) = cmd.window_size_kind() {
                window_size_to_image(hwnd, kind);
            }
        }
        // Refresh (#46; `_viv_refresh`, viv.c:14539-14552): drop the caches
        // and re-read the current file from disk.
        menu::Cmd::ViewRefresh => refresh_current(hwnd),
        // The fit trio (#46; viv.c:2015-2044): 1:1 dies, the config flips,
        // the size pass re-anchors at the new fit inputs, one repaint. The
        // Fill row toggles the FULLSCREEN fill config while fullscreen —
        // upstream's own quirk.
        menu::Cmd::ViewAllowShrinking => toggle_fit_input(hwnd, FitInput::AllowShrinking),
        menu::Cmd::ViewKeepAspect => toggle_fit_input(hwnd, FitInput::KeepAspect),
        menu::Cmd::ViewFillWindow => toggle_fit_input(hwnd, FitInput::FillWindow),
        // The on-top radios (#46; viv.c:2317-2328).
        menu::Cmd::ViewOntopAlways => set_ontop(hwnd, SetOntop::Always),
        menu::Cmd::ViewOntopWhilePlaying => set_ontop(hwnd, SetOntop::WhilePlaying),
        menu::Cmd::ViewOntopNever => set_ontop(hwnd, SetOntop::Never),
        // Upstream's two fit commands both collapse the zoom position back
        // to the fit level (`VIV_ID_VIEW_BESTFIT` zeroes the zoom position
        // and refits, `VIV_ID_VIEW_ZOOM_RESET` drops 1:1 and the zoom
        // position — viv.c:1678-1687/2059-2064); riviv's Ctrl+0 reset is
        // that action.
        menu::Cmd::ViewBestFit | menu::Cmd::ViewZoomReset => zoom_reset(hwnd),
        // The Pan/Scan family (#44; upstream viv.c:2246-2313): the six
        // size steps move the per-axis factor indices, the eight arrows
        // and Center the pan position, Reset both to identity.
        menu::Cmd::ViewPanScanIncreaseSize => panscan_step(hwnd, 1, 1),
        menu::Cmd::ViewPanScanDecreaseSize => panscan_step(hwnd, -1, -1),
        menu::Cmd::ViewPanScanIncreaseWidth => panscan_step(hwnd, 1, 0),
        menu::Cmd::ViewPanScanDecreaseWidth => panscan_step(hwnd, -1, 0),
        menu::Cmd::ViewPanScanIncreaseHeight => panscan_step(hwnd, 0, 1),
        menu::Cmd::ViewPanScanDecreaseHeight => panscan_step(hwnd, 0, -1),
        menu::Cmd::ViewPanScanMoveUp => panscan_pan(hwnd, 0, -5),
        menu::Cmd::ViewPanScanMoveDown => panscan_pan(hwnd, 0, 5),
        menu::Cmd::ViewPanScanMoveLeft => panscan_pan(hwnd, -5, 0),
        menu::Cmd::ViewPanScanMoveRight => panscan_pan(hwnd, 5, 0),
        menu::Cmd::ViewPanScanMoveUpLeft => panscan_pan(hwnd, -5, -5),
        menu::Cmd::ViewPanScanMoveUpRight => panscan_pan(hwnd, 5, -5),
        menu::Cmd::ViewPanScanMoveDownLeft => panscan_pan(hwnd, -5, 5),
        menu::Cmd::ViewPanScanMoveDownRight => panscan_pan(hwnd, 5, 5),
        menu::Cmd::ViewPanScanMoveCenter => panscan_center(hwnd),
        menu::Cmd::ViewPanScanReset => panscan_reset(hwnd),
        menu::Cmd::ViewZoomIn => zoom_step_centered(hwnd, false),
        menu::Cmd::ViewZoomOut => zoom_step_centered(hwnd, true),
        // The Options dialog (#24) — modal over the viewer; commits into
        // the live config on OK (instant effect + save).
        menu::Cmd::ViewOptions => crate::options_dlg::open(hwnd),
        // The slideshow family (#37; upstream viv.c:2072/1838-1851/1947-
        // 1963): the start-only toggle, the running toggle, the preset
        // steps and rows, and the Custom dialog.
        menu::Cmd::ViewSlideshow => slideshow_start(hwnd),
        menu::Cmd::SlideshowPause => slideshow_toggle(hwnd),
        // The toolbar-only pair (#45; upstream viv.c:1818-1831): both are
        // the same running-state TOGGLE, each gated to its half — Play
        // fires only when NOT running, Pause only when running.
        menu::Cmd::SlideshowPlayOnly => {
            // SAFETY: the borrow spans only the running read.
            if (unsafe { state_of(hwnd) }).is_some_and(|state| !state.slideshow) {
                slideshow_toggle(hwnd);
            }
        }
        menu::Cmd::SlideshowPauseOnly => {
            // SAFETY: the borrow spans only the running read.
            if (unsafe { state_of(hwnd) }).is_some_and(|state| state.slideshow) {
                slideshow_toggle(hwnd);
            }
        }
        menu::Cmd::SlideshowRateDecrease => slideshow_step(hwnd, true),
        menu::Cmd::SlideshowRateIncrease => slideshow_step(hwnd, false),
        menu::Cmd::SlideshowRateCustom => slideshow_open_custom_dialog(hwnd),
        // The 17 preset rows (upstream viv.c:1947-1962) — the ms value is
        // part of the command itself.
        cmd @ (menu::Cmd::SlideshowRate250
        | menu::Cmd::SlideshowRate500
        | menu::Cmd::SlideshowRate1000
        | menu::Cmd::SlideshowRate2000
        | menu::Cmd::SlideshowRate3000
        | menu::Cmd::SlideshowRate4000
        | menu::Cmd::SlideshowRate5000
        | menu::Cmd::SlideshowRate6000
        | menu::Cmd::SlideshowRate7000
        | menu::Cmd::SlideshowRate8000
        | menu::Cmd::SlideshowRate9000
        | menu::Cmd::SlideshowRate10000
        | menu::Cmd::SlideshowRate20000
        | menu::Cmd::SlideshowRate30000
        | menu::Cmd::SlideshowRate40000
        | menu::Cmd::SlideshowRate50000
        | menu::Cmd::SlideshowRate60000) => {
            if let Some(rate_ms) = cmd.slideshow_rate_ms() {
                slideshow_set_rate(hwnd, rate_ms);
            }
        }
        // The animation family (#38; upstream viv.c:1851-1944 — the pause
        // toggle, the six jumps (the short/long quartet has no menu row
        // but the same WM_COMMAND path, keyboard-only), the four frame
        // commands, and the three rate commands).
        menu::Cmd::AnimationPlayPause => animation_pause(hwnd),
        menu::Cmd::AnimationJumpForwardMedium => animation_jump(hwnd, JumpKind::Medium, false),
        menu::Cmd::AnimationJumpBackwardMedium => animation_jump(hwnd, JumpKind::Medium, true),
        menu::Cmd::AnimationJumpForwardShort => animation_jump(hwnd, JumpKind::Short, false),
        menu::Cmd::AnimationJumpBackwardShort => animation_jump(hwnd, JumpKind::Short, true),
        menu::Cmd::AnimationJumpForwardLong => animation_jump(hwnd, JumpKind::Long, false),
        menu::Cmd::AnimationJumpBackwardLong => animation_jump(hwnd, JumpKind::Long, true),
        menu::Cmd::AnimationFrameStep => frame_command(hwnd, |image, now| image.frame_step(now)),
        menu::Cmd::AnimationFramePrev => frame_command(hwnd, |image, now| image.frame_prev(now)),
        menu::Cmd::AnimationFirstFrame => frame_command(hwnd, |image, now| image.frame_first(now)),
        menu::Cmd::AnimationLastFrame => frame_command(hwnd, |image, now| image.frame_last(now)),
        menu::Cmd::AnimationRateDecrease => animation_rate_step(hwnd, true),
        menu::Cmd::AnimationRateIncrease => animation_rate_step(hwnd, false),
        menu::Cmd::AnimationRateReset => animation_rate_reset(hwnd),
        menu::Cmd::NavNext => {
            nav_next(hwnd, false, true, false);
        }
        menu::Cmd::NavPrev => {
            nav_next(hwnd, true, true, false);
        }
        menu::Cmd::NavHome => home_open(hwnd, false, false),
        menu::Cmd::NavEnd => home_open(hwnd, true, false),
        // The #39 sort family (upstream viv.c:1750-1816): the five mode
        // rows re-click the active mode into a direction flip and pick a
        // new mode with its default direction; the direction pair sets the
        // flag outright. Every sort change clears the preload/last caches
        // WITHOUT re-preloading (viv.c:1791-1794/1801-1814 — the next
        // in-flight load's completion or the next navigation re-arms).
        cmd @ (menu::Cmd::NavSortName
        | menu::Cmd::NavSortFullPath
        | menu::Cmd::NavSortSize
        | menu::Cmd::NavSortDateModified
        | menu::Cmd::NavSortDateCreated) => {
            if let Some(mode) = cmd.sort_mode() {
                sort_click(hwnd, mode);
            }
        }
        menu::Cmd::NavSortAscending => sort_set_direction(hwnd, true),
        menu::Cmd::NavSortDescending => sort_set_direction(hwnd, false),
        // Toggle shuffle (upstream viv.c:1723-1748): turning it OFF frees
        // the shuffle order (the index array); on stays lazy — the next
        // navigation builds a fresh order (`_viv_do_initial_shuffle`).
        // Both directions clear the caches and re-preload when idle
        // (viv.c:1738-1747).
        menu::Cmd::NavShuffle => shuffle_toggle(hwnd),
        menu::Cmd::NavJumpTo => crate::jumpto_dlg::open(hwnd),
        menu::Cmd::HelpCommandLineOptions => show_usage(hwnd),
        menu::Cmd::HelpAbout => show_about(hwnd),
    }
}

/// A sort-mode menu click (upstream viv.c:1756-1789): same mode flips the
/// direction, a different mode lands on its default direction. The config
/// write persists through the exit save (the `sort`/`sort_ascending` ini
/// keys). Every change clears the caches and terminates an in-flight
/// PRELOAD (viv.c:1791-1793 — `_viv_clear_loading_preload` touches only a
/// preload load, never a foreground one; the parked slot's drop is that
/// terminate).
fn sort_click(hwnd: HWND, mode: playlist::SortMode) {
    // SAFETY: the borrow spans only the config stores and the cache drops
    // — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        let (mode, ascending) = playlist::apply_sort_click(
            mode,
            playlist::SortMode::from_config(state.config.nav_sort),
            state.config.nav_sort_ascending != 0,
        );
        state.config.nav_sort = mode as i32;
        state.config.nav_sort_ascending = i32::from(ascending);
        state.preload = None;
        state.last_cache = None;
    }
}

/// The Ascending/Descending rows (upstream viv.c:1798-1816): set the flag
/// directly — no mode change, no toggle. The cache clear matches the mode
/// rows (viv.c:1801-1814).
fn sort_set_direction(hwnd: HWND, ascending: bool) {
    // SAFETY: the borrow spans only the config store and the cache drops
    // — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.nav_sort_ascending = i32::from(ascending);
        state.preload = None;
        state.last_cache = None;
    }
}

/// The Shuffle row (upstream viv.c:1723-1748): flip the config; OFF frees
/// the shuffle order so a later ON re-shuffles fresh. Both directions
/// clear the caches (viv.c:1738-1741) and re-preload when no load is in
/// flight (viv.c:1744-1747).
fn shuffle_toggle(hwnd: HWND) {
    let mut re_preload = false;
    // SAFETY: the borrow spans the flag flip, the order drop and the
    // cache drops — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.shuffle = i32::from(state.config.shuffle == 0);
        if state.config.shuffle == 0 {
            state.playlist.drop_shuffle();
        }
        state.preload = None;
        state.last_cache = None;
        re_preload = state.session.is_none();
    }
    if re_preload {
        request_preload(hwnd);
    }
}

/// View→Menu (upstream `VIV_ID_VIEW_MENU`, viv.c:1975-1978): flip
/// `config_show_menu` and rebuild the frame. In fullscreen the flip still
/// lands (upstream's `_viv_update_frame` early-returns there, viv.c:9828 —
/// the exit rebuild applies the new config to the restored window).
fn toggle_menu(hwnd: HWND) {
    // SAFETY: the borrow spans the config flip — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.show_menu = i32::from(state.config.show_menu == 0);
    }
    update_frame(hwnd);
}

/// View→Caption (`VIV_ID_VIEW_CAPTION`, viv.c:1966-1970) and View→Frame
/// (`VIV_ID_VIEW_THICKFRAME`, viv.c:1971-1974) — the two hidden style-bit
/// toggles (#46): flip the config, then the same frame rebuild (the
/// WS_CAPTION|WS_SYSMENU / WS_THICKFRAME pair is recomputed inside).
fn toggle_caption(hwnd: HWND) {
    // SAFETY: the borrow spans the config flip — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.show_caption = i32::from(state.config.show_caption == 0);
    }
    update_frame(hwnd);
}

fn toggle_thickframe(hwnd: HWND) {
    // SAFETY: the borrow spans the config flip — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.show_thickframe = i32::from(state.config.show_thickframe == 0);
    }
    update_frame(hwnd);
}

/// View→Status Bar (`VIV_ID_VIEW_STATUS`, viv.c:1979-1983): flip
/// `config_show_status` and rebuild (the bar is created/destroyed inside
/// the frame pass).
fn toggle_status(hwnd: HWND) {
    // SAFETY: the borrow spans the config flip — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.show_status = i32::from(state.config.show_status == 0);
    }
    update_frame(hwnd);
}

/// The three Preset rows and the CLI `/minimal`//compact` pair (#46;
/// upstream viv.c:1990-2013 / 4879-4888): all five chrome configs are
/// ASSIGNED (no carry-over), then the frame rebuild applies them.
fn apply_preset(hwnd: HWND, preset: crate::frame::Preset) {
    let t = preset.toggles();
    // SAFETY: the borrow spans the five config stores — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.show_menu = i32::from(t.menu);
        state.config.show_status = i32::from(t.status);
        state.config.show_controls = i32::from(t.controls);
        state.config.show_caption = i32::from(t.caption);
        state.config.show_thickframe = i32::from(t.thickframe);
    }
    update_frame(hwnd);
}

/// The View→On Top rows (#46; upstream viv.c:2317-2328): Always is a TOGGLE
/// (`!config_ontop` — from the while-playing value 2 the C `!` lands on 0,
/// a quirk kept bug-for-bug), While-Playing assigns 2, Never assigns 0;
/// every arm re-evaluates the topmost bit.
fn set_ontop(hwnd: HWND, mode: SetOntop) {
    // SAFETY: the borrow spans the config write — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.ontop = match mode {
            // The C toggle: 0↔1, and 2 collapses to 0.
            SetOntop::Always => i32::from(state.config.ontop == 0),
            SetOntop::WhilePlaying => 2,
            SetOntop::Never => 0,
        };
    }
    update_ontop(hwnd);
}

/// Which on-top row fired (the three WM_COMMAND arms' payload).
enum SetOntop {
    Always,
    WhilePlaying,
    Never,
}

/// `_viv_update_ontop` (upstream viv.c:9920-9940): push the topmost bit for
/// the mode RIGHT NOW — mode 1 always, mode 2 while a slideshow runs or an
/// animation plays (`_viv_frame_count > 1 && _viv_animation_play`; riviv's
/// `is_animated()` is the same total-frame-count verdict), anything else
/// never.
fn update_ontop(hwnd: HWND) {
    // SAFETY: the borrow spans only the mode/flag reads.
    let want = (unsafe { state_of(hwnd) }).is_some_and(|state| {
        let animating =
            state.animation_playing && state.image.as_ref().is_some_and(|i| i.is_animated());
        crate::frame::ontop_active(state.config.ontop, state.slideshow, animating)
    });
    // SAFETY: hwnd live; NOSIZE|NOMOVE|NOACTIVATE like upstream's
    // SetWindowPos (viv.c:9939) — fail-soft is harmless: the flag-free
    // z-order just stays until the next trigger re-runs this.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(if want { HWND_TOPMOST } else { HWND_NOTOPMOST }),
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE,
        )
    };
}

/// `_viv_update_frame` (upstream viv.c:9823-9925) — the one frame rebuild
/// every chrome change funnels through (#46 generalizes the menu and
/// controls arms to the full upstream contract): in fullscreen it is a
/// no-op (the borderless cover carries none of this chrome; the exit
/// rebuild applies the configs). Otherwise: get out of the maximized state
/// first, capture the old frame around the CURRENT style/menu/children,
/// apply the five configs (style bits, menu attach, status bar and toolbar
/// create/destroy), then shift the outer rect by the new-minus-old delta so
/// the VIEWPORT — the client minus the status bar and the strip — keeps its
/// exact on-screen rectangle, and re-maximize only when both the caption
/// and the thick frame survive (upstream's condition, viv.c:9921-9924 — a
/// borderless window must not re-zoom onto our resize borders).
fn update_frame(hwnd: HWND) {
    // The fullscreen skip FIRST (viv.c:9828).
    // SAFETY: read-only borrow ends inside is_some_and.
    if (unsafe { state_of(hwnd) }).is_some_and(|state| state.fullscreen) {
        return;
    }
    // Get out of the maximized state (viv.c:9832-9839) — the explicit rect
    // below would land on the zoomed placement otherwise.
    // SAFETY: read-only zoomed query on the live window.
    let was_maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    if was_maximized {
        // SAFETY: live window.
        let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    let mut client = RECT::default();
    // SAFETY: read-only client query; a failure reads the zeroed rect and
    // the shift collapses to a no-op.
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    // The old outer rect BEFORE anything changes: the CURRENT style, the
    // CURRENT menu attach and the CURRENT chrome-children heights
    // (upstream reads oldrect before the SetMenu/status/controls block,
    // viv.c:9840-9853).
    // SAFETY: read-only style query on the live window.
    let old_style = WINDOW_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32);
    // SAFETY: read-only menu query on the live window.
    let old_has_menu = !unsafe { GetMenu(hwnd) }.is_invalid();
    // SAFETY: the borrow spans only the height reads.
    let (old_status, old_controls) = (unsafe { state_of(hwnd) }).map_or((0, 0), |state| {
        (crate::status::height(state.status), state.controls.height())
    });
    let mut old_outer = client;
    // SAFETY: in/out rect valid; failure leaves a zero frame delta like
    // upstream's unchecked AdjustWindowRect.
    let _ = unsafe { AdjustWindowRect(&mut old_outer, old_style, old_has_menu) };
    old_outer.bottom += old_status + old_controls;
    // The five target values (the config was already written by the
    // caller — upstream's case arms, viv.c:1966-2013).
    // SAFETY: the borrow spans only the config reads and the handle copy.
    let (show_menu, show_status, show_controls, show_caption, show_thickframe, menu) =
        (unsafe { state_of(hwnd) }).map_or((true, true, true, true, true, HMENU::default()), |s| {
            (
                s.config.show_menu != 0,
                s.config.show_status != 0,
                s.config.show_controls != 0,
                s.config.show_caption != 0,
                s.config.show_thickframe != 0,
                s.menu,
            )
        });
    // The style bits (viv.c:9855-9871): caption and sysmenu travel
    // together, the thick frame alone.
    let mut new_style = old_style;
    if show_caption {
        new_style |= WS_CAPTION | WS_SYSMENU;
    } else {
        new_style &= !(WS_CAPTION | WS_SYSMENU);
    }
    if show_thickframe {
        new_style |= WS_THICKFRAME;
    } else {
        new_style &= !WS_THICKFRAME;
    }
    // Menu attach/detach (viv.c:9873-9889) — `menu` is the state's own bar
    // when attaching; None detaches. The BOOL return is ignored like
    // upstream (a failure keeps the old attach state, and the rect shift
    // still matches the menu-less frame).
    // SAFETY: hwnd is live and owned by this thread.
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
    // The chrome children (viv.c:9891-9892): create/destroy per config —
    // each ends with its own on_size docking pass against the current
    // client.
    status_show(hwnd, show_status);
    controls_show(hwnd, show_controls);
    // The new outer rect over the same client: the NEW style, the TARGET
    // menu attach and the AFTER children heights (viv.c:9894-9903).
    // SAFETY: the borrow spans only the height reads.
    let (new_status, new_controls) = (unsafe { state_of(hwnd) }).map_or((0, 0), |state| {
        (crate::status::height(state.status), state.controls.height())
    });
    let mut new_outer = client;
    // SAFETY: same call as above with the new style/menu.
    let _ = unsafe { AdjustWindowRect(&mut new_outer, new_style, show_menu && !menu.is_invalid()) };
    new_outer.bottom += new_status + new_controls;
    let mut window = RECT::default();
    // SAFETY: read-only outer-rect query, fail-soft like upstream's
    // unchecked GetWindowRect (viv.c:9905).
    let _ = unsafe { GetWindowRect(hwnd, &mut window) };
    // Shift by new-minus-old per edge over the same client (viv.c:9907-
    // 9913). Wrapping like the rest of riviv's rect math so pathological
    // values cannot panic.
    window.left = window
        .left
        .wrapping_add(new_outer.left.wrapping_sub(old_outer.left));
    window.top = window
        .top
        .wrapping_add(new_outer.top.wrapping_sub(old_outer.top));
    window.right = window
        .right
        .wrapping_add(new_outer.right.wrapping_sub(old_outer.right));
    window.bottom = window
        .bottom
        .wrapping_add(new_outer.bottom.wrapping_sub(old_outer.bottom));
    // SAFETY: read-modify-write of the style on the owning thread.
    unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, new_style.0 as isize) };
    // SAFETY: live window; re-asserts top like upstream's HWND_TOP
    // (viv.c:9918-9920) and applies the style + rect in one FRAMECHANGED
    // pass — the WM_SIZE it triggers re-docks whatever children remain.
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
    // Re-maximize only with both the caption and the resize frame present
    // (viv.c:9921-9924: "if there is no caption or thick frame we should
    // not allow maximize / avoid our resize borders when maximized").
    if was_maximized && show_caption && show_thickframe {
        // SAFETY: live window.
        let _ = unsafe { ShowWindow(hwnd, SW_MAXIMIZE) };
    }
}

/// `_viv_status_show` (upstream viv.c:10932-10963) — the status-bar half
/// of the frame rebuild (#46): create the bar when it should exist,
/// destroy it when it should not (upstream hides by DESTROYING, never
/// ShowWindow), then re-run the size pass so the remaining chrome docks
/// (upstream ends the function with `_viv_on_size`, viv.c:10961).
fn status_show(hwnd: HWND, show: bool) {
    // SAFETY: the borrow spans only the liveness read.
    let alive = (unsafe { state_of(hwnd) }).is_some_and(|state| !state.status.is_invalid());
    if show && !alive {
        // SAFETY: returns this exe's module handle; no side effects.
        if let Ok(hinstance) = unsafe { GetModuleHandleW(None) } {
            match crate::status::create(hwnd, hinstance.into()) {
                Ok(bar) => {
                    // SAFETY: the borrow spans only the field store;
                    // nothing below pumps.
                    if let Some(state) = unsafe { state_of(hwnd) } {
                        state.status = bar;
                    }
                }
                // Same graceful degradation as the startup create — a NULL
                // bar no-ops everywhere (upstream leaves the global NULL
                // on a CreateWindow failure too).
                Err(msg) => eprintln!("status bar unavailable: {msg}"),
            }
        }
    } else if !show && alive {
        // Take the handle out of the state INSIDE the borrow, destroy
        // OUTSIDE: DestroyWindow delivers messages (the child's teardown
        // plus a WM_PARENTNOTIFY here) and no future handler arm on those
        // may alias this borrow.
        let bar = {
            // SAFETY: the borrow spans only the take-and-clear.
            let Some(state) = (unsafe { state_of(hwnd) }) else {
                return;
            };
            std::mem::take(&mut state.status)
        };
        // SAFETY: our live child window, torn down on the owning thread.
        let _ = unsafe { DestroyWindow(bar) };
    }
    on_size(hwnd);
}

// ---- Toolbar (#45; upstream viv.c:10963-11088 / 11441-11704 / 2667-2691) ----

/// Gather the four dynamic button states from the live state (the inputs
/// of upstream `_viv_toolbar_update_buttons`, viv.c:11665-11704): the
/// slideshow flag, the render-equals-source compare for 1:1, and the
/// at-fit-level-outside-1:1 verdict for Best Fit. `None` = no toolbar.
fn toolbar_states(state: &WindowState, hwnd: HWND) -> Option<crate::toolbar::ButtonStates> {
    if !state.controls.is_alive() {
        return None;
    }
    // The 1:1 comparator is upstream's raw `_viv_get_render_size` equality
    // (viv.c:11686-11690) — zoom-level size only, panscan-blind by
    // construction, the same compare the menu check uses.
    let render_eq_image = state.image.as_ref().is_some_and(|image| {
        let (vp, src) = viewport_and_src(hwnd, state);
        let fit = fit_policy(state);
        let (rw, rh) = state.view.render_size(src.0, src.1, vp, fit);
        (rw, rh) == (image.width(), image.height())
    });
    let at_best_fit = state.view.level() == 0 && !state.view.is_one_to_one();
    Some(crate::toolbar::ButtonStates::compute(
        state.slideshow,
        render_eq_image,
        at_best_fit,
    ))
}

/// Push the dynamic button states (`_viv_toolbar_update_buttons` —
/// upstream calls it from `_viv_view_set`, the slideshow start/pause, the
/// load-reply adoption, `_viv_on_size` and the fullscreen-slideshow slice
/// of `_viv_check_menus`, viv.c:6571/6839/7088/7617/1654/2872).
fn refresh_toolbar(hwnd: HWND) {
    // SAFETY: the borrow spans the pure state gather; the TB sends run
    // outside it.
    let pushed = (unsafe { state_of(hwnd) }).and_then(|state| {
        toolbar_states(state, hwnd).map(|states| (state.controls.toolbar, states))
    });
    if let Some((toolbar, states)) = pushed {
        crate::toolbar::apply_states(toolbar, &states);
    }
}

/// Create/destroy the strip+toolbar+image-list set per `show` (upstream
/// `_viv_controls_show`, viv.c:10963-11088) and re-run the size pass —
/// upstream ends the function with `_viv_on_size()` whatever happened
/// (viv.c:11090).
fn controls_show(hwnd: HWND, show: bool) {
    // SAFETY: the borrow spans only the liveness check.
    let alive = (unsafe { state_of(hwnd) }).is_some_and(|state| state.controls.is_alive());
    if show && !alive {
        // SAFETY: module handle query, no side effects.
        match unsafe { GetModuleHandleW(None) } {
            Ok(hinstance) => {
                match crate::toolbar::create(hwnd, hinstance.into()) {
                    Ok(set) => {
                        // SAFETY: the borrow spans only the field store;
                        // the initial button states push below happens
                        // outside it.
                        if let Some(state) = unsafe { state_of(hwnd) } {
                            state.controls = set;
                        }
                    }
                    Err(msg) => {
                        // Same graceful degradation as the status bar
                        // (upstream's CreateWindow failures leave the
                        // globals NULL).
                        eprintln!("toolbar unavailable: {msg}");
                    }
                }
            }
            Err(e) => eprintln!("toolbar unavailable: GetModuleHandleW failed: {e}"),
        }
    } else if !show && alive {
        // SAFETY: the borrow spans only the set take-and-clear.
        if let Some(state) = unsafe { state_of(hwnd) } {
            let mut set = std::mem::take(&mut state.controls);
            crate::toolbar::destroy(&mut set);
        }
    }
    on_size(hwnd);
}

/// View → Controls (`VIV_ID_VIEW_CONTROLS`, viv.c:1985-1988): flip the
/// config and rebuild the frame around the strip appearing/disappearing
/// (in fullscreen the flip still lands; the rebuild waits for the exit,
/// like upstream's `_viv_update_frame` early return).
fn toggle_controls(hwnd: HWND) {
    // SAFETY: the borrow spans the config flip — nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.config.show_controls = i32::from(state.config.show_controls == 0);
    }
    update_frame(hwnd);
}

/// `_viv_start_move_window` (viv.c:14720-14729): enter the system move
/// loop by feeding the main window a synthetic WM_NCLBUTTONDOWN over its
/// caption, anchored at the live cursor. `from` is any window of ours —
/// the strip, the status bar — the main window is its parent.
pub(crate) fn start_move_window(from: HWND) {
    // SAFETY: parent query on our own child; the main window is the
    // result (the strip/status have no other parent).
    let main =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetParent(from) }.unwrap_or_default();
    if main.is_invalid() {
        return;
    }
    enter_move_loop(main);
}

/// The move loop entry itself — the synthetic caption press at the live
/// cursor (upstream's SendMessage, viv.c:14726-14728).
fn enter_move_loop(hwnd: HWND) {
    let mut pt = POINT::default();
    // SAFETY: read-only cursor query on this thread.
    let _ = unsafe { GetCursorPos(&mut pt) };
    // SAFETY: hwnd is live and owned by this thread; the WM_NCLBUTTONDOWN
    // enters the modal move loop (pumps until button release), exactly
    // upstream's SendMessage.
    let _ = unsafe {
        SendMessageW(
            hwnd,
            WM_NCLBUTTONDOWN,
            Some(WPARAM(HTCAPTION as usize)),
            Some(LPARAM(((pt.y as isize) << 16) | (pt.x as isize & 0xffff))),
        )
    };
}

/// The `config_toolbar_move_window` gate read from the main window behind
/// `child` (upstream's three drag arms all consult it: strip viv.c:11457,
/// status part 0 viv.c:11557, empty menu bar viv.c:2668).
pub(crate) fn toolbar_move_window_enabled(child: HWND) -> bool {
    // SAFETY: parent query on our own child.
    let main =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetParent(child) }.unwrap_or_default();
    if main.is_invalid() {
        return false;
    }
    // SAFETY: the borrow spans only the config read.
    (unsafe { state_of(main) }).is_some_and(|state| state.config.toolbar_move_window != 0)
}

/// The WM_NCLBUTTONDOWN empty-menu-bar drag (upstream viv.c:2665-2691):
/// with `toolbar_move_window` set, a press on the menu bar that lands on
/// NO item starts the move loop instead. Returns whether it was handled
/// (the caller then skips DefWindowProc).
fn on_nclbuttondown_menu_drag(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    if wparam.0 != HTMENU as usize {
        return false;
    }
    // SAFETY: read-only menu query on the live window.
    let menu = unsafe { GetMenu(hwnd) };
    if menu.is_invalid() {
        return false;
    }
    // SAFETY: the borrow spans only the config read.
    let enabled =
        (unsafe { state_of(hwnd) }).is_some_and(|state| state.config.toolbar_move_window != 0);
    if !enabled {
        return false;
    }
    // The press point is in SCREEN coordinates (WM_NCLBUTTONDOWN
    // contract) — what MenuItemFromPoint takes (viv.c:2677-2683).
    let pt = POINT {
        x: (lparam.0 & 0xffff) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    };
    // SAFETY: read-only hit test against our own menu.
    let hit = unsafe { MenuItemFromPoint(Some(hwnd), menu, pt) };
    if hit == -1 {
        enter_move_loop(hwnd);
        true
    } else {
        false
    }
}

/// WM_KEYDOWN / WM_SYSKEYDOWN (upstream viv.c:6346-6406): the ESC arm
/// first (cancel drag / leave fullscreen), then the binding-table route —
/// exact modifier+VK match, first command in table order wins — into the
/// same `on_command` dispatch the menu uses. The bindings live in the
/// config's [`crate::keys::KeyMap`] (ini `*_keys` overlays applied at
/// load; upstream walks `_viv_key_list`, viv.c:6390-6405).
fn on_keydown(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
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
    // and leaving fullscreen that way ALSO pauses a running slideshow, the
    // "stop presenting" gesture, viv.c:6376-6382). This arm is NOT a
    // binding: it stands even when the ini binds ESC somewhere.
    if vk == VK_ESCAPE.0 && !ctrl && !shift && !alt {
        // SAFETY: the borrow spans the two Option takes.
        let canceled = (unsafe { state_of(hwnd) })
            .map(|state| (state.drag.take().is_some(), state.mscroll.take().is_some()));
        if let Some((was_dragging, was_mscrolling)) = canceled
            && (was_dragging || was_mscrolling)
        {
            // SAFETY: the capture was taken on this thread by a
            // button-down arm; the raw cursor re-show balances the
            // middle-drag's raw hide (upstream `_viv_doing_cancel`,
            // viv.c:7850-7871).
            let _ = unsafe { ReleaseCapture() };
            if was_mscrolling {
                // SAFETY: as above.
                unsafe {
                    let _ = ShowCursor(true);
                }
            }
            return;
        }
        // SAFETY: the read-only borrow ends inside is_some_and.
        let fullscreen = (unsafe { state_of(hwnd) }).is_some_and(|state| state.fullscreen);
        if fullscreen {
            toggle_fullscreen(hwnd);
            // ...and pause the slideshow (viv.c:6376-6382).
            // SAFETY: the read-only borrow ends inside is_some_and.
            if (unsafe { state_of(hwnd) }).is_some_and(|state| state.slideshow) {
                slideshow_toggle(hwnd);
            }
        }
        return;
    }
    // The binding route (upstream hands the repeat bit to
    // `_viv_command_with_is_key_repeat`, whose only consumers are the
    // navigation commands, viv.c:1707-1715 — riviv applies the same gate
    // at the route: an auto-repeated next/prev waits for the in-flight
    // load instead of stacking opens).
    // SAFETY: the read-only borrow ends inside and_then.
    let cmd = (unsafe { state_of(hwnd) }).and_then(|s| s.config.keys.lookup(ctrl, alt, shift, vk));
    let Some(cmd) = cmd else { return };
    if matches!(cmd, menu::Cmd::NavNext | menu::Cmd::NavPrev) && (lparam.0 & 0x4000_0000) != 0 {
        // SAFETY: the read-only borrow ends inside is_some_and.
        let waits = (unsafe { state_of(hwnd) }).is_some_and(|s| nav_repeat_waits_for_load(s));
        if waits {
            return;
        }
    }
    on_command(hwnd, cmd);
}

/// WM_DROPFILES (upstream viv.c:3076-3128): the drop body plus the
/// DragFinish teardown a REAL shell drop owns. #41 splits the body out
/// because the clipboard paste routes a system-owned HDROP through the
/// same logic — freeing that would GlobalFree the clipboard's block.
fn on_drop_files(hwnd: HWND, hdrop: HDROP) {
    apply_drop_files(hwnd, hdrop);
    // SAFETY: hdrop arrived with this message from the shell; DragFinish
    // frees it exactly once (upstream never frees — a real-drop leak
    // riviv fixes; this wrapper is the only place that may).
    unsafe {
        DragFinish(hdrop);
    }
}

/// The drop-application body shared by WM_DROPFILES and the clipboard
/// paste (#41; upstream viv.c:3076-3126): shift = append, plain =
/// replace, the playlist build, the home, the foreground raise. NEVER
/// frees `hdrop` — real drops free through [`on_drop_files`], and the
/// paste's clipboard block belongs to the system.
pub(crate) fn apply_drop_files(hwnd: HWND, hdrop: HDROP) {
    // SAFETY: `hdrop` stays valid for the whole body (freed only by a
    // caller, after this returns), and nothing here pumps messages (the
    // FS scans and metadata reads inside the playlist helpers cannot).
    unsafe {
        // Upstream branches on shift BEFORE anything else: shift means
        // append (`add_current_if_empty`, viv.c:3090-3094 — the current
        // file becomes the first playlist entry with a FRESH id when the
        // list is empty), no shift means replace (`clearall` runs even for
        // a single dropped file, viv.c:3095-3098). ANY drop exits random
        // mode first (viv.c:3082-3087).
        let is_shift = GetKeyState(i32::from(VK_SHIFT.0)) < 0;
        // SAFETY: the borrow spans the playlist mutation and one field
        // store.
        if let Some(state) = state_of(hwnd) {
            state.random_search = None;
            if is_shift {
                playlist_add_current_if_empty(state);
            } else {
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
                home_open(hwnd, false, false);
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
    // Dock the toolbar strip (#45; upstream viv.c:1621-1634): spanning
    // the full width at the bottom of the space the status bar left,
    // with the toolbar itself centered inside. The handles are copied
    // out first — the measure below sends to the toolbar.
    // SAFETY: the borrow spans only the handle copy and the client read.
    let docked = (unsafe { state_of(hwnd) }).and_then(|state| {
        if !state.controls.is_alive() {
            return None;
        }
        let mut client = RECT::default();
        // SAFETY: read-only rect query on the live window.
        let _ = unsafe { GetClientRect(hwnd, &mut client) };
        Some((
            state.controls.rebar,
            state.controls.toolbar,
            client.right - client.left,
            client.bottom - client.top,
        ))
    });
    if let Some((rebar, toolbar_hwnd, wide, high)) = docked {
        let status_h = crate::status::height(bar);
        let controls_h = crate::toolbar::controls_height(crate::toolbar::logical_dpi());
        let toolbar_wide = crate::toolbar::toolbar_wide(toolbar_hwnd);
        let (rebar_rect, tb_rect) =
            crate::toolbar::strip_layout(wide, high - status_h, controls_h, toolbar_wide);
        // SAFETY: both are our live child windows; the pair is fail-soft
        // like upstream's unchecked SetWindowPos calls (viv.c:1631-1632).
        unsafe {
            let _ = SetWindowPos(
                rebar,
                None,
                rebar_rect.left,
                rebar_rect.top,
                rebar_rect.right - rebar_rect.left,
                rebar_rect.bottom - rebar_rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = SetWindowPos(
                toolbar_hwnd,
                None,
                tb_rect.left,
                tb_rect.top,
                tb_rect.right - tb_rect.left,
                tb_rect.bottom - tb_rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
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
    // The strip's button states close the resize (upstream ends
    // `_viv_on_size` with `_viv_toolbar_update_buttons()`, viv.c:1654 —
    // the render-size-dependent 1:1/Best Fit grays track the new
    // viewport).
    refresh_toolbar(hwnd);
}

/// The screensaver system-command value (winuser.h 0xF140 — the windows
/// crate ships SC_MONITORPOWER but not this one; upstream switches on both
/// raw, viv.c:3913-3914).
const SC_SCREENSAVE_CMD: isize = 0xf140;

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
            // Upstream viv.c:4424-4450: the minimum track size wrap the
            // toolbar's width and the status+controls heights through
            // AdjustWindowRectEx. With the strip hidden (or before the
            // state exists — this message can precede WM_NCCREATE) the
            // inputs collapse and the system's own SM_CXMINTRACK floor
            // takes over.
            // SAFETY: lparam points to a MINMAXINFO for the duration of
            // the message.
            let mmi = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            // SAFETY: the borrow spans the three reads; the sends and the
            // style queries below run outside it.
            let (toolbar_wide, status_h, controls_h) =
                (unsafe { state_of(hwnd) }).map_or((0, 0, 0), |state| {
                    (
                        crate::toolbar::toolbar_wide(state.controls.toolbar),
                        crate::status::height(state.status),
                        state.controls.height(),
                    )
                });
            let (w, h) = crate::toolbar::min_track_client(toolbar_wide, status_h, controls_h);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            // SAFETY: read-only style queries on the live window.
            let style = WINDOW_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32);
            // SAFETY: read-only ex-style query on the live window.
            let ex_style = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32);
            // SAFETY: read-only menu query on the live window.
            let has_menu = !unsafe { GetMenu(hwnd) }.is_invalid();
            // SAFETY: in/out rect valid; a failure leaves the raw client
            // mins (the system floor still applies, upstream never checks
            // either).
            let _ = unsafe { AdjustWindowRectEx(&mut rect, style, has_menu, ex_style) };
            mmi.ptMinTrackSize = POINT {
                x: rect.right - rect.left,
                y: rect.bottom - rect.top,
            };
            LRESULT(0)
        }
        // The empty-menu-bar drag (#45; upstream viv.c:2665-2691): with
        // `toolbar_move_window` set, a press on the menu bar that hits no
        // item starts the move loop. Everything else (real items, the
        // caption, edges) falls through to the default procedure.
        WM_NCLBUTTONDOWN => {
            if on_nclbuttondown_menu_drag(hwnd, wparam, lparam) {
                LRESULT(0)
            } else {
                // SAFETY: the parameters are exactly this callback's own;
                // the default procedure owns the caption/system handling.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
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
        // Right-click: the full context menu (#49; upstream viv.c:3376-3545
        // — the Menu recovery row gates on the hidden bar, everything else
        // always shows). Breaks to the default procedure like upstream.
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
        // The middle-button scroll pair (#44; upstream viv.c:3329-3345 /
        // 3667-3670).
        WM_MBUTTONDOWN => {
            on_middle_button_down(hwnd);
            LRESULT(0)
        }
        WM_MBUTTONUP => {
            on_middle_button_up(hwnd);
            LRESULT(0)
        }
        // The mouse back/forward buttons (#44; upstream intercepts DOWN,
        // DBLCLK and the NC variants identically in its pre-dispatch
        // filter, viv.c:6302-6343 — each fires the action once, and the
        // messages still fall through to normal dispatch there, so no
        // swallow here either).
        WM_XBUTTONDOWN | WM_XBUTTONDBLCLK | WM_NCXBUTTONDOWN | WM_NCXBUTTONDBLCLK => {
            on_xbutton(hwnd, wparam, lparam);
            // SAFETY: the parameters are exactly this callback's own; the
            // default procedure owns everything unmatched.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
        WM_PASTE => {
            // The clipboard paste (#41; upstream viv.c:4021-4047) — the
            // EditPaste command forwards here (viv.c:2347-2349).
            clipboard::on_paste(hwnd);
            LRESULT(0)
        }
        // The single-instance handoff receive (#21; upstream viv.c:3688-3719)
        // + the Everything-search replies (#22; upstream viv.c:3724-3904):
        // only the command-line id claims the message (upstream `return 1`
        // for it alone, viv.c:3720); the Everything ids are processed and
        // then fall to the default — upstream's arms break to DefWindowProc
        // (returning FALSE, which Everything does not check, viv.c:3904-3907).
        // A null lparam is a malformed foreign send no legitimate sender
        // makes — upstream null-derefs straight into an access violation
        // here; riviv routes it to the default instead (the PR #27 rule:
        // where C crashes on pathological input, Rust must degrade
        // gracefully, never UB).
        WM_COPYDATA => {
            if lparam.0 != 0
                // SAFETY: lparam points at the sender-owned COPYDATASTRUCT
                // for the duration of the message (the WM_COPYDATA
                // contract), and the null guard keeps hostile sends out of
                // the cast.
                && on_copydata(hwnd, unsafe { &*(lparam.0 as *const COPYDATASTRUCT) })
            {
                LRESULT(1)
            } else if lparam.0 != 0 {
                // SAFETY: same contract + null guard as above.
                let cds = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
                match cds.dwData {
                    everything::COPYDATA_OPEN_EVERYTHING_SEARCH => {
                        on_everything_reply(hwnd, cds, false)
                    }
                    everything::COPYDATA_ADD_EVERYTHING_SEARCH => {
                        on_everything_reply(hwnd, cds, true)
                    }
                    everything::COPYDATA_RANDOM_EVERYTHING_SEARCH => on_random_reply(hwnd, cds),
                    _ => {}
                }
                // SAFETY: hwnd/msg are exactly what this callback received;
                // the default procedure handles everything we do not.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
            } else if wparam.0 == slideshow::SLIDESHOW_TIMER_ID {
                on_slideshow_timer(hwnd);
                LRESULT(0)
            } else if wparam.0 == cursor::HIDE_CURSOR_TIMER_ID {
                on_hide_cursor_timer(hwnd);
                LRESULT(0)
            } else if wparam.0 == status::TEMP_TEXT_TIMER_ID {
                // The 3-second flash expiry (upstream viv.c:3135-3137):
                // clear the text; the refresh inside restores the verdict
                // chain to the main part.
                status_set_temp_text(hwnd, None);
                LRESULT(0)
            } else {
                // SAFETY: hwnd/msg are exactly what this callback received;
                // the default procedure handles everything we do not.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        // The status bar's click notifications (#47; upstream viv.c:
        // 3976-4010 — NM_CLICK on part 1 toggles the frames-remaining
        // counter). Only the status bar's idFrom is consumed; everything
        // else (the rebar family reaches the strip's own proc, not here)
        // rides the default handling.
        WM_NOTIFY => {
            // SAFETY: lParam points at the sender's NMHDR for the duration
            // of the message — a same-process child (the status bar), so
            // the read cannot fault.
            let hdr = lparam.0 as *const NMHDR;
            if !hdr.is_null()
                // SAFETY: the two header fields sit at the block's front.
                && unsafe { ((*hdr).idFrom, (*hdr).code) }
                    == (status::STATUS_BAR_ID as usize, NM_CLICK)
            {
                // SAFETY: an NM_CLICK from the status bar carries NMMOUSE
                // in the same notification block.
                let nm = unsafe { &*(lparam.0 as *const NMMOUSE) };
                on_status_nm_click(hwnd, nm);
            }
            // SAFETY: upstream breaks out of its switch onto the default
            // return; the frame's own handling for unmatched notifications
            // is the default procedure's.
            LRESULT(unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }.0)
        }
        // Upstream viv.c:3907-3938: with prevent_sleep on, a running
        // slideshow or a PLAYING animation swallows the monitor-power /
        // screensaver system commands (return 0, no DefWindowProc);
        // everything else falls through. The raw compare (no 0xFFF0 mask)
        // is upstream's own; the animation arm needs BOTH the running
        // timer and the playing flag (viv.c:3929-3936 — #38 carries the
        // real pause flag).
        WM_SYSCOMMAND
            if wparam.0 == SC_MONITORPOWER as usize || wparam.0 == SC_SCREENSAVE_CMD as usize =>
        {
            // SAFETY: the read-only borrow ends inside is_some_and.
            let block = (unsafe { state_of(hwnd) }).is_some_and(|s| {
                s.config.prevent_sleep != 0
                    && (s.slideshow || (s.animation_timer_running && s.animation_playing))
            });
            if block {
                LRESULT(0)
            } else {
                // SAFETY: hwnd/msg are exactly what this callback received;
                // the default procedure owns the system command.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        // The background decode's kick: the queue holds the replies, this
        // just wakes the UI thread to drain them (upstream _VIV_WM_REPLY).
        REPLY_KICK_MESSAGE => {
            on_load_replies(hwnd);
            LRESULT(0)
        }
        // The random-Everything retry (#22; upstream
        // _VIV_WM_RETRY_RANDOM_EVERYTHING_SEARCH, viv.c:2754-2756): an
        // out-of-range index learned the real total; redraw one. Upstream
        // breaks to the default afterwards — equivalent to returning 0 for
        // a WM_APP message nobody else consumes.
        everything::RETRY_RANDOM_MESSAGE => {
            everything::send_random(hwnd);
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
            // Tear the toolbar set down before the window dies (upstream
            // `_viv_controls_show(0)` ahead of DestroyWindow, viv.c:5500 —
            // the children would go with the parent anyway; the image list
            // is the real resource being freed).
            // SAFETY: the borrow spans only the set take-and-clear.
            if let Some(state) = unsafe { state_of(hwnd) } {
                let mut set = std::mem::take(&mut state.controls);
                crate::toolbar::destroy(&mut set);
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

/// The embedded icon resource (id 1, build.rs) at one of the system's icon
/// metrics — the window class's hIcon/hIconSm (#26; upstream loads its rc
/// icon the same way, viv.c:5346-5350). A load failure degrades to a zero
/// handle (the class default icon) rather than failing startup: the icon
/// is cosmetic. The handle is owned by the module's resource section for
/// the process lifetime (no DestroyIcon — class icons outlive every
/// orderly teardown).
// The resource-ordinal-to-pointer cast inside is the MAKEINTRESOURCEW FFI
// idiom (the "pointer" IS the id) — not a dereference target.
#[allow(clippy::manual_dangling_ptr)]
fn load_icon_resource(
    hinstance: HMODULE,
    cx_metric: SYSTEM_METRICS_INDEX,
    cy_metric: SYSTEM_METRICS_INDEX,
) -> HICON {
    // SAFETY: read-only system-metric queries.
    let (cx, cy) = unsafe { (GetSystemMetrics(cx_metric), GetSystemMetrics(cy_metric)) };
    // SAFETY: hinstance is this process's module handle (valid for the
    // process lifetime); resource id 1 is the icon build.rs embeds; the
    // returned shared handle is used only as the class icon.
    unsafe {
        LoadImageW(
            Some(hinstance.into()),
            PCWSTR(1usize as *const u16),
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR,
        )
        .map(|h| HICON(h.0))
        .unwrap_or_default()
    }
}

pub(crate) fn run() -> Result<(), String> {
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
    let mut config = Config::load();
    // The install-family command line (upstream viv.c:5268-5276, between
    // config load and the mutex): `-install`-style switches perform their
    // work — associations as this user, everything else through the
    // elevated re-execution — and the process exits instead of ever
    // creating a window. The raw GetCommandLineW is read INSIDE (the
    // quoting semantics and the `/isrunas <rest>` re-execution need it;
    // `file_args` cannot see either). At this point nothing else exists to
    // tear down: no worker thread, no mutex, and COM needs no shutdown.
    if crate::assoc::process_install_command_line(&mut config) {
        return Ok(());
    }
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
        controls: crate::toolbar::ControlsSet::default(),
        menu: HMENU::default(),
        status_file_not_found: false,
        status_load_failed: false,
        displayed_file_bytes: None,
        pending_file_bytes: None,
        playlist: Playlist::new(),
        nav_current: None,
        view: View::new(),
        drag: None,
        mscroll: None,
        fullscreen: false,
        fullscreen_was_maxed: false,
        fullscreen_restore_rect: RECT::default(),
        fullscreen_zoom_offset: 0,
        cursor: CursorVisibility::new(),
        in_popup_menu: false,
        tracking_mouse: false,
        is_mouseover: false,
        last_cursor_pt: POINT { x: -1, y: -1 },
        src_pixel: (-1, -1),
        src_rgb: (0, 0, 0),
        status_temp: None,
        prevent_deactivate_show: false,
        last_cl_tick: None,
        last_open_folder: None,
        random_search: None,
        random_tot_results: 0,
        everything_request_flags: 0,
        random_rand_state: 0,
        slideshow: false,
        slideshow_timeup: false,
        animation_looped: false,
        animation_playing: true,
        animation_rate_pos: crate::anim::RATE_ONE,
        prevent_sleep_active: false,
        preload: None,
        last_cache: None,
        last_nav_prev: false,
        displayed_entry: None,
    };

    // SAFETY: returns the module handle of this exe; no side effects.
    let hinstance =
        unsafe { GetModuleHandleW(None) }.map_err(|e| format!("GetModuleHandleW failed: {e}"))?;

    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS | CS_VREDRAW | CS_HREDRAW, // CS_DBLCLKS now, double-click = M2
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        // The embedded icon resource (#26; build.rs pins it as the FIRST
        // icon = id 1): LoadImageW at the system's large/small metric so
        // the shell resolves both sizes (upstream loads its rc icon the
        // same way, viv.c:5346-5350). A load failure degrades to the
        // class-default icon — cosmetic, not fatal.
        hIcon: load_icon_resource(hinstance, SM_CXICON, SM_CYICON),
        hIconSm: load_icon_resource(hinstance, SM_CXSMICON, SM_CYSMICON),
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
    let title = HSTRING::from_wide(&title_wide(None, TitleFormat::FilenameOnly));
    // The loaded bindings (still borrowed from the state box; the menu bar
    // below needs them after the box moves into the window).
    let keys = state.config.keys.clone();
    let state_ptr = Box::into_raw(Box::new(state));

    // The menu bar (upstream `_viv_create_menu` before CreateWindowExW,
    // viv.c:5352): built once from the command table over the LOADED key
    // bindings (the ini's `*_keys` overlays are already in the config;
    // upstream reads `_viv_key_list`, likewise seeded before this point).
    // A creation failure degrades to a menu-less window — every menu call
    // guards on the invalid handle (the status-bar posture).
    let menu_bar = create_menu_bar(&keys);
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

    // Hand the bar to the state FIRST (upstream keeps `_viv_hmenu` in a
    // global set before creation, viv.c:718 — the frame pass below and the
    // View→Menu toggle re-attach this handle; a detach leaves GetMenu
    // empty).
    // SAFETY: the borrow spans only the field store; nothing pumps.
    if let Some(state) = unsafe { state_of(hwnd) } {
        state.menu = menu_bar;
    }

    // Apply a remembered caption/thickframe-off pair right after creation
    // (upstream viv.c:5398-5401: only when either is off — the creation
    // style already matches the both-on default), BEFORE the chrome
    // children exist so the height math runs over zeroes exactly like
    // upstream's call site.
    // SAFETY: read-only config reads; update_frame takes its own borrows.
    if (unsafe { state_of(hwnd) })
        .is_some_and(|state| state.config.show_caption == 0 || state.config.show_thickframe == 0)
    {
        update_frame(hwnd);
    }

    // Create the status bar child now that the parent window exists —
    // per config (#46; upstream `_viv_status_show(config_show_status)` at
    // init, viv.c:5415). Creation failure degrades gracefully like
    // upstream — its `_viv_status_hwnd` stays NULL and
    // `_viv_status_update` no-ops — the viewer must keep working; the
    // handle stays invalid and every status call guards on it.
    // SAFETY: read-only config read, then the creation pass.
    if (unsafe { state_of(hwnd) }).is_some_and(|state| state.config.show_status != 0) {
        // SAFETY: returns this exe's module handle; no side effects.
        if let Ok(hinstance) = unsafe { GetModuleHandleW(None) } {
            let bar = match status::create(hwnd, hinstance.into()) {
                Ok(bar) => bar,
                Err(msg) => {
                    eprintln!("status bar unavailable: {msg}");
                    HWND::default()
                }
            };
            // SAFETY: the borrow spans only the field store; the window is
            // created and owned by this thread, nothing below pumps
            // messages.
            if let Some(state) = unsafe { state_of(hwnd) } {
                state.status = bar;
            }
        }
    }

    // The toolbar strip per config (upstream `_viv_controls_show(
    // config_show_controls)` right after the status bar, viv.c:5416) —
    // inside controls_show the strip docks itself and the button states
    // see their first push. Creation failure degrades silently like the
    // status bar's.
    // SAFETY: read-only config read, then the creation pass.
    if (unsafe { state_of(hwnd) }).is_some_and(|state| state.config.show_controls != 0) {
        controls_show(hwnd, true);
    }

    // The on-top bit for a remembered mode (upstream `_viv_update_ontop`
    // at init, viv.c:5422 — mode 1 pins the window before it shows).
    update_ontop(hwnd);

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
    // first run through never takes add-mode (the tick starts unset) and
    // has no current file, so both parse inputs are false. The RAW line —
    // #48's second pass needs the quoting `args_os` cannot see.
    let cl = crate::assoc::command_line_wide();
    let parsed = cli::parse(&cl, false, false);
    process_parsed_cl(hwnd, &parsed);

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
