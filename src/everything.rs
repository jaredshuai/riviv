//! Everything IPC search (#22): the QUERY2/LIST2 wire codec (pure), the
//! query senders and the hand-assembled Search Everything dialog.
//!
//! Upstream talks to a running Everything (1.4a) through its WM_COPYDATA
//! IPC: `_viv_send_everything_search` (viv.c:13378-13483) wraps the term in
//! an image-extension filter, probes which file infos the index carries,
//! and sends an `EVERYTHING_IPC_QUERY2` (everything_ipc.h:758-792, pack(1),
//! seven DWORDs + NUL-terminated UTF-16) to the Everything taskbar window;
//! the results come back as an `EVERYTHING_IPC_LIST2` WM_COPYDATA to OUR
//! window, dispatched in `window.rs`'s WM_COPYDATA arm (viv.c:3808-3904:
//! OPEN clears + refills the playlist + homes, ADD appends, RANDOM opens
//! one random item with an out-of-range retry loop driven by
//! `_VIV_WM_RETRY_RANDOM_EVERYTHING_SEARCH`, viv.c:248/3796-3802).
//!
//! The dialog (upstream IDD_EVERYTHING, rc:175-183 + viv.c:13329-13376) is
//! hand-assembled like the Options dialog (#24): no rc template, Tab/Enter/
//! Esc via a local IsDialogMessageW pump over the disabled owner.
//!
//! Borrow discipline (the `state_of` contract, window.rs): the LIST2 reply
//! is delivered by Everything's SendMessage, which can reenter wnd_proc
//! INSIDE our own query SendMessage — so no state borrow may straddle
//! `find_everything` / the probes / the query send. Every sender clones
//! what it needs in one short borrow, drops it, then talks to Everything.

use std::ffi::c_void;
use std::os::windows::ffi::OsStringExt;

use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush, HBRUSH, HGDIOBJ,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON,
    BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    ES_AUTOHSCROLL, FindWindowA, GWLP_USERDATA, GetDialogBaseUnits, GetDlgItem, GetMessageW,
    GetParent, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, HMENU, IsChild, IsDialogMessageW,
    IsWindow, MB_ICONERROR, MB_OK, MSG, MessageBoxW, PostQuitMessage, RegisterClassExW, SW_SHOW,
    SendMessageW, SetWindowLongPtrW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_APP, WM_CLOSE, WM_COMMAND, WM_COPYDATA, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_SETFONT, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE,
    WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, s, w};

use crate::copydata::STRING_SIZE;
use crate::loc;
use crate::window::{home_open, state_of};

/// The Everything taskbar-notification window class (everything_ipc.h:420)
/// — the IPC target, always present while Everything runs.
const EVERYTHING_WNDCLASS: PCSTR = s!("EVERYTHING_TASKBAR_NOTIFICATION");

/// Everything's private message id (everything_ipc.h:32: `WM_USER`) and the
/// file-info-indexed probe (h:101) — sent as
/// `SendMessage(hwnd, WM_USER, 411, EVERYTHING_IPC_FILE_INFO_*)` with 411 in
/// **wParam** and the file info in **lParam** (h:101's own usage comment).
const EVERYTHING_WM_IPC: u32 = 0x0400;
const IPC_IS_FILE_INFO_INDEXED: usize = 411;
const FILE_INFO_FILE_SIZE: isize = 1;
const FILE_INFO_DATE_CREATED: isize = 3;
const FILE_INFO_DATE_MODIFIED: isize = 4;

/// The WM_COPYDATA payload id of a Unicode QUERY2 (everything_ipc.h:679) —
/// hardcoded to the W value; riviv is always "Unicode".
const COPYDATA_QUERY2W: usize = 18;

/// QUERY2 request flags (everything_ipc.h:708-714).
const REQ_FULL_PATH_AND_NAME: u32 = 0x0000_0004;
const REQ_SIZE: u32 = 0x0000_0010;
const REQ_DATE_CREATED: u32 = 0x0000_0020;
const REQ_DATE_MODIFIED: u32 = 0x0000_0040;

/// `EVERYTHING_IPC_ALLRESULTS` (everything_ipc.h:457).
const ALL_RESULTS: u32 = 0xFFFF_FFFF;

/// `EVERYTHING_IPC_SORT_NAME_ASCENDING` (everything_ipc.h:681).
const SORT_NAME_ASCENDING: u32 = 1;

/// `EVERYTHING_IPC_FOLDER` item flag (everything_ipc.h:438).
const ITEM_FOLDER: u32 = 0x0000_0001;

/// Upstream's per-item `filename_len < MAX_PATH` gate (viv.c:3750/3850).
const MAX_PATH_CHARS: u32 = 260;

/// The WM_COPYDATA payload ids of Everything replies — upstream's
/// `_VIV_COPYDATA_*` enum order (viv.c:293-297) after COMMAND_LINE (0).
pub(crate) const COPYDATA_OPEN_EVERYTHING_SEARCH: usize = 1;
pub(crate) const COPYDATA_ADD_EVERYTHING_SEARCH: usize = 2;
pub(crate) const COPYDATA_RANDOM_EVERYTHING_SEARCH: usize = 3;

