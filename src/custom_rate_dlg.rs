//! The Set Custom Rate dialog (#37): a hand-assembled modal following the
//! #22 search-dialog pattern (no rc template — #24's decision, recorded in
//! README Differences). The pure compose/clamp lives in `slideshow.rs`;
//! this shell only collects `(value, unit)` and reports whether OK ended
//! the dialog.
//!
//! Upstream is `_viv_custom_rate_proc` over rc IDD_CUSTOM_RATE
//! (viv.c:7436-7512): an ES_NUMBER edit, a three-row unit combo
//! (milliseconds/seconds/minutes — the combo index IS the type value),
//! OK/Cancel; the OK arm just reads the controls and ends the dialog
//! (`_viv_set_custom_rate`, viv.c:7453-7483, composes and applies OUTSIDE
//! the dialog). The runtime re-layout widens the label to its localized
//! text and splits the remainder between the edit and the combo
//! (viv.c:7457-4787's `os_set_dialog_item_x_wide` calls) — mirrored below
//! in pixels.

use std::ffi::c_void;
use std::sync::Mutex;

use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, DEFAULT_GUI_FONT, GetDC, GetStockObject, GetSysColorBrush,
    GetTextExtentPoint32W, HBRUSH, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BN_CLICKED, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL,
    CB_SETCURSEL, CBS_DROPDOWNLIST, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, ES_AUTOHSCROLL, ES_NUMBER, GetDialogBaseUnits, GetDlgItem, GetMessageW,
    GetWindowRect, GetWindowTextW, HMENU, IsChild, IsDialogMessageW, IsWindow, MSG,
    PostQuitMessage, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW,
    SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_GETFONT, WM_SETFONT, WNDCLASSEXW,
    WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_RIGHT,
    WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::loc;
use crate::slideshow::CustomRateUnit;
use crate::text::to_wide;

/// The dialog's window class (the options_dlg/everything.rs pattern: a
/// plain window class the local IsDialogMessageW pump can drive).
const CLASS: PCWSTR = w!("riviv_custom_rate");

/// Control ids (rc resource.h:1026-1027 + the dialog-manager IDOK/IDCANCEL
/// values — IsDialogMessageW turns Enter/Esc into WM_COMMAND on these).
const EDIT_ID: i32 = 1026;
const COMBO_ID: i32 = 1027;
const IDOK_BTN: i32 = 1;
const IDCANCEL_BTN: i32 = 2;

/// rc IDD_CUSTOM_RATE geometry (voidImageViewer.rc): a 202×63 dlu client.
const DLG_WIDE: i32 = 202;
const DLG_HIGH: i32 = 63;

/// What OK collected (the two `config_slideshow_custom_rate*` values;
/// upstream viv.c:7489-7491 reads them in the dialog and composes after).
pub(crate) struct CustomRateOutcome {
    pub(crate) value: u32,
    pub(crate) unit: CustomRateUnit,
}

/// The dialog's result slot. The controls die with the dialog inside the
/// OK arm's DestroyWindow, so the proc reads them BEFORE destroying and
/// parks the outcome here — the open() pump-end reads it after. One
/// dialog exists at a time (the owner is disabled while it runs).
static OUTCOME: Mutex<Option<(u32, CustomRateUnit)>> = Mutex::new(None);

/// Open the modal over `owner` seeded with the current custom values.
/// `None` = Cancel/Esc (upstream's DialogBox returning 0); `Some` = OK
/// (the caller composes + applies the rate).
pub(crate) fn open(
    owner: HWND,
    current_value: i32,
    current_type: i32,
) -> Option<CustomRateOutcome> {
    *OUTCOME.lock().ok()? = None;
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

    let caption = HSTRING::from(loc::get(loc::Id::CustomRateCaption));
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
        eprintln!("riviv: custom rate dialog CreateWindowExW failed (GLE={gle})");
        return None;
    };
    build_controls(dlg, font, &dlu, current_value, current_type);
    // Upstream's WM_INITDIALOG returns TRUE — focus lands on the first tab
    // stop (the edit); a hand-built dialog sets it itself.
    // SAFETY: the dialog is live and owned by this thread.
    unsafe {
        let _ = SetFocus(Some(GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default()));
    }

    // The modal loop (everything.rs's shape): disable the owner, pump
    // through IsDialogMessage until the dialog is destroyed (OK-on-collect,
    // Cancel, Esc, X). The unfiltered GetMessageW keeps the disabled
    // owner's timers dispatching like DialogBox (the animation timer keeps
    // firing while the rate dialog is up).
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
    // The pump ended — take whatever the last dialog left in the slot.
    OUTCOME
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .map(|(value, unit)| CustomRateOutcome { value, unit })
}

