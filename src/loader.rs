//! Decode pipeline + the load reply state machine (#4).
//!
//! Decoding runs on a background thread (see `loadthread.rs`): this module
//! is the producer side — file path -> per-frame replies pushed into a
//! sink — and the UI side — [`apply_reply`], the pure protocol state
//! machine that assembles replies into a [`LoadedImage`]. The protocol
//! mirrors upstream's load thread (viv.c:10331-10831): first frame first,
//! every animation frame as its own reply, a terminal reply at the end.
//! Frame compositing/dispose is the decoder's job, exactly like #3.
//!
//! One upstream deviation by necessity: the image crate's frame iterators
//! cannot report a total frame count up front (GDI+/libwebp can,
//! viv.c first-frame reply carries `frame_count`), so "the animation is
//! fully loaded" is signaled by the terminal `Complete` reply instead of a
//! pre-known total; the scheduler treats the loaded-prefix edge as "wait"
//! until then (see `anim.rs`).

use std::ffi::OsStr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use image::metadata::Orientation;
use image::{AnimationDecoder, Frames, GenericImageView, ImageDecoder, ImageFormat, ImageReader};

use crate::anim::{FrameScheduler, gif_delay_ms};
use crate::pixels::{WINDOWED_BACKGROUND_RGB, composite_over_background_in_place};
use crate::surface::{DibFrame, Surface};

/// Cumulative decoded-frame budget. Without a total cap a hostile file
/// could declare an unbounded frame stream and exhaust memory — streaming
/// (#4) makes the gate per-frame instead of per-load, but the limit stands
/// (mid-stream overflow fails the load like #3 did; see `apply_reply` for
/// what the UI does with that failure). Matches the single-image
/// allocation cap the decoder limits already enforce.
const MAX_TOTAL_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// Frame-count budget: every frame surface costs two GDI objects (DC + DIB)
/// and the default per-process GDI limit is 10000, so 4096 frames keeps
/// roughly 1800 objects of headroom for the window itself.
const MAX_FRAMES: usize = 4096;

// ---------------------------------------------------------------------------
// Worker side: the streaming producer
// ---------------------------------------------------------------------------