/// The random-search retry kick (upstream `_VIV_WM_RETRY_RANDOM_EVERYTHING_
/// SEARCH` = WM_USER+2, viv.c:248): an out-of-range random offset learns
/// the real total and re-sends. WM_APP+2 here — WM_APP+1 is the load kick,
/// and WM_APP is the canonical private-message range for a window class.
pub(crate) const RETRY_RANDOM_MESSAGE: u32 = WM_APP + 2;

/// The 1601→1970 epoch shift in 100 ns ticks — FILETIMEs convert onto
/// riviv's signed unix-tick mtimes (playlist.rs `modified_ticks`), keeping
/// pre-1970 values negative so the descending-mtime sort survives.
const FILETIME_UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// One QUERY2, field order = wire order (everything_ipc.h:758-792, pack(1):
/// seven DWORDs then the NUL-terminated UTF-16 search string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Query2 {
    /// The window receiving the results — 32 bits by protocol design
    /// ("only 32bits are required to store a window handle. (even on
    /// x64)", everything_ipc.h:761; upstream truncates the same way,
    /// viv.c:13446 — a handle above 32 bits loses its reply, upstream's
    /// own degradation).
    pub(crate) reply_hwnd: u32,
    pub(crate) reply_copydata_message: u32,
    pub(crate) search_flags: u32,
    pub(crate) offset: u32,
    pub(crate) max_results: u32,
    pub(crate) request_flags: u32,
    pub(crate) sort_type: u32,
}

impl Query2 {
    /// The wire bytes: a 28-byte header + the search string NUL-terminated
    /// (upstream's size math, viv.c:13425: `sizeof(QUERY2)` — which does
    /// NOT include the string, the C struct only comments it — plus
    /// `(len + 1) * sizeof(wchar_t)`).
    pub(crate) fn encode(&self, search_string: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(28 + (search_string.len() + 1) * 2);
        for field in [
            self.reply_hwnd,
            self.reply_copydata_message,
            self.search_flags,
            self.offset,
            self.max_results,
            self.request_flags,
            self.sort_type,
        ] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        for unit in search_string {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

/// Wrap a search term for image-only results (upstream's literal,
/// viv.c:13421-13423): the nine playable extensions — the same list as the
/// association table — then the term in Everything's `<>` literal quoting.
pub(crate) fn wrap_search(search: &[u16]) -> Vec<u16> {
    const PREFIX: &str = "ext:bmp;gif;ico;jpeg;jpg;png;tif;tiff;webp <";
    let mut out: Vec<u16> = PREFIX.encode_utf16().collect();
    out.extend_from_slice(search);
    out.push(u16::from(b'>'));
    out
}

/// The request flags for a query (upstream viv.c:13427-13442): always the
/// full path; a FILE_SIZE-indexed database adds SIZE **and** DATE_MODIFIED
/// (upstream's own coupling), then each indexed date adds itself.
pub(crate) fn compute_request_flags(
    size_indexed: bool,
    modified_indexed: bool,
    created_indexed: bool,
) -> u32 {
    let mut flags = REQ_FULL_PATH_AND_NAME;
    if size_indexed {
        flags |= REQ_SIZE | REQ_DATE_MODIFIED;
    }
    if modified_indexed {
        flags |= REQ_DATE_MODIFIED;
    }
    if created_indexed {
        flags |= REQ_DATE_CREATED;
    }
    flags
}

/// One parsed LIST2 item: the full path (when it survived upstream's
/// filters) and the mtime in riviv's unix ticks — `None` when the query did
/// not request DATE_MODIFIED (upstream's zeroed FILETIME becomes 0 at the
/// playlist, matching `Playlist::add`'s unstatable corner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct List2Item {
    pub(crate) path: Vec<u16>,
    pub(crate) modified_ticks: Option<i64>,
}

/// A decoded LIST2 (everything_ipc.h:823-847): the totals plus the items
/// that passed upstream's filters. `numitems` rides along for the RANDOM
/// arm's retry decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct List2Reply {
    pub(crate) totitems: u32,
    pub(crate) numitems: u32,
    pub(crate) items: Vec<List2Item>,
}

/// Read a little-endian DWORD at `at`, if fully inside `bytes`.
fn dword_at(bytes: &[u8], at: usize) -> Option<u32> {
    let raw = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(raw.try_into().expect("4 bytes")))
}

/// FILETIME (100 ns since 1601) → riviv's signed unix ticks. The cast to
/// i64 comes FIRST: a pre-1970 value (Everything zeroes what it cannot
/// stat) must land negative, never wrap around in u64.
pub(crate) fn filetime_to_unix_ticks(filetime: u64) -> i64 {
    (filetime as i64).wrapping_sub(FILETIME_UNIX_EPOCH_TICKS)
}

