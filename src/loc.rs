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
    /// Menu bar captions and items (#23; upstream's `_viv_commands[]`
    /// localization ids, viv.c:798-965). Added as a block in upstream table
    /// order — File menu, View menu, Navigate menu, Help menu.
    MenuFile,
    /// "Open File..." (viv.c:802).
    MenuOpenFile,
    /// "Open Folder..." (viv.c:803).
    MenuOpenFolder,
    /// "Add File..." (viv.c:805) — the Ctrl+Shift+O append path.
    MenuAddFile,
    /// "Exit" (viv.c:821).
    MenuExit,
    /// "&View" top-level caption (viv.c:839).
    MenuView,
    /// View → "Menu" toggle (viv.c:842).
    MenuMenu,
    /// View → "Fullscreen" (viv.c:853).
    MenuFullscreen,
    /// View → "1:1" (viv.c:864).
    MenuOneToOne,
    /// View → "Best Fit" (viv.c:865).
    MenuBestFit,
    /// View → Zoom popup caption (viv.c:907).
    MenuZoom,
    /// Zoom → "Zoom In" (viv.c:908).
    MenuZoomIn,
    /// Zoom → "Zoom Out" (viv.c:909).
    MenuZoomOut,
    /// Zoom → "Reset" (viv.c:910).
    MenuZoomReset,
    /// View → "Options..." (viv.c:935) — the placeholder item the Options
    /// issue (#24) wires.
    MenuOptions,
    /// "&Navigate" top-level caption (viv.c:952).
    MenuNavigate,
    /// Navigate → "Next" (viv.c:953).
    MenuNext,
    /// Navigate → "Previous" (viv.c:954).
    MenuPrevious,
    /// Navigate → "Home" (viv.c:955).
    MenuHome,
    /// Navigate → "End" (viv.c:956).
    MenuEnd,
    /// "&Help" top-level caption (viv.c:962).
    MenuHelp,
    /// Help → "About" (viv.c:965).
    MenuAbout,
    /// Options dialog caption (#24; upstream LOCALIZATION_ID_OPTIONS_CAPTION,
    /// en_us.h:209 — the app-name half carries the riviv brand like AppName).
    OptionsCaption,
    /// Options page name: General (en_us.h:210).
    OptionsGeneral,
    /// Options page name: View (en_us.h:211).
    OptionsView,
    /// Options page name: Controls (en_us.h:212).
    OptionsControls,
    /// OK button (en_us.h:213).
    OptionsOk,
    /// Cancel button (en_us.h:214).
    OptionsCancel,
    /// General page: store-settings-in-appdata checkbox (en_us.h:215 — the
    /// %APPDATA% subpath is the riviv brand, like AppName).
    OptionsAppdata,
    /// General page: multiple instances checkbox (en_us.h:216).
    OptionsMultipleInstances,
    /// View page: shrink blit mode label (en_us.h:221).
    OptionsShrinkBlitMode,
    /// View page: magnify blit mode label (en_us.h:222).
    OptionsMagnifyBlitMode,
    /// Blit mode combo item: Nearest = COLORONCOLOR (en_us.h:223).
    OptionsBlitNearest,
    /// Blit mode combo item: Linear = HALFTONE (en_us.h:224).
    OptionsBlitLinear,
    /// View page: keep aspect ratio checkbox (en_us.h:86 — upstream ships
    /// this as a MENU string; riviv's options page reuses it).
    OptionsKeepAspectRatio,
    /// View page: fill window checkbox (en_us.h:87, menu string reused).
    OptionsFillWindow,
    /// View page: fullscreen fill checkbox — riviv-authored (upstream has
    /// no separate string: its single "Fill Window" command toggles the
    /// fullscreen half while fullscreen, viv.c:2038-2047).
    OptionsFullscreenFill,
    /// View page: auto-size window label (en_us.h:225).
    OptionsAutoZoom,
    /// Auto-size combo item: 50% (en_us.h:226).
    OptionsAutoZoom50,
    /// Auto-size combo item: 100% (en_us.h:227).
    OptionsAutoZoom100,
    /// Auto-size combo item: 200% (en_us.h:228).
    OptionsAutoZoom200,
    /// Auto-size combo item: Auto Fit (en_us.h:229).
    OptionsAutoZoomAutoFit,
    /// View page: windowed background color label (en_us.h:233).
    OptionsWindowedBg,
    /// View page: fullscreen background color label (en_us.h:234).
    OptionsFullscreenBg,
    /// View page: remaining-frames checkbox — riviv-authored (upstream has
    /// no string: the toggle is a click on the status-bar frame counter,
    /// viv.c:3994-3999).
    OptionsFrameMinus,
    /// Controls page: left click action label (en_us.h:235).
    OptionsLeftClickAction,
    /// Controls page: right click action label (en_us.h:236).
    OptionsRightClickAction,
    /// Controls page: mouse wheel action label (en_us.h:237).
    OptionsMouseWheelAction,
    /// Left-click combo item: Scroll (en_us.h:263).
    ActionScroll,
    /// Left-click combo item: Zoom In (en_us.h:266).
    ActionZoomIn,
    /// Left-click combo item: Next Image (en_us.h:267).
    ActionNextImage,
    /// Right-click combo item: Context Menu (en_us.h:270).
    ActionContextMenu,
    /// Right-click combo item: Zoom Out (en_us.h:271).
    ActionZoomOut,
    /// Right-click combo item: Previous Image (en_us.h:272).
    ActionPreviousImage,
    /// Wheel combo item: Zoom (en_us.h:273).
    ActionZoom,
    /// Wheel combo item: Next/Previous (en_us.h:274).
    ActionNextPrev,
    /// Wheel combo item: Previous/Next (en_us.h:275).
    ActionPrevNext,
}

