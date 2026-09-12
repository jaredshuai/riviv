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

/// Upstream title format (`_viv_update_title`): `filename - AppName`,
/// app name only when no image is loaded. Built from raw wide code units so
/// filenames containing unpaired UTF-16 surrogates survive verbatim instead
/// of collapsing into U+FFFD replacement characters. The app name comes
/// from the loc tables (viv.c:1247) — an untranslated brand, so the title
/// itself is language-independent.
pub(crate) fn title_wide(path: Option<&OsStr>) -> Vec<u16> {
    let mut title: Vec<u16> = Vec::new();
    if let Some(name) = path.and_then(|p| Path::new(p).file_name()) {
        title.extend(name.encode_wide());
        title.extend(" - ".encode_utf16());
    }
    title.extend(loc::get(Id::AppName).encode_utf16());
    title
}

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
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
/// shown when no image is displayed.
pub(crate) fn status_dimension_text(
    wide: Option<i32>,
    high: Option<i32>,
    file_bytes: Option<u64>,
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
    text
}

/// Smallest status part width at the system DPI: 72 px at 96 DPI, scaled
/// (viv.c:11226 — `(72 * os_logical_wide) / 96`, truncating).
pub(crate) fn min_status_part_wide(dpi: u32) -> i32 {
    (72 * dpi / 96) as i32
}

/// Right-edge layout for SB_SETPARTS: `[main][frame][dimension]`
/// (viv.c:11229-11344 minus the preload/pixel-info parts riviv does not
/// have). `frame_w`/`dimension_w` are the measured text widths; each part
/// gets a `SM_CXEDGE * 5` text margin and is floored at `min_wide`; the
/// dimension part additionally reserves the size-grip strip
/// (SM_CXVSCROLL + SM_CXBORDER, viv.c:11292). The main part takes what is
/// left (floor 0); the dimension part runs to the right edge (-1). When
/// the window is too cramped, the trailing dimension part keeps its width
/// and the LEADING parts collapse first — upstream accumulates from the
/// unclamped remainder (viv.c:11306-11341), which makes the frame boundary
/// `client_w - dimension_w` (possibly negative = a collapsed part).
pub(crate) fn status_part_edges(
    client_w: i32,
    frame_text_w: i32,
    dimension_text_w: i32,
    margin: i32,
    grip: i32,
    min_wide: i32,
) -> [i32; 3] {
    // An empty part shows no text, so it takes no width (upstream only
    // measures non-empty buffers, viv.c:11241-11290).
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
    let main_edge = (client_w - frame_w - dimension_w).max(0);
    // Upstream's unclamped accumulation: part_wide (after max(0) for part
    // 0) continues from the raw remainder, so the frame boundary is
    // client_w - dimension_w regardless of the floor above — a cramped
    // window starves the frame counter, never the dimension part
    // (Codex PR #13 round 3).
    [main_edge, client_w - dimension_w, -1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn title_is_filename_first_then_app_name() {
        let title = title_wide(Some(OsStr::new(r"C:\pics\cat.png")));
        assert_eq!(String::from_utf16_lossy(&title), "cat.png - riviv");
    }

    #[test]
    fn title_preserves_unpaired_surrogate_code_units() {
        // Windows filenames may contain unpaired UTF-16 surrogates; they must
        // reach the title verbatim (upstream SetWindowTextW takes wide strings).
        let name = OsString::from_wide(&[0xD800, u16::from(b'a')]);
        let title = title_wide(Some(name.as_os_str()));
        let expected: Vec<u16> = [0xD800, u16::from(b'a')]
            .into_iter()
            .chain(" - riviv".encode_utf16())
            .collect();
        assert_eq!(title, expected);
    }

    #[test]
    fn title_without_image_is_app_name_only() {
        assert_eq!(String::from_utf16_lossy(&title_wide(None)), "riviv");
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
        assert_eq!(
            status_dimension_text(Some(1920), Some(1080), Some(1_263_616)),
            "1920 x 1080 (1,234 KB)"
        );
        assert_eq!(
            status_dimension_text(Some(800), Some(600), Some(1)),
            "800 x 600 (1 KB)",
            "sub-KB files ceil up to 1 KB"
        );
        assert_eq!(
            status_dimension_text(Some(800), Some(600), Some(1024)),
            "800 x 600 (1 KB)",
            "an exact KB is not rounded up"
        );
    }

    #[test]
    fn dimension_text_omits_unknown_size_and_blank_without_image() {
        assert_eq!(
            status_dimension_text(Some(1920), Some(1080), None),
            "1920 x 1080"
        );
        assert_eq!(
            status_dimension_text(Some(1920), Some(1080), Some(0)),
            "1920 x 1080",
            "upstream skips a zero size (viv.c:11152)"
        );
        assert_eq!(status_dimension_text(None, None, Some(5)), "");
        assert_eq!(status_dimension_text(None, None, None), "");
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
        assert_eq!(
            status_part_edges(1000, 40, 120, 10, 17, 72),
            [781, 853, -1],
            "main fills the remainder, dimension runs to the right edge"
        );
    }

    #[test]
    fn part_edges_floor_small_parts_at_the_minimum() {
        // frame: text 5 + margin 10 = 15 -> floored to 72; dimension empty
        // takes nothing (upstream only measures non-empty buffers).
        assert_eq!(status_part_edges(1000, 5, 0, 10, 17, 72), [928, 1000, -1]);
    }

    #[test]
    fn a_cramped_window_starves_the_frame_counter_not_the_dimension_part() {
        // frame: 500+10 floored at 72 -> 510; dimension: 510+17 = 527 —
        // both far beyond a 100 px client. Part 0 floors at 0 (viv.c:11308);
        // the frame boundary continues from the UNCLAMPED remainder
        // (client - dimension, viv.c:11331-11336) and may go negative (a
        // collapsed part) — the dimension part keeps its width instead.
        let edges = status_part_edges(100, 500, 500, 10, 17, 72);
        assert_eq!(edges[0], 0, "main part floored at 0 (viv.c:11308)");
        assert_eq!(
            edges[1],
            100 - 527,
            "frame boundary = client - dimension, negative = collapsed"
        );
        assert_eq!(edges[2], -1);
    }

    #[test]
    fn a_cramped_window_keeps_the_dimension_part_the_priority() {
        // client 150, frame needs 72, dimension needs 89: the dimension
        // part keeps its 89 px; the frame counter is squeezed to 61 and the
        // main part to 0 — the trailing dimension text stays readable.
        assert_eq!(status_part_edges(150, 5, 60, 10, 17, 72), [0, 61, -1]);
        // Comfortable case unchanged: main fills the remainder, frame gets
        // its full width, dimension runs to the right edge.
        assert_eq!(status_part_edges(500, 5, 60, 10, 17, 72), [339, 411, -1]);
    }

    #[test]
    fn an_empty_window_shows_only_the_dimension_part_at_the_edge() {
        // No image and no frame counter: everything collapses to the main
        // part plus the (empty, zero-width) slots.
        assert_eq!(status_part_edges(640, 0, 0, 10, 17, 72), [640, 640, -1]);
    }
}