/// One step of the load protocol, in delivery order (upstream
/// `_VIV_REPLY_LOAD_IMAGE_*`, viv.c:310-313). Generic over the frame
/// payload: the worker sends bare DIBs (`DibFrame`, the default); the UI
/// maps them to DC-wrapping `Surface`s before applying, so the state
/// machine is unit-testable without GDI.
#[derive(Debug, PartialEq)]
pub(crate) enum LoadReply<F = DibFrame> {
    /// The first decoded frame — the UI swaps the display to it (old image
    /// visible until this arrives). `delay_ms` is the frame's own delay,
    /// relevant only if more frames follow.
    FirstFrame { frame: F, delay_ms: u32 },
    /// A later animation frame; appended to the loaded prefix.
    AdditionalFrame { frame: F, delay_ms: u32 },
    /// The stream ended and every frame has been delivered — the loaded
    /// prefix is the full frame set (unlocks wrap-around).
    Complete,
    /// User-level failure (bad path / undecodable / over budget): the
    /// message is unused until the M2 status bar (#5, upstream "Failed to
    /// load image."); the UI decides keep-vs-clear in `apply_reply`.
    FailedUser(#[allow(dead_code)] String),
    /// System-level failure (GDI exhaustion): fail loud on the UI thread
    /// (ADR 0001) — the reply exists so the modal box never runs on the
    /// worker thread.
    FatalSystem(String),
}

/// Why the producer stopped early (mapped to terminal replies by
/// `decode_to_sink`).
enum Stop {
    /// User-level: keep the message for the FailedUser reply.
    User(String),
    /// System-level: fail loud via FatalSystem.
    Fatal(String),
    /// The load was superseded — exit silently; the UI already stopped
    /// reading this session's queue (upstream's thread just returns,
    /// viv.c:10331 exit paths).
    Terminated,
}

/// Decode `path` on the caller's (worker) thread, pushing replies into
/// `sink` as frames materialize. `terminate` is polled between frames;
/// the image crate cannot interrupt a frame mid-decode, so termination
/// lands within one frame (upstream is no finer-grained either).
pub(crate) fn decode_to_sink(
    path: &OsStr,
    terminate: &AtomicBool,
    sink: &mut dyn FnMut(LoadReply),
) {
    match produce(path, terminate, sink) {
        Ok(()) => sink(LoadReply::Complete),
        Err(Stop::User(msg)) => sink(LoadReply::FailedUser(msg)),
        Err(Stop::Fatal(msg)) => sink(LoadReply::FatalSystem(msg)),
        Err(Stop::Terminated) => {}
    }
}

fn produce(
    path: &OsStr,
    terminate: &AtomicBool,
    sink: &mut dyn FnMut(LoadReply),
) -> Result<(), Stop> {
    let shown = path.to_string_lossy();
    let user = |msg: String| Stop::User(format!("{shown}: {msg}"));
    // Sniff the format from file contents (upstream GDI+ behavior): renamed or
    // extensionless files still decode. `with_guessed_format` rewinds the
    // stream, so the concrete decoders below start at byte 0.
    let reader = ImageReader::open(Path::new(path)).map_err(|e| user(e.to_string()))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| user(e.to_string()))?;
    // GIF and WebP are the only formats whose animation we honor — APNG stays
    // static, matching upstream where GDI+ exposes no time dimension for it.
    // Animated formats go through the frame iterators so transparency
    // compositing and dispose handling are uniform.
    match reader.format() {
        Some(ImageFormat::Gif) => {
            let mut decoder = image::codecs::gif::GifDecoder::new(reader.into_inner())
                .map_err(|e| user(e.to_string()))?;
            // Set limits before into_frames(): the frame iterator clones them at
            // construction, guarding the per-frame canvas allocation.
            decoder
                .set_limits(image::Limits::default())
                .map_err(|e| user(e.to_string()))?;
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            // GIF frame delays arrive as centiseconds × 10 ms from the image crate;
            // the zero/absent fallback to 100 ms is upstream behavior (viv.c:10749).
            // Every frame costs a full canvas, so the budget gate knows the per-frame
            // cost up front (decoder dimensions == canvas dimensions).
            let (w, h) = decoder.dimensions();
            let per_frame_bytes = w as usize * h as usize * 4;
            stream_animation(
                decoder.into_frames(),
                gif_delay_ms,
                orientation,
                per_frame_bytes,
                terminate,
                sink,
            )
        }
        Some(ImageFormat::WebP) => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(reader.into_inner())
                .map_err(|e| user(e.to_string()))?;
            decoder
                .set_limits(image::Limits::default())
                .map_err(|e| user(e.to_string()))?;
            // The WebP frame iterator reports num_frames() == 0 for non-animated
            // bitstreams, so still WebP files must take the static decoder or they
            // surface as "no frames decoded" load failures.
            if !decoder.has_animation() {
                return sink_static(decoder, sink);
            }
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            // WebP delays are the decoder's millisecond values, used as-is like
            // upstream's libwebp path (viv.c:10289 — no zero fallback; the scheduler
            // floors zero to 1 ms instead). Per-frame budget cost as for GIF above.
            let (w, h) = decoder.dimensions();
            let per_frame_bytes = w as usize * h as usize * 4;
            stream_animation(
                decoder.into_frames(),
                |ms| ms,
                orientation,
                per_frame_bytes,
                terminate,
                sink,
            )
        }
        _ => sink_static(
            reader.into_decoder().map_err(|e| user(e.to_string()))?,
            sink,
        ),
    }
}

