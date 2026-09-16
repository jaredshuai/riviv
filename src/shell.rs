//! The File-menu shell verb family (#42): the one-line shell commands —
//! Edit/Preview/Print/Properties through ShellExecuteEx verbs, Open File
//! Location through SHOpenFolderAndSelectItems, Set Desktop Wallpaper
//! behind its stobject.dll load — all riding one PIDL-based execute.
//!
//! Upstream: `_viv_file_preview`/`_viv_file_print`/
//! `_viv_file_set_desktop_wallpaper` (viv.c:7679-7712), `_viv_file_edit`/
//! `_viv_open_file_location`/`_viv_properties` (viv.c:7769-7848), all on
//! `os_shell_execute` (os.c:1094-1140). The PIDL route +
//! SEE_MASK_INVOKEIDLIST is the load-bearing detail: verbs resolve
//! through the item's context-menu handlers exactly like an Explorer
//! right-click, which is what makes "properties" show the sheet and
//! "setdesktopwallpaper" reach the shell's wallpaper handler.

use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;

use windows::Win32::Foundation::{CloseHandle, E_ABORT, HWND};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::LoadLibraryA;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILCreateFromPathW, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    SHOpenFolderAndSelectItems, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, s};

use crate::menu::Cmd;
use crate::text::to_wide;
use crate::window::state_of;

/// The shell verb a verb-family command launches (upstream's literal
/// strings: "preview" viv.c:7684, "print" viv.c:7692, "edit" viv.c:7772,
/// "properties" viv.c:7845). `None` for every command with its own
/// handler — Location goes through SHOpenFolderAndSelectItems, Wallpaper
/// carries the stobject gate, Close is the blank path in window.rs.
pub(crate) fn verb_of(cmd: Cmd) -> Option<&'static str> {
    match cmd {
        Cmd::FilePreview => Some("preview"),
        Cmd::FilePrint => Some("print"),
        Cmd::FileEdit => Some("edit"),
        Cmd::FileProperties => Some("properties"),
        _ => None,
    }
}

/// The parent folder of a path, empty when there is none (upstream
/// `string_get_path_part`, string.c:693-712 — the prefix before the last
/// separator; an empty part makes `ILCreateFromPath` return the desktop
/// pidl, its comment at viv.c:7786-7787). One deliberate deviation:
/// upstream slices `C:\f.png` down to `C:` (the per-drive CURRENT
/// directory, not the root); `Path::parent` keeps the `C:\` root, which
/// is what the caller means — README Differences.
pub(crate) fn parent_folder(path: &OsStr) -> Option<std::ffi::OsString> {
    Path::new(path)
        .parent()
        .map(|p| p.as_os_str().to_os_string())
}

/// A nul-terminated wide copy of a path. OsStr→UTF-16 directly — a
/// lossy UTF-8 round trip could mangle an exotic name before the shell
/// ever saw it.
fn wide(path: &OsStr) -> Vec<u16> {
    path.encode_wide().chain(once(0)).collect()
}

/// The viewer's current file (upstream `_viv_current_fd->cFileName`):
/// the displayed playlist entry's path, cloned out of the window state
/// before any shell call. `None` is the bare current-file gate every
/// #42 handler starts with (`if (*cFileName)`, viv.c:7684 etc.).
fn current_path(hwnd: HWND) -> Option<std::ffi::OsString> {
    // SAFETY: read-only state read; the path is cloned out before return
    // so no borrow survives the shell calls.
    let state = unsafe { state_of(hwnd) }?;
    state.nav_current.as_ref().map(|e| e.path.clone())
}

