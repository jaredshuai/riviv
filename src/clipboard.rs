//! The clipboard command family (#41) — Cut/Copy (`_viv_copy`,
//! viv.c:7376-7447), Copy Filename (`_viv_copy_filename`, 7449-7480), Copy
//! Image (`_viv_copy_image` + `_viv_set_clipboard_image`,
//! 7537-7552/7485-7530) and Paste (`WM_PASTE`, 4021-4047, reached through
//! the EditPaste command, viv.c:2347-2349) — plus the READ side (#66,
//! upstream wishlist viv.c:80/105): the `clipboard:` pseudo-filename's
//! DIB-family reader feeding the virtual display. The wire formats are
//! pure and unit-tested; the Win32 clipboard session is the thin shell
//! half.
//!
//! Upstream routes the paste's HDROP through the real WM_DROPFILES
//! handler (`SendMessage(hwnd, WM_DROPFILES, hdrop, 0)`, viv.c:4038).
//! riviv's drop handler frees real shell drops with `DragFinish` (a leak
//! upstream lacks), which would GlobalFree the system-owned clipboard
//! block — so the paste path calls the shared drop-application body
//! directly instead. The observable behavior (including the
//! post-drop foreground raise, viv.c:3126) is the same.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW,
    HBITMAP, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{CF_BITMAP, CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::HDROP;
use windows::core::w;

use crate::surface::create_bgra_dib;
use crate::window::{apply_drop_files, request_open_clipboard, state_of};

/// The `clipboard:` display name (#66; upstream wishlist viv.c:80 — "open
/// a file with the filename clipboard: to open the clipboard"): the
/// literal pseudo-name the virtual display carries (window title,
/// verdicts), the clipboard family's counterpart of
/// `loadthread::STDIN_NAME`. NTFS reserves ':' the same way, so no real
/// file can collide.
pub(crate) const CLIPBOARD_NAME: &str = "clipboard:";

/// The DROPFILES header is 20 bytes on every architecture (DWORD pFiles +
/// POINT pt + two DWORDs — no tail padding); upstream sets
/// `df->pFiles = sizeof(DROPFILES)` (viv.c:7397).
const DROPFILES_LEN: usize = 20;

/// The CF_HDROP payload for a file list (upstream builds one file,
/// viv.c:7389-7417): a DROPFILES header with `fWide = 1`, then every path
/// as wide chars NUL-terminated, then the empty-string list terminator —
/// the double NUL the shell format requires.
pub(crate) fn hdrop_bytes(paths: &[&OsStr]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        DROPFILES_LEN + paths.iter().map(|p| 2 * (p.len() + 1)).sum::<usize>() + 2,
    );
    out.extend_from_slice(&(DROPFILES_LEN as u32).to_le_bytes()); // pFiles
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.x (upstream zeroes)
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    out.extend_from_slice(&0u32.to_le_bytes()); // fNC
    out.extend_from_slice(&1u32.to_le_bytes()); // fWide — wide names
    for path in paths {
        for unit in path.encode_wide() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]); // this path's NUL
    }
    out.extend_from_slice(&[0, 0]); // the list terminator
    out
}

/// The `Preferred DropEffect` DWORD payload (upstream viv.c:7421-7440):
/// Cut asks for a MOVE, Copy for COPY|LINK — the value Explorers consume
/// when pasting a dropped file elsewhere.
pub(crate) fn drop_effect_bytes(cut: bool) -> [u8; 4] {
    // DROPEFFECT_MOVE = 2; DROPEFFECT_COPY|DROPEFFECT_LINK = 1|4 (viv.c:7427).
    (if cut { 2u32 } else { 5u32 }).to_le_bytes()
}

