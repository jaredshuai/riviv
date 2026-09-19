//! The toolbar (#45; upstream viv.c:10963-11088 / 11441-11704): a plain
//! custom "_VIV_REBAR"-style strip docked at the BOTTOM of the client area
//! (just above the status bar — viv.c:1621-1634 positions it at
//! `high - controls_high` AFTER the status subtraction; the toolbar's
//! `CCS_TOP` style only says "share the row", not where the strip goes)
//! hosting a `ToolbarWindow32` with six labeled buttons — Previous /
//! Next / Play / Pause / Best Fit / Actual Size — plus two separators
//! (viv.c:11000-11075). The play/pause pair are check-group buttons whose
//! state mirrors the slideshow running flag; the 1:1 / Best Fit buttons
//! gray out when the view is already there (upstream
//! `_viv_toolbar_update_buttons`, viv.c:11665-11704). Dragging the strip
//! moves the window when `toolbar_move_window` is set (viv.c:11455-11465),
//! like the empty-menu-bar and status-part-0 drags of the same config.
//!
//! Upstream ships its button art as rc icons (IDI_PREV..IDI_1TO1); the
//! c-original archive carries no rc, so riviv draws its own glyphs at the
//! DPI-scaled size at runtime (`16*logical/96`, viv.c:10995) — README
//! Differences. Everything decidable is pure: heights, layout rects, the
//! button state machine, the button table, and the glyph coverage masks.

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, COLOR_3DHIGHLIGHT, COLOR_3DSHADOW,
    COLOR_BTNFACE, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, EndPaint,
    FillRect, HBITMAP, PAINTSTRUCT,
};
use windows::Win32::UI::Controls::{
    CCS_NODIVIDER, CCS_NORESIZE, CCS_TOP, CDDS_PREPAINT, CDRF_NOTIFYITEMDRAW, HIMAGELIST,
    ILC_COLOR24, ILC_MASK, ImageList_Add, ImageList_Create, ImageList_Destroy, NM_CUSTOMDRAW,
    NMHDR, NMTBCUSTOMDRAW, TB_ADDBUTTONS, TB_BUTTONCOUNT, TB_BUTTONSTRUCTSIZE, TB_GETITEMRECT,
    TB_SETBUTTONINFO, TB_SETEXTENDEDSTYLE, TB_SETIMAGELIST, TBBUTTON, TBBUTTONINFOW, TBIF_STATE,
    TBSTATE_CHECKED, TBSTATE_ENABLED, TBSTYLE_BUTTON, TBSTYLE_CHECK, TBSTYLE_EX_DOUBLEBUFFER,
    TBSTYLE_EX_HIDECLIPPEDBUTTONS, TBSTYLE_EX_MIXEDBUTTONS, TBSTYLE_FLAT, TBSTYLE_GROUP,
    TBSTYLE_LIST, TBSTYLE_SEP, TBSTYLE_TOOLTIPS, TBSTYLE_TRANSPARENT, TOOLBARCLASSNAME,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetDlgItem, HMENU,
    IDC_ARROW, LoadCursorW, RegisterClassExW, SendMessageW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_NOTIFY, WM_PAINT, WNDCLASSEXW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::loc;
use crate::menu::Cmd;
use crate::text::to_wide;

/// Child-window id for BOTH the rebar strip (child of the main window)
/// and the toolbar (child of the rebar) — upstream uses `VIV_ID_TOOLBAR`
/// for both (viv.h:197-198, the WM_NOTIFY idFrom match, viv.c:11473).
pub(crate) const TOOLBAR_ID: isize = 101;

/// The rebar strip's window class (upstream registers "_VIV_REBAR",
/// viv.c:10970-10978 — a plain class, NOT the rebar common control; it
/// never hosts bands, it is just a painted strip with a wndproc). riviv
/// names it in its own namespace like the main class.
const REBAR_CLASS: PCWSTR = w!("riviv_rebar");

// ---- pure layer ---------------------------------------------------------

/// The strip's height while shown (upstream `_viv_get_controls_high`,
/// viv.c:11441-11447): a FIXED `32 * logical / 96` — never measured from
/// the window. `dpi` is the system DPI (96 at 100%, 192 at 200%).
pub(crate) fn controls_height(dpi: i32) -> i32 {
    (32 * dpi) / 96
}

/// The icon cell size (upstream's ImageList_Create args, viv.c:10995):
/// `16 * logical / 96` per axis.
pub(crate) fn icon_size(dpi: i32) -> (i32, i32) {
    ((16 * dpi) / 96, (16 * dpi) / 96)
}

/// The four buttons whose state is dynamic (upstream
/// `_viv_toolbar_update_buttons`, viv.c:11665-11704). The play/pause
/// check-group mirrors the slideshow flag each way; 1:1 and Best Fit gray
/// out when the view is already exactly there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ButtonStates {
    /// Play checked iff the slideshow runs (viv.c:11671).
    pub(crate) play_checked: bool,
    /// Pause is the mirrored radio (viv.c:11677).
    pub(crate) pause_checked: bool,
    /// 1:1 enabled unless the render already equals the source size
    /// (viv.c:11686-11690 — fsState 0 = grayed when `rw == image_wide &&
    /// rh == image_high`).
    pub(crate) one_to_one_enabled: bool,
    /// Best Fit enabled unless already at the fit level outside 1:1
    /// (viv.c:11693-11697 — fsState 0 when `zoom_pos == 0 && !1to1`).
    pub(crate) best_fit_enabled: bool,
}

