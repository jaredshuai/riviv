//! File associations, the install-family command line and the start-menu
//! shortcuts (#26).
//!
//! Upstream anchors: the association tables and registry layout
//! (`_viv_install_association_by_extension` etc., viv.c:8831-9053), the
//! install/uninstall CLI (`_viv_process_install_command_line_options`,
//! viv.c:4454-4742) and the start-menu shortcuts (viv.c:12708-12797).
//! The registry namespace is riviv's own (`riviv.<ext>` progids, the
//! `riviv.Backup` value, the `riviv` start-menu folder) so both viewers
//! coexist on one machine — the same policy as the mutex/class names
//! (README Differences).
//!
//! Two halves, the keys.rs discipline: everything above the shell marker is
//! a dependency-free model pinned by the `#[cfg(test)]` net at the bottom;
//! the Win32 half below it stays thin (SAFETY-commented, no derivable
//! logic).

// ===================== pure model =====================

/// The associable extensions, upstream table order (viv.c:1136-1147). The
/// CLI's install/uninstall flag bits index this table (`1 << i`,
/// viv.c:4600-4615) — the order IS the wire format.
pub(crate) const EXTENSIONS: [&str; 9] = [
    "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
];

/// The progid description per extension (localization_en_us.h:288-296 —
/// byte-identical in zh_cn.h:289-297, so no table split by language).
const DESCRIPTIONS: [&str; 9] = [
    "Bitmap Image",
    "Animated GIF Image",
    "Icon File",
    "JPEG Image",
    "JPEG Image",
    "PNG Image",
    "TIFF Image",
    "TIFF Image",
    "WebP Image",
];

/// The DefaultIcon override per extension (viv.c:1163-1174): only `.ico`
/// files show their own content (`%1`); everything else uses the exe icon.
const ICON_LOCATIONS: [Option<&str>; 9] =
    [None, None, Some("%1"), None, None, None, None, None, None];

/// The extension's index in [`EXTENSIONS`], case-insensitively — the CLI
/// word matches extensions the same way (`string_icompare_lowercase_ascii`,
/// viv.c:4602).
pub(crate) fn extension_index(word: &str) -> Option<usize> {
    EXTENSIONS
        .iter()
        .position(|ext| ext.eq_ignore_ascii_case(word))
}

pub(crate) fn description(index: usize) -> &'static str {
    DESCRIPTIONS[index]
}

pub(crate) fn icon_location(index: usize) -> Option<&'static str> {
    ICON_LOCATIONS[index]
}

/// The HKCU-relative registry keys under `SOFTWARE\Classes` (viv.c:8840-8920
/// builds the same strings). `\` separators verbatim.
const CLASSES: &str = "SOFTWARE\\Classes";

/// The progid an extension is pointed at (`riviv.png`; upstream's is
/// `voidImageViewer.png`, viv.c:8846-8847 — namespaced apart on purpose).
pub(crate) fn progid(index: usize) -> String {
    format!("riviv.{}", EXTENSIONS[index])
}

/// `SOFTWARE\Classes\.png` — the extension key carrying the default value
/// and the pre-install backup.
pub(crate) fn dot_key(index: usize) -> String {
    format!("{CLASSES}\\.{}", EXTENSIONS[index])
}

/// `SOFTWARE\Classes\riviv.png` — the progid key carrying the description.
pub(crate) fn progid_key(index: usize) -> String {
    format!("{CLASSES}\\{}", progid(index))
}

/// `SOFTWARE\Classes\riviv.png\DefaultIcon`.
pub(crate) fn default_icon_key(index: usize) -> String {
    format!("{}\\DefaultIcon", progid_key(index))
}

/// `SOFTWARE\Classes\riviv.png\shell\open\command`.
pub(crate) fn command_key(index: usize) -> String {
    format!("{}\\shell\\open\\command", progid_key(index))
}

/// The value name holding the pre-install `.ext` default on the dot key
/// (upstream `voidImageViewer.Backup`, viv.c:8922-8929 — namespaced).
pub(crate) const BACKUP_VALUE: &str = "riviv.Backup";

/// The shell open command stored under the progid (`"<exe>" "%1"`,
/// viv.c:8905-8907) — also the second half of `is_association`'s compare
/// (viv.c:9037-9039).
pub(crate) fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// The DefaultIcon command when the extension has no `%1` override
/// (`<exe>,0`, viv.c:8870-8872).
pub(crate) fn icon_command(exe: &str) -> String {
    format!("{exe},0")
}

/// One command-line word as upstream's `string_get_word` yields it
/// (string.c:804-848): the UNQUOTED text (`""` collapses to a literal `"`),
/// plus whether the word BEGAN with a quote — the caller's switch test
/// refuses quoted words (viv.c:4501-4514).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    pub(crate) text: String,
    pub(crate) quoted: bool,
}

/// Whether a word is a switch: unquoted, `/`- or `-`-prefixed, and with no
/// '.' anywhere in the word (string_is_dot, string.c:856-871 — so
/// `-foo.png` is a FILE; the same rule as `main.rs`'s `is_switch`, plus the
/// quote awareness the raw command line provides).
pub(crate) fn is_switch(word: &Word) -> bool {
    !word.quoted
        && word
            .text
            .chars()
            .next()
            .is_some_and(|c| c == '/' || c == '-')
        && !word.text.contains('.')
}

/// Whitespace per `wchar_is_ws` (wchar.c:37-45): space, tab, CR, LF.
fn is_ws(c: u16) -> bool {
    c == u16::from(b' ') || c == u16::from(b'\t') || c == u16::from(b'\r') || c == u16::from(b'\n')
}

/// Split a raw command line (no exe-name special casing — every word comes
/// back; the callers decide what words[0] is). `rest_after_first` is the
/// offset of the raw remainder AFTER the first word and its trailing
/// whitespace — upstream's `cl_start` (viv.c:4481-4487), re-attached to the
/// `/isrunas` re-execution verbatim.
pub(crate) struct CommandLine {
    pub(crate) words: Vec<Word>,
    /// `words[0]`'s remainder offset into the original `cl` — 0 when there
    /// is no first word.
    pub(crate) rest_after_first: usize,
}

