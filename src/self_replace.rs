//! The portable self-replace helper (#237; ADR 0009 D5) — the Sumatra
//! helper pattern mapped to riviv (spike #235, codex #236's corrected
//! three-step adopted).
//!
//! The OLD instance downloads the verified new exe beside itself as
//! `riviv-update-<tag>.exe`, spawns THAT file as the helper with
//! `-update-self-to <original path>`, and exits through its normal
//! WM_CLOSE path. The helper (this same exe image, running from the
//! staging name) waits on the PARENT's inherited handle — not the
//! single-instance mutex: with `multiple_instances=1` no mutex exists
//! at all, and waiting on it would pass immediately while the parent's
//! image is still live (codex #236 P1). Then: copy own bytes to
//! `<orig>.tmp` → `ReplaceFileW(orig, tmp, backup=orig.old)` (the
//! one-call form of the adopted rename dance: old bytes stay in place
//! until the replacement lands, and the backup is built in) → launch
//! the new exe → delete `<orig>.old` → exit. The helper's own staging
//! file cannot be deleted while it runs, so every startup sweeps the
//! `riviv-update-*.exe` residuals beside the exe (the staged-ini
//! cleanup discipline; the installed flavor's %TEMP% staging rides the
//! same sweep).
//!
//! The `-update-self-to` short-circuit lives in `run()` BEFORE the
//! single-instance mutex (#109): a helper launch while the old instance
//! still owns the `RIVIV` mutex would otherwise be forwarded to the old
//! window as a "second-instance" command line and silently discarded.
//!
//! Layering: the helper-line parse and the sweep predicate are pure and
//! tested; the wait/replace/launch sequence is the unsafe shell (its
//! every failure keeps the old exe in place — fail-soft box + stderr
//! breadcrumb, never a crash).

use std::path::Path;
use std::path::PathBuf;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::HANDLE_FLAG_INHERIT;
use windows::Win32::Foundation::SetHandleInformation;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Storage::FileSystem::ReplaceFileW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::Threading::OpenProcess;
use windows::Win32::System::Threading::PROCESS_SYNCHRONIZE;
use windows::core::PCWSTR;

use crate::cli::is_ws;
use crate::cli::next_word;
use crate::text::to_wide;

/// The helper switch (ASCII case-insensitive, `-` or `/` prefix — the
/// tokenizer's normal switch shape). An internal maintenance switch,
/// like the installer's `/install` family: not in the README usage.
const SELF_UPDATE_SWITCH: &str = "update-self-to";

/// How long the helper waits for the parent to exit before giving up
/// (fail-soft: the old version keeps running untouched; the residual
/// staging file is swept by the next startup). The parent's last act
/// after spawning the helper is its own WM_CLOSE, so this is a hang
/// guard, not a race budget.
const PARENT_WAIT_MS: u32 = 60_000;

/// The staging-name prefix every startup sweeps (portable downloads
/// beside the exe, installed downloads in %TEMP% — one prefix, two
/// directories).
const STAGING_PREFIX: &str = "riviv-update-";

/// ASCII case-insensitive compare (the tokenizer already stripped the
/// switch prefix; `cli::eq_switch` is private there, and one local
/// helper keeps this module's parse self-contained).
fn eq_word(word: &[u16], name: &str) -> bool {
    word.len() == name.len()
        && word.iter().zip(name.as_bytes()).all(|(c, b)| {
            let lower = if (b'A' as u16..=b'Z' as u16).contains(c) {
                *c + 32
            } else {
                *c
            };
            lower == *b as u16
        })
}

/// Parse a raw command line as the helper's launch line: the
/// `-update-self-to` switch (unquoted, `-`/`/`-prefixed, dot-free —
/// `cli::parse`'s switch test) with the original exe path as the next
/// word. Returns `None` for every non-helper line (the normal startup
/// path), `Some(path)` for a helper line (an EMPTY path = a dangling
/// switch: the helper runs and fails with the malformed-argument
/// breadcrumb rather than opening a window).
pub(crate) fn parse_helper_line(cl: &[u16]) -> Option<Vec<u16>> {
    let mut i = 0usize;
    while i < cl.len() && is_ws(cl[i]) {
        i += 1;
    }
    let _exe = next_word(cl, &mut i);
    let mut pending = false;
    while i < cl.len() {
        let started_quoted = cl[i] == u16::from(b'"');
        let word = next_word(cl, &mut i);
        if pending {
            return Some(word.text);
        }
        let is_switch_word = !started_quoted
            && matches!(word.text.first(), Some(&c) if c == u16::from(b'/') || c == u16::from(b'-'))
            && !word.text.contains(&u16::from(b'.'));
        if is_switch_word && eq_word(&word.text[1..], SELF_UPDATE_SWITCH) {
            pending = true;
        }
    }
    if pending { Some(Vec::new()) } else { None }
}

