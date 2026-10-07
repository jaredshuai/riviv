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
//! #214 added the startup auto-check (ADR 0007 D6; spike
//! s-startup-update-check, P1-P4 all recommended): the same worker and
//! reply protocol carry it, with a daily gate (`should_check`) decided
//! before any spawn and a silent presentation — only the Available
//! verdict shows, as the 3-second status-bar flash; UpToDate/Failed
//! stay quiet (the user never asked anything). A manual click during a
//! silent startup flight escalates the pending verdict back to the
//! manual boxes (P4=A).
//!
//! Layering per the quality tiers: parsing, semver comparison, the
//! response-budget decision, and the daily-gate math are pure functions
//! under test below; the WinHTTP calls are the thin unsafe shell (every
//! failure maps to the user-level Failed outcome — network failures are
//! expected, never fatal, per ADR 0007 D3).

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::loc;
use crate::text::to_wide;
use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_FLAG_SECURE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
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

/// The worker's wake-up call: the message itself carries NO payload —
/// the verdict waits in `REPLY_QUEUE` (a posted pointer would let any
/// peer process forge this message and crash the handler on an invalid
/// lparam; cubic #211 P2). WM_APP+1/+2/+3 are the load kick, the
/// Everything retry, and the pane jump — this is the class's fourth
/// private message (ADR 0007 D2).
pub(crate) const UPDATE_REPLY_MESSAGE: u32 = WM_APP + 4;

/// One check at a time: a second menu click while a check is in flight
/// is ignored (two replies would stack two message boxes). Cleared by
/// the worker before it posts, so a fresh click right after a verdict
/// starts a new check.
static CHECK_RUNNING: AtomicBool = AtomicBool::new(false);

/// P4=A escalation (#214): a MANUAL click sets this even when the
/// single-flight gate swallows it — the in-flight (silent startup)
/// verdict then presents in the manual three-box form when it lands,
/// because a user who asked must get an answer (ADR 0007 D3's manual
/// half). Read-and-cleared by `on_reply`.
static MANUAL_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The verdict hand-off (ADR 0007 D2): the worker parks the Outcome
/// here and posts the bare notification; the UI thread's handler pops.
/// Single-flight means at most one producer and one entry — a forged or
/// spurious message can only ever find the queue empty and be ignored.
static REPLY_QUEUE: Mutex<VecDeque<Outcome>> = Mutex::new(VecDeque::new());

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
/// body being a JSON object (first non-space char `{`) and about the
/// value's shape (the caller's parse_semver rejects garbage, and a
/// non-200 error body carries no tag_name at all, which is exactly the
/// Failed path — that is why no status-code query FFI exists, ADR 0007
/// D4). Returns the first match; the API emits exactly one.
pub(crate) fn parse_tag_name(body: &str) -> Option<&str> {
    // The structure gate: a plain-text blob that merely quotes the key
    // cannot smuggle a tag past the strict value parse (cubic #211 P2).
    let body = body.trim_start();
    if !body.starts_with('{') {
        return None;
    }
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
    // The running version is our own build metadata — parsed leniently
    // (a `-prerelease` suffix drops before the numeric compare) so a
    // beta build still compares instead of always reporting failure;
    // the server-side tag stays strict (cubic #211 P3).
    let current = current.split_once('-').map_or(current, |(v, _)| v);
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
    // checked_add: on a 32-bit usize a hostile avail near u32::MAX
    // would wrap the plain sum below the cap (adversarial review #211
    // P3 — unreachable on the x64-only ship, but this is the guard).
    have.checked_add(add).is_some_and(|t| t <= MAX_BODY_BYTES)
}

/// The startup gate's verdict (#214, P1=A/P3=A; ADR 0007 D6).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Do nothing and touch nothing: the switch is off, or today is
    /// already consumed.
    Skip,
    /// First run (or a hand-mangled/never-written gate): mark today so
    /// this never reads as first-run again, but send no request — a
    /// fresh install is necessarily current (the spike's first-run
    /// skip, Sumatra's precedent).
    MarkOnly,
    /// A previous day owns the mark: mark today AND run the check.
    Check,
}

