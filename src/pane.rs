//! The playlist pane (#199; upstream's viv.c:36 wishlist note "playlist
//! pane or tool window", never implemented there — riviv-authored surface
//! per the #197 spike, user ruling 2026-10-03 "keep it like the original":
//! the fourth member of the chrome-toggle family, View→Menu/Status Bar/
//! Controls now Playlist Pane).
//!
//! Shape (spike O1-O6, all recommended options): a right-edge docked
//! SysListView32 child in owner-data (virtual) mode — the playlist has no
//! upstream cap, a dropped tree makes ten-thousand-row lists — showing
//! one filename column in the Jump To order (O2b: `nav_compare`, the
//! fixed name-ascending collation upstream itself uses for its one list
//! UI — NOT the fd_compare/shuffle walk order), snapshot-rebuilt (O2a)
//! on playlist mutations, selection following the current image on every
//! navigation. Interactions (O4): click selects, double-click and Enter
//! jump via the same `request_open(OpenOrigin::Nav)` contract Jump To
//! uses. Nothing here mutates the playlist.
//!
//! The pure half (width clamp, layout rects, current-index) is
//! unit-tested; the shell stays thin: every Win32 call sits behind a
//! borrow-free seam (state is copied out before a message that can
//! re-enter the wnd_proc is sent).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject};
use windows::Win32::UI::Controls::{
    LVCF_FMT, LVCF_WIDTH, LVCFMT_LEFT, LVCOLUMNW, LVIF_TEXT, LVIS_SELECTED, LVITEMW,
    LVM_ENSUREVISIBLE, LVM_INSERTCOLUMNW, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_NOCOLUMNHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL, NMLVDISPINFOW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetParent, HMENU, SendMessageW, WINDOW_STYLE, WM_KEYDOWN, WM_SETFONT,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::playlist::{self, Playlist, PlaylistEntry};

/// The pane child's control id — the HMENU parameter of CreateWindowExW,
/// the status bar's namespace (STATUS_BAR_ID = 100; 101 is free).
pub(crate) const PANE_ID: u16 = 101;

/// The subclass id (arbitrary, unique per window).
const SUBCLASS_ID: usize = 0x199;

/// Enter-on-selection: the subclass forwards the key to the owner as this
/// message (WM_APP+1 = load replies, +2 = the Everything retry — this is
/// the next free slot; the owner arms it in window.rs).
pub(crate) const PANE_JUMP_MESSAGE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 3;

/// The persisted width's domain (px): a filename column that stays useful
/// without stealing the viewer. Default 260 (the Jump To dialog's 187 dlu
/// ≈ 280 px is the closest original-family anchor).
pub(crate) const MIN_WIDTH: i32 = 120;
pub(crate) const MAX_WIDTH: i32 = 720;
pub(crate) const DEFAULT_WIDTH: i32 = 260;

/// Clamp a hand-edited ini width into the domain. `raw <= 0` reads as the
/// default (a blanked key must not zero the pane away — the toggle owns
/// visibility, the width never does).
pub(crate) fn clamp_width(raw: i32) -> i32 {
    if raw <= 0 {
        DEFAULT_WIDTH
    } else {
        raw.clamp(MIN_WIDTH, MAX_WIDTH)
    }
}

/// The pane dock and the viewport size under it (on_size's side-dock —
/// the first non-bottom chrome, mirroring toolbar::strip_layout's
/// purity). `bottom_chrome_h` is the status bar + toolbar strip already
/// docked below; the pane spans from the client top down to it, hugging
/// the right edge; the viewport gives up the width. A degenerate client
/// keeps every dimension >= 0 (the CreateWindowExW/SetWindowPos rule
/// view_target_size already upholds).
pub(crate) fn layout(client: (i32, i32), bottom_chrome_h: i32, pane_w: i32) -> (RECT, (i32, i32)) {
    let (wide, high) = (client.0.max(0), client.1.max(0));
    let body_h = (high - bottom_chrome_h).max(0);
    let pane_w = pane_w.clamp(0, wide);
    let pane = RECT {
        left: wide - pane_w,
        top: 0,
        right: wide,
        bottom: body_h,
    };
    (pane, (wide - pane_w, body_h))
}

/// The pane's item snapshot (#197 O2a): the playlist's entries in the
/// Jump To order, or — single-file mode — the current image's directory
/// scanned FLAT (the jumpto_dlg #39 construction, extracted verbatim:
/// is_valid_path files, metadata-read ticks, id 0), or nothing on an
/// empty viewer. Returns the scanned directory so the caller can re-scan
/// only when the fallback source moved.
pub(crate) fn snapshot(
    list: &Playlist,
    current_path: Option<&OsStr>,
) -> (Vec<PlaylistEntry>, Option<PathBuf>) {
    if !list.is_empty() {
        let mut items = list.entries().to_vec();
        // Always name-ascending (the Jump To order, viv.c:13085-13086 —
        // upstream `_viv_nav_compare`, NEVER the config sort). Rust's sort
        // is stable; upstream's qsort is not — fully-tied items (same name
        // AND id) can permute differently, which no observable behavior
        // reads.
        items.sort_by(playlist::nav_compare);
        return (items, None);
    }
    let Some(current) = current_path else {
        // An empty viewer shows an empty list — upstream scans the cwd
        // only in _viv_home, never here (viv.c:13035-13066).
        return (Vec::new(), None);
    };
    let Some(dir) = Path::new(current).parent() else {
        return (Vec::new(), None);
    };
    let mut scanned = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
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
    scanned.sort_by(playlist::nav_compare);
    (scanned, Some(dir.to_path_buf()))
}

/// The current image's position in a snapshot: full-path exact match
/// (jumpto_dlg #39 rule, viv.c:13089-13109 — case-sensitive whole-string
/// equality; no fuzzy, no id fallback: the fallback scan's id-0 entries
/// only match by path anyway).
pub(crate) fn current_index(
    items: &[PlaylistEntry],
    current_path: Option<&OsStr>,
) -> Option<usize> {
    let current = current_path?;
    items.iter().position(|e| e.path == current)
}

/// The pane's per-window state (a WindowState field, like `controls`).
#[derive(Debug, Default)]
pub(crate) struct Pane {
    /// The ListView child. Default-invalid until `run()` creates it;
    /// every consumer guards on it (a failed creation loses the pane,
    /// like a failed status bar).
    pub(crate) hwnd: HWND,
    /// The item snapshot the owner-data callbacks read (UI thread only —
    /// the ListView asks for text during its paint via WM_NOTIFY).
    pub(crate) items: Vec<PlaylistEntry>,
    /// The directory the fallback snapshot scanned (None = the snapshot
    /// came from the playlist); the display-change hook rescans when the
    /// current file's parent leaves it.
    pub(crate) scanned_dir: Option<PathBuf>,
}

/// Create the pane child over `parent` (the `run()` startup path, next to
/// the viewport creation). The caller owns visibility (config-gated
/// ShowWindow) and the first on_size docks it.
pub(crate) fn create(parent: HWND) -> Result<Pane, String> {
    // SAFETY: parent is our live top-level window; the class is comctl32's
    // list view, registered by run()'s InitCommonControlsEx (ICC_WIN95_
    // CLASSES covers ICC_LISTVIEW_CLASSES).
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("SysListView32"),
            PCWSTR::null(),
            // Owner-data virtual report, no header, one selection that
            // stays painted when the focus leaves (LVS_SHOWSELALWAYS —
            // the pane is a monitor, not an editor). The LVS_* bits are
            // the low word's plain u32s, like SBARS_SIZEGRIP in status.rs.
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_CLIPCHILDREN.0
                    | WS_CLIPSIBLINGS.0
                    | LVS_REPORT
                    | LVS_NOCOLUMNHEADER
                    | LVS_SHOWSELALWAYS
                    | LVS_OWNERDATA
                    | LVS_SINGLESEL,
            ),
            0,
            0,
            DEFAULT_WIDTH,
            0,
            Some(parent),
            // SAFETY: the control id rides the HMENU slot (the status
            // bar's 100/101 namespace).
            Some(HMENU(PANE_ID as *mut core::ffi::c_void)),
            None,
            None,
        )
    }
    .map_err(|e| format!("SysListView32 CreateWindowExW failed: {e}"))?;
    // SAFETY: hwnd is the live child we just created; every call below is
    // documented for it, on the owning thread.
    unsafe {
        // The stock dialog font, like every control this codebase makes.
        let font = GetStockObject(DEFAULT_GUI_FONT);
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(true as isize)),
        );
        // Full-row select + double buffering (the comctl-recommended
        // flicker-free owner-data pair).
        SendMessageW(
            hwnd,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(0)),
            Some(LPARAM(
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
            )),
        );
        // The one filename column; its width tracks the pane on layout.
        let col = LVCOLUMNW {
            mask: LVCF_FMT | LVCF_WIDTH,
            fmt: LVCFMT_LEFT,
            cx: DEFAULT_WIDTH,
            ..Default::default()
        };
        SendMessageW(
            hwnd,
            LVM_INSERTCOLUMNW,
            Some(WPARAM(0)),
            Some(LPARAM(&col as *const _ as isize)),
        );
        // Enter-on-selection needs key handling the ListView does not do
        // itself — subclass it and forward the key to the owner.
        if !SetWindowSubclass(hwnd, Some(pane_subclass_proc), SUBCLASS_ID, 0).as_bool() {
            return Err("SetWindowSubclass failed".to_string());
        }
    }
    Ok(Pane {
        hwnd,
        items: Vec::new(),
        scanned_dir: None,
    })
}

