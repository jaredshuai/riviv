//! Playlist model and navigation math (#6; upstream playlist globals +
//! `_viv_playlist_*` / `_viv_next` / `_viv_home`, viv.c:9341-9611 / 5817-6263).
//!
//! Upstream's playlist is a linked list of `WIN32_FIND_DATA`s whose
//! `cFileName` holds the FULL path; navigation never re-orders it — every
//! key press re-scans the entries with `_viv_fd_compare` and picks the
//! smallest entry strictly after the current one (wrapping to the global
//! smallest). Since #39 the compare is parametrized by the config sort
//! (`sort` / `sort_ascending`, config.c:43-44 — the default stays
//! DATE_MODIFIED + descending, which lands on: mtime descending, filename
//! ascending, insertion id ascending); with `shuffle` on (#39) navigation
//! instead walks a lazily built shuffle index array (`_viv_playlist_
//! shuffle_indexes`, viv.c:12816-12871).
//!
//! The pure half (compare / next / home / shuffle / extension check) is
//! unit-tested; the FS half (recursive folder scan) mirrors upstream's
//! synchronous FindFirstFile walk, run on the UI thread exactly like
//! upstream — a huge dropped tree freezes the window in upstream too.

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

/// One navigable image. `modified`/`created` are the file's mtime/ctime in
/// 100 ns ticks since an arbitrary fixed epoch — only the relative order
/// ever matters, and every entry (playlist scan, folder scan, direct open)
/// reads them from `std::fs` so the epochs agree.
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
    pub(crate) created: i64,
    pub(crate) size: u64,
    pub(crate) id: u64,
}

/// The navigation sort key (upstream `CONFIG_NAV_SORT_*`, config.h:30-34;
/// the ini `sort` key, config.c:116). The discriminants are the wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortMode {
    /// `CONFIG_NAV_SORT_NAME` = 0 — the filename part only.
    Name,
    /// `CONFIG_NAV_SORT_SIZE` = 1.
    Size,
    /// `CONFIG_NAV_SORT_DATE_MODIFIED` = 2 (the shipped default).
    DateModified,
    /// `CONFIG_NAV_SORT_DATE_CREATED` = 3.
    DateCreated,
    /// `CONFIG_NAV_SORT_FULL_PATH_AND_FILENAME` = 4 — the whole path.
    FullPath,
    /// A hand-edited ini value outside 0-4. Upstream's compare switch has
    /// no default (viv.c:5713-5807), so everything compares EQUAL:
    /// navigation finds no candidate and no-ops, and none of the five sort
    /// radios checks (the CheckMenuItem compares all fail, viv.c:7188-7192).
    Unknown,
}

impl SortMode {
    /// The config value back (BYTE-truncated by the loader already).
    pub(crate) fn from_config(value: i32) -> SortMode {
        match value {
            0 => SortMode::Name,
            1 => SortMode::Size,
            2 => SortMode::DateModified,
            3 => SortMode::DateCreated,
            4 => SortMode::FullPath,
            _ => SortMode::Unknown,
        }
    }
}

/// The direction a sort-mode menu click lands on (upstream's handler,
/// viv.c:1762-1788): picking a DIFFERENT mode resets to that mode's
/// default direction — Name/Full Path check ascending, the two dates and
/// Size check descending.
pub(crate) fn default_ascending(mode: SortMode) -> bool {
    matches!(mode, SortMode::Name | SortMode::FullPath)
}

/// A click on a sort-mode row (upstream viv.c:1756-1789): clicking the
/// already-active mode TOGGLES the direction; picking another mode selects
/// it with its default direction.
pub(crate) fn apply_sort_click(
    mode: SortMode,
    current: SortMode,
    current_ascending: bool,
) -> (SortMode, bool) {
    if mode == current {
        (mode, !current_ascending)
    } else {
        (mode, default_ascending(mode))
    }
}

/// The playlist (upstream `_viv_playlist_start/_last/_count/_viv_playlist_id`).
/// Insertion order is preserved; navigation sorts on the fly. The shuffle
/// visit order (#39; upstream `_viv_playlist_shuffle_indexes` +
/// `_viv_playlist_shuffle_allocated`, viv.c:12816-12871) is a lazily built
/// array of entry indices — `None` until shuffle navigation first needs it,
/// dropped when shuffle turns off or the playlist clears (viv.c:1729-1736 /
/// 9369-9375).
pub(crate) struct Playlist {
    entries: Vec<PlaylistEntry>,
    next_id: u64,
    shuffle_order: Option<Vec<u32>>,
    /// xorshift64* state behind the shuffle draws (upstream: CRT `rand()`
    /// seeded from QueryPerformanceCounter at each full shuffle, viv.c:
    /// 12821-12826). 0 = not seeded; `ensure_shuffle` re-seeds.
    rng_state: u64,
}

