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
use std::io::{BufRead, Cursor, Seek};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use image::metadata::Orientation;
use image::{AnimationDecoder, Frames, GenericImageView, ImageDecoder, ImageFormat, ImageReader};

use crate::anim::{self, FrameScheduler, gif_delay_ms};
use crate::icm;
use crate::pixels::{
    PixelFrame, composite_over_background_bgra_in_place, composite_over_background_in_place,
};
use crate::surface::Surface;

/// Cumulative decoded-frame budget. Without a total cap a hostile file
/// could declare an unbounded frame stream and exhaust memory — streaming
/// (#4) makes the gate per-frame instead of per-load, but the limit stands
/// (mid-stream overflow fails the load like #3 did; see `apply_reply` for
/// what the UI does with that failure). Matches the single-image
/// allocation cap the decoder limits already enforce. The `stdin:` read
/// caps its raw stream at the same bound (#65). Counts the decoded
/// (master) bytes — the only frame memory there is since #90 removed the
/// GDI face derivation (#76's on-demand DIBs used to sit outside this
/// budget; the D2D uploads copy out of the master without a standing CPU
/// twin).
pub(crate) const MAX_TOTAL_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// Frame-count budget: a hostile file can pack an unbounded COUNT of tiny
/// frames under the byte cap, so the stream stops at 4096 frames
/// regardless of size (each displayed frame holds its full CPU master in
/// the UI state until the image swaps). The pre-#90 rationale — GDI
/// object exhaustion from one DC + DIB per face — died with the GDI arm.
const MAX_FRAMES: usize = 4096;

// ---------------------------------------------------------------------------
// Worker side: the streaming producer
// ---------------------------------------------------------------------------

/// One step of the load protocol, in delivery order (upstream
/// `_VIV_REPLY_LOAD_IMAGE_*`, viv.c:310-313). Generic over the frame
/// payload: the worker sends pure memory frames (`PixelFrame`, the
/// default — #76: no GDI object crosses the thread boundary); the UI
/// maps them into `Surface`s (the master's UI-thread holder; an
/// infallible move since #90 removed the GDI face derivation) before
/// applying, so the state machine is unit-testable without any UI
/// dependency.
#[derive(Debug, PartialEq)]
pub(crate) enum LoadReply<F = PixelFrame> {
    /// The first decoded frame — the UI swaps the display to it (old image
    /// visible until this arrives). `delay_ms` is the frame's own delay,
    /// relevant only if more frames follow.
    FirstFrame { frame: F, delay_ms: u32 },
    /// A later animation frame; appended to the loaded prefix.
    AdditionalFrame { frame: F, delay_ms: u32 },
    /// The stream ended and every frame has been delivered — the loaded
    /// prefix is the full frame set (unlocks wrap-around).
    Complete,
    /// User-level failure (bad path / undecodable / over budget): the UI
    /// decides keep-vs-clear in `apply_reply`; the status bar shows
    /// upstream's "Failed to load image." (#5). The message itself is a
    /// diagnostic detail — carried for the M2 debug-log channel.
    FailedUser(#[allow(dead_code)] String),
    /// System-level failure: fail loud on the UI thread (ADR 0001) —
    /// the reply exists so the modal box never runs on the worker
    /// thread. (Its historical producer, GDI-object exhaustion at the
    /// face conversion, died with the GDI arm in #90; the variant stays
    /// as the protocol's terminal error class.)
    FatalSystem(String),
}

/// Why the producer stopped early (mapped to terminal replies by
/// `decode_to_sink`).
enum Stop {
    /// User-level: keep the message for the FailedUser reply.
    User(String),
    /// The load was superseded — exit silently; the UI already stopped
    /// reading this session's queue (upstream's thread just returns,
    /// viv.c:10331 exit paths).
    Terminated,
}

/// Decode `path` on the caller's (worker) thread, pushing replies into
/// `sink` as frames materialize. `terminate` is polled between frames;
/// the image crate cannot interrupt a frame mid-decode, so termination
/// lands within one frame (upstream is no finer-grained either).
///
/// Request-time decode inputs threaded to the sinks: the compositing
/// background (upstream stashes the viewport and the composite at request
/// time, viv.c:1557-1558 + the decode-time composite; riviv's viewport
/// snapshot died with the #81 mip retirement) and the `icm` flag's
/// request-time snapshot (#77 — a config flip mid-load must not change
/// frames already in flight).
pub(crate) struct DecodeEnv {
    pub(crate) background: [u8; 3],
    pub(crate) icm: bool,
}

pub(crate) fn decode_to_sink(
    path: &OsStr,
    env: DecodeEnv,
    terminate: &AtomicBool,
    first_frame_painted: Option<&crate::loadthread::FirstFramePainted>,
    sink: &mut dyn FnMut(LoadReply),
) {
    match produce(path, env, terminate, first_frame_painted, sink) {
        Ok(()) => sink(LoadReply::Complete),
        Err(Stop::User(msg)) => sink(LoadReply::FailedUser(msg)),
        Err(Stop::Terminated) => {}
    }
}

/// The `stdin:` virtual display (#65; upstream wishlist viv.c:81): decode
/// an in-memory byte stream through the SAME pipeline as a file — the
/// format is sniffed from the contents (`with_guessed_format` semantics
/// preserved), so the pipe's payload is decoded by magic bytes, not a
/// filename extension. The caller owns the blocking stdin read (see
/// `loadthread.rs`'s detached reader); `bytes` is everything the pipe
/// delivered.
pub(crate) fn decode_bytes_to_sink(
    bytes: &[u8],
    env: DecodeEnv,
    terminate: &AtomicBool,
    first_frame_painted: Option<&crate::loadthread::FirstFramePainted>,
    sink: &mut dyn FnMut(LoadReply),
) {
    // Sniff the format from the stream contents exactly like the file
    // path does (renamed/extensionless pipes still decode); the shown
    // name prefix flows from here into every user-level failure below.
    let outcome = (|| -> Result<(), Stop> {
        let reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| Stop::User(format!("{STDIN_SHOWN_NAME}: {e}")))?;
        decode_reader(
            STDIN_SHOWN_NAME,
            reader,
            env,
            terminate,
            first_frame_painted,
            sink,
        )
    })();
    match outcome {
        Ok(()) => sink(LoadReply::Complete),
        Err(Stop::User(msg)) => sink(LoadReply::FailedUser(msg)),
        Err(Stop::Terminated) => {}
    }
}

