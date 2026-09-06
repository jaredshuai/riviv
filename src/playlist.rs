//! Playlist model and navigation math (#6; upstream playlist globals +
//! `_viv_playlist_*` / `_viv_next` / `_viv_home`, viv.c:9341-9611 / 5817-6263).
//!
//! Upstream's playlist is a linked list of `WIN32_FIND_DATA`s whose
//! `cFileName` holds the FULL path; navigation never re-orders it — every
//! key press re-scans the entries with `_viv_fd_compare` and picks the
//! smallest entry strictly after the current one (wrapping to the global
//! smallest). The default sort config (config.c:43-44) is
//! `DATE_MODIFIED` + descending, which after the ascending-negation at
//! viv.c:5809-5812 lands on: **mtime descending, filename ascending,
//! insertion id ascending**. This module hardcodes that default; the sort
//! config UI is M3.
//!
//! The pure half (compare / next / home / extension check) is unit-tested;
//! the FS half (recursive folder scan) mirrors upstream's synchronous
//! FindFirstFile walk, run on the UI thread exactly like upstream — a huge
//! dropped tree freezes the window in upstream too.

use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::fs::Metadata;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Globalization::{
    CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringW, LOCALE_USER_DEFAULT, NORM_IGNORECASE,
    SORT_DIGITSASNUMBERS, SORT_STRINGSORT,
};
use windows::Win32::Storage::FileSystem::{
    FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

/// One navigable image. `modified` is the file's mtime in 100 ns ticks
/// since an arbitrary fixed epoch — only the relative order ever matters,
/// and every entry (playlist scan, folder scan, direct open) reads it from
/// `std::fs` so the epochs agree.
///
/// `id` is the insertion id (upstream parks it in `dwReserved0/1`,
/// viv.c:9478-9480). The counter starts at 0 and `clear` resets it, so the
/// first entry after a clear is id 0 — the SAME id a direct open carries
/// (viv.c:1375-1376 zeroes it). That collision is upstream's own node
/// identity: `_viv_playlist_from_fd` (viv.c:13548-13567) matches by id
/// equality with no non-zero guard, which is exactly how the shift-drop
/// "add current if empty" node (id 0, same file as the direct-opened
/// current) gets excluded from navigation candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaylistEntry {
    pub(crate) path: OsString,
    pub(crate) modified: i64,
    pub(crate) id: u64,
}

/// The playlist (upstream `_viv_playlist_start/_last/_count/_viv_playlist_id`).
/// Insertion order is preserved; navigation sorts on the fly.
pub(crate) struct Playlist {
    entries: Vec<PlaylistEntry>,
    next_id: u64,
}

impl Playlist {
    pub(crate) fn new() -> Self {
        Playlist {
            entries: Vec::new(),
            next_id: 0,
        }
    }

    /// Upstream `_viv_playlist_clearall` — also resets the id counter
    /// (viv.c:9380), so ids restart at 1 after a replace-drop.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.next_id = 0;
    }

    /// Append an entry, assigning the next insertion id (upstream
    /// `_viv_playlist_add`, viv.c:9471-9537: assign-then-increment from the
    /// counter `clear` zeroed — first entry after a clear is id 0 — minus
    /// the shuffle-index bookkeeping, shuffle is M3).
    pub(crate) fn add(&mut self, path: OsString, modified: i64) -> &PlaylistEntry {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(PlaylistEntry { path, modified, id });
        self.entries.last().expect("just pushed")
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }

    /// The first-inserted entry — what upstream opens for a multi-argument
    /// command line (`_viv_playlist_start`, insertion order, viv.c:5077-5080).
    pub(crate) fn first(&self) -> Option<&PlaylistEntry> {
        self.entries.first()
    }
}

/// The 9 playable extensions (upstream `_viv_association_extensions`,
/// viv.c:1136-1147) — ASCII, compared case-insensitively.
const EXTENSIONS: [&str; 9] = [
    "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
];

