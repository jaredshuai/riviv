//! #43 file-management actions: the rename-dialog composition (pure) and
//! the SHFileOperation shells (delete / rename / copy-to / move-to).
//!
//! Upstream anchors: `_viv_delete` viv.c:7200-7229 (FO_DELETE, recycle vs
//! permanent, playlist+nav follow-up in the caller), `_viv_rename_proc`
//! viv.c:7232-7360 (the OK arm's compose-then-FO_RENAME and the collision
//! mapping read), the Copy To / Move To handler viv.c:2496-2568
//! (GetSaveFileName + FO_COPY/FO_MOVE, return ignored). The pure helpers
//! here are byte-for-byte ports of the string.c primitives that dialog
//! leans on; the shells stay thin so every decision is testable without
//! the file system.

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::UI::Shell::{
    FO_COPY, FO_DELETE, FO_MOVE, FO_RENAME, FOF_ALLOWUNDO, FOF_WANTMAPPINGHANDLE, SHFILEOPSTRUCTW,
    SHFileOperationW, SHFreeNameMappings, SHNAMEMAPPINGW,
};
use windows::core::PCWSTR;

use crate::text::to_wide_os;
use std::ffi::OsStr;

// ---------- pure: the rename OK arm's composition ----------

/// Upstream `string_get_extension` (string.c:768-787): everything after
/// the LAST '.' anywhere in the string, "" when there is none. Scanning
/// the whole path (not just the file name) is upstream's own quirk: a dot
/// inside a directory name (`C:\my.dir\a`) leaks into the "extension"
/// (`dir\a`), and the rename composer below will happily append
/// `.dir\a` to the typed name. Kept verbatim.
fn extension_after_last_dot(s: &[u16]) -> &[u16] {
    let mut last: Option<usize> = None;
    for (i, &ch) in s.iter().enumerate() {
        if ch == u16::from(b'.') {
            last = Some(i + 1);
        }
    }
    let from = last.unwrap_or(s.len());
    &s[from..]
}

/// Upstream `string_get_path_part` (string.c:693-710): everything before
/// the last '\\' — `C:\dir\a.png` keeps `C:\dir`, a root-level file keeps
/// the bare `C:` (the same cut #42 recorded as a `Path::parent`
/// deviation upstream has here).
fn path_before_last_sep(s: &[u16]) -> &[u16] {
    let mut last: Option<usize> = None;
    for (i, &ch) in s.iter().enumerate() {
        if ch == u16::from(b'\\') {
            last = Some(i);
        }
    }
    let to = last.unwrap_or(s.len());
    &s[..to]
}

/// The dialog's path join — upstream routes through `PathCombine`
/// (string.c:688-691). This stand-in covers the dialog's real inputs (a
/// directory plus a bare typed name): one separator between them, the
/// name alone when the directory is empty, the directory alone when the
/// name is. Absolute typed paths (`C:\x`, `\\srv`) are NOT resolved like
/// PathCombine would — FO_RENAME cannot move a file across directories
/// anyway, so such a rename fails at the shell either way.
fn join(dir: &[u16], name: &[u16]) -> Vec<u16> {
    if dir.is_empty() {
        return name.to_vec();
    }
    if name.is_empty() {
        return dir.to_vec();
    }
    let mut out = dir.to_vec();
    if *dir.last().expect("non-empty") != u16::from(b'\\') {
        out.push(u16::from(b'\\'));
    }
    out.extend_from_slice(name);
    out
}

/// The rename dialog's OK composition (viv.c:7273-7288): the typed name
/// joins the old file's directory and the old file's extension is
/// APPENDED — a typed name that already carries a dot produces a
/// double extension (`foo.png` renaming `a.png` → `foo.png.png`), the
/// upstream quirk kept. `unchanged` is the byte-exact old-vs-new compare
/// (`string_compare`, case sensitive) that skips the shell call when the
/// names coincide; a pure case difference is NOT unchanged — it renames
/// (and Windows may report the case-only rename through the mapping).
pub(crate) fn rename_target(old_full: &[u16], typed: &[u16]) -> (Vec<u16>, bool) {
    let dir = path_before_last_sep(old_full);
    let ext = extension_after_last_dot(old_full);
    let mut new_full = join(dir, typed);
    if !ext.is_empty() {
        new_full.push(u16::from(b'.'));
        new_full.extend_from_slice(ext);
    }
    let unchanged = old_full == new_full.as_slice();
    (new_full, unchanged)
}