/// The pane's subclass: the only key the pane owns is Enter = jump (the
/// owner answers PANE_JUMP_MESSAGE with the request_open contract);
/// everything else runs the comctl32 default chain.
unsafe extern "system" fn pane_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_KEYDOWN && wparam.0 == VK_RETURN.0 as usize {
        // SAFETY: GetParent of a child is its owner — the live main
        // window; the message is ours (WM_APP+3), consumed or ignored.
        if let Ok(owner) = unsafe { GetParent(hwnd) } {
            // SAFETY: same-thread send into the owner's wnd_proc.
            unsafe { SendMessageW(owner, PANE_JUMP_MESSAGE, Some(WPARAM(0)), Some(LPARAM(0))) };
        }
        return LRESULT(0);
    }
    // SAFETY: hwnd is the subclassed child; DefSubclassProc runs the
    // remaining chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The pane's paint-side callback: fill the owner-data text for one row
/// (the filename part, the Jump To display rule). `nmlv` is the
/// LVN_GETDISPINFO block the ListView passed through WM_NOTIFY.
///
/// SAFETY: `nmlv` points at the notification block for the duration of
/// the message; the text buffer inside it is the ListView's own.
pub(crate) unsafe fn disp_info(pane: &Pane, nmlv: *mut NMLVDISPINFOW) {
    // SAFETY: the pointer is the notification block the caller validated;
    // the borrow lives to the end of this fill only.
    let di = unsafe { &mut (*nmlv).item };
    if di.iItem < 0 || (di.mask & LVIF_TEXT).0 == 0 {
        return;
    }
    let Some(entry) = pane.items.get(di.iItem as usize) else {
        // A row past the snapshot tail (a stale paint racing a rebuild):
        // empty text, the next SETITEMCOUNT repaints with the truth.
        if !di.pszText.is_null() && di.cchTextMax > 0 {
            // SAFETY: pszText is the ListView's own buffer.
            unsafe { *di.pszText.as_ptr() = 0 };
        }
        return;
    };
    let name = playlist::filename_part(entry.path.as_os_str());
    let mut written = 0usize;
    for (unit, slot) in name.iter().copied().zip(
        // SAFETY: pszText is the ListView's buffer of cchTextMax wchars.
        unsafe {
            std::slice::from_raw_parts_mut(di.pszText.as_ptr(), di.cchTextMax.max(0) as usize)
        }
        .iter_mut(),
    ) {
        *slot = unit;
        written += 1;
    }
    if written < di.cchTextMax.max(0) as usize {
        // SAFETY: written < cchTextMax leaves room for the NUL.
        unsafe { *di.pszText.as_ptr().add(written) = 0 };
    }
}

