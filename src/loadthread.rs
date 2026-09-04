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
//! cannot leak. Frames cross the boundary as bare DIBs (`DibFrame`); the
//! UI thread wraps them in DC-carrying Surfaces (see `surface.rs`).

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::loader::{LoadReply, decode_to_sink};
use crate::surface::DibFrame;

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

/// One queued decode request; crossing the channel requires `Send`
/// (DibFrame and the Arcs are, HWND via the wrapper above).
struct Job {
    path: OsString,
    hwnd: SendHwnd,
    terminate: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<LoadReply<DibFrame>>>>,
}

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

    /// Queue `path` for decoding and return the session that owns its
    /// replies. The old session (if any) must be dropped by the caller —
    /// its Drop flags the job, and the worker skips it at the next check.
    pub(crate) fn request(&self, hwnd: HWND, path: OsString) -> LoadSession {
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let terminate = Arc::new(AtomicBool::new(false));
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        // The worker consumes its own copy; the session keeps the
        // original for the UI (window title / Ctrl+O initial dir).
        let worker_path = path.clone();
        let job = Job {
            path: worker_path,
            hwnd: SendHwnd(hwnd),
            terminate: Arc::clone(&terminate),
            queue: Arc::clone(&queue),
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
            path,
            hwnd: SendHwnd(hwnd),
            terminate,
            queue,
        } = job;
        // Superseded while still queued: nothing was decoded, nothing to
        // reply — skip before touching the file.
        if terminate.load(Ordering::Relaxed) {
            continue;
        }
        // The sink gets its own handle for its retry loop; the original
        // stays with the decode call's terminate parameter.
        let sink_terminate = Arc::clone(&terminate);
        let mut sink = move |reply: LoadReply<DibFrame>| {
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
        decode_to_sink(&path, &terminate, &mut sink);
    }
}

/// The in-flight (or queued) load the UI is interested in. Dropping it
/// flags the worker to abandon the job at its next terminate check.
pub(crate) struct LoadSession {
    id: u64,
    /// The file being loaded — the UI adopts it as the window title/path
    /// when this session's first frame takes the display.
    path: OsString,
    terminate: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<LoadReply<DibFrame>>>>,
}

impl LoadSession {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn path(&self) -> &OsStr {
        &self.path
    }

    /// Ask the worker to abandon this job at its next check (immediately
    /// for queued jobs, between frames for animations, after the single
    /// decode for stills — upstream granularity, viv.c:10610).
    pub(crate) fn terminate(&self) {
        self.terminate.store(true, Ordering::Release);
    }

    /// Take every queued reply, in delivery order (upstream's handler
    /// drains the whole list per kick, viv.c:2770).
    pub(crate) fn drain(&self) -> Vec<LoadReply<DibFrame>> {
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