/// The shown name for `stdin:` load failures — the message prefix
/// template renders it as `stdin: <reason>`, mirroring the file path's
/// prefix (the DISPLAY name keeps its trailing colon; see
/// `loadthread::STDIN_NAME`. #65).
const STDIN_SHOWN_NAME: &str = "stdin";

/// The shown name for `clipboard:` load failures (#66) — same shape as
/// STDIN_SHOWN_NAME (the DISPLAY name is `clipboard:`, see
/// `clipboard::CLIPBOARD_NAME`).
const CLIPBOARD_SHOWN_NAME: &str = "clipboard";

/// The `clipboard:` virtual display's decode (#66; upstream wishlist
/// viv.c:80/105): one clipboard DIB payload — the raw CF_DIB/CF_DIBV5
/// bytes, or the DIB the CF_BITMAP reader synthesizes — parsed by the
/// pure `dib` module into a final top-down BGRA frame. A one-frame
/// stream exactly like a still decode: first frame, then Complete; no
/// interruption points (the parse is a single pass). Every payload
/// problem is user-level (keep old image, no dialog, no exit — ADR
/// 0001); since #76 the frame itself is pure memory, and since #90 the
/// UI-side conversion (`Surface::from_master`) is an infallible
/// ownership move — the decode side has no system-level failure class
/// left.
pub(crate) fn decode_dib_to_sink(payload: &[u8], env: DecodeEnv, sink: &mut dyn FnMut(LoadReply)) {
    let outcome = (|| -> Result<(), Stop> {
        let dib = crate::dib::parse_dib(payload, env.background, MAX_TOTAL_FRAME_BYTES)
            .map_err(|e| Stop::User(format!("{CLIPBOARD_SHOWN_NAME}: {e}")))?;
        let frame = PixelFrame::from_bgra(dib.width, dib.height, dib.bgra);
        sink(LoadReply::FirstFrame { frame, delay_ms: 0 });
        Ok(())
    })();
    match outcome {
        Ok(()) => sink(LoadReply::Complete),
        Err(Stop::User(msg)) => sink(LoadReply::FailedUser(msg)),
        Err(Stop::Terminated) => {}
    }
}

fn produce(
    path: &OsStr,
    env: DecodeEnv,
    terminate: &AtomicBool,
    first_frame_painted: Option<&crate::loadthread::FirstFramePainted>,
    sink: &mut dyn FnMut(LoadReply),
) -> Result<(), Stop> {
    let shown = path.to_string_lossy();
    // Sniff the format from file contents (upstream GDI+ behavior): renamed or
    // extensionless files still decode. `with_guessed_format` rewinds the
    // stream, so the concrete decoders below start at byte 0.
    let reader =
        ImageReader::open(Path::new(path)).map_err(|e| Stop::User(format!("{shown}: {e}")))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| Stop::User(format!("{shown}: {e}")))?;
    decode_reader(&shown, reader, env, terminate, first_frame_painted, sink)
}

/// Extract and prepare the ICC->sRGB transform while the decoder is
/// still alive — `icc_profile` must be called before `into_frames`/
/// `from_decoder` consumes it (#77). The `icm=0` path short-circuits
/// BEFORE touching the decoder's metadata: the off decode stays
/// byte-for-byte the pre-#77 one (no `icc_profile()` call, so a
/// metadata error cannot even leave a breadcrumb). A metadata read
/// error downgrades to untagged (the decode itself is unaffected);
/// formats without ICC support (BMP/ICO/DIB) return `Ok(None)`.
fn prepare_transform<D: ImageDecoder>(
    decoder: &mut D,
    env: &DecodeEnv,
    shown: &str,
) -> Option<icm::Transform> {
    if !env.icm {
        return None;
    }
    let icc = match decoder.icc_profile() {
        Ok(profile) => profile,
        Err(e) => {
            eprintln!("riviv: icm: {shown}: icc_profile() failed ({e}) — decoding untagged");
            None
        }
    };
    icm::prepare(true, icc, shown)
}

/// The shared post-decode frame pipeline — ADR 0002 D2's order
/// (`decode(RGBA) -> ICM -> composite over bg -> BGRA`), identical for
/// static and animated decodes (#77): when a transform was prepared,
/// `TranslateBitmapBits` maps the RGBA decode buffer straight into the
/// master's BGRA layout (the swizzle folded into its output format) and
/// the composite runs on that BGRA output. A refused transform pass
/// falls back to the untransformed path for the frame; the
/// `transform=None` path is byte-for-byte the pre-#77 one (composite
/// RGBA, `from_rgba` swizzles).
fn assemble_frame(
    width: u32,
    height: u32,
    mut rgba: Vec<u8>,
    env: &DecodeEnv,
    transform: Option<&icm::Transform>,
) -> PixelFrame {
    if let Some(transform) = transform {
        let mut bgra = vec![0u8; rgba.len()];
        if transform.apply(width, height, &rgba, &mut bgra) {
            composite_over_background_bgra_in_place(&mut bgra, env.background);
            return PixelFrame::from_bgra(width, height, bgra);
        }
    }
    composite_over_background_in_place(&mut rgba, env.background);
    PixelFrame::from_rgba(width, height, rgba)
}