/// The CF_UNICODETEXT payload for Copy Filename (upstream viv.c:7453-7475):
/// the full path as wide chars with a single NUL.
pub(crate) fn unicode_text_bytes(path: &OsStr) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 * (path.len() + 1));
    for unit in path.encode_wide() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Whether the image-dependent clipboard commands are usable (upstream's
/// `is_image_enabled` for the EnableMenuItem family, viv.c:7103): a
/// current file exists AND neither a not-found nor a failed verdict
/// stands. Only the MENU graying uses the two verdict flags — the
/// keyboard handlers gate on the bare current-file check (viv.c:7377).
pub(crate) fn image_gate(has_current: bool, not_found: bool, load_failed: bool) -> bool {
    has_current && !not_found && !load_failed
}

/// The registered `Preferred DropEffect` format id (upstream caches it in
/// `_viv_CF_PREFERREDDROPEFFECT`, viv.c:7584-7592 — same value for the
/// process's lifetime). 0 means registration failed; the effect payload
/// is then skipped (fail-soft like upstream's unchecked SetClipboardData).
fn preferred_drop_effect() -> u32 {
    static EFFECT_FORMAT: OnceLock<u32> = OnceLock::new();
    *EFFECT_FORMAT.get_or_init(|| {
        // SAFETY: RegisterClipboardFormatW only reads the literal name.
        unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) }
    })
}

/// Edit → Cut / Copy (upstream `_viv_copy`, viv.c:7376-7447): the current
/// file onto the clipboard as CF_BITMAP (the displayed frame's pixels)
/// plus CF_HDROP plus `Preferred DropEffect`. Gated on the bare
/// current-file check — a failed load leaves the name current upstream
/// (the FAILED handler never touches current_fd, viv.c:2832-2840), so the
/// keyboard path still copies it; the menu gray is display-only.
pub(crate) fn copy_current(hwnd: HWND, cut: bool) {
    // SAFETY: read-only state reads; the frame's master bytes are CLONED
    // out so the borrow ends before the clipboard session (which must
    // not hold one).
    let (path, frame) = match unsafe { state_of(hwnd) } {
        Some(state) => match state.nav_current.clone() {
            Some(entry) => (
                entry.path,
                state.image.as_ref().map(|i| {
                    let master = i.surface().master();
                    (
                        master.pixels.to_vec(),
                        master.width as i32,
                        master.height as i32,
                    )
                }),
            ),
            None => return,
        },
        None => return,
    };
    // SAFETY: the clipboard session runs on the owning UI thread; hwnd is
    // live. OpenClipboard/EmptyClipboard/SetClipboardData failures are
    // fail-soft like upstream (no checks at viv.c:7377-7445 — a contended
    // clipboard simply skips the copy).
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        // CF_BITMAP first (upstream 7380): Copy/Cut also carry the pixels.
        if let Some((pixels, w, h)) = frame {
            set_clipboard_image(&pixels, w, h);
        }
        set_hglobal(CF_HDROP.0 as u32, &hdrop_bytes(&[path.as_os_str()]));
        let fmt = preferred_drop_effect();
        if fmt != 0 {
            set_hglobal(fmt, &drop_effect_bytes(cut));
        }
        let _ = CloseClipboard();
    }
}

/// Edit → Copy Filename (upstream `_viv_copy_filename`, viv.c:7449-7480):
/// the full path as CF_UNICODETEXT.
pub(crate) fn copy_filename(hwnd: HWND) {
    let path = {
        // SAFETY: read-only state read, cloned out before the session.
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        match state.nav_current.clone() {
            Some(entry) => entry.path,
            None => return, // the bare current-file gate, viv.c:7450
        }
    };
    // SAFETY: the clipboard session, as in copy_current.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        set_hglobal(
            CF_UNICODETEXT.0 as u32,
            &unicode_text_bytes(path.as_os_str()),
        );
        let _ = CloseClipboard();
    }
}