pub(crate) fn split_command_line(cl: &[u16]) -> CommandLine {
    let mut words = Vec::new();
    let mut rest = 0usize;
    let mut i = 0usize;
    while i < cl.len() {
        while i < cl.len() && is_ws(cl[i]) {
            i += 1;
        }
        if i >= cl.len() {
            break;
        }
        let quoted = cl[i] == u16::from(b'"');
        let mut text = Vec::new();
        let mut in_quote = false;
        while i < cl.len() {
            let c = cl[i];
            if c == u16::from(b'"') && cl.get(i + 1) == Some(&u16::from(b'"')) {
                // A doubled quote is one literal quote (string.c:811-815).
                text.push(u16::from(b'"'));
                i += 2;
            } else if c == u16::from(b'"') {
                in_quote = !in_quote;
                i += 1;
            } else if !in_quote && is_ws(c) {
                break;
            } else {
                text.push(c);
                i += 1;
            }
        }
        words.push(Word {
            text: String::from_utf16_lossy(&text),
            quoted,
        });
        if words.len() == 1 {
            while i < cl.len() && is_ws(cl[i]) {
                i += 1;
            }
            rest = i;
        }
    }
    CommandLine {
        words,
        rest_after_first: rest,
    }
}

/// The `/isrunas` re-execution parameters: the marker plus the original
/// command line's remainder after the exe name, verbatim (viv.c:4650-4653).
pub(crate) fn isrunas_params(rest: &str) -> String {
    format!("/isrunas {rest}")
}

/// What the install-family parser made of the command line
/// (`_viv_process_install_command_line_options`'s locals, viv.c:4456-4479).
/// `install_path`/`install_options` are `Some` only for non-empty captured
/// words; `uninstall` records the switch separately from its optional path
/// (a bare `-uninstall` defaults the path to the exe's directory,
/// viv.c:4541-4546).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct InstallPlan {
    pub(crate) install_path: Option<String>,
    pub(crate) install_options: Option<String>,
    pub(crate) uninstall: bool,
    pub(crate) uninstall_path: Option<String>,
    /// 1 = `/appdata`, -1 = `/noappdata`, 0 = absent.
    pub(crate) appdata: i32,
    /// 1 = `/startmenu`, -1 = `/nostartmenu`, 0 = absent.
    pub(crate) startmenu: i32,
    /// Extension install bits, `1 << EXTENSIONS` index (viv.c:4610).
    pub(crate) install_flags: u32,
    /// Extension uninstall bits (viv.c:4606).
    pub(crate) uninstall_flags: u32,
    pub(crate) is_runas: bool,
    /// Any switch that demands the admin pass (upstream `is_admin_install`):
    /// install/install-options/uninstall/appdata/noappdata/startmenu/
    /// nostartmenu all set it (viv.c:4525-4581).
    admin_switch: bool,
    /// Any switch handled at standard-user level (upstream
    /// `is_standard_user_install`): the extension switches and `-uninstall`
    /// (viv.c:4556/4613).
    standard_switch: bool,
}

impl InstallPlan {
    /// Whether the process must exit after processing (upstream's return
    /// value: `is_admin_install || is_standard_user_install`, viv.c:4736).
    pub(crate) fn handled(&self) -> bool {
        self.admin_switch || self.standard_switch
    }

    /// Whether the non-elevated pass re-executes itself via `runas` when it
    /// is not admin (upstream's `is_admin_install` gate, viv.c:4637).
    pub(crate) fn needs_admin(&self) -> bool {
        self.admin_switch
    }
}

/// Parse the words AFTER the exe name (upstream viv.c:4490-4619 — the loop
/// parses every word; the `// skip first parameter.` comment above it is a
/// stale copy from the config parser, and the NSIS/Options/re-exec callers
/// all pass `/install`-style switches as the FIRST parameter). A switch
/// word consumed as an argument (`-install <next>`) takes the next word
/// blindly, like upstream's `string_get_word` into `install_path`.
pub(crate) fn parse_install_options(words: &[Word]) -> InstallPlan {
    let mut plan = InstallPlan::default();
    let mut i = 0usize;
    while i < words.len() {
        let word = &words[i];
        i += 1;
        if !is_switch(word) {
            continue; // files and junk words are discarded (no else branch)
        }
        let name = word.text[1..].to_ascii_lowercase();
        match name.as_str() {
            "install" => {
                plan.install_path = words
                    .get(i)
                    .map(|w| w.text.clone())
                    .filter(|s| !s.is_empty());
                i += 1;
                // viv.c:4523 clears uninstall_path — the file cleanup (and
                // the bare-switch path defaulting) is path-driven, so the
                // request bit clears with it. The 0xffffffff uninstall FLAGS
                // from an earlier /uninstall survive (upstream clears the
                // path only).
                plan.uninstall = false;
                plan.uninstall_path = None;
                plan.admin_switch = true;
            }
            "install-options" => {
                plan.install_options = words
                    .get(i)
                    .map(|w| w.text.clone())
                    .filter(|s| !s.is_empty());
                i += 1;
                plan.admin_switch = true;
            }
            "uninstall" => {
                // An explicitly EMPTY path word (`/uninstall ""`) is no
                // path at all — upstream's `uninstall_path[0] == 0` then
                // defaults to the exe dir (viv.c:4541-4546); keeping ""
                // would resolve the cleanup against the CURRENT directory.
                plan.uninstall_path = words
                    .get(i)
                    .map(|w| w.text.clone())
                    .filter(|s| !s.is_empty());
                i += 1;
                plan.uninstall = true;
                plan.uninstall_flags = 0xffff_ffff; // uninstall all (viv.c:4549)
                plan.install_flags = 0;
                plan.startmenu = -1;
                plan.install_path = None; // viv.c:4553
                plan.admin_switch = true;
                plan.standard_switch = true;
            }
            "appdata" => {
                plan.appdata = 1;
                plan.admin_switch = true;
            }
            "noappdata" => {
                plan.appdata = -1;
                plan.admin_switch = true;
            }
            "startmenu" => {
                plan.startmenu = 1;
                plan.admin_switch = true;
            }
            "nostartmenu" => {
                plan.startmenu = -1;
                plan.admin_switch = true;
            }
            "isrunas" => {
                plan.is_runas = true;
            }
            _ => {
                // An extension switch, optionally `no`-prefixed (viv.c:4588-4617).
                let (name, is_no) = match name.strip_prefix("no") {
                    Some(rest) => (rest, true),
                    None => (name.as_str(), false),
                };
                if let Some(ext) = extension_index(name) {
                    if is_no {
                        plan.uninstall_flags |= 1 << ext;
                    } else {
                        plan.install_flags |= 1 << ext;
                    }
                    plan.standard_switch = true;
                }
                // Unknown switches fall through untouched: `/slideshow` and
                // friends stay the config parser's business (handled=false
                // keeps this process alive for the normal open path).
            }
        }
    }
    plan
}