/// Decode one frame at a time of an animation, replying per frame
/// (upstream first frame + additional frames, viv.c:10304/10318/10719-10751).
///
/// `normalize_delay` maps the image crate's reported delay (ms) to the delay
/// we schedule with; it carries the per-format fallback rules.
/// `per_frame_bytes` is the canvas cost of one frame (`w * h * 4`), known
/// from the decoder header before any frame is decoded.
fn stream_animation(
    mut frames: Frames<'_>,
    normalize_delay: fn(u32) -> u32,
    orientation: Orientation,
    per_frame_bytes: usize,
    terminate: &AtomicBool,
    sink: &mut dyn FnMut(LoadReply),
) -> Result<(), Stop> {
    let user = |msg: String| Stop::User(msg);
    let mut emitted = 0usize;
    let mut canvas: Option<(u32, u32)> = None;
    let mut total_frame_bytes: usize = 0;
    // Explicit next() loop (not `for`): the budget gate must run BEFORE the
    // iterator is asked for the next frame — Frames::next() decodes and
    // allocates the frame's full canvas before returning it, so a gate that
    // runs after the pull would let a hostile file overshoot the budget by
    // one canvas (plus the iterator's own compositing canvas) before the
    // user-level error lands. A file landing exactly on the budget edge is
    // rejected conservatively (fail the load) — distinguishing it from
    // "one more frame exists" would require decoding that frame.
    loop {
        if terminate.load(Ordering::Relaxed) {
            return Err(Stop::Terminated);
        }
        if emitted >= MAX_FRAMES || total_frame_bytes + per_frame_bytes > MAX_TOTAL_FRAME_BYTES {
            return Err(user("animation exceeds the decode budget".to_string()));
        }
        let Some(frame) = frames.next() else {
            break;
        };
        let frame = frame.map_err(|e| user(e.to_string()))?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        // Both animated formats report whole-millisecond delays (GIF cs × 10,
        // WebP ms); the division can only ever truncate a fractional value
        // neither format produces.
        let delay_ms = normalize_delay(numer / denom.max(1));
        // Orientation applies to every frame (upstream runs
        // _viv_orientate_hbitmap per frame, viv.c:10615-10623).
        let mut img = image::DynamicImage::ImageRgba8(frame.into_buffer());
        img.apply_orientation(orientation);
        let mut buffer = img.into_rgba8();
        let (w, h) = buffer.dimensions();
        if w == 0 || h == 0 {
            return Err(user("empty frame".to_string()));
        }
        // The iterators deliver full-canvas frames matching the decoder's
        // canvas; a deviation means a corrupt stream (defensive — treat it as
        // a bad file, not a crash).
        match canvas {
            None => canvas = Some((w, h)),
            Some((cw, ch)) if cw != w || ch != h => {
                return Err(user("frame size differs from the canvas".to_string()));
            }
            Some(_) => {}
        }
        total_frame_bytes += buffer.len();
        // Resolve transparency against the windowed background before the
        // DIB copy — the render path has no alpha channel of its own.
        composite_over_background_in_place(&mut buffer, WINDOWED_BACKGROUND_RGB);
        // DIB failures are purely system-level (GDI allocation); fail
        // loud (ADR 0001) through the FatalSystem reply.
        let frame = DibFrame::from_rgba(w, h, &mut buffer.into_raw()).map_err(Stop::Fatal)?;
        if emitted == 0 {
            sink(LoadReply::FirstFrame { frame, delay_ms });
        } else {
            sink(LoadReply::AdditionalFrame { frame, delay_ms });
        }
        emitted += 1;
    }
    if emitted == 0 {
        return Err(user("no frames decoded".to_string()));
    }
    Ok(())
}

fn sink_static<D: ImageDecoder>(
    mut decoder: D,
    sink: &mut dyn FnMut(LoadReply),
) -> Result<(), Stop> {
    let user = |msg: String| Stop::User(msg);
    // Guard against hostile/corrupt headers declaring huge pixel sizes: the
    // default 512MB allocation cap turns them into user-level load errors
    // instead of an OOM crash (ADR 0001 — a bad file must not kill the viewer).
    decoder
        .set_limits(image::Limits::default())
        .map_err(|e| user(e.to_string()))?;
    // Apply EXIF orientation before reading dimensions (upstream
    // config_orientation = 1, config.c:90) so phone photos are not sideways.
    // Best-effort like upstream os.c:1545-1600: malformed orientation metadata
    // falls back to no rotation instead of rejecting the image.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| user(e.to_string()))?;
    img.apply_orientation(orientation);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(user("empty image".to_string()));
    }
    let mut rgba = img.into_rgba8().into_raw();
    // Transparent regions show the windowed background instead of the raw
    // RGB under the alpha channel (upstream composites over
    // config_windowed_background_color; identity for opaque images).
    composite_over_background_in_place(&mut rgba, WINDOWED_BACKGROUND_RGB);
    let frame = DibFrame::from_rgba(w, h, &mut rgba).map_err(Stop::Fatal)?;
    // A static image is a one-frame stream: first frame, then Complete from
    // decode_to_sink. delay_ms is unused (no second frame ever follows).
    sink(LoadReply::FirstFrame { frame, delay_ms: 0 });
    Ok(())
}