/// `os_shell_execute` (os.c:1094-1140): resolve the file to an item id
/// list, then invoke through SEE_MASK_INVOKEIDLIST (+ NOCLOSEPROCESS and
/// an infinite wait when asked — no #42 caller asks). Err on a null pidl
/// or a failed launch; upstream returns 0 and its callers ignore it —
/// riviv threads the Err for diagnostics, the handlers stay fail-soft.
pub(crate) fn shell_execute(
    hwnd: HWND,
    path: &OsStr,
    verb: Option<&str>,
    params: Option<&str>,
    wait: bool,
) -> Result<(), String> {
    let path_w = wide(path);
    // SAFETY: path_w outlives the call and ILCreateFromPathW only reads
    // it; the null return is checked immediately below, the pidl is
    // task-freed once at the tail (as os.c:1147 does).
    let pidl = unsafe { ILCreateFromPathW(PCWSTR(path_w.as_ptr())) };
    if pidl.is_null() {
        return Err(format!("ILCreateFromPathW({path:?}) returned null"));
    }
    let verb_w = verb.map(to_wide);
    let params_w = params.map(|p| wide(OsStr::new(p)));
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST | if wait { SEE_MASK_NOCLOSEPROCESS } else { 0 },
        hwnd,
        lpVerb: match &verb_w {
            Some(v) => PCWSTR(v.as_ptr()),
            None => PCWSTR::null(),
        },
        lpParameters: match &params_w {
            Some(p) => PCWSTR(p.as_ptr()),
            None => PCWSTR::null(),
        },
        lpIDList: pidl.cast::<std::ffi::c_void>(),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: sei and every string it points at are alive in this frame;
    // the struct is written only by the API.
    let launched = unsafe { ShellExecuteExW(&mut sei) }.is_ok();
    let mut result = Ok(());
    if !launched {
        result = Err(format!("ShellExecuteExW({path:?}, verb {verb:?}) failed"));
    } else if wait && !sei.hProcess.is_invalid() {
        // SAFETY: hProcess came from this successful execute; upstream
        // waits INFINITE then closes the handle (os.c:1140-1143).
        unsafe {
            WaitForSingleObject(sei.hProcess, u32::MAX);
            let _ = CloseHandle(sei.hProcess);
        }
    }
    // SAFETY: pidl came from ILCreateFromPathW above — task-allocated,
    // freed exactly once here.
    unsafe { CoTaskMemFree(Some(pidl.cast::<std::ffi::c_void>().cast_const())) };
    result
}

/// File → Edit / Preview / Print / Properties (upstream's four
/// one-liners, viv.c:7683-7695 / 7769-7775 / 7842-7848): the bare
/// current-file gate, then one verb execute. Fail-soft — upstream ignores
/// the return.
pub(crate) fn run_verb(hwnd: HWND, cmd: Cmd) {
    let Some(path) = current_path(hwnd) else {
        return;
    };
    if let Some(verb) = verb_of(cmd) {
        let _ = shell_execute(hwnd, &path, Some(verb), None, false);
    }
}

/// File → Open File Location (upstream `_viv_open_file_location`,
/// viv.c:7777-7835): select the file in its folder — folder pidl + item
/// pidl into SHOpenFolderAndSelectItems; E_ABORT counts as success
/// (upstream's comment: aborted, QTBar after 10 seconds, viv.c:7809) —
/// and any failure falls back to opening the folder itself.
pub(crate) fn open_file_location(hwnd: HWND) {
    let Some(path) = current_path(hwnd) else {
        return;
    };
    let mut openpathok = false;
    let folder_w = wide(&parent_folder(&path).unwrap_or_default());
    // SAFETY: folder_w outlives the call; null return checked below,
    // pidl task-freed once inside the branch (os.c:7824).
    let folder_idlist = unsafe { ILCreateFromPathW(PCWSTR(folder_w.as_ptr())) };
    if !folder_idlist.is_null() {
        let path_w = wide(&path);
        // SAFETY: path_w outlives the call; null return checked below,
        // pidl task-freed once at the branch tail (os.c:7816).
        let idlist = unsafe { ILCreateFromPathW(PCWSTR(path_w.as_ptr())) };
        if !idlist.is_null() {
            // SAFETY: both pidls are valid at this point; the one-element
            // array is the cidl=1 selection upstream passes (viv.c:7803).
            let hres = unsafe {
                SHOpenFolderAndSelectItems(folder_idlist, Some(&[idlist as *const ITEMIDLIST]), 0)
            };
            // SAFETY: frees the item pidl from this frame (os.c:7816).
            unsafe { CoTaskMemFree(Some(idlist.cast::<std::ffi::c_void>().cast_const())) };
            openpathok = match hres {
                Ok(()) => true,
                // Aborted ≈ success — the folder did open (viv.c:7809-7812).
                Err(e) => e.code() == E_ABORT,
            };
        }
        // SAFETY: frees the folder pidl from this frame (os.c:7824).
        unsafe { CoTaskMemFree(Some(folder_idlist.cast::<std::ffi::c_void>().cast_const())) };
    }
    if !openpathok {
        // The fallback: open the folder itself (viv.c:7826-7834). An
        // empty parent resolves to the desktop pidl inside
        // shell_execute, exactly upstream's note (viv.c:7786-7787).
        let _ = shell_execute(
            hwnd,
            &parent_folder(&path).unwrap_or_default(),
            None,
            None,
            false,
        );
    }
}