/// The rename dialog's prefill (viv.c:7244-7246): the file-name part after
/// the last '\\', truncated at that NAME's last '.' (`string_remove_
/// extension`, string.c:742-766 — a leading-dot name prefills EMPTY, a
/// double extension keeps one: `a.tar.gz` → `a.tar`). Unlike
/// `extension_after_last_dot` this cut only looks at the name tail.
pub(crate) fn rename_stem(old_full: &[u16]) -> Vec<u16> {
    let sep = old_full
        .iter()
        .rposition(|&ch| ch == u16::from(b'\\'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let name = &old_full[sep..];
    let dot = name
        .iter()
        .rposition(|&ch| ch == u16::from(b'.'))
        .unwrap_or(name.len());
    name[..dot].to_vec()
}

/// A single path into the double-null-terminated list `pFrom`/`pTo` want
/// (upstream `string_copy_double_null`, string.c:716-729). The input must
/// already end in its own NUL (`to_wide_os` output).
pub(crate) fn double_null_terminated(path_nul: &[u16]) -> Vec<u16> {
    debug_assert!(path_nul.last() == Some(&0), "input must be NUL-terminated");
    let mut out = path_nul.to_vec();
    out.push(0);
    out
}

// ---------- shell: SHFileOperation ----------

/// What an SHFileOperation came to (the ret/aborted pair upstream's
/// delete and rename arms branch on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellOutcome {
    /// ret == 0 and nothing aborted — the operation happened.
    Done,
    /// ret == 0 but `fAnyOperationsAborted` — the user answered No at a
    /// collision/confirm dialog.
    Aborted,
    /// ret != 0 — the operation failed outright (fail-soft upstream).
    Failed,
}

/// `_viv_delete`'s shell half (viv.c:7208-7219): FO_DELETE over the
/// current file, `FOF_ALLOWUNDO` for the recycle bin and no flags for a
/// permanent delete. The confirmation dialog is the shell's own (upstream
/// passes no FOF_NOCONFIRMATION — the "send to Recycle Bin?" prompt
/// appears with the viewer as owner).
pub(crate) fn shell_delete(hwnd: HWND, path: &OsStr, permanently: bool) -> ShellOutcome {
    let from = double_null_terminated(&to_wide_os(path));
    let mut fo = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        pTo: PCWSTR::null(),
        fFlags: if permanently {
            0
        } else {
            FOF_ALLOWUNDO.0 as u16
        },
        ..Default::default()
    };
    // SAFETY: `fo` is a valid stack struct; `from` outlives the call and
    // ends in the double NUL the API requires (built above); every other
    // pointer field is null. The API writes only back into `fo`.
    let ret = unsafe { SHFileOperationW(&mut fo) };
    outcome_of(ret, fo.fAnyOperationsAborted)
}

/// The rename FO_RENAME (viv.c:7293-7338): FOF_ALLOWUNDO |
/// FOF_WANTMAPPINGHANDLE, and on success the single-entry name mapping's
/// resolved path when the shell renamed onto a collision name ("xxx -
/// Copy"). The mapping object is always freed (SHFreeNameMappings pairs
/// with a non-null handle, viv.c:7337-7340).
pub(crate) enum RenameOutcome {
    /// Renamed; the resolved path when a mapping existed (the name the
    /// file ACTUALLY landed on), else the composed one — the caller
    /// decides which to record.
    Done { resolved_new_path: Option<Vec<u16>> },
    /// The user clicked No at the collision dialog — upstream keeps the
    /// rename dialog open for another try.
    Aborted,
    /// The shell refused the rename — upstream ends the dialog anyway
    /// (`dont_end_dialog` stays 0, viv.c:7350).
    Failed,
}

/// The documented `hNameMappings` header (shellapi.h's own
/// HANDLETOMAPPINGS): the count followed by a POINTER to the mapping
/// array. Upstream's `_viv_name_mapping_t` (viv.c:444-448) is this
/// struct verbatim; only the count==1 case is read (viv.c:7318-7326).
#[repr(C)]
struct HandleToMappings {
    count: u32,
    mappings: *const SHNAMEMAPPINGW,
}

pub(crate) fn shell_rename(hwnd: HWND, from: &OsStr, to: &[u16]) -> RenameOutcome {
    let from_list = double_null_terminated(&to_wide_os(from));
    let to_list = double_null_terminated(to);
    let mut fo = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_RENAME,
        pFrom: PCWSTR(from_list.as_ptr()),
        pTo: PCWSTR(to_list.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_WANTMAPPINGHANDLE).0 as u16,
        ..Default::default()
    };
    // SAFETY: as `shell_delete`; both lists are double-NUL terminated and
    // outlive the call.
    let ret = unsafe { SHFileOperationW(&mut fo) };
    if ret != 0 {
        return RenameOutcome::Failed;
    }
    let mut resolved = None;
    if !fo.hNameMappings.is_null() {
        // SAFETY: the non-null handle points at the shell-allocated
        // HANDLETOMAPPINGS block for this call; read-only access between
        // SHFileOperationW returning and SHFreeNameMappings below.
        let header = unsafe { &*(fo.hNameMappings as *const HandleToMappings) };
        if header.count == 1 && !header.mappings.is_null() {
            // SAFETY: the array pointer is valid for count entries; count
            // is exactly 1 here.
            let entry = unsafe { &*header.mappings };
            if !entry.pszNewPath.is_null() {
                let wide = PCWSTR(entry.pszNewPath.as_ptr());
                // SAFETY: pszNewPath is a shell-owned NUL-terminated wide
                // string, valid until the SHFreeNameMappings below.
                resolved = Some(unsafe { wide.as_wide() }.to_vec());
            }
        }
        // SAFETY: the handle came from this SHFileOperationW and is freed
        // exactly once.
        unsafe { SHFreeNameMappings(Some(HANDLE(fo.hNameMappings))) };
    }
    if fo.fAnyOperationsAborted.as_bool() {
        RenameOutcome::Aborted
    } else {
        RenameOutcome::Done {
            resolved_new_path: resolved,
        }
    }
}