impl Playlist {
    pub(crate) fn new() -> Self {
        Playlist {
            entries: Vec::new(),
            next_id: 0,
            shuffle_order: None,
            rng_state: 0,
        }
    }

    /// Upstream `_viv_playlist_clearall` — also resets the id counter
    /// (viv.c:9380) and frees the shuffle index array (viv.c:9369-9375), so
    /// ids restart at 0 after a replace-drop (the first entry post-clear is
    /// id 0, the id a direct open carries).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.next_id = 0;
        self.shuffle_order = None;
        self.rng_state = 0;
    }

    /// Append an entry, assigning the next insertion id (upstream
    /// `_viv_playlist_add`, viv.c:9471-9537: assign-then-increment from the
    /// counter `clear` zeroed — first entry after a clear is id 0). With a
    /// shuffle order already built, the new index joins at a RANDOM
    /// position of the order (viv.c:9496-9531: the displaced slot's tail
    /// trick over `rand() % (count + 1)`).
    pub(crate) fn add(
        &mut self,
        path: OsString,
        modified: i64,
        created: i64,
        size: u64,
    ) -> &PlaylistEntry {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(PlaylistEntry {
            path,
            modified,
            created,
            size,
            id,
        });
        if let Some(order) = &mut self.shuffle_order {
            let len = order.len();
            let new_index = (self.entries.len() - 1) as u32;
            let at = next_rand(&mut self.rng_state, len + 1);
            if at < len {
                let displaced = order[at];
                order.push(displaced);
                order[at] = new_index;
            } else {
                order.push(new_index);
            }
        }
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

    /// Build the shuffle order if absent (upstream `_viv_shuffle_playlist`,
    /// viv.c:12816-12871: Fisher-Yates `j = i + rand % (count - i)`, seeded
    /// from QueryPerformanceCounter — the shell passes that as `seed`). An
    /// existing order is kept (the `_viv_playlist_start && !indexes` guard
    /// of `_viv_do_initial_shuffle`, viv.c:13582); an empty playlist has
    /// nothing to order (viv.c:12818's count check).
    pub(crate) fn ensure_shuffle(&mut self, seed: u64) {
        if self.entries.is_empty() || self.shuffle_order.is_some() {
            return;
        }
        self.rng_state = seed | 1; // xorshift state must be nonzero
        let count = self.entries.len();
        let mut order: Vec<u32> = (0..count as u32).collect();
        for i in 0..count - 1 {
            let j = i + next_rand(&mut self.rng_state, count - i);
            order.swap(i, j);
        }
        self.shuffle_order = Some(order);
    }

    /// Shuffle turned off (upstream frees the index array in the handler,
    /// viv.c:1726-1736) — turning shuffle back on re-shuffles fresh
    /// (`ensure_shuffle` rebuilds, matching `_viv_do_initial_shuffle`'s
    /// `!indexes` guard).
    pub(crate) fn drop_shuffle(&mut self) {
        self.shuffle_order = None;
        self.rng_state = 0;
    }

    /// The shuffle-order neighbor of `current` (upstream `_viv_next`'s
    /// shuffle arm, viv.c:5881-5923): the FIRST order slot whose entry id
    /// equals the current's (`_viv_playlist_shuffle_index_from_fd`,
    /// viv.c:13527-13546 — the id-0 collision picks the first match, an
    /// upstream quirk), stepped and wrapped; a current matching nothing
    /// starts at the order's front (back for `prev`). The array wrap needs
    /// no same-path guard — every slot is a distinct entry.
    pub(crate) fn shuffle_target(
        &self,
        current: &PlaylistEntry,
        prev: bool,
    ) -> Option<&PlaylistEntry> {
        let order = self.shuffle_order.as_ref()?;
        let count = order.len();
        if count == 0 {
            return None;
        }
        let found = order
            .iter()
            .position(|&i| self.entries[i as usize].id == current.id);
        let index = match found {
            Some(i) => {
                if prev {
                    (i + count - 1) % count
                } else {
                    (i + 1) % count
                }
            }
            // Not in the order: front for next, back for prev (viv.c:5908-5918).
            None => {
                if prev {
                    count - 1
                } else {
                    0
                }
            }
        };
        self.entries.get(order[index] as usize)
    }

    /// The shuffle-order edge for Home/End (upstream `_viv_home`'s shuffle
    /// arm, viv.c:6137-6152): slot 0 for Home, the last slot for End.
    pub(crate) fn shuffle_edge(&self, end: bool) -> Option<&PlaylistEntry> {
        let order = self.shuffle_order.as_ref()?;
        let &index = if end { order.last()? } else { order.first()? };
        self.entries.get(index as usize)
    }
}