/// Parse an `EVERYTHING_IPC_LIST2` (upstream viv.c:3832-3896): a 20-byte
/// header (totitems, numitems, offset, request_flags echo, sort_type), the
/// `numitems` item headers (flags + data_offset), then each item's data —
/// a DWORD character length, the path, and the requested trailers in
/// SIZE → DATE_CREATED → DATE_MODIFIED order per the flags STORED AT SEND
/// TIME (upstream never reads the echoed request_flags, viv.c:3866+).
///
/// An item joins the output only through upstream's filters: not a folder,
/// length under MAX_PATH, and a playable extension. riviv's hardening over
/// upstream's raw pointer walk (which out-of-bounds-reads garbage): an
/// item header past the buffer ends the walk (a hostile `numitems` cannot
/// spin), a malformed data area only drops that item.
pub(crate) fn parse_list2(bytes: &[u8], stored_request_flags: u32) -> List2Reply {
    let read_dword = |at: usize| dword_at(bytes, at);
    let totitems = read_dword(0).unwrap_or(0);
    let numitems = read_dword(4).unwrap_or(0);
    let mut items = Vec::new();
    let mut at = 20usize;
    for _ in 0..numitems {
        let Some(flags) = read_dword(at) else { break };
        let Some(data_offset) = read_dword(at + 4) else {
            break;
        };
        at += 8;
        if flags & ITEM_FOLDER != 0 {
            continue; // upstream's commented-out folder arm (viv.c:3836-3840)
        }
        let Some(len) = read_dword(data_offset as usize) else {
            continue;
        };
        let p = data_offset as usize + 4;
        if len >= MAX_PATH_CHARS {
            continue; // upstream's `filename_len < MAX_PATH` gate (viv.c:3850)
        }
        let len = len as usize;
        let Some(str_end) = p.checked_add(len * 2).and_then(|e| e.checked_add(2)) else {
            continue;
        };
        let Some(str_bytes) = bytes.get(p..str_end) else {
            continue;
        };
        let path: Vec<u16> = str_bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take(len)
            .collect();
        if !crate::playlist::is_valid_path(std::ffi::OsString::from_wide(&path).as_os_str()) {
            continue; // upstream `_viv_is_valid_filename` (viv.c:3861)
        }
        // The trailer walk consumes SIZE, DATE_CREATED, DATE_MODIFIED in
        // wire order (viv.c:3866-3890) — only MODIFIED is kept, the others
        // are stepped over to reach it.
        let mut trailer = str_end;
        let mut read_trailer = || -> Option<u64> {
            let lo = u64::from(read_dword(trailer)?);
            let hi = u64::from(read_dword(trailer.checked_add(4)?)?);
            trailer += 8;
            Some(lo | (hi << 32))
        };
        if stored_request_flags & REQ_SIZE != 0 {
            let _ = read_trailer();
        }
        if stored_request_flags & REQ_DATE_CREATED != 0 {
            let _ = read_trailer();
        }
        let modified_ticks = (stored_request_flags & REQ_DATE_MODIFIED != 0)
            .then(&mut read_trailer)
            .flatten()
            .map(filetime_to_unix_ticks);
        items.push(List2Item {
            path,
            modified_ticks,
        });
    }
    List2Reply {
        totitems,
        numitems,
        items,
    }
}

/// The MSVC CRT `rand()` — the generator upstream's offset formula is
/// written against (viv.c:13912): a 32-bit LCG, output `(state >> 16) &
/// 0x7fff`, `RAND_MAX` 32767. The state lives in `WindowState` (upstream:
/// CRT global), seeded from the performance counter at randomize time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MsvcRand(pub(crate) u32);

impl MsvcRand {
    /// Advance and yield the next output (MSVC advances BEFORE reading).
    pub(crate) fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011);
        (self.0 >> 16) & 0x7fff
    }
}

/// The random result index (upstream viv.c:13912:
/// `((rand() * RAND_MAX) + rand()) % tot_results`): two draws widened into
/// a ~30-bit value — 32767² + 32767 stays under i32::MAX, so the C
/// arithmetic never overflows either. `tot == 0` cannot happen upstream
/// (the total is 0xFFFFFFFF or a reply's positive `totitems`); 0 maps to 0
/// here to keep the function total.
pub(crate) fn random_offset(r1: u32, r2: u32, tot: u32) -> u32 {
    if tot == 0 {
        return 0;
    }
    (r1 * 32767 + r2) % tot
}

/// Locate a running Everything IPC window (upstream `FindWindowA(
/// EVERYTHING_IPC_WNDCLASSA, 0)`, viv.c:13412).
fn find_everything() -> Option<HWND> {
    // SAFETY: a pure class lookup; the returned window is only ever used
    // through SendMessage calls below, never dereferenced.
    unsafe { FindWindowA(EVERYTHING_WNDCLASS, PCSTR::null()) }.ok()
}

/// Probe which file infos Everything's index carries (upstream
/// viv.c:13429-13442) and fold them into request flags.
fn probe_request_flags(everything_hwnd: HWND) -> u32 {
    let indexed = |info: isize| {
        // SAFETY: a read-only WM_USER query to a foreign top-level window
        // with 411 in wParam and the file info in lParam
        // (everything_ipc.h:101's usage); an absent/low-integrity
        // Everything answers 0 = "not indexed", exactly upstream's
        // ignored-return probe.
        unsafe {
            SendMessageW(
                everything_hwnd,
                EVERYTHING_WM_IPC,
                Some(WPARAM(IPC_IS_FILE_INFO_INDEXED)),
                Some(LPARAM(info)),
            )
        }
        .0 != 0
    };
    compute_request_flags(
        indexed(FILE_INFO_FILE_SIZE),
        indexed(FILE_INFO_DATE_MODIFIED),
        indexed(FILE_INFO_DATE_CREATED),
    )
}

