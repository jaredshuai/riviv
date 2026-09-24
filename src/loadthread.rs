//! Background decode: one worker thread, a job channel, per-load reply
//! queues (issue #4's port of upstream's load-thread plumbing).
//!
//! Upstream model (viv.c): `_viv_open` hands the file to a single load
//! thread; if one is already running, the request is chained
//! (`_viv_load_image_next_fd`) and picked up only when the previous load
//! exits at its next per-frame terminate check (viv.c:1520-1572, 10610) —
//! so at most one decode is ever active. The thread decodes frame by frame
//! and posts replies through `_viv_reply_add` — a linked list under a
//! critical section (viv.c:10869-10905) — and the UI drains the whole
//! queue per kick message (`_VIV_WM_REPLY`, viv.c:2762-3060). At quit the
//! UI *waits* for the thread ("it's critical we wait for load image to
//! finish", viv.c:5470-5479).
//!
//! Riviv translation: one persistent worker reading jobs from a channel
//! (the chain, minus the churn). Each open creates a session with its own
//! reply queue; superseding a session flags its job, and the worker skips
//! it at the next check (instantly if the job was still queued, between
//! frames if decoding — a still image's single decode cannot be
//! interrupted, the same granularity upstream has). The queue — not the
//! posted message — owns the replies, so a kick lost to window teardown
//! cannot leak. Frames cross the boundary as pure memory (`PixelFrame`,
//! #76 — no GDI object leaves the worker); the UI thread wraps them in
//! Surfaces (the master's holder, `surface.rs` — the #76-era GDI-face
//! derivation died with the render arm in #90).

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::loader::{
    DecodeEnv, LoadReply, decode_bytes_to_sink, decode_dib_to_sink, decode_to_sink,
};
use crate::pixels::PixelFrame;

/// Kick message, posted whenever the worker queues a reply (upstream
/// `_VIV_WM_REPLY`; `WM_APP + n` is the reserved range for private
/// window-class messages). Unlike upstream — which posts only on the
/// queue's empty -> non-empty transition — every push posts, and a failed
/// post is retried until it lands or the job is terminated: a lost kick
/// (e.g. the destination queue momentarily at its quota) must not strand
/// a non-empty queue, least of all the stream's final reply, which has no
/// successor push to heal it. The extra messages are bounded by the reply
/// count and drain to no-ops.
pub(crate) const REPLY_KICK_MESSAGE: u32 = WM_APP + 1;

/// Session ids, so the UI can tell which load produced the displayed
/// image (`displayed_from` in `WindowState`).
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// HWND is a plain handle value but holds a raw pointer, so std does not
/// consider it `Send`.
#[derive(Clone, Copy)]
struct SendHwnd(HWND);
// SAFETY: moving the handle into the job is sound — the worker only posts
// messages to it, and a post to a dead window just fails. Same
// cross-thread handoff as upstream's `_viv_reply_add` posting to
// `_viv_hwnd` (viv.c:10902).
unsafe impl Send for SendHwnd {}

/// What a job decodes (#65/#66): a file on disk, the process's stdin
/// captured as one in-memory stream (the `stdin:` pseudo-filename), or
/// the clipboard's image read as one DIB payload (the `clipboard:`
/// pseudo-filename — the clipboard is global, so unlike stdin this
/// source reads the same bytes whichever window requested it). The
/// display name for a virtual source is the literal pseudo-name — the
/// session carries it so titles/verdicts read like a file load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoadSource {
    File(OsString),
    Stdin,
    Clipboard,
}

/// The `stdin:` display name, shared by the session and the request
/// shell so the title/status read one constant (#65; the pseudo-name a
/// piped launch types on the command line). The clipboard family's
/// counterpart is `clipboard::CLIPBOARD_NAME`.
pub(crate) const STDIN_NAME: &str = "stdin:";

