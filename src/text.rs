//! Wide-string text construction for the Win32 layer (pure logic, unit-tested).
//!
//! Title + file-dialog filter + the status-bar text model (#5): the main
//! part's priority text, the frame counter, the dimension/file-size part,
//! and the part-width arithmetic (upstream `_viv_status_update`,
//! viv.c:11106-11413). The GDI shell that measures real text and pushes
//! the results into the common control lives in `status.rs`.

use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use crate::loc::{self, Id};
use crate::panscan;

/// The title-bar filename clause (`config_title_bar_format`, #47; upstream
/// `_viv_update_title`'s switch, viv.c:1226-1246). Upstream's `default:`
/// arm joins case 1 — an unknown ini value (3+) shows the filename, which
/// the clamped `from_config` encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleFormat {
    /// 0 — the full path.
    FullPath,
    /// 1 (and every unknown value, upstream's `default:`) — the final path
    /// component only.
    FilenameOnly,
    /// 2 — no filename clause at all.
    None,
}

impl TitleFormat {
    pub(crate) fn from_config(value: i32) -> Self {
        match value {
            0 => TitleFormat::FullPath,
            2 => TitleFormat::None,
            // 1 and out-of-range values take upstream's default arm.
            _ => TitleFormat::FilenameOnly,
        }
    }
}

/// Upstream title format (`_viv_update_title`): `filename - AppName`,
/// app name only when no image is loaded. Built from raw wide code units so
/// filenames containing unpaired UTF-16 surrogates survive verbatim instead
/// of collapsing into U+FFFD replacement characters. The app name comes
/// from the loc tables (viv.c:1247) — an untranslated brand, so the title
/// itself is language-independent.
pub(crate) fn title_wide(path: Option<&OsStr>, format: TitleFormat) -> Vec<u16> {
    let mut title: Vec<u16> = Vec::new();
    let name = match format {
        TitleFormat::FullPath => path,
        TitleFormat::FilenameOnly => path.and_then(|p| Path::new(p).file_name()),
        TitleFormat::None => None,
    };
    if let Some(name) = name {
        title.extend(name.encode_wide());
        title.extend(" - ".encode_utf16());
    }
    title.extend(loc::get(Id::AppName).encode_utf16());
    title
}

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

/// A nul-terminated wide copy of an OsStr path — the same lossless
/// OsStr→UTF-16 conversion shell.rs uses; #43's file operations share it
/// so exotic names survive to the shell unmangled.
pub(crate) fn to_wide_os(path: &std::ffi::OsStr) -> Vec<u16> {
    path.encode_wide().chain(once(0)).collect()
}

/// File-dialog filter as a double-null-terminated wide string, upstream's
/// exact shape (viv.c:2363): `<label> (<patterns>)\0<patterns>\0<all>
/// (*.*)\0*.*\0` with both labels from the loc tables and the nine
/// supported extensions in upstream's (alphabetical) order.
pub(crate) fn dialog_filter() -> Vec<u16> {
    const PATTERNS: &str = "*.bmp;*.gif;*.ico;*.jpeg;*.jpg;*.png;*.tif;*.tiff;*.webp";
    let images = loc::get(Id::OpenAllImageFiles);
    let all = loc::get(Id::OpenAllFiles);
    format!(
        "{} ({})\0{}\0{} (*.*)\0*.*\0",
        images, PATTERNS, PATTERNS, all
    )
    .encode_utf16()
    .chain(once(0))
    .collect()
}

// ---------------------------------------------------------------------------
// Status bar (#5)
// ---------------------------------------------------------------------------

/// Main-part (part 0) text by priority, first match wins
/// (viv.c:11346-11380 minus temp text, which is a later milestone):
/// Loading > File not found > Failed to load > Slideshow playing > empty.
/// The strings come from the loc tables (viv.c:11358/11364/11370/11376) —
/// English until `loc::init` detects a Chinese UI language.
pub(crate) fn status_main_text(
    loading: bool,
    not_found: bool,
    failed: bool,
    slideshow: bool,
) -> &'static str {
    if loading {
        loc::get(Id::StatusBarLoading)
    } else if not_found {
        loc::get(Id::StatusBarFileNotFound)
    } else if failed {
        loc::get(Id::StatusBarFailedToLoadImage)
    } else if slideshow {
        loc::get(Id::StatusBarSlideshowPlaying)
    } else {
        ""
    }
}