impl ButtonStates {
    pub(crate) fn compute(slideshow: bool, render_eq_image: bool, at_best_fit: bool) -> Self {
        Self {
            play_checked: slideshow,
            pause_checked: !slideshow,
            one_to_one_enabled: !render_eq_image,
            best_fit_enabled: !at_best_fit,
        }
    }
}

/// The strip placement inside a client area (upstream viv.c:1626-1634):
/// the rebar spans the full width at the bottom of the space left after
/// the status subtraction, the toolbar rides centered inside it with a
/// 6px inset. Coordinates are client-relative.
pub(crate) fn strip_layout(
    client_wide: i32,
    high_above_status: i32,
    controls_h: i32,
    toolbar_wide: i32,
) -> (RECT, RECT) {
    let rebar = RECT {
        left: 0,
        top: high_above_status - controls_h,
        right: client_wide,
        bottom: high_above_status,
    };
    let toolbar = RECT {
        left: (client_wide / 2) - (toolbar_wide / 2),
        top: 6,
        right: (client_wide / 2) - (toolbar_wide / 2) + toolbar_wide,
        bottom: controls_h - 6,
    };
    (rebar, toolbar)
}

/// The WM_GETMINMAXINFO minimum-track CLIENT size (upstream viv.c:4424-
/// 4450): the window never tracks narrower than its toolbar (or shorter
/// than status + controls). The caller wraps it through
/// AdjustWindowRectEx for the frame; the system still floors the result
/// at SM_CXMINTRACK when the toolbar is hidden (wide 0).
pub(crate) fn min_track_client(toolbar_wide: i32, status_h: i32, controls_h: i32) -> (i32, i32) {
    (toolbar_wide, status_h + controls_h)
}

/// The six button glyphs, in image-list order (upstream's AddIcon order,
/// viv.c:10997-11002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glyph {
    Prev,
    Play,
    Pause,
    Next,
    BestFit,
    OneToOne,
}

/// One button row of the toolbar (upstream's TBBUTTON block, viv.c:11001-
/// 11075): prev, next, separator, play, pause, separator, best fit, 1:1 —
/// with the bitmap index each button shows (0,3,·,1,2,·,4,5).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ButtonSpec {
    /// The command the button fires; `None` marks a separator row.
    pub(crate) cmd: Option<Cmd>,
    /// The image-list bitmap; `None` on separators (upstream leaves 0).
    pub(crate) bitmap: Option<usize>,
}

pub(crate) const BUTTONS: [ButtonSpec; 8] = [
    ButtonSpec {
        cmd: Some(Cmd::NavPrev),
        bitmap: Some(0),
    },
    ButtonSpec {
        cmd: Some(Cmd::NavNext),
        bitmap: Some(3),
    },
    ButtonSpec {
        cmd: None,
        bitmap: None,
    },
    ButtonSpec {
        cmd: Some(Cmd::SlideshowPlayOnly),
        bitmap: Some(1),
    },
    ButtonSpec {
        cmd: Some(Cmd::SlideshowPauseOnly),
        bitmap: Some(2),
    },
    ButtonSpec {
        cmd: None,
        bitmap: None,
    },
    ButtonSpec {
        cmd: Some(Cmd::ViewBestFit),
        bitmap: Some(4),
    },
    ButtonSpec {
        cmd: Some(Cmd::ViewOneToOne),
        bitmap: Some(5),
    },
];

/// Whether a glyph covers the unit-square point (`fx` right, `fy` down,
/// both in 0..1) — the coverage function the mask builder samples at
/// pixel centers. All shapes live inside a 0.14..0.86 safe margin.
pub(crate) fn glyph_covered(glyph: Glyph, fx: f64, fy: f64) -> bool {
    let bar = |x0: f64, x1: f64, y0: f64, y1: f64| fx >= x0 && fx < x1 && fy >= y0 && fy < y1;
    // A right-pointing triangle spanning xa..xb (full height at xa,
    // apex at xb).
    let tri_right =
        |xa: f64, xb: f64| fx >= xa && fx < xb && (fy - 0.5).abs() < 0.35 * (xb - fx) / (xb - xa);
    // The mirrored left-pointing triangle (apex at xa, full at xb).
    let tri_left =
        |xa: f64, xb: f64| fx >= xa && fx < xb && (fy - 0.5).abs() < 0.35 * (fx - xa) / (xb - xa);
    match glyph {
        Glyph::Prev => bar(0.15, 0.3, 0.15, 0.85) || tri_left(0.36, 0.86),
        Glyph::Play => tri_right(0.18, 0.83),
        Glyph::Pause => bar(0.2, 0.38, 0.15, 0.85) || bar(0.62, 0.8, 0.15, 0.85),
        Glyph::Next => tri_right(0.14, 0.64) || bar(0.7, 0.85, 0.15, 0.85),
        Glyph::BestFit => {
            // Viewfinder corner brackets: margin, arm length, thickness.
            let (m, a, t) = (0.14f64, 0.34, 0.15);
            let h = |x0: f64, y: f64, x1: f64| bar(x0, x1, y - t / 2.0, y + t / 2.0);
            let v = |x: f64, y0: f64, y1: f64| bar(x - t / 2.0, x + t / 2.0, y0, y1);
            (h(m, m, m + a) || v(m, m, m + a))
                || (h(1.0 - m - a, m, 1.0 - m) || v(1.0 - m, m, m + a))
                || (h(m, 1.0 - m, m + a) || v(m, 1.0 - m - a, 1.0 - m))
                || (h(1.0 - m - a, 1.0 - m, 1.0 - m) || v(1.0 - m, 1.0 - m - a, 1.0 - m))
        }
        Glyph::OneToOne => {
            // "1 : 1" — two digit bars with base serifs and a colon dot
            // pair, all inside the safe margin.
            let digit =
                |x0: f64, x1: f64| bar(x0, x1, 0.18, 0.72) || bar(x0 - 0.03, x1 + 0.03, 0.72, 0.8);
            digit(0.2, 0.31)
                || bar(0.445, 0.555, 0.36, 0.46)
                || bar(0.445, 0.555, 0.54, 0.64)
                || digit(0.69, 0.8)
        }
    }
}

