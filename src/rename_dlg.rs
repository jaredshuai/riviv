//! The Rename dialog (#43): a hand-assembled modal following the
//! custom_rate_dlg pattern (no rc template — #24's standing decision).
//!
//! Upstream is `_viv_rename_proc` over rc IDD_RENAME (viv.c:7232-7360):
//! an ES_AUTOHSCROLL edit prefilled with the extension-stripped file
//! name, OK/Cancel, and a HIDDEN edit (rc IDC_RENAME_OLD_EDIT) carrying
//! the old full path — riviv parks that path in the `OLD_PATH` slot
//! instead, an invisible implementation swap. The OK arm composes the
//! new path (`filemgmt::rename_target`), runs FO_RENAME with
//! FOF_WANTMAPPINGHANDLE, and on a collision the user answers No to the
//! dialog STAYS OPEN for another try (the 1.0.0.16 changelog's own fix,
//! viv.c:243-244); on success the resolved collision name (when any)
//! updates the current file, the playlist and the title — all before the
//! dialog ends, because the modal pump keeps the slideshow timer firing
//! and upstream re-checks the current file at exactly that point
//! (viv.c:7328-7336).

use std::ffi::OsStr;
use std::ffi::OsString;
use std::ffi::c_void;
use std::os::windows::ffi::OsStringExt;
use std::sync::Mutex;

use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush, HBRUSH, HGDIOBJ,
};
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BN_CLICKED, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW,
    DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, GetDialogBaseUnits,
    GetDlgItem, GetMessageW, GetParent, GetWindowRect, GetWindowTextW, HMENU, IsChild,
    IsDialogMessageW, IsWindow, MSG, PostQuitMessage, RegisterClassExW, SW_SHOW, SendMessageW,
    ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_SETFONT, WNDCLASSEXW, WS_CAPTION, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::filemgmt;
use crate::loc;
use crate::text::to_wide_os;
use crate::window::{refresh_title, state_of};

/// The dialog's window class (the options_dlg/custom_rate_dlg pattern).
const CLASS: PCWSTR = w!("riviv_rename");

/// Control ids — the rc resource.h value (1042) plus the dialog-manager
/// IDOK/IDCANCEL IsDialogMessageW turns Enter/Esc into.
const EDIT_ID: i32 = 1042;
const IDOK_BTN: i32 = 1;
const IDCANCEL_BTN: i32 = 2;

/// rc IDD_RENAME geometry (voidImageViewer.rc:152-162): a 186×51 dlu
/// client — edit (7,7,172×14), OK (74,30,50×14), Cancel (129,30,50×14).
const DLG_WIDE: i32 = 186;
const DLG_HIGH: i32 = 51;

/// The hidden IDC_RENAME_OLD_EDIT row's content: the old full path
/// (NUL-free). One dialog exists at a time (the owner is disabled while
/// it runs).
static OLD_PATH: Mutex<Option<Vec<u16>>> = Mutex::new(None);

/// Upstream GetDlgItemText's STRING_SIZE (config.h) — the typed-name read
/// buffer, NUL included.
const STRING_SIZE: usize = 1024;

