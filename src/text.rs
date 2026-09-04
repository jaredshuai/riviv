//! Wide-string text construction for the Win32 layer (pure logic, unit-tested).
//!
//! M2 seam: status-bar text composition — "Loading...", "Failed to load
//! image.", frame counters, dimension parts (#5) — lands here beside the
//! title construction.

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
}