/// The `size × size` coverage mask, row-major (pixel-center sampling).
pub(crate) fn glyph_mask(glyph: Glyph, size: i32) -> Vec<bool> {
    let n = size.max(0) as usize;
    let mut mask = vec![false; n * n];
    let s = size as f64;
    for y in 0..n {
        for x in 0..n {
            let fx = (x as f64 + 0.5) / s;
            let fy = (y as f64 + 0.5) / s;
            mask[y * n + x] = glyph_covered(glyph, fx, fy);
        }
    }
    mask
}

// ---- Win32 shell --------------------------------------------------------

/// The system DPI, read once like upstream's `os_logical_wide` (os.c:817-
/// 818, captured during init and never re-read). Chrome keeps system-DPI
/// proportions on EVERY monitor by design (#79's LOGPIXELS audit: the
/// strip-height formula and the image-list cell must share one DPI source
/// or the buttons desync from the strip — PerMonitorV2 changes the IMAGE
/// viewport's per-monitor exactness, not chrome's proportions). The
/// pre-#79 screen-DC read returned this same number; the 96 floor keeps
/// the old failed-DC default. (This once-only freeze next to status.rs's
/// per-refresh read is the same split master had — and if the system DPI
/// ever changes mid-run, the strip freezing while the part math re-reads
/// is the smaller evil than the buttons desyncing from their cells.)
pub(crate) fn logical_dpi() -> i32 {
    static DPI: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *DPI.get_or_init(|| {
        // SAFETY: resolved in the CALLING THREAD's DPI context — GetDpiForSystem
        // is only "process-wide" for aware threads (an unaware thread would
        // read 96). riviv never switches a thread's context, so the PMv2
        // default (self-checked at startup in window::run) makes this the
        // real system DPI; the 96 floor only guards a failed (0) return.
        unsafe { GetDpiForSystem() as i32 }.max(96)
    })
}

/// The owned window set: both child windows plus the shared image list
/// (upstream `_viv_rebar_hwnd`/`_viv_toolbar_hwnd`/
/// `_viv_toolbar_image_list`, viv.c:662-664 — created and destroyed only
/// together, in `_viv_controls_show`, viv.c:10963-11088).
#[derive(Default)]
pub(crate) struct ControlsSet {
    pub(crate) rebar: HWND,
    pub(crate) toolbar: HWND,
    images: HIMAGELIST,
}

impl ControlsSet {
    /// Both windows alive (the image list rides along — it exists only on
    /// the same lifetime).
    pub(crate) fn is_alive(&self) -> bool {
        !self.rebar.is_invalid() && !self.toolbar.is_invalid()
    }

    /// The strip height to subtract from the viewport: the fixed formula
    /// while alive, 0 otherwise (upstream `_viv_get_controls_high`'s null
    /// branch, viv.c:11448-11450).
    pub(crate) fn height(&self) -> i32 {
        if self.is_alive() {
            controls_height(logical_dpi())
        } else {
            0
        }
    }
}

