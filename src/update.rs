//! In-app update check (#210; ADR 0007; spike #208 with O1-O6 all
//! recommended). Help -> Check for Updates runs one WinHTTP GET against
//! GitHub's `releases/latest` API on a background thread, compares the
//! tag against `CARGO_PKG_VERSION`, and reports back on
//! `UPDATE_REPLY_MESSAGE` (WM_APP+4) for the UI thread to show one of
//! three message boxes. Notify-only by ruling: riviv never downloads,
//! verifies, or replaces anything — "new version" opens the releases
//! page in the browser (the canonical URL always redirects to the
//! latest release, so the response's html_url never needs parsing).
//!
//! Layering per the quality tiers: parsing, semver comparison, and the
//! response-budget decision are pure functions under test below; the
//! WinHTTP calls are the thin unsafe shell (every failure maps to the
//! user-level Failed outcome — network failures are expected, never
//! fatal, per ADR 0007 D3).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::loc;
use crate::text::to_wide;
use windows::Win32::Foundation::{GetLastError, HWND, LPARAM};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_FLAG_SECURE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, MESSAGEBOX_STYLE,
    MessageBoxW, PostMessageW, SW_SHOWNORMAL, WM_APP,
};
use windows::core::PCWSTR;

/// The canonical releases page (O3: the one channel). The "open download
/// page" button opens this URL.
pub(crate) const RELEASES_URL: &str = "https://github.com/jaredshuai/riviv/releases/latest";
const API_HOST: &str = "api.github.com";
const API_PATH: &str = "/repos/jaredshuai/riviv/releases/latest";

/// The worker's reply lands here; wparam is unused, lparam carries a
/// boxed Outcome. WM_APP+1/+2/+3 are the load kick, the Everything
/// retry, and the pane jump — this is the class's fourth private
/// message (ADR 0007 D2).
pub(crate) const UPDATE_REPLY_MESSAGE: u32 = WM_APP + 4;

/// One check at a time: a second menu click while a check is in flight
/// is ignored (two replies would stack two message boxes). Cleared by
/// the worker before it posts, so a fresh click right after a verdict
/// starts a new check.
static CHECK_RUNNING: AtomicBool = AtomicBool::new(false);

/// What the UI thread should do after a check completes.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// The latest release is at or below the running version.
    UpToDate,
    /// A strictly newer release exists; the payload is the tag minus
    /// the leading `v` (the dialog composes it into the localized text).
    Available(String),
    /// The network failed or the response was unparseable. A separate
    /// user-level category from ADR 0001's image-load failure — that
    /// ADR does not rule here (ADR 0007 D3).
    Failed,
}

/// `MAJOR.MINOR.PATCH`, numeric per part. Accepts GitHub's `v`-prefixed
/// tag spelling and Cargo's bare spelling (they name the same version —
/// the release flow derives the tag from the crate version); anything
/// else (empty, fewer parts, non-numeric, prerelease suffix) parses to
/// None so a malformed tag can never fake a "new version".
pub(crate) fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let bare = s.strip_prefix('v').unwrap_or(s);
    let mut it = bare.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Pull the `"tag_name"` value out of the API response. Hand-rolled on
