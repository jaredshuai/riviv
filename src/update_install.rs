//! The install half of the update flow (#237; ADR 0009): download the
//! chosen release asset streaming to disk, verify it against the
//! domain-separated Ed25519 signature (`sig.rs`), then hand off to the
//! flavor's installer — the NSIS Setup (installed copies) or the
//! portable self-replace helper (`self_replace.rs`).
//!
//! Layering per the quality tiers: the asset-selection math (tag +
//! flavor + language → asset file name), the staging names, the URL
//! construction and the 128 MB budget guard are pure functions under
//! test below; the WinHTTP streaming and the hand-off spawns are the
//! thin unsafe shells. Failure semantics follow ADR 0009's table: every
//! failure keeps the running version untouched; a verify failure
//! additionally DELETES the download and never offers the releases page
//! (the channel itself may be poisoned — handing the user a ladder back
//! to it would turn the security boundary into a click-through).

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IDOK, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MessageBoxW,
    PostMessageW, WM_APP, WM_CLOSE,
};
use windows::core::PCWSTR;

use crate::loc;
use crate::loc::Language;
use crate::text::to_wide;

/// The flavor the update takes (ADR 0009 D4/D5): an installed copy runs
/// its language's Setup; a portable copy self-replaces in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    /// An `Uninstall.exe` sits beside the running exe — the marker the
    /// NSIS-staged `/install` pass has always written (assoc.rs copies
    /// it into the install dir; `install_start_menu_shortcuts` already
    /// used the same adjacency test).
    Installed,
    /// No marker: a portable exe (any directory, USB drives included).
    Portable,
}

/// The release asset file name for one update (pure; the names are the
/// ritual's own — pinned against the real v0.8.0 release listing in the
/// tests). `tag` is the raw tag_name spelling ("v0.9.0").
pub(crate) fn asset_name(tag: &str, flavor: Flavor, lang: Language) -> String {
    let ver = tag.strip_prefix('v').unwrap_or(tag);
    match flavor {
        Flavor::Portable => format!("riviv-{ver}-portable.exe"),
        Flavor::Installed => {
            let locale = match lang {
                Language::English => "en-US",
                Language::ChineseSimplified => "zh-CN",
                Language::ChineseTraditional => "zh-TW",
            };
            format!("riviv-{ver}.x64.{locale}-Setup.exe")
        }
    }
}

/// The staged download's file name (`riviv-update-<tag>-<pid>.exe`) —
/// the portable helper IS this file, and the startup sweep matches the
/// `riviv-update-` prefix (ADR 0009 D5). The pid suffix is the
/// per-flight uniqueness: with `multiple_instances=1` two instances can
/// accept the same update, and a shared deterministic name would let
/// one flight's truncate land under another's read/verify/spawn —
/// worst case handing the helper a partial copy (review #238 codex).
pub(crate) fn staging_file_name(tag: &str) -> String {
    format!("riviv-update-{tag}-{}.exe", std::process::id())
}

/// The download host (the release-assets redirect chain starts at
/// github.com; WinHTTP's default policy follows the same-scheme 302s —
/// spike #235 E3, zero new redirect code).
pub(crate) const DL_HOST: &str = "github.com";

/// The URL path for one asset (or its `.sig`, `with_sig = true`).
pub(crate) fn dl_path(tag: &str, asset: &str, with_sig: bool) -> String {
    if with_sig {
        format!("/jaredshuai/riviv/releases/download/{tag}/{asset}.sig")
    } else {
        format!("/jaredshuai/riviv/releases/download/{tag}/{asset}")
    }
}

/// The asset-stream budget (ADR 0009 D3): the real assets are ~8.5 MB;
/// a server that streams past this cap is a hostile stream, and the
/// download fails without an unbounded allocation. Pure so the boundary
/// is testable (the D4 64 KB JSON budget's sibling).
pub(crate) const MAX_ASSET_BYTES: usize = 128 * 1024 * 1024;
pub(crate) fn asset_fits(have: usize, add: usize) -> bool {
    // checked_add: same wrap guard as body_fits (adversarial #211 P3).
    have.checked_add(add).is_some_and(|t| t <= MAX_ASSET_BYTES)
}

