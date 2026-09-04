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

/// Upstream title format (`_viv_update_title`): `filename - AppName`,
/// app name only when no image is loaded. Built from raw wide code units so
/// filenames containing unpaired UTF-16 surrogates survive verbatim instead
/// of collapsing into U+FFFD replacement characters.
pub(crate) fn title_wide(path: Option<&OsStr>) -> Vec<u16> {
    let mut title: Vec<u16> = Vec::new();
    if let Some(name) = path.and_then(|p| Path::new(p).file_name()) {
        title.extend(name.encode_wide());
        title.extend(" - ".encode_utf16());
    }
    title.extend("riviv".encode_utf16());
    title
}

pub(crate) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

/// File-dialog filter as a double-null-terminated wide string.
pub(crate) fn dialog_filter() -> Vec<u16> {
    "Images (*.png;*.jpg;*.jpeg;*.bmp;*.ico;*.tif;*.tiff;*.gif;*.webp)\0\
     *.png;*.jpg;*.jpeg;*.bmp;*.ico;*.tif;*.tiff;*.gif;*.webp\0\
     All files (*.*)\0*.*\0"
        .encode_utf16()
        .chain(once(0))
        .collect()
}

// ---------------------------------------------------------------------------
// Status bar (#5)
// ---------------------------------------------------------------------------

/// Exact upstream strings (localization_en_us.h:197-199).
pub(crate) const STATUS_LOADING: &str = "Loading...";
pub(crate) const STATUS_FILE_NOT_FOUND: &str = "File not found.";
pub(crate) const STATUS_LOAD_FAILED: &str = "Failed to load image.";

/// Main-part (part 0) text by priority, first match wins
/// (viv.c:11346-11380 minus temp text and slideshow, which are later
/// milestones): Loading > File not found > Failed to load > empty.
pub(crate) fn status_main_text(loading: bool, not_found: bool, failed: bool) -> &'static str {
    if loading {
        STATUS_LOADING
    } else if not_found {
        STATUS_FILE_NOT_FOUND
    } else if failed {
        STATUS_LOAD_FAILED
    } else {
        ""
    }
}

/// Frame counter part: `current / total` (viv.c:11183-11209 — upstream
/// shows `position + 1`; we take the 1-based position directly). Empty for
/// static images; without a pre-known total the streaming decode counts
/// against the loaded prefix (see `loader.rs`).
pub(crate) fn status_frame_text(position_1based: usize, total: usize) -> String {
    if total <= 1 {
        String::new()
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
/// left (floor 0); the dimension part runs to the right edge (-1).
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
    [main_edge, main_edge + frame_w, -1]
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
    fn main_part_shows_loading_while_a_load_is_in_flight() {
        // Loading outranks a sticky failure from the previous open
        // (viv.c:11354-11362 order) — a replacement load of a bad file must
        // not flash the failure text while decoding.
        assert_eq!(status_main_text(true, true, true), "Loading...");
        assert_eq!(status_main_text(true, false, false), "Loading...");
    }

    #[test]
    fn main_part_prefers_not_found_over_decode_failure() {
        assert_eq!(status_main_text(false, true, true), "File not found.");
        assert_eq!(
            status_main_text(false, false, true),
            "Failed to load image."
        );
        assert_eq!(status_main_text(false, false, false), "");
    }

    #[test]
    fn frame_counter_is_one_based_and_empty_for_static_images() {
        assert_eq!(status_frame_text(1, 12), "1 / 12");
        assert_eq!(status_frame_text(12, 12), "12 / 12");
        assert_eq!(status_frame_text(1, 1), "", "static image — no counter");
        assert_eq!(status_frame_text(0, 0), "", "no image at all");
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
    fn part_edges_never_go_negative_on_a_cramped_window() {
        let edges = status_part_edges(100, 500, 500, 10, 17, 72);
        assert_eq!(edges[0], 0, "main part floored at 0 (viv.c:11308)");
        assert!(edges[1] >= edges[0], "edges stay ordered");
        assert_eq!(edges[2], -1);
    }

    #[test]
    fn an_empty_window_shows_only_the_dimension_part_at_the_edge() {
        // No image and no frame counter: everything collapses to the main
        // part plus the (empty, zero-width) slots.
        assert_eq!(status_part_edges(640, 0, 0, 10, 17, 72), [640, 640, -1]);
    }
}
