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
    /// "&Edit" top-level caption (#41; viv.c:824).
    MenuEdit,
    /// Edit → "Cu&t" (#41; viv.c:826).
    MenuCut,
    /// Edit → "&Copy" (#41; viv.c:827).
    MenuCopy,
    /// Edit → "Copy Filename" (#41; viv.c:828 — MF_OWNERDRAW upstream,
    /// keyboard Ctrl+Shift+C).
    MenuCopyFilename,
    /// Edit → "Cop&y Image" (#41; viv.c:829).
    MenuCopyImage,
    /// Edit → "&Paste" (#41; viv.c:830 — MF_OWNERDRAW upstream, keyboard
    /// Ctrl+V).
    MenuPaste,
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
    /// View → Pan/Scan popup caption (#44; viv.c:870).
    MenuPanScan,
    /// Pan/Scan → "Increase Size" (#44; viv.c:871).
    MenuPanScanIncreaseSize,
    /// Pan/Scan → "Decrease Size" (#44; viv.c:872).
    MenuPanScanDecreaseSize,
    /// Pan/Scan → "Increase Width" (#44; viv.c:873).
    MenuPanScanIncreaseWidth,
    /// Pan/Scan → "Decrease Width" (#44; viv.c:874).
    MenuPanScanDecreaseWidth,
    /// Pan/Scan → "Increase Height" (#44; viv.c:875).
    MenuPanScanIncreaseHeight,
    /// Pan/Scan → "Decrease Height" (#44; viv.c:876).
    MenuPanScanDecreaseHeight,
    /// Pan/Scan → "Move Up" (#44; viv.c:890).
    MenuPanScanMoveUp,
    /// Pan/Scan → "Move Down" (#44; viv.c:891).
    MenuPanScanMoveDown,
    /// Pan/Scan → "Move Left" (#44; viv.c:892).
    MenuPanScanMoveLeft,
    /// Pan/Scan → "Move Right" (#44; viv.c:893).
    MenuPanScanMoveRight,
    /// Pan/Scan → "Move Up Left" (#44; viv.c:881 — MF_OWNERDRAW
    /// upstream, keyboard-only like the #38 jump family).
    MenuPanScanMoveUpLeft,
    /// Pan/Scan → "Move Up Right" (#44; viv.c:882 — MF_OWNERDRAW).
    MenuPanScanMoveUpRight,
    /// Pan/Scan → "Move Down Left" (#44; viv.c:883 — MF_OWNERDRAW).
    MenuPanScanMoveDownLeft,
    /// Pan/Scan → "Move Down Right" (#44; viv.c:884 — MF_OWNERDRAW).
    MenuPanScanMoveDownRight,
    /// Pan/Scan → "Move Center" (#44; viv.c:894).
    MenuPanScanMoveCenter,
    /// Pan/Scan → "Reset" (#44; viv.c:896).
    MenuPanScanReset,
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
    /// General page: start-menu shortcuts checkbox (en_us.h:217).
    OptionsStartMenu,
    /// General page: associations group box caption (en_us.h:218).
    OptionsAssociations,
    /// General page: Check All button (en_us.h:219).
    OptionsCheckAll,
    /// General page: Check None button (en_us.h:220).
    OptionsCheckNone,
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
    /// Controls page: X button (mouse back/forward) action label — a
    /// riviv addition (#44): upstream exposes `xbutton_action` in the ini
    /// only, with no Options row.
    OptionsXButtonAction,
    /// Controls page: the commands list caption (en_us.h:238).
    OptionsCommands,
    /// Controls page: the key-list group caption (en_us.h:239).
    OptionsSettingsForSelected,
    /// Controls page: the Add-key button (en_us.h:240).
    OptionsAddKey,
    /// Controls page: the Edit-key button (en_us.h:241).
    OptionsEditKey,
    /// Controls page: the Remove-key button (en_us.h:242).
    OptionsRemoveKey,
    /// Edit-key dialog caption, Add flavor (en_us.h:243).
    AddKeyCaption,
    /// Edit-key dialog caption, Edit flavor (en_us.h:244).
    EditKeyCaption,
    /// Edit-key dialog: the shortcut label (en_us.h:245).
    OptionsShortcutKey,
    /// Edit-key dialog: the currently-used-by label (en_us.h:246).
    OptionsShortcutKeyUsedBy,
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
    /// File → "Open Everything &Search..." (#22; viv.c:804, en_us.h:37 —
    /// upstream MF_OWNERDRAW-hides the row; riviv shows it like Add File).
    MenuOpenEverythingSearch,
    /// File → "Add Everything Search..." (viv.c:807, en_us.h:40 — no
    /// accelerator in either language).
    MenuAddEverythingSearch,
    /// Search dialog caption, Open flavor (#22; en_us.h:282).
    EverythingLoadCaption,
    /// Search dialog caption, Add flavor (#22; en_us.h:281).
    EverythingAddCaption,
    /// Search dialog's Randomize checkbox (#22; en_us.h:283).
    EverythingRandomize,
    /// "Everything not available" error box (#22; en_us.h:280).
    EverythingNotAvailable,
    /// View → "Slideshow" (#37; en_us.h:78).
    MenuSlideshow,
    /// "&Slideshow" top-level caption (#37; en_us.h:118).
    MenuSlideshowMenu,
    /// Slideshow → "&Play/Pause" (#37; en_us.h:119).
    MenuSlideshowPlayPause,
    /// Slideshow → Rate popup caption (#37; en_us.h:120).
    MenuSlideshowRate,
    /// Slideshow → Rate → "&Decrease Rate" (#37; en_us.h:121).
    MenuSlideshowRateDecrease,
    /// Slideshow → Rate → "&Increase Rate" (#37; en_us.h:122).
    MenuSlideshowRateIncrease,
    /// The 17 preset rows + Custom (#37; en_us.h:123-140).
    MenuRate250Milliseconds,
    MenuRate500Milliseconds,
    MenuRate1Second,
    MenuRate2Seconds,
    MenuRate3Seconds,
    MenuRate4Seconds,
    MenuRate5Seconds,
    MenuRate6Seconds,
    MenuRate7Seconds,
    MenuRate8Seconds,
    MenuRate9Seconds,
    MenuRate10Seconds,
    MenuRate20Seconds,
    MenuRate30Seconds,
    MenuRate40Seconds,
    MenuRate50Seconds,
    MenuRate1Minute,
    MenuRateCustom,
    /// Set Custom Rate dialog caption (#37; en_us.h:248).
    CustomRateCaption,
    /// "&Custom rate:" label (#37; en_us.h:249).
    CustomRateLabel,
    /// The dialog's unit combo rows (#37; en_us.h:250-252 — milliseconds,
    /// seconds, minutes in type order 0/1/2).
    CustomRateMilliseconds,
    CustomRateSeconds,
    CustomRateMinutes,
    /// Status-bar main part while a slideshow runs (#37; en_us.h:200).
    StatusBarSlideshowPlaying,
    /// Temp-text readout words (#47; upstream keeps one printf template
    /// per flash — en_us.h:201-206/zh_cn.h:201-206. riviv composes the
    /// labels around the same number slots (byte-identical output).
    /// "Pos" — the panscan position flash's first word.
    StatusBarPosLabel,
    /// "Zoom" — the panscan position flash's second word.
    StatusBarZoomLabel,
    /// "Aspect Ratio" — the panscan position flash's third word.
    StatusBarAspectLabel,
    /// "Animation rate" — the animation-rate flash.
    StatusBarAnimationRateLabel,
    /// "Slideshow rate" — the slideshow-rate flash.
    StatusBarSlideshowRateLabel,
    /// Slideshow-rate unit, whole minutes (en_us.h:204).
    StatusBarMinutes,
    /// Slideshow-rate unit, whole seconds (en_us.h:205).
    StatusBarSeconds,
    /// Slideshow-rate unit, raw milliseconds (en_us.h:206).
    StatusBarMilliseconds,
    /// Left-click action value 1 (#37; en_us.h:264).
    ActionPlayPauseSlideshow,
    /// Left-click action value 2 (#38; en_us.h:265).
    ActionPlayPauseAnimation,
    /// "&Animation" top-level caption (#38; en_us.h:142).
    MenuAnimation,
    /// Animation → "&Play/Pause" (#38; en_us.h:144).
    MenuAnimationPlayPause,
    /// Animation → "Jump &Forward" / "Jump &Backward" (medium, #38;
    /// en_us.h:145-146).
    MenuAnimationJumpForward,
    MenuAnimationJumpBackward,
    /// The MF_OWNERDRAW short/long jump quartet (#38; en_us.h:147-150 —
    /// never in the menu bar, named for the Controls list + ini).
    MenuAnimationShortJumpForward,
    MenuAnimationShortJumpBackward,
    MenuAnimationLongJumpForward,
    MenuAnimationLongJumpBackward,
    /// Animation → "F&rame Step" / "Pre&vious Frame" (#38; en_us.h:151-152).
    MenuAnimationFrameStep,
    MenuAnimationPreviousFrame,
    /// Animation → "F&irst Frame" / "&Last Frame" (#38; en_us.h:153-154).
    MenuAnimationFirstFrame,
    MenuAnimationLastFrame,
    /// Animation → "&Decrease Rate" / "&Increase Rate" / "R&eset Rate"
    /// (#38; en_us.h:155-157).
    MenuAnimationRateDecrease,
    MenuAnimationRateIncrease,
    MenuAnimationRateReset,
    /// Options View page: loop-once checkbox (#38; en_us.h:230 — the
    /// slideshow-scoped "advance waits for one full animation pass"
    /// setting).
    OptionsLoopAnimationsOnce,
    /// ---- #39: Navigate sort / shuffle / Jump To ----
    /// Navigate (Sort) popup caption "&Sort" (en_us.h:165).
    MenuSort,
    /// Sort (5 radio rows, en_us.h:166-170).
    MenuSortName,
    MenuSortFullPath,
    MenuSortSize,
    MenuSortDateModified,
    MenuSortDateCreated,
    /// Sort direction radio pair (en_us.h:171-172).
    MenuSortAscending,
    MenuSortDescending,
    /// Navigate "Shuffle" (en_us.h:173).
    MenuShuffle,
    /// Navigate "&Jump To..." (en_us.h:174).
    MenuJumpTo,
    /// Jump To dialog caption "Jump To" (en_us.h:255).
    JumpToCaption,
    /// ---- #40: preload + last-image cache ----
    /// Status-bar PRELOAD part while a preload decodes its first frame
    /// (en_us.h:196 / zh_cn.h:196).
    StatusBarPreload,
    /// Options View page: preload checkbox (en_us.h:231 / zh_cn.h:232).
    OptionsPreloadNext,
    /// Options View page: last-cache checkbox (en_us.h:232 / zh_cn.h:233).
    OptionsCacheLast,
    /// Options View page: title-bar-format row (#47; en_us.h:276-279 /
    /// zh_cn.h:277-280). The static label.
    OptionsTitleBarFormat,
    /// Combo item: full path (value 0).
    OptionsTitleBarFormatFullPath,
    /// Combo item: filename only (value 1).
    OptionsTitleBarFormatFilenameOnly,
    /// Combo item: none (value 2).
    OptionsTitleBarFormatNone,
    /// ---- #48: second-pass CLI ----
    /// Help → "Command Line Options" (en_us.h:179 / zh_cn.h:179).
    MenuCommandLineOptions,
    /// The usage box body (`_viv_command_line_options`, viv.c:11862-11894 —
    /// upstream hardcodes the English text; riviv localizes it per the
    /// issue, with the exe name brand-swapped like AppName).
    UsageText,
    /// ---- #45: toolbar ----
    /// View → "Controls" toggle (en_us.h:72 / zh_cn.h:72).
    MenuControls,
    /// Toolbar button labels (en_us.h:188-193 / zh_cn.h:188-193).
    ToolbarPreviousImage,
    ToolbarNextImage,
    ToolbarPlaySlideshow,
    ToolbarPauseSlideshow,
    ToolbarBestFit,
    ToolbarActualSize,
    /// ---- #46: view presets / window size / on-top / view toggles ----
    /// View → "Caption" toggle (en_us.h:68 / zh_cn.h:68).
    MenuCaption,
    /// View → "Frame" toggle (en_us.h:69 / zh_cn.h:69).
    MenuThickFrame,
    /// View → "Status &Bar" (en_us.h:71 / zh_cn.h:71).
    MenuStatusBar,
    /// View → "&Preset" popup (en_us.h:73 / zh_cn.h:73).
    MenuPreset,
    /// Preset → "&Minimal" (en_us.h:74 / zh_cn.h:74).
    MenuMinimal,
    /// Preset → "&Compact" (en_us.h:75 / zh_cn.h:75).
    MenuCompact,
    /// Preset → "&Normal" (en_us.h:76 / zh_cn.h:76).
    MenuNormal,
    /// View → "&Window Size" popup (en_us.h:79 / zh_cn.h:79).
    MenuWindowSize,
    /// Window Size → "50%" (en_us.h:80 / zh_cn.h:80).
    MenuWindowSize50,
    /// Window Size → "100%" (en_us.h:81 / zh_cn.h:81).
    MenuWindowSize100,
    /// Window Size → "200%" (en_us.h:82 / zh_cn.h:82).
    MenuWindowSize200,
    /// Window Size → "&Auto Fit" (en_us.h:83 / zh_cn.h:83).
    MenuWindowSizeAutoFit,
    /// View → "&Refresh" (en_us.h:84 / zh_cn.h:84).
    MenuRefresh,
    /// View → "&Allow Shrinking" (en_us.h:85 / zh_cn.h:85).
    MenuAllowShrinking,
    /// View → "&Keep Aspect Ratio" (en_us.h:86 / zh_cn.h:86).
    MenuKeepAspectRatio,
    /// View → "&Fill Window" (en_us.h:87 / zh_cn.h:87).
    MenuFillWindow,
    /// View → "On &Top" popup (en_us.h:111 / zh_cn.h:111).
    MenuOnTop,
    /// On Top → "&Always" (en_us.h:112 / zh_cn.h:112).
    MenuAlways,
    /// On Top → "&While Playing Slideshow or Animating" (en_us.h:113 /
    /// zh_cn.h:113).
    MenuWhilePlaying,
    /// On Top → "&Never" (en_us.h:114 / zh_cn.h:114).
    MenuNever,
    /// File → "Open File &Location..." (#42; viv.c:811, en_us.h:41).
    MenuOpenFileLocation,
    /// File → "&Edit..." (#42; viv.c:812, en_us.h:42 — the File-menu
    /// shell row, a DIFFERENT upstream id from the top-level Edit popup).
    MenuFileEdit,
    /// File → "Pre&view..." (#42; viv.c:813, en_us.h:43).
    MenuPreview,
    /// File → "&Print..." (#42; viv.c:814, en_us.h:44).
    MenuPrint,
    /// File → "Set Des&ktop Wallpaper" (#42; viv.c:815, en_us.h:45).
    MenuSetDesktopWallpaper,
    /// File → "&Close" (#42; viv.c:816, en_us.h:46 — MF_OWNERDRAW
    /// upstream, keyboard Ctrl+W only).
    MenuClose,
    /// File → "P&roperties" (#42; viv.c:820, en_us.h:51).
    MenuProperties,
    /// File → "&Delete" (#43; viv.c:817, en_us.h:47 — the visible row that
    /// live-probes Shift at dispatch time).
    MenuDelete,
    /// File → "Delete (Recycle)" (#43; viv.c:818, en_us.h:48 — MF_OWNERDRAW
    /// upstream, keyboard Del).
    MenuDeleteRecycle,
    /// File → "Delete (Permanently)" (#43; viv.c:819, en_us.h:49 —
    /// MF_OWNERDRAW upstream, keyboard Shift+Del).
    MenuDeletePermanently,
    /// File → "Rena&me" (#43; viv.c:821, en_us.h:50, F2).
    MenuRename,
    /// Edit → "Rotate Cloc&kwise" (#43; viv.c:833, en_us.h:61).
    MenuRotateClockwise,
    /// Edit → "Rotate Cou&nterclockwise" (#43; viv.c:834, en_us.h:62).
    MenuRotateCounterclockwise,
    /// Edit → "Copy to &Folder..." (#43; viv.c:836, en_us.h:63).
    MenuCopyTo,
    /// Edit → "Mo&ve to Folder..." (#43; viv.c:837, en_us.h:64).
    MenuMoveTo,
    /// The rename dialog caption (#43; en_us.h:254, "Rename").
    RenameCaption,
    /// The Copy To save-dialog title (#43; en_us.h:284, "Copy To").
    CopyToCaption,
    /// The Move To save-dialog title (#43; en_us.h:285, "Move To").
    MoveToCaption,
}