/// Edit → Copy Image (upstream `_viv_copy_image`, viv.c:7537-7552): the
/// displayed frame's pixels alone, as CF_BITMAP.
pub(crate) fn copy_image(hwnd: HWND) {
    let frame = {
        // SAFETY: read-only state read, values copied out (HDC is Copy).
        let Some(state) = (unsafe { state_of(hwnd) }) else {
            return;
        };
        // The bare current-file gate (viv.c:7538) plus the frame check
        // `_viv_set_clipboard_image` makes (viv.c:7487) — no display, no
        // blit. #65 widens the gate to the `stdin:` VIRTUAL display:
        // frames on screen with no backing file still blit (the keyboard
        // mirror of the menu's display-kind split; a failed load over a
        // kept display still has its current file — upstream's keyboard
        // quirk unchanged).
        if state.nav_current.is_none() && !state.virtual_display {
            return;
        }
        state.image.as_ref().map(|i| {
            let master = i.surface().master();
            (
                master.pixels.to_vec(),
                master.width as i32,
                master.height as i32,
            )
        })
    };
    let Some((pixels, w, h)) = frame else {
        return;
    };
    // SAFETY: the clipboard session, as in copy_current.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        set_clipboard_image(&pixels, w, h);
        let _ = CloseClipboard();
    }
}

/// The blit half of upstream `_viv_set_clipboard_image` (viv.c:7485-7530):
/// copy the displayed frame's pixels into a fresh SCREEN-compatible
/// bitmap and hand it to the clipboard. The frame's master is the
/// source (#76): a throwaway DIB derived from these bytes feeds the
/// blit — the same 1:1 SRCCOPY from the same pixel values the display's
/// own GDI face would have served, so the DDB out is byte-identical.
/// The master itself is never surrendered — the clipboard takes the
/// copy, the display keeps the original. The clipboard session must
/// already be open (upstream calls this both from _viv_copy, which
/// opened it, and from _viv_copy_image).
///
/// SAFETY (callers): `pixels` holds exactly `wide * high * 4` top-down
/// BGRA bytes (the master); the clipboard is open.
unsafe fn set_clipboard_image(pixels: &[u8], wide: i32, high: i32) {
    // SAFETY: blanket for the edition-2024 block — every call below is the
    // GDI primitive its comment names; the per-path teardown the comments
    // describe is what the body itself encodes.
    unsafe {
        // SAFETY: GetDC(None) is the screen DC; released on every path below.
        let screen = GetDC(None);
        if screen.is_invalid() {
            return;
        }
        // SAFETY: the throwaway source face — a DIB memcpy of the master,
        // selected into its own temp DC for the blit (upstream blits the
        // frame's mem DC, same bytes; deleted on every path below).
        let Ok(dib) = create_bgra_dib(wide, high, pixels) else {
            let _ = ReleaseDC(None, screen);
            return;
        };
        // SAFETY: plain DC creation/teardown; DeleteDC on every path.
        let src = CreateCompatibleDC(Some(screen));
        if src.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(dib.0));
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: `dib` is valid and unselected; the old object is restored
        // before the DC is deleted. A failed selection would leave the DC's
        // 1x1 stock bitmap as the blit source — bail with the symmetric
        // teardown instead of copying garbage to the clipboard (review
        // PR #84 F6).
        let src_old = SelectObject(src, HGDIOBJ(dib.0));
        if src_old.is_invalid() {
            let _ = DeleteDC(src);
            let _ = DeleteObject(HGDIOBJ(dib.0));
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: plain DC creation/teardown; DeleteDC on every path.
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            let _ = SelectObject(src, src_old);
            let _ = DeleteDC(src);
            let _ = DeleteObject(HGDIOBJ(dib.0));
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: CreateCompatibleBitmap against the screen DC gives the DDB
        // format clipboard consumers expect (upstream viv.c:7500-7502).
        let bitmap = CreateCompatibleBitmap(screen, wide, high);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem);
            let _ = SelectObject(src, src_old);
            let _ = DeleteDC(src);
            let _ = DeleteObject(HGDIOBJ(dib.0));
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: `bitmap` is valid and unselected; the old object is
        // restored before the DC is deleted. Same failed-selection guard
        // as above: blitting from the stock bitmap would hand the
        // clipboard a 1x1 garbage DDB (review PR #84 F6).
        let old = SelectObject(mem, HGDIOBJ(bitmap.0));
        if old.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem);
            let _ = SelectObject(src, src_old);
            let _ = DeleteDC(src);
            let _ = DeleteObject(HGDIOBJ(dib.0));
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: both DCs are live; src holds exactly a wide x high frame.
        let _ = BitBlt(mem, 0, 0, wide, high, Some(src), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        // The clipboard takes ownership on success — the bitmap is NOT
        // deleted (upstream's comment at viv.c:7519: "the system now owns the
        // handle"). On failure upstream leaks it; riviv frees (invisible to
        // behavior, no orphaned GDI object under memory pressure).
        match SetClipboardData(CF_BITMAP.0 as u32, Some(HANDLE(bitmap.0 as *mut _))) {
            Ok(_) => {}
            Err(_) => {
                // SAFETY: the bitmap never reached the clipboard and stays ours.
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
        }
        // SAFETY: the bitmap is deselected; the DCs are ours to delete; the
        // temp DIB is deselected before its deletion.
        let _ = DeleteDC(mem);
        let _ = SelectObject(src, src_old);
        let _ = DeleteDC(src);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = ReleaseDC(None, screen);
    }
}