/// The preload part's text: the fixed loc string while a preload load is
/// decoding its first frame, empty (the part disappears) otherwise
/// (upstream viv.c:11210-11213, "PRELOAD" / 预加载).
pub(crate) fn status_preload_text(pending: bool) -> &'static str {
    if pending {
        loc::get(Id::StatusBarPreload)
    } else {
        ""
    }
}

/// Frame counter part: `current / total` (viv.c:11183-11209 — upstream
/// shows `position + 1`; we take the 1-based position directly). Empty for
/// static images; without a pre-known total the streaming decode counts
/// against the loaded prefix (see `loader.rs`).
/// The frame-counter part (upstream viv.c:11176-11208): `pos / total`, or
/// with `remaining` on `- (total - pos) / total` — the frames-left form
/// (`config_frame_minus`, toggled by a status-bar click upstream; riviv
/// exposes it as an Options checkbox, #24). `total` is riviv's loaded
/// prefix (the `m` in the README deviation note), so "remaining" counts
/// down within what has streamed in so far.
pub(crate) fn status_frame_text(position_1based: usize, total: usize, remaining: bool) -> String {
    if total <= 1 {
        String::new()
    } else if remaining {
        format!("- {} / {}", total - position_1based + 1, total)
    } else {
        format!("{position_1based} / {total}")
    }
}

/// Decimal grouping with `,` — the invariant form of upstream's
/// `GetNumberFormat(LOCALE_USER_DEFAULT, Grouping=3, lpThousandSep=",")`
/// (viv.c:11164-11172). Used for the KB figure only; upstream formats the
/// pixel dimensions without separators (`string_format_number`).
pub(crate) fn thousands_grouped(n: u64) -> String {
    let digits = n.to_string();
    let first = digits.len() % 3;
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    if first > 0 {
        out.push_str(&digits[..first]);
    }
    for (i, group) in digits.as_bytes()[first..].chunks(3).enumerate() {
        if i > 0 || first > 0 {
            out.push(',');
        }
        // chunks over ASCII digits are valid UTF-8 slices.
        out.push_str(std::str::from_utf8(group).unwrap_or(""));
    }
    out
}

/// Dimension part: `W x H (N KB)` — file size ceiled to KB and
/// thousands-grouped (viv.c:11132-11181). The size clause is omitted when
/// no file is attached to the display (unknown size); nothing at all is
/// shown when no image is displayed. `backend` (#80) appends the effective
/// D2D backend as a suffix (` d2d/hw` / ` d2d/warp`) — the suffix is part
/// of the measured width (the caller measures the composed string);
/// `None` (through #89 the gdi baseline; since #90 the transient no-stack
/// window before a deferred fatal — see window.rs's backend plumbing)
/// keeps the text byte-identical to upstream.
pub(crate) fn status_dimension_text(
    wide: Option<i32>,
    high: Option<i32>,
    file_bytes: Option<u64>,
    backend: Option<&str>,
) -> String {
    let (Some(w), Some(h)) = (wide, high) else {
        return String::new();
    };
    let mut text = format!("{w} x {h}");
    if let Some(bytes) = file_bytes
        && bytes > 0
    {
        let kb = bytes.div_ceil(1024);
        text.push_str(&format!(" ({} KB)", thousands_grouped(kb)));
    }
    if let Some(backend) = backend {
        text.push(' ');
        text.push_str(backend);
    }
    text
}

/// Smallest status part width at the system DPI: 72 px at 96 DPI, scaled
/// (viv.c:11226 — `(72 * os_logical_wide) / 96`, truncating).
pub(crate) fn min_status_part_wide(dpi: u32) -> i32 {
    (72 * dpi / 96) as i32
}

