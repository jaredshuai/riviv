//! The Options dialog's Win32 shell (#24): a hand-assembled modal frame
//! (pure-code `CreateWindowExW` — no rc template, no `DialogBox`; the
//! decision is recorded in README Differences) over the pure tables in
//! `options.rs`.
//!
//! Upstream shape (viv.c:8553-8807 + rc IDD_OPTIONS:53-66): a tree view
//! picks one of three embedded pages; OK collects every control into the
//! config, applies the runtime invalidations and saves; Cancel just
//! leaves. riviv keeps that shape with: the owner disabled + a local
//! `IsDialogMessageW` pump for Tab/Enter/Esc (dialog-manager semantics
//! without a dialog manager), `WS_EX_CONTROLPARENT` pages so the tab walk
//! recurses (exactly the rc's `DS_CONTROL`), and owner-drawn color
//! swatches in place of upstream's `BS_BITMAP` swatch
//! (`_viv_update_color_button_bitmap`, viv.c:8905+). `config_options_last_page`
//! updates on tree selection — including on Cancel, like upstream's global
//! write in `_viv_options_treeview_changed` (viv.c:8485); the exit-save
//! then persists it.
//!
//! Cosmetic deviation: no `EnableThemeDialogTexture` gradient on the pages
//! (that API needs real dialog windows) — they sit on plain dialog gray.

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, FillRect, GetStockObject,
    GetSysColorBrush, HBRUSH, HGDIOBJ, InvalidateRect,
};
use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::{
    BST_CHECKED, DRAWITEMSTRUCT, NMTREEVIEWW, TVGN_CARET, TVI_LAST, TVI_ROOT, TVIF_PARAM,
    TVIF_TEXT, TVINSERTSTRUCTW, TVITEMW, TVM_INSERTITEM, TVM_SELECTITEM, TVN_SELCHANGEDW,
    TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON,
    BS_OWNERDRAW, BS_PUSHBUTTON, CB_ADDSTRING, CB_ERR, CB_GETCURSEL, CB_SETCURSEL,
    CBS_DROPDOWNLIST, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GWLP_USERDATA, GetDialogBaseUnits, GetMessageW, GetParent, GetWindowLongPtrW, GetWindowRect,
    HMENU, IsChild, IsDialogMessageW, IsWindow, IsWindowVisible, MSG, PostQuitMessage,
    RegisterClassExW, SW_HIDE, SW_SHOW, SendMessageW, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_NOTIFY, WM_SETFONT, WNDCLASSEXW, WNDPROC,
    WS_BORDER, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_GROUP,
    WS_POPUP, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use crate::options::{self, Field, Kind, OptionsModel, PAGES};
use crate::window::state_of;
use crate::{loc, text};

/// Dialog control ids (each child's `hMenu` integer). IDOK/IDCANCEL are the
/// Win32 standards IsDialogMessage routes Enter/Esc through.
const IDOK_BTN: i32 = 1;
const IDCANCEL_BTN: i32 = 2;
const TREE_ID: i32 = 101;

/// Page p's control i (and the page containers themselves at 100*p+5).
/// Deterministic so the WM_COMMAND dispatch and the smoke script share it.
fn ctrl_id(page: usize, index: usize) -> i32 {
    100 * page as i32 + 10 + index as i32
}

fn page_of_ctrl(id: i32) -> usize {
    ((id - 10) / 100).clamp(0, PAGES.len() as i32 - 1) as usize
}

fn index_of_ctrl(id: i32) -> usize {
    ((id - 10) % 100) as usize
}

const DIALOG_CLASS: PCWSTR = w!("riviv_options");
const PAGE_CLASS: PCWSTR = w!("riviv_options_page");

/// The rc geometry (voidImageViewer.rc IDD_OPTIONS:53-66), in dialog
/// units: client 310x271, tree (6,6) 84x240, page host at (106,26)
/// 186x214 (the placeholder's rect — where upstream parks the page
/// dialogs), buttons bottom-right.
const DLG_WIDE: i32 = 310;
const DLG_HIGH: i32 = 271;
const TREE: (i32, i32, i32, i32) = (6, 6, 84, 240);
const PAGE: (i32, i32, i32, i32) = (106, 26, 186, 214);
const BTN_W: i32 = 50;
const BTN_H: i32 = 14;

/// Per-dialog heap state, GWLP_USERDATA-owned like the main window's.
struct DlgState {
    owner: HWND,
    tree: HWND,
    pages: [HWND; 3],
    /// Control handles per page, aligned with `PAGES[p].controls`.
    ctrls: [Vec<HWND>; 3],
    /// The working model: checkbox/combo reads at OK overwrite the plain
    /// fields; the two colors are edited in place by the pickers.
    model: OptionsModel,
    /// ChooseColor's 16 custom-color slots (the API needs a stable
    /// address for the dialog's lifetime).
    cust_colors: [COLORREF; 16],
}

/// `state_of` for the dialog — the same single-threaded, no-reentrancy
/// contract as `window::state_of` (its safety note).
///
/// # Safety
///
/// The returned borrow is exclusive; callers must not hold it across a
/// message pump (`ChooseColorW`, `GetMessageW`) or take a second one while
/// it lives.
unsafe fn dlg_state_of(hwnd: HWND) -> Option<&'static mut DlgState> {
    // SAFETY: between the store at the end of `open` and WM_DESTROY the
    // slot holds a live Box pointer; callers run on the dialog's thread
    // inside its own message handlers.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut DlgState;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: stored once before the dialog became interactive, freed
    // once at WM_DESTROY.
    Some(unsafe { &mut *ptr })
}

