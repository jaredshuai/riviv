//! The clipboard command family (#41) — Cut/Copy (`_viv_copy`,
//! viv.c:7376-7447), Copy Filename (`_viv_copy_filename`, 7449-7480), Copy
//! Image (`_viv_copy_image` + `_viv_set_clipboard_image`,
//! 7537-7552/7485-7530) and Paste (`WM_PASTE`, 4021-4047, reached through
//! the EditPaste command, viv.c:2347-2349). The wire formats are pure and
//! unit-tested; the Win32 clipboard session is the thin shell half.
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

use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, HDC,
    HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{CF_BITMAP, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::HDROP;
use windows::core::w;

use crate::window::{apply_drop_files, state_of};

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
    // SAFETY: read-only state reads, all values copied out before the
    // clipboard session (which pumps nothing but must not hold a borrow).
    let (path, frame) = match unsafe { state_of(hwnd) } {
        Some(state) => match state.nav_current.clone() {
            Some(entry) => (
                entry.path,
                state.image.as_ref().map(|i| {
                    (
                        i.surface().mem_dc(),
                        i.surface().width(),
                        i.surface().height(),
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
        if let Some((src, w, h)) = frame {
            set_clipboard_image(src, w, h);
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
        // blit.
        match state.nav_current.clone() {
            Some(_) => state.image.as_ref().map(|i| {
                (
                    i.surface().mem_dc(),
                    i.surface().width(),
                    i.surface().height(),
                )
            }),
            None => return,
        }
    };
    let Some((src, w, h)) = frame else {
        return;
    };
    // SAFETY: the clipboard session, as in copy_current.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        set_clipboard_image(src, w, h);
        let _ = CloseClipboard();
    }
}

/// The blit half of upstream `_viv_set_clipboard_image` (viv.c:7485-7530):
/// copy the displayed frame's pixels into a fresh SCREEN-compatible
/// bitmap and hand it to the clipboard. The frame's own DIB is never
/// surrendered — the clipboard takes the copy, the display keeps the
/// original. The clipboard session must already be open (upstream calls
/// this both from _viv_copy, which opened it, and from _viv_copy_image).
///
/// SAFETY (callers): `src` is a live memory DC with the frame DIB
/// selected, owned by the display's Surface; the clipboard is open.
unsafe fn set_clipboard_image(src: HDC, wide: i32, high: i32) {
    // SAFETY: blanket for the edition-2024 block — every call below is the
    // GDI primitive its comment names; the per-path teardown the comments
    // describe is what the body itself encodes.
    unsafe {
        // SAFETY: GetDC(None) is the screen DC; released on every path below.
        let screen = GetDC(None);
        if screen.is_invalid() {
            return;
        }
        // SAFETY: plain DC creation/teardown; DeleteDC on every path.
        let mem = CreateCompatibleDC(Some(screen));
        if mem.is_invalid() {
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: CreateCompatibleBitmap against the screen DC gives the DDB
        // format clipboard consumers expect (upstream viv.c:7500-7502).
        let bitmap = CreateCompatibleBitmap(screen, wide, high);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem);
            let _ = ReleaseDC(None, screen);
            return;
        }
        // SAFETY: `bitmap` is valid and unselected; the old object is
        // restored before the DC is deleted.
        let old = SelectObject(mem, HGDIOBJ(bitmap.0));
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
        // SAFETY: the bitmap is deselected; the DC is ours to delete.
        let _ = DeleteDC(mem);
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

/// WM_PASTE (upstream viv.c:4021-4047), reached through the EditPaste
/// command (viv.c:2347-2349): a CF_HDROP clipboard becomes a file drop on
/// the viewer; every other clipboard shape (text, bare images) is a
/// silent no-op — the CF_DIB paste of the upstream wish list is not
/// implemented (issue #41 scope).
pub(crate) fn on_paste(hwnd: HWND) {
    // SAFETY: the clipboard session runs on the owning UI thread. The
    // GetClipboardData block is system-owned: locked only for the drop
    // application, never DragFinish'd, unlocked before CloseClipboard.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        if let Ok(handle) = GetClipboardData(CF_HDROP.0 as u32) {
            let hmem = windows::Win32::Foundation::HGLOBAL(handle.0);
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
        }
        let _ = CloseClipboard();
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