/// Send one QUERY2 to Everything (upstream viv.c:13444-13462). The
/// SendMessage return is ignored like upstream's; the reply — if any —
/// lands in `window.rs`'s WM_COPYDATA arm, possibly reentering inside this
/// very call, which is why no state borrow is live here.
fn send_query(everything_hwnd: HWND, hwnd_main: HWND, query: &Query2, search_string: &[u16]) {
    let bytes = query.encode(search_string);
    let cds = COPYDATASTRUCT {
        dwData: COPYDATA_QUERY2W,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr() as *mut c_void,
    };
    // SAFETY: `cds` outlives the synchronous send (WM_COPYDATA's contract:
    // the receiver reads, never keeps); wParam is the main window like
    // upstream (viv.c:13460) — Everything routes on reply_hwnd, the wParam
    // is the conventional sender id.
    let _ = unsafe {
        SendMessageW(
            everything_hwnd,
            WM_COPYDATA,
            Some(WPARAM(hwnd_main.0 as usize)),
            Some(LPARAM(&cds as *const COPYDATASTRUCT as isize)),
        )
    };
}

/// The full send behind the dialog's OK (upstream
/// `_viv_send_everything_search`, viv.c:13378-13483).
///
/// Returns whether the dialog may close: the randomize arm always succeeds
/// (it re-queries through navigation from here on); the plain arm succeeds
/// when a query went out, and shows the "Everything not available" box over
/// `hwnd_msg_parent` (the dialog) and fails when Everything is not running.
pub(crate) fn send_search(
    hwnd_main: HWND,
    hwnd_msg_parent: HWND,
    add: bool,
    randomize: bool,
    search: &[u16],
) -> bool {
    if randomize {
        // add is ignored in this mode (viv.c:13382).
        // SAFETY: the borrow spans plain field stores and the playlist
        // clear — nothing pumps (the QPC query is a kernel call).
        if let Some(state) = unsafe { state_of(hwnd_main) } {
            state.random_search = Some(search.to_vec());
            state.random_tot_results = ALL_RESULTS;
            state.playlist.clear();
            let mut counter: i64 = 0;
            // SAFETY: a cheap counter read; the result only seeds the LCG
            // (upstream takes the LARGE_INTEGER's LowPart, the low 32
            // bits, viv.c:13390-13395).
            let _ = unsafe { QueryPerformanceCounter(&mut counter) };
            state.random_rand_state = counter as u32;
        }
        // The random mode's first query goes through home (upstream
        // `_viv_home(0,0)` whose random branch sends it, viv.c:13397/6122).
        home_open(hwnd_main, false);
        return true;
    }
    // A plain search exits random mode BEFORE looking for Everything
    // (viv.c:13405-13410).
    // SAFETY: the borrow spans one field store.
    if let Some(state) = unsafe { state_of(hwnd_main) } {
        state.random_search = None;
    }
    let Some(everything_hwnd) = find_everything() else {
        // Upstream's error box (viv.c:13466-13480): MB_OK|MB_ICONERROR over
        // the DIALOG (the code, not the stale MB_ICONQUESTION comment).
        // SAFETY: a modal box over the live dialog; pumps internally, no
        // state borrow is live.
        unsafe {
            MessageBoxW(
                Some(hwnd_msg_parent),
                &HSTRING::from(loc::get(loc::Id::EverythingNotAvailable)),
                &HSTRING::from(loc::get(loc::Id::AppName)),
                MB_OK | MB_ICONERROR,
            )
        };
        return false;
    };
    // Probe, then STORE the flags BEFORE sending (upstream order,
    // viv.c:13427→13451): the reply can reenter inside the send below and
    // must read the flags this query asked for.
    let flags = probe_request_flags(everything_hwnd);
    // SAFETY: the borrow spans one field store.
    if let Some(state) = unsafe { state_of(hwnd_main) } {
        state.everything_request_flags = flags;
    }
    let query = Query2 {
        // 32-bit truncation is the protocol's own design (see Query2).
        reply_hwnd: hwnd_main.0 as u32,
        reply_copydata_message: if add {
            COPYDATA_ADD_EVERYTHING_SEARCH as u32
        } else {
            COPYDATA_OPEN_EVERYTHING_SEARCH as u32
        },
        search_flags: 0,
        offset: 0,
        max_results: ALL_RESULTS,
        request_flags: flags,
        sort_type: SORT_NAME_ASCENDING,
    };
    send_query(everything_hwnd, hwnd_main, &query, &wrap_search(search));
    true
}