/// Create the strip + toolbar + image list as children of `parent`
/// (upstream `_viv_controls_show(1)`, viv.c:10963-11075). The caller
/// re-lays-out via its on-size and refreshes the button states; creation
/// failure is a user-level degrade (no toolbar, viewer keeps working)
/// surfaced as Err.
pub(crate) fn create(parent: HWND, hinstance: HINSTANCE) -> Result<ControlsSet, String> {
    register_rebar_class(hinstance).map_err(|e| format!("rebar class: {e}"))?;
    // SAFETY: parent/hinstance live; the class was registered above; the
    // styles are upstream's strip styles (viv.c:10982-10989).
    let rebar = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            REBAR_CLASS,
            PCWSTR::null(),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPCHILDREN.0 | WS_CLIPSIBLINGS.0),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(TOOLBAR_ID as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
    }
    .map_err(|e| format!("rebar CreateWindowExW failed: {e}"))?;
    // SAFETY: rebar just created and owned by this thread; the styles are
    // upstream's toolbar styles verbatim (viv.c:10991-10995) — the CCS_*
    // trio suppresses the control's own sizing/divider so the manual
    // SetWindowPos layout owns the geometry.
    let toolbar = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            TOOLBARCLASSNAME,
            PCWSTR::null(),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_CLIPCHILDREN.0
                    | WS_CLIPSIBLINGS.0
                    | TBSTYLE_TRANSPARENT
                    | TBSTYLE_LIST
                    | TBSTYLE_FLAT
                    | TBSTYLE_TOOLTIPS
                    | CCS_NODIVIDER as u32
                    | CCS_NORESIZE as u32
                    | CCS_TOP as u32,
            ),
            0,
            0,
            0,
            0,
            Some(rebar),
            Some(HMENU(TOOLBAR_ID as *mut core::ffi::c_void)),
            Some(hinstance),
            None,
        )
    };
    let toolbar = match toolbar {
        Ok(t) => t,
        Err(e) => {
            // Leave nothing half-created behind.
            // SAFETY: rebar is ours and childless now.
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(rebar) };
            return Err(format!("toolbar CreateWindowExW failed: {e}"));
        }
    };
    // SAFETY: toolbar is ours; the extended-style set is upstream's
    // (viv.c:10996) — MIXEDBUTTONS keeps labeled buttons textual,
    // HIDECLIPPEDBUTTONS drops buttons the strip cannot fit, DOUBLEBUFFER
    // kills flicker.
    unsafe {
        let _ = SendMessageW(
            toolbar,
            TB_SETEXTENDEDSTYLE,
            Some(WPARAM(0)),
            Some(LPARAM(
                (TBSTYLE_EX_MIXEDBUTTONS | TBSTYLE_EX_HIDECLIPPEDBUTTONS | TBSTYLE_EX_DOUBLEBUFFER)
                    as isize,
            )),
        );
        // SAFETY: same window; the size handshake every TB_ADDBUTTONS
        // caller owes the control (viv.c:10997).
        let _ = SendMessageW(
            toolbar,
            TB_BUTTONSTRUCTSIZE,
            Some(WPARAM(std::mem::size_of::<TBBUTTON>())),
            None,
        );
    }
    let images = match build_image_list() {
        Ok(images) => images,
        Err(e) => {
            // SAFETY: both windows are ours; destroying the parent strip
            // takes the toolbar child with it.
            let _ = unsafe { DestroyWindow(rebar) };
            return Err(e);
        }
    };
    // SAFETY: toolbar is ours; the list lives until the strip dies.
    unsafe {
        let _ = SendMessageW(
            toolbar,
            TB_SETIMAGELIST,
            Some(WPARAM(0)),
            Some(LPARAM(images.0)),
        );
    }
    // The button texts come from the live localization table (upstream
    // reads localization_get_string per button, viv.c:11003-11071); the
    // strings only need to outlive the TB_ADDBUTTONS call — the control
    // copies them.
    let texts: Vec<Vec<u16>> = BUTTONS
        .iter()
        .map(|spec| match spec.cmd {
            Some(cmd) => to_wide(loc::get(button_loc(cmd))),
            None => Vec::new(),
        })
        .collect();
    let mut buttons: [TBBUTTON; 8] = std::array::from_fn(|i| {
        let spec = &BUTTONS[i];
        match spec.cmd {
            Some(cmd) => TBBUTTON {
                iBitmap: spec.bitmap.map_or(0, |b| b as i32),
                idCommand: i32::from(cmd.id()),
                fsState: TBSTATE_ENABLED as u8,
                fsStyle: (TBSTYLE_BUTTON | TBSTYLE_CHECK | TBSTYLE_GROUP) as u8,
                ..Default::default()
            },
            None => TBBUTTON {
                fsStyle: TBSTYLE_SEP as u8,
                ..Default::default()
            },
        }
    });
    // The plain (non play/pause) buttons drop the check-group styles —
    // only the slideshow pair is TBSTYLE_CHECK|TBSTYLE_GROUP upstream
    // (viv.c:11009-11048); prev/next/best-fit/1:1 are plain buttons.
    for (i, spec) in BUTTONS.iter().enumerate() {
        if let Some(cmd) = spec.cmd
            && !matches!(cmd, Cmd::SlideshowPlayOnly | Cmd::SlideshowPauseOnly)
        {
            buttons[i].fsStyle = TBSTYLE_BUTTON as u8;
        }
    }
    for (i, text) in texts.iter().enumerate() {
        if BUTTONS[i].cmd.is_some() && !text.is_empty() {
            buttons[i].iString = text.as_ptr() as isize;
        }
    }
    // SAFETY: toolbar is ours; the array outlives the call.
    unsafe {
        let _ = SendMessageW(
            toolbar,
            TB_ADDBUTTONS,
            Some(WPARAM(buttons.len())),
            Some(LPARAM(buttons.as_ptr() as isize)),
        );
    }
    Ok(ControlsSet {
        rebar,
        toolbar,
        images,
    })
}

/// Tear the set down (upstream `_viv_controls_show(0)`, viv.c:11077-
/// 11088): toolbar window first, then the strip, then the image list.
pub(crate) fn destroy(set: &mut ControlsSet) {
    if set.toolbar.is_invalid() && set.rebar.is_invalid() && set.images.is_invalid() {
        return;
    }
    // SAFETY: both windows are ours on this thread; the failure paths of
    // DestroyWindow are simply ignored like upstream's unchecked calls.
    unsafe {
        let _ = DestroyWindow(set.toolbar);
        let _ = DestroyWindow(set.rebar);
        if !set.images.is_invalid() {
            // SAFETY: the handle is ours and was never destroyed before.
            let _ = ImageList_Destroy(Some(set.images));
        }
    }
    *set = ControlsSet::default();
}

