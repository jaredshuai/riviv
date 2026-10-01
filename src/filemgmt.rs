//! #43 file-management actions: the rename-dialog composition (pure) and
//! the SHFileOperation shells (delete / rename / copy-to / move-to), plus
//! #178's recycle-bin pair restore (the `$I`/`$R` metadata parse and the
//! move-back).
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
use std::os::windows::ffi::{OsStrExt, OsStringExt};

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

// ---------- #178 pure + fs: the recycle-bin pair restore ----------

/// One parsed `$I` metadata file — the index half of the Vista+ recycle
/// bin's `$I`/`$R` pair (the `$R` twin carries the data). Layout,
/// byte-verified against this machine's real writer (smoke178 run 1's
/// ground-truth dump): u64 LE version (2), u64 LE original size, u64 LE
/// deletion FILETIME, then a u32 LE path length in UTF-16 units, then
/// exactly that many path units — no terminator on the writer observed
/// here; some writers count a trailing NUL, which the parse tolerates.
/// There is no public per-item enumeration API (the crate ships only
/// SHEmptyRecycleBin/SHQueryRecycleBin, and upstream's own note says "no
/// undo api", viv.c:138) — this format is the de-facto standard every
/// recycle-bin tool reads.
struct RecycleMeta {
    deleted_at: u64,
    original_path: Vec<u16>,
}

/// The fixed `$I` header: version + size + deletion FILETIME, u64 LE each.
const I_HEADER: usize = 24;
/// The only `$I` version a supported OS writes (min-OS Win10 1607; the
/// pre-Vista `INFO` format died with 9x).
const I_VERSION: u64 = 2;

fn parse_i_file(bytes: &[u8]) -> Option<RecycleMeta> {
    let header: &[u8; I_HEADER] = bytes.get(..I_HEADER)?.try_into().ok()?;
    let word = |at: usize| -> Option<u64> {
        Some(u64::from_le_bytes(header[at..at + 8].try_into().ok()?))
    };
    if word(0)? != I_VERSION {
        return None;
    }
    let deleted_at = word(16)?;
    // The u32 length prefix right past the fixed header, then exactly
    // that many UTF-16 units — a count overrunning the file is malformed.
    let count = u32::from_le_bytes(bytes.get(I_HEADER..I_HEADER + 4)?.try_into().ok()?) as usize;
    let path = bytes.get(I_HEADER + 4..I_HEADER + 4 + count * 2)?;
    let units: Vec<u16> = path
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    // Tolerate a trailing NUL inside the declared count (writer
    // variance); a NUL anywhere else ends the path early.
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    Some(RecycleMeta {
        deleted_at,
        original_path: units[..end].to_vec(),
    })
}

/// The `$R` twin's path: an `$I` file's name with the `I` flipped to `R`
/// (`$I3F9A.png` ↔ `$R3F9A.png`). None when the name is not an `$I` file.
fn r_twin(i_path: &[u16]) -> Option<Vec<u16>> {
    let name_at = i_path
        .iter()
        .rposition(|&unit| unit == u16::from(b'\\'))
        .map_or(0, |at| at + 1);
    if i_path.get(name_at) == Some(&u16::from(b'$'))
        && i_path.get(name_at + 1) == Some(&u16::from(b'I'))
    {
        let mut twin = i_path.to_vec();
        twin[name_at + 1] = u16::from(b'R');
        Some(twin)
    } else {
        None
    }
}

/// The volume root whose `$Recycle.Bin` holds a deleted file's pair
/// (`C:\a.png` → `C:\` — the bin is per-volume, so the pair sits on the
/// file's own drive). None for UNC / drive-relative / relative paths:
/// undo refuses to guess there.
fn drive_root(path: &[u16]) -> Option<Vec<u16>> {
    let is_letter = |unit: u16| {
        (unit >= u16::from(b'a') && unit <= u16::from(b'z'))
            || (unit >= u16::from(b'A') && unit <= u16::from(b'Z'))
    };
    let sep = u16::from(b'\\');
    if path.len() >= 3
        && is_letter(path[0])
        && path[1] == u16::from(b':')
        && (path[2] == sep || path[2] == u16::from(b'/'))
    {
        Some(vec![path[0], path[1], sep])
    } else {
        None
    }
}