/// One `GlobalAlloc(GMEM_MOVEABLE)` + copy + `SetClipboardData` (upstream
/// builds the same shape at viv.c:7389/7416 and 7453/7470). The system
/// owns the block on success; on failure upstream leaks it and riviv
/// frees (as with the CF_BITMAP above).
///
/// SAFETY (callers): the clipboard is open; `bytes` is a final payload.
unsafe fn set_hglobal(format: u32, bytes: &[u8]) {
    // SAFETY: blanket for the edition-2024 block — hmem is a private
    // GMEM_MOVEABLE allocation of exactly `bytes.len()` bytes, locked and
    // unlocked here; ownership moves to the system only through the
    // SetClipboardData success arm.
    unsafe {
        // Upstream's `if (hmem)` — an allocation failure silently skips the
        // format (viv.c:7390), leaving the earlier formats in place.
        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
            return;
        };
        // SAFETY: hmem is ours, `bytes.len()` bytes were allocated.
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            // Publishing the never-written block would put uninitialized
            // bytes on the clipboard — skip the format instead (as with
            // the alloc failure above).
            // SAFETY: the block is still ours and never reached the clipboard.
            let _ = GlobalFree(Some(hmem));
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
        let _ = GlobalUnlock(hmem);
        match SetClipboardData(format, Some(HANDLE(hmem.0 as *mut _))) {
            Ok(_) => {}
            Err(_) => {
                // SAFETY: the block never reached the clipboard and stays ours.
                let _ = GlobalFree(Some(hmem));
            }
        }
    }
}

/// Whether a locked CF_HDROP payload is structurally sound enough to hand
/// to the shell's `DragQueryFile` parser: the DROPFILES header fits,
/// `pFiles` lands inside the block, and the string list reaches an empty
/// string (two consecutive NUL units) before the block ends — the bounds
/// `DragQueryFile` cannot check itself (an HDROP is a bare pointer, not a
/// sized allocation; a lone NUL only ends one string, the walk then reads
/// on). Upstream trusts the clipboard blindly (viv.c:4033-4038 passes the
/// lock straight through); riviv validates because a foreign process
/// authored this memory. A malformed payload no-ops the paste exactly
/// like every other unrecognized clipboard shape.
pub(crate) fn hdrop_payload_is_sound(payload: &[u8]) -> bool {
    if payload.len() < DROPFILES_LEN {
        return false;
    }
    let p_files = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if p_files > payload.len() {
        return false;
    }
    let body = &payload[p_files..];
    // fWide is a BOOL: nonzero = UTF-16 units, zero = ANSI bytes.
    let wide = u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]) != 0;
    let unit = if wide { 2 } else { 1 };
    // The list ends at an empty string: a NUL unit at a string start —
    // position 0, or right after a string's own terminating NUL. Treating
    // the virtual unit before the body as a terminator models position 0.
    let mut prev_nul = true;
    for c in body.chunks_exact(unit) {
        let nul = c.iter().all(|&b| b == 0);
        if nul && prev_nul {
            return true;
        }
        prev_nul = nul;
    }
    false
}

