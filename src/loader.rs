//! Decoding pipeline: file path -> decoded frames -> [`LoadedImage`].
//!
//! Animation (#3, landed): GIF/WebP decode through the image crate's
//! animation iterators, which composite dispose onto the canvas and deliver
//! full-canvas RGBA frames — the equivalent of upstream's per-frame
//! GdipImageSelectActiveFrame drawing (viv.c:10583-10665) and libwebp
//! compositing (webp.c via `_viv_webp_frame_proc`). Every frame, static or
//! animated, is composited over the windowed background at decode time.
//!
//! M2 seam: the background decode thread with its request/reply protocol
//! (#4) lands here beside this synchronous decode; its "next frame not
//! decoded yet" scheduler branch returns with it.

use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::metadata::Orientation;
use image::{AnimationDecoder, Frames, GenericImageView, ImageDecoder, ImageFormat, ImageReader};

use crate::anim::{FrameScheduler, gif_delay_ms};
use crate::pixels::{WINDOWED_BACKGROUND_RGB, composite_over_background_in_place};
use crate::surface::Surface;

/// Cumulative decoded-frame budget. Animations decode every frame up front
/// (#4 streams instead), so without a total cap a hostile file could declare
/// an unbounded frame stream and exhaust memory. Matches the single-image
/// allocation cap the decoder limits already enforce.
const MAX_TOTAL_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// Frame-count budget: every frame surface costs two GDI objects (DC + DIB)
/// and the default per-process GDI limit is 10000, so 4096 frames keeps
/// roughly 1800 objects of headroom for the window itself.
const MAX_FRAMES: usize = 4096;

/// Two failure layers (ADR 0001): user-level keeps the old image / opens an
/// empty window (upstream `_viv_load_failed` behavior); system-level GDI
/// failures are fatal with context.
pub(crate) enum LoadError {
    /// Bad path / undecodable image — user-level, suppressed silently.
    /// The message is unused in M1 and becomes the M2 status-bar text
    /// (upstream "Failed to load image.").
    User(#[allow(dead_code)] String),
    /// DIB/DC infrastructure failure — system-level, fail loud.
    System(String),
}

/// A fully decoded image: one composited frame surface per decoded frame,
/// plus the animation timeline for multi-frame images.
///
/// The scheduler's time anchor is (re)set by the window layer when the image
/// is actually displayed — a decoded-but-not-yet-displayed image has no
/// timeline of its own (upstream `_viv_start_first_frame` is likewise a
/// UI-side action, viv.c:14310).
pub(crate) struct LoadedImage {
    frames: Vec<Surface>,
    /// Per-frame delays in ms; empty for single-frame images (delays only
    /// exist when the image is animated, and then match `frames` 1:1).
    delays_ms: Vec<u32>,
    position: usize,
    scheduler: FrameScheduler,
}

impl LoadedImage {
    fn new(frames: Vec<Surface>, delays_ms: Vec<u32>) -> Self {
        debug_assert!(delays_ms.is_empty() || delays_ms.len() == frames.len());
        LoadedImage {
            frames,
            delays_ms,
            position: 0,
            scheduler: FrameScheduler::new(0),
        }
    }

    /// Canvas width — all frames share it (enforced at decode).
    pub(crate) fn width(&self) -> i32 {
        self.frames[0].width()
    }

    pub(crate) fn height(&self) -> i32 {
        self.frames[0].height()
    }

    /// The frame currently displayed (frame 0 until the timer advances).
    pub(crate) fn surface(&self) -> &Surface {
        &self.frames[self.position]
    }

    pub(crate) fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Show frame 0 and anchor the animation timeline to `tick_start`
    /// (upstream `_viv_start_first_frame`, viv.c:14312-14317).
    pub(crate) fn restart_animation(&mut self, tick_start: u64) {
        self.position = 0;
        self.scheduler.restart(tick_start);
    }

    /// WM_TIMER body: advance by the time elapsed since the last event and
    /// return whether the displayed frame changed (a repaint is due).
    pub(crate) fn advance_on_timer(&mut self, now: u64, freq: u64) -> bool {
        debug_assert!(self.is_animated(), "static images are never on a timer");
        let advance = self
            .scheduler
            .on_timer(now, freq, &self.delays_ms, self.position);
        self.position = advance.position;
        advance.repaint
    }
}

pub(crate) fn load_image(path: &OsStr) -> Result<LoadedImage, LoadError> {
    let shown = path.to_string_lossy();
    let user = |msg: String| LoadError::User(format!("{shown}: {msg}"));
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
        Some(ImageFormat::Gif) => load_gif(reader.into_inner(), &user),
        Some(ImageFormat::WebP) => load_webp(reader.into_inner(), &user),
        _ => load_static(
            reader.into_decoder().map_err(|e| user(e.to_string()))?,
            &user,
        ),
    }
}

