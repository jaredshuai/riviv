//! The Jump To dialog (#39): a hand-assembled modal following the #22/#24/
//! #37 dialog pattern (no rc template — README Differences). Search-
//! filters the navigable images by substring and opens the selection.
//!
//! Upstream is `_viv_show_jumpto` + `_viv_jumpto_proc` over rc IDD_JUMPTO
//! (viv.c:13003-13280): a resizable modeless-with-modal-pump dialog (owner
//! disabled, local `GetMessageW` loop until `_viv_jump_ret`), an edit +
//! list + OK/Cancel. The item source is the playlist when one exists,
//! else a FLAT scan of the current image's directory (viv.c:13020-13066 —
//! both filtered by the image extension check); the list is ALWAYS sorted
//! name-ascending (`_viv_nav_compare`, viv.c:13324), regardless of the
//! navigation sort config. A search containing `\`, `/` or `:` matches
//! whole paths, anything else matches filename parts (case-insensitive
//! substring, `StrStrI`); the empty search shows everything. The current
//! image is preselected and scrolled to the viewport's middle (viv.c:
//! 13114-13119 / `_viv_center_listbox_item`, 14793+). The pump forwards
//! the edit's arrow/PgUp/PgDn to the list (selection moves, focus stays
//! — viv.c:13204-13262) and every WM_MOUSEWHEEL to the list (viv.c:13199-
//! 13202). Opening the selection goes through the normal open path with
//! the item's ids (the `_viv_open(&fd, 0)` of `_viv_jumpto_open_sel`,
//! viv.c:12984-13001).
//!
//! One upstream deviation, deliberate: the rc LISTBOX row carries no
//! LBS_NOTIFY, yet the proc handles LBN_DBLCLK — without the style a
//! listbox never SENDS the notification. The double-click-to-open is
//! clearly the intent (the handler exists), so riviv's list carries
//! LBS_NOTIFY explicitly.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Mutex;

use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, DEFAULT_GUI_FONT, GetDC, GetDeviceCaps, GetStockObject, GetSysColorBrush,
    HBRUSH, HGDIOBJ, LOGPIXELSY, ReleaseDC,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, SetFocus, VK_DOWN, VK_NEXT, VK_PRIOR, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BN_CLICKED, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW,
    DefWindowProcW, DestroyWindow, DispatchMessageW, EN_CHANGE, ES_AUTOHSCROLL, GetDialogBaseUnits,
    GetDlgItem, GetMessageW, GetParent, GetWindowRect, GetWindowTextW, HMENU, IsChild,
    IsDialogMessageW, IsWindow, LB_ADDSTRING, LB_ERR, LB_GETCOUNT, LB_GETCURSEL, LB_GETITEMDATA,
    LB_GETITEMHEIGHT, LB_RESETCONTENT, LB_SETCURSEL, LB_SETITEMDATA, LB_SETTOPINDEX, LBN_DBLCLK,
    LBS_NOTIFY, MSG, PostQuitMessage, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER,
    SendMessageW, SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_KEYDOWN, WM_MOUSEWHEEL, WM_SETFONT,
    WM_SETREDRAW, WM_SIZE, WM_SYSKEYDOWN, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS,
    WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::loc;
use crate::playlist::{self, PlaylistEntry};
use crate::window::{OpenOrigin, request_open};

/// The dialog's window class (the options_dlg/everything.rs/custom_rate
/// pattern: a plain window class the local IsDialogMessageW pump drives).
const CLASS: PCWSTR = w!("riviv_jumpto");

/// Control ids (rc resource.h: LIST=1031, EDIT=1044 + the dialog-manager
/// IDOK/IDCANCEL values — IsDialogMessageW turns Enter/Esc into WM_COMMAND
/// on these).
const EDIT_ID: i32 = 1044;
const LIST_ID: i32 = 1031;
const IDOK_BTN: i32 = 1;
const IDCANCEL_BTN: i32 = 2;

/// rc IDD_JUMPTO geometry (voidImageViewer.rc): a 187×229 dlu client,
/// resizable (WS_THICKFRAME).
const DLG_WIDE: i32 = 187;
const DLG_HIGH: i32 = 229;

/// The dialog's item set (upstream `_viv_nav_items[]` — a global the proc
/// frees at WM_DESTROY). Single dialog at a time: the owner is disabled
/// while it runs, so an unguarded Mutex is contention-free in practice.
static NAV_ITEMS: Mutex<Vec<PlaylistEntry>> = Mutex::new(Vec::new());