/// The startup-sweep predicate: staging residuals (`riviv-update-*`
/// `.exe`) and the replace backup (`<exe file name>.old`). Pure so the
/// naming contract is testable.
pub(crate) fn is_staging_residual(file_name: &str, exe_file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    (lower.starts_with(STAGING_PREFIX) && lower.ends_with(".exe"))
        || lower == format!("{}.old", exe_file_name.to_ascii_lowercase())
}

/// The parent half: spawn the verified staging exe as the helper with
/// an inherited self-handle as its stdin, so the helper can wait out
/// this process. The caller then closes through the normal WM_CLOSE
/// path (config save included) — this function only spawns. Returns
/// false on a spawn failure (the caller's fail-soft box; the staging
/// file stays for the next attempt or the sweep).
pub(crate) fn spawn_helper(staging: &Path, orig: &Path) -> bool {
    use windows::Win32::System::Threading::CreateProcessW;
    use windows::Win32::System::Threading::PROCESS_INFORMATION;
    use windows::Win32::System::Threading::STARTF_USESTDHANDLES;
    use windows::Win32::System::Threading::STARTUPINFOW;

    // The wait object: a handle to OURSELVES the child can wait on —
    // not the mutex (with multiple_instances=1 no mutex exists, and it
    // would pass early while our image is still live; codex #236 P1).
    // SAFETY: OpenProcess on our own PID; PROCESS_SYNCHRONIZE is the
    // only right granted (the child cannot terminate or write us).
    let parent = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, GetCurrentProcessId()) } {
        Ok(h) => h,
        Err(e) => {
            eprintln!("self-update: OpenProcess(self) failed: {e}");
            return false;
        }
    };
    // SAFETY: the handle is ours and live; marking it inheritable is a
    // flag change on kernel state we own.
    if unsafe { SetHandleInformation(parent, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }.is_err()
    {
        eprintln!("self-update: SetHandleInformation failed");
        // SAFETY: closing the handle we just opened (no child spawned).
        let _ = unsafe { CloseHandle(parent) };
        return false;
    }
    let mut cmdline = to_wide(&format!(
        "\"{}\" -{} \"{}\"",
        staging.display(),
        SELF_UPDATE_SWITCH,
        orig.display()
    ));
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        // The inheritance channel: STARTF_USESTDHANDLES + hStdInput is
        // the classic discoverable way to pass one handle to a GUI
        // child (the helper reads it back via GetStdHandle; a viewer
        // window never reads stdin, so the slot is free).
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: parent,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();
    // SAFETY: cmdline is a fresh NUL-terminated wide buffer handed over
    // as a mutable pointer (CreateProcessW's documented may-write
    // contract); si/pi are live out-params; bInheritHandles=TRUE is
    // what crosses the stdin handle; no env/cwd override — the helper
    // resolves every path absolutely itself.
    let spawned = unsafe {
        CreateProcessW(
            PCWSTR(cmdline.as_ptr()),
            Some(windows::core::PWSTR(cmdline.as_mut_ptr())),
            None,
            None,
            true,
            Default::default(),
            None,
            None,
            &si,
            &mut pi,
        )
    };
    if let Err(e) = spawned {
        eprintln!("self-update: CreateProcessW(helper) failed: {e}");
        // SAFETY: closing the handle we opened (the child never came).
        let _ = unsafe { CloseHandle(parent) };
        return false;
    }
    // SAFETY: the child's returned thread/process handles are ours to
    // close — we never wait on the helper from here (the dependency
    // runs the other way: the helper waits on us).
    unsafe {
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
    }
    // The raw parent HANDLE stays open for the child (a raw HANDLE has
    // no Drop — nothing to leak-guard); the OS reclaims it at our exit.
    // Closing it here would orphan the child's wait object.
    true
}