/// WM_PASTE (upstream viv.c:4021-4047, reached through the EditPaste
/// command, viv.c:2347-2349), widened by #66: a CF_HDROP clipboard stays
/// the #41 file drop with priority; with no HDROP, the DIB family
/// (CF_DIBV5 / CF_DIB / CF_BITMAP, upstream wishlist viv.c:105) opens
/// the `clipboard:` virtual display through the load worker. Every other
/// clipboard shape (text, empty) remains the silent no-op upstream has.
pub(crate) fn on_paste(hwnd: HWND) {
    // The image request is deferred until AFTER CloseClipboard: the load
    // worker re-opens the clipboard for its read, and a request issued
    // under OUR open session would arrive to a busy clipboard.
    let mut paste_image = false;
    // SAFETY: the clipboard session runs on the owning UI thread. The
    // GetClipboardData block is system-owned: locked only for the drop
    // application, never DragFinish'd, unlocked before CloseClipboard.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        if let Ok(handle) = GetClipboardData(CF_HDROP.0 as u32) {
            let hmem = HGLOBAL(handle.0);
            // SAFETY: a clipboard HGLOBAL is lockable for the session's
            // duration (upstream GlobalLocks it at viv.c:4033).
            let ptr = GlobalLock(hmem);
            if !ptr.is_null() {
                // SAFETY: GlobalSize reports the block's full allocation;
                // the lock makes exactly that many bytes readable at ptr,
                // and the slice borrows only locked clipboard memory for
                // the validation read below.
                let size = GlobalSize(hmem);
                let payload = std::ptr::slice_from_raw_parts(ptr.cast::<u8>(), size);
                if hdrop_payload_is_sound(&*payload) {
                    // The drop body runs inline (upstream SendMessage's to
                    // itself — same thread, same order), WITHOUT the
                    // DragFinish real drops get.
                    apply_drop_files(hwnd, HDROP(ptr));
                }
                // SAFETY: paired with the lock above.
                let _ = GlobalUnlock(hmem);
            }
        } else if dib_family_available() {
            // No HDROP — the bitmap family takes the paste (#66).
            paste_image = true;
        }
        let _ = CloseClipboard();
    }
    if paste_image {
        request_open_clipboard(hwnd);
    }
}

/// Whether the clipboard offers any of the DIB family formats (#66) —
/// the paste path's cheap probe (IsClipboardFormatAvailable needs no
/// open session) deciding between the #41 file path (HDROP) and the
/// `clipboard:` virtual display. Text and other shapes answer false and
/// paste stays a no-op.
pub(crate) fn dib_family_available() -> bool {
    // SAFETY: pure format-presence queries; no session, no handles.
    [CF_DIBV5.0 as u32, CF_DIB.0 as u32, CF_BITMAP.0 as u32]
        .iter()
        .any(|&f| unsafe { IsClipboardFormatAvailable(f) }.is_ok())
}

/// Read the clipboard's IMAGE as one DIB payload (#66; the `clipboard:`
/// pseudo-filename's source): CF_DIBV5, then CF_DIB (raw GlobalLock'd
/// copies), then CF_BITMAP — converted through GetDIBits into the same
/// 32bpp BI_RGB top-down shape, so the pure `dib` parser handles every
/// path uniformly (and any-depth DDBs display even though the first two
/// formats reject them). First format OFFERED wins; a payload that then
/// fails to parse fails the load honestly — no per-format fallback that
/// would mask what a producer actually wrote. The session is short
/// (open, copy, close — nothing parsed under the lock). `Ok(None)` = no
/// image format on the clipboard; `Err` = the clipboard would not open
/// (busy). Both are user-level failures upstream of here.
pub(crate) fn read_clipboard_dib() -> Result<Option<Vec<u8>>, String> {
    // SAFETY: the caller owns the threading contract (the detached reader
    // below). OpenClipboard(None) associates the session with the calling
    // task; every handle below is system-owned clipboard memory locked
    // only for the copy; CloseClipboard runs on every path.
    unsafe {
        if OpenClipboard(None).is_err() {
            return Err("the clipboard is busy".into());
        }
        let out = read_clipboard_dib_locked();
        let _ = CloseClipboard();
        out
    }
}