/// Open the Jump To dialog over `owner` (upstream `_viv_show_jumpto`,
/// viv.c:13173-13280). Runs its own message loop until the dialog closes;
/// the selection's open request (if any) is queued before return.
pub(crate) fn open(owner: HWND) {
    register_class();
    // Collect the item set BEFORE any window exists (upstream does it in
    // WM_INITDIALOG — same thread, no pumping between): the playlist if
    // one exists, else the current image's directory scanned FLAT, else
    // nothing (an empty viewer shows an empty list — upstream scans the
    // cwd only in _viv_home, never here, viv.c:13035-13066).
    // SAFETY: the borrow spans the clones and the FS scan — read_dir and
    // metadata never pump messages.
    let mut items = (unsafe { crate::window::state_of(owner) })
        .map(|state| {
            let current = state.nav_current.as_ref().map(|e| e.path.clone());
            if !state.playlist.is_empty() {
                state.playlist.entries().to_vec()
            } else if let Some(current) = current {
                let mut scanned = Vec::new();
                if let Some(dir) = Path::new(&current).parent()
                    && let Ok(read) = std::fs::read_dir(dir)
                {
                    for entry in read.flatten() {
                        let Ok(metadata) = entry.metadata() else {
                            continue;
                        };
                        if metadata.is_dir() {
                            continue; // _viv_is_valid_filename's gate
                        }
                        let path = entry.path();
                        if playlist::is_valid_path(path.as_os_str()) {
                            scanned.push(PlaylistEntry {
                                path: path.into_os_string(),
                                modified: playlist::modified_ticks(&metadata),
                                created: playlist::created_ticks(&metadata),
                                size: metadata.len(),
                                id: 0,
                            });
                        }
                    }
                }
                scanned
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default();
    // Always name-ascending (viv.c:13085-13086). Rust's sort is stable;
    // upstream's qsort is not — fully-tied items (same name AND id) can
    // permute differently, which no observable behavior reads.
    items.sort_by(playlist::nav_compare);
    // The current image's position: full-path exact match (viv.c:13089-
    // 13109, `string_compare` — case-sensitive whole-string equality).
    // SAFETY: the read-only borrow ends inside the map (the path is
    // cloned out); nothing pumps.
    let current_path = (unsafe { crate::window::state_of(owner) })
        .and_then(|s| s.nav_current.as_ref().map(|e| e.path.clone()));
    let cur_index = current_path.and_then(|current| {
        items
            .iter()
            .position(|e| e.path == current)
            .map(|i| i as i32)
    });
    if let Ok(mut slot) = NAV_ITEMS.lock() {
        *slot = items;
    }

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
    let style = WINDOW_STYLE(
        WS_POPUP.0
            | WS_VISIBLE.0
            | WS_CLIPSIBLINGS.0
            | WS_CAPTION.0
            | WS_SYSMENU.0
            | WS_THICKFRAME.0,
    );
    let ex = WINDOW_EX_STYLE(WS_EX_DLGMODALFRAME.0 | WS_EX_CONTROLPARENT.0);
    // SAFETY: in/out rect valid for the call; a failure leaves the raw
    // client size — cosmetic only.
    let _ = unsafe { AdjustWindowRectEx(&mut outer, style, false, ex) };
    let wide = outer.right - outer.left;
    let high = outer.bottom - outer.top;
    let ox = owner_rect.left + (owner_rect.right - owner_rect.left - wide).max(0) / 2;
    let oy = owner_rect.top + (owner_rect.bottom - owner_rect.top - high).max(0) / 2;

    let caption = HSTRING::from(loc::get(loc::Id::JumpToCaption));
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
        eprintln!("riviv: jumpto dialog CreateWindowExW failed (GLE={gle})");
        if let Ok(mut slot) = NAV_ITEMS.lock() {
            slot.clear();
        }
        return;
    };
    build_controls(dlg, font, &dlu);
    on_size(dlg);
    on_search(dlg);
    // The current item preselected and centered (viv.c:13114-13119); the
    // search already selected row 0 for the no-current case.
    if let Some(cur) = cur_index {
        // SAFETY: our own live dialog; the returned child is checked.
        let list = unsafe { GetDlgItem(Some(dlg), LIST_ID) }.unwrap_or_default();
        // SAFETY: our own live child; a failed select just leaves row 0.
        let _ = unsafe {
            SendMessageW(
                list,
                LB_SETCURSEL,
                Some(WPARAM(cur as usize)),
                Some(LPARAM(0)),
            )
        };
        center_list_item(list, cur);
    }
    // Upstream's WM_INITDIALOG returns TRUE — focus lands on the first tab
    // stop (the edit); a hand-built dialog sets it itself.
    // SAFETY: the dialog is live and owned by this thread.
    unsafe {
        let _ = SetFocus(Some(GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default()));
    }

    // The modal loop (upstream viv.c:13186-13275): disable the owner, pump
    // until the dialog dies, forwarding the wheel and the edit's navigation
    // keys to the list (the `continue` arms skip IsDialogMessage so the
    // EDIT never eats them).
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
            let list = GetDlgItem(Some(dlg), LIST_ID).unwrap_or_default();
            let edit = GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default();
            if for_dialog {
                match msg.message {
                    WM_MOUSEWHEEL => {
                        // SAFETY: forwarding to our own live child.
                        let _ =
                            SendMessageW(list, WM_MOUSEWHEEL, Some(msg.wParam), Some(msg.lParam));
                        continue;
                    }
                    WM_SYSKEYDOWN | WM_KEYDOWN
                        if msg.hwnd == edit
                            && matches!(
                                (msg.wParam.0 & 0xffff) as i32,
                                vk if vk == VK_UP.0 as i32
                                    || vk == VK_DOWN.0 as i32
                                    || vk == VK_NEXT.0 as i32
                                    || vk == VK_PRIOR.0 as i32
                            ) =>
                    {
                        // SAFETY: forwarding to our own live child.
                        let _ = SendMessageW(list, msg.message, Some(msg.wParam), Some(msg.lParam));
                        continue;
                    }
                    _ => {}
                }
                if IsDialogMessageW(dlg, &raw const msg).as_bool() {
                    if !IsWindow(Some(dlg)).as_bool() {
                        break;
                    }
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
            if !IsWindow(Some(dlg)).as_bool() {
                break;
            }
        }
        let _ = EnableWindow(owner, true);
        if IsWindow(Some(dlg)).as_bool() {
            let _ = DestroyWindow(dlg);
        }
    }
    if let Ok(mut slot) = NAV_ITEMS.lock() {
        slot.clear();
    }
}

/// Register the dialog class once per process (the options_dlg pattern: an
/// already-registered class is not a failure, anything else is reported
/// loud per ADR 0001 and the flag stays down so a later open retries).
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
            lpfnWndProc: Some(jumpto_proc),
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

/// Build the controls at the rc positions (the edit/list/button geometry
/// the size pass then measures itself against).
fn build_controls(dlg: HWND, font: HGDIOBJ, dlu: &impl Fn(i32, i32) -> (i32, i32)) {
    // SAFETY: every creation uses the live `dlg` parent with fixed ids;
    // the strings outlive the calls. Failures leave missing controls the
    // handlers guard against (empty list, no-op opens).
    unsafe {
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_CLIENTEDGE.0),
            w!("EDIT"),
            None,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
            dlu(7, 0).0,
            dlu(0, 7).1,
            dlu(173, 0).0,
            dlu(0, 14).1,
            Some(dlg),
            Some(HMENU(EDIT_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        // LBS_NOTIFY is riviv's fix for the rc's missing style (see the
        // module doc) — without it the list never reports LBN_DBLCLK.
        // Upstream's row is WS_VSCROLL + tabstop only (no LBS_SORT: the
        // insertion order IS the sorted order; no LBS_STANDARD: it bundles
        // the sort + a border).
        let list = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_CLIENTEDGE.0),
            w!("LISTBOX"),
            None,
            WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | WS_VSCROLL.0 | LBS_NOTIFY as u32,
            ),
            dlu(7, 0).0,
            dlu(0, 29).1,
            dlu(173, 0).0,
            dlu(0, 170).1,
            Some(dlg),
            Some(HMENU(LIST_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        let push = |text: &str, id: i32, x: i32, default: bool| {
            let (px, py) = dlu(x, 208);
            let (pw, ph) = dlu(50, 14);
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
        push(loc::get(loc::Id::OptionsOk), IDOK_BTN, 74, true);
        push(loc::get(loc::Id::OptionsCancel), IDCANCEL_BTN, 130, false);
        // The dialog font on every control (the rc's DS_FIXEDSYS —
        // DEFAULT_GUI_FONT like the other hand-built dialogs).
        for ctrl in [edit, list] {
            if !ctrl.is_invalid() {
                let _ = SendMessageW(
                    ctrl,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        for id in [IDOK_BTN, IDCANCEL_BTN] {
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
    }
}

/// The proportional layout (upstream `_viv_jumpto_on_size`, viv.c:12873-
/// 12917, also run at dialog init): a 12-logical-pixel edge (DPI-scaled),
/// the edit full-width at the top, a 6-logical-pixel gap, the list filling
/// the middle, OK left of Cancel at the bottom right.
fn on_size(dlg: HWND) {
    // SAFETY: our own live window and children; pure queries plus
    // move/resize calls, no z/activation change.
    unsafe {
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(dlg, &mut client);
        let wide = client.right - client.left;
        let high = client.bottom - client.top;
        let edit = GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default();
        let list = GetDlgItem(Some(dlg), LIST_ID).unwrap_or_default();
        let ok = GetDlgItem(Some(dlg), IDOK_BTN).unwrap_or_default();
        if edit.is_invalid() || list.is_invalid() || ok.is_invalid() {
            return;
        }
        let mut rect = RECT::default();
        let _ = GetWindowRect(edit, &mut rect);
        let edit_high = rect.bottom - rect.top;
        let _ = GetWindowRect(ok, &mut rect);
        let button_wide = rect.right - rect.left;
        let button_high = rect.bottom - rect.top;

        // os_logical_high — the system DPI (riviv is DPI-aware; the
        // screen DC's LOGPIXELSY is the same number upstream stores).
        // SAFETY (outer block): short-lived screen DC query, released
        // before the layout continues.
        let dc = GetDC(None);
        let dpi = GetDeviceCaps(Some(dc), LOGPIXELSY).max(96);
        let _ = ReleaseDC(None, dc);
        let edge = 12 * dpi / 96;
        let gap = 6 * dpi / 96;

        let mut y = edge;
        let _ = SetWindowPos(
            edit,
            None,
            edge,
            y,
            wide - edge - edge,
            edit_high,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        y += edit_high + gap;
        let list_high = (high - button_high - edge - y - edge).max(0);
        let _ = SetWindowPos(
            list,
            None,
            edge,
            y,
            wide - edge - edge,
            list_high,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        y += list_high + edge;
        let _ = SetWindowPos(
            ok,
            None,
            wide - edge - button_wide - edge - button_wide,
            y,
            button_wide,
            button_high,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let cancel = GetDlgItem(Some(dlg), IDCANCEL_BTN).unwrap_or_default();
        if !cancel.is_invalid() {
            let _ = SetWindowPos(
                cancel,
                None,
                wide - edge - button_wide,
                y,
                button_wide,
                button_high,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Refill the list from the edit's text (upstream `_viv_jumpto_on_search`,
/// viv.c:12919-12982): a search containing `\`, `/` or `:` matches WHOLE
/// paths, anything else matches filename parts; the empty search matches
/// everything; the match is a case-insensitive substring (`StrStrI`).
/// Row 0 is selected after the refill.
fn on_search(dlg: HWND) {
    // SAFETY: our own live children; pure message calls.
    unsafe {
        let edit = GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default();
        let list = GetDlgItem(Some(dlg), LIST_ID).unwrap_or_default();
        if edit.is_invalid() || list.is_invalid() {
            return;
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(edit, &mut buf).max(0) as usize;
        let search = String::from_utf16_lossy(&buf[..len]);
        let path_search = is_path_search(&search);

        let _ = SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        let _ = SendMessageW(list, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
        let items: Vec<PlaylistEntry> = NAV_ITEMS
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default();
        for (index, item) in items.iter().enumerate() {
            let name = playlist::filename_part(item.path.as_os_str());
            let name = String::from_utf16_lossy(&name);
            let haystack = if path_search {
                item.path.to_string_lossy().into_owned()
            } else {
                name.clone()
            };
            if search.is_empty() || contains_ci(&haystack, &search) {
                let wide = crate::text::to_wide(&name);
                // SAFETY: the string outlives the append; the returned
                // index feeds the item-data set right after.
                let lb_index = SendMessageW(
                    list,
                    LB_ADDSTRING,
                    Some(WPARAM(0)),
                    Some(LPARAM(wide.as_ptr() as isize)),
                )
                .0;
                let _ = SendMessageW(
                    list,
                    LB_SETITEMDATA,
                    Some(WPARAM(lb_index as usize)),
                    Some(LPARAM(index as isize)),
                );
            }
        }
        let _ = SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));
        let _ = SendMessageW(list, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0)));
    }
}

/// Open the selected row (upstream `_viv_jumpto_open_sel`, viv.c:12984-
/// 13001): the row's item data is the nav-item index; the entry opens
/// through the normal request path with its ids. No selection (LB_ERR) or
/// a stale index opens nothing.
fn open_sel(dlg: HWND) {
    // SAFETY: our own live child; pure message reads.
    let sel = unsafe {
        let list = GetDlgItem(Some(dlg), LIST_ID).unwrap_or_default();
        if list.is_invalid() {
            return;
        }
        SendMessageW(list, LB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32
    };
    if sel == LB_ERR || sel < 0 {
        return;
    }
    // SAFETY: our own live child; pure message read.
    let data = unsafe {
        let list = GetDlgItem(Some(dlg), LIST_ID).unwrap_or_default();
        SendMessageW(
            list,
            LB_GETITEMDATA,
            Some(WPARAM(sel as usize)),
            Some(LPARAM(0)),
        )
        .0
    };
    let entry = NAV_ITEMS.lock().ok().and_then(|slot| {
        let index = data as usize;
        slot.get(index).cloned()
    });
    if let Some(entry) = entry {
        // The dialog's owner is the viewer; the open request runs the
        // normal async load (its internal borrows are pump-free — the
        // established request_open contract).
        // SAFETY: read-only parent query on our own window.
        let owner = unsafe { GetParent(dlg) }.unwrap_or_default();
        if !owner.is_invalid() {
            request_open(owner, &entry.path, OpenOrigin::Nav(&entry));
        }
    }
}

/// Scroll `index` to the middle of the listbox viewport (upstream
/// `_viv_center_listbox_item`, viv.c:14793-14826): top = index −
/// visible/2, clamped into [0, count − visible].
fn center_list_item(list: HWND, index: i32) {
    // SAFETY: our own live child; pure queries and one scroll message.
    unsafe {
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(list, &mut client);
        let count = SendMessageW(list, LB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32;
        // Item height: index 0's measured height (LB_GETITEMHEIGHT).
        let item_high =
            SendMessageW(list, LB_GETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(0))).0 as i32;
        if item_high <= 0 || count <= 0 {
            return;
        }
        let visible = (client.bottom - client.top) / item_high;
        if visible <= 0 {
            return;
        }
        let mut top = index - visible / 2;
        top = top.clamp(0, (count - visible).max(0));
        let _ = SendMessageW(
            list,
            LB_SETTOPINDEX,
            Some(WPARAM(top as usize)),
            Some(LPARAM(0)),
        );
    }
}

/// The dialog frame proc (upstream `_viv_jumpto_proc`, viv.c:13003-13171).
unsafe extern "system" fn jumpto_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: every arm either forwards or runs dialog-local logic over
    // our own children; the item set is plain data.
    unsafe {
        match msg {
            WM_COMMAND => {
                let notification = ((wparam.0 >> 16) & 0xffff) as u32;
                match (wparam.0 & 0xffff) as i32 {
                    EDIT_ID if notification == EN_CHANGE => {
                        on_search(hwnd);
                        LRESULT(0)
                    }
                    LIST_ID if notification == LBN_DBLCLK => {
                        open_sel(hwnd);
                        let _ = DestroyWindow(hwnd);
                        LRESULT(0)
                    }
                    IDOK_BTN if notification == BN_CLICKED => {
                        open_sel(hwnd);
                        let _ = DestroyWindow(hwnd);
                        LRESULT(0)
                    }
                    IDCANCEL_BTN if notification == BN_CLICKED => {
                        let _ = DestroyWindow(hwnd);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_SIZE => {
                on_size(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                // X / Alt+F4 collapse to Cancel (dialog convention;
                // upstream's WM_CLOSE arm, viv.c:13161-13163).
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

// ---- The pure search helpers ----

/// A search containing `\`, `/` or `:` is a whole-path search (upstream
/// viv.c:12930-12947).
fn is_path_search(search: &str) -> bool {
    search.contains('\\') || search.contains('/') || search.contains(':')
}

/// `StrStrI` semantics: a case-insensitive substring match. Unicode
/// simple lowercase folding both sides (locale-fold differences would need
/// a case the shell never exercises here).
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_search_triggers_on_separators_and_colon() {
        assert!(!is_path_search("img"));
        assert!(!is_path_search("img 2.png"));
        assert!(is_path_search("C:\\pics"));
        assert!(is_path_search("dir/file"));
        assert!(is_path_search("C:"));
    }

    #[test]
    fn search_matches_substrings_case_insensitively() {
        assert!(contains_ci("Image.PNG", "image"));
        assert!(contains_ci("photo.jpg", ".JPG"));
        assert!(contains_ci("КИРИЛлица", "кирил"));
        assert!(!contains_ci("photo.jpg", "gif"));
        // The empty search matches everything — the caller short-circuits,
        // but the helper agrees.
        assert!(contains_ci("anything", ""));
    }

    // The item-set sort is name-ascending regardless of config — pinned
    // in playlist.rs (`nav_compare`); the dialog-level contract here is
    // just that it is USED via sort_by (compile-time).
}