/// Open the modal over `owner` for `old_path`. The state follow-up
/// (current file / playlist / title) runs inside the OK arm, exactly
/// where upstream runs it; nothing is returned — `_viv_rename` ignores
/// the DialogBoxParam result too (viv.c:7365-7371).
pub(crate) fn open(owner: HWND, old_path: &OsStr) {
    let old_wide = to_wide_os(old_path);
    // rename_target's compare needs the NUL-free form.
    let old_trimmed: Vec<u16> = old_wide[..old_wide.len() - 1].to_vec();
    if let Ok(mut slot) = OLD_PATH.lock() {
        *slot = Some(old_trimmed);
    }
    register_class();
    // SAFETY: pure stock-object query; the handle lives for the process.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    // SAFETY: pure base-unit query — the dlu conversion denominators.
    let base = unsafe { GetDialogBaseUnits() };
    let dlu = |x: i32, y: i32| -> (i32, i32) {
        (
            x * (base as u16 as i32) / 4,
            y * ((base >> 16) as u16 as i32) / 8,
        )
    };

    // Centered over the owner (upstream os_center_dialog).
    let mut owner_rect = RECT::default();
    // SAFETY: read-only query on the live owner.
    let _ = unsafe { GetWindowRect(owner, &mut owner_rect) };
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: dlu(DLG_WIDE, 0).0,
        bottom: dlu(0, DLG_HIGH).1,
    };
    let style =
        WINDOW_STYLE(WS_POPUP.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | WS_CAPTION.0 | WS_SYSMENU.0);
    let ex = WINDOW_EX_STYLE(WS_EX_DLGMODALFRAME.0);
    // SAFETY: in/out rect valid for the call; a failure leaves the raw
    // client size and a tight caption — cosmetic only.
    let _ = unsafe { AdjustWindowRectEx(&mut outer, style, false, ex) };
    let wide = outer.right - outer.left;
    let high = outer.bottom - outer.top;
    let ox = owner_rect.left + (owner_rect.right - owner_rect.left - wide).max(0) / 2;
    let oy = owner_rect.top + (owner_rect.bottom - owner_rect.top - high).max(0) / 2;

    let caption = HSTRING::from(loc::get(loc::Id::RenameCaption));
    // SAFETY: all creation parameters valid; creation-time messages run
    // before any WM_COMMAND can arrive.
    let dlg = unsafe {
        CreateWindowExW(
            ex,
            CLASS,
            &caption,
            style,
            ox,
            oy,
            wide,
            high,
            Some(owner),
            None,
            None,
            None,
        )
    };
    let Ok(dlg) = dlg else {
        // System-level failure (ADR 0001): report loud, keep the viewer.
        // SAFETY: pure thread-error-slot read immediately after the call.
        let gle = unsafe { GetLastError() }.0;
        eprintln!("riviv: rename dialog CreateWindowExW failed (GLE={gle})");
        if let Ok(mut slot) = OLD_PATH.lock() {
            *slot = None;
        }
        return;
    };
    build_controls(dlg, font, &dlu);
    // Upstream's WM_INITDIALOG returns TRUE — focus lands on the edit.
    // SAFETY: the dialog is live and owned by this thread.
    unsafe {
        let _ = SetFocus(Some(GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default()));
    }

    // The modal loop (custom_rate_dlg's shape): disable the owner, pump
    // through IsDialogMessage until the dialog is destroyed. The
    // unfiltered GetMessageW keeps the disabled owner's timers
    // dispatching like DialogBox — the slideshow can advance WHILE the
    // dialog is up, which is exactly why the OK arm re-checks the
    // current file after the shell call (viv.c:7328's comment).
    // SAFETY: standard pump calls over this thread's windows.
    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(dlg, SW_SHOW);
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0).0;
            if r == 0 {
                // WM_QUIT mid-modal: re-post after we leave so the app's
                // own loop exits too.
                PostQuitMessage(msg.wParam.0 as i32);
                break;
            }
            if r == -1 {
                break;
            }
            let for_dialog = msg.hwnd == dlg || IsChild(dlg, msg.hwnd).as_bool();
            if !(for_dialog && IsDialogMessageW(dlg, &raw const msg).as_bool()) {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
            if !IsWindow(Some(dlg)).as_bool() {
                break;
            }
        }
        let _ = EnableWindow(owner, true);
        if IsWindow(Some(dlg)).as_bool() {
            let _ = DestroyWindow(dlg);
        }
    }
    if let Ok(mut slot) = OLD_PATH.lock() {
        *slot = None;
    }
}

/// Register the dialog class once per process (custom_rate_dlg's shape):
/// an already-registered class is not a failure, anything else is
/// reported loud (ADR 0001).
fn register_class() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: the struct outlives the call; the brush is a shared system
    // brush (never deleted).
    let atom = unsafe {
        RegisterClassExW(&WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(rename_proc),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut c_void),
            lpszClassName: CLASS,
            ..Default::default()
        })
    };
    let ok = if atom == 0 {
        // SAFETY: pure thread-error-slot read immediately after the call.
        let gle = unsafe { GetLastError() }.0;
        if gle != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS.0 {
            eprintln!("riviv: RegisterClassExW({CLASS:?}) failed (GLE={gle})");
            false
        } else {
            true
        }
    } else {
        true
    };
    DONE.store(ok, Ordering::Relaxed);
}

/// Lay out the rc rows: the prefilled edit, then OK/Cancel on the bottom
/// margin. The hidden old-path edit is not built — `OLD_PATH` carries it.
fn build_controls(dlg: HWND, font: HGDIOBJ, dlu: &impl Fn(i32, i32) -> (i32, i32)) {
    // SAFETY: every creation uses the live `dlg` parent with fixed ids;
    // the strings outlive the calls. A failed creation leaves a control
    // the OK arm reads as an empty name (the unchanged path ends the
    // dialog harmlessly).
    unsafe {
        let stem = filemgmt::rename_stem(
            &OLD_PATH
                .lock()
                .ok()
                .and_then(|s| s.clone())
                .unwrap_or_default(),
        );
        let prefill = HSTRING::from_wide(&stem);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_CLIENTEDGE.0),
            w!("EDIT"),
            &prefill,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
            dlu(7, 7).0,
            dlu(0, 7).1,
            dlu(172, 0).0,
            dlu(0, 14).1,
            Some(dlg),
            Some(HMENU(EDIT_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        // Select the whole stem — a fresh rename replaces it in one go
        // (the EDIT template control starts fully selected).
        let _ = SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));

        let push = |text: &str, id: i32, x: i32, default: bool| {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                &HSTRING::from(text),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | WS_VISIBLE.0
                        | WS_TABSTOP.0
                        | if default {
                            BS_DEFPUSHBUTTON as u32
                        } else {
                            BS_PUSHBUTTON as u32
                        },
                ),
                dlu(x, 0).0,
                dlu(0, 30).1,
                dlu(50, 0).0,
                dlu(0, 14).1,
                Some(dlg),
                Some(HMENU(id as *mut c_void)),
                None,
                None,
            )
            .unwrap_or_default();
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        };
        push(loc::get(loc::Id::OptionsOk), IDOK_BTN, 74, true);
        push(loc::get(loc::Id::OptionsCancel), IDCANCEL_BTN, 129, false);
    }
}