/// Copy To / Move To (viv.c:2535-2563): FO_COPY / FO_MOVE with
/// FOF_ALLOWUNDO. The return is ignored upstream (fail-soft) — riviv
/// drops it too.
pub(crate) fn shell_copy_move(hwnd: HWND, from: &OsStr, to: &[u16], copy: bool) {
    let from_list = double_null_terminated(&to_wide_os(from));
    let to_list = double_null_terminated(to);
    let mut fo = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: if copy { FO_COPY } else { FO_MOVE },
        pFrom: PCWSTR(from_list.as_ptr()),
        pTo: PCWSTR(to_list.as_ptr()),
        fFlags: FOF_ALLOWUNDO.0 as u16,
        ..Default::default()
    };
    // SAFETY: as `shell_delete`.
    let _ = unsafe { SHFileOperationW(&mut fo) };
}

fn outcome_of(ret: i32, aborted: windows::core::BOOL) -> ShellOutcome {
    if ret != 0 {
        ShellOutcome::Failed
    } else if aborted.as_bool() {
        ShellOutcome::Aborted
    } else {
        ShellOutcome::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Vec<u16> {
        v.encode_utf16().collect()
    }

    #[test]
    fn typed_bare_name_gets_the_old_extension_back() {
        // viv.c:7280-7287: foo renaming C:\dir\a.png -> C:\dir\foo.png.
        let (new, unchanged) = rename_target(&s(r"C:\dir\a.png"), &s("foo"));
        assert_eq!(new, s(r"C:\dir\foo.png"));
        assert!(!unchanged);
    }

    #[test]
    fn typed_name_with_a_dot_doubles_the_extension() {
        // The unconditional append: foo.png renaming a.png -> foo.png.png
        // — upstream quirk, kept on purpose.
        let (new, _) = rename_target(&s(r"C:\dir\a.png"), &s("foo.png"));
        assert_eq!(new, s(r"C:\dir\foo.png.png"));
    }

    #[test]
    fn extensionless_old_file_appends_nothing() {
        let (new, _) = rename_target(&s(r"C:\dir\a"), &s("foo"));
        assert_eq!(new, s(r"C:\dir\foo"));
    }

    #[test]
    fn dot_in_the_directory_name_leaks_into_the_extension() {
        // string_get_extension scans the WHOLE path (string.c:768): the
        // last dot of C:\my.dir\a is in the directory, so the "extension"
        // is dir\a and the composer appends .dir\a — upstream bug-for-bug.
        let (new, _) = rename_target(&s(r"C:\my.dir\a"), &s("foo"));
        assert_eq!(new, s(r"C:\my.dir\foo.dir\a"));
    }

    #[test]
    fn identical_composition_skips_the_shell_case_sensitively() {
        let full = s(r"C:\dir\a.png");
        let (_, unchanged) = rename_target(&full, &s("a"));
        assert!(unchanged);
        // A case-only change is a rename, not a no-op.
        let (new, unchanged) = rename_target(&full, &s("A"));
        assert_eq!(new, s(r"C:\dir\A.png"));
        assert!(!unchanged);
    }

    #[test]
    fn root_level_file_keeps_the_bare_drive_directory() {
        // string_get_path_part cuts before the last '\\': C:\a.png -> C:
        // (the drive without the separator, upstream's own cut).
        let (new, _) = rename_target(&s(r"C:\a.png"), &s("b"));
        assert_eq!(new, s(r"C:\b.png"));
    }

    #[test]
    fn double_null_terminated_adds_the_final_nul() {
        let path = s(r"C:\dir\a.png");
        let mut with_nul = path.clone();
        with_nul.push(0);
        let list = double_null_terminated(&with_nul);
        assert_eq!(&list[..path.len()], &path[..]);
        assert_eq!(&list[path.len()..], &[0, 0]);
    }

    #[test]
    fn rename_stem_strips_the_path_and_the_last_name_dot() {
        assert_eq!(rename_stem(&s(r"C:\dir\a.png")), s("a"));
        assert_eq!(rename_stem(&s(r"C:\dir\a.tar.gz")), s("a.tar"));
        assert_eq!(rename_stem(&s(r"C:\dir\noext")), s("noext"));
    }

    #[test]
    fn rename_stem_of_a_leading_dot_name_is_empty() {
        // string_remove_extension cuts at the only dot: ".hidden" -> "".
        assert_eq!(rename_stem(&s(r"C:\dir\.hidden")), s(""));
    }
}
