//! Decoding pipeline: file path -> decoded RGBA -> [`Surface`].
//!
//! M2 seam: the background decode thread with its request/reply protocol (#4)
//! and streamed animation-frame decoding (#3) land here beside the
//! synchronous M1 loader.

use std::ffi::OsStr;
use std::path::Path;

use image::{GenericImageView, ImageDecoder};

use crate::surface::Surface;

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

pub(crate) fn load_surface(path: &OsStr) -> Result<Surface, LoadError> {
    let shown = path.to_string_lossy();
    let user = |msg: String| LoadError::User(format!("{shown}: {msg}"));
    // Sniff the format from file contents (upstream GDI+ behavior): renamed or
    // extensionless files still decode.
    let reader = image::ImageReader::open(Path::new(path)).map_err(|e| user(e.to_string()))?;
    let reader = reader
        .with_guessed_format()
        .map_err(|e| user(e.to_string()))?;
    let mut decoder = reader.into_decoder().map_err(|e| user(e.to_string()))?;
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
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| user(e.to_string()))?;
    img.apply_orientation(orientation);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(user("empty image".to_string()));
    }
    let mut rgba = img.into_rgba8().into_raw();
    Surface::from_rgba(w, h, &mut rgba)
}