/// Publish a fresh snapshot to the control (send-only — the caller has
/// ALREADY stored the items under the state borrow and dropped it: these
/// sends re-enter the wnd_proc, whose GETDISPINFO handler reads the new
/// items). `current` is the pre-computed selection index
/// ([`current_index`]), None leaving the selection untouched.
pub(crate) fn publish(pane_hwnd: HWND, count: usize, current: Option<usize>) {
    // SAFETY: the live pane child; SETITEMCOUNT may repaint synchronously.
    unsafe {
        SendMessageW(
            pane_hwnd,
            LVM_SETITEMCOUNT,
            Some(WPARAM(count)),
            Some(LPARAM(0)),
        );
    }
    select(pane_hwnd, current);
}

/// Move the selection/highlight onto an index and scroll it into view
/// (send-only, same re-entrancy contract as [`publish`]) — the
/// display-change hook, cheap: two state sends, no rebuild.
pub(crate) fn select(pane_hwnd: HWND, index: Option<usize>) {
    let Some(index) = index else {
        return;
    };
    let mut item = LVITEMW {
        mask: Default::default(),
        stateMask: LVIS_SELECTED,
        state: LVIS_SELECTED,
        iItem: index as i32,
        ..Default::default()
    };
    // SAFETY: the live pane child; SETITEMSTATE/ENSUREVISIBLE may repaint.
    unsafe {
        SendMessageW(
            pane_hwnd,
            LVM_SETITEMSTATE,
            Some(WPARAM(index)),
            Some(LPARAM(&mut item as *mut _ as isize)),
        );
        SendMessageW(
            pane_hwnd,
            LVM_ENSUREVISIBLE,
            Some(WPARAM(index)),
            Some(LPARAM(0)),
        );
    }
}