/// Pure half of the daily gate: `last_day <= 0` is the "never" sentinel
/// (0 default, garbage-parse 0, or a negative hand-edit — all degrade to
/// the skip-once path, never a request storm); a positive `last_day`
/// strictly before today checks; anything equal-or-newer skips. The
/// caller writes `today` into the config for BOTH marking arms —
/// skip-branch-without-mark would wedge every launch into first-run
/// status forever (cubic #216 P1).
pub(crate) fn should_check(enabled: bool, last_day: i32, today: i32) -> Gate {
    if !enabled {
        Gate::Skip
    } else if last_day <= 0 {
        Gate::MarkOnly
    } else if last_day < today {
        Gate::Check
    } else {
        Gate::Skip
    }
}

/// Days since 1970-01-01 for a civil (Gregorian) date — Howard Hinnant's
/// `days_from_civil`, the era-based form that stays exact over i32 for
/// any plausible year. Pure so the calendar math is testable.
pub(crate) fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    // Floor division for negative years: shift into the era first.
    let y_shifted = if y >= 0 { y } else { y - 399 };
    let era = y_shifted / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp: i32 = if m > 2 { m as i32 - 3 } else { m as i32 + 9 }; // Mar=0..Feb=11
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719_468
}

/// The LOCAL day ordinal for the daily gate (P3=A): local midnight
/// boundaries, so a traveler crossing time zones may consume two gate
/// days in one UTC day — harmless, the gate is a politeness cap, not an
/// audit. The unsafe shell is this one read; the calendar math above is
/// the tested pure half.
pub(crate) fn local_day_ordinal() -> i32 {
    // SAFETY: no inputs; SYSTEMTIME is a plain value type (windows-0.62
    // returns it rather than filling an out-param).
    let st = unsafe { GetLocalTime() };
    days_from_civil(st.wYear as i32, st.wMonth as u32, st.wDay as u32)
}

/// The shared single-flight spawn (ADR 0007 D2). Returns false ONLY for
/// a thread-creation failure (the gate is re-cleared so a retry can
/// run); already-in-flight returns true — the caller's escalation flag
/// is what keeps that case honest for manual clicks.
fn dispatch(hwnd: HWND) -> bool {
    if CHECK_RUNNING.swap(true, Ordering::SeqCst) {
        return true;
    }
    // HWND is a plain Win32 handle (not a pointer into Rust memory),
    // but windows-rs marks the raw pointer non-Send; carry it across
    // the spawn as an isize instead.
    let hwnd_raw = hwnd.0 as isize;
    match std::thread::Builder::new()
        .name("update-check".to_string())
        .spawn(move || worker(HWND(hwnd_raw as *mut _)))
    {
        Ok(_) => true,
        Err(_) => {
            CHECK_RUNNING.store(false, Ordering::SeqCst);
            false
        }
    }
}

/// Help -> Check for Updates (called on the UI thread, #210). Thread
/// spawn failure answers the user directly — Builder::spawn maps to a
/// user-level outcome instead of panicking the process (#65 P2
/// discipline; we are still on the UI thread, so the box can show
/// synchronously).
pub(crate) fn begin(hwnd: HWND) {
    // Set BEFORE the gate: a click swallowed by an in-flight silent
    // startup check still owes the user the manual presentation when
    // that verdict lands (P4=A).
    MANUAL_REQUESTED.store(true, Ordering::SeqCst);
    if !dispatch(hwnd) {
        show_box(
            hwnd,
            loc::get(loc::Id::UpdateFailedText),
            MB_ICONERROR | MB_OK,
        );
    }
}