// ---------------------------------------------------------------------------
// Temp text (#47 — upstream `_viv_status_set_temp_text`, viv.c:11737-11758:
// the flash replaces the main part for 3 s, then the timer clears it)
// ---------------------------------------------------------------------------

/// The panscan/zoom flash (`_viv_status_update_temp_pos_zoom`, viv.c:
/// 11760-11791): pan position as −1..+1 (the 0..=1000 scale over 500, in
/// f32 division like upstream, then nudged ±0.0005 in f64 — upstream's
/// rounding shove), the two factor-table values, and the aspect ratio
/// `(zx * wide) / (zy * high)` in f32 like the C expression. Upstream
/// composes one localized printf template; riviv composes the three labels
/// (byte-identical output — the languages only swap the words around the
/// same number slots).
pub(crate) fn temp_pos_zoom_text(
    pos_x: i32,
    pos_y: i32,
    zoom_x: usize,
    zoom_y: usize,
    image_wide: i32,
    image_high: i32,
) -> String {
    let nudge = |pos: i32| -> f64 {
        let v = ((pos - 500) as f32 / 500.0f32) as f64;
        if v < 0.0 { v - 0.0005 } else { v + 0.0005 }
    };
    let (zx, zy) = (panscan::value_at(zoom_x), panscan::value_at(zoom_y));
    let aspect = (zx * image_wide as f32) / (zy * image_high as f32);
    format!(
        "{} {:.3} {:.3}, {} {:.3} {:.3}, {} {:.3}",
        loc::get(Id::StatusBarPosLabel),
        nudge(pos_x),
        nudge(pos_y),
        loc::get(Id::StatusBarZoomLabel),
        zx,
        zy,
        loc::get(Id::StatusBarAspectLabel),
        aspect,
    )
}

/// The animation-rate flash (`_viv_status_update_temp_animation_rate`,
/// viv.c:11793-11801): the 21-entry table's value at the current index.
pub(crate) fn temp_animation_rate_text(rate: f32) -> String {
    format!("{} {:.3}", loc::get(Id::StatusBarAnimationRateLabel), rate)
}

/// The slideshow-rate flash (`_viv_status_update_slideshow_rate`, viv.c:
/// 11823-11850): the coarsest unit that divides the rate exactly — minutes,
/// then seconds, else raw milliseconds (0 and inexact values read as ms).
pub(crate) fn temp_slideshow_rate_text(rate_ms: i32) -> String {
    let (r, unit) = if rate_ms / 60000 != 0 && rate_ms % 60000 == 0 {
        (rate_ms / 60000, Id::StatusBarMinutes)
    } else if rate_ms / 1000 != 0 && rate_ms % 1000 == 0 {
        (rate_ms / 1000, Id::StatusBarSeconds)
    } else {
        (rate_ms, Id::StatusBarMilliseconds)
    };
    format!(
        "{} {} {}",
        loc::get(Id::StatusBarSlideshowRateLabel),
        r,
        loc::get(unit)
    )
}

// ---------------------------------------------------------------------------
// Pixel-info parts (#47 — upstream viv.c:11217-11221, raw literals, not
// localized)
// ---------------------------------------------------------------------------

/// The POS part: the source-pixel coordinate under the cursor.
pub(crate) fn status_pixel_pos_text(x: i32, y: i32) -> String {
    format!("POS: {x},{y}")
}

/// The RGB part: the source pixel's color.
pub(crate) fn status_pixel_rgb_text(rgb: (u8, u8, u8)) -> String {
    format!("RGB: {},{},{}", rgb.0, rgb.1, rgb.2)
}