/// Where the staged download lands (ADR 0009 D3): the portable flavor
/// MUST stage beside the exe — the self-replace renames are same-volume
/// only, and a USB-portable's %TEMP% is a different volume (codex #236
/// P2). The installed flavor stages in %TEMP% because a standard-user
/// process cannot write into Program Files (the Setup elevates itself
/// when it needs to).
pub(crate) fn staging_dir(flavor: Flavor, exe_dir: &std::path::Path) -> PathBuf {
    match flavor {
        Flavor::Portable => exe_dir.to_path_buf(),
        Flavor::Installed => std::env::temp_dir(),
    }
}

/// The flavor decision from the exe's directory: `Uninstall.exe`
/// adjacency (ADR 0009 D4 — the registry-uninstall-key and ini keys are
/// NOT discriminants: the current `riviv.nsi` never writes the former
/// and `appdata` is an orthogonal user option; codex #236 P2).
pub(crate) fn detect_flavor(exe_dir: &std::path::Path) -> Flavor {
    if exe_dir.join("Uninstall.exe").is_file() {
        Flavor::Installed
    } else {
        Flavor::Portable
    }
}

/// The install flow's reply notification (WM_APP+5 — the check's +4
/// sibling; same payload-free contract: the verdict waits in
/// `INSTALL_QUEUE`, so a forged message can only find it empty).
pub(crate) const INSTALL_REPLY_MESSAGE: u32 = WM_APP + 5;

/// One install at a time (the check's CHECK_RUNNING sibling); released
/// by the UI thread when it CONSUMES a verdict.
static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

/// The verdict hand-off (the check's REPLY_QUEUE sibling).
static INSTALL_QUEUE: Mutex<VecDeque<InstallOutcome>> = Mutex::new(VecDeque::new());

/// What the UI thread should do after an install flight completes.
#[derive(Debug)]
pub(crate) enum InstallOutcome {
    /// Verified and staged — run the flavor's handoff (portable: the
    /// self-replace helper; installed: the Setup relay after one
    /// confirm box).
    Ready {
        flavor: Flavor,
        /// The verified staging file (`riviv-update-<tag>-<pid>.exe`).
        staged: PathBuf,
        /// The running exe's directory (the Setup relay's /installdir;
        /// the portable arm reads current_exe itself).
        install_dir: PathBuf,
        /// The staging GUARD: an open no-delete-share handle on the
        /// staged file, held from download through the handoff so a
        /// concurrent startup sweep cannot delete a live flight's
        /// file (review #238 codex r3). Dropped right after the
        /// handoff call returns — by then the helper/Setup image is
        /// mapped and protects itself.
        guard: Option<std::fs::File>,
    },
    /// Network/disk class (the staging partial is already deleted).
    DownloadFailed,
    /// The hard boundary: the signature did not verify (the download is
    /// already deleted; nothing runs, no ladder to the releases page).
    SigFailed,
}

/// The YES arm of the three-button Available box (#237; ADR 0009 D9):
/// one background flight — select asset, fetch sig + asset, verify,
/// park the verdict, post the notification. The UI thread stays free.
pub(crate) fn begin_install(hwnd: HWND, tag: String) {
    if INSTALL_RUNNING.swap(true, Ordering::SeqCst) {
        eprintln!("update install: a flight is already running — click ignored");
        return;
    }
    // HWND crosses the spawn as an isize (the check's dispatch
    // precedent — windows-rs marks the raw pointer non-Send).
    let hwnd_raw = hwnd.0 as isize;
    match std::thread::Builder::new()
        .name("update-install".to_string())
        .spawn(move || {
            let outcome = run_install(&tag);
            if let Ok(mut queue) = INSTALL_QUEUE.lock() {
                queue.clear();
                queue.push_back(outcome);
            }
            // SAFETY: hwnd was live when begin_install ran on the UI
            // thread; if the window has died since, the post fails and
            // the verdict stays unclaimed (nothing crosses the message
            // boundary).
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(hwnd_raw as *mut _)),
                    INSTALL_REPLY_MESSAGE,
                    Default::default(),
                    Default::default(),
                )
            };
        }) {
        Ok(_) => {}
        Err(e) => {
            INSTALL_RUNNING.store(false, Ordering::SeqCst);
            eprintln!("update install: worker spawn failed: {e}");
            crate::update::show_box(
                HWND(hwnd_raw as *mut _),
                loc::get(loc::Id::UpdateDownloadFailedText),
                MB_ICONERROR | MB_OK,
            );
        }
    }
}