/// Convert a reply's frame payload — the UI thread maps worker DIBs into
/// DC-carrying Surfaces (memory DCs belong to their creating thread), with
/// a conversion failure surfacing as the system-level FatalSystem reply
/// (GDI exhaustion, ADR 0001).
pub(crate) fn map_reply_frame<E, F>(
    reply: LoadReply<E>,
    convert: impl Fn(E) -> Result<F, String>,
) -> LoadReply<F> {
    match reply {
        LoadReply::FirstFrame { frame, delay_ms } => match convert(frame) {
            Ok(frame) => LoadReply::FirstFrame { frame, delay_ms },
            Err(msg) => LoadReply::FatalSystem(msg),
        },
        LoadReply::AdditionalFrame { frame, delay_ms } => match convert(frame) {
            Ok(frame) => LoadReply::AdditionalFrame { frame, delay_ms },
            Err(msg) => LoadReply::FatalSystem(msg),
        },
        LoadReply::Complete => LoadReply::Complete,
        LoadReply::FailedUser(msg) => LoadReply::FailedUser(msg),
        LoadReply::FatalSystem(msg) => LoadReply::FatalSystem(msg),
    }
}

// ---------------------------------------------------------------------------
// UI side: the display image + the reply state machine
// ---------------------------------------------------------------------------

/// Frame dimensions, carried by the first frame like upstream's first-frame
/// reply (`wide`/`high`, viv.c:10345-10346) — the window sizes itself from
/// them without knowing the concrete frame type.
pub(crate) trait FrameDims {
    fn dims(&self) -> (i32, i32);
}

impl FrameDims for Surface {
    fn dims(&self) -> (i32, i32) {
        (self.width(), self.height())
    }
}

/// The displayed image, assembled incrementally from replies: frames grow
/// as the background decode streams them in, `decode_complete` flips on
/// the terminal reply and unlocks wrap-around.
///
/// The timeline is anchored once, at the first frame's display (upstream
/// `_viv_start_first_frame`, viv.c:14312-14317). A frame arriving *before*
/// the loaded edge keeps that anchor (playback advances on schedule); one
/// arriving *after* it re-anchors to the arrival — synthesizing what a
/// running timer's stall branch would have zeroed while waiting
/// (viv.c:3233-3240).
pub(crate) struct LoadedImage<F = crate::surface::Surface> {
    frames: Vec<F>,
    /// Per-frame delays in ms, parallel to `frames` (a frame's delay only
    /// matters once a successor exists).
    delays_ms: Vec<u32>,
    position: usize,
    scheduler: FrameScheduler,
    decode_complete: bool,
}

impl<F> LoadedImage<F> {
    /// The image as of its first frame: displayed statically until a second
    /// frame arrives (the timeline is anchored, but no timer runs yet).
    fn first_frame(frame: F, delay_ms: u32, tick_start: u64) -> Self {
        LoadedImage {
            frames: vec![frame],
            delays_ms: vec![delay_ms],
            position: 0,
            scheduler: FrameScheduler::new(tick_start),
            decode_complete: false,
        }
    }

    /// Append a streamed frame. The window layer derives the animation
    /// timer from the resulting image state (upstream knows the frame
    /// count up front and starts the timer at the first frame; without a
    /// pre-known count the second frame is the earliest animation signal).
    fn push_frame(&mut self, frame: F, delay_ms: u32) {
        if self.decode_complete {
            // Defensive: the producer never sends frames past Complete; a
            // completed frame set is final, so a late frame is dropped.
            return;
        }
        self.frames.push(frame);
        self.delays_ms.push(delay_ms);
    }