/// purpose (one field, no JSON crate — the zero-dependency principle):
/// tolerant of field order and surrounding whitespace, strict about the
/// value's shape (the caller's parse_semver rejects garbage, and a
/// non-200 error body carries no tag_name at all, which is exactly the
/// Failed path — that is why no status-code query FFI exists, ADR 0007
/// D4). Returns the first match; the API emits exactly one.
pub(crate) fn parse_tag_name(body: &str) -> Option<&str> {
    let key = "\"tag_name\"";
    let start = body.find(key)? + key.len();
    let rest = &body[start..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// The notify decision: strictly newer numeric triple => Available;
/// equal or older (or the tag spelling differs but compares equal) =>
/// UpToDate; anything unparseable on either side => Failed.
pub(crate) fn decide(current: &str, latest_tag: &str) -> Outcome {
    match (parse_semver(current), parse_semver(latest_tag)) {
        (Some(c), Some(l)) if l > c => {
            Outcome::Available(latest_tag.trim_start_matches('v').to_string())
        }
        (Some(_), Some(_)) => Outcome::UpToDate,
        _ => Outcome::Failed,
    }
}

/// The response budget (ADR 0007 D4): the real body is a few KB; a
/// server that streams past this cap is treated as a failed check, not
/// an unbounded allocation. Pure so the boundary is testable.
const MAX_BODY_BYTES: usize = 64 * 1024;
pub(crate) fn body_fits(have: usize, add: usize) -> bool {
    have + add <= MAX_BODY_BYTES
}

/// Help -> Check for Updates (called on the UI thread). Spawns the
/// background check; a click while one is running is a no-op. Thread
/// spawn failure answers the user directly — Builder::spawn maps to a
/// user-level outcome instead of panicking the process (#65 P2
/// discipline; we are still on the UI thread, so the box can show
/// synchronously).
pub(crate) fn begin(hwnd: HWND) {
    if CHECK_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    // HWND is a plain Win32 handle (not a pointer into Rust memory),
    // but windows-rs marks the raw pointer non-Send; carry it across
    // the spawn as an isize instead.
    let hwnd_raw = hwnd.0 as isize;
    let spawned = std::thread::Builder::new()
        .name("update-check".to_string())
        .spawn(move || worker(HWND(hwnd_raw as *mut _)));
    if spawned.is_err() {
        CHECK_RUNNING.store(false, Ordering::SeqCst);
        show_box(
            hwnd,
            loc::get(loc::Id::UpdateFailedText),
            MB_ICONERROR | MB_OK,
        );
    }
}

/// The background half: fetch, decide, post the boxed outcome. The
/// in-flight gate clears before the post so the next click can start
/// once the verdict is queued.
fn worker(hwnd: HWND) {
    let outcome = match fetch_latest_body() {
        Ok(body) => decide(
            env!("CARGO_PKG_VERSION"),
            parse_tag_name(&body).unwrap_or(""),
        ),
        Err(_) => Outcome::Failed,
    };
    CHECK_RUNNING.store(false, Ordering::SeqCst);
    let boxed = Box::into_raw(Box::new(outcome));
    // SAFETY: hwnd was live when begin() ran on the UI thread. If the
    // window has died since, PostMessageW simply fails and the box is
    // reclaimed below — the pointer is handed over exactly once either
    // way.
    let posted = unsafe {
        PostMessageW(
            Some(hwnd),
            UPDATE_REPLY_MESSAGE,
            Default::default(),
            LPARAM(boxed as isize),
        )
    };
    if posted.is_err() {
        // SAFETY: the post failed, so nobody else received the pointer.
        drop(unsafe { Box::from_raw(boxed) });
    }
}

/// Runs on the UI thread from wnd_proc's UPDATE_REPLY_MESSAGE arm:
/// take the boxed outcome back and show the verdict.
pub(crate) fn on_reply(hwnd: HWND, lparam: LPARAM) {
    // SAFETY: the pointer was produced by Box::into_raw on the worker
    // and delivered by PostMessage — this arm is its single consumer.
    let outcome = unsafe { Box::from_raw(lparam.0 as *mut Outcome) };
    match *outcome {
        Outcome::UpToDate => {
            show_box(
                hwnd,
                loc::get(loc::Id::UpdateUpToDateText),
                MB_ICONINFORMATION | MB_OK,
            );
        }
        Outcome::Available(version) => {
            let text = loc::get(loc::Id::UpdateNewVersionText).replace("%s", &version);
            let text_wide = to_wide(&text);
            let caption = to_wide(loc::get(loc::Id::AppName));
            // SAFETY: both buffers outlive the modal call; hwnd is the
            // live owner.
            let choice = unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text_wide.as_ptr()),
                    PCWSTR(caption.as_ptr()),
                    MB_ICONQUESTION | MB_YESNO,
                )
            };
            if choice == IDYES {
                open_releases_page(hwnd);
            }
        }
        Outcome::Failed => {
            show_box(
                hwnd,
                loc::get(loc::Id::UpdateFailedText),
                MB_ICONERROR | MB_OK,
            );
        }
    }
}