/// Push the four dynamic button states (upstream
/// `_viv_toolbar_update_buttons`, viv.c:11665-11704 — four
/// TB_SETBUTTONINFO calls, dwMask TBIF_STATE).
pub(crate) fn apply_states(toolbar: HWND, states: &ButtonStates) {
    if toolbar.is_invalid() {
        return;
    }
    let set_state = |cmd: Cmd, checked: bool, enabled: bool| {
        let mut info = TBBUTTONINFOW {
            cbSize: std::mem::size_of::<TBBUTTONINFOW>() as u32,
            dwMask: TBIF_STATE,
            fsState: if enabled {
                TBSTATE_ENABLED as u8 | if checked { TBSTATE_CHECKED as u8 } else { 0 }
            } else {
                0
            },
            ..Default::default()
        };
        // SAFETY: toolbar is ours; the struct outlives the call.
        let _ = unsafe {
            SendMessageW(
                toolbar,
                TB_SETBUTTONINFO,
                Some(WPARAM(usize::from(cmd.id()))),
                Some(LPARAM(&mut info as *mut TBBUTTONINFOW as isize)),
            )
        };
    };
    set_state(Cmd::SlideshowPlayOnly, states.play_checked, true);
    set_state(Cmd::SlideshowPauseOnly, states.pause_checked, true);
    set_state(Cmd::ViewOneToOne, false, states.one_to_one_enabled);
    set_state(Cmd::ViewBestFit, false, states.best_fit_enabled);
}

/// The toolbar's wanted width: the min..max span over every button rect
/// (upstream `_viv_toolbar_get_wide`, viv.c:11574-11620). 0 without a
/// toolbar or without measurable buttons.
pub(crate) fn toolbar_wide(toolbar: HWND) -> i32 {
    if toolbar.is_invalid() {
        return 0;
    }
    // SAFETY: toolbar is ours; TB_BUTTONCOUNT takes no pointers.
    let count = unsafe { SendMessageW(toolbar, TB_BUTTONCOUNT, None, None) }.0 as i32;
    let mut min_x = 0;
    let mut max_x = 0;
    let mut got = false;
    for index in 0..count {
        let mut rect = RECT::default();
        // SAFETY: toolbar is ours; the RECT outlives the call.
        let ok = unsafe {
            SendMessageW(
                toolbar,
                TB_GETITEMRECT,
                Some(WPARAM(index as usize)),
                Some(LPARAM(&mut rect as *mut RECT as isize)),
            )
        }
        .0 != 0;
        if ok {
            if got {
                min_x = min_x.min(rect.left);
                max_x = max_x.max(rect.right);
            } else {
                min_x = rect.left;
                max_x = rect.right;
                got = true;
            }
        }
    }
    if got { max_x - min_x } else { 0 }
}