/// Path equality with ASCII case-folding: the recorded path and the
/// `$I`'s stored copy come from different directory listings, and NTFS
/// matches ASCII case-insensitively. Non-ASCII units compare exactly —
/// the full Unicode upcase table is the filesystem's job, not ours.
fn path_eq_folded(a: &[u16], b: &[u16]) -> bool {
    let fold = |unit: u16| {
        if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
            unit + 32
        } else {
            unit
        }
    };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| fold(*x) == fold(*y))
}

/// Why an undo-delete restore could not happen (the status flash's text).
#[derive(Debug)]
pub(crate) enum UndoDeleteError {
    /// The recorded path carries no volume root (UNC, relative) — there
    /// is no `$Recycle.Bin` location to scan.
    NoVolumeRoot,
    /// No `$I` pair matched the original path: the bin was emptied, the
    /// pair was already restored (Explorer's own undo), or the bin was
    /// disabled at delete time so the file left permanently.
    NotInBin,
    /// A file already sits at the original path — restore never
    /// overwrites it.
    TargetExists,
    /// The `$R` twin refused to move back (parent directory gone, ACL,
    /// drive absent). The pair stays intact — Explorer's undo still can.
    RestoreFailed(std::io::Error),
}

/// Scan one volume's `$Recycle.Bin` for the `$I` whose stored original
/// path matches, returning it with its `$R` twin. Every owner-SID
/// subdirectory is scanned (other users' are unreadable by design and
/// skip silently); among duplicates the NEWEST deletion time wins — a
/// delete → restore → delete cycle recycles the same path twice, and the
/// freshest pair is the latest delete's. `volume_root` is a parameter
/// (not derived here) so tests can point the scan at a fabricated bin.
fn find_pair(
    volume_root: &std::path::Path,
    original_path: &[u16],
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let bin = volume_root.join("$Recycle.Bin");
    let Ok(sids) = std::fs::read_dir(&bin) else {
        return None;
    };
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    for sid in sids.flatten() {
        let Ok(files) = std::fs::read_dir(sid.path()) else {
            continue; // another user's SID directory — unreadable by design
        };
        for entry in files.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name() else {
                continue;
            };
            // The $I prefix is pure ASCII, so encoded-byte prefix checks
            // are encoding-safe.
            if !name.as_encoded_bytes().starts_with(b"$I") {
                continue;
            }
            let Some(meta) = std::fs::read(&path).ok().and_then(|b| parse_i_file(&b)) else {
                continue;
            };
            if !path_eq_folded(&meta.original_path, original_path) {
                continue;
            }
            if best.as_ref().is_none_or(|(t, _)| meta.deleted_at > *t) {
                best = Some((meta.deleted_at, path));
            }
        }
    }
    let (_, i_path) = best?;
    let twin = {
        let wide: Vec<u16> = i_path
            .file_name()
            .map(|name| name.encode_wide().collect())
            .unwrap_or_default();
        let name = r_twin(&wide)?;
        i_path.with_file_name(std::ffi::OsString::from_wide(&name))
    };
    Some((i_path, twin))
}

/// The restore half of #178's undo-delete: move the `$R` twin back to the
/// original path and drop the `$I` index. The pair is touched only after
/// every precondition holds; a failed rename leaves the bin exactly as
/// it was. Cleaning the `$I` after a successful move is fail-soft — the
/// file is already back, and a ghost index at worst shows a broken bin
/// entry.
pub(crate) fn recycle_restore(path: &OsStr) -> Result<(), UndoDeleteError> {
    let recorded: Vec<u16> = path.encode_wide().collect();
    let Some(root) = drive_root(&recorded) else {
        return Err(UndoDeleteError::NoVolumeRoot);
    };
    let root = std::path::PathBuf::from(std::ffi::OsString::from_wide(&root));
    restore_under(&root, path, &recorded)
}