/// The worker body: the download+verify pipeline. Every failure path
/// deletes its partials and answers a user-level outcome — the running
/// version is never touched (ADR 0009 D3/D6).
fn run_install(tag: &str) -> InstallOutcome {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("update install: current_exe failed");
        return InstallOutcome::DownloadFailed;
    };
    let Some(exe_dir) = exe.parent().map(Path::to_path_buf) else {
        eprintln!("update install: the exe has no parent directory");
        return InstallOutcome::DownloadFailed;
    };
    let flavor = detect_flavor(&exe_dir);
    let asset = asset_name(tag, flavor, loc::current_language());
    let staged = staging_dir(flavor, &exe_dir).join(staging_file_name(tag));
    let sig_path = {
        let mut s = staged.as_os_str().to_os_string();
        s.push(".sig");
        PathBuf::from(s)
    };
    eprintln!(
        "update install: fetching {asset} for {tag} (flavor={flavor:?}) -> {}",
        staged.display()
    );
    // 1. The signature: raw 64 B, exactly (a hostile or error body of
    //    any other shape fails here — a 404's HTML exceeds the cap).
    let sig = match fetch_sig(&dl_path(tag, &asset, true)) {
        Ok(sig) => sig,
        Err(_) => return InstallOutcome::DownloadFailed,
    };
    // 2. The asset, streaming to disk within the 128 MB budget; a
    //    failed transfer deletes its partial. The returned open handle
    //    is the staging GUARD (no delete-sharing) — held through
    //    verify and into the handoff so a concurrent startup sweep
    //    cannot eat a live flight's file (review #238 codex r3).
    let guard = match fetch_to_file(&dl_path(tag, &asset, false), &staged) {
        Ok(g) => g,
        Err(_) => return InstallOutcome::DownloadFailed,
    };
    // 3. Verify on the completed byte stream (ADR 0009 D3: verify after
    //    the stream lands, never mid-flight). The message binds tag +
    //    asset name, so a replayed or substituted artifact fails here.
    let bytes = match std::fs::read(&staged) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("update install: reading back {}: {e}", staged.display());
            drop(guard); // release our own no-delete handle first
            let _ = std::fs::remove_file(&staged);
            return InstallOutcome::DownloadFailed;
        }
    };
    if !crate::sig::verify(tag, &asset, &bytes, &sig) {
        eprintln!(
            "update install: signature check FAILED — {asset} tag={tag} against pubkey {}… (download deleted, nothing executed)",
            &crate::sig::PUBKEY_HEX[..16]
        );
        drop(guard); // release our own no-delete handle first
        let _ = std::fs::remove_file(&staged);
        let _ = std::fs::remove_file(&sig_path);
        return InstallOutcome::SigFailed;
    }
    // The sig served its purpose (the helper does not re-verify — the
    // staged bytes it copies are these verified bytes).
    let _ = std::fs::remove_file(&sig_path);
    InstallOutcome::Ready {
        flavor,
        staged,
        install_dir: exe_dir,
        guard: Some(guard),
    }
}