/// The rebar strip's wndproc (upstream `_viv_rebar_proc`, viv.c:11451-
/// 11540): strip drag moves the window, toolbar notifications get the
/// custom BTNFACE background prepass, WM_COMMAND forwards to the main
/// window, and the paint draws the 2px etch line + face fill.
unsafe extern "system" fn rebar_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            // Upstream viv.c:11455-11465 — the drag is gated on
            // toolbar_move_window; handled drags end here (return 0).
            // SAFETY: the borrow ends at the statement; the move loop in
            // start_move_window pumps messages.
            let drag = crate::window::toolbar_move_window_enabled(hwnd);
            if drag {
                crate::window::start_move_window(hwnd);
                return LRESULT(0);
            }
            // SAFETY: the parameters are exactly this callback's own.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NOTIFY => {
            // SAFETY: lparam points at an NMHDR for the duration of the
            // message (the common control's contract).
            let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
            if hdr.idFrom == TOOLBAR_ID as _ && hdr.code == NM_CUSTOMDRAW {
                // SAFETY: NMTBCUSTOMDRAW is the NM_CUSTOMDRAW payload of
                // a toolbar we own.
                let draw = unsafe { &*(lparam.0 as *const NMTBCUSTOMDRAW) };
                if draw.nmcd.dwDrawStage == CDDS_PREPAINT {
                    let toolbar = get_toolbar_child(hwnd);
                    if !toolbar.is_invalid() {
                        let mut rect = RECT::default();
                        // SAFETY: read-only query on our child.
                        let _ = unsafe { GetClientRect(toolbar, &mut rect) };
                        // SAFETY: the DC belongs to the in-flight custom
                        // draw; the stock face brush is never deleted.
                        unsafe {
                            FillRect(
                                draw.nmcd.hdc,
                                &rect,
                                windows::Win32::Graphics::Gdi::HBRUSH(
                                    (COLOR_BTNFACE.0 as usize + 1) as *mut core::ffi::c_void,
                                ),
                            );
                        }
                    }
                    // Only the PREPAINT stage answers — everything else
                    // (other stages, other codes, other ids) breaks out to
                    // the default procedure like upstream's nested-switch
                    // falls (viv.c:11475-11517).
                    return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
                }
            }
            // SAFETY: the parameters are exactly this callback's own.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_COMMAND => {
            // Upstream viv.c:11518-11520 — toolbar clicks land on the
            // strip (the toolbar's parent) and are re-sent to the main
            // window unchanged.
            // SAFETY: GetParent returns the strip's live owner; the
            // parameters are exactly this callback's own.
            let parent = unsafe { windows::Win32::UI::WindowsAndMessaging::GetParent(hwnd) }
                .unwrap_or_default();
            if parent.is_invalid() {
                return LRESULT(0);
            }
            // SAFETY: parent is the main window on this thread.
            unsafe { SendMessageW(parent, WM_COMMAND, Some(wparam), Some(lparam)) }
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let mut rect = RECT::default();
            // SAFETY: hwnd is ours; BeginPaint/EndPaint pair on one call.
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            // SAFETY: read-only query on the strip.
            let _ = unsafe { GetClientRect(hwnd, &mut rect) };
            let wide = rect.right - rect.left;
            let high = rect.bottom - rect.top;
            let shadow = windows::Win32::Graphics::Gdi::HBRUSH(
                (COLOR_3DSHADOW.0 as usize + 1) as *mut core::ffi::c_void,
            );
            let highlight = windows::Win32::Graphics::Gdi::HBRUSH(
                (COLOR_3DHIGHLIGHT.0 as usize + 1) as *mut core::ffi::c_void,
            );
            let face = windows::Win32::Graphics::Gdi::HBRUSH(
                (COLOR_BTNFACE.0 as usize + 1) as *mut core::ffi::c_void,
            );
            // The 2px etch line at the top (shadow over highlight) then
            // the face fill (upstream viv.c:11486-11528).
            // SAFETY: the DC is BeginPaint's for this message; all three
            // brushes are stock.
            unsafe {
                let _ = FillRect(
                    hdc,
                    &RECT {
                        left: 0,
                        top: 0,
                        right: wide,
                        bottom: 1,
                    },
                    shadow,
                );
                let _ = FillRect(
                    hdc,
                    &RECT {
                        left: 0,
                        top: 1,
                        right: wide,
                        bottom: 2,
                    },
                    highlight,
                );
                let _ = FillRect(
                    hdc,
                    &RECT {
                        left: 0,
                        top: 2,
                        right: wide,
                        bottom: high,
                    },
                    face,
                );
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => {
            // SAFETY: the parameters are exactly this callback's own.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

/// The strip's toolbar child by id (both share TOOLBAR_ID, upstream's
/// WM_NOTIFY idFrom match, viv.c:11473-11475).
fn get_toolbar_child(rebar: HWND) -> HWND {
    // SAFETY: rebar is ours on this thread; a failed lookup yields an
    // invalid HWND the callers guard on.
    unsafe { GetDlgItem(Some(rebar), TOOLBAR_ID as i32) }.unwrap_or_default()
}

/// Register the strip class once per process (upstream registers on every
/// `_viv_controls_show(1)`; RegisterClassEx just fails with
/// ERROR_CLASS_ALREADY_EXISTS on the repeats, which it ignores — riviv
/// collapses that to a once-lock remembering the outcome).
fn register_rebar_class(hinstance: HINSTANCE) -> Result<(), String> {
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *REGISTERED.get_or_init(|| {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(rebar_proc),
            hInstance: hinstance,
            hCursor: {
                // SAFETY: IDC_ARROW is a shared system cursor.
                unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default()
            },
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                (windows::Win32::Graphics::Gdi::COLOR_WINDOW.0 as usize + 1)
                    as *mut core::ffi::c_void,
            ),
            lpszClassName: REBAR_CLASS,
            ..Default::default()
        };
        // SAFETY: wc outlives the call.
        let atom = unsafe { RegisterClassExW(&wc) };
        atom != 0
    });
    if ok {
        Ok(())
    } else {
        Err("RegisterClassExW failed".to_string())
    }
}

/// Build the image list with the six drawn glyphs (upstream's
/// ImageList_Create + six AddIcon calls, viv.c:10995-11002).
fn build_image_list() -> Result<HIMAGELIST, String> {
    let (cw, ch) = icon_size(logical_dpi());
    // SAFETY: sizes are the fixed 16*DPI/96 cell; the flags are
    // upstream's (ILC_COLOR24|ILC_MASK, viv.c:10995).
    let list = unsafe { ImageList_Create(cw, ch, ILC_COLOR24 | ILC_MASK, 0, 0) };
    if list.is_invalid() {
        return Err("ImageList_Create failed".to_string());
    }
    for glyph in [
        Glyph::Prev,
        Glyph::Play,
        Glyph::Pause,
        Glyph::Next,
        Glyph::BestFit,
        Glyph::OneToOne,
    ] {
        let mask = glyph_mask(glyph, cw);
        match color_mask_bitmaps(&mask, cw, ch) {
            Ok((color, mono)) => {
                // SAFETY: the list and both bitmaps are ours; Add copies
                // the bits.
                let added = unsafe { ImageList_Add(list, color, Some(mono)) };
                // SAFETY: both were created above and are no longer
                // needed.
                unsafe {
                    let _ = DeleteObject(color.into());
                    let _ = DeleteObject(mono.into());
                }
                if added < 0 {
                    // SAFETY: the half-built list is ours.
                    unsafe {
                        let _ = ImageList_Destroy(Some(list));
                    }
                    return Err("ImageList_Add failed".to_string());
                }
            }
            Err(e) => {
                // SAFETY: the half-built list is ours.
                unsafe {
                    let _ = ImageList_Destroy(Some(list));
                }
                return Err(e);
            }
        }
    }
    Ok(list)
}