/// The shared decode dispatch once a format-guessing reader exists —
/// the single pipeline both the file path (#4) and the `stdin:` bytes
/// (#65) feed. `shown` prefixes the user-level failure messages (the
/// path asked to open / the `stdin:` pseudo-name). GIF and WebP are the
/// only formats whose animation we honor — APNG stays static, matching
/// upstream where GDI+ exposes no time dimension for it. Animated
/// formats go through the frame iterators so transparency compositing
/// and dispose handling are uniform.
fn decode_reader<R: BufRead + Seek>(
    shown: &str,
    reader: ImageReader<R>,
    env: DecodeEnv,
    terminate: &AtomicBool,
    first_frame_painted: Option<&crate::loadthread::FirstFramePainted>,
    sink: &mut dyn FnMut(LoadReply),
) -> Result<(), Stop> {
    let user = |msg: String| Stop::User(format!("{shown}: {msg}"));
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
            // One ICC->sRGB transform for the whole stream, applied to
            // each frame at decode (#77) — upstream's GdipLoadImageFrom-
            // StreamICM slot.
            let transform = prepare_transform(&mut decoder, &env, shown);
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
                env,
                transform.as_ref(),
                terminate,
                first_frame_painted,
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
                return sink_static(decoder, shown, env, sink);
            }
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            let transform = prepare_transform(&mut decoder, &env, shown);
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
                env,
                transform.as_ref(),
                terminate,
                first_frame_painted,
                sink,
            )
        }
        _ => sink_static(
            reader.into_decoder().map_err(|e| user(e.to_string()))?,
            shown,
            env,
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
#[allow(clippy::too_many_arguments)]
fn stream_animation(
    mut frames: Frames<'_>,
    normalize_delay: fn(u32) -> u32,
    orientation: Orientation,
    per_frame_bytes: usize,
    env: DecodeEnv,
    icm: Option<&icm::Transform>,
    terminate: &AtomicBool,
    first_frame_painted: Option<&crate::loadthread::FirstFramePainted>,
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
        let buffer = img.into_rgba8();
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
        // ICM -> composite -> PixelFrame (#77/ADR 0002 D2). The frame
        // itself is pure memory since #76 — through #89 its GDI
        // derivations (and their failure class) lived on the UI thread;
        // since #90 there are none. (The decode-side mip pre-generation
        // decision upstream threads through the same slot,
        // viv.c:10302/10316, was retired with #81.)
        let frame = assemble_frame(w, h, buffer.into_raw(), &env, icm);
        if emitted == 0 {
            sink(LoadReply::FirstFrame { frame, delay_ms });
            // The first-frame paint handshake (#76): hold frame 1's decode
            // until the UI has painted frame 0 — the worker-side GDI
            // serialization master had between replies (without it, the
            // posted reply kicks preempt WM_PAINT and a mid-stream
            // failure can clear the partial display before its first
            // frame ever renders).
            if let Some(signal) = first_frame_painted {
                crate::loadthread::wait_first_frame_painted(signal, terminate);
            }
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
    shown: &str,
    env: DecodeEnv,
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
    // Extract + prepare the ICC->sRGB transform while the decoder is still
    // alive — `from_decoder` consumes it (#77).
    let transform = prepare_transform(&mut decoder, &env, shown);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| user(e.to_string()))?;
    img.apply_orientation(orientation);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(user("empty image".to_string()));
    }
    // ICM -> composite -> PixelFrame (#77/ADR 0002 D2): transparent
    // regions resolve against the sRGB background AFTER the color
    // transform, never before it. (Upstream pre-generates stills' mips at
    // the same slot, viv.c:10749; retired with #81.)
    let frame = assemble_frame(w, h, img.into_rgba8().into_raw(), &env, transform.as_ref());
    // A static image is a one-frame stream: first frame, then Complete from
    // decode_to_sink. delay_ms is unused (no second frame ever follows).
    sink(LoadReply::FirstFrame { frame, delay_ms: 0 });
    Ok(())
}