/// ASCII case-insensitive equality — upstream's
/// `string_icompare_lowercase_ascii` over the extension table.
fn ascii_eq_ignore_case(text: &[u16], known: &[u8]) -> bool {
    text.len() == known.len()
        && text.iter().zip(known).all(|(&c, &k)| {
            c == u16::from(k) || (k.is_ascii_alphabetic() && c == u16::from(k.to_ascii_uppercase()))
        })
}

/// Upstream `_viv_is_valid_filename` (viv.c:6265-6300): the text after the
/// LAST '.' of the (full-path) string must equal one of the extensions,
/// ASCII case-insensitively. Scanning the whole path matches upstream (it
/// scans `cFileName`, which holds the full path): a '.' in a directory
/// component only "matches" when the tail happens to equal an extension,
/// which no real directory suffix does.
pub(crate) fn is_valid_path(path: &OsStr) -> bool {
    let wide = path.encode_wide().collect::<Vec<u16>>();
    let Some(dot) = wide.iter().rposition(|&c| c == u16::from(b'.')) else {
        return false;
    };
    let ext = &wide[dot + 1..];
    EXTENSIONS
        .iter()
        .any(|known| ascii_eq_ignore_case(ext, known.as_bytes()))
}

/// The collation flags of upstream `_viv_fd_compare_name` (viv.c:5655-5661):
/// user locale, case-insensitive, string-sort, digits-as-numbers (the Win7+
/// flag upstream gates on; every supported Windows has it).
const NAME_SORT_FLAGS: u32 = NORM_IGNORECASE.0 | SORT_STRINGSORT.0 | SORT_DIGITSASNUMBERS.0;

/// Filename part of a full path — everything after the last `\`
/// (upstream `string_get_filename_part`; it does not treat `/` as a
/// separator, and neither do we).
fn filename_part(path: &OsStr) -> Vec<u16> {
    let wide = path.encode_wide().collect::<Vec<u16>>();
    match wide.iter().rposition(|&c| c == u16::from(b'\\')) {
        Some(sep) => wide[sep + 1..].to_vec(),
        None => wide,
    }
}