/// One queued decode request; crossing the channel requires `Send`
/// (PixelFrame and the Arcs are, HWND via the wrapper above). `env`
/// carries the request-time decode inputs (`DecodeEnv` — the composite
/// background and the icm snapshot; upstream stashes a render viewport
/// too for its mip pre-generation, viv.c:1557-1558, which died with the
/// #81 mip retirement + the decode-time composite).
struct Job {
    source: LoadSource,
    env: DecodeEnv,
    hwnd: SendHwnd,
    terminate: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<LoadReply<PixelFrame>>>>,
    /// The animation first-frame paint handshake (#76). Master built each
    /// frame's GDI face on the worker BETWEEN replies — frame 1's decode
    /// could never overtake frame 0's display, so a mid-stream failure
    /// (the decode budget's clear) always found frame 0 already on
    /// screen. With pure-memory frames the worker has no such work, and
    /// posted reply kicks preempt WM_PAINT — without a wait the failure
    /// reply can clear the partial display before its first frame ever
    /// paints. The decode of the frames AFTER an animation's first frame
    /// therefore waits until the UI signals "painted". Foreground opens
    /// only: a preload's parked first frame paints at adoption (much
    /// later), and stalling the single worker on it would defeat
    /// preloading. Stills need nothing — their terminal reply changes
    /// nothing about the display.
    first_frame_painted: Option<FirstFramePainted>,
}

/// The handshake signal: set by the UI thread's first paint after an
/// animation first-frame adoption, awaited by the worker's decode loop.
/// Plain data (Mutex/Condvar/Arc) — `Send` by construction.
pub(crate) type FirstFramePainted = Arc<(Mutex<bool>, Condvar)>;

/// Block until the first frame paints, the job is terminated, or the
/// wait cap expires (a minimized window never paints; the cap keeps the
/// decode moving — master had no stall to begin with there).
pub(crate) fn wait_first_frame_painted(signal: &FirstFramePainted, terminate: &AtomicBool) {
    wait_first_frame_painted_capped(signal, terminate, FIRST_FRAME_PAINT_CAP);
}

/// The cap-parameterized core (the tests drive the cap expiry arm with a
/// short deadline instead of sleeping the production 5 s — review
/// PR #84 F8).
fn wait_first_frame_painted_capped(
    signal: &FirstFramePainted,
    terminate: &AtomicBool,
    cap: std::time::Duration,
) {
    let (lock, cvar) = &**signal;
    let mut painted = lock.lock().unwrap();
    let deadline = std::time::Instant::now() + cap;
    while !*painted {
        if terminate.load(Ordering::Relaxed) {
            return;
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return;
        }
        let (guard, _timeout) = cvar
            .wait_timeout(
                painted,
                std::cmp::min(FIRST_FRAME_WAIT_TICK, deadline - now),
            )
            .unwrap();
        painted = guard;
    }
}

/// How long the worker waits for the first-frame paint before moving on
/// (see [`wait_first_frame_painted`]).
const FIRST_FRAME_PAINT_CAP: std::time::Duration = std::time::Duration::from_secs(5);
/// The polling granularity while waiting (terminate responsiveness).
const FIRST_FRAME_WAIT_TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// The process-wide decode worker handle. Jobs are processed strictly in
/// order (upstream chains the same way, viv.c:1520-1572), so at most one
/// decode is active no matter how fast the user switches images.
pub(crate) struct LoadThread {
    sender: Option<Sender<Job>>,
    handle: Option<JoinHandle<()>>,
}

impl LoadThread {
    /// Start the worker (once per process, before any load is requested).
    /// A spawn failure is system-level (ADR 0001): the caller fails loud.
    pub(crate) fn start() -> Result<Self, String> {
        let (sender, receiver) = channel();
        let handle = std::thread::Builder::new()
            .name("riviv-load".into())
            .spawn(move || worker(receiver))
            .map_err(|e| format!("load thread spawn failed: {e}"))?;
        Ok(LoadThread {
            sender: Some(sender),
            handle: Some(handle),
        })
    }

    /// Queue `source` for decoding and return the session that owns its
    /// replies. The old session (if any) must be dropped by the caller —
    /// its Drop flags the job, and the worker skips it at the next check.
    /// `env` snapshots the request-time decode inputs (see `DecodeEnv`).
    /// `wait_first_paint` arms the animation first-frame handshake (see
    /// `Job::first_frame_painted`) — true for foreground opens, false
    /// for preloads.
    pub(crate) fn request(
        &self,
        hwnd: HWND,
        source: LoadSource,
        env: DecodeEnv,
        wait_first_paint: bool,
    ) -> LoadSession {
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let terminate = Arc::new(AtomicBool::new(false));
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let first_frame_painted = wait_first_paint
            .then(|| Arc::new((Mutex::new(false), Condvar::new())) as FirstFramePainted);
        // The display name: the file's path, or the literal pseudo-name
        // for a virtual source — the UI adopts it as the window title.
        let path = match &source {
            LoadSource::File(path) => path.clone(),
            LoadSource::Stdin => OsString::from(STDIN_NAME),
            LoadSource::Clipboard => OsString::from(crate::clipboard::CLIPBOARD_NAME),
        };
        // The worker consumes its own copy; the session keeps the
        // original for the UI (window title / Ctrl+O initial dir).
        let job = Job {
            source,
            env,
            hwnd: SendHwnd(hwnd),
            terminate: Arc::clone(&terminate),
            queue: Arc::clone(&queue),
            first_frame_painted: first_frame_painted.clone(),
        };
        // Unbounded channel: never blocks the UI thread. A send can only
        // fail if the worker already exited — impossible short of the
        // spawn failure ADR 0001 already failed loud on.
        let _ = self.sender.as_ref().expect("sender dropped").send(job);
        LoadSession {
            id,
            path,
            terminate,
            queue,
            paint_signal: first_frame_painted,
        }
    }