// ===================== Win32 shell =====================
//
// Error policy (ADR 0001, classified once): association/shortcut writes
// are USER-level — a partially failing association must not kill an
// installer mid-run — so failures log to stderr and carry on, the
// `Config::save` precedent (config.rs:240-242; upstream debug_printf /
// unchecked returns). The one exception is the critical exe copy
// (`-install`), which upstream fatals on (debug_fatal, viv.c:12813) and
// this port routes to `window::fatal`. stderr is invisible under the
// windows subsystem unless redirected — same as the existing practice
// (options_dlg.rs, window.rs eprintln sites).

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND};
use windows::Win32::Storage::FileSystem::{
    CopyFileW, DeleteFileW, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, RemoveDirectoryW,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::System::Threading::{
    INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    CSIDL_COMMON_PROGRAMS, IShellLinkW, IsUserAnAdmin, SEE_MASK_INVOKEIDLIST,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHGetFolderPathW, ShellExecuteExW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowA, GetWindowThreadProcessId, SW_SHOWNORMAL, SendMessageW, WM_CLOSE,
};
use windows::core::{Interface, PCSTR, PCWSTR, s};

use crate::config::{Config, FILE_NAME, appdata_dir, exe_dir};
use crate::text::to_wide;

/// An owned registry key — RegCloseKey on drop (every arm below must not
/// leak the handles the create/open calls hand out).
struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: the handle was produced by a successful create/open in
        // this module and is closed exactly once, here.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// Open (or create, like upstream's uninstall arm — viv.c:8953 creates the
/// dot key it is about to restore into) a key under HKCU with
/// set+query access.
fn open_or_create_key(subkey: &str) -> Option<RegKey> {
    let subkey_w = to_wide(subkey);
    let mut hkey = HKEY::default();
    // SAFETY: subkey_w is NUL-terminated and outlives the call; the other
    // parameters are null/default constants; hkey is written only on
    // success, checked below.
    let ret = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            None,
            &mut hkey,
            None,
        )
    };
    if ret.is_ok() {
        Some(RegKey(hkey))
    } else {
        eprintln!("riviv: RegCreateKeyExW({subkey}) failed (GLE={})", ret.0);
        None
    }
}

/// Read a REG_SZ/REG_EXPAND_SZ value — the default value when `value` is
/// None (upstream `_viv_get_registry_string`, viv.c:9055-9083: other types
/// read as missing; an EMPTY string is a successful read).
fn get_registry_string(subkey: &str, value: Option<&str>) -> Option<String> {
    let subkey_w = to_wide(subkey);
    let mut hkey = HKEY::default();
    // SAFETY: NUL-terminated subkey; hkey written only on success.
    let ret = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &mut hkey,
        )
    };
    if !ret.is_ok() {
        return None;
    }
    let hkey = RegKey(hkey);
    let value_w = value.map(to_wide);
    let name = match &value_w {
        Some(v) => PCWSTR(v.as_ptr()),
        None => PCWSTR::null(),
    };
    let mut ty = REG_VALUE_TYPE(0);
    let mut size: u32 = 0;
    // SAFETY: name is null or points at a live NUL-terminated string; the
    // size probe passes no data buffer.
    let ret = unsafe { RegQueryValueExW(hkey.0, name, None, Some(&mut ty), None, Some(&mut size)) };
    if !ret.is_ok() || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: buf holds `size` bytes for the write; size is re-read from
    // the same call.
    let ret = unsafe {
        RegQueryValueExW(
            hkey.0,
            name,
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if !ret.is_ok() {
        return None;
    }
    let words: Vec<u16> = buf[..size as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_ne_bytes(*c))
        .take_while(|&w| w != 0)
        .collect();
    Some(String::from_utf16_lossy(&words))
}

/// Write a REG_SZ value — the default value when `value` is None. The data
/// carries its terminating NUL (upstream writes the wide string with it).
/// Returns success so callers guarding user data (the backup restore) can
/// order their follow-up on it — stricter than upstream, which never
/// checks (viv.c:8957-8971 deletes the backup even if the restore write
/// failed).
fn set_registry_string(hkey: &RegKey, value: Option<&str>, text: &str) -> bool {
    let value_w = value.map(to_wide);
    let name = match &value_w {
        Some(v) => PCWSTR(v.as_ptr()),
        None => PCWSTR::null(),
    };
    let mut data = to_wide(text);
    data.push(0);
    // SAFETY: name is null or live NUL-terminated; data's length rides the
    // slice.
    let ret = unsafe {
        RegSetValueExW(
            hkey.0,
            name,
            None,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                data.as_ptr().cast::<u8>(),
                data.len() * 2,
            )),
        )
    };
    if !ret.is_ok() {
        eprintln!("riviv: RegSetValueExW failed (GLE={})", ret.0);
        return false;
    }
    true
}