/// One random-image query (upstream `_viv_send_random_everything_search`,
/// viv.c:13871-13929): the offset redraws from the LCG, the flags re-probe
/// and overwrite the stored set (the ONLY writer on the pure-randomize
/// path), and a missing Everything is silently nothing — no error box on
/// this arm.
pub(crate) fn send_random(hwnd_main: HWND) {
    // SAFETY: the borrow spans the clone and the LCG advance — nothing
    // pumps.
    let armed = (unsafe { state_of(hwnd_main) }).and_then(|state| {
        let search = state.random_search.clone()?;
        let tot = state.random_tot_results;
        let mut rng = MsvcRand(state.random_rand_state);
        let r1 = rng.next();
        let r2 = rng.next();
        state.random_rand_state = rng.0;
        Some((search, tot, r1, r2))
    });
    let Some((search, tot, r1, r2)) = armed else {
        return; // not in random mode (upstream would string_cat(NULL) — UB)
    };
    let Some(everything_hwnd) = find_everything() else {
        return; // upstream's silent no-op (viv.c:13877-13928 has no else)
    };
    let flags = probe_request_flags(everything_hwnd);
    // SAFETY: the borrow spans one field store; the reply reenters only
    // inside the send below, after this borrow dropped.
    if let Some(state) = unsafe { state_of(hwnd_main) } {
        state.everything_request_flags = flags;
    }
    let query = Query2 {
        reply_hwnd: hwnd_main.0 as u32,
        reply_copydata_message: COPYDATA_RANDOM_EVERYTHING_SEARCH as u32,
        search_flags: 0,
        offset: random_offset(r1, r2, tot),
        max_results: 1,
        request_flags: flags,
        sort_type: SORT_NAME_ASCENDING,
    };
    send_query(everything_hwnd, hwnd_main, &query, &wrap_search(&search));
}

/// The dialog's client size in dialog units (rc:175: `186, 63`).
const DLG_WIDE: i32 = 186;
const DLG_HIGH: i32 = 63;
/// Control ids: upstream's resource.h values (1021/1046) for the checkbox
/// and edit, the Win32 standards for the buttons.
const RANDOM_CHECK_ID: i32 = 1021;
const EDIT_ID: i32 = 1046;
const IDOK_BTN: i32 = 1;
const IDCANCEL_BTN: i32 = 2;

/// The dialog window class.
const SEARCH_CLASS: PCWSTR = w!("riviv_search");

/// The Search Everything dialog (upstream `_viv_search_everything` +
/// IDD_EVERYTHING + `_viv_search_everything_proc`, viv.c:13329-13376,
/// rc:175-183): an edit box, a Randomize checkbox (cleared on every open —
/// the rc template carries no check state), OK/Cancel. OK sends the query
/// and closes only on success; Cancel/X/Esc always close.
pub(crate) fn open_search_dialog(owner: HWND, add: bool) {
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

    // rc: 186x63 dlu client; frame = popup + caption + sysmenu + the modal
    // frame look; centered over the owner (upstream os_center_dialog).
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

    let caption = HSTRING::from(loc::get(if add {
        loc::Id::EverythingAddCaption
    } else {
        loc::Id::EverythingLoadCaption
    }));
    // SAFETY: all creation parameters valid; creation-time messages run
    // with userdata still 0 and the proc reads it only in WM_COMMAND.
    let dlg = unsafe {
        CreateWindowExW(
            ex,
            SEARCH_CLASS,
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
        eprintln!("riviv: search dialog CreateWindowExW failed (GLE={gle})");
        return;
    };
    // Upstream parks the add flag in GWLP_USERDATA (viv.c:13337).
    // SAFETY: an integer store, read back by the proc's OK arm.
    unsafe { SetWindowLongPtrW(dlg, GWLP_USERDATA, add as isize) };
    build_controls(dlg, font, &dlu);
    // Upstream's WM_INITDIALOG returns TRUE — focus lands on the first tab
    // stop (the edit); a hand-built dialog sets it itself.
    // SAFETY: the dialog is live and owned by this thread; GetDlgItem
    // cannot pump.
    unsafe {
        let _ = SetFocus(Some(GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default()));
    }

    // The modal loop (options_dlg's shape): disable the owner, pump through
    // IsDialogMessage until the dialog is destroyed (OK-on-success, Cancel,
    // Esc, X).
    // SAFETY: standard pump calls over this thread's windows; the
    // unfiltered read keeps the disabled owner's timers dispatching like
    // DialogBox.
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
                DispatchMessageW(&msg);
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
            lpfnWndProc: Some(search_proc),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 as usize + 1) as *mut c_void),
            lpszClassName: SEARCH_CLASS,
            ..Default::default()
        })
    };
    let ok = if atom == 0 {
        // SAFETY: pure thread-error-slot read immediately after the call.
        let gle = unsafe { GetLastError() }.0;
        if gle != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS.0 {
            eprintln!("riviv: RegisterClassExW({SEARCH_CLASS:?}) failed (GLE={gle})");
            false
        } else {
            true
        }
    } else {
        true
    };
    DONE.store(ok, Ordering::Relaxed);
}