/// The helper half: wait for the parent, swap the files, launch the new
/// exe, clean up. Returns the process exit code (0 = the update
/// completed). Every failure path leaves the OLD exe in place.
pub(crate) fn run_helper(orig: &[u16]) -> u32 {
    let orig_str = String::from_utf16_lossy(orig);
    if orig_str.is_empty() {
        eprintln!("self-update helper: -update-self-to with an empty path (malformed launch)");
        return 2;
    }
    let orig = PathBuf::from(orig_str);
    let Ok(helper_exe) = std::env::current_exe() else {
        eprintln!("self-update helper: current_exe failed");
        return 2;
    };
    if !wait_for_parent() {
        eprintln!(
            "self-update helper: parent did not exit within {PARENT_WAIT_MS} ms — aborting (old version untouched)"
        );
        return 2;
    }
    // The bytes: the helper reads its own on-disk image (readable while
    // running; only rename/delete of a running image are blocked — and
    // this file is never renamed).
    let Ok(bytes) = std::fs::read(&helper_exe) else {
        eprintln!(
            "self-update helper: cannot read own image {}",
            helper_exe.display()
        );
        return 2;
    };
    let tmp = sibling(&orig, ".tmp");
    let old = sibling(&orig, ".old");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        fail_soft(&format!("writing {}", tmp.display()), &e.to_string());
        return 1;
    }
    // ReplaceFileW(orig ← tmp, backup = old): the one-call form of the
    // adopted sequence — orig's bytes survive untouched unless the
    // replacement lands, and the backup is written in. Same volume by
    // construction (both beside orig); NTFS rename pairs are atomic,
    // the FAT family's guarantees are weaker (ADR 0009 D5's recorded
    // boundary).
    let orig_w = to_wide(&orig.to_string_lossy());
    let tmp_w = to_wide(&tmp.to_string_lossy());
    let old_w = to_wide(&old.to_string_lossy());
    // SAFETY: all three buffers are NUL-terminated and live for the
    // call; ReplaceFileW fails (returning an error) rather than half-
    // replacing, and orig is not this process's own image (we run from
    // the staging name), so no running-image rename hazard applies.
    if let Err(e) = unsafe {
        ReplaceFileW(
            PCWSTR(orig_w.as_ptr()),
            PCWSTR(tmp_w.as_ptr()),
            PCWSTR(old_w.as_ptr()),
            Default::default(),
            None,
            None,
        )
    } {
        let _ = std::fs::remove_file(&tmp);
        fail_soft(&format!("replacing {}", orig.display()), &format!("{e}"));
        return 1;
    }
    // Launch the new exe (fail-soft: the replacement already succeeded;
    // the user can start it manually — the shortcut still points at
    // orig).
    if let Err(e) = crate::shell::shell_execute(
        windows::Win32::Foundation::HWND::default(),
        orig.as_os_str(),
        None,
        None,
        false,
    ) {
        eprintln!("self-update helper: launching the new exe failed: {e}");
    }
    // The old bytes: the parent has exited, so nothing holds orig.old
    // (unless a straggler instance still runs the old image — then the
    // delete fails harmlessly and the next startup's sweep retries).
    let _ = std::fs::remove_file(&old);
    0
}

/// Wait on the inherited parent handle. Returns false on timeout (or a
/// missing handle = launched outside the update flow).
fn wait_for_parent() -> bool {
    use windows::Win32::System::Console::GetStdHandle;
    use windows::Win32::System::Console::STD_INPUT_HANDLE;
    use windows::Win32::System::Threading::WaitForSingleObject;

    // SAFETY: a pure query of this process's stdin handle slot.
    let stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let Ok(h) = stdin else {
        eprintln!(
            "self-update helper: no inherited stdin handle — launched outside the update flow?"
        );
        return false;
    };
    // SAFETY: the handle is the inherited parent handle (an invalid
    // value fails the wait, not the process); the wait is bounded.
    let wait = unsafe { WaitForSingleObject(h, PARENT_WAIT_MS) };
    if wait != WAIT_OBJECT_0 {
        return false;
    }
    // SAFETY: closing the inherited handle we just consumed (single
    // owner — the parent's copy died with the parent).
    let _ = unsafe { CloseHandle(h) };
    true
}

/// A one-off failure box (the helper has no window of its own; the
/// running old instance is gone or going, so this box is the only user
/// surface) + the stderr breadcrumb.
fn fail_soft(stage: &str, err: &str) {
    eprintln!("self-update helper: {stage}: {err}");
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let text = to_wide(crate::loc::get(crate::loc::Id::UpdateReplaceFailedText));
    let caption = to_wide(crate::loc::get(crate::loc::Id::AppName));
    // SAFETY: both buffers outlive the modal call; no owner window
    // exists in the helper.
    let _ = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_ICONERROR | MB_OK,
        )
    };
}

/// `<orig><ext>` as a sibling path (the .tmp/.old naming).
fn sibling(orig: &Path, ext: &str) -> PathBuf {
    let mut s = orig.as_os_str().to_os_string();
    s.push(ext);
    PathBuf::from(s)
}