/// Install one extension's association (viv.c:8831-8940): uninstall the old
/// one first, then DefaultIcon → description → shell open command → the
/// `.ext` default with the pre-install backup.
pub(crate) fn install_association_by_extension(index: usize) {
    // Make sure we uninstall old associations first (viv.c:8843-8844) —
    // this restores any backup so the capture below sees the ORIGINAL
    // default, keeping the chain intact across reinstalls.
    uninstall_association_by_extension(index);

    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    if let Some(hkey) = open_or_create_key(&default_icon_key(index)) {
        match icon_location(index) {
            Some(icon) => {
                set_registry_string(&hkey, None, icon); // .ico shows itself (%1)
            }
            None => {
                set_registry_string(&hkey, None, &icon_command(&exe));
            }
        }
    }
    if let Some(hkey) = open_or_create_key(&progid_key(index)) {
        set_registry_string(&hkey, None, description(index));
    }
    if let Some(hkey) = open_or_create_key(&command_key(index)) {
        set_registry_string(&hkey, None, &open_command(&exe));
    }
    if let Some(hkey) = open_or_create_key(&dot_key(index)) {
        // Back up the current default ONLY when no backup exists yet
        // (viv.c:8922-8930) — an absent default backs up as an empty
        // string, which is a value that exists.
        if get_registry_string(&dot_key(index), Some(BACKUP_VALUE)).is_none() {
            let current = get_registry_string(&dot_key(index), None).unwrap_or_default();
            set_registry_string(&hkey, Some(BACKUP_VALUE), &current);
        }
        set_registry_string(&hkey, None, &progid(index));
    }
}

/// Uninstall one extension's association (viv.c:8942-8981): restore the
/// backed-up `.ext` default (only when a Backup value exists — with none,
/// the default is left UNTOUCHED), delete the Backup value, remove the
/// progid. The progid removal is `RegDeleteTreeW` where upstream calls the
/// single-level `RegDeleteKey` — which FAILS on a key with subkeys (the
/// progid always has DefaultIcon + shell\open\command) and leaks the tree;
/// riviv cleans up fully instead (README Differences; the issue's
/// "卸载后无残留" acceptance demands it).
pub(crate) fn uninstall_association_by_extension(index: usize) {
    if let Some(hkey) = open_or_create_key(&dot_key(index))
        && let Some(backup) = get_registry_string(&dot_key(index), Some(BACKUP_VALUE))
    {
        // Delete the backup ONLY after the restore landed — deleting it on
        // a failed write would lose the user's previous association
        // outright (upstream deletes unconditionally, viv.c:8957-8971).
        if set_registry_string(&hkey, None, &backup) {
            let backup_w = to_wide(BACKUP_VALUE);
            // SAFETY: NUL-terminated live string; the return is logged —
            // upstream debug_printfs it (viv.c:8963-8971).
            let ret = unsafe { RegDeleteValueW(hkey.0, PCWSTR(backup_w.as_ptr())) };
            if !ret.is_ok() {
                eprintln!(
                    "riviv: RegDeleteValueW({BACKUP_VALUE}) failed (GLE={})",
                    ret.0
                );
            }
        }
    }
    let progid_w = to_wide(&progid_key(index));
    // SAFETY: NUL-terminated live string (DEVIATION: tree delete, see doc).
    let ret = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(progid_w.as_ptr())) };
    // A missing progid is the normal uninstall-first path of a first
    // install (nothing to remove yet); anything else is logged.
    if !ret.is_ok() && ret.0 != ERROR_FILE_NOT_FOUND.0 {
        eprintln!(
            "riviv: RegDeleteTreeW({}) failed (GLE={})",
            progid_key(index),
            ret.0
        );
    }
}

/// Whether the extension is currently associated with THIS exe
/// (`_viv_is_association`, viv.c:8983-9053: BOTH the `.ext` default
/// pointing at the progid AND the open command matching the current exe —
/// DefaultIcon is not part of the verdict).
pub(crate) fn is_association(index: usize) -> bool {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    get_registry_string(&dot_key(index), None).is_some_and(|v| v == progid(index))
        && get_registry_string(&command_key(index), None).is_some_and(|v| v == open_command(&exe))
}

/// Install every flagged extension (viv.c:11903-11916 loop).
pub(crate) fn install_association(flags: u32) {
    for (i, _) in EXTENSIONS
        .iter()
        .enumerate()
        .filter(|(i, _)| flags & (1 << i) != 0)
    {
        install_association_by_extension(i);
    }
}

/// Uninstall every flagged extension (viv.c:11918-11927 loop).
pub(crate) fn uninstall_association(flags: u32) {
    for (i, _) in EXTENSIONS
        .iter()
        .enumerate()
        .filter(|(i, _)| flags & (1 << i) != 0)
    {
        uninstall_association_by_extension(i);
    }
}

/// The start-menu folder under the all-users Programs folder
/// (`CSIDL_COMMON_PROGRAMS`, viv.c:12718/12742 — upstream's folder is
/// "void Image Viewer"; riviv namespaces it like everything else).
const START_MENU_DIR: &str = "riviv";

fn common_programs() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut buf = [0u16; 260]; // MAX_PATH
    // SAFETY: a MAX_PATH out-buffer for the duration of the call; the rest
    // are null/default parameters (the config.rs appdata_dir pattern).
    let hr = unsafe { SHGetFolderPathW(None, CSIDL_COMMON_PROGRAMS as i32, None, 0, &mut buf) };
    if hr.is_ok()
        && let Some(len) = buf.iter().position(|&c| c == 0)
        && len > 0
    {
        return Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])));
    }
    None
}

fn start_menu_dir() -> Option<PathBuf> {
    common_programs().map(|p| p.join(START_MENU_DIR))
}