fn load_gif(
    reader: BufReader<File>,
    user: &impl Fn(String) -> LoadError,
) -> Result<LoadedImage, LoadError> {
    let mut decoder =
        image::codecs::gif::GifDecoder::new(reader).map_err(|e| user(e.to_string()))?;
    // Set limits before into_frames(): the frame iterator clones them at
    // construction, guarding the per-frame canvas allocation.
    decoder
        .set_limits(image::Limits::default())
        .map_err(|e| user(e.to_string()))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    // GIF frame delays arrive as centiseconds × 10 ms from the image crate;
    // the zero/absent fallback to 100 ms is upstream behavior (viv.c:10749).
    load_frames(decoder.into_frames(), gif_delay_ms, orientation, user)
}

fn load_webp(
    reader: BufReader<File>,
    user: &impl Fn(String) -> LoadError,
) -> Result<LoadedImage, LoadError> {
    let mut decoder =
        image::codecs::webp::WebPDecoder::new(reader).map_err(|e| user(e.to_string()))?;
    decoder
        .set_limits(image::Limits::default())
        .map_err(|e| user(e.to_string()))?;
    // The WebP frame iterator reports num_frames() == 0 for non-animated
    // bitstreams, so still WebP files must take the static decoder or they
    // surface as "no frames decoded" load failures.
    if !decoder.has_animation() {
        return load_static(decoder, user);
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    // WebP delays are the decoder's millisecond values, used as-is like
    // upstream's libwebp path (viv.c:10289 — no zero fallback; the scheduler
    // floors zero to 1 ms instead).
    load_frames(decoder.into_frames(), |ms| ms, orientation, user)
}

/// Decode every frame of an animation into composited surfaces.
///
/// `normalize_delay` maps the image crate's reported delay (ms) to the delay
/// we schedule with; it carries the per-format fallback rules.
fn load_frames(
    frames: Frames<'_>,
    normalize_delay: fn(u32) -> u32,
    orientation: Orientation,
    user: &impl Fn(String) -> LoadError,
) -> Result<LoadedImage, LoadError> {
    let mut frame_surfaces: Vec<Surface> = Vec::new();
    let mut delays_ms: Vec<u32> = Vec::new();
    let mut canvas: Option<(u32, u32)> = None;
    let mut total_frame_bytes: usize = 0;
    for frame in frames {
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
        // The iterators deliver full-canvas frames; a size change means a
        // corrupt stream (defensive — treat it as a bad file, not a crash).
        match canvas {
            None => canvas = Some((w, h)),
            Some((cw, ch)) if cw != w || ch != h => {
                return Err(user("frame size differs from the canvas".to_string()));
            }
            Some(_) => {}
        }
        total_frame_bytes += buffer.len();
        if total_frame_bytes > MAX_TOTAL_FRAME_BYTES || frame_surfaces.len() >= MAX_FRAMES {
            return Err(user("animation exceeds the decode budget".to_string()));
        }
        // Resolve transparency against the windowed background before the
        // DIB copy — the render path has no alpha channel of its own.
        composite_over_background_in_place(&mut buffer, WINDOWED_BACKGROUND_RGB);
        delays_ms.push(delay_ms);
        // Surface failures are purely system-level (GDI allocation); map them
        // into the fail-loud layer (ADR 0001).
        frame_surfaces
            .push(Surface::from_rgba(w, h, &mut buffer.into_raw()).map_err(LoadError::System)?);
    }
    if frame_surfaces.is_empty() {
        return Err(user("no frames decoded".to_string()));
    }
    // A single-frame GIF/WebP is a static image: no delays, no timeline.
    let delays_ms = if frame_surfaces.len() > 1 {
        delays_ms
    } else {
        Vec::new()
    };
    Ok(LoadedImage::new(frame_surfaces, delays_ms))
}

fn load_static<D: ImageDecoder>(
    mut decoder: D,
    user: &impl Fn(String) -> LoadError,
) -> Result<LoadedImage, LoadError> {
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
    let surface = Surface::from_rgba(w, h, &mut rgba).map_err(LoadError::System)?;
    Ok(LoadedImage::new(vec![surface], Vec::new()))
}