/// Runs on the UI thread from wnd_proc's INSTALL_REPLY_MESSAGE arm:
/// claim the queued verdict and act (ADR 0009's handoff semantics).
pub(crate) fn on_install_reply(hwnd: HWND) {
    let Some(outcome) = INSTALL_QUEUE.lock().ok().and_then(|mut q| q.pop_front()) else {
        return;
    };
    INSTALL_RUNNING.store(false, Ordering::SeqCst);
    match outcome {
        InstallOutcome::Ready {
            flavor: Flavor::Portable,
            staged,
            guard,
            ..
        } => {
            // No second confirm — the three-button YES already committed
            // (P4=A: the asker gets the full automatic channel).
            let Ok(orig) = std::env::current_exe() else {
                eprintln!("update install: current_exe failed at handoff");
                crate::update::show_box(
                    hwnd,
                    loc::get(loc::Id::UpdateReplaceFailedText),
                    MB_ICONERROR | MB_OK,
                );
                drop(guard); // release our own no-delete handle first
                let _ = std::fs::remove_file(&staged);
                return;
            };
            if crate::self_replace::spawn_helper(&staged, &orig) {
                // The helper's image is mapped now — it protects itself;
                // the guard's job (sweep protection) is done.
                drop(guard);
                close_self(hwnd);
            } else {
                drop(guard); // release our own no-delete handle first
                crate::update::show_box(
                    hwnd,
                    loc::get(loc::Id::UpdateReplaceFailedText),
                    MB_ICONERROR | MB_OK,
                );
                let _ = std::fs::remove_file(&staged);
            }
        }
        InstallOutcome::Ready {
            flavor: Flavor::Installed,
            staged,
            install_dir,
            guard,
        } => {
            // The one pre-launch notice (the spike's adopted box):
            // riviv closes and the installer runs. Cancel keeps the
            // current version and cleans the staging file.
            let text = to_wide(loc::get(loc::Id::UpdateInstallConfirmText));
            let caption = to_wide(loc::get(loc::Id::AppName));
            // SAFETY: both buffers outlive the modal call; hwnd is the
            // live owner.
            let choice = unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    PCWSTR(caption.as_ptr()),
                    MB_ICONINFORMATION | MB_OKCANCEL,
                )
            };
            if choice == IDOK {
                // First the Setup, THEN our own close (codex #236 P2:
                // exiting first would leave nobody to launch it). The
                // Setup's staged /install elevates and closes running
                // viewers itself, so our exit is a courtesy, not a
                // handshake the installer depends on.
                let params = format!("/installdir \"{}\"", install_dir.display());
                if let Err(e) = crate::shell::shell_execute(
                    hwnd,
                    staged.as_os_str(),
                    None,
                    Some(&params),
                    false,
                ) {
                    eprintln!("update install: launching the Setup failed: {e}");
                    drop(guard); // release our own no-delete handle first
                    let _ = std::fs::remove_file(&staged);
                    crate::update::show_box(
                        hwnd,
                        loc::get(loc::Id::UpdateReplaceFailedText),
                        MB_ICONERROR | MB_OK,
                    );
                    return;
                }
                // The Setup's image is mapped (its process creation ran
                // inside shell_execute) — it protects itself now.
                drop(guard);
                close_self(hwnd);
            } else {
                drop(guard); // release our own no-delete handle first
                let _ = std::fs::remove_file(&staged);
            }
        }
        InstallOutcome::DownloadFailed => {
            crate::update::show_box(
                hwnd,
                loc::get(loc::Id::UpdateDownloadFailedText),
                MB_ICONWARNING | MB_OK,
            );
        }
        InstallOutcome::SigFailed => {
            crate::update::show_box(
                hwnd,
                loc::get(loc::Id::UpdateSigFailedText),
                MB_ICONERROR | MB_OK,
            );
        }
    }
}

/// The polite self-exit: through the normal WM_CLOSE path (config save
/// included), like the user clicking X.
fn close_self(hwnd: HWND) {
    // SAFETY: hwnd is the live owner on the UI thread; the close path
    // runs in the pump as for any user close.
    let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, Default::default(), Default::default()) };
}

/// Fetch one raw 64 B signature into memory (the sig must be EXACTLY
/// 64 B — any other length is a hostile or error body).
fn fetch_sig(path: &str) -> Result<[u8; 64], ()> {
    let mut buf: Vec<u8> = Vec::new();
    crate::update::winhttp_get(DL_HOST, path, &mut |chunk, _| {
        if buf.len() + chunk.len() > 64 {
            eprintln!("update install: the .sig body exceeds 64 B — not a signature");
            return Err(());
        }
        buf.extend_from_slice(chunk);
        Ok(())
    })?;
    let n = buf.len();
    buf.try_into().map_err(|_| {
        eprintln!("update install: the .sig body is {n} B, expected 64");
    })
}