/// The `clipboard:` read, terminate-aware (#66): the blocking session
/// runs on a DETACHED helper thread — the same contract as stdin's
/// reader (loadthread.rs). A clipboard owner using DELAYED RENDERING can
/// stall GetClipboardData indefinitely (the system waits for its
/// WM_RENDERFORMAT reply while the owner hangs), and a stalled decode
/// worker would hang the window-teardown join ("it's critical we wait
/// for load image to finish", viv.c:5476). The worker polls the channel
/// and abandons the reader at the terminate flag; the abandoned reader
/// holds only its buffers until the owner finally answers (or process
/// exit reclaims the session — nothing else is shared). `None` =
/// terminated mid-read: exit silently, like a file decode between
/// frames.
pub(crate) fn read_clipboard_dib_terminated(
    terminate: &AtomicBool,
) -> Option<Result<Option<Vec<u8>>, String>> {
    let (sender, receiver) = channel::<Result<Option<Vec<u8>>, String>>();
    // Builder::spawn, not thread::spawn (the stdin lesson, #65): an OS
    // thread-creation failure is THIS load's user-level failure, not a
    // panic that kills the decode worker.
    let reader = std::thread::Builder::new()
        .name("riviv-clipboard".into())
        .spawn(move || {
            // SAFETY: the reader thread owns the whole session, as above.
            let outcome = read_clipboard_dib();
            // The sender drops silently if nobody waits (terminated job).
            let _ = sender.send(outcome);
        });
    if let Err(e) = reader {
        return Some(Err(format!("reader thread spawn failed: {e}")));
    }
    // On success the JoinHandle is deliberately dropped: DETACHED, for
    // exactly the stall the helper exists to survive.
    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(outcome) => return Some(outcome),
            Err(RecvTimeoutError::Timeout) => {
                if terminate.load(Ordering::Relaxed) {
                    return None; // abandon the detached reader
                }
            }
            // Unreachable (the reader always sends or dies with the
            // process); treat as a failed read so the wait can never spin.
            Err(RecvTimeoutError::Disconnected) => {
                return Some(Err("the reader vanished".into()));
            }
        }
    }
}

/// The open-session body of `read_clipboard_dib` (the probe ladder).
///
/// SAFETY (callers): the clipboard is open.
unsafe fn read_clipboard_dib_locked() -> Result<Option<Vec<u8>>, String> {
    // SAFETY: blanket for the edition-2024 block — the clipboard is open
    // (the caller's contract); every block below is system-owned
    // clipboard memory locked only for the stated copy.
    unsafe {
        for format in [CF_DIBV5.0 as u32, CF_DIB.0 as u32] {
            let Ok(handle) = GetClipboardData(format) else {
                continue; // not offered — next format
            };
            let hmem = HGLOBAL(handle.0);
            // SAFETY: a clipboard HGLOBAL is lockable for the session's
            // duration (the same contract on_paste relies on).
            let ptr = GlobalLock(hmem);
            if ptr.is_null() {
                continue;
            }
            // SAFETY: GlobalSize reports the block's full allocation; the
            // lock makes exactly that many bytes readable at ptr, and the
            // slice borrows only locked clipboard memory for the copy.
            let size = GlobalSize(hmem);
            let bytes = std::ptr::slice_from_raw_parts(ptr.cast::<u8>(), size);
            let copied = (*bytes).to_vec();
            // SAFETY: paired with the lock above.
            let _ = GlobalUnlock(hmem);
            return Ok(Some(copied));
        }
        // CF_BITMAP: the DDB the system synthesizes for older producers.
        if let Ok(handle) = GetClipboardData(CF_BITMAP.0 as u32)
            && let Some(dib) = bitmap_to_dib(HBITMAP(handle.0))
        {
            return Ok(Some(dib));
        }
        Ok(None)
    }
}