    /// Stop the worker and wait for it. Closing the channel ends the
    /// recv() loop; the active job is abandoned at its terminate check
    /// (the session's Drop already set the flag — bounded by the frame
    /// currently decoding), queued jobs are skipped, then the thread
    /// exits. Upstream waits INFINITE for the same reason ("it's critical
    /// we wait for load image to finish before we kill the main window",
    /// viv.c:5476).
    pub(crate) fn quit(&mut self) {
        // Drop the sender first — a live sender would keep recv() (and
        // therefore join) waiting forever.
        self.sender = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker(receiver: Receiver<Job>) {
    while let Ok(job) = receiver.recv() {
        let Job {
            source,
            env,
            hwnd: SendHwnd(hwnd),
            terminate,
            queue,
            first_frame_painted,
        } = job;
        // Superseded while still queued: nothing was decoded, nothing to
        // reply — skip before touching the source.
        if terminate.load(Ordering::Relaxed) {
            continue;
        }
        // The sink gets its own handle for its retry loop; the original
        // stays with the decode call's terminate parameter.
        let sink_terminate = Arc::clone(&terminate);
        let mut sink = move |reply: LoadReply<PixelFrame>| {
            // unwrap: a poisoned lock means some thread panicked while
            // holding the queue — an undefined state we fail loud on
            // (ADR 0001) rather than limp past.
            queue.lock().unwrap().push_back(reply);
            // The kick must survive a full destination queue: the LAST
            // reply of a stream has no successor push to heal a lost post,
            // so retry until it lands or the job is terminated. Teardown
            // sets the terminate flag before the window finishes dying
            // (WM_NCDESTROY terminates the session, then quits the worker),
            // which bounds the loop; a post to the dead window fails
            // harmlessly until then.
            loop {
                // SAFETY: `hwnd` is a valid handle captured for this job;
                // PostMessageW never dereferences it on this thread and
                // validates it on the owning thread's queue (the queue —
                // not the kick — owns the replies, so a lost post costs
                // drain latency, never data).
                if unsafe { PostMessageW(Some(hwnd), REPLY_KICK_MESSAGE, WPARAM(0), LPARAM(0)) }
                    .is_ok()
                {
                    break;
                }
                if sink_terminate.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        match source {
            LoadSource::File(path) => {
                decode_to_sink(
                    &path,
                    env,
                    &terminate,
                    first_frame_painted.as_ref(),
                    &mut sink,
                );
            }
            LoadSource::Stdin => match read_stdin_terminated(&terminate) {
                Some(StdinOutcome::Bytes(bytes)) => {
                    decode_bytes_to_sink(
                        &bytes,
                        env,
                        &terminate,
                        first_frame_painted.as_ref(),
                        &mut sink,
                    );
                }
                Some(StdinOutcome::Failed(msg)) => sink(LoadReply::FailedUser(msg)),
                None => {} // terminated mid-read: exit silently
            },
            // The clipboard read is a short open-copy-close session on a
            // DETACHED helper thread, terminate-aware (#66): a delayed-
            // rendering clipboard owner can stall GetClipboardData
            // forever, and the stall must not hang this worker (the
            // window teardown joins it) — the stdin reader's contract
            // (see read_stdin_terminated). Every read problem is
            // user-level (keep old image, no dialog, no exit — ADR
            // 0001), exactly like a foreign pipe's bytes.
            LoadSource::Clipboard => {
                match crate::clipboard::read_clipboard_dib_terminated(&terminate) {
                    Some(Ok(Some(payload))) => decode_dib_to_sink(&payload, env, &mut sink),
                    Some(Ok(None)) => sink(LoadReply::FailedUser(format!(
                        "{} no image on the clipboard",
                        crate::clipboard::CLIPBOARD_NAME
                    ))),
                    Some(Err(msg)) => sink(LoadReply::FailedUser(format!(
                        "{} {msg}",
                        crate::clipboard::CLIPBOARD_NAME
                    ))),
                    None => {} // terminated mid-read: exit silently
                }
            }
        }
    }
}

/// The raw byte cap for the `stdin:` stream (#65): without one a hostile
/// pipe could push the read into an OOM that kills the viewer — the same
/// posture as the decoder's own allocation caps (user-level failure, not
/// a crash; ADR 0001).
const MAX_STDIN_BYTES: usize = crate::loader::MAX_TOTAL_FRAME_BYTES;

/// The stdin read chunk (Codex P1): a fixed buffer keeps the stream's Vec
/// from ever growing past the cap — a plain `read_to_end` doubles capacity
/// amortized, so a >512 MiB pipe would transiently allocate ~1 GiB before
/// the length check could reject it. The loop bails at the cap edge,
/// before the offending chunk is appended.
const STDIN_CHUNK: usize = 64 * 1024;

/// What the stdin read produced (#65): the stream's bytes, or a
/// user-level failure message (already prefixed `stdin:` like the file
/// path's shown-name failures).
enum StdinOutcome {
    Bytes(Vec<u8>),
    Failed(String),
}

/// Read the process's stdin to EOF on a DETACHED helper thread and wait
/// terminate-aware (#65). The blocking read cannot be interrupted — a
/// console stdin that never sees EOF would otherwise hang the worker's
/// join at window teardown ("it's critical we wait for load image to
/// finish", viv.c:5476) — so the read runs on its own inert thread (no
/// GDI, no window: nothing the process teardown needs to reclaim) and
/// the worker polls the channel, abandoning the reader when this job is
/// superseded or the window dies. The abandoned reader holds only its
/// buffer until the pipe closes (or forever, until process exit reclaims
/// it — the reader never touches shared state either way). `None` = the
/// job was terminated while reading: exit silently, exactly like a file
/// decode between frames.
fn read_stdin_terminated(terminate: &AtomicBool) -> Option<StdinOutcome> {
    let (sender, receiver) = channel::<StdinOutcome>();
    // Builder::spawn, not thread::spawn (Codex P2): an OS thread-creation
    // failure must surface as THIS load's user-level failure, not a panic
    // that kills the decode worker (every later load would hang Loading).
    let reader = std::thread::Builder::new()
        .name("riviv-stdin".into())
        .spawn(move || {
            let outcome = read_stdin_once();
            // The sender drops silently if nobody waits (terminated job).
            let _ = sender.send(outcome);
        });
    if let Err(e) = reader {
        // The failed spawn leaves nothing behind — this load fails
        // user-level and the worker lives on.
        return Some(StdinOutcome::Failed(format!(
            "stdin: reader thread spawn failed: {e}"
        )));
    }
    // On success the JoinHandle is deliberately dropped: the thread is
    // DETACHED — joining it would reintroduce the blocked-read hang the
    // whole helper exists to avoid.
    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(outcome) => return Some(outcome),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if terminate.load(Ordering::Relaxed) {
                    return None; // abandon the detached reader
                }
            }
            // Unreachable (the reader always sends or dies with the
            // process); treat as a failed read so the wait can never spin.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Some(StdinOutcome::Failed(
                    "stdin: the reader vanished".to_string(),
                ));
            }
        }
    }
}