/// Stream one asset to disk within the 128 MB budget, deleting the
/// partial on any failure (the network-failure row of ADR 0009 D6).
///
/// The file is created WITHOUT delete-sharing (`FILE_SHARE_READ` only —
/// a std `File::create` shares delete POSIX-style) and the handle is
/// RETURNED OPEN as the flight's staging guard: from the first written
/// byte until the handoff drops it, another instance's startup sweep
/// cannot delete this staging file (review #238 codex r3/cubic P2 —
/// the sweep race degrades to a harmless no-op instead of eating a
/// live flight's download/verify/launch).
fn fetch_to_file(path: &str, dest: &Path) -> Result<std::fs::File, ()> {
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::Foundation::GENERIC_WRITE;
    use windows::Win32::Storage::FileSystem::{
        CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
    };
    let dest_w = to_wide(&dest.to_string_lossy());
    // SAFETY: dest_w is NUL-terminated and live; the Result-wrapped
    // handle is checked before use and owned by the File built from it
    // (closed exactly once on drop).
    let raw = unsafe {
        CreateFileW(
            PCWSTR(dest_w.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_READ,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    let raw = match raw {
        Ok(h) => h,
        Err(e) => {
            eprintln!("update install: create {}: {e}", dest.display());
            return Err(());
        }
    };
    // SAFETY: raw is a fresh valid handle we own; the File takes
    // exclusive ownership (no double close).
    let mut file = unsafe { std::fs::File::from_raw_handle(raw.0 as _) };
    let mut total = 0usize;
    let result = crate::update::winhttp_get(DL_HOST, path, &mut |chunk, _| {
        if !asset_fits(total, chunk.len()) {
            eprintln!(
                "update install: the asset exceeds the {} B budget",
                MAX_ASSET_BYTES
            );
            return Err(());
        }
        if let Err(e) = file.write_all(chunk) {
            eprintln!("update install: write {}: {e}", dest.display());
            return Err(());
        }
        total += chunk.len();
        Ok(())
    });
    if result.is_err() {
        drop(file); // release our own no-delete handle before removing
        let _ = std::fs::remove_file(dest);
        Err(())
    } else {
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_pin_the_real_release_listing() {
        // Independent truth: the v0.8.0 release's four asset names
        // (gh release view, 2026-10-27) — the selection must reproduce
        // them exactly, or the download 404s and the ritual's signed
        // names stop matching.
        assert_eq!(
            asset_name("v0.8.0", Flavor::Portable, Language::English),
            "riviv-0.8.0-portable.exe"
        );
        assert_eq!(
            asset_name("v0.8.0", Flavor::Installed, Language::ChineseSimplified),
            "riviv-0.8.0.x64.zh-CN-Setup.exe"
        );
        assert_eq!(
            asset_name("v0.8.0", Flavor::Installed, Language::ChineseTraditional),
            "riviv-0.8.0.x64.zh-TW-Setup.exe"
        );
        assert_eq!(
            asset_name("v0.8.0", Flavor::Installed, Language::English),
            "riviv-0.8.0.x64.en-US-Setup.exe"
        );
        // The tag's v-prefix is stripped into the version; a bare
        // spelling yields the same name (both spellings name one tag).
        assert_eq!(
            asset_name("0.8.0", Flavor::Portable, Language::English),
            asset_name("v0.8.0", Flavor::Portable, Language::English)
        );
    }

    #[test]
    fn staging_and_url_names_compose_from_the_tag() {
        // The staging name carries the pid (per-flight uniqueness) but
        // keeps the sweep-matching prefix and .exe tail.
        let name = staging_file_name("v0.9.0");
        assert!(name.starts_with("riviv-update-v0.9.0-"), "{name}");
        assert!(name.ends_with(".exe"), "{name}");
        assert!(
            name.ends_with(&format!("-{}.exe", std::process::id())),
            "{name}"
        );
        assert_eq!(
            dl_path("v0.9.0", "riviv-0.9.0-portable.exe", false),
            "/jaredshuai/riviv/releases/download/v0.9.0/riviv-0.9.0-portable.exe"
        );
        assert_eq!(
            dl_path("v0.9.0", "riviv-0.9.0-portable.exe", true),
            "/jaredshuai/riviv/releases/download/v0.9.0/riviv-0.9.0-portable.exe.sig"
        );
    }

    #[test]
    fn asset_budget_boundary() {
        assert!(asset_fits(0, MAX_ASSET_BYTES));
        assert!(asset_fits(8_523_776, 1));
        assert!(!asset_fits(MAX_ASSET_BYTES, 1));
        assert!(!asset_fits(1, usize::MAX));
    }

    #[test]
    fn flavor_detection_follows_uninstall_adjacency() {
        let tmp = std::env::temp_dir().join(format!(
            "riviv-t237-flavor-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        // Bare dir = portable.
        assert_eq!(detect_flavor(&tmp), Flavor::Portable);
        // Uninstall.exe beside = installed (the NSIS-staged marker).
        std::fs::write(tmp.join("Uninstall.exe"), b"stub").unwrap();
        assert_eq!(detect_flavor(&tmp), Flavor::Installed);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn staging_dir_splits_by_flavor() {
        let exe_dir = std::path::Path::new(r"D:\tools\riviv");
        assert_eq!(staging_dir(Flavor::Portable, exe_dir), exe_dir);
        assert_eq!(
            staging_dir(Flavor::Installed, exe_dir),
            std::env::temp_dir()
        );
    }
}