/// One xorshift64* draw reduced into `[0, modulus)` (the shuffle's
/// randomness source; any full-period PRNG matches upstream's CRT `rand()`
/// behaviorally — the order is random either way).
fn next_rand(state: &mut u64, modulus: usize) -> usize {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    let value = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (value % modulus as u64) as usize
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

/// The collation flags of upstream's string compares (viv.c:5655-5661):
/// user locale, case-insensitive, string-sort, digits-as-numbers (the Win7+
/// flag upstream gates on; every supported Windows has it).
const NAME_SORT_FLAGS: u32 = NORM_IGNORECASE.0 | SORT_STRINGSORT.0 | SORT_DIGITSASNUMBERS.0;

/// Filename part of a full path — everything after the last `\`
/// (upstream `string_get_filename_part`; it does not treat `/` as a
/// separator, and neither do we).
pub(crate) fn filename_part(path: &OsStr) -> Vec<u16> {
    let wide = path.encode_wide().collect::<Vec<u16>>();
    match wide.iter().rposition(|&c| c == u16::from(b'\\')) {
        Some(sep) => wide[sep + 1..].to_vec(),
        None => wide,
    }
}

/// Locale collation of two strings (upstream's CompareString call,
/// viv.c:5666/5693). A CompareStringW failure (0) falls through to
/// `Equal`, exactly like upstream's switch default, so the caller's
/// tiebreak decides.
fn collate(a: &[u16], b: &[u16]) -> Ordering {
    // SAFETY: read-only collation query over two valid u16 slices whose
    // lengths the wrapper passes explicitly; the result is a value code.
    let ret = unsafe { CompareStringW(LOCALE_USER_DEFAULT, NAME_SORT_FLAGS, a, b) };
    match ret {
        CSTR_LESS_THAN => Ordering::Less,
        CSTR_GREATER_THAN => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

/// Filename-part collation (upstream `_viv_fd_compare_name`, viv.c:5648-
/// 5678) — the Jump To list's sort and every sort mode's tiebreak.
fn compare_name(a: &OsStr, b: &OsStr) -> Ordering {
    collate(&filename_part(a), &filename_part(b))
}

/// Whole-path collation (upstream `_viv_fd_compare_path_and_name`,
/// viv.c:5680-5705 — same flags over the full `cFileName`, no filename
/// extraction). Exposed for the Jump To dialog's name-ascending item sort
/// through [`compare_name`]'s flags.
fn compare_full_path(a: &OsStr, b: &OsStr) -> Ordering {
    let a = a.encode_wide().collect::<Vec<u16>>();
    let b = b.encode_wide().collect::<Vec<u16>>();
    collate(&a, &b)
}

/// The Jump To item order (#39; upstream `_viv_nav_compare`, viv.c:13324:
/// literally `_viv_fd_compare_name` — filename collation with the
/// insertion-id tiebreak, ALWAYS, regardless of the config sort).
pub(crate) fn nav_compare(a: &PlaylistEntry, b: &PlaylistEntry) -> Ordering {
    compare_name(&a.path, &b.path).then_with(|| a.id.cmp(&b.id))
}

/// The navigation order (upstream `_viv_fd_compare`, viv.c:5707-5815):
/// the configured key first; per-mode tiebreaks — Name and Full Path fall
/// to the insertion id (`_viv_compare_id`, viv.c:5623-5646), Size and the
/// two dates fall to the NEGATED name compare ("we want name ascending
/// when we are size descending", viv.c:5745) whose own id fallback rides
/// the same negation (`_viv_fd_compare_name` ends in `_viv_compare_id`,
/// so under the descending flip the tie group reads name ASCENDING then
/// id ASCENDING) — and the final direction flip for descending
/// (viv.c:5809-5812). An unknown config value compares everything EQUAL
/// (upstream's switch has no default) so navigation no-ops.
pub(crate) fn fd_compare(
    a: &PlaylistEntry,
    b: &PlaylistEntry,
    mode: SortMode,
    ascending: bool,
) -> Ordering {
    /// The Size/Date tie group: `-_viv_fd_compare_name` — the filename
    /// collation with its id fallback, negated as one unit.
    fn negated_name_then_id(a: &PlaylistEntry, b: &PlaylistEntry) -> Ordering {
        compare_name(&a.path, &b.path)
            .then_with(|| a.id.cmp(&b.id))
            .reverse()
    }
    let raw = match mode {
        SortMode::Name => compare_name(&a.path, &b.path).then_with(|| a.id.cmp(&b.id)),
        SortMode::FullPath => compare_full_path(&a.path, &b.path).then_with(|| a.id.cmp(&b.id)),
        SortMode::Size => a.size.cmp(&b.size).then_with(|| negated_name_then_id(a, b)),
        SortMode::DateModified => a
            .modified
            .cmp(&b.modified)
            .then_with(|| negated_name_then_id(a, b)),
        SortMode::DateCreated => a
            .created
            .cmp(&b.created)
            .then_with(|| negated_name_then_id(a, b)),
        SortMode::Unknown => Ordering::Equal,
    };
    if ascending { raw } else { raw.reverse() }
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
/// same-path checks. The compare itself is the caller's sort config
/// (`mode`/`ascending`, #39).
pub(crate) fn next<'a>(
    entries: &'a [PlaylistEntry],
    current: Option<&PlaylistEntry>,
    prev: bool,
    from_playlist: bool,
    mode: SortMode,
    ascending: bool,
) -> Option<&'a PlaylistEntry> {
    let current = current?;
    let mut best: Option<&PlaylistEntry> = None;
    let mut start: Option<&PlaylistEntry> = None;
    for entry in entries {
        if from_playlist && entry.id == current.id {
            continue; // the node `current` is (plain id equality, viv.c:5928-5933)
        }
        let cmp = fd_compare(entry, current, mode, ascending);
        // best: strictly after (before, for prev) the current, and closer to
        // it than the best so far (upstream viv.c:5937-5962).
        let beats_best = |probe: &PlaylistEntry, best: Option<&PlaylistEntry>| {
            best.is_none_or(|b| {
                let rel = fd_compare(probe, b, mode, ascending);
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
                let rel = fd_compare(probe, s, mode, ascending);
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

/// The home/end entry — the global minimum (or maximum for `end`) under
/// the caller's sort, current included; re-opening the current entry is
/// allowed (upstream `_viv_home` playlist arm, viv.c:6156-6180, has no
/// exclusion).
pub(crate) fn home(
    entries: &[PlaylistEntry],
    end: bool,
    mode: SortMode,
    ascending: bool,
) -> Option<&PlaylistEntry> {
    let mut best: Option<&PlaylistEntry> = None;
    for entry in entries {
        let take = best
            .map(|b| {
                let cmp = fd_compare(entry, b, mode, ascending);
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

/// mtime in 100 ns ticks, signed like the raw FILETIME upstream compares
/// (viv.c:5757-5771): pre-1970 timestamps (zeroed/invalid FILETIMEs read
/// as 1601) stay NEGATIVE instead of collapsing to a tie at 0, keeping the
/// date-modified order intact.
pub(crate) fn modified_ticks(metadata: &Metadata) -> i64 {
    filetime_ticks(metadata.modified())
}

/// ctime on the same tick scale (the Date Created sort key, viv.c:5780-
/// 5806): an unavailable creation time reads 0, tying at the epoch's
/// bottom like upstream's zeroed FILETIME.
pub(crate) fn created_ticks(metadata: &Metadata) -> i64 {
    filetime_ticks(metadata.created())
}

fn filetime_ticks(time: std::io::Result<std::time::SystemTime>) -> i64 {
    time.ok()
        .map_or(0, |t| match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i64 / 100,
            Err(e) => -(e.duration().as_nanos() as i64 / 100),
        })
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
            playlist.add(
                path.into_os_string(),
                modified_ticks(&metadata),
                created_ticks(&metadata),
                metadata.len(),
            );
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
                    created_ticks(&metadata),
                    metadata.len(),
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
                playlist.add(
                    path.into_os_string(),
                    modified_ticks(&metadata),
                    created_ticks(&metadata),
                    metadata.len(),
                );
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
        entry_full(path, modified, 0, 0, id)
    }

    fn entry_full(path: &str, modified: i64, created: i64, size: u64, id: u64) -> PlaylistEntry {
        PlaylistEntry {
            path: OsString::from(path),
            modified,
            created,
            size,
            id,
        }
    }

    /// The shipped default sort (`DATE_MODIFIED` + descending, config.c:
    /// 43-44) — what the pre-#39 callers hardcoded.
    fn cmp(a: &PlaylistEntry, b: &PlaylistEntry) -> Ordering {
        fd_compare(a, b, SortMode::DateModified, false)
    }

    fn nxt<'a>(
        entries: &'a [PlaylistEntry],
        current: Option<&PlaylistEntry>,
        prev: bool,
        from_playlist: bool,
    ) -> Option<&'a PlaylistEntry> {
        next(
            entries,
            current,
            prev,
            from_playlist,
            SortMode::DateModified,
            false,
        )
    }

    fn hm(entries: &[PlaylistEntry], end: bool) -> Option<&PlaylistEntry> {
        home(entries, end, SortMode::DateModified, false)
    }

    // The raw sort order: mtime descending, name ascending, id ascending.
    #[test]
    fn newer_files_sort_before_older_ones() {
        let old = entry("a.png", 100, 1);
        let new = entry("z.png", 200, 2);
        assert_eq!(cmp(&new, &old), Ordering::Less);
        assert_eq!(cmp(&old, &new), Ordering::Greater);
    }

    #[test]
    fn same_mtime_falls_back_to_name_ascending() {
        let a = entry("b.png", 100, 1);
        let b = entry("a.png", 100, 2);
        assert_eq!(cmp(&b, &a), Ordering::Less); // "a" before "b"
    }

    // SORT_DIGITSASNUMBERS: "2" before "10" — the explorer-style order.
    #[test]
    fn equal_mtime_names_sort_digits_as_numbers() {
        let img2 = entry("img2.png", 100, 1);
        let img10 = entry("img10.png", 100, 2);
        assert_eq!(cmp(&img2, &img10), Ordering::Less);
    }

    #[test]
    fn name_compare_ignores_case_but_not_the_directory_part() {
        // Same filename part, case-folded: the collation ties and (with
        // equal ids) so does the full compare.
        let lower = entry("IMG.png", 100, 0);
        let upper = entry("img.PNG", 100, 0);
        assert_eq!(cmp(&lower, &upper), Ordering::Equal);
        // The directory part is NOT compared — different folders with the
        // same filename part tie on name and fall to the id tiebreak.
        let x = entry("C:\\x\\a.png", 100, 0);
        let y = entry("C:\\y\\a.png", 100, 1);
        assert_eq!(cmp(&x, &y), Ordering::Less); // insertion order
    }

    #[test]
    fn name_ties_fall_to_insertion_id() {
        let first = entry("x/a.png", 100, 1);
        let second = entry("y/a.png", 100, 2); // same filename part, other folder
        assert_eq!(cmp(&first, &second), Ordering::Less);
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
            nxt(&entries, Some(&cur), false, true).map(|e| e.path.clone()),
            Some(old.path.clone()) // next = the next-older file
        );
        assert_eq!(
            nxt(&entries, Some(&cur), true, true).map(|e| e.path.clone()),
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
            nxt(&entries, Some(&c.clone()), false, true).map(|e| e.path.clone()),
            Some(a.path.clone())
        );
        // ...and prev from the first wraps to the last.
        assert_eq!(
            nxt(&entries, Some(&a.clone()), true, true).map(|e| e.path.clone()),
            Some(c.path.clone())
        );
    }

    #[test]
    fn single_entry_playlist_does_not_navigate() {
        let only = entry("only.png", 100, 0);
        let entries = [only.clone()];
        assert_eq!(nxt(&entries, Some(&only), false, true), None);
        assert_eq!(nxt(&entries, Some(&only), true, true), None);
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
            nxt(&entries, Some(&duplicate), false, true).map(|e| e.path.clone()),
            Some(other.path.clone())
        );
        // next from `original` (id 0): the duplicate compares strictly after
        // by the id tiebreak — it IS the best and is opened despite the
        // identical path (upstream behavior).
        assert_eq!(
            nxt(&entries, Some(&original), false, true).map(|e| e.path.clone()),
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
        assert_eq!(nxt(&entries, Some(&dup2), false, true), None);
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
            nxt(&entries, Some(&direct), false, true).map(|e| e.path.clone()),
            Some(OsString::from("b.png"))
        );
        // prev: a is wrongly excluded, so the wrap target is the sort-max c.
        assert_eq!(
            nxt(&entries, Some(&direct), true, true).map(|e| e.path.clone()),
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
            nxt(&entries, Some(&current), false, false).map(|e| e.path.clone()),
            Some(OsString::from("c.png"))
        );
        assert_eq!(
            nxt(&entries, Some(&current), true, false).map(|e| e.path.clone()),
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
        assert_eq!(nxt(&entries, Some(&only), false, false), None);
        assert_eq!(nxt(&entries, Some(&only), true, false), None);
    }

    // home()/end(): the global edges, current included.
    #[test]
    fn home_picks_the_sort_extremes() {
        let a = entry("a.png", 300, 1); // sort-first (newest)
        let b = entry("b.png", 200, 2);
        let c = entry("c.png", 100, 3); // sort-last
        let entries = [b.clone(), c.clone(), a.clone()];
        assert_eq!(
            hm(&entries, false).map(|e| e.path.clone()),
            Some(a.path.clone())
        );
        assert_eq!(
            hm(&entries, true).map(|e| e.path.clone()),
            Some(c.path.clone())
        );
    }

    #[test]
    fn home_on_an_empty_playlist_finds_nothing() {
        assert_eq!(hm(&[], false), None);
        assert_eq!(hm(&[], true), None);
    }

    // Playlist bookkeeping: ids run 0,1,2,... and clear resets the counter —
    // the first entry after a clear is id 0, the id a direct open carries
    // (upstream viv.c:661/9478-9480/9380).
    #[test]
    fn ids_increment_and_reset_on_clear() {
        let mut playlist = Playlist::new();
        assert!(playlist.is_empty());
        assert_eq!(playlist.add(OsString::from("a.png"), 1, 0, 0).id, 0);
        assert_eq!(playlist.add(OsString::from("b.png"), 2, 0, 0).id, 1);
        playlist.clear();
        assert!(playlist.is_empty());
        assert_eq!(playlist.add(OsString::from("c.png"), 3, 0, 0).id, 0); // counter reset
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

    // Pre-1970 mtimes (zeroed/invalid FILETIMEs read as 1601) stay
    // NEGATIVE ticks — the date-modified order survives instead of every
    // pre-epoch file collapsing into one tie at 0 (cubic PR #14).
    #[test]
    fn pre_epoch_mtimes_keep_their_order() {
        use std::time::{Duration, SystemTime};
        let root = std::env::temp_dir().join(format!("riviv-plp-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let ancient = root.join("ancient.png");
        let modern = root.join("modern.png");
        for (file, when) in [
            (
                &ancient,
                SystemTime::UNIX_EPOCH - Duration::from_secs(63_072_000),
            ), // 1968
            (
                &modern,
                SystemTime::UNIX_EPOCH + Duration::from_secs(63_072_000),
            ), // 1972
        ] {
            std::fs::write(file, b"x").unwrap();
            std::fs::File::options()
                .write(true)
                .open(file)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let mut playlist = Playlist::new();
        add_filename(&mut playlist, &ancient);
        add_filename(&mut playlist, &modern);
        let entries = playlist.entries();
        let ancient = entries
            .iter()
            .find(|e| e.path == root.join("ancient.png"))
            .unwrap();
        let modern = entries
            .iter()
            .find(|e| e.path == root.join("modern.png"))
            .unwrap();
        assert!(ancient.modified < 0, "pre-epoch ticks must be negative");
        assert_eq!(cmp(modern, ancient), Ordering::Less); // newer sorts first
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

    // ---- #39: the sort modes ----

    // The mode table round-trips the config wire values 0-4 and parks
    // garbage elsewhere (upstream's switch has no default — an unknown
    // value compares everything equal, viv.c:5713-5807).
    #[test]
    fn sort_mode_round_trips_the_config_values() {
        for (value, mode) in [
            (0, SortMode::Name),
            (1, SortMode::Size),
            (2, SortMode::DateModified),
            (3, SortMode::DateCreated),
            (4, SortMode::FullPath),
        ] {
            assert_eq!(SortMode::from_config(value), mode);
        }
        for garbage in [5, 200, -1, 256] {
            assert_eq!(SortMode::from_config(garbage), SortMode::Unknown);
        }
    }

    #[test]
    fn unknown_sort_values_compare_everything_equal() {
        let a = entry("a.png", 100, 0);
        let b = entry("z.png", 999_999, 7);
        assert_eq!(fd_compare(&a, &b, SortMode::Unknown, true), Ordering::Equal);
        assert_eq!(
            fd_compare(&b, &a, SortMode::Unknown, false),
            Ordering::Equal
        );
    }

    // Name sort: filename parts only — different folders with the same
    // filename part tie and fall to insertion id (upstream
    // _viv_fd_compare_name + _viv_compare_id, viv.c:5648-5678).
    #[test]
    fn name_sort_compares_filename_parts_only() {
        let a = entry("C:\\a\\b.png", 100, 0);
        let b = entry("C:\\z\\a.png", 100, 1);
        assert_eq!(fd_compare(&a, &b, SortMode::Name, true), Ordering::Greater); // "b" > "a" regardless of folders
        assert_eq!(fd_compare(&a, &b, SortMode::Name, false), Ordering::Less);
        // Same filename part in two folders: the id tiebreak, not the path.
        let x = entry("C:\\z\\a.png", 100, 1);
        let y = entry("C:\\a\\a.png", 100, 0);
        assert_eq!(fd_compare(&x, &y, SortMode::Name, true), Ordering::Greater); // id 1 after id 0
    }

    // Full Path sort: the whole path collation — the folder prefix decides
    // before the filename (upstream _viv_fd_compare_path_and_name,
    // viv.c:5680-5705).
    #[test]
    fn full_path_sort_compares_the_whole_path() {
        let a = entry("C:\\a\\z.png", 100, 0);
        let b = entry("C:\\z\\a.png", 100, 1);
        assert_eq!(fd_compare(&a, &b, SortMode::FullPath, true), Ordering::Less); // "C:\a\..." < "C:\z\..."
    }

    // Size/Date sorts tie-break on the NEGATED name compare (viv.c:5745:
    // "we want name ascending when we are size descending") — under the
    // final descending flip the tie group reads name ASCENDING; under
    // ascending it reads name DESCENDING (the negation applies before the
    // direction flip, to the whole name+id group).
    #[test]
    fn size_sort_descending_ties_break_name_ascending() {
        let big_b = entry_full("b.png", 0, 0, 100, 0);
        let big_a = entry_full("a.png", 0, 0, 100, 1);
        // Equal sizes, descending: "a" before "b".
        assert_eq!(
            fd_compare(&big_b, &big_a, SortMode::Size, false),
            Ordering::Greater
        );
        assert_eq!(
            fd_compare(&big_a, &big_b, SortMode::Size, false),
            Ordering::Less
        );
        // Smaller file sorts later under descending.
        let small = entry_full("z.png", 0, 0, 50, 2);
        assert_eq!(
            fd_compare(&small, &big_a, SortMode::Size, false),
            Ordering::Greater
        );
        // Ascending flips the key AND the tie group: equal sizes now tie
        // name DESCENDING — "b" before "a".
        assert_eq!(
            fd_compare(&big_a, &big_b, SortMode::Size, true),
            Ordering::Greater
        );
        assert_eq!(
            fd_compare(&big_b, &big_a, SortMode::Size, true),
            Ordering::Less
        );
    }

    #[test]
    fn date_created_sort_uses_the_created_ticks() {
        let older = entry_full("old.png", 9_999, 100, 0, 0);
        let newer = entry_full("new.png", 0, 200, 0, 1);
        // mtime is irrelevant under Date Created (and differs deliberately).
        assert_eq!(
            fd_compare(&newer, &older, SortMode::DateCreated, true),
            Ordering::Greater
        );
        assert_eq!(
            fd_compare(&newer, &older, SortMode::DateCreated, false),
            Ordering::Less
        );
        // The default mode (DateModified, descending) orders the other way
        // round — old.png has the newer mtime.
        assert_eq!(cmp(&newer, &older), Ordering::Greater);
    }

    // The mode-click policy (upstream's handler, viv.c:1756-1789).
    #[test]
    fn sort_clicks_toggle_direction_on_the_same_mode() {
        // Same mode: flip.
        assert_eq!(
            apply_sort_click(SortMode::Name, SortMode::Name, true),
            (SortMode::Name, false)
        );
        assert_eq!(
            apply_sort_click(SortMode::Name, SortMode::Name, false),
            (SortMode::Name, true)
        );
    }

    #[test]
    fn sort_clicks_pick_the_modes_default_direction() {
        // Name / Full Path default ascending; Size and both dates default
        // descending (viv.c:1764-1787).
        for (mode, asc) in [
            (SortMode::Name, true),
            (SortMode::FullPath, true),
            (SortMode::Size, false),
            (SortMode::DateModified, false),
            (SortMode::DateCreated, false),
        ] {
            assert_eq!(
                apply_sort_click(mode, SortMode::Unknown, false),
                (mode, asc),
                "{mode:?}"
            );
            assert_eq!(
                apply_sort_click(mode, SortMode::DateModified, true),
                (mode, asc),
                "{mode:?} from a different current mode"
            );
        }
    }

    // ---- #39: shuffle ----

    // The Fisher-Yates draw: a fixed seed yields a fixed permutation, and
    // the order is a permutation of every index exactly once.
    #[test]
    fn ensure_shuffle_builds_a_fixed_permutation_per_seed() {
        let mut pl = Playlist::new();
        for i in 0..8 {
            pl.add(OsString::from(format!("{i}.png")), i, 0, 0);
        }
        pl.ensure_shuffle(0xDEAD_BEEF);
        let order_a = pl.shuffle_order.clone().unwrap();
        pl.drop_shuffle();
        pl.ensure_shuffle(0xDEAD_BEEF);
        assert_eq!(pl.shuffle_order.as_ref().unwrap(), &order_a);
        let mut sorted = order_a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8u32).collect::<Vec<u32>>());
        // A different seed is (virtually surely) a different order.
        pl.drop_shuffle();
        pl.ensure_shuffle(0x0123_4567_89AB_CDEF);
        assert_ne!(pl.shuffle_order.as_ref().unwrap(), &order_a);
    }

    // ensure_shuffle keeps an existing order (the `!indexes` guard of
    // _viv_do_initial_shuffle, viv.c:13582) and skips empties (viv.c:12818).
    #[test]
    fn ensure_shuffle_is_lazy_and_keeps_an_existing_order() {
        let mut pl = Playlist::new();
        pl.ensure_shuffle(1);
        assert!(pl.shuffle_order.is_none(), "empty playlist: no order");
        pl.add(OsString::from("a.png"), 1, 0, 0);
        pl.add(OsString::from("b.png"), 2, 0, 0);
        pl.ensure_shuffle(1);
        let first = pl.shuffle_order.clone().unwrap();
        pl.ensure_shuffle(999);
        assert_eq!(pl.shuffle_order.as_ref().unwrap(), &first);
    }

    // An add with a live order joins at a random position: the displaced
    // slot's tail trick (viv.c:9496-9531) — the new index lands somewhere
    // in [0, len] and whichever old index it displaces moves to the TAIL
    // (so old indices may reorder among themselves — that is upstream's
    // own array behavior, faithfully kept).
    #[test]
    fn adds_join_a_live_shuffle_order_in_place() {
        let mut pl = Playlist::new();
        pl.add(OsString::from("a.png"), 1, 0, 0);
        pl.add(OsString::from("b.png"), 2, 0, 0);
        pl.ensure_shuffle(42);
        pl.add(OsString::from("c.png"), 3, 0, 0); // index 2
        let after = pl.shuffle_order.clone().unwrap();
        assert_eq!(after.len(), 3);
        let mut sorted = after.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2], "still a permutation");
        assert!(after.contains(&2), "the new entry is in the order");
        // The tail trick verbatim: if the new index did not land at the
        // tail, the tail now holds an OLD index (the displaced one).
        if let Some(at) = after.iter().position(|&i| i == 2)
            && at < 2
        {
            assert!(
                after[2] == 0 || after[2] == 1,
                "the tail holds the displaced old index"
            );
        }
    }

    // clear drops the order and the counter (viv.c:9369-9380).
    #[test]
    fn clear_drops_the_shuffle_order() {
        let mut pl = Playlist::new();
        pl.add(OsString::from("a.png"), 1, 0, 0);
        pl.ensure_shuffle(7);
        assert!(pl.shuffle_order.is_some());
        pl.clear();
        assert!(pl.shuffle_order.is_none());
        // Rebuilt after re-adding (turn-off-then-on re-shuffles the same way).
        pl.add(OsString::from("b.png"), 1, 0, 0);
        pl.ensure_shuffle(7);
        assert!(pl.shuffle_order.is_some());
    }

    // shuffle_target: found current steps and wraps; a foreign current
    // starts at the order's edge (viv.c:5881-5923).
    #[test]
    fn shuffle_target_steps_wraps_and_defaults_for_foreign_currents() {
        let mut pl = Playlist::new();
        for name in ["a", "b", "c", "d"] {
            pl.add(OsString::from(format!("{name}.png")), 1, 0, 0);
        }
        pl.ensure_shuffle(1234);
        let order = pl.shuffle_order.clone().unwrap();
        let at = |i: usize| pl.entries[order[i] as usize].clone();
        // found: step forward/back with wrap
        let cur = at(1);
        assert_eq!(pl.shuffle_target(&cur, false).map(|e| e.id), Some(at(2).id));
        assert_eq!(pl.shuffle_target(&cur, true).map(|e| e.id), Some(at(0).id));
        // wrap at both ends
        let last = at(3);
        assert_eq!(
            pl.shuffle_target(&last, false).map(|e| e.id),
            Some(at(0).id)
        );
        let first = at(0);
        assert_eq!(
            pl.shuffle_target(&first, true).map(|e| e.id),
            Some(at(3).id)
        );
        // foreign current (id not in the playlist): front for next, back for prev
        let foreign = entry("zz.png", 0, 999);
        assert_eq!(
            pl.shuffle_target(&foreign, false).map(|e| e.id),
            Some(at(0).id)
        );
        assert_eq!(
            pl.shuffle_target(&foreign, true).map(|e| e.id),
            Some(at(3).id)
        );
        // id-0 collision: a direct-open current (id 0) matches the FIRST
        // order slot holding id 0 — upstream's quirk.
        let direct = entry("other.png", 5, 0);
        let first_zero = order
            .iter()
            .position(|&i| pl.entries[i as usize].id == 0)
            .unwrap();
        assert_eq!(
            pl.shuffle_target(&direct, false).map(|e| e.id),
            Some(at((first_zero + 1) % 4).id)
        );
    }

    // Home/End under shuffle read the order's edges (viv.c:6137-6152).
    #[test]
    fn shuffle_edges_are_the_order_ends() {
        let mut pl = Playlist::new();
        for name in ["a", "b", "c"] {
            pl.add(OsString::from(format!("{name}.png")), 1, 0, 0);
        }
        pl.ensure_shuffle(99);
        let order = pl.shuffle_order.clone().unwrap();
        assert_eq!(
            pl.shuffle_edge(false).map(|e| e.id),
            Some(pl.entries[order[0] as usize].id)
        );
        assert_eq!(
            pl.shuffle_edge(true).map(|e| e.id),
            Some(pl.entries[order[2] as usize].id)
        );
        // Without an order: nothing (the caller falls to the sort arms).
        pl.drop_shuffle();
        assert_eq!(pl.shuffle_edge(false), None);
        assert_eq!(pl.shuffle_edge(true), None);
    }

    // Single-entry shuffle: target always wraps to itself (upstream's
    // index math lands back on slot 0).
    #[test]
    fn single_entry_shuffle_targets_itself() {
        let mut pl = Playlist::new();
        pl.add(OsString::from("only.png"), 1, 0, 0);
        pl.ensure_shuffle(5);
        let only = pl.entries[0].clone();
        assert_eq!(
            pl.shuffle_target(&only, false).map(|e| e.path.clone()),
            Some(OsString::from("only.png"))
        );
        assert_eq!(
            pl.shuffle_target(&only, true).map(|e| e.path.clone()),
            Some(OsString::from("only.png"))
        );
    }
}