/// The startup half (#214, P1=A): called once from run() before the
/// pump, after the daily gate marked today. Silent on every failure —
/// a spawn failure is an stderr breadcrumb, not a box (the user never
/// asked anything; cubic #216 P2).
pub(crate) fn begin_startup(hwnd: HWND) {
    if !dispatch(hwnd) {
        eprintln!("startup update check: worker spawn failed — skipped silently");
    }
}

/// The background half: fetch, decide, park the verdict in the queue,
/// post the bare notification. The in-flight gate clears before the
/// post so the next click can start once the verdict is queued.
fn worker(hwnd: HWND) {
    let outcome = match fetch_latest_body() {
        Ok(body) => decide(
            env!("CARGO_PKG_VERSION"),
            parse_tag_name(&body).unwrap_or(""),
        ),
        Err(_) => Outcome::Failed,
    };
    CHECK_RUNNING.store(false, Ordering::SeqCst);
    if let Ok(mut queue) = REPLY_QUEUE.lock() {
        // Single-flight guarantees no concurrent producer; the clear
        // drops any verdict a dying window never claimed.
        queue.clear();
        queue.push_back(outcome);
    }
    // SAFETY: hwnd was live when begin() ran on the UI thread. If the
    // window has died since, the post simply fails and the queued
    // verdict stays unclaimed — nothing crosses the message boundary,
    // so there is no pointer to hand over or reclaim.
    let _ = unsafe {
        PostMessageW(
            Some(hwnd),
            UPDATE_REPLY_MESSAGE,
            Default::default(),
            Default::default(),
        )
    };
}