/// One blocking stdin-to-EOF read, cap-enforced without over-allocation:
/// fixed-size chunks, the cap verdict landing BEFORE the chunk that
/// crosses it is appended (see STDIN_CHUNK). An unreadable stdin (no
/// console/pipe — e.g. an Explorer launch) reads as the Err.
fn read_stdin_once() -> StdinOutcome {
    use std::io::Read as _;
    let mut stdin = std::io::stdin();
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; STDIN_CHUNK];
    loop {
        match stdin.read(&mut chunk) {
            Ok(0) => return StdinOutcome::Bytes(bytes), // EOF
            Ok(n) => {
                if bytes.len() + n > MAX_STDIN_BYTES {
                    return StdinOutcome::Failed(format!(
                        "stdin: exceeds the {MAX_STDIN_BYTES} byte read cap"
                    ));
                }
                bytes.extend_from_slice(&chunk[..n]);
            }
            Err(e) => return StdinOutcome::Failed(format!("stdin: {e}")),
        }
    }
}

/// The in-flight (or queued) load the UI is interested in. Dropping it
/// flags the worker to abandon the job at its next terminate check.
pub(crate) struct LoadSession {
    id: u64,
    /// The file being loaded — the UI adopts it as the window title/path
    /// when this session's first frame takes the display. A virtual
    /// source (`stdin:` #65, `clipboard:` #66) carries the literal
    /// pseudo-name.
    path: OsString,
    terminate: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<LoadReply<PixelFrame>>>>,
    /// The first-frame paint handshake the UI arms at an animation's
    /// first-frame adoption and fires at the paint that renders it (see
    /// `Job::first_frame_painted`). `None` for preloads and stills.
    paint_signal: Option<FirstFramePainted>,
}