/// Size the pane child (on_size's dock) and track the column width to the
/// new client width.
pub(crate) fn dock(pane_hwnd: HWND, rect: RECT) {
    // SAFETY: the caller resized via SetWindowPos; the column width
    // follows the new interior (minus the vertical scrollbar's would-be
    // room is not worth the probe — the header is hidden, a clipped tail
    // ellipsizes like every native list).
    unsafe {
        let wide = rect.right - rect.left;
        SendMessageW(
            pane_hwnd,
            LVM_SETCOLUMNWIDTH,
            Some(WPARAM(0)),
            Some(LPARAM(wide as isize)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn entry(path: &str, id: u64) -> PlaylistEntry {
        PlaylistEntry {
            path: OsString::from(path),
            modified: id as i64,
            created: 0,
            size: 0,
            id,
        }
    }

    #[test]
    fn clamp_width_reads_nonpositive_as_the_default() {
        assert_eq!(clamp_width(0), DEFAULT_WIDTH);
        assert_eq!(clamp_width(-5), DEFAULT_WIDTH);
    }

    #[test]
    fn clamp_width_pins_both_ends() {
        assert_eq!(clamp_width(1), MIN_WIDTH);
        assert_eq!(clamp_width(MIN_WIDTH), MIN_WIDTH);
        assert_eq!(clamp_width(MAX_WIDTH), MAX_WIDTH);
        assert_eq!(clamp_width(MAX_WIDTH + 1), MAX_WIDTH);
    }

    #[test]
    fn layout_docks_the_pane_right_and_shrinks_the_viewport() {
        let (pane, view) = layout((800, 600), 40, 200);
        assert_eq!(
            (pane.left, pane.top, pane.right, pane.bottom),
            (600, 0, 800, 560)
        );
        assert_eq!(view, (600, 560));
    }

    #[test]
    fn layout_hides_to_zero_width_when_the_pane_is_off() {
        let (pane, view) = layout((800, 600), 40, 0);
        assert_eq!((pane.left, pane.right), (800, 800));
        assert_eq!(view, (800, 560));
    }

    #[test]
    fn layout_never_hands_out_negative_dimensions() {
        let (pane, view) = layout((100, 0), 40, 200);
        assert_eq!((pane.top, pane.bottom), (0, 0));
        assert_eq!((view.0, view.1), (0, 0));
        // A pane wider than the client pins to the client width, not past it.
        assert_eq!((pane.left, pane.right), (0, 100));
    }

    #[test]
    fn current_index_matches_the_exact_full_path() {
        let items = vec![entry(r"c:\a\b.png", 0), entry(r"c:\a\c.png", 1)];
        assert_eq!(
            current_index(&items, Some(OsStr::new(r"c:\a\c.png"))),
            Some(1)
        );
        assert_eq!(current_index(&items, Some(OsStr::new(r"c:\a\d.png"))), None);
        assert_eq!(current_index(&items, None), None);
    }
}