/// `recycle_restore` with the volume root supplied — the seam the unit
/// tests aim at a fabricated `$Recycle.Bin` tree.
fn restore_under(
    root: &std::path::Path,
    path: &OsStr,
    recorded: &[u16],
) -> Result<(), UndoDeleteError> {
    if std::path::Path::new(path).exists() {
        return Err(UndoDeleteError::TargetExists);
    }
    let Some((i_path, r_path)) = find_pair(root, recorded) else {
        return Err(UndoDeleteError::NotInBin);
    };
    std::fs::rename(&r_path, path).map_err(UndoDeleteError::RestoreFailed)?;
    let _ = std::fs::remove_file(&i_path);
    Ok(())
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

    // ---------- #178: the $I parse and the pair surgery ----------

    /// A minimal valid `$I` body in the REAL layout this machine's
    /// writer produces (byte-verified by smoke178 run 1): version 2,
    /// size 7, deletion FILETIME, u32 unit count, the path units with no
    /// terminator.
    fn i_file(path: &str, deleted_at: u64) -> Vec<u8> {
        let units: Vec<u16> = path.encode_utf16().collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u64.to_le_bytes());
        bytes.extend_from_slice(&7u64.to_le_bytes());
        bytes.extend_from_slice(&deleted_at.to_le_bytes());
        bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn i_file_v2_parses_time_and_path() {
        let meta = parse_i_file(&i_file(r"C:\pics\a.png", 0x1122_3344_5566_7788))
            .expect("valid v2 body parses");
        assert_eq!(meta.deleted_at, 0x1122_3344_5566_7788);
        assert_eq!(meta.original_path, s(r"C:\pics\a.png"));
        // Bytes past the declared path are ignored (alignment padding).
        let mut padded = i_file(r"C:\pics\a.png", 1);
        padded.extend_from_slice(&[0xCC; 4]);
        assert_eq!(
            parse_i_file(&padded)
                .expect("padding is not fatal")
                .original_path,
            s(r"C:\pics\a.png")
        );
        // A writer that counts its trailing NUL parses to the same path.
        let mut with_nul = i_file(r"C:\a", 2);
        let count_at = I_HEADER;
        let count =
            u32::from_le_bytes(with_nul[count_at..count_at + 4].try_into().unwrap()) as usize;
        with_nul[count_at..count_at + 4].copy_from_slice(&((count + 1) as u32).to_le_bytes());
        with_nul.push(0);
        with_nul.push(0);
        assert_eq!(
            parse_i_file(&with_nul)
                .expect("NUL inside the count is tolerated")
                .original_path,
            s(r"C:\a")
        );
    }

    /// The first bytes smoke178 run 1 dumped off a REAL pair this
    /// machine's shell wrote (v2 | size 0x4f | FILETIME | count 0x3b |
    /// "C:\U...") — the ground truth the length-prefix layout came from.
    /// The dump cut at 48 bytes; reconstructing the exact temp path gives
    /// 58 units, so the writer's count 0x3b=59 INCLUDES the trailing
    /// NUL the tolerance arm already covers.
    #[test]
    fn i_file_parses_the_captured_real_writer_bytes() {
        let path = "C:\\Users\\jared\\AppData\\Local\\Temp\\riviv-178-smoke\\fx\\a.png";
        let units: Vec<u16> = path.encode_utf16().collect();
        assert_eq!(units.len(), 58, "58 units + NUL = the captured 0x3b");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u64.to_le_bytes());
        bytes.extend_from_slice(&0x4fu64.to_le_bytes());
        bytes.extend_from_slice(&0x01dd_514e_e7b4_aee0u64.to_le_bytes());
        bytes.extend_from_slice(&0x3bu32.to_le_bytes());
        for unit in &units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
        let meta = parse_i_file(&bytes).expect("the real writer's shape");
        assert_eq!(meta.deleted_at, 0x01dd_514e_e7b4_aee0);
        assert_eq!(meta.original_path, units);
    }

    #[test]
    fn i_file_rejects_short_wrong_version_and_overrun_counts() {
        assert!(parse_i_file(&[]).is_none(), "empty");
        assert!(
            parse_i_file(&i_file(r"C:\a", 1)[..23]).is_none(),
            "truncated header"
        );
        let mut v1 = i_file(r"C:\a", 1);
        v1[0] = 1; // version 1 — the pre-Vista layout, unsupported
        assert!(parse_i_file(&v1).is_none(), "version 1");
        // A count that overruns the file is malformed, not a short read.
        let mut overrun = i_file(r"C:\a", 1);
        let count_at = I_HEADER;
        overrun[count_at..count_at + 4].copy_from_slice(&9u32.to_le_bytes());
        assert!(parse_i_file(&overrun).is_none(), "count overruns EOF");
        // The length prefix itself truncated.
        assert!(
            parse_i_file(&i_file(r"C:\a", 1)[..26]).is_none(),
            "prefix cut"
        );
    }

    #[test]
    fn r_twin_flips_the_i_prefix_in_place() {
        assert_eq!(
            r_twin(&s(r"C:\$Recycle.Bin\S-1-5\$I3F9A.png")).as_deref(),
            Some(&s(r"C:\$Recycle.Bin\S-1-5\$R3F9A.png")[..])
        );
        assert_eq!(r_twin(&s("plain.png")), None, "not an $I name");
        assert_eq!(r_twin(&s(r"C:\x\$R3F9A.png")), None, "already the twin");
    }

    #[test]
    fn drive_root_accepts_only_drive_absolute_paths() {
        assert_eq!(
            drive_root(&s(r"C:\pics\a.png")).as_deref(),
            Some(&s("C:\\")[..])
        );
        assert_eq!(
            drive_root(&s(r"d:\a")).as_deref(),
            Some(&s(r"d:\")[..]),
            "lowercase drive letter"
        );
        assert_eq!(
            drive_root(&s(r"E:/a.png")).as_deref(),
            Some(&s(r"E:\")[..]),
            "forward slash accepted, normalized to backslash"
        );
        assert_eq!(drive_root(&s(r"\\srv\share\a.png")), None, "UNC");
        assert_eq!(drive_root(&s(r"C:a.png")), None, "drive-relative");
        assert_eq!(drive_root(&s("a.png")), None, "bare relative");
    }

    #[test]
    fn path_eq_folded_folds_ascii_only() {
        assert!(path_eq_folded(&s(r"C:\Pics\A.PNG"), &s(r"c:\pics\a.png")));
        // The case difference in the ASCII segment folds.
        assert!(path_eq_folded(&s("图\\a.png"), &s("图\\A.png")));
        // Full-width Ａ/ａ are non-ASCII lookalikes of A/a: we do NOT
        // fold them — the upcase table is the filesystem's, not ours.
        assert!(!path_eq_folded(&s("Ａ.png"), &s("ａ.png")));
    }

    /// A fabricated bin under a unique temp directory, the shape
    /// `find_pair`/`restore_under` walk: `<root>/$Recycle.Bin/<sid>/$I..`.
    fn fake_bin(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir()
            .join(format!("riviv-178-i-{}", tag))
            .join("vol");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("$Recycle.Bin").join("S-1-5-21-a")).unwrap();
        std::fs::create_dir_all(root.join("$Recycle.Bin").join("S-1-5-21-b")).unwrap();
        root
    }

    fn drop_i(root: &std::path::Path, sid: &str, name: &str, path: &str, deleted_at: u64) {
        std::fs::write(
            root.join("$Recycle.Bin").join(sid).join(name),
            i_file(path, deleted_at),
        )
        .unwrap();
    }

    #[test]
    fn find_pair_matches_folded_and_takes_the_newest() {
        let root = fake_bin("find");
        // Same original path in two owner dirs plus a case-variant copy
        // and an unrelated pair; the newest match wins.
        drop_i(&root, "S-1-5-21-a", "$IOLD1.png", r"D:\pics\a.png", 100);
        drop_i(&root, "S-1-5-21-b", "$INEW1.png", r"d:\PICS\A.png", 200);
        drop_i(&root, "S-1-5-21-a", "$IOTH.png", r"D:\pics\other.png", 300);
        let victim: Vec<u16> = OsStr::new(r"D:\pics\a.png").encode_wide().collect();
        let (i_path, r_path) = find_pair(&root, &victim).expect("a match exists");
        assert_eq!(i_path, root.join("$Recycle.Bin/S-1-5-21-b/$INEW1.png"));
        assert_eq!(r_path, root.join("$Recycle.Bin/S-1-5-21-b/$RNEW1.png"));
        let absent: Vec<u16> = OsStr::new(r"D:\pics\gone.png").encode_wide().collect();
        assert!(find_pair(&root, &absent).is_none());
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn restore_under_moves_the_twin_back_and_cleans_the_index() {
        let root = fake_bin("restore");
        let target = root.join("pics").join("a.png");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            root.join("$Recycle.Bin/S-1-5-21-a/$R2222.png"),
            b"recycled bytes",
        )
        .unwrap();
        drop_i(&root, "S-1-5-21-a", "$I2222.png", r"D:\pics\a.png", 50);
        let recorded: Vec<u16> = OsStr::new(r"D:\pics\a.png").encode_wide().collect();
        restore_under(&root, target.as_os_str(), &recorded).expect("restore succeeds");
        assert_eq!(std::fs::read(&target).unwrap(), b"recycled bytes");
        assert!(!root.join("$Recycle.Bin/S-1-5-21-a/$R2222.png").exists());
        assert!(!root.join("$Recycle.Bin/S-1-5-21-a/$I2222.png").exists());
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn restore_under_never_overwrites_and_reports_why() {
        let root = fake_bin("refuse");
        let target = root.join("a.png");
        std::fs::write(&target, b"new occupant").unwrap();
        drop_i(&root, "S-1-5-21-a", "$I3333.png", r"D:\a.png", 50);
        std::fs::write(root.join("$Recycle.Bin/S-1-5-21-a/$R3333.png"), b"old").unwrap();
        let recorded: Vec<u16> = OsStr::new(r"D:\a.png").encode_wide().collect();
        match restore_under(&root, target.as_os_str(), &recorded) {
            Err(UndoDeleteError::TargetExists) => {}
            other => panic!("expected TargetExists, got {:?}", other),
        }
        // The pair is untouched — Explorer's undo still can.
        assert!(root.join("$Recycle.Bin/S-1-5-21-a/$I3333.png").exists());
        assert!(root.join("$Recycle.Bin/S-1-5-21-a/$R3333.png").exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"new occupant");
        // A recorded path with no pair at all → NotInBin.
        let never: Vec<u16> = OsStr::new(r"D:\never.png").encode_wide().collect();
        match restore_under(&root, root.join("absent.png").as_os_str(), &never) {
            Err(UndoDeleteError::NotInBin) => {}
            other => panic!("expected NotInBin, got {:?}", other),
        }
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn recycle_restore_refuses_paths_without_a_volume_root() {
        match recycle_restore(OsStr::new(r"\\srv\share\a.png")) {
            Err(UndoDeleteError::NoVolumeRoot) => {}
            other => panic!("expected NoVolumeRoot, got {:?}", other),
        }
    }
}