impl Id {
    /// Variant count; array-typing both tables against this keeps them
    /// length-locked to the enum by construction.
    pub(crate) const COUNT: usize = Self::MoveToCaption as usize + 1;
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
    "&File",             // MenuFile
    "&Open File...",     // MenuOpenFile
    "Open &Folder...",   // MenuOpenFolder
    "&Add File...",      // MenuAddFile
    "E&xit",             // MenuExit
    "&Edit",             // MenuEdit (#41)
    "Cu&t",              // MenuCut
    "&Copy",             // MenuCopy
    "Copy Filename",     // MenuCopyFilename — no mnemonic upstream
    "Cop&y Image",       // MenuCopyImage
    "&Paste",            // MenuPaste
    "&View",             // MenuView
    "&Menu",             // MenuMenu
    "F&ullscreen",       // MenuFullscreen
    "1:1",               // MenuOneToOne
    "&Best Fit",         // MenuBestFit
    "Pa&n && Scan",      // MenuPanScan (#44; en_us.h:90)
    "&Increase Size",    // MenuPanScanIncreaseSize (en_us.h:91)
    "&Decrease Size",    // MenuPanScanDecreaseSize (en_us.h:92)
    "I&ncrease Width",   // MenuPanScanIncreaseWidth (en_us.h:93)
    "D&ecrease Width",   // MenuPanScanDecreaseWidth (en_us.h:94)
    "In&crease Height",  // MenuPanScanIncreaseHeight (en_us.h:95)
    "De&cre&ase Height", // MenuPanScanDecreaseHeight (en_us.h:96)
    "Move &Up",          // MenuPanScanMoveUp (en_us.h:101)
    "Move &Down",        // MenuPanScanMoveDown (en_us.h:102)
    "Move &Left",        // MenuPanScanMoveLeft (en_us.h:103)
    "Move &Right",       // MenuPanScanMoveRight (en_us.h:104)
    "Move Up Left",      // MenuPanScanMoveUpLeft (en_us.h:105)
    "Move Up Right",     // MenuPanScanMoveUpRight (en_us.h:106)
    "Move Down Left",    // MenuPanScanMoveDownLeft (en_us.h:107)
    "Move Down Right",   // MenuPanScanMoveDownRight (en_us.h:108)
    "Move Cen&ter",      // MenuPanScanMoveCenter (en_us.h:109)
    "Re&set",            // MenuPanScanReset (en_us.h:110)
    "&Zoom",             // MenuZoom
    "Zoom &In",          // MenuZoomIn
    "Zoom &Out",         // MenuZoomOut
    "&Reset",            // MenuZoomReset
    "&Options...",       // MenuOptions
    "&Navigate",         // MenuNavigate
    "&Next",             // MenuNext
    "P&revious",         // MenuPrevious
    "&Home",             // MenuHome
    "&End",              // MenuEnd
    "&Help",             // MenuHelp
    "&About",            // MenuAbout
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
    "Start &menu shortcuts",               // OptionsStartMenu
    "Associations",                        // OptionsAssociations
    "Check &All",                          // OptionsCheckAll
    "Check &None",                         // OptionsCheckNone
    "&Shrink blit mode:",                  // OptionsShrinkBlitMode
    "&Magnify blit mode:",                 // OptionsMagnifyBlitMode
    "Nearest",                             // OptionsBlitNearest
    "Linear",                              // OptionsBlitLinear
    "&Keep Aspect Ratio",                  // OptionsKeepAspectRatio
    "Fill Window", // OptionsFillWindow — upstream menu string minus its &F: the upstream OPTIONS page has no fill checkbox, and riviv's View page already carries "&Fullscreen background color:" — two &F on one dialog collide
    "Fill Window (Fullscreen)", // OptionsFullscreenFill (riviv)
    "Auto si&ze window:", // OptionsAutoZoom
    "50%",         // OptionsAutoZoom50
    "100%",        // OptionsAutoZoom100
    "200%",        // OptionsAutoZoom200
    "Auto Fit",    // OptionsAutoZoomAutoFit
    "&Windowed background color:", // OptionsWindowedBg
    "&Fullscreen background color:", // OptionsFullscreenBg
    "Show &remaining frames", // OptionsFrameMinus (riviv)
    "&Left click action:", // OptionsLeftClickAction
    "&Right click action:", // OptionsRightClickAction
    "&Mouse wheel action:", // OptionsMouseWheelAction
    "X &button action:", // OptionsXButtonAction — riviv addition (#44)
    "&Commands:",  // OptionsCommands
    "Settings for selected command", // OptionsSettingsForSelected
    "&Add...",     // OptionsAddKey
    "&Edit...",    // OptionsEditKey
    "Remo&ve",     // OptionsRemoveKey
    "Add Keyboard Shortcut", // AddKeyCaption
    "Edit Keyboard Shortcut", // EditKeyCaption
    "Shortcut &key:", // OptionsShortcutKey
    "Shortcut key currently used by:", // OptionsShortcutKeyUsedBy
    "Scroll",      // ActionScroll
    "Zoom In",     // ActionZoomIn
    "Next Image",  // ActionNextImage
    "Context Menu", // ActionContextMenu
    "Zoom Out",    // ActionZoomOut
    "Previous Image", // ActionPreviousImage
    "Zoom",        // ActionZoom
    "Next/Previous", // ActionNextPrev
    "Previous/Next", // ActionPrevNext
    // Everything block (#22; en_us.h:37/40/280-283).
    "Open Everything &Search...", // MenuOpenEverythingSearch
    "Add Everything Search...",   // MenuAddEverythingSearch
    "Load Everything Search",     // EverythingLoadCaption
    "Add Everything Search",      // EverythingAddCaption
    "Randomize",                  // EverythingRandomize
    "Everything not available",   // EverythingNotAvailable
    // Slideshow block (#37; en_us.h:78/118-140/200/248-252/264).
    "&Slideshow",           // MenuSlideshow
    "&Slideshow",           // MenuSlideshowMenu
    "&Play/Pause",          // MenuSlideshowPlayPause
    "&Rate",                // MenuSlideshowRate
    "&Decrease Rate",       // MenuSlideshowRateDecrease
    "&Increase Rate",       // MenuSlideshowRateIncrease
    "250 Milliseconds",     // MenuRate250Milliseconds
    "500 Milliseconds",     // MenuRate500Milliseconds
    "&1 Second",            // MenuRate1Second
    "&2 Seconds",           // MenuRate2Seconds
    "&3 Seconds",           // MenuRate3Seconds
    "&4 Seconds",           // MenuRate4Seconds
    "&5 Seconds",           // MenuRate5Seconds
    "&6 Seconds",           // MenuRate6Seconds
    "&7 Seconds",           // MenuRate7Seconds
    "&8 Seconds",           // MenuRate8Seconds
    "&9 Seconds",           // MenuRate9Seconds
    "1&0 Seconds",          // MenuRate10Seconds
    "20 Seconds",           // MenuRate20Seconds
    "30 Seconds",           // MenuRate30Seconds
    "40 Seconds",           // MenuRate40Seconds
    "50 Seconds",           // MenuRate50Seconds
    "1 Minute",             // MenuRate1Minute
    "Custom...",            // MenuRateCustom
    "Set Custom Rate",      // CustomRateCaption
    "&Custom rate:",        // CustomRateLabel
    "milliseconds",         // CustomRateMilliseconds
    "seconds",              // CustomRateSeconds
    "minutes",              // CustomRateMinutes
    "Slideshow playing",    // StatusBarSlideshowPlaying
    "Pos",                  // StatusBarPosLabel (en_us.h:201, composed)
    "Zoom",                 // StatusBarZoomLabel (en_us.h:201, composed)
    "Aspect Ratio",         // StatusBarAspectLabel (en_us.h:201, composed)
    "Animation rate",       // StatusBarAnimationRateLabel (en_us.h:202)
    "Slideshow rate",       // StatusBarSlideshowRateLabel (en_us.h:203)
    "minutes",              // StatusBarMinutes (en_us.h:204)
    "seconds",              // StatusBarSeconds (en_us.h:205)
    "milliseconds",         // StatusBarMilliseconds (en_us.h:206)
    "Play/Pause Slideshow", // ActionPlayPauseSlideshow
    "Play/Pause Animation", // ActionPlayPauseAnimation
    // Animation menu block (#38; en_us.h:142/144-157).
    "&Animation",                                  // MenuAnimation
    "&Play/Pause",                                 // MenuAnimationPlayPause
    "Jump &Forward",                               // MenuAnimationJumpForward
    "Jump &Backward",                              // MenuAnimationJumpBackward
    "Short Jump &Forward",                         // MenuAnimationShortJumpForward
    "Short Jump &Backward",                        // MenuAnimationShortJumpBackward
    "Long Jump &Forward",                          // MenuAnimationLongJumpForward
    "Long Jump &Backward",                         // MenuAnimationLongJumpBackward
    "F&rame Step",                                 // MenuAnimationFrameStep
    "Pre&vious Frame",                             // MenuAnimationPreviousFrame
    "F&irst Frame",                                // MenuAnimationFirstFrame
    "&Last Frame",                                 // MenuAnimationLastFrame
    "&Decrease Rate",                              // MenuAnimationRateDecrease
    "&Increase Rate",                              // MenuAnimationRateIncrease
    "R&eset Rate",                                 // MenuAnimationRateReset
    "&Play animations at least once in slideshow", // OptionsLoopAnimationsOnce (en_us.h:230)
    "&Sort",                                       // MenuSort (en_us.h:165)
    "&Name",                                       // MenuSortName (en_us.h:166)
    "Full &Path",                                  // MenuSortFullPath (en_us.h:167)
    "&Size",                                       // MenuSortSize (en_us.h:168)
    "Date &Modified",                              // MenuSortDateModified (en_us.h:169)
    "Date &Created",                               // MenuSortDateCreated (en_us.h:170)
    "&Ascending",                                  // MenuSortAscending (en_us.h:171)
    "&Descending",                                 // MenuSortDescending (en_us.h:172)
    "Shuffle",                                     // MenuShuffle (en_us.h:173)
    "&Jump To...",                                 // MenuJumpTo (en_us.h:174)
    "Jump To",                                     // JumpToCaption (en_us.h:255)
    "PRELOAD",                                     // StatusBarPreload (en_us.h:196)
    "Preload &next image",                         // OptionsPreloadNext (en_us.h:231)
    "Cache &last image",                           // OptionsCacheLast (en_us.h:232)
    "&Title bar format:",                          // OptionsTitleBarFormat (en_us.h:276)
    "Full Path",                                   // OptionsTitleBarFormatFullPath (en_us.h:277)
    "Filename Only",         // OptionsTitleBarFormatFilenameOnly (en_us.h:278)
    "None",                  // OptionsTitleBarFormatNone (en_us.h:279)
    "&Command Line Options", // MenuCommandLineOptions (en_us.h:179)
    // UsageText — upstream viv.c:11862-11894 verbatim except the exe name
    // brand swap; the /everything and /random rows stay hidden (upstream
    // comments them out).
    "Usage:\nriviv.exe [/switches] [filename(s)]\n\
     \n\
     Switches:\n\
     /slideshow\tStart a slideshow.\n\
     /close\t\tClose after the slideshow finishes.\n\
     /fullscreen\tStart fullscreen.\n\
     /maximized\tStart maximized.\n\
     /window\t\tStart windowed.\n\
     /ontop\t\tShow on top of other windows.\n\
     /minimal\t\tBorderless window.\n\
     /compact\t\tBordered window.\n\
     /x <x> /y <y> /width <width> /height <height>\n\
     \t\tSet the Window position and size.\n\
     /rate <rate>\tSet the slideshow rate in milliseconds.\n\
     /name\t\tSort by name.\n\
     /path\t\tSort by full path.\n\
     /size\t\tSort by size.\n\
     /dm\t\tSort by date modified.\n\
     /dc\t\tSort by date created.\n\
     /ascending\tSort in ascending order.\n\
     /descending\tSort in descending order.\n\
     /shuffle\t\tShuffle playlist.\n\
     /<bmp|gif|ico|jpeg|jpg|png|tif|tiff|webp>\n\
     \t\tInstall association.\n\
     /no<bmp|gif|ico|jpeg|jpg|png|tif|tiff|webp>\n\
     \t\tUninstall association.\n\
     /appdata\t\tSave settings in appdata.\n\
     /noappdata\tSave settings in exe path.\n\
     /startmenu\tAdd Start menu shortcuts.\n\
     /nostartmenu\tRemove Start menu shortcuts.\n\
     /install <path>\tInstall to the specified path.\n\
     /install-options <...> Run with the specified options after installation.\n\
     /uninstall <path>\tUninstall from the specified path.",
    "&Controls",       // MenuControls (en_us.h:72)
    "Previous Image",  // ToolbarPreviousImage (en_us.h:188)
    "Next Image",      // ToolbarNextImage (en_us.h:189)
    "Play Slideshow",  // ToolbarPlaySlideshow (en_us.h:190)
    "Pause Slideshow", // ToolbarPauseSlideshow (en_us.h:191)
    "Best Fit",        // ToolbarBestFit (en_us.h:192)
    "Actual Size",     // ToolbarActualSize (en_us.h:193)
    // #46 block (en_us.h:68-87/111-114).
    "Caption",                               // MenuCaption (en_us.h:68)
    "Frame",                                 // MenuThickFrame (en_us.h:69)
    "Status &Bar",                           // MenuStatusBar (en_us.h:71)
    "&Preset",                               // MenuPreset (en_us.h:73)
    "&Minimal",                              // MenuMinimal (en_us.h:74)
    "&Compact",                              // MenuCompact (en_us.h:75)
    "&Normal",                               // MenuNormal (en_us.h:76)
    "&Window Size",                          // MenuWindowSize (en_us.h:79)
    "50%",                                   // MenuWindowSize50 (en_us.h:80)
    "100%",                                  // MenuWindowSize100 (en_us.h:81)
    "200%",                                  // MenuWindowSize200 (en_us.h:82)
    "&Auto Fit",                             // MenuWindowSizeAutoFit (en_us.h:83)
    "&Refresh",                              // MenuRefresh (en_us.h:84)
    "&Allow Shrinking",                      // MenuAllowShrinking (en_us.h:85)
    "&Keep Aspect Ratio",                    // MenuKeepAspectRatio (en_us.h:86)
    "&Fill Window",                          // MenuFillWindow (en_us.h:87)
    "On &Top",                               // MenuOnTop (en_us.h:111)
    "&Always",                               // MenuAlways (en_us.h:112)
    "&While Playing Slideshow or Animating", // MenuWhilePlaying (en_us.h:113)
    "&Never",                                // MenuNever (en_us.h:114)
    // The #42 shell verb block (en_us.h:41-46/51).
    "Open File &Location...",   // MenuOpenFileLocation
    "&Edit...",                 // MenuFileEdit
    "Pre&view...",              // MenuPreview
    "&Print...",                // MenuPrint
    "Set Des&ktop Wallpaper",   // MenuSetDesktopWallpaper
    "&Close",                   // MenuClose
    "P&roperties",              // MenuProperties
    "&Delete",                  // MenuDelete (#43)
    "Delete (Recycle)",         // MenuDeleteRecycle (#43)
    "Delete (Permanently)",     // MenuDeletePermanently (#43)
    "Rena&me",                  // MenuRename (#43)
    "Rotate Cloc&kwise",        // MenuRotateClockwise (#43)
    "Rotate Cou&nterclockwise", // MenuRotateCounterclockwise (#43)
    "Copy to &Folder...",       // MenuCopyTo (#43)
    "Mo&ve to Folder...",       // MenuMoveTo (#43)
    "Rename",                   // RenameCaption (#43)
    "Copy To",                  // CopyToCaption (#43)
    "Move To",                  // MoveToCaption (#43)
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
    "编辑(&E)",          // MenuEdit (#41)
    "剪切(&T)",          // MenuCut
    "复制(&C)",          // MenuCopy
    "复制文件名",        // MenuCopyFilename — no mnemonic upstream
    "复制图像(&Y)",      // MenuCopyImage
    "粘贴(&P)",          // MenuPaste
    "视图(&V)",          // MenuView
    "菜单(&M)",          // MenuMenu
    "全屏(&F)",          // MenuFullscreen
    "1:1",               // MenuOneToOne
    "最佳适应(&B)",      // MenuBestFit
    "平移和扫描(&N)",    // MenuPanScan (#44; zh_cn.h:90)
    "增大尺寸(&I)",      // MenuPanScanIncreaseSize (zh_cn.h:91)
    "减小尺寸(&D)",      // MenuPanScanDecreaseSize (zh_cn.h:92)
    "增加宽度(&W)",      // MenuPanScanIncreaseWidth (zh_cn.h:93)
    "减小宽度(&W)",      // MenuPanScanDecreaseWidth (zh_cn.h:94)
    "增加高度(&H)",      // MenuPanScanIncreaseHeight (zh_cn.h:95)
    "减小高度(&E)",      // MenuPanScanDecreaseHeight (zh_cn.h:96)
    "向上移动(&U)",      // MenuPanScanMoveUp (zh_cn.h:101)
    "向下移动(&D)",      // MenuPanScanMoveDown (zh_cn.h:102)
    "向左移动(&L)",      // MenuPanScanMoveLeft (zh_cn.h:103)
    "向右移动(&R)",      // MenuPanScanMoveRight (zh_cn.h:104)
    "向左上移动",        // MenuPanScanMoveUpLeft (zh_cn.h:105)
    "向右上移动",        // MenuPanScanMoveUpRight (zh_cn.h:106)
    "向左下移动",        // MenuPanScanMoveDownLeft (zh_cn.h:107)
    "向右下移动",        // MenuPanScanMoveDownRight (zh_cn.h:108)
    "居中(&C)",          // MenuPanScanMoveCenter (zh_cn.h:109)
    "重置(&S)",          // MenuPanScanReset (zh_cn.h:110)
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
    "开始菜单快捷方式(&M)",               // OptionsStartMenu
    "文件关联",                           // OptionsAssociations
    "全选(&A)",                           // OptionsCheckAll
    "全不选(&N)",                         // OptionsCheckNone
    "缩小位图模式(&S):",                  // OptionsShrinkBlitMode
    "放大位图模式(&M):",                  // OptionsMagnifyBlitMode
    "最近邻",                             // OptionsBlitNearest
    "线性",                               // OptionsBlitLinear
    "保持纵横比(&K)",                     // OptionsKeepAspectRatio
    "填充窗口",                           // OptionsFillWindow — 同上去助记符
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
    "X 键操作(&B):",                      // OptionsXButtonAction — riviv 增补 (#44)
    "命令(&C):",                          // OptionsCommands
    "所选命令的设置",                     // OptionsSettingsForSelected
    "添加(&A)...",                        // OptionsAddKey
    "编辑(&E)...",                        // OptionsEditKey
    "删除(&V)",                           // OptionsRemoveKey
    "添加键盘快捷键",                     // AddKeyCaption
    "编辑键盘快捷键",                     // EditKeyCaption
    "快捷键(&K):",                        // OptionsShortcutKey
    "当前使用此快捷键的命令:",            // OptionsShortcutKeyUsedBy
    "滚动",                               // ActionScroll
    "放大",                               // ActionZoomIn
    "下一张图像",                         // ActionNextImage
    "上下文菜单",                         // ActionContextMenu
    "缩小",                               // ActionZoomOut
    "上一张图像",                         // ActionPreviousImage
    "缩放",                               // ActionZoom
    "下一张/上一张",                      // ActionNextPrev
    "上一张/下一张",                      // ActionPrevNext
    // Everything block (#22; zh_cn.h:37/40/281-284).
    "打开 Everything 搜索(&S)...", // MenuOpenEverythingSearch
    "添加 Everything 搜索...",     // MenuAddEverythingSearch
    "加载 Everything 搜索",        // EverythingLoadCaption
    "添加 Everything 搜索",        // EverythingAddCaption
    "随机化",                      // EverythingRandomize
    "Everything 不可用",           // EverythingNotAvailable
    // Slideshow block (#37; zh_cn.h:78/118-140/200/203-206/249-253/265).
    "幻灯片(&S)",      // MenuSlideshow
    "幻灯片(&S)",      // MenuSlideshowMenu
    "播放/暂停(&P)",   // MenuSlideshowPlayPause
    "速率(&R)",        // MenuSlideshowRate
    "降低速率(&D)",    // MenuSlideshowRateDecrease
    "提高速率(&I)",    // MenuSlideshowRateIncrease
    "250 毫秒",        // MenuRate250Milliseconds
    "500 毫秒",        // MenuRate500Milliseconds
    "&1 秒",           // MenuRate1Second
    "&2 秒",           // MenuRate2Seconds
    "&3 秒",           // MenuRate3Seconds
    "&4 秒",           // MenuRate4Seconds
    "&5 秒",           // MenuRate5Seconds
    "&6 秒",           // MenuRate6Seconds
    "&7 秒",           // MenuRate7Seconds
    "&8 秒",           // MenuRate8Seconds
    "&9 秒",           // MenuRate9Seconds
    "1&0 秒",          // MenuRate10Seconds
    "20 秒",           // MenuRate20Seconds
    "30 秒",           // MenuRate30Seconds
    "40 秒",           // MenuRate40Seconds
    "50 秒",           // MenuRate50Seconds
    "1 分钟",          // MenuRate1Minute
    "自定义...",       // MenuRateCustom
    "设置自定义速率",  // CustomRateCaption
    "自定义速率(&C):", // CustomRateLabel
    "毫秒",            // CustomRateMilliseconds
    "秒",              // CustomRateSeconds
    "分钟",            // CustomRateMinutes
    "幻灯片播放中",    // StatusBarSlideshowPlaying
    "位置",            // StatusBarPosLabel (zh_cn.h:201, composed)
    "缩放",            // StatusBarZoomLabel (zh_cn.h:201, composed)
    "宽高比",          // StatusBarAspectLabel (zh_cn.h:201, composed)
    "动画速率",        // StatusBarAnimationRateLabel (zh_cn.h:202)
    "幻灯片播放间隔",  // StatusBarSlideshowRateLabel (zh_cn.h:203)
    "分钟",            // StatusBarMinutes (zh_cn.h:204)
    "秒",              // StatusBarSeconds (zh_cn.h:205)
    "毫秒",            // StatusBarMilliseconds (zh_cn.h:206)
    "播放/暂停幻灯片", // ActionPlayPauseSlideshow
    "播放/暂停动画",   // ActionPlayPauseAnimation
    // Animation menu block (#38; zh_cn.h:143/144-157/231).
    "动画(&A)",                       // MenuAnimation
    "播放/暂停(&P)",                  // MenuAnimationPlayPause
    "向前跳转(&F)",                   // MenuAnimationJumpForward
    "向后跳转(&B)",                   // MenuAnimationJumpBackward
    "短距离向前跳转(&F)",             // MenuAnimationShortJumpForward
    "短距离向后跳转(&B)",             // MenuAnimationShortJumpBackward
    "长距离向前跳转(&F)",             // MenuAnimationLongJumpForward
    "长距离向后跳转(&B)",             // MenuAnimationLongJumpBackward
    "下一帧(&S)",                     // MenuAnimationFrameStep
    "上一帧(&V)",                     // MenuAnimationPreviousFrame
    "第一帧(&I)",                     // MenuAnimationFirstFrame
    "最后一帧(&L)",                   // MenuAnimationLastFrame
    "降低速率(&D)",                   // MenuAnimationRateDecrease
    "提高速率(&I)",                   // MenuAnimationRateIncrease
    "重置速率(&E)",                   // MenuAnimationRateReset
    "在幻灯片中至少播放一次动画(&P)", // OptionsLoopAnimationsOnce (zh_cn.h:231)
    "排序(&S)",                       // MenuSort (zh_cn.h:165)
    "名称(&N)",                       // MenuSortName (zh_cn.h:166)
    "完整路径(&P)",                   // MenuSortFullPath (zh_cn.h:167)
    "大小(&S)",                       // MenuSortSize (zh_cn.h:168)
    "修改日期(&M)",                   // MenuSortDateModified (zh_cn.h:169)
    "创建日期(&C)",                   // MenuSortDateCreated (zh_cn.h:170)
    "升序(&A)",                       // MenuSortAscending (zh_cn.h:171)
    "降序(&D)",                       // MenuSortDescending (zh_cn.h:172)
    "随机(&S)",                       // MenuShuffle (zh_cn.h:173)
    "跳转到(&J)...",                  // MenuJumpTo (zh_cn.h:174)
    "跳转到",                         // JumpToCaption (zh_cn.h:256)
    "预加载",                         // StatusBarPreload (zh_cn.h:196)
    "预加载下一张图像(&N)",           // OptionsPreloadNext (zh_cn.h:232)
    "缓存最后一张图像(&L)",           // OptionsCacheLast (zh_cn.h:233)
    "标题栏格式(&T):",                // OptionsTitleBarFormat (zh_cn.h:277)
    "完整路径",                       // OptionsTitleBarFormatFullPath (zh_cn.h:278)
    "仅文件名",                       // OptionsTitleBarFormatFilenameOnly (zh_cn.h:279)
    "无",                             // OptionsTitleBarFormatNone (zh_cn.h:280)
    "命令行选项(&C)",                 // MenuCommandLineOptions (zh_cn.h:179)
    // UsageText — riviv's translation of viv.c:11862-11894 (upstream
    // hardcodes English; the issue mandates the bilingual body).
    "用法:\nriviv.exe [/开关] [文件名]\n\
     \n\
     开关:\n\
     /slideshow\t开始幻灯片播放。\n\
     /close\t\t幻灯片播放完毕后退出。\n\
     /fullscreen\t全屏启动。\n\
     /maximized\t最大化启动。\n\
     /window\t\t窗口化启动。\n\
     /ontop\t\t窗口置顶。\n\
     /minimal\t\t无边框窗口。\n\
     /compact\t\t有边框窗口。\n\
     /x <x> /y <y> /width <宽> /height <高>\n\
     \t\t设置窗口位置和大小。\n\
     /rate <速率>\t设置幻灯片速率(毫秒)。\n\
     /name\t\t按名称排序。\n\
     /path\t\t按完整路径排序。\n\
     /size\t\t按大小排序。\n\
     /dm\t\t按修改日期排序。\n\
     /dc\t\t按创建日期排序。\n\
     /ascending\t升序排序。\n\
     /descending\t降序排序。\n\
     /shuffle\t\t随机播放列表。\n\
     /<bmp|gif|ico|jpeg|jpg|png|tif|tiff|webp>\n\
     \t\t安装文件关联。\n\
     /no<bmp|gif|ico|jpeg|jpg|png|tif|tiff|webp>\n\
     \t\t卸载文件关联。\n\
     /appdata\t\t将设置保存到 appdata。\n\
     /noappdata\t将设置保存到 exe 所在目录。\n\
     /startmenu\t添加开始菜单快捷方式。\n\
     /nostartmenu\t移除开始菜单快捷方式。\n\
     /install <路径>\t安装到指定路径。\n\
     /install-options <...> 安装后以指定选项运行。\n\
     /uninstall <路径>\t从指定路径卸载。",
    "控件(&C)", // MenuControls (zh_cn.h:72)
    "上一个",   // ToolbarPreviousImage (zh_cn.h:188)
    "下一个",   // ToolbarNextImage (zh_cn.h:189)
    "播放",     // ToolbarPlaySlideshow (zh_cn.h:190)
    "暂停",     // ToolbarPauseSlideshow (zh_cn.h:191)
    "最佳适应", // ToolbarBestFit (zh_cn.h:192)
    "实际大小", // ToolbarActualSize (zh_cn.h:193)
    // #46 block (zh_cn.h:68-87/111-114).
    "标题栏",                 // MenuCaption (zh_cn.h:68)
    "边框",                   // MenuThickFrame (zh_cn.h:69)
    "状态栏(&B)",             // MenuStatusBar (zh_cn.h:71)
    "预设(&P)",               // MenuPreset (zh_cn.h:73)
    "最小(&M)",               // MenuMinimal (zh_cn.h:74)
    "紧凑(&C)",               // MenuCompact (zh_cn.h:75)
    "正常(&N)",               // MenuNormal (zh_cn.h:76)
    "窗口大小(&W)",           // MenuWindowSize (zh_cn.h:79)
    "50%",                    // MenuWindowSize50 (zh_cn.h:80)
    "100%",                   // MenuWindowSize100 (zh_cn.h:81)
    "200%",                   // MenuWindowSize200 (zh_cn.h:82)
    "自动适应(&A)",           // MenuWindowSizeAutoFit (zh_cn.h:83)
    "刷新(&R)",               // MenuRefresh (zh_cn.h:84)
    "允许缩小(&A)",           // MenuAllowShrinking (zh_cn.h:85)
    "保持纵横比(&K)",         // MenuKeepAspectRatio (zh_cn.h:86)
    "填充窗口(&F)",           // MenuFillWindow (zh_cn.h:87)
    "置顶(&T)",               // MenuOnTop (zh_cn.h:111)
    "总是(&A)",               // MenuAlways (zh_cn.h:112)
    "播放幻灯片或动画时(&W)", // MenuWhilePlaying (zh_cn.h:113)
    "从不(&N)",               // MenuNever (zh_cn.h:114)
    // The #42 shell verb block (zh_cn.h:41-46/51).
    "打开文件位置(&L)...", // MenuOpenFileLocation
    "编辑(&E)...",         // MenuFileEdit
    "预览(&V)...",         // MenuPreview
    "打印(&P)...",         // MenuPrint
    "设置为桌面壁纸(&D)",  // MenuSetDesktopWallpaper
    "关闭(&C)",            // MenuClose
    "属性(&P)",            // MenuProperties
    "删除(&D)",            // MenuDelete (#43)
    "删除（回收站）",      // MenuDeleteRecycle (#43)
    "删除（永久）",        // MenuDeletePermanently (#43)
    "重命名(&M)",          // MenuRename (#43)
    "顺时针旋转(&K)",      // MenuRotateClockwise (#43)
    "逆时针旋转(&N)",      // MenuRotateCounterclockwise (#43)
    "复制到文件夹(&F)...", // MenuCopyTo (#43)
    "移动到文件夹(&V)...", // MenuMoveTo (#43)
    "重命名",              // RenameCaption (#43)
    "复制到",              // CopyToCaption (#43)
    "移动到",              // MoveToCaption (#43)
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
    fn usage_text_lists_the_close_switch_in_both_languages() {
        // #67: the /close row rides in the usage body (the row after
        // /slideshow), in English and the zh translation alike.
        let en = get_for(Language::English, Id::UsageText);
        assert!(en.contains("/slideshow\tStart a slideshow."));
        assert!(en.contains("/close\t\tClose after the slideshow finishes."));
        let zh = get_for(Language::ChineseSimplified, Id::UsageText);
        assert!(zh.contains("/close\t\t幻灯片播放完毕后退出。"));
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
        assert_eq!(get_for(en, Id::OptionsStartMenu), "Start &menu shortcuts");
        assert_eq!(get_for(en, Id::OptionsAssociations), "Associations");
        assert_eq!(get_for(en, Id::OptionsCheckAll), "Check &All");
        assert_eq!(get_for(en, Id::OptionsCheckNone), "Check &None");
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
        assert_eq!(get_for(en, Id::OptionsFillWindow), "Fill Window");
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
        assert_eq!(get_for(zh, Id::OptionsStartMenu), "开始菜单快捷方式(&M)");
        assert_eq!(get_for(zh, Id::OptionsAssociations), "文件关联");
        assert_eq!(get_for(zh, Id::OptionsCheckAll), "全选(&A)");
        assert_eq!(get_for(zh, Id::OptionsCheckNone), "全不选(&N)");
        assert_eq!(get_for(zh, Id::OptionsShrinkBlitMode), "缩小位图模式(&S):");
        assert_eq!(get_for(zh, Id::OptionsMagnifyBlitMode), "放大位图模式(&M):");
        assert_eq!(get_for(zh, Id::OptionsBlitNearest), "最近邻");
        assert_eq!(get_for(zh, Id::OptionsBlitLinear), "线性");
        assert_eq!(get_for(zh, Id::OptionsKeepAspectRatio), "保持纵横比(&K)");
        assert_eq!(get_for(zh, Id::OptionsFillWindow), "填充窗口");
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
