//! Bilingual (en-US / zh-CN) string tables + one-shot system-language
//! detection (upstream `localization.c`/`localization.h`).
//!
//! riviv carries ONLY the IDs it already displays — the enum grows with
//! each consuming issue (menus #23, the options dialog #24, associations +
//! installer #26 add their own entries) instead of importing upstream's
//! full 247-ID table as dead code. Upstream picks the language once at
//! startup from `GetUserDefaultUILanguage` (localization.c:55-73) — the
//! traditional-Chinese langids included, mapped onto the simplified table —
//! and has no runtime switching; riviv mirrors that: a process-wide
//! default of English, overwritten once by `init` before any UI exists
//! (unit tests never call `init`, so pure-function tests see English).
//!
//! The entries are the upstream strings verbatim
//! (localization_en_us.h/localization_zh_cn.h, cited per entry) except the
//! app name, which carries the riviv brand in BOTH languages — upstream
//! leaves "void Image Viewer" untranslated in its zh table too.

use std::sync::atomic::{AtomicU8, Ordering};

use windows::Win32::Globalization::GetUserDefaultUILanguage;

/// A displayable string. Variant order must match the table order in
/// `EN_US` and `ZH_CN` (both are `[&str; Id::COUNT]`, so a length or
/// order drift is a compile error / test failure, not a runtime hazard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Id {
    /// Window-title suffix and message-box caption base
    /// (upstream `LOCALIZATION_ID_APP_NAME`).
    AppName,
    /// Status-bar main part, a load in flight (viv.c:11358).
    StatusBarLoading,
    /// Status-bar main part, the file vanished before the open
    /// (viv.c:11364).
    StatusBarFileNotFound,
    /// Status-bar main part, decoding failed (viv.c:11370).
    StatusBarFailedToLoadImage,
    /// Ctrl+O / Ctrl+Shift+O dialog caption (viv.c:2365).
    OpenImageCaption,
    /// Ctrl+O filter label over the 9-image-extension pattern (viv.c:2363).
    OpenAllImageFiles,
    /// Ctrl+O filter label over `*.*` (viv.c:2363).
    OpenAllFiles,
}

impl Id {
    /// Variant count; array-typing both tables against this keeps them
    /// length-locked to the enum by construction.
    pub(crate) const COUNT: usize = Self::OpenAllFiles as usize + 1;
}

/// Table choice (upstream `LOCALIZATION_LANGUAGE_*`, localization.h:30-32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    English,
    ChineseSimplified,
}

/// en-US table — upstream localization_en_us.h:31/197-199/260-262.
const EN_US: [&str; Id::COUNT] = [
    "riviv",      // AppName — upstream "void Image Viewer"; the brand entry is swapped per-issue
    "Loading...", // StatusBarLoading (en_us.h:197)
    "File not found.", // StatusBarFileNotFound (en_us.h:198)
    "Failed to load image.", // StatusBarFailedToLoadImage (en_us.h:199)
    "Open Image", // OpenImageCaption (en_us.h:260)
    "All Image Files", // OpenAllImageFiles (en_us.h:261)
    "All Files",  // OpenAllFiles (en_us.h:262)
];

/// zh-CN table — upstream localization_zh_cn.h:31/197-199/261-263.
const ZH_CN: [&str; Id::COUNT] = [
    "riviv",          // AppName — upstream keeps the app name untranslated here too
    "加载中...",      // StatusBarLoading (zh_cn.h:197)
    "未找到文件。",   // StatusBarFileNotFound (zh_cn.h:198)
    "加载图片失败。", // StatusBarFailedToLoadImage (zh_cn.h:199)
    "打开图像",       // OpenImageCaption (zh_cn.h:261)
    "所有图像文件",   // OpenAllImageFiles (zh_cn.h:262)
    "所有文件",       // OpenAllFiles (zh_cn.h:263)
];

/// Map a `GetUserDefaultUILanguage` LANGID onto the table choice
/// (localization.c:64-71): both traditional-Chinese langids land on the
/// SIMPLIFIED table — upstream ships no zh-Hant strings. The neutral
/// Chinese langid (0x0004) is not in the list and stays English.
pub(crate) fn language_from_langid(langid: u16) -> Language {
    match langid {
        0x0804 | 0x0404 | 0x0C04 => Language::ChineseSimplified,
        _ => Language::English,
    }
}

/// Process-wide table choice, 0 = English (the C global's default,
/// localization.c:34) / 1 = Chinese. Set once by `init` before any UI
/// exists and never rebound; every read afterwards happens on the UI
/// thread, so Relaxed ordering is enough (a plain set-once flag, not a
/// synchronizer).
static LANGUAGE: AtomicU8 = AtomicU8::new(0);