/// Lay out the controls (rc:180-183): the edit at (6,6,173,14) with
/// ES_AUTOHSCROLL, the checkbox at (6,24,51,10), OK at (72,42,50,14) as
/// the default button, Cancel at (127,42,50,14). The checkbox caption is
/// localized at runtime like upstream's WM_INITDIALOG (viv.c:13340).
fn build_controls(dlg: HWND, font: HGDIOBJ, dlu: &impl Fn(i32, i32) -> (i32, i32)) {
    // SAFETY: every creation uses the live `dlg` parent, ids fixed above,
    // and strings that outlive the calls; failures leave a missing control
    // the OK arm treats as empty text.
    unsafe {
        let (ex, ey) = dlu(6, 6);
        let (ew, eh) = dlu(173, 14);
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_CLIENTEDGE.0),
            w!("EDIT"),
            None,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
            ex,
            ey,
            ew,
            eh,
            Some(dlg),
            Some(HMENU(EDIT_ID as *mut c_void)),
            None,
            None,
        );
        let (cx, cy) = dlu(6, 24);
        let (cw, ch) = dlu(51, 10);
        let check = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            &HSTRING::from(loc::get(loc::Id::EverythingRandomize)),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32),
            cx,
            cy,
            cw,
            ch,
            Some(dlg),
            Some(HMENU(RANDOM_CHECK_ID as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default();
        // rc carries no check state — every open starts unchecked, restated
        // for the hand-built path (upstream relies on the fresh template).
        let _ = SendMessageW(check, BM_SETCHECK, Some(WPARAM(0)), Some(LPARAM(0)));
        let push = |text: &str, id: i32, x: i32, default: bool| {
            let (px, py) = dlu(x, 42);
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
        push(loc::get(loc::Id::OptionsOk), IDOK_BTN, 72, true);
        push(loc::get(loc::Id::OptionsCancel), IDCANCEL_BTN, 127, false);
        // The dialog font on every control (the rc's DS_SETFONT).
        for id in [EDIT_ID, RANDOM_CHECK_ID, IDOK_BTN, IDCANCEL_BTN] {
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

/// The dialog frame proc (upstream `_viv_search_everything_proc`,
/// viv.c:13329-13371).
unsafe extern "system" fn search_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: every arm either forwards or runs dialog-local logic; the
    // send path drops its state borrows before anything that pumps.
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
            WM_DESTROY => {
                // Nothing heap-allocated lives in userdata (the add flag is
                // an integer) — just clear it.
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
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

/// OK: read the term, send the query, close only on success (upstream
/// viv.c:13348-13359 — `EndDialog` sits INSIDE the success branch).
fn on_ok(dlg: HWND) {
    let mut buf = [0u16; STRING_SIZE];
    // SAFETY: the edit is a live child; GetWindowTextW caps at the buffer
    // (1023 + NUL), the same truncation as upstream's GetDlgItemText
    // (viv.c:13352). Fail-soft: an unavailable edit reads as empty.
    let len =
        unsafe { GetWindowTextW(GetDlgItem(Some(dlg), EDIT_ID).unwrap_or_default(), &mut buf) }
            .max(0) as usize;
    let len = len.min(STRING_SIZE - 1);
    // SAFETY: short-lived integer userdata read, no borrow.
    let add = unsafe { GetWindowLongPtrW(dlg, GWLP_USERDATA) } != 0;
    // SAFETY: a pure child lookup + a message read on the dialog's own
    // checkbox.
    let randomize = unsafe {
        let check = GetDlgItem(Some(dlg), RANDOM_CHECK_ID).unwrap_or_default();
        SendMessageW(check, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0
            == BST_CHECKED.0 as isize
    };
    // The reply window is the OWNER (upstream `_viv_hwnd`, viv.c:13446);
    // the error box's parent is the dialog (viv.c:13476).
    // SAFETY: GetParent on the dialog we built on this thread; a failure
    // reads as the null owner (a desktop-parented error box).
    let owner = unsafe { GetParent(dlg) }.unwrap_or_default();
    if send_search(owner, dlg, add, randomize, &buf[..len]) {
        // SAFETY: legal on this thread; no borrow is live.
        let _ = unsafe { DestroyWindow(dlg) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn unwide(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    #[test]
    fn query2_wire_is_seven_dwords_then_nul_terminated_utf16() {
        // everything_ipc.h:758-792 (pack(1)): the field order IS the wire
        // order, request_flags sixth, sort_type seventh.
        let q = Query2 {
            reply_hwnd: 0x11223344,
            reply_copydata_message: 3,
            search_flags: 0,
            offset: 7,
            max_results: 0xFFFF_FFFF,
            request_flags: 0x74,
            sort_type: 1,
        };
        let bytes = q.encode(&wide("ab"));
        assert_eq!(bytes.len(), 28 + (2 + 1) * 2);
        assert_eq!(&bytes[..4], &0x11223344u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &3u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &0u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &7u32.to_le_bytes());
        assert_eq!(&bytes[16..20], &0xFFFF_FFFFu32.to_le_bytes());
        assert_eq!(&bytes[20..24], &0x74u32.to_le_bytes(), "request_flags 6th");
        assert_eq!(&bytes[24..28], &1u32.to_le_bytes(), "sort_type 7th");
        assert_eq!(&bytes[28..], &[b'a', 0, b'b', 0, 0, 0]);
    }

    #[test]
    fn query2_empty_search_is_bare_header_plus_nul() {
        let q = Query2 {
            reply_hwnd: 0,
            reply_copydata_message: 1,
            search_flags: 0,
            offset: 0,
            max_results: 0,
            request_flags: 4,
            sort_type: 1,
        };
        let mut expect = Vec::new();
        for f in [0u32, 1, 0, 0, 0, 4, 1] {
            expect.extend_from_slice(&f.to_le_bytes());
        }
        expect.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(q.encode(&[]), expect);
    }

    #[test]
    fn wrap_search_is_ext_filter_then_quoted_term() {
        // The prefix is upstream's literal (viv.c:13421) and must stay
        // identical to the association table's extension list.
        let list = crate::assoc::EXTENSIONS.join(";");
        let text = unwide(&wrap_search(&wide("cat")));
        assert_eq!(text, format!("ext:{list} <cat>"));
    }

    #[test]
    fn wrap_search_empty_term_keeps_the_quoting() {
        // Upstream has no empty check (viv.c:13352→13421): an empty term
        // searches the extension set alone.
        assert_eq!(
            unwide(&wrap_search(&[])),
            format!("ext:{} <>", crate::assoc::EXTENSIONS.join(";"))
        );
    }

    #[test]
    fn request_flags_fold_the_probe_matrix_like_upstream() {
        // viv.c:13427-13442: base is the full path; a size-indexed db adds
        // SIZE **and** DATE_MODIFIED; each indexed date adds itself.
        assert_eq!(
            compute_request_flags(false, false, false),
            REQ_FULL_PATH_AND_NAME
        );
        assert_eq!(
            compute_request_flags(true, false, false),
            REQ_FULL_PATH_AND_NAME | REQ_SIZE | REQ_DATE_MODIFIED
        );
        assert_eq!(
            compute_request_flags(false, true, true),
            REQ_FULL_PATH_AND_NAME | REQ_DATE_CREATED | REQ_DATE_MODIFIED
        );
        assert_eq!(
            compute_request_flags(true, true, true),
            REQ_FULL_PATH_AND_NAME | REQ_SIZE | REQ_DATE_CREATED | REQ_DATE_MODIFIED
        );
    }

    /// Hand-build a LIST2 the way Everything does (the test-side encoder):
    /// items as (flags, path, size, created, modified) with the trailers
    /// written per the flag set so the parse exercises the skip order.
    type RawItem<'a> = (u32, &'a str, Option<u64>, Option<u64>, Option<u64>);
    fn list2(totitems: u32, items: &[RawItem<'_>], flags: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&totitems.to_le_bytes());
        out.extend_from_slice(&(items.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // offset
        out.extend_from_slice(&flags.to_le_bytes()); // echoed (unused)
        out.extend_from_slice(&1u32.to_le_bytes()); // sort
        let mut headers = Vec::new();
        let mut data = Vec::new();
        for &(item_flags, path, size, created, modified) in items {
            let data_offset = 20 + items.len() * 8 + data.len();
            headers.extend_from_slice(&item_flags.to_le_bytes());
            headers.extend_from_slice(&(data_offset as u32).to_le_bytes());
            let w = wide(path);
            data.extend_from_slice(&(w.len() as u32).to_le_bytes());
            for u in w {
                data.extend_from_slice(&u.to_le_bytes());
            }
            data.extend_from_slice(&0u16.to_le_bytes());
            if flags & REQ_SIZE != 0 {
                data.extend_from_slice(&size.unwrap_or(0).to_le_bytes());
            }
            if flags & REQ_DATE_CREATED != 0 {
                data.extend_from_slice(&created.unwrap_or(0).to_le_bytes());
            }
            if flags & REQ_DATE_MODIFIED != 0 {
                data.extend_from_slice(&modified.unwrap_or(0).to_le_bytes());
            }
        }
        out.extend_from_slice(&headers);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn list2_round_trips_plain_paths() {
        // The layout a fresh riviv parses with: no Everything probes ran,
        // so the stored request flags are 0 = full path only.
        let bytes = list2(
            2,
            &[
                (0, r"C:\a\b.png", None, None, None),
                (0, r"C:\c.gif", None, None, None),
            ],
            0,
        );
        let reply = parse_list2(&bytes, 0);
        assert_eq!(reply.totitems, 2);
        assert_eq!(reply.numitems, 2);
        assert_eq!(reply.items.len(), 2);
        assert_eq!(unwide(&reply.items[0].path), r"C:\a\b.png");
        assert_eq!(reply.items[0].modified_ticks, None);
        assert_eq!(unwide(&reply.items[1].path), r"C:\c.gif");
    }

    #[test]
    fn list2_skips_size_and_created_to_read_modified() {
        // With the full flag set the trailers ride SIZE→CREATED→MODIFIED
        // (viv.c:3866-3890); the parser must step over the first two to
        // reach the mtime it keeps.
        let modified_filetime = 133_000_000_000_000_000u64; // ~2022
        let bytes = list2(
            1,
            &[(
                0,
                r"C:\a.png",
                Some(1234),
                Some(111),
                Some(modified_filetime),
            )],
            REQ_FULL_PATH_AND_NAME | REQ_SIZE | REQ_DATE_CREATED | REQ_DATE_MODIFIED,
        );
        let reply = parse_list2(&bytes, 0x74);
        assert_eq!(reply.items.len(), 1);
        assert_eq!(
            reply.items[0].modified_ticks,
            Some(filetime_to_unix_ticks(modified_filetime))
        );
    }

    #[test]
    fn list2_without_modified_flag_yields_none_mtime() {
        let flags = REQ_FULL_PATH_AND_NAME | REQ_SIZE;
        let bytes = list2(1, &[(0, r"C:\a.png", Some(9), None, None)], flags);
        let reply = parse_list2(&bytes, flags);
        assert_eq!(reply.items[0].modified_ticks, None);
    }

    #[test]
    fn list2_zero_items_parses_empty() {
        let bytes = list2(0, &[], 0);
        let reply = parse_list2(&bytes, 0);
        assert_eq!(
            (reply.totitems, reply.numitems, reply.items.len()),
            (0, 0, 0)
        );
    }

    #[test]
    fn list2_drops_folders_and_bad_extensions() {
        let bytes = list2(
            3,
            &[
                (ITEM_FOLDER, r"C:\dir.png", None, None, None),
                (0, r"C:\a.txt", None, None, None),
                (0, r"C:\ok.PNG", None, None, None), // case-insensitive ext
            ],
            0,
        );
        let reply = parse_list2(&bytes, 0);
        assert_eq!(reply.items.len(), 1);
        assert_eq!(unwide(&reply.items[0].path), r"C:\ok.PNG");
    }

    #[test]
    fn list2_hostile_numitems_cannot_spin() {
        // numitems = 0xFFFFFFFF over a one-item buffer: the walk must end
        // at the first header past the data, not iterate 4 billion times.
        let mut bytes = list2(1, &[(0, r"C:\a.png", None, None, None)], 0);
        bytes[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let reply = parse_list2(&bytes, 0);
        assert_eq!(reply.items.len(), 1);
    }

    #[test]
    fn list2_hostile_data_offset_and_lying_length_drop_only_that_item() {
        let mut bytes = list2(
            2,
            &[
                (0, r"C:\a.png", None, None, None),
                (0, r"C:\b.png", None, None, None),
            ],
            0,
        );
        // First item's data_offset → far out of range.
        bytes[20 + 4..20 + 8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        // Second item's length DWORD lies: claims 5 chars for a 5-char path
        // + NUL — in bounds, must parse.
        let reply = parse_list2(&bytes, 0);
        assert_eq!(reply.items.len(), 1);
        assert_eq!(unwide(&reply.items[0].path), r"C:\b.png");
    }

    #[test]
    fn list2_length_at_max_path_boundary() {
        // upstream's `< MAX_PATH` gate (viv.c:3850): 259 passes, 260 drops.
        let path259 = format!("C:\\{}", "y".repeat(252)).to_string() + ".png"; // 3+252+4
        let path260 = format!("C:\\{}", "y".repeat(253)).to_string() + ".png";
        assert_eq!(path259.encode_utf16().count(), 259);
        assert_eq!(path260.encode_utf16().count(), 260);
        let bytes = list2(
            2,
            &[
                (0, &path259, None, None, None),
                (0, &path260, None, None, None),
            ],
            0,
        );
        let reply = parse_list2(&bytes, 0);
        assert_eq!(reply.items.len(), 1);
        assert_eq!(reply.items[0].path, wide(&path259));
    }

    #[test]
    fn filetime_zero_lands_negative_like_pre_epoch() {
        // The i64 cast comes FIRST: a 1601 FILETIME must not wrap in u64.
        assert_eq!(filetime_to_unix_ticks(0), -116_444_736_000_000_000);
        // The unix epoch itself is the shift; 100 ns ticks after it.
        assert_eq!(filetime_to_unix_ticks(116_444_736_000_000_000), 0);
        assert_eq!(filetime_to_unix_ticks(116_444_736_010_000_000), 10_000_000);
    }

    #[test]
    fn msvc_rand_matches_the_crt_sequence() {
        // The MSVC LCG: state = state*214013 + 2531011, output bits
        // 16..30, advanced before reading.
        let mut rng = MsvcRand(1);
        assert_eq!(
            rng.next(),
            ((1u32.wrapping_mul(214013).wrapping_add(2531011)) >> 16) & 0x7fff
        );
        let mut rng0 = MsvcRand(0);
        assert_eq!(rng0.next(), 2531011u32 >> 16);
        // Two draws in a row chain the state.
        let mut chained = MsvcRand(1);
        let a = chained.next();
        let expect_b = {
            let s = 1u32.wrapping_mul(214013).wrapping_add(2531011);
            ((s.wrapping_mul(214013).wrapping_add(2531011)) >> 16) & 0x7fff
        };
        assert_eq!(chained.next(), expect_b);
        assert_ne!(a, expect_b);
    }

    #[test]
    fn random_offset_formula_and_initial_total() {
        // (r1*32767 + r2) % tot — the products stay under i32::MAX so the C
        // arithmetic and this one agree bit for bit.
        assert_eq!(random_offset(0, 0, 100), 0);
        assert_eq!(random_offset(1, 1, 10), (32767 + 1) % 10);
        assert_eq!(
            random_offset(32767, 32767, 0xFFFF_FFFF),
            32767 * 32767 + 32767
        );
        // The initial tot (0xFFFFFFFF) never bounds the draw.
        assert_eq!(random_offset(100, 200, 0xFFFF_FFFF), 100 * 32767 + 200);
        // Unreachable upstream (guarded by totitems > 0); total function.
        assert_eq!(random_offset(5, 5, 0), 0);
    }

    #[test]
    fn reply_payload_ids_keep_the_upstream_enum_order() {
        // viv.c:293-297: COMMAND_LINE(0, copydata.rs), OPEN, ADD, RANDOM.
        assert_eq!(crate::copydata::COPYDATA_COMMAND_LINE, 0);
        assert_eq!(COPYDATA_OPEN_EVERYTHING_SEARCH, 1);
        assert_eq!(COPYDATA_ADD_EVERYTHING_SEARCH, 2);
        assert_eq!(COPYDATA_RANDOM_EVERYTHING_SEARCH, 3);
    }
}
