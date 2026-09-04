//! Background decode session: the thread, the reply queue, and the kick
//! message (issue #4's port of upstream's load-thread plumbing).
//!
//! Upstream model (viv.c): `_viv_open` spawns `_viv_load_image_thread_proc`,
//! which decodes one frame at a time and posts replies through
//! `_viv_reply_add` — a linked list under a critical section, with a
//! `PostMessage` kick only when the queue transitions empty -> non-empty
//! (viv.c:10869-10905). The UI drains the whole queue per kick
//! (`_VIV_WM_REPLY`, viv.c:2762-3060). A new open while a load is in
//! flight sets `_viv_load_image_terminate` and the thread exits at its
//! next per-frame check (viv.c:10610); at quit the UI *waits* for it
//! ("it's critical we wait for load image to finish", viv.c:5470-5479).
//!
//! Riviv translation: one session per open, each with its own queue
//! (superseded sessions' replies are simply never drained — no global
//! terminate flag needed for correctness, though we still set one so a
//! superseded worker stops wasting CPU). The queue — not the message —
//! owns the replies, so a kick lost to window teardown cannot leak.
//! GDI note: `Surface` handles are created on the worker and consumed on
//! the UI thread; GDI objects are process-global with no thread
//! affinity, the same cross-thread handoff upstream does with frame
//! HBITMAPs (first-frame/additional-frame replies, viv.c:2900/2989).

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::loader::{LoadReply, decode_to_sink};

/// Kick message: posted when the reply queue goes empty -> non-empty, so
/// at most one message is in flight per drain cycle (upstream
/// `_VIV_WM_REPLY`; `WM_APP + n` is the reserved range for private
/// window-class messages).
pub(crate) const REPLY_KICK_MESSAGE: u32 = WM_APP + 1;

/// Session ids, so the UI can tell which load produced the displayed
/// image (`displayed_from` in `WindowState`).
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// HWND is a plain handle value but holds a raw pointer, so std does not
/// consider it `Send`.
#[derive(Clone, Copy)]
struct SendHwnd(HWND);
// SAFETY: moving the handle into the worker is sound — the worker only
// posts messages to it, and a post to a dead window just fails. Same
// cross-thread handoff as upstream's `_viv_reply_add` posting to
// `_viv_hwnd` (viv.c:10902).
unsafe impl Send for SendHwnd {}

pub(crate) struct LoadSession {
    id: u64,
    /// The file being loaded — the UI adopts it as the window title/path
    /// when this session's first frame takes the display.
    path: OsString,
    /// Set when this session is superseded or the window is closing; the
    /// worker checks it between frames (upstream `_viv_load_image_terminate`).
    terminate: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<LoadReply>>>,
    /// Taken by [`LoadSession::join`]; dropping without joining detaches
    /// (fine — see Drop below).
    handle: Option<JoinHandle<()>>,
}

impl LoadSession {
    /// Start decoding `path` on a worker thread (upstream: the CreateThread
    /// arm of `_viv_open`, viv.c:1569). A spawn failure is system-level
    /// (ADR 0001): the caller fails loud.
    pub(crate) fn spawn(hwnd: HWND, path: OsString) -> Result<Self, String> {
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let terminate = Arc::new(AtomicBool::new(false));
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let worker_terminate = Arc::clone(&terminate);
        // The queue — not the posted message — owns the replies, so the
        // worker can safely outlive a kick that never gets dispatched.
        let worker_queue = Arc::clone(&queue);
        // The worker consumes its own copy; the session keeps the original
        // for the UI (window title / Ctrl+O initial dir on first frame).
        let worker_path = path.clone();
        let send_hwnd = SendHwnd(hwnd);
        let handle = std::thread::Builder::new()
            .name("riviv-load".into())
            .spawn(move || {
                // Mention the wrapper as a whole before destructuring:
                // precise closure capture would otherwise move only the raw
                // HWND field (edition 2021+ captures minimal paths, even
                // through patterns), and the field alone is not Send.
                let send_hwnd = send_hwnd;
                let SendHwnd(hwnd) = send_hwnd;
                let mut sink = move |reply: LoadReply| {
                    // unwrap: a poisoned lock means some thread panicked
                    // while holding the queue — an undefined state we fail
                    // loud on (ADR 0001) rather than limp past.
                    let mut q = worker_queue.lock().unwrap();
                    let was_empty = q.is_empty();
                    q.push_back(reply);
                    drop(q);
                    if was_empty {
                        // Posted outside the lock like upstream (viv.c:10898-
                        // 10903). A failed post (window gone) is not an
                        // error: the queue is freed when the session is.
                        // SAFETY: `hwnd` is a valid handle captured for this
                        // worker; PostMessageW never dereferences it on this
                        // thread and validates it on the owning thread's
                        // queue, failing cleanly if the window is gone.
                        let _ = unsafe {
                            PostMessageW(Some(hwnd), REPLY_KICK_MESSAGE, WPARAM(0), LPARAM(0))
                        };
                    }
                };
                decode_to_sink(&worker_path, &worker_terminate, &mut sink);
            })
            .map_err(|e| format!("load thread spawn failed: {e}"))?;
        Ok(LoadSession {
            id,
            path,
            terminate,
            queue,
            handle: Some(handle),
        })
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn path(&self) -> &OsStr {
        &self.path
    }

    /// Ask the worker to stop at its next frame boundary. Never blocks:
    /// the image crate cannot interrupt a frame mid-decode, so a frame
    /// already in flight still completes (upstream granularity, viv.c:10610).
    pub(crate) fn terminate(&self) {
        self.terminate.store(true, Ordering::Release);
    }

    /// Take every queued reply, in delivery order (upstream's handler
    /// drains the whole list per kick, viv.c:2770).
    pub(crate) fn drain(&self) -> Vec<LoadReply> {
        // unwrap on poisoning, as in the sink.
        let mut q = self.queue.lock().unwrap();
        q.drain(..).collect()
    }

    /// Wait for the worker to exit. Called at window teardown — upstream
    /// waits INFINITE for the same reason ("it's critical we wait for load
    /// image to finish before we kill the main window", viv.c:5476); the
    /// terminate flag bounds the wait to the in-flight frame.
    pub(crate) fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LoadSession {
    fn drop(&mut self) {
        // Supersession or window teardown without an explicit join: flag
        // the worker (it exits within a frame) and detach it. Its Arcs keep
        // the queue alive until the worker's last push, so replies never
        // outlive their allocation and GDI teardown happens on whichever
        // thread drops the last reference — both legal for GDI.
        self.terminate();
    }
}