/// The OK arm's state follow-up (viv.c:7328-7336): when the current file
/// is still the one being renamed (the slideshow may have moved on mid-
/// dialog), re-point it, the in-flight session's adoption path, the
/// playlist entry and the caption at the resolved name. Runs with NO
/// state borrow held across the shell call (the caller arranged that).
fn apply_rename(owner: HWND, old: &[u16], new_full: &[u16]) {
    let old_os = OsString::from_wide(old);
    let new_os = OsString::from_wide(new_full);
    let applied;
    {
        // SAFETY: the borrow spans plain field writes only — nothing below
        // it pumps messages; the shell call has already returned.
        let Some(state) = (unsafe { state_of(owner) }) else {
            return;
        };
        let is_current = state.nav_current.as_ref().is_some_and(|e| e.path == old_os);
        applied = is_current;
        if is_current {
            if let Some(entry) = state.nav_current.as_mut() {
                entry.path = new_os.clone();
            }
            // The title reads the DISPLAYED path (refresh_title) —
            // upstream's single `current_fd.cFileName` update covers both
            // roles; riviv keeps the two copies in step.
            state.path = Some(new_os.clone());
            // An in-flight load adopts its own session path as the title
            // when its first frame lands — retitle that copy too or the
            // old name would resurface (upstream has no such second copy).
            if let Some(session) = state.session.as_mut()
                && session.path() == old_os.as_os_str()
            {
                session.set_path(new_os.clone());
            }
            // The last-cache's snapshot keeps the OLD path on purpose —
            // upstream renames only `current_fd`, never the cached copy.
            state
                .playlist
                .rename_path(old_os.as_os_str(), new_os.as_os_str());
        }
    }
    if applied {
        refresh_title(owner);
    }
}

/// The dialog proc (upstream `_viv_rename_proc`, viv.c:7232-7360).
unsafe extern "system" fn rename_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: every arm either forwards or runs dialog-local logic; the
    // old-path slot is plain data.
    unsafe {
        match msg {
            WM_COMMAND if (wparam.0 >> 16) & 0xffff == BN_CLICKED as usize => {
                match (wparam.0 & 0xffff) as i32 {
                    IDOK_BTN => {
                        on_ok(hwnd);
                        LRESULT(0)
                    }
                    IDCANCEL_BTN => {
                        let _ = DestroyWindow(hwnd);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_CLOSE => {
                // X / Alt+F4 collapse to Cancel (dialog convention).
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            // Dialog-gray control backgrounds — the dialog manager's
            // default a plain window must supply itself.
            WM_CTLCOLORBTN | WM_CTLCOLORSTATIC => {
                LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// The OK arm (viv.c:7268-7352): compose, skip when unchanged, rename,
/// keep the dialog open on a collision-No, otherwise end it.
/// SAFETY: `hwnd` is the live dialog; everything runs on this thread
/// with no state borrow held across the shell call.
unsafe fn on_ok(hwnd: HWND) {
    // SAFETY: every call inside runs against the live `hwnd` on this
    // thread — the rate_proc wrap pattern (one block, one contract).
    unsafe {
        let Some(old) = OLD_PATH.lock().ok().and_then(|s| s.clone()) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let mut buf = [0u16; STRING_SIZE];
        // The edit is a live child of `hwnd`; the buffer bounds the read
        // (GetWindowTextW's own truncation).
        let edit = GetDlgItem(Some(hwnd), EDIT_ID).unwrap_or_default();
        let len = GetWindowTextW(edit, &mut buf).max(0) as usize;
        let typed = &buf[..len];

        let (mut new_full, unchanged) = filemgmt::rename_target(&old, typed);
        if unchanged {
            // Old and new coincide byte-for-byte — no shell call, the
            // dialog just ends (viv.c:7289's compare gate).
            let _ = DestroyWindow(hwnd);
            return;
        }
        // The double-null list builder wants a NUL-terminated input.
        new_full.push(0);

        // GetParent on the live dialog returns its owner, established at
        // creation.
        let owner = GetParent(hwnd).unwrap_or_default();
        let old_os = OsString::from_wide(&old);
        match filemgmt::shell_rename(owner, &old_os, &new_full) {
            filemgmt::RenameOutcome::Done { resolved_new_path } => {
                // The name the file ACTUALLY landed on: the collision
                // resolution's when the mapping has it, else the composed
                // one (viv.c:7310-7326).
                let final_wide =
                    resolved_new_path.unwrap_or_else(|| new_full[..new_full.len() - 1].to_vec());
                apply_rename(owner, &old, &final_wide);
                let _ = DestroyWindow(hwnd);
            }
            filemgmt::RenameOutcome::Aborted => {
                // The user answered No at the collision dialog — keep the
                // rename dialog open for another try (viv.c:7299-7303).
            }
            filemgmt::RenameOutcome::Failed => {
                // The shell refused (ret != 0) — upstream ends the dialog
                // anyway (`dont_end_dialog` stays 0, viv.c:7350).
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}