/// Right-edge layout for SB_SETPARTS: `[main][preload?][pos][rgb][frame]
/// [dimension]` (viv.c:11296-11342). The preload part EXISTS only while
/// its text is non-empty (upstream pushes its boundary inside
/// `if (*preload_buf)`, so the part count alternates); the POS and RGB
/// parts ALWAYS exist — upstream's `if (pixel_pos_buf)` tests the ARRAY
/// pointer, which is always true (viv.c:11320-11329), so with pixel-info
/// off they sit between main and frame as zero-width invisible parts
/// (bug-for-bug: it is what makes upstream's NM_CLICK part-1 toggle land
/// on the POS readout when pixel-info is on, and on dead space when off).
/// `frame_w`/`dimension_w` are the measured text widths; each part gets a
/// `SM_CXEDGE * 5` text margin, the frame and dimension parts are floored
/// at `min_wide` (preload/pos/rgb are not — upstream measures them raw,
/// viv.c:11266-11290), and the dimension part additionally reserves the
/// size-grip strip (SM_CXVSCROLL + SM_CXBORDER, viv.c:11292). The main
/// part takes what is left (floor 0); the dimension part runs to the right
/// edge (-1). When the window is too cramped, the trailing dimension part
/// keeps its width and the LEADING parts collapse first — upstream
/// accumulates each boundary from the unclamped remainder (viv.c:
/// 11305-11341), which makes the frame boundary `client_w - dimension_w`
/// (possibly negative = a collapsed part).
#[expect(
    clippy::too_many_arguments,
    reason = "the upstream part walk, one width per part"
)]
pub(crate) fn status_part_edges(
    client_w: i32,
    preload_text_w: i32,
    pixel_pos_w: i32,
    pixel_rgb_w: i32,
    frame_text_w: i32,
    dimension_text_w: i32,
    margin: i32,
    grip: i32,
    min_wide: i32,
) -> Vec<i32> {
    // An empty part shows no text, so it takes no width (upstream only
    // measures non-empty buffers, viv.c:11241-11290) and — for the preload
    // part — does not exist at all (the boundary push sits inside the
    // non-empty check). The POS/RGB parts take their raw width when their
    // text shows (no minimum floor, like the preload part).
    let preload_w = if preload_text_w > 0 {
        preload_text_w + margin
    } else {
        0
    };
    let pos_w = if pixel_pos_w > 0 {
        pixel_pos_w + margin
    } else {
        0
    };
    let rgb_w = if pixel_rgb_w > 0 {
        pixel_rgb_w + margin
    } else {
        0
    };
    let frame_w = if frame_text_w > 0 {
        (frame_text_w + margin).max(min_wide)
    } else {
        0
    };
    let dimension_w = if dimension_text_w > 0 {
        (dimension_text_w + margin).max(min_wide) + grip
    } else {
        0
    };
    // Upstream's unclamped accumulation: part_wide (after max(0) for part
    // 0) continues from the raw remainder, so the frame boundary is
    // client_w - dimension_w regardless of the floor above — a cramped
    // window starves the frame counter, never the dimension part
    // (Codex PR #13 round 3).
    let raw = client_w - preload_w - pos_w - rgb_w - frame_w - dimension_w;
    let mut edges = vec![raw.max(0)];
    if preload_w > 0 {
        edges.push(raw + preload_w);
    }
    edges.push(raw + preload_w + pos_w);
    edges.push(raw + preload_w + pos_w + rgb_w);
    edges.push(raw + preload_w + pos_w + rgb_w + frame_w);
    edges.push(-1);
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn title_is_filename_first_then_app_name() {
        let title = title_wide(
            Some(OsStr::new(r"C:\pics\cat.png")),
            TitleFormat::FilenameOnly,
        );
        assert_eq!(String::from_utf16_lossy(&title), "cat.png - riviv");
    }

    #[test]
    fn title_preserves_unpaired_surrogate_code_units() {
        // Windows filenames may contain unpaired UTF-16 surrogates; they must
        // reach the title verbatim (upstream SetWindowTextW takes wide strings).
        let name = OsString::from_wide(&[0xD800, u16::from(b'a')]);
        let title = title_wide(Some(name.as_os_str()), TitleFormat::FilenameOnly);
        let expected: Vec<u16> = [0xD800, u16::from(b'a')]
            .into_iter()
            .chain(" - riviv".encode_utf16())
            .collect();
        assert_eq!(title, expected);
    }

    #[test]
    fn title_without_image_is_app_name_only() {
        assert_eq!(
            String::from_utf16_lossy(&title_wide(None, TitleFormat::FilenameOnly)),
            "riviv"
        );
    }

    #[test]
    fn title_format_full_path_shows_the_whole_path() {
        // viv.c:1226-1234: format 0 uses cFileName — upstream's full path.
        let title = title_wide(Some(OsStr::new(r"C:\pics\cat.png")), TitleFormat::FullPath);
        assert_eq!(String::from_utf16_lossy(&title), r"C:\pics\cat.png - riviv");
    }

    #[test]
    fn title_format_none_shows_the_app_name_alone() {
        let title = title_wide(Some(OsStr::new(r"C:\pics\cat.png")), TitleFormat::None);
        assert_eq!(String::from_utf16_lossy(&title), "riviv");
    }

    #[test]
    fn title_format_from_config_takes_upstreams_default_arm() {
        // viv.c:1235-1246: case 1 AND default share the filename arm, so
        // out-of-range ini values (3+) show the filename, not nothing.
        assert_eq!(TitleFormat::from_config(0), TitleFormat::FullPath);
        assert_eq!(TitleFormat::from_config(1), TitleFormat::FilenameOnly);
        assert_eq!(TitleFormat::from_config(2), TitleFormat::None);
        assert_eq!(TitleFormat::from_config(3), TitleFormat::FilenameOnly);
        assert_eq!(TitleFormat::from_config(-1), TitleFormat::FilenameOnly);
    }

    #[test]
    fn dialog_filter_uses_localized_labels_and_upstream_extension_order() {
        // viv.c:2363 — `<label> (<patterns>)\0<patterns>\0<all> (*.*)\0*.*\0`.
        // The labels read the loc tables, whose default (pre-init) language
        // is English, so this pins the en-US shape deterministically.
        let filter = dialog_filter();
        let s = String::from_utf16_lossy(&filter[..filter.len() - 1]);
        assert_eq!(
            s,
            "All Image Files (*.bmp;*.gif;*.ico;*.jpeg;*.jpg;*.png;*.tif;*.tiff;*.webp)\0\
             *.bmp;*.gif;*.ico;*.jpeg;*.jpg;*.png;*.tif;*.tiff;*.webp\0\
             All Files (*.*)\0*.*\0"
        );
        assert_eq!(*filter.last().unwrap(), 0, "double-null terminated");
    }

    #[test]
    fn main_part_shows_loading_while_a_load_is_in_flight() {
        // Loading outranks a sticky failure from the previous open
        // (viv.c:11354-11362 order) — a replacement load of a bad file must
        // not flash the failure text while decoding.
        assert_eq!(status_main_text(true, true, true, true), "Loading...");
        assert_eq!(status_main_text(true, false, false, false), "Loading...");
    }

    #[test]
    fn main_part_prefers_not_found_over_decode_failure() {
        assert_eq!(
            status_main_text(false, true, true, false),
            "File not found."
        );
        assert_eq!(
            status_main_text(false, false, true, false),
            "Failed to load image."
        );
        assert_eq!(status_main_text(false, false, false, false), "");
    }

    #[test]
    fn main_part_shows_slideshow_playing_below_every_verdict() {
        // viv.c:11374-11377: the slideshow line shows only when no verdict
        // outranks it (the loading/FNF/FAILED chain all come first).
        assert_eq!(
            status_main_text(false, false, false, true),
            "Slideshow playing"
        );
        assert_eq!(status_main_text(true, false, false, true), "Loading...");
        assert_eq!(
            status_main_text(false, true, false, true),
            "File not found."
        );
        assert_eq!(
            status_main_text(false, false, true, true),
            "Failed to load image."
        );
    }

    #[test]
    fn frame_counter_is_one_based_and_empty_for_static_images() {
        assert_eq!(status_frame_text(1, 12, false), "1 / 12");
        assert_eq!(status_frame_text(12, 12, false), "12 / 12");
        assert_eq!(
            status_frame_text(1, 1, false),
            "",
            "static image — no counter"
        );
        assert_eq!(status_frame_text(0, 0, false), "", "no image at all");
        // frame_minus (viv.c:11187-11203): "- remaining / total", where the
        // first frame still counts the whole total as remaining.
        assert_eq!(status_frame_text(1, 12, true), "- 12 / 12");
        assert_eq!(status_frame_text(5, 12, true), "- 8 / 12");
        assert_eq!(status_frame_text(12, 12, true), "- 1 / 12");
    }

    #[test]
    fn dimension_text_pairs_size_with_grouped_kilobytes() {
        // viv.c:11132-11181: plain W x H, size ceiled to KB, KB grouped.
        // No backend (the pre-#90 gdi baseline; now the transient
        // no-stack window) — the text is upstream's byte-for-byte.
        assert_eq!(
            status_dimension_text(Some(1920), Some(1080), Some(1_263_616), None),
            "1920 x 1080 (1,234 KB)"
        );
        assert_eq!(
            status_dimension_text(Some(800), Some(600), Some(1), None),
            "800 x 600 (1 KB)",
            "sub-KB files ceil up to 1 KB"
        );
        assert_eq!(
            status_dimension_text(Some(800), Some(600), Some(1024), None),
            "800 x 600 (1 KB)",
            "an exact KB is not rounded up"
        );
    }

    #[test]
    fn dimension_text_appends_the_d2d_backend_suffix() {
        // #80: the effective D2D backend rides the dimension part (the
        // "which renderer was live" ticket evidence); a None backend
        // shows nothing (no stack — the pre-#90 gdi baseline).
        assert_eq!(
            status_dimension_text(Some(640), Some(480), None, Some("d2d/hw")),
            "640 x 480 d2d/hw"
        );
        assert_eq!(
            status_dimension_text(Some(640), Some(480), Some(2048), Some("d2d/warp")),
            "640 x 480 (2 KB) d2d/warp",
            "the suffix trails the size clause"
        );
    }

    #[test]
    fn dimension_text_omits_unknown_size_and_blank_without_image() {
        assert_eq!(
            status_dimension_text(Some(1920), Some(1080), None, None),
            "1920 x 1080"
        );
        assert_eq!(
            status_dimension_text(Some(1920), Some(1080), Some(0), None),
            "1920 x 1080",
            "upstream skips a zero size (viv.c:11152)"
        );
        assert_eq!(
            status_dimension_text(None, None, Some(5), Some("d2d/hw")),
            ""
        );
        assert_eq!(status_dimension_text(None, None, None, None), "");
    }

    #[test]
    fn thousands_grouping_inserts_commas_every_three_digits() {
        assert_eq!(thousands_grouped(0), "0");
        assert_eq!(thousands_grouped(12), "12");
        assert_eq!(thousands_grouped(123), "123");
        assert_eq!(thousands_grouped(1234), "1,234");
        assert_eq!(thousands_grouped(1234567), "1,234,567");
        assert_eq!(thousands_grouped(1_000_000), "1,000,000");
    }

    #[test]
    fn min_part_width_scales_from_72px_at_96dpi() {
        assert_eq!(min_status_part_wide(96), 72);
        assert_eq!(min_status_part_wide(120), 90);
        assert_eq!(min_status_part_wide(144), 108);
        assert_eq!(min_status_part_wide(192), 144);
    }

    #[test]
    fn part_edges_give_the_main_part_what_is_left() {
        // 1000 px client, frame text 40 px, dimension text 120 px,
        // margin 10 (SM_CXEDGE*5 at 2 px), grip 17 (SM_CXVSCROLL+BORDER),
        // min 72: frame = max(50, 72) = 72; dimension = max(130, 72)+17 = 147.
        // The POS/RGB parts exist but are empty (pixel-info off) — two
        // zero-width boundaries at the raw remainder (upstream's always-true
        // pointer checks, viv.c:11320-11329).
        assert_eq!(
            status_part_edges(1000, 0, 0, 0, 40, 120, 10, 17, 72),
            [781, 781, 781, 853, -1],
            "main fills the remainder, dimension runs to the right edge"
        );
    }

    #[test]
    fn part_edges_floor_small_parts_at_the_minimum() {
        // frame: text 5 + margin 10 = 15 -> floored to 72; dimension empty
        // takes nothing (upstream only measures non-empty buffers).
        assert_eq!(
            status_part_edges(1000, 0, 0, 0, 5, 0, 10, 17, 72),
            [928, 928, 928, 1000, -1]
        );
    }

    #[test]
    fn a_nonempty_preload_inserts_its_part_between_main_and_frame() {
        // Upstream pushes the preload boundary inside `if (*preload_buf)`
        // (viv.c:11312-11316): the part count goes up, the preload text
        // takes width+margin with NO minimum floor (viv.c:11266-11271), and
        // the main part shrinks by it. preload 50 -> 60 wide; raw remainder
        // = 1000 - 60 - 72 - 147 = 721; boundaries accumulate unclamped.
        assert_eq!(
            status_part_edges(1000, 50, 0, 0, 40, 120, 10, 17, 72),
            [721, 781, 781, 781, 853, -1],
            "main shrinks by the preload part, frame/dimension edges keep upstream's accumulation"
        );
    }

    #[test]
    fn a_tiny_preload_text_still_takes_its_raw_width() {
        // No min_wide floor on the preload part (upstream measures it raw,
        // unlike the frame counter): preload 5+10 = 15; the frame counter
        // (5+10 = 15) floors at 72; dimension empty. raw = 1000-15-72.
        assert_eq!(
            status_part_edges(1000, 5, 0, 0, 5, 0, 10, 17, 72),
            [913, 928, 928, 928, 1000, -1]
        );
    }

    #[test]
    fn pixel_parts_take_their_raw_width_between_main_and_frame() {
        // With pixel-info on and the cursor over the image, POS measures
        // 40+10 = 50 and RGB 50+10 = 60 (no minimum floor, like preload);
        // the layout is [main][pos][rgb][frame][dimension] (viv.c:
        // 11320-11329 — upstream's always-true pointer checks push both
        // parts unconditionally; with text they take real width).
        assert_eq!(
            status_part_edges(1000, 0, 40, 50, 40, 120, 10, 17, 72),
            [671, 721, 781, 853, -1],
            "pos/rgb parts sit between main and frame at raw width"
        );
    }

    #[test]
    fn pixel_parts_have_no_minimum_floor() {
        // A tiny POS text (5+10 = 15) is not floored at min_wide — only
        // the frame and dimension parts are (viv.c:11266-11290 measures
        // preload/pos/rgb raw).
        assert_eq!(
            status_part_edges(1000, 0, 5, 0, 5, 0, 10, 17, 72),
            [913, 928, 928, 1000, -1]
        );
    }

    #[test]
    fn a_cramped_window_starves_the_frame_counter_not_the_dimension_part() {
        // frame: 500+10 floored at 72 -> 510; dimension: 510+17 = 527 —
        // both far beyond a 100 px client. Part 0 floors at 0 (viv.c:11308);
        // the zero-width pos/rgb and the frame boundary continue from the
        // UNCLAMPED remainder (client - dimension, viv.c:11331-11336) and
        // may go negative (a collapsed part) — the dimension part keeps its
        // width instead.
        let edges = status_part_edges(100, 0, 0, 0, 500, 500, 10, 17, 72);
        assert_eq!(edges[0], 0, "main part floored at 0 (viv.c:11308)");
        assert_eq!(
            edges[3],
            100 - 527,
            "frame boundary = client - dimension, negative = collapsed"
        );
        assert_eq!(edges[4], -1);
    }

    #[test]
    fn a_cramped_window_keeps_the_dimension_part_the_priority() {
        // client 150, frame needs 72, dimension needs 89: the dimension
        // part keeps its 89 px; the frame counter is squeezed to 61 and the
        // main part to 0 — the trailing dimension text stays readable.
        assert_eq!(
            status_part_edges(150, 0, 0, 0, 5, 60, 10, 17, 72),
            [0, -11, -11, 61, -1]
        );
        // Comfortable case unchanged: main fills the remainder, frame gets
        // its full width, dimension runs to the right edge.
        assert_eq!(
            status_part_edges(500, 0, 0, 0, 5, 60, 10, 17, 72),
            [339, 339, 339, 411, -1]
        );
    }

    #[test]
    fn an_empty_window_shows_only_the_dimension_part_at_the_edge() {
        // No image and no frame counter: everything collapses to the main
        // part plus the (empty, zero-width) slots.
        assert_eq!(
            status_part_edges(640, 0, 0, 0, 0, 0, 10, 17, 72),
            [640, 640, 640, 640, -1]
        );
    }

    #[test]
    fn temp_pos_zoom_at_center_reads_the_nudge_not_zero() {
        // Upstream's ±0.0005 shove (viv.c:11766-11785) moves a centered
        // position to 0.0005, which %.3f renders as "0.001" — the binary
        // double sits just above the decimal midpoint. C printf and Rust
        // format the identical f64 identically, so this is bug-for-bug.
        assert_eq!(
            temp_pos_zoom_text(
                500,
                500,
                panscan::DST_ZOOM_ONE,
                panscan::DST_ZOOM_ONE,
                300,
                200
            ),
            "Pos 0.001 0.001, Zoom 1.000 1.000, Aspect Ratio 1.500"
        );
    }

    #[test]
    fn temp_pos_zoom_carries_the_f32_asymmetry_of_the_c_expressions() {
        // One arrow right (pos 505): (5/500) in f32 lands at
        // 0.009999999776… + 0.0005 → "0.010"; one arrow left (pos 499):
        // f32(−0.002) = −0.002000000094… − 0.0005 → "−0.003" — the two
        // sides round differently because the f32 quotients do (viv.c:
        // 11763-11785 computes the division in float like this).
        assert_eq!(
            temp_pos_zoom_text(
                505,
                499,
                panscan::DST_ZOOM_ONE,
                panscan::DST_ZOOM_ONE,
                300,
                200
            ),
            "Pos 0.010 -0.003, Zoom 1.000 1.000, Aspect Ratio 1.500"
        );
    }

    #[test]
    fn temp_pos_zoom_extremes_and_one_factor_step() {
        // Pan fully left/right = ±1.000; one size step up = factor
        // 1.02 on both axes (the ±1.02 table's first step); the aspect
        // follows the image (300x200 = 1.5) since both axes step together.
        let one_up = panscan::DST_ZOOM_ONE + 1;
        assert_eq!(
            temp_pos_zoom_text(0, 1000, one_up, one_up, 300, 200),
            "Pos -1.000 1.000, Zoom 1.020 1.020, Aspect Ratio 1.500"
        );
    }

    #[test]
    fn temp_animation_rate_formats_to_three_decimals() {
        assert_eq!(temp_animation_rate_text(1.0), "Animation rate 1.000");
        assert_eq!(temp_animation_rate_text(0.5), "Animation rate 0.500");
    }

    #[test]
    fn temp_slideshow_rate_picks_the_coarsest_exact_unit() {
        // viv.c:11823-11845: whole minutes, then whole seconds, else raw
        // milliseconds; zero and inexact values read as ms.
        assert_eq!(temp_slideshow_rate_text(60000), "Slideshow rate 1 minutes");
        assert_eq!(temp_slideshow_rate_text(120000), "Slideshow rate 2 minutes");
        assert_eq!(temp_slideshow_rate_text(1000), "Slideshow rate 1 seconds");
        assert_eq!(temp_slideshow_rate_text(30000), "Slideshow rate 30 seconds");
        assert_eq!(
            temp_slideshow_rate_text(1500),
            "Slideshow rate 1500 milliseconds"
        );
        assert_eq!(temp_slideshow_rate_text(0), "Slideshow rate 0 milliseconds");
    }

    #[test]
    fn pixel_part_texts_are_upstreams_raw_literals() {
        // viv.c:11219-11220 — not localized, printf shapes.
        assert_eq!(status_pixel_pos_text(150, 100), "POS: 150,100");
        assert_eq!(status_pixel_pos_text(0, 0), "POS: 0,0");
        assert_eq!(status_pixel_rgb_text((255, 0, 0)), "RGB: 255,0,0");
        assert_eq!(status_pixel_rgb_text((1, 22, 255)), "RGB: 1,22,255");
    }
}