/// One CF_BITMAP → DIB payload conversion, run while the clipboard is
/// open (the HBITMAP is only valid for the session): GetObject for the
/// extent, then GetDIBits into a fresh 32bpp BI_RGB top-down buffer —
/// the exact shape `dib::parse_dib` handles. `None` on any GDI step
/// failure (the format then reads as absent, a user-level no-image).
///
/// SAFETY (callers): `bitmap` is the clipboard's live CF_BITMAP handle —
/// valid while the clipboard is open, selected into no DC, and never to
/// be deleted by us (the system owns it).
unsafe fn bitmap_to_dib(bitmap: HBITMAP) -> Option<Vec<u8>> {
    // SAFETY: blanket for the edition-2024 block — `bitmap` is the
    // clipboard's live CF_BITMAP (open session, selected nowhere, never
    // deleted by us); the GDI handles created here are torn down on
    // every path their comments describe.
    unsafe {
        let mut bm = BITMAP::default();
        // SAFETY: `bm` outlives the call and GetObjectW only writes it.
        if GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some((&mut bm as *mut BITMAP).cast()),
        ) == 0
        {
            return None;
        }
        if bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        // SAFETY: the screen DC is released on every path below.
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        // SAFETY: plain DC creation; deleted on every path below.
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            let _ = ReleaseDC(None, screen);
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down — parse_dib's output convention
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // The byte counts are usize math from the start — an extreme
        // bitmap's w*h would overflow i32 long before the allocation.
        let mut rows = vec![0u8; w as usize * h as usize * 4];
        // SAFETY: `rows` holds exactly w*h*4 writable bytes; `info` is a
        // valid BITMAPINFO for the request; the bitmap is valid and
        // selected nowhere. The return is the number of scan lines
        // copied — `h` means the whole bitmap came through.
        let copied = GetDIBits(
            mem,
            bitmap,
            0,
            h as u32,
            Some(rows.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        // SAFETY: the DC is ours to delete; the screen DC is released.
        let _ = DeleteDC(mem);
        let _ = ReleaseDC(None, screen);
        if copied != h {
            return None;
        }
        // Serialize header + rows as one CF_DIB-shaped payload.
        // SAFETY: reading the repr(C) header as its 40 raw bytes —
        // BITMAPINFOHEADER is exactly 40 bytes of plain fields.
        let header = std::slice::from_raw_parts(
            (&info.bmiHeader as *const BITMAPINFOHEADER).cast::<u8>(),
            size_of::<BITMAPINFOHEADER>(),
        );
        let mut payload = Vec::with_capacity(header.len() + rows.len());
        payload.extend_from_slice(header);
        payload.extend_from_slice(&rows);
        Some(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    // ---- CF_HDROP payload (upstream viv.c:7389-7417) ----

    #[test]
    fn hdrop_header_is_a_wide_dropfiles_at_offset_20() {
        let bytes = hdrop_bytes(&[OsStr::new(r"C:\pics\a.png")]);
        assert_eq!(&bytes[0..4], &20u32.to_le_bytes()); // pFiles
        assert_eq!(&bytes[4..12], &[0u8; 8]); // pt zeroed
        assert_eq!(&bytes[12..16], &0u32.to_le_bytes()); // fNC
        assert_eq!(&bytes[16..20], &1u32.to_le_bytes()); // fWide
        let name: Vec<u8> = OsStr::new(r"C:\pics\a.png")
            .encode_wide()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(&bytes[20..20 + name.len()], &name[..]);
        // The path's NUL plus the list terminator = the double NUL.
        assert_eq!(&bytes[20 + name.len()..], &[0, 0, 0, 0]);
    }

    #[test]
    fn hdrop_lists_are_separated_and_terminated_by_nuls() {
        let bytes = hdrop_bytes(&[OsStr::new("a"), OsStr::new("b")]);
        // One wide char + its NUL = 4 bytes per single-char entry.
        let a: Vec<u8> = OsStr::new("a")
            .encode_wide()
            .flat_map(|u| u.to_le_bytes())
            .chain([0, 0])
            .collect();
        let b: Vec<u8> = OsStr::new("b")
            .encode_wide()
            .flat_map(|u| u.to_le_bytes())
            .chain([0, 0])
            .collect();
        assert_eq!(&bytes[20..24], &a[..]);
        assert_eq!(&bytes[24..28], &b[..]);
        assert_eq!(&bytes[28..], &[0, 0]);
    }

    // ---- the foreign-payload soundness gate (paste hardening, #55) ----

    #[test]
    fn a_well_formed_wide_hdrop_is_sound() {
        // Round-trip: what hdrop_bytes builds must always pass.
        let bytes = hdrop_bytes(&[OsStr::new(r"C:\pics\a.png")]);
        assert!(hdrop_payload_is_sound(&bytes));
        // A count-0 list (header + bare terminator) is bounded too.
        let empty_list = hdrop_bytes(&[]);
        assert_eq!(empty_list.len(), DROPFILES_LEN + 2);
        assert!(hdrop_payload_is_sound(&empty_list));
    }

    #[test]
    fn a_truncated_or_misoffset_header_is_unsound() {
        // Header cut short.
        assert!(!hdrop_payload_is_sound(&[0u8; 19]));
        // pFiles points past the block's end.
        let mut bytes = hdrop_bytes(&[OsStr::new("a")]);
        bytes[0..4].copy_from_slice(&100u32.to_le_bytes());
        assert!(!hdrop_payload_is_sound(&bytes));
        // pFiles lands exactly at the end — an empty body cannot hold a
        // terminator, so the shell walk would run off the block.
        let empty_list = hdrop_bytes(&[]);
        assert!(!hdrop_payload_is_sound(&empty_list[..DROPFILES_LEN]));
    }

    #[test]
    fn a_list_without_an_empty_string_is_unsound() {
        // A wide path with its own NUL but no list terminator: a lone NUL
        // only ends the string — the walk reads on, past the block.
        let mut unterminated = Vec::new();
        unterminated.extend_from_slice(&hdrop_bytes(&[OsStr::new("a")]));
        unterminated.truncate(unterminated.len() - 2);
        assert!(!hdrop_payload_is_sound(&unterminated));
        // Same shape in ANSI (fWide = 0).
        let mut ansi = Vec::new();
        ansi.extend_from_slice(&(DROPFILES_LEN as u32).to_le_bytes());
        ansi.extend_from_slice(&0i32.to_le_bytes());
        ansi.extend_from_slice(&0i32.to_le_bytes());
        ansi.extend_from_slice(&0u32.to_le_bytes()); // fNC
        ansi.extend_from_slice(&0u32.to_le_bytes()); // fWide
        ansi.extend_from_slice(b"a\0");
        assert!(!hdrop_payload_is_sound(&ansi));
        ansi.push(0); // the empty string arrives — now bounded.
        assert!(hdrop_payload_is_sound(&ansi));
    }

    // ---- Preferred DropEffect payload (upstream viv.c:7421-7440) ----

    #[test]
    fn cut_asks_for_a_move_and_copy_for_copy_or_link() {
        assert_eq!(drop_effect_bytes(true), 2u32.to_le_bytes()); // DROPEFFECT_MOVE
        assert_eq!(drop_effect_bytes(false), 5u32.to_le_bytes()); // COPY|LINK
    }

    // ---- CF_UNICODETEXT payload (upstream viv.c:7453-7475) ----

    #[test]
    fn filename_text_is_the_full_path_with_a_single_nul() {
        let bytes = unicode_text_bytes(OsStr::new("ab"));
        assert_eq!(bytes, [0x61, 0, 0x62, 0, 0, 0]);
    }

    // ---- the menu-enable gate (upstream viv.c:7103) ----

    #[test]
    fn the_image_gate_needs_a_current_file_and_no_verdict() {
        assert!(image_gate(true, false, false));
        // A failed or vanished current file grays the clipboard quartet.
        assert!(!image_gate(true, true, false));
        assert!(!image_gate(true, false, true));
        // A blank viewer has nothing to copy even with no verdict.
        assert!(!image_gate(false, false, false));
    }
}