/// Read the controls' current content (runs inside the proc, before the
/// destroying arm).
/// SAFETY: `dlg` is the live dialog owning the controls.
unsafe fn collect(dlg: HWND) -> (u32, CustomRateUnit) {
    // GetDlgItemInt semantics (upstream viv.c:7490 passes NULL for the
    // success flag, so garbage/empty just reads 0).
    let mut buf = [0u16; 32];
    // SAFETY: the edit is a live child of `dlg`; the buffer bounds the
    // read.
    let len =
        unsafe { GetWindowTextW(GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default(), &mut buf) }
            .max(0) as usize;
    let value = parse_dialog_uint(&String::from_utf16_lossy(&buf[..len]));
    // SAFETY: a message read on the dialog's own combo; CB_ERR (-1) reads
    // as the milliseconds row (upstream cannot hit it — the build selected
    // a row before the dialog could close).
    let sel = unsafe {
        SendMessageW(
            GetDlgItem(Some(dlg), COMBO_ID).unwrap_or_default(),
            CB_GETCURSEL,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
        .0 as i32
    };
    let unit = match sel {
        1 => CustomRateUnit::Seconds,
        2 => CustomRateUnit::Minutes,
        _ => CustomRateUnit::Milliseconds,
    };
    (value, unit)
}

/// GetDlgItemInt's parse: leading digits only, empty or non-numeric → 0,
/// u32 overflow → 0.
fn parse_dialog_uint(text: &str) -> u32 {
    let mut acc: u64 = 0;
    for ch in text
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
    {
        acc = acc * 10 + u64::from(ch as u8 - b'0');
        if acc > u64::from(u32::MAX) {
            return 0;
        }
    }
    acc as u32
}

/// Register the dialog class once per process — the options_dlg pattern:
/// an already-registered class is not a failure, anything else is reported
/// loud (ADR 0001) and the flag stays down so a later open retries.
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
            lpfnWndProc: Some(rate_proc),
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

/// Lay out the controls: the rc rows' y/heights in dlu, then upstream's
/// runtime re-layout of the x axis (viv.c:7457-7487) — the label widens to
/// its localized text (measured in the dialog font), and the remaining
/// width splits evenly between the edit and the unit combo.
fn build_controls(
    dlg: HWND,
    font: HGDIOBJ,
    dlu: &impl Fn(i32, i32) -> (i32, i32),
    current_value: i32,
    current_type: i32,
) {
    // SAFETY: every creation uses the live `dlg` parent with fixed ids;
    // the strings outlive the calls. Failures leave missing controls the
    // OK arm reads as empty/first-row.
    unsafe {
        let label_text = loc::get(loc::Id::CustomRateLabel);
        let label = HSTRING::from(label_text);
        let label_wide = to_wide(label_text);
        let (_, sy) = dlu(0, 14);
        let (_, sh) = dlu(0, 8);
        let static_ctrl = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            &label,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            12,
            sy,
            dlu(45, 0).0,
            sh,
            Some(dlg),
            None, // a STATIC sends no WM_COMMAND; the id is irrelevant
            None,
            None,
        )
        .unwrap_or_default();

        let (_, ey) = dlu(0, 12);
        let (_, eh) = dlu(0, 12);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_CLIENTEDGE.0 | WS_EX_RIGHT.0),
            w!("EDIT"),
            None,
            WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | (ES_AUTOHSCROLL | ES_NUMBER) as u32,
            ),
            dlu(54, 0).0,
            ey,
            dlu(66, 0).0,
            eh,
            Some(dlg),
            Some(HMENU(EDIT_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        // SetDlgItemInt: the current custom value as text (negative ini
        // values print unsigned-ish — upstream's SetDlgItemInt(...,FALSE)
        // casts the UINT the same way).
        let text = HSTRING::from((current_value as u32).to_string());
        let _ = SetWindowTextW(edit, &text);

        let combo = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("COMBOBOX"),
            None,
            WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32,
            ),
            dlu(126, 0).0,
            ey,
            dlu(66, 0).0,
            dlu(0, 30).1,
            Some(dlg),
            Some(HMENU(COMBO_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        // The unit rows in type order (viv.c:7494-7496) — the index IS the
        // config value.
        for id in [
            loc::Id::CustomRateMilliseconds,
            loc::Id::CustomRateSeconds,
            loc::Id::CustomRateMinutes,
        ] {
            let s = HSTRING::from(loc::get(id));
            let _ = SendMessageW(
                combo,
                CB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(s.as_ptr() as isize)),
            );
        }
        let _ = SendMessageW(
            combo,
            CB_SETCURSEL,
            Some(WPARAM(current_type.clamp(0, 2) as usize)),
            Some(LPARAM(0)),
        );

        let push = |text: &str, id: i32, x: i32, default: bool| {
            let (px, py) = dlu(x, 36);
            let (pw, ph) = dlu(48, 14);
            let _ = CreateWindowExW(
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
                px,
                py,
                pw,
                ph,
                Some(dlg),
                Some(HMENU(id as *mut c_void)),
                None,
                None,
            );
        };
        push(loc::get(loc::Id::OptionsOk), IDOK_BTN, 90, true);
        push(loc::get(loc::Id::OptionsCancel), IDCANCEL_BTN, 144, false);

        // The dialog font on every control (the rc's DS_FIXEDSYS —
        // DEFAULT_GUI_FONT like the other hand-built dialogs).
        for id in [EDIT_ID, COMBO_ID, IDOK_BTN, IDCANCEL_BTN] {
            let ctrl = GetDlgItem(Some(dlg), id).unwrap_or_default();
            if !ctrl.is_invalid() {
                let _ = SendMessageW(
                    ctrl,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }

        // Upstream's runtime re-layout (viv.c:7457-7487), in pixels: the
        // label spans [12, 12+sw+6), the edit and the combo each take half
        // of what remains inside the 12px margins, 6px apart.
        let static_wide = measure_text_wide(static_ctrl, &label_wide);
        let mut client = RECT::default();
        // SAFETY: read-only query on the live dialog.
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(dlg, &mut client);
        let usable = client.right - client.left - 12 - 12;
        let label_span = static_wide + 6;
        let half = ((usable - label_span - 6) / 2).max(0);
        // SAFETY: our own live children — move + resize, no z/activation
        // change. Failures leave the rc positions (the dialog stays
        // usable, just unbalanced).
        let _ = SetWindowPos(
            static_ctrl,
            None,
            12,
            sy,
            label_span,
            sh,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            edit,
            None,
            12 + label_span,
            ey,
            half,
            eh,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            combo,
            None,
            12 + label_span + half + 6,
            ey,
            half,
            dlu(0, 30).1,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// `os_get_static_wide` (os.c): the control's own text measured in its
/// font — 0 when anything in the chain is unavailable (the caller keeps
/// the rc width).
/// SAFETY: `ctrl` is our live child window.
unsafe fn measure_text_wide(ctrl: HWND, text: &[u16]) -> i32 {
    if ctrl.is_invalid() || text.is_empty() {
        return 0;
    }
    // SAFETY: read-only DC query on our live child.
    let hdc = unsafe { GetDC(Some(ctrl)) };
    if hdc.is_invalid() {
        return 0;
    }
    // SAFETY: short-lived DC + font select, both undone before return.
    let wide = unsafe {
        let font = SendMessageW(ctrl, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0)));
        let last = if font.0 != 0 {
            SelectObject(hdc, HGDIOBJ(font.0 as *mut c_void))
        } else {
            HGDIOBJ(std::ptr::null_mut())
        };
        let mut size = SIZE::default();
        let ok = GetTextExtentPoint32W(hdc, text, &mut size).as_bool();
        if !last.is_invalid() {
            let _ = SelectObject(hdc, last);
        }
        if ok { size.cx } else { 0 }
    };
    // SAFETY: releasing the DC borrowed above.
    let _ = unsafe { ReleaseDC(Some(ctrl), hdc) };
    wide
}

/// The dialog frame proc (upstream `_viv_custom_rate_proc`,
/// viv.c:7436-7512 — OK reads the controls and ends; Cancel just ends).
unsafe extern "system" fn rate_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: every arm either forwards or runs dialog-local logic; the
    // outcome slot is plain data.
    unsafe {
        match msg {
            WM_COMMAND if (wparam.0 >> 16) & 0xffff == BN_CLICKED as usize => {
                match (wparam.0 & 0xffff) as i32 {
                    IDOK_BTN => {
                        // Park the collected values BEFORE destroying —
                        // the children die with the window (upstream's
                        // EndDialog(hwnd,1) closes after the reads).
                        if let Ok(mut slot) = OUTCOME.lock() {
                            *slot = Some(collect(hwnd));
                        }
                        let _ = DestroyWindow(hwnd);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// GetDlgItemInt's lenient parse (upstream passes NULL for the
    /// success flag, viv.c:7490): leading digits only, empty or garbage →
    /// 0, overflow → 0.
    #[test]
    fn dialog_uint_parses_like_getdlgitemint() {
        assert_eq!(parse_dialog_uint("3"), 3);
        assert_eq!(parse_dialog_uint("012"), 12);
        assert_eq!(parse_dialog_uint(""), 0);
        assert_eq!(parse_dialog_uint("abc"), 0);
        assert_eq!(parse_dialog_uint("12abc34"), 12);
        assert_eq!(parse_dialog_uint(" 7"), 7);
        assert_eq!(parse_dialog_uint("4294967295"), 4_294_967_295);
        assert_eq!(parse_dialog_uint("4294967296"), 0, "overflow reads 0");
        assert_eq!(parse_dialog_uint("99999999999999"), 0);
    }
}