    /// A second frame arrived, making the image animated. If playback had
    /// already reached the loaded edge (frame 0's delay fully elapsed
    /// while no timer ran), the anchor resets to the arrival — the first
    /// timer event must not credit the whole decode gap against frames
    /// that just landed. Before the edge, the anchor stands and playback
    /// advances on schedule.
    fn second_frame(&mut self, frame: F, delay_ms: u32, now: u64, freq: u64) {
        if self.decode_complete {
            // Defensive, same as push_frame: a completed frame set is final.
            return;
        }
        if self
            .scheduler
            .at_frame_edge(now, freq, &self.delays_ms, self.position)
        {
            self.scheduler = FrameScheduler::new(now);
        }
        self.push_frame(frame, delay_ms);
    }

    /// The stream ended: the loaded prefix is the full frame set
    /// (wrap-around unlocked at the last-frame edge).
    fn mark_complete(&mut self) {
        self.decode_complete = true;
    }

    /// Canvas width — all frames share it (enforced at decode).
    pub(crate) fn width(&self) -> i32
    where
        F: FrameDims,
    {
        self.frames[0].dims().0
    }

    pub(crate) fn height(&self) -> i32
    where
        F: FrameDims,
    {
        self.frames[0].dims().1
    }

    /// The frame currently displayed (frame 0 until the timer advances).
    pub(crate) fn surface(&self) -> &F {
        &self.frames[self.position]
    }

    pub(crate) fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// WM_TIMER body: advance by the time elapsed since the last event and
    /// return whether the displayed frame changed (a repaint is due).
    pub(crate) fn advance_on_timer(&mut self, now: u64, freq: u64) -> bool {
        debug_assert!(self.is_animated(), "static images are never on a timer");
        let advance = self.scheduler.on_timer(
            now,
            freq,
            &self.delays_ms,
            self.position,
            self.decode_complete,
        );
        self.position = advance.position;
        advance.repaint
    }
}

/// What the window layer must do after a reply is applied (batched so the
/// Win32 calls happen outside the state borrow, in a fixed order). The
/// animation timer is deliberately NOT an action: the window layer
/// reconciles it from the final image state after the whole drain, so a
/// batch that both starts and retires an animation cannot leave a stale
/// timer running.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UiAction {
    /// The pixels on screen changed — repaint.
    Invalidate,
    /// Adopt the session's path (window title + Ctrl+O initial dir).
    SetWindowTitle,
    /// Resize the window to the fitted image (startup first frame).
    ResizeWindowToImage,
}

#[derive(Default, Debug)]
pub(crate) struct ReplyOutcome {
    pub(crate) actions: Vec<UiAction>,
    /// System-level failure to fail loud about (ADR 0001) — the caller
    /// shows the fatal modal AFTER dropping its state borrow.
    pub(crate) fatal: Option<String>,
}