/// Locale collation of two filename parts (upstream
/// `_viv_fd_compare_name`, viv.c:5648-5678). A CompareStringW failure (0)
/// falls through to `Equal`, exactly like upstream's switch default, so the
/// caller's id tiebreak decides.
fn compare_name(a: &OsStr, b: &OsStr) -> Ordering {
    let a = filename_part(a);
    let b = filename_part(b);
    // SAFETY: read-only collation query over two valid u16 slices whose
    // lengths the wrapper passes explicitly; the result is a value code.
    let ret = unsafe { CompareStringW(LOCALE_USER_DEFAULT, NAME_SORT_FLAGS, &a, &b) };
    match ret {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_GREATER_THAN => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

/// The navigation order (upstream `_viv_fd_compare` under the default
/// config `DATE_MODIFIED` + not-ascending, config.c:43-44): mtime
/// descending, then filename ascending (the negated-name tiebreak at
/// viv.c:5774 un-negates under the final inversion), then insertion id
/// ascending (`_viv_compare_id`, viv.c:5623-5646).
pub(crate) fn fd_compare(a: &PlaylistEntry, b: &PlaylistEntry) -> Ordering {
    a.modified
        .cmp(&b.modified)
        .reverse()
        .then_with(|| compare_name(&a.path, &b.path))
        .then_with(|| a.id.cmp(&b.id))
}

/// The next entry relative to `current` (upstream `_viv_next`'s scan,
/// viv.c:5926-6099, unified over the playlist and folder-scan arms):
///
/// - `best` — the smallest entry strictly after `current` (largest strictly
///   before, for `prev`). Entries comparing equal to `current` never
///   qualify (upstream's `compare_ret != 0`).
/// - `start` — the wrap target: the global smallest (largest for `prev`)
///   over every entry except the playlist node `current` IS (identity =
///   insertion id equality, upstream `_viv_playlist_from_fd`, no non-zero
///   guard — the id-0 collision between a direct-opened current and the
///   "add current if empty" node is upstream's own mechanism).
///
/// `best` wins — with NO same-path check (upstream viv.c:6079-6082 opens
/// the best duplicate of the current file just like any other entry); the
/// wrap target is only opened when its path differs from `current`'s
/// (upstream's string compare, viv.c:6088); a single-image playlist ends
/// with neither and navigation is a no-op (never blanks).
///
/// `from_playlist` selects the arm: the playlist skips the one node whose
/// id equals `current.id` (upstream `_viv_playlist_from_fd`,
/// viv.c:5928-5933); the folder scan has NO node exclusion at all — its
/// entries are built with id 0 and upstream never identity-matches them
/// (viv.c:6013-6069), so the same-file case is left to the compare and
/// same-path checks.
pub(crate) fn next<'a>(
    entries: &'a [PlaylistEntry],
    current: Option<&PlaylistEntry>,
    prev: bool,
    from_playlist: bool,
) -> Option<&'a PlaylistEntry> {
    let current = current?;
    let mut best: Option<&PlaylistEntry> = None;
    let mut start: Option<&PlaylistEntry> = None;
    for entry in entries {
        if from_playlist && entry.id == current.id {
            continue; // the node `current` is (plain id equality, viv.c:5928-5933)
        }
        let cmp = fd_compare(entry, current);
        // best: strictly after (before, for prev) the current, and closer to
        // it than the best so far (upstream viv.c:5937-5962).
        let beats_best = |probe: &PlaylistEntry, best: Option<&PlaylistEntry>| {
            best.is_none_or(|b| {
                let rel = fd_compare(probe, b);
                if prev {
                    rel == Ordering::Greater
                } else {
                    rel == Ordering::Less
                }
            })
        };
        let after = if prev {
            cmp == Ordering::Less
        } else {
            cmp == Ordering::Greater
        };
        if after && beats_best(entry, best) {
            best = Some(entry);
        }
        // start: the wrap extreme over every non-current entry (upstream
        // viv.c:5965-5982 — strictly more extreme than the start so far).
        let beats_start = |probe: &PlaylistEntry, start: Option<&PlaylistEntry>| {
            start.is_none_or(|s| {
                let rel = fd_compare(probe, s);
                if prev {
                    rel == Ordering::Greater
                } else {
                    rel == Ordering::Less
                }
            })
        };
        if beats_start(entry, start) {
            start = Some(entry);
        }
    }
    if best.is_some() {
        return best;
    }
    // Wrap — but never "open" the very same path again (upstream viv.c:6086-6092).
    start.filter(|s| s.path != current.path)
}

/// The home/end entry — the global minimum (or maximum for `end`), current
/// included; re-opening the current entry is allowed (upstream `_viv_home`
/// playlist arm, viv.c:6156-6180, has no exclusion).
pub(crate) fn home(entries: &[PlaylistEntry], end: bool) -> Option<&PlaylistEntry> {
    let mut best: Option<&PlaylistEntry> = None;
    for entry in entries {
        let take = best
            .map(|b| {
                let cmp = fd_compare(entry, b);
                if end {
                    cmp == Ordering::Greater
                } else {
                    cmp == Ordering::Less
                }
            })
            .unwrap_or(true);
        if take {
            best = Some(entry);
        }
    }
    best
}

/// mtime in 100 ns ticks — FILETIME granularity without caring about the
/// epoch (every caller reads through `std::fs`).
pub(crate) fn modified_ticks(metadata: &Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64 / 100)
        .unwrap_or(0)
}

/// Recursively add a folder's images (upstream `_viv_playlist_add_path`,
/// viv.c:9539-9583): subfolders recurse (read_dir never yields `.`/`..`,
/// so the upstream guard is structural), files join the extension filter.
/// Runs synchronously on the UI thread like upstream's FindFirstFile walk.
pub(crate) fn add_path(playlist: &mut Playlist, dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return; // upstream's INVALID_HANDLE_VALUE arm: silently nothing
    };
    for entry in read.flatten() {
        // Windows DirEntry::metadata serves the find-data attributes and
        // timestamps with no extra syscall — the same bits upstream's
        // FindFirstData carried (directory reparse points included).
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if metadata.is_dir() {
            add_path(playlist, &path);
        } else if is_valid_path(path.as_os_str()) {
            playlist.add(path.into_os_string(), modified_ticks(&metadata));
        }
    }
}