/// One localized verdict box (caption = the app name, like show_about).
fn show_box(hwnd: HWND, text: &str, flags: MESSAGEBOX_STYLE) {
    let text_wide = to_wide(text);
    let caption = to_wide(loc::get(loc::Id::AppName));
    // SAFETY: both buffers outlive the modal call; hwnd is the live
    // owner.
    let _ = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text_wide.as_ptr()),
            PCWSTR(caption.as_ptr()),
            flags,
        )
    };
}

/// Notify-only's single action (ADR 0007 D5): open the canonical
/// releases page — it always redirects to the latest release.
fn open_releases_page(hwnd: HWND) {
    let url = to_wide(RELEASES_URL);
    // SAFETY: url is NUL-terminated and outlives the call; hwnd is the
    // live owner; a failed launch (no browser association is a broken
    // shell) leaves the box's verdict standing — fail-soft on purpose.
    let _ = unsafe {
        ShellExecuteW(
            Some(hwnd),
            windows::core::w!("open"),
            PCWSTR(url.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
}

/// The unsafe shell: one GET over WinHTTP with a UA header (GitHub's
/// API rejects UA-less requests) and hard timeouts. Every failure maps
/// to Err — network failures are expected and user-level (ADR 0007
/// D1/D3), with the GetLastError stage breadcrumbed to stderr as the
/// smoke/diagnostic channel.
fn fetch_latest_body() -> Result<String, ()> {
    // RAII so every early return closes what was opened; drop order
    // below is request -> connect -> session (children before the
    // session), and the guard tolerates the null of an aborted open.
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: this guard owns a live WinHTTP handle and
                // drops it exactly once.
                let _ = unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    macro_rules! fail {
        ($stage:expr) => {{
            // SAFETY: reads the calling thread's last error immediately
            // after the failed call.
            let code = unsafe { GetLastError() }.0;
            eprintln!("update check: {} failed (GetLastError={code})", $stage);
            return Err(());
        }};
    }

    let agent = to_wide(&format!("riviv/{}", env!("CARGO_PKG_VERSION")));
    // SAFETY: agent is NUL-terminated and outlives the call; null proxy
    // params let the automatic-proxy access type resolve on its own.
    let session = unsafe {
        WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    };
    if session.is_null() {
        fail!("WinHttpOpen");
    }
    let session = Handle(session);
    // SAFETY: session.0 is the live session handle.
    if unsafe { WinHttpSetTimeouts(session.0, 0, 5_000, 5_000, 10_000) }.is_err() {
        fail!("WinHttpSetTimeouts");
    }
    let host = to_wide(API_HOST);
    // SAFETY: host is NUL-terminated; session.0 is the live session.
    let connect = unsafe {
        WinHttpConnect(
            session.0,
            PCWSTR(host.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        )
    };
    if connect.is_null() {
        fail!("WinHttpConnect");
    }
    let connect = Handle(connect);
    let verb = to_wide("GET");
    let path = to_wide(API_PATH);
    // SAFETY: verb/path are NUL-terminated; null version/referrer and a
    // null accept-types array select the documented defaults (HTTP/1.1,
    // no referer, no Accept header); connect.0 is the live connection.
    let request = unsafe {
        WinHttpOpenRequest(
            connect.0,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    };
    if request.is_null() {
        fail!("WinHttpOpenRequest");
    }
    let request = Handle(request);
    // The AddRequestHeaders wrapper passes the slice length as the
    // header length, so this slice deliberately carries no NUL.
    let ua: Vec<u16> = format!("User-Agent: riviv/{}", env!("CARGO_PKG_VERSION"))
        .encode_utf16()
        .collect();
    // SAFETY: ua holds the exact header text; request.0 is live.
    if unsafe { WinHttpAddRequestHeaders(request.0, &ua, WINHTTP_ADDREQ_FLAG_ADD) }.is_err() {
        fail!("WinHttpAddRequestHeaders");
    }
    // SAFETY: no extra headers here (UA added above), no body to send;
    // request.0 is live.
    if unsafe { WinHttpSendRequest(request.0, None, None, 0, 0, 0) }.is_err() {
        fail!("WinHttpSendRequest");
    }
    // SAFETY: null reserved pointer per the contract; request.0 is live.
    if unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }.is_err() {
        fail!("WinHttpReceiveResponse");
    }
    let mut body: Vec<u8> = Vec::new();
    loop {
        let mut avail: u32 = 0;
        // SAFETY: avail is the out-param; request.0 is live.
        if unsafe { WinHttpQueryDataAvailable(request.0, &mut avail) }.is_err() {
            fail!("WinHttpQueryDataAvailable");
        }
        if avail == 0 {
            break;
        }
        if !body_fits(body.len(), avail as usize) {
            eprintln!(
                "update check: response exceeds the {} B budget",
                MAX_BODY_BYTES
            );
            return Err(());
        }
        let mut chunk = vec![0u8; avail as usize];
        let mut read: u32 = 0;
        // SAFETY: chunk holds avail bytes; read is the out-param;
        // request.0 is live.
        if unsafe { WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), avail, &mut read) }
            .is_err()
        {
            fail!("WinHttpReadData");
        }
        if read == 0 {
            break;
        }
        chunk.truncate(read as usize);
        body.extend_from_slice(&chunk);
    }
    // GitHub's API is UTF-8; a hostile body that is not decodes lossily
    // into something tag-less, which decide() maps to Failed anyway.
    Ok(String::from_utf8_lossy(&body).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parses_both_spellings_and_rejects_garbage() {
        assert_eq!(parse_semver("v0.5.0"), Some((0, 5, 0)));
        assert_eq!(parse_semver("0.5.0"), Some((0, 5, 0)));
        assert_eq!(parse_semver("1.22.333"), Some((1, 22, 333)));
        assert_eq!(parse_semver(""), None);
        assert_eq!(parse_semver("0.5"), None);
        assert_eq!(parse_semver("0.5.0.1"), None);
        assert_eq!(parse_semver("0.5.0-beta"), None);
        assert_eq!(parse_semver("vX.Y.Z"), None);
    }

    #[test]
    fn semver_compares_numerically_not_lexicographically() {
        // The dictionary would say "0.10.0" < "0.9.0".
        assert!(parse_semver("0.10.0").unwrap() > parse_semver("0.9.0").unwrap());
        assert!(parse_semver("1.0.0").unwrap() > parse_semver("0.99.99").unwrap());
        assert_eq!(
            parse_semver("v0.5.0").unwrap(),
            parse_semver("0.5.0").unwrap()
        );
    }

    #[test]
    fn tag_name_is_extracted_from_response_shapes() {
        // Compact (what the API emits) and pretty-printed shapes, with
        // tag_name anywhere among its siblings.
        assert_eq!(
            parse_tag_name(r#"{"url":"x","tag_name":"v0.5.0","assets":[]}"#),
            Some("v0.5.0")
        );
        assert_eq!(
            parse_tag_name("{\n  \"name\": \"riviv\",\n  \"tag_name\": \"v1.2.3\"\n}"),
            Some("v1.2.3")
        );
        // No tag (an error body, or the field absent) is None — never a
        // guess.
        assert_eq!(parse_tag_name(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_tag_name(""), None);
        // A truncated value (no closing quote) is None.
        assert_eq!(parse_tag_name(r#"{"tag_name":"v0.5.0"#), None);
    }

    #[test]
    fn decide_maps_the_three_verdicts() {
        assert!(matches!(decide("0.5.0", "v0.5.0"), Outcome::UpToDate));
        assert!(matches!(decide("0.5.0", "v0.4.0"), Outcome::UpToDate));
        match decide("0.5.0", "v0.6.0") {
            Outcome::Available(v) => assert_eq!(v, "0.6.0"),
            other => panic!("expected Available, got {other:?}"),
        }
        assert!(matches!(decide("0.5.0", "garbage"), Outcome::Failed));
        assert!(matches!(decide("bad", "v0.6.0"), Outcome::Failed));
    }

    #[test]
    fn body_budget_boundary() {
        assert!(body_fits(0, MAX_BODY_BYTES));
        assert!(body_fits(1, MAX_BODY_BYTES - 1));
        assert!(!body_fits(1, MAX_BODY_BYTES));
        assert!(!body_fits(MAX_BODY_BYTES, 1));
    }
}