/// Runs on the UI thread from wnd_proc's UPDATE_REPLY_MESSAGE arm:
/// claim the queued verdict and show it. A message with nothing queued
/// (forged or spurious) is ignored. The presentation is two-shaped
/// (#214): MANUAL_REQUESTED (a menu click — possibly one the
/// single-flight gate swallowed during a silent startup flight, P4=A)
/// gets the three manual boxes; otherwise this was a startup check and
/// only the Available verdict surfaces, as the status-bar flash (P2=A)
/// — UpToDate and Failed stay silent (ADR 0007 D3: the user never
/// asked; the fetch's stderr breadcrumbs are the diagnostic channel).
pub(crate) fn on_reply(hwnd: HWND) {
    let Some(outcome) = REPLY_QUEUE.lock().ok().and_then(|mut q| q.pop_front()) else {
        return;
    };
    let manual = MANUAL_REQUESTED.swap(false, Ordering::SeqCst);
    match outcome {
        Outcome::UpToDate if manual => {
            show_box(
                hwnd,
                loc::get(loc::Id::UpdateUpToDateText),
                MB_ICONINFORMATION | MB_OK,
            );
        }
        Outcome::Available(version) if manual => {
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
        Outcome::Failed if manual => {
            show_box(
                hwnd,
                loc::get(loc::Id::UpdateFailedText),
                MB_ICONERROR | MB_OK,
            );
        }
        // The startup presentation: only Available is visible, as the
        // 3-second status-bar flash (the existing #47 infrastructure);
        // the daily gate re-notifies on the next day's launch, so one
        // missed flash (replaced by a panscan flash, status bar off,
        // fullscreen slideshow) has no lasting cost.
        Outcome::Available(version) => {
            let text = loc::get(loc::Id::UpdateAvailableTemp).replace("%s", &version);
            crate::window::status_set_temp_text(hwnd, Some(text));
        }
        Outcome::UpToDate | Outcome::Failed => {}
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
    // SAFETY: session.0 is the live session handle. All four timeouts
    // are nonzero — resolve included: 0 would mean infinite per the
    // MSDN contract, and a hung resolver would wedge the single-flight
    // gate for the rest of the session with no Failed box ever shown
    // (cubic #211 P2).
    if unsafe { WinHttpSetTimeouts(session.0, 5_000, 5_000, 5_000, 10_000) }.is_err() {
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
        // Non-JSON bodies that merely quote the key are None — the
        // structure gate (cubic #211 P2).
        assert_eq!(
            parse_tag_name("garbage \"tag_name\":\"v9.9.9\" trailing"),
            None
        );
        // Leading whitespace before the `{` stays tolerated.
        assert_eq!(
            parse_tag_name("  {\"tag_name\":\"v0.5.0\"}"),
            Some("v0.5.0")
        );
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
        // A prerelease-suffixed running version compares leniently (the
        // server-side tag stays strict; cubic #211 P3).
        assert!(matches!(
            decide("1.0.0-beta.1", "v1.0.0"),
            Outcome::UpToDate
        ));
        match decide("1.0.0-beta.1", "v1.1.0") {
            Outcome::Available(v) => assert_eq!(v, "1.1.0"),
            other => panic!("expected Available, got {other:?}"),
        }
        assert!(matches!(decide("0.5.0", "v1.0.0-rc1"), Outcome::Failed));
    }

    #[test]
    fn body_budget_boundary() {
        assert!(body_fits(0, MAX_BODY_BYTES));
        assert!(body_fits(1, MAX_BODY_BYTES - 1));
        assert!(!body_fits(1, MAX_BODY_BYTES));
        assert!(!body_fits(MAX_BODY_BYTES, 1));
        // A hostile chunk size that would wrap the sum stays outside.
        assert!(!body_fits(1, usize::MAX));
    }

    #[test]
    fn civil_days_anchor_the_epoch_and_march_boundaries() {
        // #214: the daily gate's unit. Anchors and leap-day boundaries,
        // all hand-derived from month arithmetic (1970 and 1971 are
        // common years; 1972, 2000 divisible-by-400, and 2024 leap;
        // 2100 divisible-by-100-not-400 does NOT).
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(1971, 1, 1), 365);
        // 1972-02-28 = 788, Feb 29 exists, so Mar 1 = 790.
        assert_eq!(days_from_civil(1972, 2, 28), 788);
        assert_eq!(days_from_civil(1972, 3, 1), 790);
        assert_eq!(days_from_civil(1972, 2, 29), 789);
        // Year boundaries run through the March-era shift: the day
        // BEFORE Jan 1 belongs to the previous era-year.
        assert_eq!(
            days_from_civil(1972, 1, 1) - days_from_civil(1971, 12, 31),
            1
        );
        // 2000-02-29 exists (divisible by 400); 2100-02-29 must not be
        // reachable — instead 2100-03-01 is one day after 2100-02-28.
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
        assert_eq!(
            days_from_civil(2100, 3, 1) - days_from_civil(2100, 2, 28),
            1
        );
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28),
            2
        );
        // A 2026-class date is comfortably int-width.
        assert!(days_from_civil(2026, 10, 7) > 20_000);
        assert!(days_from_civil(2026, 10, 7) < 21_000);
        // Consecutive ordinals across a mid-year month boundary.
        assert_eq!(
            days_from_civil(2026, 6, 1) - days_from_civil(2026, 5, 31),
            1
        );
    }

    #[test]
    fn the_daily_gate_maps_every_input_class() {
        // #214 (P1=A/P3=A): off never touches the mark; never (0, or a
        // garbage/negative hand-edit) marks without asking; a strictly
        // older local day checks; today-or-future skips untouched.
        assert_eq!(should_check(false, 0, 100), Gate::Skip);
        assert_eq!(should_check(false, 99, 100), Gate::Skip);
        assert_eq!(should_check(true, 0, 100), Gate::MarkOnly);
        assert_eq!(should_check(true, -5, 100), Gate::MarkOnly);
        assert_eq!(should_check(true, 99, 100), Gate::Check);
        assert_eq!(should_check(true, 1, 20_700), Gate::Check);
        assert_eq!(should_check(true, 100, 100), Gate::Skip);
        assert_eq!(should_check(true, 101, 100), Gate::Skip);
    }
}