/// Detect the system language once (upstream `localization_init`,
/// localization.c:55-73, WinMain's second call after `os_init`,
/// viv.c:5158-5159). `GetUserDefaultUILanguage` has no failure mode —
/// it falls back to en-US itself — so like upstream there is nothing to
/// check and no error path.
pub(crate) fn init() {
    // SAFETY: pure query taking no inputs; returns the user's default UI
    // language LANGID.
    let langid = unsafe { GetUserDefaultUILanguage() };
    LANGUAGE.store(language_from_langid(langid) as u8, Ordering::Relaxed);
}

/// The table choice `get` is currently reading (English until `init`).
pub(crate) fn current_language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::ChineseSimplified,
        _ => Language::English,
    }
}

fn table(lang: Language) -> &'static [&'static str; Id::COUNT] {
    match lang {
        Language::English => &EN_US,
        Language::ChineseSimplified => &ZH_CN,
    }
}

/// Localized string for `id` (upstream `localization_get_string`,
/// localization.c:36-48 — the index is enum-validated by construction, so
/// the debug Fatal on a bad id has no Rust counterpart).
pub(crate) fn get(id: Id) -> &'static str {
    get_for(current_language(), id)
}

/// Table lookup without the process global (pure; the unit tests pin the
/// exact strings through this).
pub(crate) fn get_for(lang: Language, id: Id) -> &'static str {
    table(lang)[id as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_langids_select_the_simplified_table() {
        // 0x0804 zh-CN, 0x0404 zh-TW, 0x0C04 zh-HK — upstream maps the
        // TRADITIONAL langids onto the simplified zh table as well
        // (localization.c:65-71).
        assert_eq!(language_from_langid(0x0804), Language::ChineseSimplified);
        assert_eq!(language_from_langid(0x0404), Language::ChineseSimplified);
        assert_eq!(language_from_langid(0x0C04), Language::ChineseSimplified);
    }

    #[test]
    fn every_other_langid_stays_english() {
        assert_eq!(language_from_langid(0x0409), Language::English); // en-US
        assert_eq!(language_from_langid(0x0809), Language::English); // en-GB
        assert_eq!(language_from_langid(0x0411), Language::English); // ja
        assert_eq!(language_from_langid(0x0407), Language::English); // de
        // The neutral Chinese langid is NOT in upstream's list — only the
        // three full langids qualify (localization.c:68).
        assert_eq!(language_from_langid(0x0004), Language::English);
        assert_eq!(language_from_langid(0x0000), Language::English);
    }

    #[test]
    fn every_id_has_a_non_empty_string_in_both_tables() {
        for i in 0..Id::COUNT {
            assert!(!EN_US[i].is_empty(), "en table empty at index {i}");
            assert!(!ZH_CN[i].is_empty(), "zh table empty at index {i}");
        }
    }

    #[test]
    fn tables_carry_the_upstream_strings_riviv_displays() {
        // localization_en_us.h:197-199/260-262 and localization_zh_cn.h
        // verbatim — loaded per entry in the table definitions above.
        let en = Language::English;
        assert_eq!(get_for(en, Id::StatusBarLoading), "Loading...");
        assert_eq!(get_for(en, Id::StatusBarFileNotFound), "File not found.");
        assert_eq!(
            get_for(en, Id::StatusBarFailedToLoadImage),
            "Failed to load image."
        );
        assert_eq!(get_for(en, Id::OpenImageCaption), "Open Image");
        assert_eq!(get_for(en, Id::OpenAllImageFiles), "All Image Files");
        assert_eq!(get_for(en, Id::OpenAllFiles), "All Files");

        let zh = Language::ChineseSimplified;
        assert_eq!(get_for(zh, Id::StatusBarLoading), "加载中...");
        assert_eq!(get_for(zh, Id::StatusBarFileNotFound), "未找到文件。");
        assert_eq!(
            get_for(zh, Id::StatusBarFailedToLoadImage),
            "加载图片失败。"
        );
        assert_eq!(get_for(zh, Id::OpenImageCaption), "打开图像");
        assert_eq!(get_for(zh, Id::OpenAllImageFiles), "所有图像文件");
        assert_eq!(get_for(zh, Id::OpenAllFiles), "所有文件");
    }

    #[test]
    fn app_name_is_the_untranslated_brand_in_both_tables() {
        // Upstream's zh table also carries the untranslated app name
        // ("void Image Viewer", zh_cn.h:31); riviv swaps its own brand
        // into both entries — the title never changes with the language.
        assert_eq!(get_for(Language::English, Id::AppName), "riviv");
        assert_eq!(get_for(Language::ChineseSimplified, Id::AppName), "riviv");
    }

    #[test]
    fn the_default_language_is_english_until_init_runs() {
        // Unit tests never call init(), so the process global stays at
        // its compile-time default — upstream's `localization_language`
        // starts LOCALIZATION_LANGUAGE_ENGLISH (localization.c:34).
        assert_eq!(current_language(), Language::English);
    }
}