/// Materialize a coverage mask as (24bpp color, 1bpp AND-mask) bitmaps —
/// black glyph pixels where covered, transparent where the mask bit is 1
/// (the ImageList_Add color+mask pair upstream reaches through
/// ImageList_AddIcon's HICONs).
fn color_mask_bitmaps(mask: &[bool], wide: i32, high: i32) -> Result<(HBITMAP, HBITMAP), String> {
    let w = wide.max(0);
    let h = high.max(0);
    if w == 0 || h == 0 {
        return Err("empty icon cell".to_string());
    }
    // 24bpp rows pad to DWORD boundaries, top-down (negative height) so
    // row 0 is the top — matching the mask's CreateBitmap order.
    let row_bytes = (w as usize * 3).div_ceil(4) * 4;
    let mut color_bits = vec![0xffu8; row_bytes * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if mask.get(y * w as usize + x).copied().unwrap_or(false) {
                let at = y * row_bytes + x * 3;
                // BGR black.
                color_bits[at] = 0;
                color_bits[at + 1] = 0;
                color_bits[at + 2] = 0;
            }
        }
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: the header describes color_bits' layout; the section handle
    // is None (the DIB owns system memory).
    let color = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
        .map_err(|e| format!("CreateDIBSection failed: {e}"))?;
    if bits.is_null() {
        // SAFETY: the section was created; failing to hand back bits
        // means it is unusable.
        unsafe {
            let _ = DeleteObject(color.into());
        };
        return Err("CreateDIBSection gave no bits".to_string());
    }
    // SAFETY: bits points at the DIB's own buffer of exactly
    // row_bytes*h bytes for the lifetime of the section.
    unsafe {
        std::ptr::copy_nonoverlapping(color_bits.as_ptr(), bits.cast::<u8>(), color_bits.len());
    }
    // 1bpp AND mask: MSB-first, 1 = transparent, DWORD-padded rows,
    // top row first (the HBITMAP memory layout — no negative-height
    // trick here, CreateBitmap rows are already top-down).
    let mask_row_bytes = (w as usize).div_ceil(32) * 4;
    let mut mask_bits = vec![0xffu8; mask_row_bytes * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let covered = mask.get(y * w as usize + x).copied().unwrap_or(false);
            if covered {
                let byte = y * mask_row_bytes + x / 8;
                mask_bits[byte] &= !(0x80u8 >> (x % 8));
            }
        }
    }
    // SAFETY: the bits slice matches the bitmap's own layout for w×h×1bpp.
    let mono = unsafe {
        CreateBitmap(
            w,
            h,
            1,
            1,
            Some(mask_bits.as_ptr().cast::<core::ffi::c_void>()),
        )
    };
    if mono.is_invalid() {
        // SAFETY: color was created above and is no longer needed.
        unsafe {
            let _ = DeleteObject(color.into());
        };
        return Err("CreateBitmap failed".to_string());
    }
    Ok((color, mono))
}