impl LoadSession {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn path(&self) -> &OsStr {
        &self.path
    }

    /// The first-frame paint handshake, for the UI to arm at the
    /// first-frame adoption edge (#76).
    pub(crate) fn paint_signal(&self) -> Option<&FirstFramePainted> {
        self.paint_signal.as_ref()
    }

    /// Whether this session decodes a VIRTUAL source (#65 `stdin:`,
    /// #66 `clipboard:`) — the adoption edge reads it to flag the
    /// display virtual (the navigation/graying semantics that follow; no
    /// backing file).
    pub(crate) fn is_virtual(&self) -> bool {
        self.path == STDIN_NAME || self.path == crate::clipboard::CLIPBOARD_NAME
    }

    /// #43 rename follow-up: retitle an in-flight session whose file was
    /// just renamed, so the adoption arm lands the NEW name in the title
    /// (upstream has a single `current_fd.cFileName` to update; riviv's
    /// session carries its own copy).
    pub(crate) fn set_path(&mut self, path: OsString) {
        self.path = path;
    }

    /// Ask the worker to abandon this job at its next check (immediately
    /// for queued jobs, between frames for animations, after the single
    /// decode for stills — upstream granularity, viv.c:10610).
    pub(crate) fn terminate(&self) {
        self.terminate.store(true, Ordering::Release);
    }

    /// Take every queued reply, in delivery order (upstream's handler
    /// drains the whole list per kick, viv.c:2770).
    pub(crate) fn drain(&self) -> Vec<LoadReply<PixelFrame>> {
        // unwrap on poisoning, as in the sink.
        let mut q = self.queue.lock().unwrap();
        q.drain(..).collect()
    }
}

impl Drop for LoadSession {
    fn drop(&mut self) {
        // Supersession or window teardown: flag the job (the worker skips
        // or abandons it) and free the replies already queued for this
        // dead session immediately — the worker's Arc keeps the queue
        // alive until its final push, but holding already-decoded frames
        // hostage until then would pin a superseded load's full frame
        // budget for no one's benefit.
        self.terminate();
        // unwrap on poisoning, as in the sink.
        self.queue.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_painted_signal_releases_the_wait_immediately() {
        let signal: FirstFramePainted = Arc::new((Mutex::new(true), Condvar::new()));
        let terminate = AtomicBool::new(false);
        let t0 = std::time::Instant::now();
        wait_first_frame_painted(&signal, &terminate);
        assert!(t0.elapsed() < FIRST_FRAME_WAIT_TICK, "no waiting once set");
    }

    #[test]
    fn a_terminated_job_does_not_wait_for_the_paint() {
        let signal: FirstFramePainted = Arc::new((Mutex::new(false), Condvar::new()));
        let terminate = AtomicBool::new(true);
        let t0 = std::time::Instant::now();
        wait_first_frame_painted(&signal, &terminate);
        assert!(
            t0.elapsed() < FIRST_FRAME_WAIT_TICK,
            "terminate short-circuits"
        );
    }

    #[test]
    fn an_unpainted_unterminated_wait_gives_up_at_the_cap() {
        // The minimized-window arm: nothing paints, nobody terminates —
        // the decode must move on when the cap expires (short cap here;
        // production runs 5 s, review PR #84 F8).
        let signal: FirstFramePainted = Arc::new((Mutex::new(false), Condvar::new()));
        let terminate = AtomicBool::new(false);
        let cap = std::time::Duration::from_millis(60);
        let t0 = std::time::Instant::now();
        wait_first_frame_painted_capped(&signal, &terminate, cap);
        assert!(t0.elapsed() >= cap, "the cap is honored");
        assert!(t0.elapsed() < cap + FIRST_FRAME_WAIT_TICK, "no overrun");
    }
}