impl Id {
    /// Variant count; array-typing both tables against this keeps them
    /// length-locked to the enum by construction.
    pub(crate) const COUNT: usize = Self::ActionPrevNext as usize + 1;
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
    // Menu block (en_us.h:34/35/36/38/52/67/70/77/88/89/97-100/115/160-164/177/182).
    "&File",           // MenuFile
    "&Open File...",   // MenuOpenFile
    "Open &Folder...", // MenuOpenFolder
    "&Add File...",    // MenuAddFile
    "E&xit",           // MenuExit
    "&View",           // MenuView
    "&Menu",           // MenuMenu
    "F&ullscreen",     // MenuFullscreen
    "1:1",             // MenuOneToOne
    "&Best Fit",       // MenuBestFit
    "&Zoom",           // MenuZoom
    "Zoom &In",        // MenuZoomIn
    "Zoom &Out",       // MenuZoomOut
    "&Reset",          // MenuZoomReset
    "&Options...",     // MenuOptions
    "&Navigate",       // MenuNavigate
    "&Next",           // MenuNext
    "P&revious",       // MenuPrevious
    "&Home",           // MenuHome
    "&End",            // MenuEnd
    "&Help",           // MenuHelp
    "&About",          // MenuAbout
    // Options block (#24; en_us.h:209-237/263-275 + the two menu strings
    // 86-87 + two riviv-authored labels).
    "Options - riviv",                     // OptionsCaption (brand swap)
    "General",                             // OptionsGeneral
    "View",                                // OptionsView
    "Controls",                            // OptionsControls
    "OK",                                  // OptionsOk
    "Cancel",                              // OptionsCancel
    "&Store settings in %APPDATA%\\riviv", // OptionsAppdata (path swap)
    "Allow multiple &instances",           // OptionsMultipleInstances
    "&Shrink blit mode:",                  // OptionsShrinkBlitMode
    "&Magnify blit mode:",                 // OptionsMagnifyBlitMode
    "Nearest",                             // OptionsBlitNearest
    "Linear",                              // OptionsBlitLinear
    "&Keep Aspect Ratio",                  // OptionsKeepAspectRatio
    "&Fill Window",                        // OptionsFillWindow
    "Fill Window (Fullscreen)",            // OptionsFullscreenFill (riviv)
    "Auto si&ze window:",                  // OptionsAutoZoom
    "50%",                                 // OptionsAutoZoom50
    "100%",                                // OptionsAutoZoom100
    "200%",                                // OptionsAutoZoom200
    "Auto Fit",                            // OptionsAutoZoomAutoFit
    "&Windowed background color:",         // OptionsWindowedBg
    "&Fullscreen background color:",       // OptionsFullscreenBg
    "Show &remaining frames",              // OptionsFrameMinus (riviv)
    "&Left click action:",                 // OptionsLeftClickAction
    "&Right click action:",                // OptionsRightClickAction
    "&Mouse wheel action:",                // OptionsMouseWheelAction
    "Scroll",                              // ActionScroll
    "Zoom In",                             // ActionZoomIn
    "Next Image",                          // ActionNextImage
    "Context Menu",                        // ActionContextMenu
    "Zoom Out",                            // ActionZoomOut
    "Previous Image",                      // ActionPreviousImage
    "Zoom",                                // ActionZoom
    "Next/Previous",                       // ActionNextPrev
    "Previous/Next",                       // ActionPrevNext
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
    // Menu block (zh_cn.h:34/35/36/38/52/67/70/77/88/89/97-100/115/160-164/177/182).
    "文件(&F)",          // MenuFile
    "打开文件(&O)...",   // MenuOpenFile
    "打开文件夹(&F)...", // MenuOpenFolder
    "添加文件(&A)...",   // MenuAddFile
    "退出(&X)",          // MenuExit
    "视图(&V)",          // MenuView
    "菜单(&M)",          // MenuMenu
    "全屏(&F)",          // MenuFullscreen
    "1:1",               // MenuOneToOne
    "最佳适应(&B)",      // MenuBestFit
    "缩放(&Z)",          // MenuZoom
    "放大(&I)",          // MenuZoomIn
    "缩小(&O)",          // MenuZoomOut
    "重置(&R)",          // MenuZoomReset
    "选项(&O)...",       // MenuOptions
    "导航(&N)",          // MenuNavigate
    "下一个(&N)",        // MenuNext
    "上一个(&P)",        // MenuPrevious
    "首页(&H)",          // MenuHome
    "末页(&E)",          // MenuEnd
    "帮助(&H)",          // MenuHelp
    "关于(&A)",          // MenuAbout
    // Options block (#24; zh_cn.h:209-238/264-276 + 86-87 + two riviv labels).
    "选项 - riviv",                       // OptionsCaption (brand swap)
    "常规",                               // OptionsGeneral
    "视图",                               // OptionsView
    "控件",                               // OptionsControls
    "确定",                               // OptionsOk
    "取消",                               // OptionsCancel
    "在 %APPDATA%\\riviv 中存储设置(&S)", // OptionsAppdata (path swap)
    "允许多个实例(&I)",                   // OptionsMultipleInstances
    "缩小位图模式(&S):",                  // OptionsShrinkBlitMode
    "放大位图模式(&M):",                  // OptionsMagnifyBlitMode
    "最近邻",                             // OptionsBlitNearest
    "线性",                               // OptionsBlitLinear
    "保持纵横比(&K)",                     // OptionsKeepAspectRatio
    "填充窗口(&F)",                       // OptionsFillWindow
    "全屏时填充窗口",                     // OptionsFullscreenFill (riviv)
    "自动调窗(&Z):",                      // OptionsAutoZoom
    "50%",                                // OptionsAutoZoom50
    "100%",                               // OptionsAutoZoom100
    "200%",                               // OptionsAutoZoom200
    "自动适应",                           // OptionsAutoZoomAutoFit
    "窗口背景颜色(&W):",                  // OptionsWindowedBg
    "全屏背景颜色(&F):",                  // OptionsFullscreenBg
    "显示剩余帧(&R)",                     // OptionsFrameMinus (riviv)
    "左键操作(&L):",                      // OptionsLeftClickAction
    "右键操作(&R):",                      // OptionsRightClickAction
    "鼠标滚轮操作(&M):",                  // OptionsMouseWheelAction
    "滚动",                               // ActionScroll
    "放大",                               // ActionZoomIn
    "下一张图像",                         // ActionNextImage
    "上下文菜单",                         // ActionContextMenu
    "缩小",                               // ActionZoomOut
    "上一张图像",                         // ActionPreviousImage
    "缩放",                               // ActionZoom
    "下一张/上一张",                      // ActionNextPrev
    "上一张/下一张",                      // ActionPrevNext
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
    fn menu_strings_carry_the_upstream_texts_in_both_languages() {
        // The #23 menu block — localization_en_us.h /
        // localization_zh_cn.h verbatim per entry in the table definitions.
        let en = Language::English;
        let zh = Language::ChineseSimplified;
        assert_eq!(get_for(en, Id::MenuFile), "&File");
        assert_eq!(get_for(en, Id::MenuOpenFile), "&Open File...");
        assert_eq!(get_for(en, Id::MenuOpenFolder), "Open &Folder...");
        assert_eq!(get_for(en, Id::MenuAddFile), "&Add File...");
        assert_eq!(get_for(en, Id::MenuExit), "E&xit");
        assert_eq!(get_for(en, Id::MenuView), "&View");
        assert_eq!(get_for(en, Id::MenuMenu), "&Menu");
        assert_eq!(get_for(en, Id::MenuFullscreen), "F&ullscreen");
        assert_eq!(get_for(en, Id::MenuOneToOne), "1:1");
        assert_eq!(get_for(en, Id::MenuBestFit), "&Best Fit");
        assert_eq!(get_for(en, Id::MenuZoom), "&Zoom");
        assert_eq!(get_for(en, Id::MenuZoomIn), "Zoom &In");
        assert_eq!(get_for(en, Id::MenuZoomOut), "Zoom &Out");
        assert_eq!(get_for(en, Id::MenuZoomReset), "&Reset");
        assert_eq!(get_for(en, Id::MenuOptions), "&Options...");
        assert_eq!(get_for(en, Id::MenuNavigate), "&Navigate");
        assert_eq!(get_for(en, Id::MenuNext), "&Next");
        assert_eq!(get_for(en, Id::MenuPrevious), "P&revious");
        assert_eq!(get_for(en, Id::MenuHome), "&Home");
        assert_eq!(get_for(en, Id::MenuEnd), "&End");
        assert_eq!(get_for(en, Id::MenuHelp), "&Help");
        assert_eq!(get_for(en, Id::MenuAbout), "&About");

        assert_eq!(get_for(zh, Id::MenuFile), "文件(&F)");
        assert_eq!(get_for(zh, Id::MenuOpenFile), "打开文件(&O)...");
        assert_eq!(get_for(zh, Id::MenuOpenFolder), "打开文件夹(&F)...");
        assert_eq!(get_for(zh, Id::MenuAddFile), "添加文件(&A)...");
        assert_eq!(get_for(zh, Id::MenuExit), "退出(&X)");
        assert_eq!(get_for(zh, Id::MenuView), "视图(&V)");
        assert_eq!(get_for(zh, Id::MenuMenu), "菜单(&M)");
        assert_eq!(get_for(zh, Id::MenuFullscreen), "全屏(&F)");
        assert_eq!(get_for(zh, Id::MenuOneToOne), "1:1");
        assert_eq!(get_for(zh, Id::MenuBestFit), "最佳适应(&B)");
        assert_eq!(get_for(zh, Id::MenuZoom), "缩放(&Z)");
        assert_eq!(get_for(zh, Id::MenuZoomIn), "放大(&I)");
        assert_eq!(get_for(zh, Id::MenuZoomOut), "缩小(&O)");
        assert_eq!(get_for(zh, Id::MenuZoomReset), "重置(&R)");
        assert_eq!(get_for(zh, Id::MenuOptions), "选项(&O)...");
        assert_eq!(get_for(zh, Id::MenuNavigate), "导航(&N)");
        assert_eq!(get_for(zh, Id::MenuNext), "下一个(&N)");
        assert_eq!(get_for(zh, Id::MenuPrevious), "上一个(&P)");
        assert_eq!(get_for(zh, Id::MenuHome), "首页(&H)");
        assert_eq!(get_for(zh, Id::MenuEnd), "末页(&E)");
        assert_eq!(get_for(zh, Id::MenuHelp), "帮助(&H)");
        assert_eq!(get_for(zh, Id::MenuAbout), "关于(&A)");
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
    fn options_strings_carry_the_upstream_texts_in_both_languages() {
        // The #24 options block — localization_en_us.h:209-237/263-275 +
        // the two reused menu strings (86-87); the caption/appdata path
        // carry the riviv brand, and OptionsFullscreenFill /
        // OptionsFrameMinus are riviv-authored (no upstream string).
        let en = Language::English;
        assert_eq!(get_for(en, Id::OptionsCaption), "Options - riviv");
        assert_eq!(get_for(en, Id::OptionsGeneral), "General");
        assert_eq!(get_for(en, Id::OptionsView), "View");
        assert_eq!(get_for(en, Id::OptionsControls), "Controls");
        assert_eq!(get_for(en, Id::OptionsOk), "OK");
        assert_eq!(get_for(en, Id::OptionsCancel), "Cancel");
        assert_eq!(
            get_for(en, Id::OptionsAppdata),
            "&Store settings in %APPDATA%\\riviv"
        );
        assert_eq!(
            get_for(en, Id::OptionsMultipleInstances),
            "Allow multiple &instances"
        );
        assert_eq!(get_for(en, Id::OptionsShrinkBlitMode), "&Shrink blit mode:");
        assert_eq!(
            get_for(en, Id::OptionsMagnifyBlitMode),
            "&Magnify blit mode:"
        );
        assert_eq!(get_for(en, Id::OptionsBlitNearest), "Nearest");
        assert_eq!(get_for(en, Id::OptionsBlitLinear), "Linear");
        assert_eq!(
            get_for(en, Id::OptionsKeepAspectRatio),
            "&Keep Aspect Ratio"
        );
        assert_eq!(get_for(en, Id::OptionsFillWindow), "&Fill Window");
        assert_eq!(get_for(en, Id::OptionsAutoZoom), "Auto si&ze window:");
        assert_eq!(get_for(en, Id::OptionsAutoZoom50), "50%");
        assert_eq!(get_for(en, Id::OptionsAutoZoom100), "100%");
        assert_eq!(get_for(en, Id::OptionsAutoZoom200), "200%");
        assert_eq!(get_for(en, Id::OptionsAutoZoomAutoFit), "Auto Fit");
        assert_eq!(
            get_for(en, Id::OptionsWindowedBg),
            "&Windowed background color:"
        );
        assert_eq!(
            get_for(en, Id::OptionsFullscreenBg),
            "&Fullscreen background color:"
        );
        assert_eq!(
            get_for(en, Id::OptionsLeftClickAction),
            "&Left click action:"
        );
        assert_eq!(
            get_for(en, Id::OptionsRightClickAction),
            "&Right click action:"
        );
        assert_eq!(
            get_for(en, Id::OptionsMouseWheelAction),
            "&Mouse wheel action:"
        );
        assert_eq!(get_for(en, Id::ActionScroll), "Scroll");
        assert_eq!(get_for(en, Id::ActionZoomIn), "Zoom In");
        assert_eq!(get_for(en, Id::ActionNextImage), "Next Image");
        assert_eq!(get_for(en, Id::ActionContextMenu), "Context Menu");
        assert_eq!(get_for(en, Id::ActionZoomOut), "Zoom Out");
        assert_eq!(get_for(en, Id::ActionPreviousImage), "Previous Image");
        assert_eq!(get_for(en, Id::ActionZoom), "Zoom");
        assert_eq!(get_for(en, Id::ActionNextPrev), "Next/Previous");
        assert_eq!(get_for(en, Id::ActionPrevNext), "Previous/Next");

        let zh = Language::ChineseSimplified;
        assert_eq!(get_for(zh, Id::OptionsCaption), "选项 - riviv");
        assert_eq!(get_for(zh, Id::OptionsGeneral), "常规");
        assert_eq!(get_for(zh, Id::OptionsView), "视图");
        assert_eq!(get_for(zh, Id::OptionsControls), "控件");
        assert_eq!(get_for(zh, Id::OptionsOk), "确定");
        assert_eq!(get_for(zh, Id::OptionsCancel), "取消");
        assert_eq!(
            get_for(zh, Id::OptionsAppdata),
            "在 %APPDATA%\\riviv 中存储设置(&S)"
        );
        assert_eq!(
            get_for(zh, Id::OptionsMultipleInstances),
            "允许多个实例(&I)"
        );
        assert_eq!(get_for(zh, Id::OptionsShrinkBlitMode), "缩小位图模式(&S):");
        assert_eq!(get_for(zh, Id::OptionsMagnifyBlitMode), "放大位图模式(&M):");
        assert_eq!(get_for(zh, Id::OptionsBlitNearest), "最近邻");
        assert_eq!(get_for(zh, Id::OptionsBlitLinear), "线性");
        assert_eq!(get_for(zh, Id::OptionsKeepAspectRatio), "保持纵横比(&K)");
        assert_eq!(get_for(zh, Id::OptionsFillWindow), "填充窗口(&F)");
        assert_eq!(get_for(zh, Id::OptionsAutoZoom), "自动调窗(&Z):");
        assert_eq!(get_for(zh, Id::OptionsAutoZoomAutoFit), "自动适应");
        assert_eq!(get_for(zh, Id::OptionsWindowedBg), "窗口背景颜色(&W):");
        assert_eq!(get_for(zh, Id::OptionsFullscreenBg), "全屏背景颜色(&F):");
        assert_eq!(get_for(zh, Id::OptionsLeftClickAction), "左键操作(&L):");
        assert_eq!(get_for(zh, Id::OptionsRightClickAction), "右键操作(&R):");
        assert_eq!(
            get_for(zh, Id::OptionsMouseWheelAction),
            "鼠标滚轮操作(&M):"
        );
        assert_eq!(get_for(zh, Id::ActionScroll), "滚动");
        assert_eq!(get_for(zh, Id::ActionZoomIn), "放大");
        assert_eq!(get_for(zh, Id::ActionNextImage), "下一张图像");
        assert_eq!(get_for(zh, Id::ActionContextMenu), "上下文菜单");
        assert_eq!(get_for(zh, Id::ActionZoomOut), "缩小");
        assert_eq!(get_for(zh, Id::ActionPreviousImage), "上一张图像");
        assert_eq!(get_for(zh, Id::ActionZoom), "缩放");
        assert_eq!(get_for(zh, Id::ActionNextPrev), "下一张/上一张");
        assert_eq!(get_for(zh, Id::ActionPrevNext), "上一张/下一张");
    }

    #[test]
    fn the_default_language_is_english_until_init_runs() {
        // Unit tests never call init(), so the process global stays at
        // its compile-time default — upstream's `localization_language`
        // starts LOCALIZATION_LANGUAGE_ENGLISH (localization.c:34).
        assert_eq!(current_language(), Language::English);
    }
}