/// Whether the start-menu folder exists (viv.c:12708-12731 — existence of
/// the directory itself).
pub(crate) fn is_start_menu_shortcuts() -> bool {
    start_menu_dir().is_some_and(|dir| path_exists(&dir))
}

fn path_exists(path: &Path) -> bool {
    let wide = to_wide(&path.to_string_lossy());
    // SAFETY: NUL-terminated path string for a pure attribute query.
    (unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) }) != INVALID_FILE_ATTRIBUTES
}

/// Create a .lnk (upstream `os_create_shell_link`, os.c:1036-1073): skipped
/// entirely when the TARGET does not exist, working directory = the
/// target's folder, no arguments, no description, overwrite allowed.
fn create_shell_link(target: &Path, link: &Path) {
    if !path_exists(target) {
        return;
    }
    let target_w = to_wide(&target.to_string_lossy());
    let link_w = to_wide(&link.to_string_lossy());
    let empty = to_wide("");
    let dir_w = to_wide(
        &target
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    // SAFETY: CoCreateInstance over the shell's registered ShellLink class;
    // every string below is a live NUL-terminated buffer; failures are
    // user-level (fail-soft, the policy block above).
    unsafe {
        let sl: IShellLinkW = match CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) {
            Ok(sl) => sl,
            Err(e) => {
                eprintln!("riviv: CoCreateInstance(ShellLink) failed: {e}");
                return;
            }
        };
        let _ = sl.SetPath(PCWSTR(target_w.as_ptr()));
        let _ = sl.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()));
        let _ = sl.SetArguments(PCWSTR(empty.as_ptr()));
        let _ = sl.SetDescription(PCWSTR(empty.as_ptr()));
        match sl.cast::<IPersistFile>() {
            Ok(pf) => {
                if let Err(e) = pf.Save(PCWSTR(link_w.as_ptr()), true) {
                    eprintln!("riviv: IPersistFile::Save failed: {e}");
                }
            }
            Err(e) => eprintln!("riviv: IShellLink cast to IPersistFile failed: {e}"),
        }
    }
}

/// Install the start-menu shortcuts (viv.c:12733-12767): always uninstall
/// first (so a language change renames cleanly upstream — the folder is
/// fixed here), then the app link and, when an Uninstall.exe sits beside
/// the exe, its link.
pub(crate) fn install_start_menu_shortcuts() {
    uninstall_start_menu_shortcuts();
    let Some(dir) = start_menu_dir() else {
        return;
    };
    // Upstream os_make_sure_path_exists (viv.c:12753).
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("riviv: create start-menu dir failed: {e}");
        return;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    create_shell_link(&exe, &dir.join("riviv.lnk"));
    let uninstall_exe = exe_dir()
        .map(|d| d.join("Uninstall.exe"))
        .unwrap_or_default();
    create_shell_link(&uninstall_exe, &dir.join("Uninstall.lnk"));
}

/// Remove the start-menu shortcuts (viv.c:12769-12791): both .lnk files
/// then the folder.
pub(crate) fn uninstall_start_menu_shortcuts() {
    let Some(dir) = start_menu_dir() else {
        return;
    };
    // SAFETY: NUL-terminated live strings; DeleteFileW/RemoveDirectoryW
    // failures are user-level fail-soft (upstream ignores them).
    unsafe {
        let lnk = to_wide(&dir.join("riviv.lnk").to_string_lossy());
        let _ = DeleteFileW(PCWSTR(lnk.as_ptr()));
        let lnk = to_wide(&dir.join("Uninstall.lnk").to_string_lossy());
        let _ = DeleteFileW(PCWSTR(lnk.as_ptr()));
        let dir_w = to_wide(&dir.to_string_lossy());
        let _ = RemoveDirectoryW(PCWSTR(dir_w.as_ptr()));
    }
}

/// Whether the process is elevated (upstream `os_is_admin`, os.c:727-748 —
/// Vista+ IsUserAnAdmin; older systems are out of scope here).
pub(crate) fn is_admin() -> bool {
    // SAFETY: a parameterless shell32 query.
    unsafe { IsUserAnAdmin() }.as_bool()
}

/// ShellExecuteEx wrapper (upstream `os_shell_execute`, os.c:1094+):
/// INVOKEIDLIST mask like upstream, NOCLOSEPROCESS + a wait when asked.
/// Returns Err on a failed launch (the callers decide fail-soft vs loud).
pub(crate) fn shell_execute(
    file: &str,
    params: Option<&str>,
    verb: Option<&str>,
    wait: bool,
) -> Result<(), String> {
    let file_w = to_wide(file);
    let params_w = params.map(to_wide);
    let verb_w = verb.map(to_wide);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST | if wait { SEE_MASK_NOCLOSEPROCESS } else { 0 },
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: match &params_w {
            Some(p) => PCWSTR(p.as_ptr()),
            None => PCWSTR::null(),
        },
        lpVerb: match &verb_w {
            Some(v) => PCWSTR(v.as_ptr()),
            None => PCWSTR::null(),
        },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: sei outlives the call and every string it points at is alive
    // in this frame; the struct is written only by the API.
    unsafe { ShellExecuteExW(&mut sei) }
        .map_err(|e| format!("ShellExecuteExW({file}) failed: {e}"))?;
    if wait && !sei.hProcess.is_invalid() {
        // SAFETY: hProcess came from this successful execute; the wait is
        // bounded by the child's exit and the handle is closed exactly once.
        unsafe {
            let _ = WaitForSingleObject(sei.hProcess, INFINITE);
            let _ = CloseHandle(sei.hProcess);
        }
    }
    Ok(())
}