/// Open the modal Options dialog over `owner` (upstream `_viv_options`,
/// viv.c:8802-8806 `DialogBox`). Blocks until OK/Cancel/Esc/close: the
/// owner is disabled, the pump runs here, and a WM_QUIT that arrives
/// mid-modal is re-posted so the outer loop still exits (the DialogBox
/// contract).
pub(crate) fn open(owner: HWND) {
    // SAFETY: a short read of the owner state before any dialog exists;
    // nothing pumps between the borrow and its end.
    let model = unsafe { state_of(owner) }.map(|s| OptionsModel::from_config(&s.config));
    let Some(model) = model else { return };
    // SAFETY: a second short read of the owner state, same contract.
    let last_page = unsafe { state_of(owner) }
        .map(|s| s.config.options_last_page.clamp(0, PAGES.len() as i32 - 1) as usize)
        .unwrap_or(0);

    // SAFETY: pure stock-object query; the handle lives for the process.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    // SAFETY: pure base-unit query (the system font's, the documented dlu
    // conversion denominators 4 and 8).
    let base = unsafe { GetDialogBaseUnits() };
    let dlu = |x: i32, y: i32| -> (i32, i32) {
        (
            x * (base as u16 as i32) / 4,
            y * ((base >> 16) as u16 as i32) / 8,
        )
    };
    register_classes();

    // Center the frame over the owner (upstream `os_center_dialog`).
    let mut owner_rect = RECT::default();
    // SAFETY: read-only query on the live owner.
    let _ = unsafe { GetWindowRect(owner, &mut owner_rect) };
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: dlu(DLG_WIDE, 0).0,
        bottom: dlu(0, DLG_HIGH).1,
    };
    let style = WINDOW_STYLE(WS_POPUP.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0);
    let ex = WINDOW_EX_STYLE(WS_EX_DLGMODALFRAME.0 | WS_EX_CONTROLPARENT.0);
    // SAFETY: in/out rect valid for the call; a failure leaves the raw
    // client size and the caption draws tight — cosmetic only.
    let _ = unsafe { AdjustWindowRectEx(&mut outer, style, false, ex) };
    let wide = outer.right - outer.left;
    let high = outer.bottom - outer.top;
    let ox = owner_rect.left + (owner_rect.right - owner_rect.left - wide).max(0) / 2;
    let oy = owner_rect.top + (owner_rect.bottom - owner_rect.top - high).max(0) / 2;

    let caption = HSTRING::from(loc::get(loc::Id::OptionsCaption));
    // SAFETY: all creation parameters valid; creation-time messages (WM_CREATE
    // family) run with userdata still 0 and the proc guards on null.
    let dlg = unsafe {
        CreateWindowExW(
            ex,
            DIALOG_CLASS,
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
        // System-level failure (ADR 0001): fail loud with the error code;
        // the viewer itself keeps running without its options dialog.
        // SAFETY: pure thread-error-slot read immediately after the call.
        let gle = unsafe { GetLastError() }.0;
        eprintln!("riviv: options dialog CreateWindowExW failed (GLE={gle})");
        return;
    };
    // SAFETY: the box is leaked into GWLP_USERDATA exactly once; freed at
    // WM_DESTROY.
    unsafe {
        SetWindowLongPtrW(
            dlg,
            GWLP_USERDATA,
            Box::into_raw(Box::new(DlgState {
                owner,
                tree: HWND::default(),
                pages: [HWND::default(); 3],
                ctrls: [Vec::new(), Vec::new(), Vec::new()],
                model,
                cust_colors: [COLORREF(0); 16],
            })) as *mut c_void as _,
        )
    };
    build_dialog(dlg, font, dlu, last_page);

    // The modal loop: disable the owner, show the frame, pump through
    // IsDialogMessage until the dialog is destroyed (OK/Cancel/Esc/X).
    // SAFETY: legal on the owning thread.
    // SAFETY: legal on the owning thread; the BOOL return of a disabled
    // owner is informational only.
    let _ = unsafe { EnableWindow(owner, false) };
    // SAFETY: the dialog is live and owned by this thread.
    // SAFETY: the dialog is live and owned by this thread; the previous
    // visibility state is of no interest.
    let _ = unsafe { ShowWindow(dlg, SW_SHOW) };
    let mut msg = MSG::default();
    // SAFETY: standard pump calls. The read is unfiltered (None) so the
    // disabled owner's WM_TIMER still dispatches — DialogBox behaves the
    // same, which is what keeps animations running behind the dialog.
    unsafe {
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
                DispatchMessageW(&msg);
            }
            if !IsWindow(Some(dlg)).as_bool() {
                break;
            }
        }
    }
    // SAFETY: legal on the owning thread; re-enable before cleanup so
    // focus returns even if the destroy path hiccuped.
    let _ = unsafe { EnableWindow(owner, true) };
    // SAFETY: read-only validity query on the dialog we own.
    if unsafe { IsWindow(Some(dlg)) }.as_bool() {
        // The loop broke on WM_QUIT, not on a destroyed dialog.
        // SAFETY: owned by this thread, no borrows live.
        let _ = unsafe { DestroyWindow(dlg) };
    }
}