/// Convert a reply's frame payload — the UI thread maps worker memory
/// frames into Surfaces. Since #90 removed the GDI face derivation the
/// production conversion (`Surface::from_master`) is an infallible
/// ownership move; the Err→FatalSystem arm remains the generic contract
/// of the protocol's terminal error class (kept for the state machine's
/// test net, ADR 0001).
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
    /// advances on schedule. The probe only applies while PLAYING and
    /// compares the rate-scaled delay (#38): upstream's stall branch —
    /// what this synthesizes — lives inside the play gate (viv.c:3195-
    /// 3240) and consumes the scaled delay (viv.c:3209). While PAUSED
    /// the arrival always re-anchors: upstream's timer has been running
    /// since the first frame, re-anchoring while discarding time every
    /// event (viv.c:3195), but riviv's timer only starts at this second
    /// frame — without the reset, a resume landing between the timer
    /// start and its first event would credit the whole paused/decode
    /// gap and skip frames (cubic round 1). `timer_tick` is still zero
    /// here (no timer ever ran), so the reset loses nothing.
    fn second_frame(
        &mut self,
        frame: F,
        delay_ms: u32,
        now: u64,
        freq: u64,
        playback: anim::Playback,
    ) {
        if self.decode_complete {
            // Defensive, same as push_frame: a completed frame set is final.
            return;
        }
        if !playback.playing
            || self.scheduler.at_frame_edge(
                now,
                freq,
                &self.delays_ms,
                self.position,
                playback.rate_pos,
            )
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

    /// Restart the timeline at frame 0 anchored to `now` — the display
    /// adoption of a PARKED image (preload partial adoption, last-cache
    /// activation) needs it: upstream re-anchors at
    /// `_viv_start_first_frame` (viv.c:14313-14319), while the parked
    /// image's anchor dates from when its first frame was decoded, which
    /// may be seconds in the past.
    pub(crate) fn reanchor_at(&mut self, now: u64) {
        self.position = 0;
        self.scheduler = FrameScheduler::new(now);
    }

    /// Convert every frame through `convert` (pure-memory master ->
    /// Surface wrap on the UI thread when a parked image takes the display
    /// — the same worker-to-UI handoff the drain does per reply, batched
    /// here for the adoption path; infallible since #90 deleted the GDI
    /// face derivation — the wrap is a plain ownership move, there is
    /// nothing to fail). The first failure aborts, dropping the remaining
    /// frames; position and completeness survive the mapping.
    pub(crate) fn map_frames<G, E>(
        self,
        convert: impl Fn(F) -> Result<G, E>,
    ) -> Result<LoadedImage<G>, E> {
        Ok(LoadedImage {
            frames: self
                .frames
                .into_iter()
                .map(convert)
                .collect::<Result<Vec<_>, E>>()?,
            delays_ms: self.delays_ms,
            position: self.position,
            scheduler: self.scheduler,
            decode_complete: self.decode_complete,
        })
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
    /// Serves the clipboard image copy (#41), the D2D uploads' master read
    /// and the tests.
    pub(crate) fn surface(&self) -> &F {
        &self.frames[self.position]
    }

    /// Every loaded frame, mutably — #43's rotate pass touches them all
    /// (upstream loops the whole `_viv_frames` array, viv.c:7729-7749).
    pub(crate) fn frames_mut(&mut self) -> &mut [F] {
        &mut self.frames
    }

    pub(crate) fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Whether the decode stream has ended — the closest thing to
    /// upstream's pre-known `_viv_frame_count`: until it flips, a
    /// one-frame display might still be an animation's first frame, which
    /// the slideshow gate must treat as "animation, hold" (cubic round 1).
    pub(crate) fn decode_complete(&self) -> bool {
        self.decode_complete
    }

    /// The 1-based frame position for the status bar's `n / m` counter
    /// (#5; upstream shows `_viv_frame_position + 1`, viv.c:11204). `0`
    /// while nothing is displayed so the counter can hide on `total <= 1`.
    pub(crate) fn frame_position_1based(&self) -> usize {
        self.position + 1
    }

    /// Frames delivered so far. The decode streams, so this is the loaded
    /// prefix until `decode_complete` — the status bar's `m` counts against
    /// it and grows mid-load (image frame iterators cannot report the
    /// total up front; see the module header).
    pub(crate) fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// WM_TIMER body: advance by the time elapsed since the last event and
    /// report the result (the repaint need and the wrap-past-the-last-frame
    /// mark — #37's slideshow gate waits on that loop completion, upstream
    /// `_viv_frame_looped`, viv.c:3243). `stop_at_loop` is that gate: break
    /// the catch-up at the wrap (viv.c:3245-3250). `playback` is the pause
    /// flag + rate position the loop reads (#38; upstream globals, viv.c:
    /// 3195/3209).
    pub(crate) fn advance_on_timer(
        &mut self,
        now: u64,
        freq: u64,
        stop_at_loop: bool,
        playback: anim::Playback,
    ) -> anim::FrameAdvance {
        debug_assert!(self.is_animated(), "static images are never on a timer");
        let advance = self.scheduler.on_timer(
            now,
            freq,
            &self.delays_ms,
            self.position,
            self.decode_complete,
            stop_at_loop,
            playback,
        );
        self.position = advance.position;
        advance
    }

    /// Animation → Frame Step (upstream `_viv_frame_step`, viv.c:9255-9284):
    /// advance one frame with the streaming edge guard, re-anchoring the
    /// timeline to the command moment (the handler's
    /// `_viv_animation_timer_tick_start = now; _viv_timer_tick = 0` pair,
    /// viv.c:9274-9275). `false` = the guard held (or a static image): no
    /// move, no repaint. The looped-mark reset and the pause flip live in
    /// the caller — they happen even when this returns `false` (viv.c:
    /// 9257-9262 run before the frame-count guard).
    pub(crate) fn frame_step(&mut self, now: u64) -> bool {
        let Some(next) = anim::step_position(&self.delays_ms, self.position, self.decode_complete)
        else {
            return false;
        };
        self.position = next;
        self.scheduler = FrameScheduler::new(now);
        true
    }

    /// Animation → Previous Frame (upstream `_viv_frame_prev`, viv.c:9286-
    /// 9315): retreat one frame, wrapping to the last LOADED frame from
    /// frame 0, re-anchoring to the command moment. `false` = static image.
    pub(crate) fn frame_prev(&mut self, now: u64) -> bool {
        if self.delays_ms.len() <= 1 {
            return false;
        }
        self.position = anim::prev_position(&self.delays_ms, self.position);
        self.scheduler = FrameScheduler::new(now);
        true
    }

    /// Animation → First Frame (upstream `VIV_ID_ANIMATION_FRAME_HOME`,
    /// viv.c:1887-1908): jump to frame 0, re-anchoring to the command
    /// moment. `false` = static image.
    pub(crate) fn frame_first(&mut self, now: u64) -> bool {
        if self.delays_ms.len() <= 1 {
            return false;
        }
        self.position = 0;
        self.scheduler = FrameScheduler::new(now);
        true
    }

    /// Animation → Last Frame (upstream `VIV_ID_ANIMATION_FRAME_END`,
    /// viv.c:1910-1932): jump to the last LOADED frame (`loaded_count - 1`,
    /// viv.c:1921 — during a streamed decode that is the prefix's tail),
    /// re-anchoring to the command moment. `false` = static image.
    pub(crate) fn frame_last(&mut self, now: u64) -> bool {
        if self.delays_ms.len() <= 1 {
            return false;
        }
        self.position = self.delays_ms.len() - 1;
        self.scheduler = FrameScheduler::new(now);
        true
    }

    /// The Animation jump commands (upstream `_viv_frame_skip`, viv.c:
    /// 10056-10103): walk `direction_ms` of RAW frame delays forward or
    /// backward and re-anchor to the command moment. Unlike the step
    /// family this neither pauses playback nor resets the looped mark
    /// (the upstream handler touches neither). `false` = static image.
    pub(crate) fn frame_skip(&mut self, now: u64, direction_ms: i32) -> bool {
        if self.delays_ms.len() <= 1 {
            return false;
        }
        self.position = anim::skip_position(
            &self.delays_ms,
            self.position,
            self.decode_complete,
            direction_ms,
        );
        self.scheduler = FrameScheduler::new(now);
        true
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
    /// Reset playback to playing — upstream's `_viv_clear` sets
    /// `_viv_animation_play = 1` (viv.c:1291) and runs at exactly the
    /// display-death moments that emit this: the first-frame adoption
    /// (viv.c:2951) and the failed-load clear (viv.c:2804/2835). The
    /// window layer's open request deliberately does NOT reset playback:
    /// upstream's not-found verdict runs no `_viv_open`/`_viv_clear` at
    /// all (viv.c:5094-5098) and a queued load keeps the old display up
    /// until its reply, so a paused animation must stay paused while its
    /// image is still the one on screen (cubic round 1).
    ResetPlayback,
}

#[derive(Default, Debug)]
pub(crate) struct ReplyOutcome {
    pub(crate) actions: Vec<UiAction>,
    /// System-level failure to fail loud about (ADR 0001) — the caller
    /// shows the fatal modal AFTER dropping its state borrow.
    pub(crate) fatal: Option<String>,
    /// This session's load is over — either the stream completed or it
    /// failed (user-level, before any frame of this session could matter
    /// for the status). The window layer clears its "Loading..." indicator
    /// from this: a fresh open that has not replied yet must keep showing
    /// it, and only the drained session may end what it started (#5;
    /// upstream keys the indicator off the load thread itself,
    /// viv.c:11354-11358 — riviv has no per-load thread to observe).
    pub(crate) load_ended: bool,
    /// The load failed at user level (bad file / undecodable / over
    /// budget) — the status bar shows upstream's "Failed to load image."
    /// (#5) until the next open resets it. Independent of keep-vs-clear:
    /// the display decision lives in `actions`.
    pub(crate) load_failed: bool,
}

/// Apply one load reply to the window's display state — the pure half of
/// the protocol state machine (request -> first frame -> appended frames ->
/// termination, issue #4). `session_id` is the replying session;
/// `displayed_from` tracks which session produced the currently displayed
/// image, so replies from a superseded or failed load are inert.
///
/// `now` is the QPC reading taken before any state borrow (the caller's
/// fatal path must not run across a borrow — PR #10 P1); `freq` is the
/// same QPC frequency, needed to judge whether a late second frame
/// arrived past the loaded edge (see `LoadedImage::second_frame`).
/// `playback` carries the animation pause flag and rate position (#38):
/// the re-anchor probe compares against the rate-scaled delay (viv.c:
/// 3209) and a paused arrival re-anchors unconditionally (cubic round 1
/// — see `second_frame`).
pub(crate) fn apply_reply<F>(
    image: &mut Option<LoadedImage<F>>,
    displayed_from: &mut Option<u64>,
    session_id: u64,
    now: u64,
    freq: u64,
    playback: anim::Playback,
    reply: LoadReply<F>,
) -> ReplyOutcome {
    match reply {
        LoadReply::FirstFrame { frame, delay_ms } => {
            // Upstream always shows the first frame, even of a load being
            // terminated (viv.c:2892-2895: "if we check the terminate flag
            // and hold down right, we might never see an image"); a newer
            // session's first frame simply wins by construction — this
            // handler only ever drains the newest session's queue. The
            // window layer takes the displaced display out BEFORE calling
            // this (its last-cache park, upstream viv.c:2949).
            *image = Some(LoadedImage::first_frame(frame, delay_ms, now));
            *displayed_from = Some(session_id);
            ReplyOutcome {
                actions: vec![
                    UiAction::Invalidate,
                    UiAction::SetWindowTitle,
                    UiAction::ResetPlayback,
                ],
                ..Default::default()
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
                    img.second_frame(frame, delay_ms, now, freq, playback);
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
            ReplyOutcome {
                load_ended: true,
                ..Default::default()
            }
        }
        LoadReply::FailedUser(_) => {
            if *displayed_from == Some(session_id) {
                // A partial image of ours is (or was) on screen — the old
                // image is gone, so clear to blank like upstream's FAILED
                // handler (viv.c:2832-2840).
                *image = None;
                *displayed_from = None;
                ReplyOutcome {
                    actions: vec![
                        UiAction::Invalidate,
                        UiAction::SetWindowTitle,
                        UiAction::ResetPlayback,
                    ],
                    load_ended: true,
                    load_failed: true,
                    ..Default::default()
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
                ReplyOutcome {
                    load_ended: true,
                    load_failed: true,
                    ..Default::default()
                }
            }
        }
        LoadReply::FatalSystem(msg) => ReplyOutcome {
            fatal: Some(msg),
            load_ended: true,
            ..Default::default()
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
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        assert_eq!(
            out.actions,
            vec![
                UiAction::Invalidate,
                UiAction::SetWindowTitle,
                UiAction::ResetPlayback
            ]
        );
        assert_eq!(displayed_from, Some(1));
        let img = image.unwrap();
        assert!(!img.is_animated(), "one frame so far — static");
        assert_eq!(*img.surface(), 1);
    }

    #[test]
    fn the_second_frame_arriving_early_keeps_the_first_frame_anchor() {
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
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
            80,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        assert_eq!(out.actions, Vec::<UiAction>::new());
        let mut img = image.unwrap();
        assert!(img.is_animated());
        assert!(
            !img.advance_on_timer(99, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 1);
        assert!(
            img.advance_on_timer(100, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn the_second_frame_arriving_late_reanchors_to_the_arrival() {
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
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
            5_000,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        let mut img = image.unwrap();
        assert!(
            !img.advance_on_timer(5_099, FREQ, false, anim::Playback::new())
                .repaint,
            "99 ms since arrival"
        );
        assert_eq!(*img.surface(), 1);
        assert!(
            img.advance_on_timer(5_100, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
        // No frame 3: hold at the edge without accumulating.
        assert!(
            !img.advance_on_timer(8_000, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn a_paused_second_frame_reanchors_so_a_fast_resume_cannot_skip() {
        // cubic round 1: pausing before the second frame arrives freezes
        // the anchor at the first frame — and riviv's timer only starts AT
        // the second frame, so nothing has been re-anchoring while paused
        // the way upstream's always-running timer does (viv.c:3195). The
        // arrival must therefore re-anchor unconditionally while paused:
        // a resume landing between the timer start and its first event
        // would otherwise credit the whole paused/decode gap (capped at
        // one second) and skip frames immediately.
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        let paused = anim::Playback {
            playing: false,
            ..anim::Playback::new()
        };
        // Frame 2 arrives 5 s into the pause — far past the loaded edge
        // AND paused, so the re-anchor must come from the paused arm.
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            5_000,
            FREQ,
            paused,
            additional(2),
        );
        let mut img = image.unwrap();
        // Resume 10 ms after the arrival: the first timer event measures
        // from the arrival, so nothing advances yet...
        assert!(
            !img.advance_on_timer(5_010, FREQ, false, anim::Playback::new())
                .repaint,
            "10 ms since the arrival"
        );
        assert_eq!(*img.surface(), 1);
        // ...and frame 2 shows on schedule 100 ms after the arrival.
        assert!(
            img.advance_on_timer(5_100, FREQ, false, anim::Playback::new())
                .repaint
        );
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
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            100,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        // Session 2 superseded session 1 and failed pre-first-frame.
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            150,
            FREQ,
            anim::Playback::new(),
            Reply::FailedUser("bad file".into()),
        );
        // The kept display must NOT reset playback (cubic round 1): the
        // paused old animation stays paused — upstream resets play only in
        // `_viv_clear`, which never runs while the old image survives.
        assert!(!out.actions.contains(&UiAction::ResetPlayback));
        let mut img = image.unwrap();
        assert!(img.is_animated(), "old animation kept");
        // The kept prefix plays and wraps — not stalled at the edge.
        assert!(
            img.advance_on_timer(200, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
        assert!(
            img.advance_on_timer(300, FREQ, false, anim::Playback::new())
                .repaint,
            "wraps at the edge"
        );
        assert_eq!(*img.surface(), 1);
    }

    #[test]
    fn frames_beyond_the_second_do_not_disturb_the_running_timeline() {
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            100,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        // Frame 3 arrives 100 ms later, mid-playback: the anchor stays at
        // the first frame — 350 ms of elapsed playback crosses the delays
        // of frames 1 and 2 (100 + 100) and stops at frame 3's.
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            200,
            FREQ,
            anim::Playback::new(),
            additional(3),
        );
        let mut img = image.unwrap();
        assert!(
            img.advance_on_timer(350, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 3);
    }

    #[test]
    fn stale_session_frames_are_dropped_without_touching_the_display() {
        let mut image = Some(Img::first_frame(1, 100, 0));
        let mut displayed_from = Some(1);
        // A frame from any session other than the one that produced the
        // display is dropped. Per-session queues make this unreachable in
        // practice (the handler drains one queue in delivery order); the
        // guard pins the protocol against future plumbing changes.
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            10,
            FREQ,
            anim::Playback::new(),
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
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        let img = image.as_mut().unwrap();
        assert!(
            img.advance_on_timer(100, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
        // At the edge with the decode still in flight: hold frame 2.
        assert!(
            !img.advance_on_timer(5_000, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
        img.mark_complete();
        // Same edge after completion: wrap to frame 0.
        assert!(
            img.advance_on_timer(5_100, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 1);
    }

    #[test]
    fn reanchoring_a_parked_image_restarts_its_timeline() {
        // A preload adopted mid-decode anchors at its first frame's decode
        // time; reanchor_at moves the anchor to the ADOPTION moment — the
        // first timer event must not credit the parked gap (upstream
        // re-anchors in _viv_start_first_frame, viv.c:14313-14319).
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        let img = image.as_mut().unwrap();
        // Parked long enough for frame 0's delay to expire, then adopted
        // at t=10_000.
        img.reanchor_at(10_000);
        // 50 ms after the new anchor: frame 0's 100 ms delay not yet spent.
        assert!(
            !img.advance_on_timer(10_050, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 1, "position reset to the first frame");
        // At the edge the timeline advances from the NEW anchor.
        assert!(
            img.advance_on_timer(10_100, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn mapping_frames_converts_every_frame_and_keeps_the_timeline() {
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        let mut img = image.unwrap();
        img.mark_complete();
        let mapped = img
            .map_frames(|f| Ok::<String, String>(f.to_string()))
            .unwrap();
        assert_eq!(mapped.frame_count(), 2);
        assert!(mapped.decode_complete());
        // The frame payloads went through the conversion; the parked
        // position survives (adoption re-anchors separately).
        assert_eq!(mapped.frame_position_1based(), 1);
    }

    #[test]
    fn a_frame_mapping_failure_aborts_with_the_error() {
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        let img = image.unwrap();
        let err = img
            .map_frames(|_| Err::<u32, _>("conversion failed".to_string()))
            .err()
            .expect("the mapping must fail");
        assert_eq!(err, "conversion failed");
    }

    #[test]
    fn terminal_replies_report_load_ended_and_failures_report_load_failed() {
        // The status bar's Loading indicator is keyed off these protocol
        // facts (#5): a TERMINAL reply ends the load (Complete/FailedUser/
        // FatalSystem), only FailedUser marks it failed. FirstFrame never
        // ends it — the decode is still streaming (a still image ends at
        // its Complete, one reply later).
        let mut image = None;
        let mut displayed_from = None;
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        assert!(!out.load_ended, "the first frame is not the stream's end");
        assert!(!out.load_failed);

        // Mid-stream replies keep Loading up.
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            5,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        assert!(!out.load_ended);
        assert!(!out.load_failed);

        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            10,
            FREQ,
            anim::Playback::new(),
            Reply::Complete,
        );
        assert!(out.load_ended, "the terminal reply ends the load");
        assert!(!out.load_failed);

        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            20,
            FREQ,
            anim::Playback::new(),
            Reply::FailedUser("bad file".into()),
        );
        assert!(out.load_ended);
        assert!(out.load_failed);

        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            30,
            FREQ,
            anim::Playback::new(),
            Reply::FatalSystem("GDI gone".into()),
        );
        assert!(out.load_ended);
        assert!(!out.load_failed, "system failures fail loud, not status");
    }

    #[test]
    fn frame_position_is_reported_one_based_for_the_status_counter() {
        let mut img = Img::first_frame(1, 100, 0);
        assert_eq!(img.frame_position_1based(), 1);
        assert_eq!(img.frame_count(), 1);
        img.push_frame(2, 100);
        img.push_frame(3, 100);
        assert_eq!(img.frame_count(), 3);
        assert!(
            img.advance_on_timer(100, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(img.frame_position_1based(), 2);
        assert!(
            img.advance_on_timer(200, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(img.frame_position_1based(), 3);
    }

    #[test]
    fn user_failure_before_our_first_frame_keeps_the_old_display() {
        let mut image = Some(Img::first_frame(1, 100, 0));
        let mut displayed_from = Some(7); // displayed image from another load
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            10,
            FREQ,
            anim::Playback::new(),
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
            10,
            FREQ,
            anim::Playback::new(),
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
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            10,
            FREQ,
            anim::Playback::new(),
            Reply::FailedUser("over budget".into()),
        );
        assert_eq!(
            out.actions,
            vec![
                UiAction::Invalidate,
                UiAction::SetWindowTitle,
                UiAction::ResetPlayback
            ]
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
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            10,
            FREQ,
            anim::Playback::new(),
            Reply::FailedUser("over budget".into()),
        );
        assert_eq!(
            out.actions,
            vec![
                UiAction::Invalidate,
                UiAction::SetWindowTitle,
                UiAction::ResetPlayback
            ]
        );
        assert!(image.is_none());
    }

    #[test]
    fn system_failure_is_surfaced_for_the_ui_thread_to_fail_loud() {
        let mut image = None;
        let mut displayed_from = None;
        let out = apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            Reply::FatalSystem("CreateDIBSection failed".into()),
        );
        assert_eq!(out.fatal.as_deref(), Some("CreateDIBSection failed"));
        assert_eq!(out.actions, Vec::<UiAction>::new());
    }

    #[test]
    fn complete_for_a_stale_session_leaves_the_displayed_stream_open() {
        let mut image = None;
        let mut displayed_from = None;
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            frame(1),
        );
        apply_reply(
            &mut image,
            &mut displayed_from,
            1,
            0,
            FREQ,
            anim::Playback::new(),
            additional(2),
        );
        // Session 2's Complete must not freeze session 1's frame set.
        apply_reply(
            &mut image,
            &mut displayed_from,
            2,
            0,
            FREQ,
            anim::Playback::new(),
            Reply::Complete,
        );
        let mut img = image.unwrap();
        img.advance_on_timer(100, FREQ, false, anim::Playback::new());
        // Still open at the edge: holds instead of wrapping.
        assert!(
            !img.advance_on_timer(5_000, FREQ, false, anim::Playback::new())
                .repaint
        );
        assert_eq!(*img.surface(), 2);
    }

    #[test]
    fn reply_frame_mapping_preserves_replies_and_wraps_conversion_failures() {
        // The generic mapping keeps protocol replies intact and turns a
        // conversion Err into the fail-loud reply — a protocol SHAPE kept
        // for future callers; since #90 deleted the face derivation the
        // production convert (master -> Surface) is a plain ownership
        // move, so this arm is unexercised in production (review PR #84
        // F2/F7).
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

#[cfg(test)]
mod stdin_bytes_tests {
    use super::*;

    fn env() -> DecodeEnv {
        DecodeEnv {
            background: [255, 255, 255],
            icm: false,
        }
    }

    fn env_icm(icm: bool) -> DecodeEnv {
        DecodeEnv { icm, ..env() }
    }

    /// The first (and only) pixel of a PixelFrame as (R, G, B) — the
    /// status-bar readout's view of the master buffer.
    fn first_rgb(frame: &PixelFrame) -> (u8, u8, u8) {
        crate::pixels::sample_bgra(&frame.pixels, frame.width as i32, 0, 0).unwrap()
    }

    /// A solid 4×4 PNG of an arbitrary pixel encoded in-memory,
    /// optionally carrying an embedded ICC profile (#77's end-to-end
    /// fixture).
    fn png_pixel_bytes(icc: Option<Vec<u8>>, pixel: [u8; 4]) -> Vec<u8> {
        let mut png = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut png);
        if let Some(profile) = icc {
            image::ImageEncoder::set_icc_profile(&mut enc, profile).expect("icc embed");
        }
        image::ImageEncoder::write_image(
            enc,
            &image::RgbaImage::from_pixel(4, 4, image::Rgba(pixel)),
            4,
            4,
            image::ExtendedColorType::Rgba8,
        )
        .expect("in-memory encode");
        png
    }

    /// The suite's standard fixture: a solid 4×4 PNG of (200,60,10).
    fn png_bytes(icc: Option<Vec<u8>>) -> Vec<u8> {
        png_pixel_bytes(icc, [200, 60, 10, 255])
    }

    /// A two-frame 4×4 animated GIF, optionally carrying an embedded ICC
    /// profile through the de-facto `ICCRGBG1012` application extension —
    /// the block the gif crate's reader collects into `icc_profile`. Both
    /// frames are palette index 0 = (200,60,10); the delays differ so the
    /// per-frame scheduling rides along the same replies.
    fn animated_gif_bytes(icc: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        let palette: &[u8] = &[200, 60, 10, 255, 255, 255];
        // Scoped so the encoder's Drop (which writes the GIF trailer)
        // runs before `out` is returned.
        {
            let mut enc = gif::Encoder::new(&mut out, 4, 4, palette).expect("gif encoder");
            if let Some(icc) = icc {
                enc.write_raw_extension(gif::AnyExtension(0xFF), &[b"ICCRGBG1012", icc])
                    .expect("icc application extension");
            }
            for delay_cs in [10u16, 20] {
                let mut frame = gif::Frame::from_indexed_pixels(4, 4, vec![0u8; 16], None);
                frame.delay = delay_cs;
                enc.write_frame(&frame).expect("gif frame");
            }
        }
        out
    }

    fn first_frame_pixels(bytes: &[u8], icm: bool) -> PixelFrame {
        let terminate = AtomicBool::new(false);
        let mut replies = Vec::new();
        decode_bytes_to_sink(bytes, env_icm(icm), &terminate, None, &mut |r| {
            replies.push(r)
        });
        match replies.into_iter().next() {
            Some(LoadReply::FirstFrame { frame, .. }) => frame,
            other => panic!("expected a first frame, got {other:?}"),
        }
    }

    #[test]
    fn a_tagged_non_srgb_image_is_transformed_before_compositing() {
        // The whole Stage-1 chain end to end: embedded AdobeRGB-like ICC
        // -> sRGB on the decode worker. The source pixel (200,60,10) is
        // deep red under sRGB reading but a muted tone in the wider
        // space; the transform must land near the characterized mscms
        // output (the offline expectation, ±2/255 per the #77 L3
        // acceptance — the screen smoke measured the same value).
        crate::icm::test_fixtures::require_srgb();
        let png = png_bytes(Some(crate::icm::test_fixtures::adobe_like_icc()));
        let frame = first_frame_pixels(&png, true);
        let (r, g, b) = first_rgb(&frame);
        assert!(
            (r as i32 - 239).abs() <= 2 && (g as i32 - 57).abs() <= 2 && b <= 2,
            "got ({r},{g},{b}), expected the characterized mscms (239,57,0) ±2"
        );
    }

    #[test]
    fn a_transparent_tagged_image_composites_over_the_background_after_the_transform() {
        // The transparent-tagged regression net (#77 review F1): a
        // semi-transparent pixel must first take the ICC transform
        // (moving its color) and THEN composite over the windowed
        // background — the alpha surviving the transform is what makes
        // the blend run at all (a forced-opaque alpha would leave the
        // raw transformed color on screen instead). The expected value
        // is the characterized transform output run through the
        // composite's own integer formula `bg + (src - bg) * a / 255`.
        crate::icm::test_fixtures::require_srgb();
        let png = png_pixel_bytes(
            Some(crate::icm::test_fixtures::adobe_like_icc()),
            [200, 60, 10, 128],
        );
        let env = DecodeEnv {
            background: [10, 20, 30],
            ..env_icm(true)
        };
        let terminate = AtomicBool::new(false);
        let mut replies = Vec::new();
        decode_bytes_to_sink(&png, env, &terminate, None, &mut |r| replies.push(r));
        let Some(LoadReply::FirstFrame { frame, .. }) = replies.into_iter().next() else {
            panic!("expected a first frame");
        };
        let (r, g, b) = first_rgb(&frame);
        let transformed = [239i32, 57, 0]; // characterized mscms output
        let expected: Vec<i32> = [10i32, 20, 30]
            .iter()
            .zip(transformed)
            .map(|(bg, t)| bg + (t - bg) * 128 / 255)
            .collect();
        assert!(
            (r as i32 - expected[0]).abs() <= 2
                && (g as i32 - expected[1]).abs() <= 2
                && (b as i32 - expected[2]).abs() <= 2,
            "got ({r},{g},{b}), expected {expected:?} (±2)"
        );
    }

    #[test]
    fn every_frame_of_a_tagged_animation_is_transformed() {
        // #77's animation acceptance: ONE prepared transform, applied to
        // EACH decoded frame — not only the first — through the GIF
        // arm's streaming path, with the per-frame delays riding along
        // the same replies (10/20 centiseconds -> 100/200 ms).
        crate::icm::test_fixtures::require_srgb();
        let gif = animated_gif_bytes(Some(&crate::icm::test_fixtures::adobe_like_icc()));
        let terminate = AtomicBool::new(false);
        let mut replies = Vec::new();
        decode_bytes_to_sink(&gif, env_icm(true), &terminate, None, &mut |r| {
            replies.push(r)
        });
        let mut colors = Vec::new();
        let mut delays = Vec::new();
        for reply in &replies {
            match reply {
                LoadReply::FirstFrame { frame, delay_ms }
                | LoadReply::AdditionalFrame { frame, delay_ms } => {
                    colors.push(first_rgb(frame));
                    delays.push(*delay_ms);
                }
                LoadReply::Complete => {}
                other => panic!("unexpected reply {other:?}"),
            }
        }
        assert_eq!(colors.len(), 2, "two frames decoded, got {colors:?}");
        assert_eq!(delays, vec![100, 200]);
        for (r, g, b) in colors {
            assert!(
                (r as i32 - 239).abs() <= 2 && (g as i32 - 57).abs() <= 2 && b <= 2,
                "every frame lands on the transformed color, got ({r},{g},{b})"
            );
        }
    }

    #[test]
    fn an_srgb_equivalent_tagged_image_decodes_byte_identical() {
        // The identity contract end to end: a different-but-equivalent
        // sRGB blob must NOT round-trip the pixels through the CMM (its
        // own sRGB->sRGB pass drifts a few LSBs).
        crate::icm::test_fixtures::require_srgb();
        let png = png_bytes(Some(crate::icm::test_fixtures::srgb_like_icc()));
        let frame = first_frame_pixels(&png, true);
        assert_eq!(first_rgb(&frame), (200, 60, 10), "verbatim decode");
    }

    #[test]
    fn icm_off_and_untagged_images_keep_the_decoded_bytes_verbatim() {
        let tagged = png_bytes(Some(crate::icm::test_fixtures::adobe_like_icc()));
        let frame = first_frame_pixels(&tagged, false);
        assert_eq!(first_rgb(&frame), (200, 60, 10), "icm=0 ignores the tag");
        let untagged = png_bytes(None);
        let frame = first_frame_pixels(&untagged, true);
        assert_eq!(first_rgb(&frame), (200, 60, 10), "no profile, no work");
    }

    #[test]
    fn stdin_bytes_decode_through_the_same_pipeline_as_files() {
        // A 3×2 PNG encoded in-memory, fed through the `stdin:` entry
        // (#65): the same format-sniffing decode the file path runs —
        // FirstFrame then Complete, dimensions from the decoded frame
        // (the pure-memory PixelFrame, exactly what a file load delivers).
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([200, 100, 50, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("in-memory encode");
        let terminate = AtomicBool::new(false);
        let mut replies = Vec::new();
        decode_bytes_to_sink(&png, env(), &terminate, None, &mut |r| replies.push(r));
        assert_eq!(replies.len(), 2);
        match &replies[0] {
            LoadReply::FirstFrame { frame, delay_ms } => {
                assert_eq!(frame.dims(), (3, 2));
                assert_eq!(*delay_ms, 0, "a static stream has no successor");
            }
            LoadReply::AdditionalFrame { .. }
            | LoadReply::Complete
            | LoadReply::FailedUser(_)
            | LoadReply::FatalSystem(_) => panic!("expected FirstFrame"),
        }
        assert!(matches!(replies[1], LoadReply::Complete));
    }

    #[test]
    fn undecodable_stdin_bytes_fail_user_with_the_pseudo_name_prefix() {
        // Garbage and empty streams: one FailedUser whose message carries
        // the shown name `stdin:` — the same prefix the file path builds
        // from the path it was asked to open.
        for bytes in [&b"not an image at all"[..], &b""[..]] {
            let terminate = AtomicBool::new(false);
            let mut replies = Vec::new();
            decode_bytes_to_sink(bytes, env(), &terminate, None, &mut |r| replies.push(r));
            assert_eq!(replies.len(), 1);
            match &replies[0] {
                LoadReply::FailedUser(msg) => {
                    assert!(msg.starts_with("stdin: "), "{msg}");
                }
                LoadReply::FirstFrame { .. }
                | LoadReply::AdditionalFrame { .. }
                | LoadReply::Complete
                | LoadReply::FatalSystem(_) => panic!("expected FailedUser"),
            }
        }
    }
}