/// Close every running viewer instance (viv.c:12666-12704): find each
/// riviv window, WM_CLOSE it, and wait for its process to exit before
/// looking again — install/uninstall must not race a live exe.
fn close_existing_process() {
    // The SAFETY contract for the inline unsafe block in the loop head is
    // stated immediately above it (the clippy rule wants the comment right
    // before the block, even mid-condition).
    while let Some(hwnd) =
        // SAFETY: a pure top-level window search by the static riviv class
        // name (the single-instance handoff's find-class, #21); each hwnd
        // it yields is re-derived per iteration and stays live for the
        // calls on it below.
        unsafe { FindWindowA(s!("riviv"), PCSTR::null()) }
            .ok()
            .filter(|h| !h.is_invalid())
    {
        let mut pid: u32 = 0;
        // SAFETY: hwnd is live from FindWindowA; pid is written by the call.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // SAFETY: a SYNCHRONIZE-only open on the found pid; failure still
        // closes the window, just without waiting (upstream drops the
        // handle branch the same way, viv.c:12789-12796).
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.ok();
        // SAFETY: hwnd is live; a synchronous close request.
        let _ = unsafe { SendMessageW(hwnd, WM_CLOSE, None, None) };
        match process {
            Some(handle) => {
                // SAFETY: handle is owned; waited and closed exactly once.
                unsafe {
                    let _ = WaitForSingleObject(handle, INFINITE);
                    let _ = CloseHandle(handle);
                }
            }
            // No handle to wait on (upstream just skips its wait here,
            // viv.c:12789-12796): poll the window away bounded, so a
            // closed-but-not-yet-gone instance is not copied over, without
            // spinning forever on a window that refuses to close.
            None => {
                let mut retries = 100; // ~5 s at 50 ms
                loop {
                    // SAFETY: the same pure class-name search as the loop
                    // head.
                    let gone = unsafe { FindWindowA(s!("riviv"), PCSTR::null()) }
                        .ok()
                        .is_none_or(|h| h.is_invalid());
                    if gone || retries == 0 {
                        break;
                    }
                    retries -= 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
}

/// The raw GetCommandLineW as a owned wide buffer (the #21 handoff's
/// NUL-walk, window.rs:3478-3487).
fn command_line_wide() -> Vec<u16> {
    // SAFETY: GetCommandLineW returns this process's NUL-terminated line,
    // valid for the process lifetime; the walk reads up to that NUL.
    unsafe {
        let cl = GetCommandLineW();
        let mut n = 0usize;
        while *cl.0.add(n) != 0 {
            n += 1;
        }
        std::slice::from_raw_parts(cl.0, n).to_vec()
    }
}

/// Process the install-family command line (upstream
/// `_viv_process_install_command_line_options`, viv.c:4454-4742, called
/// after config load and before the single-instance gate, viv.c:5261→5270).
/// Returns true when the command line was an install-family one and the
/// caller must exit instead of showing a window.
pub(crate) fn process_install_command_line(config: &mut Config) -> bool {
    let cl = command_line_wide();
    let split = split_command_line(&cl);
    let plan = if split.words.is_empty() {
        InstallPlan::default()
    } else {
        parse_install_options(&split.words[1..])
    };
    if !plan.handled() {
        return false;
    }

    // Standard-user associations first (viv.c:4623-4635) — skipped in the
    // elevated child, which re-parses the same flags and must not redo
    // them below its own is_runas.
    if !plan.is_runas {
        if plan.install_flags != 0 {
            install_association(plan.install_flags);
        }
        if plan.uninstall_flags != 0 {
            uninstall_association(plan.uninstall_flags);
        }
    }

    // The admin pass: re-execute elevated when we are not (viv.c:4637-4658)
    // — upstream launches "/isrunas <rest>" and returns immediately (a
    // cancelled UAC just means nothing further happens).
    if plan.needs_admin() && !is_admin() && !plan.is_runas {
        let rest = String::from_utf16_lossy(&cl[split.rest_after_first..]);
        let params = isrunas_params(&rest);
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Err(e) = shell_execute(&exe, Some(&params), Some("runas"), true) {
            eprintln!("riviv: {e}"); // user-level: a refused elevation is not fatal
        }
        return true;
    }

    // The settings-location switches (viv.c:4660-4676): flip and save —
    // Config::save's appdata branch performs upstream's TWO writes (the
    // full table at the active location plus the exe-dir marker).
    if plan.appdata > 0 {
        config.appdata = 1;
        config.save();
    } else if plan.appdata < 0 {
        config.appdata = 0;
        config.save();
    }

    if plan.startmenu > 0 {
        install_start_menu_shortcuts();
    } else if plan.startmenu < 0 {
        uninstall_start_menu_shortcuts();
    }

    if let Some(install_path) = &plan.install_path {
        // Make sure no other process is running (viv.c:4692-4693).
        close_existing_process();
        let Some(src_dir) = exe_dir() else {
            return true;
        };
        let _ = std::fs::create_dir_all(install_path);
        copy_install_file(&src_dir, install_path, "riviv.exe", true);
        copy_install_file(&src_dir, install_path, "Uninstall.exe", false);
        // Upstream also copies Changes.txt (viv.c:4701) — riviv ships none;
        // recorded in README Differences.

        if let Some(options) = &plan.install_options {
            let new_exe = Path::new(install_path).join("riviv.exe");
            if let Err(e) = shell_execute(&new_exe.to_string_lossy(), Some(options), None, true) {
                eprintln!("riviv: {e}");
            }
        }
    }

    if plan.uninstall {
        // viv.c:4713-4734: default the path to the exe's directory, close
        // running instances, empty %APPDATA%\riviv, then the install dir.
        let dir = plan
            .uninstall_path
            .clone()
            .or_else(|| exe_dir().map(|d| d.to_string_lossy().into_owned()));
        close_existing_process();
        if let Some(appdata) = appdata_dir() {
            delete_file(&appdata.join(FILE_NAME));
            remove_dir(&appdata);
        }
        if let Some(dir) = dir {
            let dir = PathBuf::from(dir);
            delete_file(&dir.join("Uninstall.exe"));
            delete_file(&dir.join(FILE_NAME));
            delete_file(&dir.join("riviv.exe"));
            remove_dir(&dir);
        }
    }

    true
}

fn copy_install_file(src_dir: &Path, install_path: &str, filename: &str, critical: bool) {
    let src = src_dir.join(filename);
    let dst = Path::new(install_path).join(filename);
    // SAFETY: NUL-terminated live strings; overwrite allowed
    // (bFailIfExists=FALSE, viv.c:12813).
    let ret = unsafe {
        CopyFileW(
            PCWSTR(to_wide(&src.to_string_lossy()).as_ptr()),
            PCWSTR(to_wide(&dst.to_string_lossy()).as_ptr()),
            false,
        )
    };
    match ret {
        Err(e) if critical => {
            // Upstream debug_fatals on the exe copy (viv.c:12813-12816).
            crate::window::fatal(&format!(
                "unable to copy {} to {}: {e}",
                src.display(),
                dst.display()
            ));
        }
        Err(e) => eprintln!("riviv: unable to copy {}: {e}", dst.display()),
        Ok(()) => {}
    }
}

fn delete_file(path: &Path) {
    // SAFETY: NUL-terminated live string; failure is fail-soft (upstream
    // ignores it, viv.c:12696-12704).
    let _ = unsafe { DeleteFileW(PCWSTR(to_wide(&path.to_string_lossy()).as_ptr())) };
}

fn remove_dir(path: &Path) {
    // SAFETY: NUL-terminated live string; failure is fail-soft (a non-empty
    // or re-created directory simply stays, viv.c:4725/4733).
    let _ = unsafe { RemoveDirectoryW(PCWSTR(to_wide(&path.to_string_lossy()).as_ptr())) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<Word> {
        list.iter()
            .map(|t| Word {
                text: (*t).to_string(),
                quoted: false,
            })
            .collect()
    }

    fn quoted(text: &str) -> Word {
        Word {
            text: text.to_string(),
            quoted: true,
        }
    }

    #[test]
    fn extension_table_pins_upstream_order() {
        // viv.c:1136-1147 — the order IS the CLI bit order.
        assert_eq!(
            EXTENSIONS,
            [
                "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp"
            ]
        );
        for (i, ext) in EXTENSIONS.iter().enumerate() {
            assert_eq!(extension_index(ext), Some(i));
            assert_eq!(extension_index(&ext.to_ascii_uppercase()), Some(i));
        }
        assert_eq!(extension_index("avif"), None);
    }

    #[test]
    fn descriptions_and_icon_locations_pin_upstream() {
        // localization_en_us.h:288-296 (byte-identical in zh); viv.c:1163-1174.
        assert_eq!(description(0), "Bitmap Image");
        assert_eq!(description(1), "Animated GIF Image");
        assert_eq!(description(2), "Icon File");
        assert_eq!(description(3), "JPEG Image");
        assert_eq!(description(4), "JPEG Image");
        assert_eq!(description(5), "PNG Image");
        assert_eq!(description(6), "TIFF Image");
        assert_eq!(description(7), "TIFF Image");
        assert_eq!(description(8), "WebP Image");
        for i in 0..EXTENSIONS.len() {
            assert_eq!(icon_location(i).is_some(), i == 2, "only ico overrides");
        }
        assert_eq!(icon_location(2), Some("%1"));
    }

    #[test]
    fn registry_keys_pin_the_layout() {
        // viv.c:8840-8920 string-built keys, riviv namespaced.
        assert_eq!(progid(5), "riviv.png");
        assert_eq!(dot_key(5), "SOFTWARE\\Classes\\.png");
        assert_eq!(progid_key(5), "SOFTWARE\\Classes\\riviv.png");
        assert_eq!(
            default_icon_key(5),
            "SOFTWARE\\Classes\\riviv.png\\DefaultIcon"
        );
        assert_eq!(
            command_key(5),
            "SOFTWARE\\Classes\\riviv.png\\shell\\open\\command"
        );
        assert_eq!(BACKUP_VALUE, "riviv.Backup");
    }

    #[test]
    fn command_strings_pin_the_format() {
        assert_eq!(
            open_command(r"C:\a b\riviv.exe"),
            "\"C:\\a b\\riviv.exe\" \"%1\""
        );
        assert_eq!(icon_command(r"C:\a b\riviv.exe"), r"C:\a b\riviv.exe,0");
    }

    #[test]
    fn first_switch_after_the_exe_name_is_parsed() {
        // The stale "skip first parameter" comment is NOT behavior: the
        // parser consumes argv[0] then processes EVERY word (the NSIS
        // installer's `/install` is always the first parameter,
        // installer.nsi:409).
        let plan = parse_install_options(&words(&["riviv.exe", "/png"]));
        assert_eq!(plan.install_flags, 1 << 5);
        assert!(plan.handled());
        assert!(!plan.needs_admin()); // extension switches stay standard-user
    }

    #[test]
    fn no_prefix_switches_uninstall_one_extension() {
        let plan = parse_install_options(&words(&["riviv.exe", "/nopng"]));
        assert_eq!(plan.uninstall_flags, 1 << 5);
        assert_eq!(plan.install_flags, 0);
        assert!(plan.handled());
        assert!(!plan.needs_admin());
    }

    #[test]
    fn unknown_switches_are_not_handled() {
        // `/slideshow` and `/foo` belong to the (unimplemented) config
        // parser — handled=false keeps the normal open path alive.
        let plan = parse_install_options(&words(&["riviv.exe", "/slideshow", "/foo", "-sort"]));
        assert!(!plan.handled());
        assert_eq!(plan.install_flags, 0);
        assert_eq!(plan.uninstall_flags, 0);
    }

    #[test]
    fn install_switch_captures_the_next_word_and_clears_uninstall() {
        let plan = parse_install_options(&words(&[
            "riviv.exe",
            "/uninstall",
            r"C:\old",
            "/install",
            r"C:\new dir",
        ]));
        assert_eq!(plan.install_path.as_deref(), Some(r"C:\new dir"));
        assert!(!plan.uninstall); // the later /install cleared the request
        assert_eq!(plan.uninstall_path, None); // viv.c:4523 clears the path only
        // ...and NOT the flags: upstream's /install clears uninstall_path
        // alone, so the earlier /uninstall's 0xffffffff survives and the
        // execution pass uninstalls every extension before copying.
        assert_eq!(plan.uninstall_flags, 0xffff_ffff);
        assert!(plan.needs_admin());
    }

    #[test]
    fn uninstall_all_semantics() {
        // viv.c:4536-4557: flags=all, startmenu=-1, install_path cleared,
        // BOTH admin and standard passes armed.
        let plan = parse_install_options(&words(&["riviv.exe", "/uninstall"]));
        assert!(plan.uninstall);
        assert_eq!(plan.uninstall_path, None); // the shell defaults it to the exe dir
        assert_eq!(plan.uninstall_flags, 0xffff_ffff);
        assert_eq!(plan.install_flags, 0);
        assert_eq!(plan.startmenu, -1);
        assert_eq!(plan.install_path, None);
        assert!(plan.needs_admin());
        assert!(plan.handled());

        let plan = parse_install_options(&words(&["riviv.exe", "/uninstall", r"C:\dir"]));
        assert_eq!(plan.uninstall_path.as_deref(), Some(r"C:\dir"));

        // An explicitly EMPTY path word is no path: the exe-dir default
        // applies (`/uninstall ""` must not resolve cleanup against the
        // current directory).
        let mut list = words(&["riviv.exe", "/uninstall"]);
        list.push(quoted(""));
        let plan = parse_install_options(&list);
        assert!(plan.uninstall);
        assert_eq!(plan.uninstall_path, None);
    }

    #[test]
    fn install_options_captures_the_next_word() {
        let plan =
            parse_install_options(&words(&["riviv.exe", "/install-options", "/appdata /png"]));
        assert_eq!(plan.install_options.as_deref(), Some("/appdata /png"));
        assert!(plan.needs_admin());
        assert!(plan.handled());
    }

    #[test]
    fn appdata_and_startmenu_switches() {
        let plan = parse_install_options(&words(&["riviv.exe", "/appdata", "/startmenu"]));
        assert_eq!(plan.appdata, 1);
        assert_eq!(plan.startmenu, 1);
        assert!(plan.needs_admin());

        let plan = parse_install_options(&words(&["riviv.exe", "/noappdata", "/nostartmenu"]));
        assert_eq!(plan.appdata, -1);
        assert_eq!(plan.startmenu, -1);
        assert!(plan.needs_admin());
    }

    #[test]
    fn isrunas_sets_only_its_flag() {
        let plan = parse_install_options(&words(&["riviv.exe", "/isrunas"]));
        assert!(plan.is_runas);
        assert!(!plan.handled()); // alone, it does nothing (the elevated child re-parses)
    }

    #[test]
    fn switch_words_consumed_as_arguments_do_not_parse() {
        // `-install` grabs the next word blindly (string_get_word into
        // install_path) — the grabbed word never re-parses as a switch.
        let plan = parse_install_options(&words(&["riviv.exe", "/install", "/appdata"]));
        assert_eq!(plan.install_path.as_deref(), Some("/appdata"));
        assert_eq!(plan.appdata, 0);
    }

    #[test]
    fn quoted_and_dotted_words_are_not_switches() {
        let mut list = words(&["riviv.exe"]);
        list.push(quoted("/png"));
        list.extend(words(&["-foo.png", "plain.png"]));
        let plan = parse_install_options(&list);
        assert_eq!(plan.install_flags, 0);
        assert!(!plan.handled());
    }

    #[test]
    fn comparisons_are_case_insensitive() {
        let plan = parse_install_options(&words(&["riviv.exe", "/PNG", "-NoAppData"]));
        assert_eq!(plan.install_flags, 1 << 5);
        assert_eq!(plan.appdata, -1);
    }

    #[test]
    fn multiple_extensions_or_into_flags() {
        let plan = parse_install_options(&words(&["riviv.exe", "/bmp", "/gif", "/webp", "/nogif"]));
        assert_eq!(plan.install_flags, (1 << 0) | (1 << 1) | (1 << 8));
        assert_eq!(plan.uninstall_flags, 1 << 1);
    }

    #[test]
    fn isrunas_params_carry_the_raw_remainder() {
        // viv.c:4650-4653: "/isrunas " + cl_start verbatim.
        let cl: Vec<u16> = r#""C:\Program Files\riviv\riviv.exe" /appdata "C:\img dir""#
            .encode_utf16()
            .collect();
        let split = split_command_line(&cl);
        assert_eq!(split.words.len(), 3);
        assert_eq!(split.words[0].text, r"C:\Program Files\riviv\riviv.exe");
        assert!(split.words[0].quoted);
        let rest = String::from_utf16_lossy(&cl[split.rest_after_first..]);
        assert_eq!(rest, r#"/appdata "C:\img dir""#);
        assert_eq!(isrunas_params(&rest), r#"/isrunas /appdata "C:\img dir""#);
    }

    #[test]
    fn split_command_line_handles_quote_rules() {
        // string_get_word (string.c:804-848): quotes toggle, "" is a literal
        // quote, whitespace inside quotes does not break the word.
        let cl: Vec<u16> = "exe /a \"b c\" \"\"q\"\" \"unclosed x"
            .encode_utf16()
            .collect();
        let split = split_command_line(&cl);
        let texts: Vec<&str> = split.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["exe", "/a", "b c", "\"q\"", "unclosed x"]);
        assert_eq!(
            split.words.iter().map(|w| w.quoted).collect::<Vec<_>>(),
            [false, false, true, true, true]
        );
    }

    #[test]
    fn split_command_line_exe_only_has_empty_rest() {
        let cl: Vec<u16> = "exe".encode_utf16().collect();
        let split = split_command_line(&cl);
        assert_eq!(split.words.len(), 1);
        assert_eq!(split.rest_after_first, cl.len());
    }
}