/// Register the dialog/page classes (an already-registered class fails
/// benignly — the error is ignored like every other registration here).
/// Register the dialog/page classes once per process (upstream
/// registers at init, before any dialog, viv.c:5340+). A failure is
/// system-level: fail LOUD with the error code (ADR 0001) — but the
/// classes genuinely exist after the first dialog, so a second pass is
/// skipped entirely and ERROR_CLASS_ALREADY_EXISTS never surfaces.
fn register_classes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    // WNDPROC is an Option over the fn type both procs already have.
    let classes: [(PCWSTR, WNDPROC); 2] = [
        (DIALOG_CLASS, Some(dialog_proc)),
        (PAGE_CLASS, Some(page_proc)),
    ];
    for (class, proc) in classes {
        // SAFETY: the struct outlives the call; the brush is a shared
        // system brush (never deleted).
        let atom = unsafe {
            RegisterClassExW(&WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: proc,
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut c_void),
                lpszClassName: class,
                ..Default::default()
            })
        };
        if atom == 0 {
            // SAFETY: pure thread-error-slot read immediately after the call.
            let gle = unsafe { GetLastError() }.0;
            eprintln!("riviv: RegisterClassExW({class:?}) failed (GLE={gle})");
        }
    }
}

/// Build tree + pages + buttons into the fresh frame and show the
/// remembered page (upstream WM_INITDIALOG, viv.c:8568-8622 + the pages'
/// own INITDIALOG blocks).
fn build_dialog(dlg: HWND, font: HGDIOBJ, dlu: impl Fn(i32, i32) -> (i32, i32), last_page: usize) {
    // SAFETY: every creation below uses the live `dlg`/page parents and
    // strings that outlive the calls; the tree/page handles are stored
    // before any message can use them.
    unsafe {
        let (px, psize) = (dlu(PAGE.0, PAGE.1), dlu(PAGE.2 - PAGE.0, PAGE.3 - PAGE.1));
        // The page containers first (the tree sits beside them).
        if let Some(state) = dlg_state_of(dlg) {
            for (i, _page) in PAGES.iter().enumerate() {
                let page = CreateWindowExW(
                    WINDOW_EX_STYLE(WS_EX_CONTROLPARENT.0),
                    PAGE_CLASS,
                    None,
                    WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0),
                    px.0,
                    px.1,
                    psize.0,
                    psize.1,
                    Some(dlg),
                    Some(HMENU((100 * i + 5) as *mut c_void)),
                    None,
                    None,
                )
                .unwrap_or_default();
                state.pages[i] = page;
            }
        }
        // The tree view (rc: TVS_SHOWSELALWAYS|TVS_TRACKSELECT|WS_BORDER|
        // WS_TABSTOP — the HAS* lines duplicated from upstream's actual
        // look on modern comctl; TVS_TRACKSELECT is a hover effect riviv
        // drops).
        let (tx, ty) = dlu(TREE.0, TREE.1);
        let (tw, th) = dlu(TREE.2, TREE.3);
        let tree = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SysTreeView32"),
            None,
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_BORDER.0
                    | WS_GROUP.0
                    | WS_TABSTOP.0
                    | TVS_SHOWSELALWAYS
                    | TVS_HASBUTTONS
                    | TVS_HASLINES
                    | TVS_LINESATROOT,
            ),
            tx,
            ty,
            tw,
            th,
            Some(dlg),
            Some(HMENU(TREE_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        if let Some(state) = dlg_state_of(dlg) {
            state.tree = tree;
        }
        let _ = SendMessageW(
            tree,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        // Tree items: page titles with the page index as lParam (upstream
        // viv.c:8582-8599).
        for (i, page) in PAGES.iter().enumerate() {
            let mut title: Vec<u16> = text::to_wide(loc::get(page.title));
            let title_text = PWSTR(title.as_mut_ptr());
            let mut ins = TVINSERTSTRUCTW {
                hParent: TVI_ROOT,
                hInsertAfter: TVI_LAST,
                ..Default::default()
            };
            ins.Anonymous.item = TVITEMW {
                mask: TVIF_TEXT | TVIF_PARAM,
                pszText: title_text,
                lParam: LPARAM(i as isize),
                ..Default::default()
            };
            let item = SendMessageW(
                tree,
                TVM_INSERTITEM,
                Some(WPARAM(0)),
                Some(LPARAM(&ins as *const _ as isize)),
            )
            .0 as isize;
            if i == last_page && item != 0 {
                // Fail-soft like upstream's unchecked call (viv.c:8596):
                // a failed select just leaves the first page showing.
                let _ = SendMessageW(
                    tree,
                    TVM_SELECTITEM,
                    Some(WPARAM(TVGN_CARET as usize)),
                    Some(LPARAM(item)),
                );
            }
        }

        // The controls, page by page (creation order = tab order, like
        // the rc's order).
        if let Some(state) = dlg_state_of(dlg) {
            for (p, page) in PAGES.iter().enumerate() {
                let parent = state.pages[p];
                for (i, ctrl) in page.controls.iter().enumerate() {
                    let id = ctrl_id(p, i);
                    let (x, y) = dlu(ctrl.x, ctrl.y);
                    let (w, h) = dlu(ctrl.w, ctrl.h);
                    let handle = match &ctrl.kind {
                        Kind::Checkbox => {
                            let label = HSTRING::from(loc::get(ctrl.label));
                            let hwnd = CreateWindowExW(
                                WINDOW_EX_STYLE(0),
                                w!("BUTTON"),
                                &label,
                                WINDOW_STYLE(
                                    WS_CHILD.0
                                        | WS_VISIBLE.0
                                        | WS_TABSTOP.0
                                        | BS_AUTOCHECKBOX as u32,
                                ),
                                x,
                                y,
                                w,
                                dlu(0, 10).1,
                                Some(parent),
                                Some(HMENU(id as *mut c_void)),
                                None,
                                None,
                            )
                            .unwrap_or_default();
                            let checked = state.model.get_bool(ctrl.field).unwrap_or(false);
                            let _ = SendMessageW(
                                hwnd,
                                BM_SETCHECK,
                                Some(WPARAM(checked as usize)),
                                Some(LPARAM(0)),
                            );
                            hwnd
                        }
                        Kind::Combo(entries) => {
                            if ctrl.label_w > 0 {
                                let label = HSTRING::from(loc::get(ctrl.label));
                                let (lw, lh) = dlu(ctrl.label_w, 10);
                                let _ = CreateWindowExW(
                                    WINDOW_EX_STYLE(0),
                                    w!("STATIC"),
                                    &label,
                                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                                    x,
                                    y,
                                    lw,
                                    lh,
                                    Some(parent),
                                    None,
                                    None,
                                    None,
                                );
                            }
                            let (cx, cw) = if ctrl.label_w > 0 {
                                (dlu(ctrl.label_w, 0).0 + x, w)
                            } else {
                                (x, w)
                            };
                            let hwnd = CreateWindowExW(
                                WINDOW_EX_STYLE(0),
                                w!("COMBOBOX"),
                                None,
                                WINDOW_STYLE(
                                    WS_CHILD.0
                                        | WS_VISIBLE.0
                                        | WS_TABSTOP.0
                                        | WS_VSCROLL.0
                                        | CBS_DROPDOWNLIST as u32,
                                ),
                                cx,
                                y,
                                cw,
                                h,
                                Some(parent),
                                Some(HMENU(id as *mut c_void)),
                                None,
                                None,
                            )
                            .unwrap_or_default();
                            for e in entries.iter() {
                                let text = HSTRING::from(loc::get(e.label));
                                let _ = SendMessageW(
                                    hwnd,
                                    CB_ADDSTRING,
                                    Some(WPARAM(0)),
                                    Some(LPARAM(text.as_ptr() as isize)),
                                );
                            }
                            let sel = model_combo(&state.model, ctrl.field, entries);
                            if let Some(index) = sel {
                                let _ = SendMessageW(
                                    hwnd,
                                    CB_SETCURSEL,
                                    Some(WPARAM(index)),
                                    Some(LPARAM(0)),
                                );
                            }
                            hwnd
                        }
                        Kind::ColorButton => {
                            let label = HSTRING::from(loc::get(ctrl.label));
                            let (lw, lh) = dlu(ctrl.label_w, 10);
                            let _ = CreateWindowExW(
                                WINDOW_EX_STYLE(0),
                                w!("STATIC"),
                                &label,
                                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                                x,
                                y,
                                lw,
                                lh,
                                Some(parent),
                                None,
                                None,
                                None,
                            );
                            let (bx, _) = dlu(ctrl.label_w, 0);
                            CreateWindowExW(
                                WINDOW_EX_STYLE(0),
                                w!("BUTTON"),
                                None,
                                WINDOW_STYLE(
                                    WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32,
                                ),
                                x + bx,
                                y,
                                w,
                                dlu(0, BTN_H).1,
                                Some(parent),
                                Some(HMENU(id as *mut c_void)),
                                None,
                                None,
                            )
                            .unwrap_or_default()
                        }
                    };
                    let _ = SendMessageW(
                        handle,
                        WM_SETFONT,
                        Some(WPARAM(font.0 as usize)),
                        Some(LPARAM(1)),
                    );
                    state.ctrls[p].push(handle);
                }
            }
            // The auto-size type combo starts enabled only with its
            // checkbox on (upstream viv.c:8386).
            if let Some((combo, check)) = auto_zoom_pair(state) {
                let on = SendMessageW(check, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0
                    == BST_CHECKED.0 as isize;
                let _ = EnableWindow(combo, on);
            }
        }

        // OK / Cancel (rc:198/252,252).
        let ok_text = HSTRING::from(loc::get(loc::Id::OptionsOk));
        let cancel_text = HSTRING::from(loc::get(loc::Id::OptionsCancel));
        let by1 = dlu(0, DLG_HIGH - 19).1;
        let (bx1, bx2) = dlu(DLG_WIDE - 112, DLG_WIDE - 58);
        let bw = dlu(BTN_W, 0).0;
        for (id, label, def) in [
            (IDOK_BTN, ok_text, true),
            (IDCANCEL_BTN, cancel_text, false),
        ] {
            let style = WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | if def { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON } as u32,
            );
            let x = if def { bx1 } else { bx2 };
            let handle = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                &label,
                style,
                x,
                by1,
                bw,
                dlu(0, BTN_H).1,
                Some(dlg),
                Some(HMENU(id as *mut c_void)),
                None,
                None,
            )
            .unwrap_or_default();
            let _ = SendMessageW(
                handle,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }

        // The initial page showing (upstream `_viv_options_treeview_changed`
        // at the end of INITDIALOG, viv.c:8614).
        select_page(dlg, last_page);
    }
}

/// Show page `page`, hide the rest (upstream
/// `_viv_options_treeview_changed`, viv.c:8454-8485).
fn select_page(dlg: HWND, page: usize) {
    // SAFETY: the borrow spans only the ShowWindow calls; nothing pumps.
    unsafe {
        if let Some(state) = dlg_state_of(dlg) {
            for (i, hwnd) in state.pages.iter().enumerate() {
                let _ = ShowWindow(*hwnd, if i == page { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

/// The auto-size pair (checkbox, type combo) resolved through the
/// tables — no hardcoded indices to drift with the View page's rows.
fn auto_zoom_pair(state: &DlgState) -> Option<(HWND, HWND)> {
    let mut check = None;
    let mut combo = None;
    for (handle, ctrl) in state.ctrls[1].iter().zip(PAGES[1].controls.iter()) {
        match ctrl.field {
            Field::AutoZoom => check = Some(*handle),
            Field::AutoZoomType => combo = Some(*handle),
            _ => {}
        }
    }
    Some((combo?, check?))
}

/// The combo init pass: the selection index for a value the table
/// carries, nothing for one it does not (the blank combo).
fn model_combo(
    model: &OptionsModel,
    field: Field,
    entries: &[options::ComboEntry],
) -> Option<usize> {
    options::combo_index(entries, model.get_value(field)?)
}

/// Read every control into a fresh model (the OK pass; upstream viv.c:8641+).
fn read_controls(dlg: HWND) -> Option<OptionsModel> {
    // SAFETY: the borrow spans only the message sends to our own
    // controls; none pump messages.
    unsafe {
        let state = dlg_state_of(dlg)?;
        let mut model = state.model.clone();
        for (p, page) in PAGES.iter().enumerate() {
            for (i, ctrl) in page.controls.iter().enumerate() {
                let handle = state.ctrls[p][i];
                match &ctrl.kind {
                    Kind::Checkbox => {
                        let on =
                            SendMessageW(handle, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0
                                == BST_CHECKED.0 as isize;
                        model.set_bool(ctrl.field, on);
                    }
                    Kind::Combo(entries) => {
                        let sel =
                            SendMessageW(handle, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))).0;
                        if sel as i32 != CB_ERR {
                            // A blank combo (an unrepresented ini value) is
                            // skipped: the model keeps what it loaded.
                            model.set_value(ctrl.field, entries[sel as usize].value);
                        }
                    }
                    Kind::ColorButton => {} // edited in place by the pickers
                }
            }
        }
        Some(model)
    }
}

/// The OK button (upstream IDOK, viv.c:8641-8807): read the controls,
/// commit into the owner's live config, apply the effects, save, leave.
fn on_ok(dlg: HWND) {
    let Some(model) = read_controls(dlg) else {
        // SAFETY: legal on this thread; no borrow can be live (read_controls
        // returned).
        let _ = unsafe { DestroyWindow(dlg) };
        return;
    };
    // SAFETY: the dialog-state borrow ends above (model is owned); the
    // owner borrow spans only the commit/invalidate/save — none pump.
    unsafe {
        if let Some(owner_state) = state_of(owner_of(dlg)) {
            let effects = model.commit(&mut owner_state.config);
            if effects.repaint {
                let _ = InvalidateRect(Some(owner_of(dlg)), None, false);
            }
            // Upstream saves unconditionally at OK (viv.c:8805).
            owner_state.config.save();
        }
        let _ = DestroyWindow(dlg);
    }
}

/// The owner HWND out of the dialog state (a plain copy — no borrow).
///
/// # Safety
///
/// Call only while the state is live (before WM_DESTROY freed it).
unsafe fn owner_of(dlg: HWND) -> HWND {
    // SAFETY: short-lived read of the userdata pointer, no borrow held.
    let ptr = unsafe { GetWindowLongPtrW(dlg, GWLP_USERDATA) } as *mut DlgState;
    if ptr.is_null() {
        return HWND::default();
    }
    // SAFETY: reading one field through the live pointer.
    unsafe { (*ptr).owner }
}

/// The dialog frame proc.
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: every arm either forwards, or touches the userdata box only
    // while it is live, without pumping (ChooseColorW runs AFTER the
    // dlg-state borrow is dropped).
    unsafe {
        match msg {
            WM_COMMAND if (wparam.0 >> 16) & 0xffff == BN_CLICKED as usize => {
                on_clicked(hwnd, (wparam.0 & 0xffff) as i32);
                LRESULT(0)
            }
            WM_NOTIFY => {
                on_notify(hwnd, lparam);
                LRESULT(0)
            }
            WM_DRAWITEM => on_draw_item(lparam),
            WM_CLOSE => {
                // X / Alt+F4 = cancel (dialog convention; upstream's
                // WM_CLOSE on a modal collapses to IDCANCEL).
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DlgState;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
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

/// A control was clicked: the buttons act, the auto-size checkbox
/// re-enables its combo (upstream viv.c:8383-8389), the color buttons open
/// the picker (viv.c:8438-8452).
fn on_clicked(dlg: HWND, id: i32) {
    match id {
        IDOK_BTN => {
            on_ok(dlg);
            return;
        }
        IDCANCEL_BTN => {
            // SAFETY: legal on this thread; no borrow is live; the BOOL
            // return is informational.
            let _ = unsafe { DestroyWindow(dlg) };
            return;
        }
        _ => {}
    }
    let page = page_of_ctrl(id);
    let index = index_of_ctrl(id);
    let Some(ctrl) = PAGES.get(page).and_then(|p| p.controls.get(index)) else {
        return;
    };
    match ctrl.field {
        Field::AutoZoom => {
            // Toggle the type combo's enable to match the checkbox.
            // SAFETY: the borrow spans only two message sends.
            unsafe {
                if let Some(state) = dlg_state_of(dlg)
                    && let Some((combo, check)) = auto_zoom_pair(state)
                {
                    let on = SendMessageW(check, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0
                        == BST_CHECKED.0 as isize;
                    let _ = EnableWindow(combo, on);
                }
            }
        }
        Field::WindowedBg | Field::FullscreenBg => {
            choose_color(dlg, ctrl.field);
        }
        _ => {}
    }
}

/// The color picker (upstream `os_choose_color` = ChooseColorW with the
/// current color preselected; cancel keeps it). The dialog-state borrow is
/// dropped before the modal picker runs (it pumps messages).
fn choose_color(dlg: HWND, field: Field) {
    // SAFETY: the borrow spans only the copies out.
    let picked: Option<(u32, *mut COLORREF)> = unsafe {
        dlg_state_of(dlg).map(|state| {
            let color = state.model.get_color(field).unwrap_or([255, 255, 255]);
            let [r, g, b] = color;
            (
                u32::from(b) << 16 | u32::from(g) << 8 | u32::from(r),
                state.cust_colors.as_mut_ptr(),
            )
        })
    };
    let Some((rgb, cust)) = picked else { return };
    let mut cc = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: dlg,
        rgbResult: COLORREF(rgb),
        lpCustColors: cust,
        Flags: CC_FULLOPEN | CC_RGBINIT,
        ..Default::default()
    };
    // SAFETY: `cc` outlives the modal call; every pointer inside is valid
    // for its duration (the cust_colors array lives in the dialog state,
    // which no handler frees while the picker pumps — WM_DESTROY of the
    // dialog cannot run inside ChooseColorW since the picker disables its
    // owner, our dialog).
    let ok = unsafe { ChooseColorW(&mut cc) };
    if !ok.as_bool() {
        return; // canceled (or failed): keep the current color
    }
    let color = [
        (cc.rgbResult.0 & 0xff) as u8,
        ((cc.rgbResult.0 >> 8) & 0xff) as u8,
        ((cc.rgbResult.0 >> 16) & 0xff) as u8,
    ];
    // SAFETY: a fresh borrow for the store-back.
    unsafe {
        if let Some(state) = dlg_state_of(dlg) {
            state.model.set_color(field, color);
        }
    }
    // Repaint the swatch (the WM_DRAWITEM pass reads the stored color).
    // SAFETY: read-only id lookup; the invalidate targets our own button.
    unsafe {
        if let Some(state) = dlg_state_of(dlg) {
            let page = PAGES
                .iter()
                .position(|p| p.controls.iter().any(|c| c.field == field))
                .unwrap_or(0);
            let index = PAGES[page]
                .controls
                .iter()
                .position(|c| c.field == field)
                .unwrap_or(0);
            if let Some(&button) = state.ctrls[page].get(index) {
                let _ = InvalidateRect(Some(button), None, false);
            }
        }
    }
}

/// Tree selection changed (upstream TVN_SELCHANGED →
/// `_viv_options_treeview_changed`): swap pages and remember the page in
/// the live config — even if OK never comes (the upstream global write;
/// the exit-save persists it).
fn on_notify(dlg: HWND, lparam: LPARAM) {
    // SAFETY: the NMHDR/NMTREEVIEWW point at the tree's own notification
    // buffer, valid for the message's duration.
    let nm = lparam.0 as *const NMTREEVIEWW;
    if nm.is_null() {
        return;
    }
    // SAFETY: as above.
    let hdr = unsafe { &(*nm).hdr };
    if hdr.code != TVN_SELCHANGEDW || hdr.idFrom != TREE_ID as usize {
        return;
    }
    // SAFETY: as above.
    let page = unsafe { (*nm).itemNew.lParam.0 };
    let page = page.clamp(0, PAGES.len() as isize - 1) as usize;
    // SAFETY: read-only visibility query on the live dialog.
    if !unsafe { IsWindowVisible(dlg) }.as_bool() {
        return; // the initial TVM_SELECTITEM before the dialog showed
    }
    select_page(dlg, page);
    // SAFETY: the owner-state borrow spans only the field store; the
    // dialog borrow ended with select_page.
    unsafe {
        if let Some(owner_state) = state_of(owner_of(dlg)) {
            owner_state.config.options_last_page = page as i32;
        }
    }
}

/// Owner-draw the color swatches (upstream's BS_BITMAP solid fill,
/// `_viv_update_color_button_bitmap`'s visible half).
fn on_draw_item(lparam: LPARAM) -> LRESULT {
    // SAFETY: DRAWITEMSTRUCT points at the button's draw buffer, valid
    // for the message's duration.
    let dis = lparam.0 as *const DRAWITEMSTRUCT;
    if dis.is_null() {
        return LRESULT(0);
    }
    // SAFETY: as above.
    let (ctl_id, hdc, rect) = unsafe { ((*dis).CtlID, (*dis).hDC, (*dis).rcItem) };
    let page = page_of_ctrl(ctl_id as i32);
    let index = index_of_ctrl(ctl_id as i32);
    let Some(ctrl) = PAGES.get(page).and_then(|p| p.controls.get(index)) else {
        return LRESULT(0);
    };
    let color = match ctrl.field {
        Field::WindowedBg | Field::FullscreenBg => color_ref_for(
            // SAFETY: the draw item is valid for this message (see above).
            unsafe { draw_item_dialog(dis) },
            ctrl.field,
        ),
        _ => return LRESULT(0),
    };
    let [r, g, b] = color;
    // SAFETY: pure GDI brush creation from plain values.
    let brush = unsafe {
        CreateSolidBrush(COLORREF(
            u32::from(b) << 16 | u32::from(g) << 8 | u32::from(r),
        ))
    };
    if !brush.is_invalid() {
        // SAFETY: the DC is the draw item's own, the rect its own.
        unsafe {
            let _ = FillRect(hdc, &rect, brush);
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
    }
    LRESULT(1)
}

/// The owner-window HWND carried alongside a draw item (its `hwndItem` is
/// the button itself; the model lives on the dialog).
/// Walk a draw item's button up to the owning dialog (its page, then
/// the frame — the model lives on the frame).
unsafe fn draw_item_dialog(dis: *const DRAWITEMSTRUCT) -> HWND {
    // SAFETY: hwndItem is the live button; walk up to the page, then the
    // dialog. (GetParent twice.)
    let button = unsafe { (*dis).hwndItem };
    // SAFETY: read-only parent queries.
    let page = unsafe { GetParent(button) }.unwrap_or_default();
    // SAFETY: read-only parent queries on live windows we own.
    unsafe { GetParent(page) }.unwrap_or_default()
}

/// The current model color for a background field, read through the
/// dialog's state without a borrow (a copy).
fn color_ref_for(dlg: HWND, field: Field) -> [u8; 3] {
    // SAFETY: short-lived pointer read, no borrow held across calls.
    unsafe {
        let ptr = dlg_state_ptr(dlg);
        if ptr.is_null() {
            return [255, 255, 255];
        }
        // SAFETY: reading one Copy field through the live pointer.
        (*ptr).model.get_color(field).unwrap_or([255, 255, 255])
    }
}

unsafe fn dlg_state_ptr(dlg: HWND) -> *mut DlgState {
    // SAFETY: plain userdata read.
    unsafe { GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut DlgState }
}

/// The page container proc: dialog-gray control backgrounds.
unsafe extern "system" fn page_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the brush is a shared system brush; the forwards target the
    // owning dialog, which outlives its pages.
    unsafe {
        match msg {
            // Button/combo notifications arrive at the control's parent —
            // the page — and must reach the dialog's dispatch (the
            // auto-size enable toggle, the color pickers). Upstream's page
            // DIALOGS run their own WM_COMMAND blocks; riviv keeps one
            // dispatch on the frame and forwards.
            WM_COMMAND => {
                let parent = GetParent(hwnd).unwrap_or_default();
                SendMessageW(parent, msg, Some(wparam), Some(lparam))
            }
            // Owner-draw (the color swatches) likewise goes to the parent.
            WM_DRAWITEM => {
                let parent = GetParent(hwnd).unwrap_or_default();
                SendMessageW(parent, msg, Some(wparam), Some(lparam))
            }
            WM_CTLCOLORBTN | WM_CTLCOLORSTATIC => {
                LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