/// Apply one load reply to the window's display state — the pure half of
/// the protocol state machine (request -> first frame -> appended frames ->
/// termination, issue #4). `session_id` is the replying session;
/// `displayed_from` tracks which session produced the currently displayed
/// image, so replies from a superseded or failed load are inert.
///
/// `startup_resize_session` is the session the startup resize belongs to —
/// only THAT session's first frame consumes it, so a startup load that is
/// superseded or fails pre-first-frame cannot make a later open resize
/// the window.
///
/// `now` is the QPC reading taken before any state borrow (the caller's
/// fatal path must not run across a borrow — PR #10 P1); `freq` is the
/// same QPC frequency, needed to judge whether a late second frame
/// arrived past the loaded edge (see `LoadedImage::second_frame`).
pub(crate) fn apply_reply<F>(
    image: &mut Option<LoadedImage<F>>,
    displayed_from: &mut Option<u64>,
    session_id: u64,
    startup_resize_session: &mut Option<u64>,
    now: u64,
    freq: u64,
    reply: LoadReply<F>,
) -> ReplyOutcome {
    match reply {
        LoadReply::FirstFrame { frame, delay_ms } => {
            // Upstream always shows the first frame, even of a load being
            // terminated (viv.c:2892-2895: "if we check the terminate flag
            // and hold down right, we might never see an image"); a newer
            // session's first frame simply wins by construction — this
            // handler only ever drains the newest session's queue.
            *image = Some(LoadedImage::first_frame(frame, delay_ms, now));
            *displayed_from = Some(session_id);
            let mut actions = vec![UiAction::Invalidate, UiAction::SetWindowTitle];
            if *startup_resize_session == Some(session_id) {
                *startup_resize_session = None;
                actions.push(UiAction::ResizeWindowToImage);
            }
            ReplyOutcome {
                actions,
                fatal: None,
            }
        }
        LoadReply::AdditionalFrame { frame, delay_ms } => {
            // Only append to the image this session produced (defensive:
            // per-session queues already make cross-session delivery
            // impossible, this pins the protocol).
            if *displayed_from == Some(session_id)
                && let Some(img) = image.as_mut()
            {
                if img.is_animated() {
                    img.push_frame(frame, delay_ms);
                } else {
                    img.second_frame(frame, delay_ms, now, freq);
                }
            }
            ReplyOutcome::default()
        }
        LoadReply::Complete => {
            if *displayed_from == Some(session_id)
                && let Some(img) = image.as_mut()
            {
                img.mark_complete();
            }
            ReplyOutcome::default()
        }
        LoadReply::FailedUser(_) => {
            if *displayed_from == Some(session_id) {
                // A partial image of ours is (or was) on screen — the old
                // image is gone, so clear to blank like upstream's FAILED
                // handler (viv.c:2832-2840).
                *image = None;
                *displayed_from = None;
                ReplyOutcome {
                    actions: vec![UiAction::Invalidate, UiAction::SetWindowTitle],
                    fatal: None,
                }
            } else {
                // Nothing of this session ever reached the screen (failure
                // before its first frame): keep the old image and title,
                // no popup, no exit (issue #4 mandate; upstream clears here
                // — registered in README Differences). The kept image's own
                // stream was superseded (that is why this load ran at all),
                // so no frames or Complete will ever arrive for it —
                // finalize its prefix instead of leaving playback stalled
                // at the loaded edge forever.
                if let Some(img) = image.as_mut() {
                    img.mark_complete();
                }
                ReplyOutcome::default()
            }
        }
        LoadReply::FatalSystem(msg) => ReplyOutcome {
            actions: Vec::new(),
            fatal: Some(msg),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// freq = 1000 makes one tick one millisecond, so timing assertions
    /// read like the delays they assert against.
    const FREQ: u64 = 1000;

    /// Minimal frame payload: the state machine only stores and counts.
    type Img = LoadedImage<u32>;
    type Reply = LoadReply<u32>;

    fn frame(n: u32) -> Reply {
        Reply::FirstFrame {
            frame: n,
            delay_ms: 100,
        }
    }

    fn additional(n: u32) -> Reply {
        Reply::AdditionalFrame {
            frame: n,
            delay_ms: 100,
        }
    }

    #[test]
    fn first_frame_swaps_the_display() {
        let mut image = Some(Img::first_frame(7, 100, 0));
        image.as_mut().unwrap().push_frame(8, 100); // old image animated
        let mut displayed_from = Some(99);
        let mut resize_session = None;
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        assert_eq!(
            out.actions,
            vec![UiAction::Invalidate, UiAction::SetWindowTitle]
        );
        assert_eq!(displayed_from, Some(1));
        let img = image.unwrap();
        assert!(!img.is_animated(), "one frame so far — static");
        assert_eq!(*img.surface(), 1);
    }

    #[test]
    fn the_startup_resize_belongs_to_its_session_only() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = Some(1);
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        assert!(out.actions.contains(&UiAction::ResizeWindowToImage));
        assert_eq!(resize_session, None, "consumed with the action");
        // A later first frame (another open) must not resize again.
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            &mut resize_session,
            10,
            FREQ,
            frame(2),
        );
        assert!(!out.actions.contains(&UiAction::ResizeWindowToImage));

        // A startup load superseded before its first frame: the pending
        // resize belongs to the dead session, so the superseding open's
        // first frame must not consume it (image switches never resize).
        let mut resize_session = Some(5);
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            6,
            &mut resize_session,
            20,
            FREQ,
            frame(3),
        );
        assert!(!out.actions.contains(&UiAction::ResizeWindowToImage));
        assert_eq!(resize_session, Some(5), "not consumed by another session");
    }

    #[test]
    fn the_second_frame_arriving_early_keeps_the_first_frame_anchor() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        // Frame 2 arrives at t=80, before frame 1's 100 ms delay expires:
        // playback must advance on schedule at t=100, not restart the
        // clock at the arrival (upstream runs the timer from the first
        // frame; the stall branch only discards time once the edge is
        // actually reached, viv.c:3233-3240).
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            80,
            FREQ,
            additional(2),
        );
        assert_eq!(out.actions, Vec::<UiAction>::new());
        let mut img = image.unwrap();
        assert!(img.is_animated());
        assert!(!img.advance_on_timer(99, FREQ));
        assert_eq!(*img.surface(), 1);
        assert!(img.advance_on_timer(100, FREQ));
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn the_second_frame_arriving_late_reanchors_to_the_arrival() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        // Frame 2 arrives 5 s after frame 1's display: playback had long
        // reached the loaded edge with no timer running (a one-frame prefix
        // is static), so the anchor resets to the arrival — the decode gap
        // is NOT credited against the freshly arrived frames. Upstream's
        // timer would have been running the stall branch all along, zeroing
        // the accumulated time at every tick (viv.c:3233-3240); the re-anchor
        // synthesizes exactly that.
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            5_000,
            FREQ,
            additional(2),
        );
        let mut img = image.unwrap();
        assert!(!img.advance_on_timer(5_099, FREQ), "99 ms since arrival");
        assert_eq!(*img.surface(), 1);
        assert!(img.advance_on_timer(5_100, FREQ));
        assert_eq!(*img.surface(), 2);
        // No frame 3: hold at the edge without accumulating.
        assert!(!img.advance_on_timer(8_000, FREQ));
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn a_replacement_failing_pre_first_frame_finalizes_the_kept_animation() {
        // An animation mid-decode is superseded by a new open whose decode
        // fails before producing a frame: the old image is kept (issue #4
        // mandate), but its own stream is dead — the superseding session
        // replaced it and no worker will deliver its remaining frames or
        // Complete. Finalizing the kept prefix lets playback wrap at the
        // edge instead of stalling forever.
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            100,
            FREQ,
            additional(2),
        );
        // Session 2 superseded session 1 and failed pre-first-frame.
        apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            &mut resize_session,
            150,
            FREQ,
            Reply::FailedUser("bad file".into()),
        );
        let mut img = image.unwrap();
        assert!(img.is_animated(), "old animation kept");
        // The kept prefix plays and wraps — not stalled at the edge.
        assert!(img.advance_on_timer(200, FREQ));
        assert_eq!(*img.surface(), 2);
        assert!(img.advance_on_timer(300, FREQ), "wraps at the edge");
        assert_eq!(*img.surface(), 1);
    }

    #[test]
    fn frames_beyond_the_second_do_not_disturb_the_running_timeline() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            100,
            FREQ,
            additional(2),
        );
        // Frame 3 arrives 100 ms later, mid-playback: the anchor stays at
        // the first frame — 350 ms of elapsed playback crosses the delays
        // of frames 1 and 2 (100 + 100) and stops at frame 3's.
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            200,
            FREQ,
            additional(3),
        );
        let mut img = image.unwrap();
        assert!(img.advance_on_timer(350, FREQ));
        assert_eq!(*img.surface(), 3);
    }

    #[test]
    fn stale_session_frames_are_dropped_without_touching_the_display() {
        let mut image = Some(Img::first_frame(1, 100, 0));
        let mut displayed_from = Some(1);
        let mut resize_session = None;
        // A frame from any session other than the one that produced the
        // display is dropped. Per-session queues make this unreachable in
        // practice (the handler drains one queue in delivery order); the
        // guard pins the protocol against future plumbing changes.
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            &mut resize_session,
            10,
            FREQ,
            additional(2),
        );
        assert_eq!(out.actions, Vec::<UiAction>::new());
        assert_eq!(displayed_from, Some(1));
        assert!(!image.as_ref().unwrap().is_animated(), "frame dropped");
    }

    #[test]
    fn completion_unlocks_wrapping_at_the_loaded_edge() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            additional(2),
        );
        let img = image.as_mut().unwrap();
        assert!(img.advance_on_timer(100, FREQ));
        assert_eq!(*img.surface(), 2);
        // At the edge with the decode still in flight: hold frame 2.
        assert!(!img.advance_on_timer(5_000, FREQ));
        assert_eq!(*img.surface(), 2);
        img.mark_complete();
        // Same edge after completion: wrap to frame 0.
        assert!(img.advance_on_timer(5_100, FREQ));
        assert_eq!(*img.surface(), 1);
    }

    #[test]
    fn user_failure_before_our_first_frame_keeps_the_old_display() {
        let mut image = Some(Img::first_frame(1, 100, 0));
        let mut displayed_from = Some(7); // displayed image from another load
        let mut resize_session = None;
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            10,
            FREQ,
            Reply::FailedUser("bad file".into()),
        );
        assert_eq!(out.actions, Vec::<UiAction>::new());
        assert_eq!(out.fatal, None);
        assert_eq!(*image.as_ref().unwrap().surface(), 1, "old image kept");
        assert_eq!(displayed_from, Some(7));
        // Same for a window that never displayed anything.
        let mut image = None;
        let mut displayed_from = None;
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            10,
            FREQ,
            Reply::FailedUser("bad file".into()),
        );
        assert_eq!(out.actions, Vec::<UiAction>::new());
        assert!(image.is_none(), "still blank");
    }

    #[test]
    fn user_failure_after_our_first_frame_clears_the_display() {
        // Static partial (first frame only).
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            10,
            FREQ,
            Reply::FailedUser("over budget".into()),
        );
        assert_eq!(
            out.actions,
            vec![UiAction::Invalidate, UiAction::SetWindowTitle]
        );
        assert!(image.is_none(), "partial image cleared");
        assert_eq!(displayed_from, None);

        // Animated partial: same clear (the window layer stops the timer
        // from the cleared image state).
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            additional(2),
        );
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            10,
            FREQ,
            Reply::FailedUser("over budget".into()),
        );
        assert_eq!(
            out.actions,
            vec![UiAction::Invalidate, UiAction::SetWindowTitle]
        );
        assert!(image.is_none());
    }

    #[test]
    fn system_failure_is_surfaced_for_the_ui_thread_to_fail_loud() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            Reply::FatalSystem("CreateDIBSection failed".into()),
        );
        assert_eq!(out.fatal.as_deref(), Some("CreateDIBSection failed"));
        assert_eq!(out.actions, Vec::<UiAction>::new());
    }

    #[test]
    fn complete_for_a_stale_session_leaves_the_displayed_stream_open() {
        let mut image = None;
        let mut displayed_from = None;
        let mut resize_session = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            &mut resize_session,
            0,
            FREQ,
            additional(2),
        );
        // Session 2's Complete must not freeze session 1's frame set.
        apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            &mut resize_session,
            0,
            FREQ,
            Reply::Complete,
        );
        let mut img = image.unwrap();
        img.advance_on_timer(100, FREQ);
        // Still open at the edge: holds instead of wrapping.
        assert!(!img.advance_on_timer(5_000, FREQ));
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn reply_frame_mapping_wraps_worker_dibs_and_surfaces_wrap_failures() {
        // DibFrame -> Surface conversion is the UI-side job; the pure
        // mapping keeps protocol replies intact and turns conversion
        // failures into the fail-loud reply.
        let convert = |n: u32| -> Result<u32, String> {
            if n == 13 {
                Err("CreateCompatibleDC failed".into())
            } else {
                Ok(n + 100)
            }
        };
        let first = map_reply_frame(frame(1), convert);
        assert_eq!(
            first,
            Reply::FirstFrame {
                frame: 101,
                delay_ms: 100
            }
        );
        let failed = map_reply_frame(additional(13), convert);
        assert_eq!(
            failed,
            Reply::FatalSystem("CreateCompatibleDC failed".to_string())
        );
        let terminal = map_reply_frame(Reply::Complete, convert);
        assert_eq!(terminal, Reply::Complete);
        let user = map_reply_frame(Reply::FailedUser("x".into()), convert);
        assert_eq!(user, Reply::FailedUser("x".to_string()));
    }
}