/// Add one path — file or folder (upstream `_viv_playlist_add_filename`,
/// viv.c:9585-9611): folders recurse, files pass the extension filter,
/// unstatable paths add nothing.
pub(crate) fn add_filename(playlist: &mut Playlist, path: &Path) {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => add_path(playlist, path),
        Ok(metadata) => {
            if is_valid_path(path.as_os_str()) {
                playlist.add(
                    path.to_path_buf().into_os_string(),
                    modified_ticks(&metadata),
                );
            }
        }
        Err(_) => {}
    }
}

/// Expand a wildcard argument with FindFirstFileW (upstream's
/// GetFileAttributesEx-failed arm of `_viv_open_from_filename`,
/// viv.c:1396-1428): the system does the matching — DOS 8.3 quirks included
/// — and each match joins back onto the pattern's parent. Dirs vs files is
/// the caller's business (`add_filename` re-stats, same verdict).
fn expand_wildcard(pattern: &Path) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    let wide: Vec<u16> = pattern.as_os_str().encode_wide().chain([0]).collect();
    let mut data = WIN32_FIND_DATAW::default();
    // SAFETY: `wide` is NUL-terminated and outlives the call; `data` is a
    // valid out-pointer; the returned handle is checked and always closed.
    let handle = unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data) };
    let Ok(handle) = handle else {
        return matches; // no match: upstream's INVALID_HANDLE_VALUE arm
    };
    loop {
        let name_len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(0);
        let name = OsString::from_wide(&data.cFileName[..name_len]);
        if !name.is_empty() {
            matches.push(pattern.with_file_name(name));
        }
        // SAFETY: `handle` came from FindFirstFileW above and is not closed
        // yet; `data` is the same out-pointer the find protocol fills.
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    // SAFETY: closing exactly the FindFirstFileW handle we own, exactly once.
    let _ = unsafe { FindClose(handle) };
    matches
}