/// File → Set Desktop Wallpaper (upstream `_viv_file_set_desktop_wallpaper`,
/// viv.c:7695-7712): the verb's handler lives in stobject.dll — upstream
/// LoadLibrary's it once, caches the verdict, and skips the execute when
/// the load fails. No default key upstream: "needs a confirmation
/// dialog" (viv.c:982).
pub(crate) fn set_desktop_wallpaper(hwnd: HWND) {
    static STOBJECT_LOADED: OnceLock<bool> = OnceLock::new();
    let loaded = *STOBJECT_LOADED.get_or_init(|| {
        // SAFETY: a by-name library load; the module intentionally stays
        // loaded for the process lifetime (upstream never frees it).
        unsafe { LoadLibraryA(s!("stobject.dll")) }.is_ok()
    });
    if !loaded {
        return;
    }
    let Some(path) = current_path(hwnd) else {
        return;
    };
    // Fail-soft: upstream ignores os_shell_execute's return here too.
    let _ = shell_execute(hwnd, &path, Some("setdesktopwallpaper"), None, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_of_maps_the_four_verb_commands_and_nothing_else() {
        // Upstream's literal verbs (viv.c:7684/7692/7772/7845); the other
        // #42 commands ride their own handlers, and every non-#42 command
        // must stay unmapped so run_verb can never fire a bogus verb.
        assert_eq!(verb_of(Cmd::FilePreview), Some("preview"));
        assert_eq!(verb_of(Cmd::FilePrint), Some("print"));
        assert_eq!(verb_of(Cmd::FileEdit), Some("edit"));
        assert_eq!(verb_of(Cmd::FileProperties), Some("properties"));
        assert_eq!(verb_of(Cmd::FileOpenFileLocation), None);
        assert_eq!(verb_of(Cmd::FileSetDesktopWallpaper), None);
        assert_eq!(verb_of(Cmd::FileClose), None);
        assert_eq!(verb_of(Cmd::FileExit), None);
        assert_eq!(verb_of(Cmd::ViewRefresh), None);
    }

    #[test]
    fn parent_folder_cuts_at_the_last_separator_like_upstream() {
        // string.c:693-712: the prefix before the final separator. The
        // deliberate divergence is the root parent — upstream gives "C:"
        // (per-drive current dir), Path::parent keeps the root.
        assert_eq!(
            parent_folder(OsStr::new(r"C:\dir\sub\file.png")),
            Some(r"C:\dir\sub".into())
        );
        assert_eq!(
            parent_folder(OsStr::new(r"C:\file.png")),
            Some(r"C:\".into())
        );
        assert_eq!(
            parent_folder(OsStr::new("file.png")),
            Some("".into()) // std keeps the empty parent; upstream copies ""
        );
    }
}