/// The localization id of a toolbar button's label (upstream's six
/// LOCALIZATION_ID_TOOLBAR_*_BUTTON rows, localization.h:188-193).
fn button_loc(cmd: Cmd) -> loc::Id {
    match cmd {
        Cmd::NavPrev => loc::Id::ToolbarPreviousImage,
        Cmd::NavNext => loc::Id::ToolbarNextImage,
        Cmd::SlideshowPlayOnly => loc::Id::ToolbarPlaySlideshow,
        Cmd::SlideshowPauseOnly => loc::Id::ToolbarPauseSlideshow,
        Cmd::ViewBestFit => loc::Id::ToolbarBestFit,
        // The 1:1 button's label is "Actual Size" (LOCALIZATION_ID_
        // TOOLBAR_ACTUAL_SIZE_BUTTON).
        Cmd::ViewOneToOne => loc::Id::ToolbarActualSize,
        _ => loc::Id::ToolbarActualSize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_scale_with_the_upstream_formula() {
        // viv.c:11444: 32*logical/96 with C integer division.
        assert_eq!(controls_height(96), 32);
        assert_eq!(controls_height(192), 64);
        assert_eq!(controls_height(144), 48);
        assert_eq!(icon_size(96), (16, 16));
        assert_eq!(icon_size(192), (32, 32));
        assert_eq!(icon_size(144), (24, 24));
    }

    #[test]
    fn button_states_mirror_the_upstream_quartet() {
        // Slideshow running: play checked, pause not (viv.c:11671/11677).
        let s = ButtonStates::compute(true, false, false);
        assert!(s.play_checked && !s.pause_checked);
        // Paused: the mirror.
        let s = ButtonStates::compute(false, false, false);
        assert!(!s.play_checked && s.pause_checked);
        // 1:1 grays when the render already equals the source
        // (viv.c:11686-11690).
        let s = ButtonStates::compute(false, true, false);
        assert!(!s.one_to_one_enabled && s.best_fit_enabled);
        // Best Fit grays at the fit level outside 1:1 (viv.c:11693-11697).
        let s = ButtonStates::compute(false, false, true);
        assert!(s.one_to_one_enabled && !s.best_fit_enabled);
    }

    #[test]
    fn strip_layout_docks_the_bottom_and_centers_the_toolbar() {
        // viv.c:1631-1632: rebar at the bottom of the post-status space,
        // toolbar centered with a 6px inset.
        let (rebar, tb) = strip_layout(1000, 700, 32, 400);
        assert_eq!(
            (rebar.left, rebar.top, rebar.right, rebar.bottom),
            (0, 668, 1000, 700)
        );
        assert_eq!((tb.left, tb.top, tb.right, tb.bottom), (300, 6, 700, 26));
        // Odd widths center with C integer division (wide/2 - tw/2).
        let (_, tb) = strip_layout(1001, 700, 32, 400);
        assert_eq!(tb.left, 300);
    }

    #[test]
    fn min_track_is_the_toolbar_width_over_the_bar_heights() {
        // viv.c:4431-4433.
        assert_eq!(min_track_client(520, 23, 32), (520, 55));
        assert_eq!(min_track_client(0, 23, 0), (0, 23));
    }

    #[test]
    fn button_table_is_the_upstream_shape() {
        // viv.c:11001-11075: prev, next, sep, play, pause, sep, best fit,
        // 1:1 with bitmaps 0,3,·,1,2,·,4,5.
        assert_eq!(BUTTONS.len(), 8);
        let cmds: Vec<Option<Cmd>> = BUTTONS.iter().map(|b| b.cmd).collect();
        assert_eq!(
            cmds,
            vec![
                Some(Cmd::NavPrev),
                Some(Cmd::NavNext),
                None,
                Some(Cmd::SlideshowPlayOnly),
                Some(Cmd::SlideshowPauseOnly),
                None,
                Some(Cmd::ViewBestFit),
                Some(Cmd::ViewOneToOne),
            ]
        );
        assert_eq!(
            BUTTONS.iter().map(|b| b.bitmap).collect::<Vec<_>>(),
            vec![
                Some(0),
                Some(3),
                None,
                Some(1),
                Some(2),
                None,
                Some(4),
                Some(5)
            ]
        );
    }

    #[test]
    fn glyph_masks_have_their_signatures() {
        let at = |g: Glyph, x: f64, y: f64| glyph_covered(g, x, y);
        // Pause: two bars, gap between.
        assert!(at(Glyph::Pause, 0.28, 0.5));
        assert!(at(Glyph::Pause, 0.72, 0.5));
        assert!(!at(Glyph::Pause, 0.5, 0.5));
        // Play: a right-pointing triangle — wide at the left base,
        // tapered to nothing near the apex's corners.
        assert!(at(Glyph::Play, 0.3, 0.5));
        assert!(at(Glyph::Play, 0.55, 0.5));
        assert!(!at(Glyph::Play, 0.75, 0.2));
        // Prev/Next are horizontal mirrors of each other (bar + triangle
        // on opposite sides). Sampled off the shape edges — the half-open
        // unit-square intervals disagree with their mirrors exactly ON an
        // edge, and no pixel center ever lands there.
        for i in 1..33 {
            let x = i as f64 / 33.0;
            assert_eq!(
                at(Glyph::Prev, x, 0.5),
                at(Glyph::Next, 1.0 - x, 0.5),
                "Prev/Next mirror breaks at x={x}"
            );
        }
        // The taper: near its apex (x≈0.4 for Prev) the triangle is a
        // sliver — a mid-band point fits at the wide end (x≈0.6) but not
        // the narrow one; Next mirrors it.
        assert!(at(Glyph::Prev, 0.6, 0.45) && !at(Glyph::Prev, 0.4, 0.45));
        assert!(at(Glyph::Next, 0.4, 0.45) && !at(Glyph::Next, 0.6, 0.45));
        // BestFit: brackets at the corners, empty center.
        assert!(at(Glyph::BestFit, 0.2, 0.14));
        assert!(at(Glyph::BestFit, 0.85, 0.6));
        assert!(at(Glyph::BestFit, 0.14, 0.75));
        assert!(!at(Glyph::BestFit, 0.5, 0.5));
        // OneToOne: three column groups (digit, colon, digit), no colon
        // foot.
        assert!(at(Glyph::OneToOne, 0.25, 0.45));
        assert!(at(Glyph::OneToOne, 0.5, 0.41));
        assert!(at(Glyph::OneToOne, 0.75, 0.45));
        assert!(!at(Glyph::OneToOne, 0.5, 0.75));
        // Every glyph at every shipped size yields a non-empty mask.
        for g in [
            Glyph::Prev,
            Glyph::Play,
            Glyph::Pause,
            Glyph::Next,
            Glyph::BestFit,
            Glyph::OneToOne,
        ] {
            for size in [16i32, 24, 32] {
                let mask = glyph_mask(g, size);
                assert_eq!(mask.len(), (size * size) as usize);
                assert!(mask.iter().any(|&c| c), "{g:?} at {size}px is empty");
            }
        }
    }
}