/// Expand a wildcard argument and add every match (upstream's
/// GetFileAttributesEx-failed arm of `_viv_open_from_filename`,
/// viv.c:1396-1428): matched folders recurse through `add_path`, but
/// matched FILES join UNFILTERED — the one entry path without the
/// extension check (unlike `add_path`/`add_filename`, viv.c:1413-1417).
/// Returns whether the pattern matched anything (upstream's FindFirstFile
/// handle validity deciding ret TRUE/FALSE).
pub(crate) fn add_expanded(playlist: &mut Playlist, pattern: &Path) -> bool {
    let matches = expand_wildcard(pattern);
    let found = !matches.is_empty();
    for path in matches {
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => add_path(playlist, &path),
            Ok(metadata) => {
                playlist.add(path.into_os_string(), modified_ticks(&metadata));
            }
            Err(_) => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(path: &str, modified: i64, id: u64) -> PlaylistEntry {
        PlaylistEntry {
            path: OsString::from(path),
            modified,
            id,
        }
    }

    // The raw sort order: mtime descending, name ascending, id ascending.
    #[test]
    fn newer_files_sort_before_older_ones() {
        let old = entry("a.png", 100, 1);
        let new = entry("z.png", 200, 2);
        assert_eq!(fd_compare(&new, &old), Ordering::Less);
        assert_eq!(fd_compare(&old, &new), Ordering::Greater);
    }

    #[test]
    fn same_mtime_falls_back_to_name_ascending() {
        let a = entry("b.png", 100, 1);
        let b = entry("a.png", 100, 2);
        assert_eq!(fd_compare(&b, &a), Ordering::Less); // "a" before "b"
    }

    // SORT_DIGITSASNUMBERS: "2" before "10" — the explorer-style order.
    #[test]
    fn equal_mtime_names_sort_digits_as_numbers() {
        let img2 = entry("img2.png", 100, 1);
        let img10 = entry("img10.png", 100, 2);
        assert_eq!(fd_compare(&img2, &img10), Ordering::Less);
    }

    #[test]
    fn name_compare_ignores_case_but_not_the_directory_part() {
        // Same filename part, case-folded: the collation ties and (with
        // equal ids) so does the full compare.
        let lower = entry("IMG.png", 100, 0);
        let upper = entry("img.PNG", 100, 0);
        assert_eq!(fd_compare(&lower, &upper), Ordering::Equal);
        // The directory part is NOT compared — different folders with the
        // same filename part tie on name and fall to the id tiebreak.
        let x = entry("C:\\x\\a.png", 100, 0);
        let y = entry("C:\\y\\a.png", 100, 1);
        assert_eq!(fd_compare(&x, &y), Ordering::Less); // insertion order
    }

    #[test]
    fn name_ties_fall_to_insertion_id() {
        let first = entry("x/a.png", 100, 1);
        let second = entry("y/a.png", 100, 2); // same filename part, other folder
        assert_eq!(fd_compare(&first, &second), Ordering::Less);
    }

    // The extension filter.
    #[test]
    fn valid_extensions_match_case_insensitively() {
        assert!(is_valid_path(OsStr::new("D:\\pics\\photo.PNG")));
        assert!(is_valid_path(OsStr::new("photo.jpeg")));
        assert!(is_valid_path(OsStr::new("a.tar.webp"))); // last dot wins
    }

    #[test]
    fn invalid_extensions_and_extensionless_paths_are_rejected() {
        assert!(!is_valid_path(OsStr::new("photo.txt")));
        assert!(!is_valid_path(OsStr::new("noext")));
        assert!(!is_valid_path(OsStr::new("trailingdot.")));
        assert!(!is_valid_path(OsStr::new("D:\\my.dir\\file"))); // dot in a folder name
    }

    #[test]
    fn leading_dot_file_with_extension_is_valid() {
        assert!(is_valid_path(OsStr::new(".png"))); // dot at index 0, tail "png"
    }

    // next(): the strictly-after entry wins over the wrap target. The sort
    // order is mtime DESCENDING — "after" = OLDER, so Right moves toward
    // older files (the default upstream nav sort, config.c:43-44).
    #[test]
    fn next_takes_the_immediately_following_entry() {
        // Sort order: new(300), mid(200), old(100).
        let old = entry("old.png", 100, 0);
        let mid = entry("mid.png", 200, 1);
        let new = entry("new.png", 300, 2);
        let entries = [old.clone(), mid.clone(), new.clone()];
        let cur = mid.clone();
        assert_eq!(
            next(&entries, Some(&cur), false, true).map(|e| e.path.clone()),
            Some(old.path.clone()) // next = the next-older file
        );
        assert_eq!(
            next(&entries, Some(&cur), true, true).map(|e| e.path.clone()),
            Some(new.path.clone()) // prev = the next-newer file
        );
    }

    #[test]
    fn next_wraps_at_both_ends() {
        let a = entry("a.png", 300, 0); // sort-first
        let b = entry("b.png", 200, 1);
        let c = entry("c.png", 100, 2); // sort-last
        let entries = [a.clone(), b.clone(), c.clone()];
        // next from the last wraps to the first...
        assert_eq!(
            next(&entries, Some(&c.clone()), false, true).map(|e| e.path.clone()),
            Some(a.path.clone())
        );
        // ...and prev from the first wraps to the last.
        assert_eq!(
            next(&entries, Some(&a.clone()), true, true).map(|e| e.path.clone()),
            Some(c.path.clone())
        );
    }

    #[test]
    fn single_entry_playlist_does_not_navigate() {
        let only = entry("only.png", 100, 0);
        let entries = [only.clone()];
        assert_eq!(next(&entries, Some(&only), false, true), None);
        assert_eq!(next(&entries, Some(&only), true, true), None);
    }

    // Duplicates: the best arm has NO same-path check — next from the
    // first duplicate reopens the same file through the second (upstream
    // viv.c:6079-6082); only the WRAP arm refuses to reopen the current
    // path (viv.c:6086-6092).
    #[test]
    fn next_from_a_duplicate_opens_the_other_duplicate() {
        let original = entry("dup.png", 100, 0);
        let duplicate = entry("dup.png", 100, 1);
        let other = entry("other.png", 100, 2); // sorts after "dup" (name asc)
        let entries = [original.clone(), duplicate.clone(), other.clone()];
        // next from `duplicate` (id 1): the strictly-after entry is `other`.
        assert_eq!(
            next(&entries, Some(&duplicate), false, true).map(|e| e.path.clone()),
            Some(other.path.clone())
        );
        // next from `original` (id 0): the duplicate compares strictly after
        // by the id tiebreak — it IS the best and is opened despite the
        // identical path (upstream behavior).
        assert_eq!(
            next(&entries, Some(&original), false, true).map(|e| e.path.clone()),
            Some(duplicate.path.clone())
        );
    }

    // Three duplicates, current = the last: no strictly-after entry, and
    // the wrap target carries the current path — navigation no-ops.
    #[test]
    fn wrap_target_refuses_to_reopen_the_current_path() {
        let dup0 = entry("dup.png", 100, 0);
        let dup1 = entry("dup.png", 100, 1);
        let dup2 = entry("dup.png", 100, 2);
        let entries = [dup0, dup1, dup2.clone()];
        assert_eq!(next(&entries, Some(&dup2), false, true), None);
    }

    // A direct open (the id-0 collision): the current matches the FIRST
    // playlist node by id even when it is a different file — upstream's
    // own quirk (`_viv_playlist_from_fd` has no non-zero guard).
    #[test]
    fn direct_open_current_collides_with_the_first_playlist_node() {
        let direct = entry("b.png", 200, 0);
        let entries = [
            entry("a.png", 300, 0),
            entry("b.png", 200, 1),
            entry("c.png", 100, 2),
        ];
        // Node a (id 0) is excluded as "the current node" even though the
        // current is b. The current's own playlist node (id 1) then compares
        // strictly-after via the id tiebreak and IS the best — next reopens
        // the same file through it (upstream opens the best unconditionally).
        assert_eq!(
            next(&entries, Some(&direct), false, true).map(|e| e.path.clone()),
            Some(OsString::from("b.png"))
        );
        // prev: a is wrongly excluded, so the wrap target is the sort-max c.
        assert_eq!(
            next(&entries, Some(&direct), true, true).map(|e| e.path.clone()),
            Some(OsString::from("c.png"))
        );
    }

    // The folder-scan arm: entries built with id 0, NO node exclusion —
    // the current file's own scan copy competes and only loses to the
    // compare-equal and same-path checks.
    #[test]
    fn folder_scan_has_no_node_exclusion() {
        let entries = [
            entry("a.png", 300, 0),
            entry("b.png", 200, 0),
            entry("c.png", 100, 0),
        ];
        let current = entry("b.png", 200, 0); // a scan/direct current, id 0
        assert_eq!(
            next(&entries, Some(&current), false, false).map(|e| e.path.clone()),
            Some(OsString::from("c.png"))
        );
        assert_eq!(
            next(&entries, Some(&current), true, false).map(|e| e.path.clone()),
            Some(OsString::from("a.png"))
        );
    }

    // A folder holding only the current image: next finds no strictly-after
    // entry and the wrap target IS the current file — a no-op (upstream's
    // "don't open the same image again", viv.c:6086-6092).
    #[test]
    fn folder_scan_with_a_single_image_does_not_navigate() {
        let only = entry("only.png", 100, 0);
        let entries = [only.clone()];
        assert_eq!(next(&entries, Some(&only), false, false), None);
        assert_eq!(next(&entries, Some(&only), true, false), None);
    }

    // home()/end(): the global edges, current included.
    #[test]
    fn home_picks_the_sort_extremes() {
        let a = entry("a.png", 300, 1); // sort-first (newest)
        let b = entry("b.png", 200, 2);
        let c = entry("c.png", 100, 3); // sort-last
        let entries = [b.clone(), c.clone(), a.clone()];
        assert_eq!(
            home(&entries, false).map(|e| e.path.clone()),
            Some(a.path.clone())
        );
        assert_eq!(
            home(&entries, true).map(|e| e.path.clone()),
            Some(c.path.clone())
        );
    }

    #[test]
    fn home_on_an_empty_playlist_finds_nothing() {
        assert_eq!(home(&[], false), None);
        assert_eq!(home(&[], true), None);
    }

    // Playlist bookkeeping: ids run 0,1,2,... and clear resets the counter —
    // the first entry after a clear is id 0, the id a direct open carries
    // (upstream viv.c:661/9478-9480/9380).
    #[test]
    fn ids_increment_and_reset_on_clear() {
        let mut playlist = Playlist::new();
        assert!(playlist.is_empty());
        assert_eq!(playlist.add(OsString::from("a.png"), 1).id, 0);
        assert_eq!(playlist.add(OsString::from("b.png"), 2).id, 1);
        playlist.clear();
        assert!(playlist.is_empty());
        assert_eq!(playlist.add(OsString::from("c.png"), 3).id, 0); // counter reset
        assert_eq!(
            playlist.first().map(|e| e.path.clone()),
            Some(OsString::from("c.png"))
        );
    }

    // The recursive folder scan: subfolders, invalid extensions, ordering.
    #[test]
    fn add_path_walks_subfolders_and_filters_extensions() {
        let root = std::env::temp_dir().join(format!("riviv-pl-{}", std::process::id()));
        let sub = root.join("sub");
        let deep = sub.join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        let files = [
            root.join("a.png"),
            root.join("b.txt"), // wrong extension
            root.join("c.PNG"), // case-insensitive match
            sub.join("d.jpg"),
            deep.join("e.webp"),
            deep.join("noext"), // extensionless
        ];
        let mtime = |secs: u64| {
            use std::time::{Duration, SystemTime};
            SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
        };
        for (i, file) in files.iter().enumerate() {
            std::fs::write(file, b"x").unwrap();
            std::fs::File::options()
                .write(true)
                .open(file)
                .unwrap()
                .set_modified(mtime(10_000 + i as u64))
                .unwrap();
        }
        let mut playlist = Playlist::new();
        add_path(&mut playlist, &root);
        let mut paths: Vec<OsString> = playlist.entries().iter().map(|e| e.path.clone()).collect();
        paths.sort();
        let mut expected: Vec<OsString> = vec![
            root.join("a.png").into_os_string(),
            root.join("c.PNG").into_os_string(),
            sub.join("d.jpg").into_os_string(),
            deep.join("e.webp").into_os_string(),
        ];
        expected.sort();
        assert_eq!(paths, expected);
        // mtimes ride along from the same tick model navigation compares.
        let e_webp = playlist
            .entries()
            .iter()
            .find(|e| e.path == deep.join("e.webp").into_os_string())
            .unwrap();
        assert_eq!(e_webp.modified, 10_004_i64 * 10_000_000);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn add_filename_adds_files_and_recurses_folders() {
        let root = std::env::temp_dir().join(format!("riviv-plf-{}", std::process::id()));
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("dir").join("x.gif"), b"x").unwrap();
        std::fs::write(root.join("y.tif"), b"x").unwrap();
        std::fs::write(root.join("z.doc"), b"x").unwrap();
        let mut playlist = Playlist::new();
        add_filename(&mut playlist, &root.join("y.tif"));
        add_filename(&mut playlist, &root.join("z.doc"));
        add_filename(&mut playlist, &root.join("dir"));
        add_filename(&mut playlist, &root.join("missing.png")); // silently nothing
        let mut paths: Vec<OsString> = playlist.entries().iter().map(|e| e.path.clone()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                root.join("dir").join("x.gif").into_os_string(),
                root.join("y.tif").into_os_string(),
            ]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // Wildcards.
    #[test]
    fn wildcard_expansion_joins_matches_onto_the_parent() {
        let root = std::env::temp_dir().join(format!("riviv-plw-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("one.png"), b"x").unwrap();
        std::fs::write(root.join("two.png"), b"x").unwrap();
        std::fs::write(root.join("three.txt"), b"x").unwrap();
        let pattern = root.join("*.png");
        let mut paths: Vec<PathBuf> = expand_wildcard(pattern.as_path());
        paths.sort();
        assert_eq!(paths, vec![root.join("one.png"), root.join("two.png")]);
        assert!(expand_wildcard(root.join("*.nomatch").as_path()).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