/// The startup sweep: delete staging residuals (failed/interrupted
/// downloads, the previous helper's image, the previous backup) in
/// `dir`. Failures are ignored by design — a running helper's image
/// cannot be deleted, and the next startup retries.
pub(crate) fn sweep_staging(dir: &Path) {
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "riviv.exe".to_string());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_staging_residual(&name, &exe_name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn s(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    #[test]
    fn the_helper_line_is_recognized_in_all_spellings() {
        for cl in [
            "riviv-update-v0.9.0.exe -update-self-to C:\\tools\\riviv.exe",
            "riviv-update-v0.9.0.exe /update-self-to C:\\tools\\riviv.exe",
            "riviv-update-v0.9.0.exe -UPDATE-SELF-TO C:\\tools\\riviv.exe",
        ] {
            assert_eq!(
                s(parse_helper_line(&w(cl)).as_deref().unwrap_or(&[])),
                "C:\\tools\\riviv.exe",
                "{cl}"
            );
        }
        // A quoted path arrives pre-stripped by the tokenizer (paths
        // with spaces launch quoted).
        assert_eq!(
            s(parse_helper_line(&w(
                "h.exe -update-self-to \"C:\\Program Files\\riviv\\riviv.exe\""
            ))
            .as_deref()
            .unwrap_or(&[])),
            "C:\\Program Files\\riviv\\riviv.exe"
        );
    }

    #[test]
    fn every_other_line_is_not_a_helper_line() {
        // Normal startups, file opens, the config-switch family — and a
        // QUOTED switch word (the tokenizer's was_quote rule: it is a
        // file word, never a switch).
        for cl in [
            "riviv.exe",
            "riviv.exe a.png",
            "riviv.exe /fullscreen a.png",
            "riviv.exe -update-self.png",      // dotted: a file word
            "riviv.exe \"-update-self-to\" x", // quoted switch = file word
            "riviv-update-v0.9.0.exe a.png -slideshow",
        ] {
            assert!(parse_helper_line(&w(cl)).is_none(), "{cl}");
        }
    }

    #[test]
    fn a_dangling_switch_yields_the_empty_path() {
        // The helper must run and fail with the malformed-argument
        // breadcrumb, not fall through into a full viewer startup.
        assert_eq!(
            s(parse_helper_line(&w("h.exe -update-self-to"))
                .as_deref()
                .unwrap_or(&[])),
            ""
        );
    }

    #[test]
    fn the_sweep_matches_staging_and_backup_names() {
        assert!(is_staging_residual("riviv-update-v0.9.0.exe", "riviv.exe"));
        assert!(is_staging_residual("RIVIV-UPDATE-V0.9.0.EXE", "riviv.exe"));
        assert!(is_staging_residual("riviv.exe.old", "riviv.exe"));
        assert!(is_staging_residual("MyViewer.exe.old", "MyViewer.exe"));
        // Not residuals: the exe itself, ordinary files, a released
        // portable asset (no -update- infix), a differently-named old
        // file. The installed flavor's staged Setup DOES match the
        // prefix by design (once Setup exits, it is stale).
        assert!(!is_staging_residual("riviv.exe", "riviv.exe"));
        assert!(!is_staging_residual("riviv-update-notes.txt", "riviv.exe"));
        assert!(!is_staging_residual(
            "riviv-0.9.0-portable.exe",
            "riviv.exe"
        ));
        assert!(!is_staging_residual("riviv.old", "riviv.exe"));
        assert!(is_staging_residual(
            "riviv-update-v0.9.0.exe",
            "MyViewer.exe"
        ));
    }

    #[test]
    fn sweep_deletes_only_the_residuals() {
        // The backup name derives from the RUNNING exe's file name (the
        // impl reads current_exe — under cargo test that is the test
        // binary, so the fixture mirrors the same derivation).
        let exe_name = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let dir = std::env::temp_dir().join(format!(
            "riviv-t237-sweep-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let keep = dir.join("riviv.exe");
        let residual = dir.join("riviv-update-v9.9.9.exe");
        let backup = dir.join(format!("{exe_name}.old"));
        std::fs::write(&keep, b"exe").unwrap();
        std::fs::write(&residual, b"stale helper").unwrap();
        std::fs::write(&backup, b"old bytes").unwrap();
        sweep_staging(&dir);
        assert!(keep.is_file(), "an unrelated exe is never swept");
        assert!(!residual.exists());
        assert!(!backup.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
