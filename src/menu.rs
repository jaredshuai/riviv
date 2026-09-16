//! Menu command table (#23): the pure model behind the menu bar — the
//! command/entry tables, WM_COMMAND id mapping, accelerator-label
//! composition, and the WM_INITMENU check/enable decisions.
//!
//! Upstream's `_viv_commands[]` (viv.c:798-965) registers EVERY command the
//! C build ships, greyed or not; riviv's table deliberately carries only
//! the commands it implements (issue #23) and grows with each feature —
//! slideshow, sort modes, Everything search, clipboard etc. each append
//! their rows. Layout mirrors upstream: an entry names its PARENT menu
//! slot, popup rows introduce their slot, and the walk order is the menu
//! order (`_viv_create_menu`, viv.c:12314-12399, creates the popup menus
//! on demand in exactly this order).
//!
//! The Win32 half lives in `window.rs`: it walks [`ENTRIES`] into real
//! HMENU objects, appends the accelerator labels (from the live
//! [`crate::keys::KeyMap`]'s first binding per command — #25), and
//! dispatches WM_COMMAND through [`Cmd::from_id`].

use crate::loc;

/// A command the menu can trigger — the implemented subset of upstream's
/// `VIV_ID_*` (viv.h:71-204). Order is upstream table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmd {
    /// File → Open File... (`VIV_ID_FILE_OPEN_FILE`).
    FileOpenFile,
    /// File → Open Folder... (`VIV_ID_FILE_OPEN_FOLDER`).
    FileOpenFolder,
    /// File → Open Everything Search... (`VIV_ID_FILE_OPEN_EVERYTHING_
    /// SEARCH`, viv.c:804) — the #22 search dialog's Open flavor.
    FileOpenEverythingSearch,
    /// File → Add File... (`VIV_ID_FILE_ADD_FILE`) — the Ctrl+Shift+O
    /// append path.
    FileAddFile,
    /// File → Add Everything Search... (`VIV_ID_FILE_ADD_EVERYTHING_
    /// SEARCH`, viv.c:807) — the #22 search dialog's Add flavor.
    FileAddEverythingSearch,
    /// File → Open File &Location... (#42; `VIV_ID_FILE_OPEN_FILE_
    /// LOCATION`, viv.c:811) — Ctrl+Return: select the current file in
    /// Explorer (SHOpenFolderAndSelectItems; the fallback opens the
    /// parent folder).
    FileOpenFileLocation,
    /// File → &Edit... (#42; `VIV_ID_FILE_EDIT`, viv.c:812) — the shell
    /// "edit" verb; no default key (upstream comments Ctrl+E out — the
    /// Everything search owns it, viv.c:979).
    FileEdit,
    /// File → Pre&view... (#42; `VIV_ID_FILE_PREVIEW`, viv.c:813) — the
    /// shell "preview" verb; no default key upstream.
    FilePreview,
    /// File → &Print... (#42; `VIV_ID_FILE_PRINT`, viv.c:814) — the shell
    /// "print" verb, Ctrl+P.
    FilePrint,
    /// File → Set Des&ktop Wallpaper (#42; `VIV_ID_FILE_SET_DESKTOP_
    /// WALLPAPER`, viv.c:815) — the "setdesktopwallpaper" verb behind a
    /// one-shot stobject.dll load; no default key (upstream comments it
    /// out — "needs a confirmation dialog", viv.c:982).
    FileSetDesktopWallpaper,
    /// File → &Close (#42; `VIV_ID_FILE_CLOSE`, viv.c:816 — MF_OWNERDRAW
    /// upstream, keyboard-only Ctrl+W like the #41 hidden rows). This
    /// just clears the image (upstream `_viv_blank`, viv.c:7908).
    FileClose,
    /// File → &Delete (#43; `VIV_ID_FILE_DELETE`, viv.c:817) — the VISIBLE
    /// row that live-probes Shift at dispatch time (`GetKeyState`,
    /// viv.c:2458-2460): held = permanent, free = recycle. Upstream's row
    /// carries `MF_STRING|MF_DELETE`; the MF_DELETE bit is a ModifyMenu-
    /// family flag AppendMenu ignores — inert, modeled as a plain row. No
    /// default key (Del/Shift+Del belong to the two hidden rows).
    FileDelete,
    /// File → Delete (Recycle) (#43; `VIV_ID_FILE_DELETE_RECYCLE`,
    /// viv.c:818 — MF_OWNERDRAW upstream, keyboard Del; `_viv_delete(0)`
    /// = FO_DELETE + FOF_ALLOWUNDO, the Recycle Bin).
    FileDeleteRecycle,
    /// File → Delete (Permanently) (#43; `VIV_ID_FILE_DELETE_PERMANENTLY`,
    /// viv.c:819 — MF_OWNERDRAW upstream, keyboard Shift+Del;
    /// `_viv_delete(1)` = FO_DELETE with no undo flag).
    FileDeletePermanently,
    /// File → Rena&me (#43; `VIV_ID_FILE_RENAME`, viv.c:821, F2) — the
    /// rename dialog; FO_RENAME with collision resolution, playlist sync.
    FileRename,
    /// File → P&roperties (#42; `VIV_ID_FILE_PROPERTIES`, viv.c:820) —
    /// the shell "properties" verb; no default key upstream. Upstream
    /// menu order puts it past the delete block, so #43's Delete/Rename
    /// rows land between Close and this row.
    FileProperties,
    /// File → Exit (`VIV_ID_FILE_EXIT`).
    FileExit,
    /// Edit → Cu&t (#41; `VIV_ID_EDIT_CUT`, viv.c:826).
    EditCut,
    /// Edit → &Copy (#41; `VIV_ID_EDIT_COPY`, viv.c:827).
    EditCopy,
    /// Edit → Copy Filename (#41; `VIV_ID_EDIT_COPY_FILENAME`, viv.c:828
    /// — MF_OWNERDRAW upstream, keyboard Ctrl+Shift+C).
    EditCopyFilename,
    /// Edit → Cop&y Image (#41; `VIV_ID_EDIT_COPY_IMAGE`, viv.c:829).
    EditCopyImage,
    /// Edit → &Paste (#41; `VIV_ID_EDIT_PASTE`, viv.c:830 — MF_OWNERDRAW
    /// upstream, keyboard Ctrl+V).
    EditPaste,
    /// Edit → Rotate Cloc&kwise (#43; `VIV_ID_EDIT_ROTATE_90`, viv.c:833)
    /// — the shell "rotate90" verb (waited), then the in-memory frame
    /// rotation. No default key upstream.
    EditRotate90,
    /// Edit → Rotate Cou&nterclockwise (#43; `VIV_ID_EDIT_ROTATE_270`,
    /// viv.c:834) — the shell "rotate270" verb (waited), then the
    /// in-memory rotation the other way. No default key upstream.
    EditRotate270,
    /// Edit → Copy to &Folder... (#43; `VIV_ID_EDIT_COPY_TO`, viv.c:836)
    /// — GetSaveFileName + FO_COPY (FOF_ALLOWUNDO). No default key.
    EditCopyTo,
    /// Edit → Mo&ve to Folder... (#43; `VIV_ID_EDIT_MOVE_TO`, viv.c:837)
    /// — GetSaveFileName + FO_MOVE (FOF_ALLOWUNDO). No playlist/current
    /// sync after (upstream has none). No default key.
    EditMoveTo,
    /// View → Caption toggle (#46; `VIV_ID_VIEW_CAPTION`, viv.c:843 —
    /// MF_OWNERDRAW upstream, menu-hidden: the WS_CAPTION|WS_SYSMENU style
    /// bits are reachable through the ini and custom bindings only).
    ViewCaption,
    /// View → Frame toggle (#46; `VIV_ID_VIEW_THICKFRAME`, viv.c:844 —
    /// MF_OWNERDRAW upstream, menu-hidden like Caption).
    ViewThickFrame,
    /// View → Menu toggle (`VIV_ID_VIEW_MENU`).
    ViewMenu,
    /// View → Status Bar toggle (#46; `VIV_ID_VIEW_STATUS`, viv.c:846).
    ViewStatus,
    /// View → Controls toggle (#45; `VIV_ID_VIEW_CONTROLS`, viv.c:844 —
    /// upstream order puts it between the Status Bar row and Preset).
    ViewControls,
    /// View → Preset → Minimal (#46; `VIV_ID_VIEW_PRESET_1`, viv.c:849 —
    /// key '1'): every chrome piece off (viv.c:1990-1996).
    ViewPreset1,
    /// View → Preset → Compact (#46; `VIV_ID_VIEW_PRESET_2`, viv.c:850 —
    /// key '2'): only the thick frame stays (viv.c:1998-2004).
    ViewPreset2,
    /// View → Preset → Normal (#46; `VIV_ID_VIEW_PRESET_3`, viv.c:851 —
    /// key '3'): everything on (viv.c:2006-2013).
    ViewPreset3,
    /// View → Fullscreen (`VIV_ID_VIEW_FULLSCREEN`).
    ViewFullscreen,
    /// View → Slideshow (`VIV_ID_VIEW_SLIDESHOW`, viv.c:854) — #37: the
    /// start-only toggle (enters fullscreen first when windowed).
    ViewSlideshow,
    /// View → Window Size → 50% (#46; `VIV_ID_VIEW_WINDOW_SIZE_50`,
    /// viv.c:856 — Alt+1): size the window around half the image
    /// (viv.c:2114-2116).
    ViewWindowSize50,
    /// View → Window Size → 100% (#46; `VIV_ID_VIEW_WINDOW_SIZE_100`,
    /// viv.c:857 — Alt+2).
    ViewWindowSize100,
    /// View → Window Size → 200% (#46; `VIV_ID_VIEW_WINDOW_SIZE_200`,
    /// viv.c:858 — Alt+3).
    ViewWindowSize200,
    /// View → Window Size → Auto Fit (#46;
    /// `VIV_ID_VIEW_WINDOW_SIZE_AUTO_FIT`, viv.c:859 — Alt+4): the
    /// auto-fit monitor fraction (the #24 `auto_fit_*` config).
    ViewWindowSizeAutoFit,
    /// View → Refresh (#46; `VIV_ID_VIEW_REFRESH`, viv.c:860 — F5):
    /// re-read the current file off disk (upstream `_viv_refresh`,
    /// viv.c:14539-14552).
    ViewRefresh,
    /// View → Allow Shrinking (#46; `VIV_ID_VIEW_ALLOW_SHRINKING`,
    /// viv.c:862): the `allow_shrinking` fit input (the fit-level ladder's
    /// shrink half).
    ViewAllowShrinking,
    /// View → Keep Aspect Ratio (#46; `VIV_ID_VIEW_KEEP_ASPECT_RATIO`,
    /// viv.c:863): the `keep_aspect_ratio` fit input.
    ViewKeepAspect,
    /// View → Fill Window (#46; `VIV_ID_VIEW_FILL_WINDOW`, viv.c:864):
    /// the `fill_window` fit input — in fullscreen the
    /// `fullscreen_fill_window` one instead (upstream's own quirk,
    /// viv.c:2033-2044).
    ViewFillWindow,
    /// View → 1:1 (`VIV_ID_VIEW_1TO1`).
    ViewOneToOne,
    /// View → Best Fit (`VIV_ID_VIEW_BESTFIT`).
    ViewBestFit,
    /// View → Pan/Scan → Increase Size (#44; `VIV_ID_VIEW_PANSCAN_
    /// INCREASE_SIZE`, viv.h:110 — NUMPAD9).
    ViewPanScanIncreaseSize,
    /// View → Pan/Scan → Decrease Size (#44; `VIV_ID_VIEW_PANSCAN_
    /// DECREASE_SIZE`, viv.h:111 — NUMPAD1).
    ViewPanScanDecreaseSize,
    /// View → Pan/Scan → Increase Width (#44; `VIV_ID_VIEW_PANSCAN_
    /// INCREASE_WIDTH`, viv.h:112 — NUMPAD6).
    ViewPanScanIncreaseWidth,
    /// View → Pan/Scan → Decrease Width (#44; `VIV_ID_VIEW_PANSCAN_
    /// DECREASE_WIDTH`, viv.h:113 — NUMPAD4).
    ViewPanScanDecreaseWidth,
    /// View → Pan/Scan → Increase Height (#44; `VIV_ID_VIEW_PANSCAN_
    /// INCREASE_HEIGHT`, viv.h:114 — NUMPAD8).
    ViewPanScanIncreaseHeight,
    /// View → Pan/Scan → Decrease Height (#44; `VIV_ID_VIEW_PANSCAN_
    /// DECREASE_HEIGHT`, viv.h:115 — NUMPAD2).
    ViewPanScanDecreaseHeight,
    /// View → Pan/Scan → Move Up (#44; `VIV_ID_VIEW_PANSCAN_MOVE_UP`,
    /// viv.h:116 — Ctrl+NUMPAD8).
    ViewPanScanMoveUp,
    /// View → Pan/Scan → Move Down (#44; `VIV_ID_VIEW_PANSCAN_MOVE_DOWN`,
    /// viv.h:117 — Ctrl+NUMPAD2).
    ViewPanScanMoveDown,
    /// View → Pan/Scan → Move Left (#44; `VIV_ID_VIEW_PANSCAN_MOVE_LEFT`,
    /// viv.h:118 — Ctrl+NUMPAD4).
    ViewPanScanMoveLeft,
    /// View → Pan/Scan → Move Right (#44; `VIV_ID_VIEW_PANSCAN_MOVE_RIGHT`,
    /// viv.h:119 — Ctrl+NUMPAD6).
    ViewPanScanMoveRight,
    /// View → Pan/Scan → Move Up Left (#44; `VIV_ID_VIEW_PANSCAN_MOVE_UP_
    /// LEFT`, viv.h:120 — MF_OWNERDRAW upstream, keyboard-only
    /// Ctrl+NUMPAD7 like the #38 jump family).
    ViewPanScanMoveUpLeft,
    /// View → Pan/Scan → Move Up Right (#44; `VIV_ID_VIEW_PANSCAN_MOVE_UP_
    /// RIGHT`, viv.h:121 — MF_OWNERDRAW, Ctrl+NUMPAD9).
    ViewPanScanMoveUpRight,
    /// View → Pan/Scan → Move Down Left (#44; `VIV_ID_VIEW_PANSCAN_MOVE_
    /// DOWN_LEFT`, viv.h:122 — MF_OWNERDRAW, Ctrl+NUMPAD1).
    ViewPanScanMoveDownLeft,
    /// View → Pan/Scan → Move Down Right (#44; `VIV_ID_VIEW_PANSCAN_MOVE_
    /// DOWN_RIGHT`, viv.h:123 — MF_OWNERDRAW, Ctrl+NUMPAD3).
    ViewPanScanMoveDownRight,
    /// View → Pan/Scan → Move Center (#44; `VIV_ID_VIEW_PANSCAN_MOVE_
    /// CENTER`, viv.h:124 — Ctrl+NUMPAD5).
    ViewPanScanMoveCenter,
    /// View → Pan/Scan → Reset (#44; `VIV_ID_VIEW_PANSCAN_RESET`,
    /// viv.h:125 — NUMPAD5).
    ViewPanScanReset,
    /// Zoom → Zoom In (`VIV_ID_VIEW_ZOOM_IN`).
    ViewZoomIn,
    /// Zoom → Zoom Out (`VIV_ID_VIEW_ZOOM_OUT`).
    ViewZoomOut,
    /// Zoom → Reset (`VIV_ID_VIEW_ZOOM_RESET`).
    ViewZoomReset,
    /// View → On Top → Always (#46; `VIV_ID_VIEW_ONTOP_ALWAYS`, viv.c:890
    /// — Ctrl+T): a TOGGLE upstream (`config_ontop = !config_ontop`,
    /// viv.c:2317-2319 — from the "while" value 2 the C `!` lands on 0,
    /// a quirk kept bug-for-bug).
    ViewOntopAlways,
    /// View → On Top → While Playing Slideshow or Animating (#46;
    /// `VIV_ID_VIEW_ONTOP_WHILE_PLAYING_OR_ANIMATING`, viv.c:891):
    /// `config_ontop = 2`.
    ViewOntopWhilePlaying,
    /// View → On Top → Never (#46; `VIV_ID_VIEW_ONTOP_NEVER`, viv.c:892):
    /// `config_ontop = 0`.
    ViewOntopNever,
    /// View → Options... (`VIV_ID_VIEW_OPTIONS`) — opens the modal Options
    /// dialog (#24; upstream viv.c:2332).
    ViewOptions,
    /// Slideshow → Play/Pause (`VIV_ID_SLIDESHOW_PAUSE`, viv.c:894) — #37:
    /// the running-state TOGGLE (no fullscreen entry, unlike F11).
    SlideshowPause,
    /// The toolbar-only slideshow pair (#45; `VIV_ID_SLIDESHOW_PLAY_ONLY`
    /// / `VIV_ID_SLIDESHOW_PAUSE_ONLY`, viv.h:132-133 — upstream keeps
    /// them OUT of `_viv_commands[]` entirely; riviv registers them as
    /// hidden rows so the command table's every-command-has-a-row
    /// invariant holds — a small superset: they become bindable, README
    /// Differences).
    SlideshowPlayOnly,
    SlideshowPauseOnly,
    /// Slideshow → Rate → Decrease Rate (`VIV_ID_SLIDESHOW_RATE_DEC`,
    /// viv.c:898).
    SlideshowRateDecrease,
    /// Slideshow → Rate → Increase Rate (`VIV_ID_SLIDESHOW_RATE_INC`,
    /// viv.c:899).
    SlideshowRateIncrease,
    /// Slideshow → Rate → 250 ms (`VIV_ID_SLIDESHOW_RATE_250`, viv.c:902).
    SlideshowRate250,
    /// Slideshow → Rate → 500 ms (`VIV_ID_SLIDESHOW_RATE_500`, viv.c:903).
    SlideshowRate500,
    /// Slideshow → Rate → 1 s (`VIV_ID_SLIDESHOW_RATE_1000`, viv.c:904).
    SlideshowRate1000,
    /// Slideshow → Rate → 2 s (`VIV_ID_SLIDESHOW_RATE_2000`, viv.c:905).
    SlideshowRate2000,
    /// Slideshow → Rate → 3 s (`VIV_ID_SLIDESHOW_RATE_3000`, viv.c:906).
    SlideshowRate3000,
    /// Slideshow → Rate → 4 s (`VIV_ID_SLIDESHOW_RATE_4000`, viv.c:907).
    SlideshowRate4000,
    /// Slideshow → Rate → 5 s (`VIV_ID_SLIDESHOW_RATE_5000`, viv.c:908).
    SlideshowRate5000,
    /// Slideshow → Rate → 6 s (`VIV_ID_SLIDESHOW_RATE_6000`, viv.c:909).
    SlideshowRate6000,
    /// Slideshow → Rate → 7 s (`VIV_ID_SLIDESHOW_RATE_7000`, viv.c:910).
    SlideshowRate7000,
    /// Slideshow → Rate → 8 s (`VIV_ID_SLIDESHOW_RATE_8000`, viv.c:911).
    SlideshowRate8000,
    /// Slideshow → Rate → 9 s (`VIV_ID_SLIDESHOW_RATE_9000`, viv.c:912).
    SlideshowRate9000,
    /// Slideshow → Rate → 10 s (`VIV_ID_SLIDESHOW_RATE_10000`, viv.c:913).
    SlideshowRate10000,
    /// Slideshow → Rate → 20 s (`VIV_ID_SLIDESHOW_RATE_20000`, viv.c:914).
    SlideshowRate20000,
    /// Slideshow → Rate → 30 s (`VIV_ID_SLIDESHOW_RATE_30000`, viv.c:915).
    SlideshowRate30000,
    /// Slideshow → Rate → 40 s (`VIV_ID_SLIDESHOW_RATE_40000`, viv.c:916).
    SlideshowRate40000,
    /// Slideshow → Rate → 50 s (`VIV_ID_SLIDESHOW_RATE_50000`, viv.c:917).
    SlideshowRate50000,
    /// Slideshow → Rate → 1 min (`VIV_ID_SLIDESHOW_RATE_60000`,
    /// viv.c:918).
    SlideshowRate60000,
    /// Slideshow → Rate → Custom... (`VIV_ID_SLIDESHOW_RATE_CUSTOM`,
    /// viv.c:918 rate block tail) — the value dialog.
    SlideshowRateCustom,
    /// Animation → Play/Pause (`VIV_ID_ANIMATION_PLAY_PAUSE`, viv.c:921)
    /// — #38: the pause toggle; its menu check reads the playing flag
    /// (viv.c:7184).
    AnimationPlayPause,
    /// Animation → Jump Forward / Jump Backward (medium, viv.c:923-924)
    /// — the menu-visible pair.
    AnimationJumpForwardMedium,
    AnimationJumpBackwardMedium,
    /// The short/long jump quartet (viv.c:925-928): MF_OWNERDRAW upstream
    /// — never appended to the menu bar (viv.c:12328) but a real command
    /// (custom-shortcut-bindable, viv.c:8269-8290 lists it).
    AnimationJumpForwardShort,
    AnimationJumpBackwardShort,
    AnimationJumpForwardLong,
    AnimationJumpBackwardLong,
    /// Animation → Frame Step / Previous Frame (viv.c:930-931).
    AnimationFrameStep,
    AnimationFramePrev,
    /// Animation → First Frame / Last Frame (viv.c:932-933).
    AnimationFirstFrame,
    AnimationLastFrame,
    /// Animation → Decrease / Increase / Reset Rate (viv.c:935-937) — the
    /// 21-entry speed table (`anim::RATE_TABLE`).
    AnimationRateDecrease,
    AnimationRateIncrease,
    AnimationRateReset,
    /// Navigate → Next (`VIV_ID_NAV_NEXT`).
    NavNext,
    /// Navigate → Previous (`VIV_ID_NAV_PREV`).
    NavPrev,
    /// Navigate → Home (`VIV_ID_NAV_HOME`).
    NavHome,
    /// Navigate → End (`VIV_ID_NAV_END`).
    NavEnd,
    /// Navigate → Sort → the five mode radios (#39; `VIV_ID_NAV_SORT_*`,
    /// viv.c:946-950 — menu-table order Name/Full Path/Size/Date Modified/
    /// Date Created, NOT the handler's case order).
    NavSortName,
    NavSortFullPath,
    NavSortSize,
    NavSortDateModified,
    NavSortDateCreated,
    /// Navigate → Sort → Ascending / Descending radios (#39; viv.c:952-953).
    NavSortAscending,
    NavSortDescending,
    /// Navigate → Shuffle (#39; `VIV_ID_NAV_SHUFFLE`, viv.c:954 — a plain
    /// check row, not a radio).
    NavShuffle,
    /// Navigate → Jump To... (#39; `VIV_ID_NAV_JUMPTO`, viv.c:956 — default
    /// key 'J', viv.c:1046).
    NavJumpTo,
    /// Help → Command Line Options (`VIV_ID_HELP_COMMAND_LINE_OPTIONS`,
    /// viv.c:960 — no default key) — the usage box (#48).
    HelpCommandLineOptions,
    /// Help → About (`VIV_ID_HELP_ABOUT`).
    HelpAbout,
}

impl Cmd {
    /// Variant count; also the id space size (ids are 1-based — 0 is the
    /// separator/no-command id in Win32 menus and must stay unassigned).
    pub(crate) const COUNT: usize = Self::HelpAbout as usize + 1;

    /// The WM_COMMAND command id (upstream uses the `VIV_ID_*` enum values;
    /// riviv's ids are app-internal — nothing interoperates — so they run
    /// 1-based over the enum order).
    pub(crate) fn id(self) -> u16 {
        self as u16 + 1
    }

    /// The sort mode a Sort-submenu radio row selects (#39; `None` for the
    /// direction pair, Shuffle and Jump To — every non-mode command).
    pub(crate) fn sort_mode(self) -> Option<crate::playlist::SortMode> {
        match self {
            Self::NavSortName => Some(crate::playlist::SortMode::Name),
            Self::NavSortFullPath => Some(crate::playlist::SortMode::FullPath),
            Self::NavSortSize => Some(crate::playlist::SortMode::Size),
            Self::NavSortDateModified => Some(crate::playlist::SortMode::DateModified),
            Self::NavSortDateCreated => Some(crate::playlist::SortMode::DateCreated),
            _ => None,
        }
    }

    /// The rate (in ms) a Rate-submenu row selects; `None` for Custom
    /// (which opens the dialog) and every non-rate command (viv.c:902-918).
    pub(crate) fn slideshow_rate_ms(self) -> Option<u32> {
        match self {
            Self::SlideshowRate250 => Some(250),
            Self::SlideshowRate500 => Some(500),
            Self::SlideshowRate1000 => Some(1_000),
            Self::SlideshowRate2000 => Some(2_000),
            Self::SlideshowRate3000 => Some(3_000),
            Self::SlideshowRate4000 => Some(4_000),
            Self::SlideshowRate5000 => Some(5_000),
            Self::SlideshowRate6000 => Some(6_000),
            Self::SlideshowRate7000 => Some(7_000),
            Self::SlideshowRate8000 => Some(8_000),
            Self::SlideshowRate9000 => Some(9_000),
            Self::SlideshowRate10000 => Some(10_000),
            Self::SlideshowRate20000 => Some(20_000),
            Self::SlideshowRate30000 => Some(30_000),
            Self::SlideshowRate40000 => Some(40_000),
            Self::SlideshowRate50000 => Some(50_000),
            Self::SlideshowRate60000 => Some(60_000),
            _ => None,
        }
    }

    /// The window-size kind a Window Size submenu row selects (#46; the
    /// `kind` argument of `window_size_to_image` / `fit::window_size_
    /// client`, matching #24's auto_zoom_type values): `None` for every
    /// non-window-size command.
    pub(crate) fn window_size_kind(self) -> Option<i32> {
        match self {
            Self::ViewWindowSize50 => Some(0),
            Self::ViewWindowSize100 => Some(1),
            Self::ViewWindowSize200 => Some(2),
            Self::ViewWindowSizeAutoFit => Some(3),
            _ => None,
        }
    }

    /// Inverse of [`Cmd::id`] for the WM_COMMAND dispatch (upstream's
    /// `_viv_command` switch default: unknown ids fall through untouched).
    pub(crate) fn from_id(id: u16) -> Option<Self> {
        if id == 0 || id > Self::COUNT as u16 {
            return None;
        }
        // The enum is field-free and starts at 0: the discriminant is the
        // index. Transmuting the arithmetic back through the enum keeps
        // from_id total without a hand-written match to drift out of sync.
        Some(Self::ALL[usize::from(id - 1)])
    }

    /// Every command in id order (the WM_INITMENU state application and the
    /// table tests walk this).
    pub(crate) const ALL: [Cmd; Cmd::COUNT] = [
        Self::FileOpenFile,
        Self::FileOpenFolder,
        Self::FileOpenEverythingSearch,
        Self::FileAddFile,
        Self::FileAddEverythingSearch,
        Self::FileOpenFileLocation,
        Self::FileEdit,
        Self::FilePreview,
        Self::FilePrint,
        Self::FileSetDesktopWallpaper,
        Self::FileClose,
        Self::FileDelete,
        Self::FileDeleteRecycle,
        Self::FileDeletePermanently,
        Self::FileRename,
        Self::FileProperties,
        Self::FileExit,
        Self::EditCut,
        Self::EditCopy,
        Self::EditCopyFilename,
        Self::EditCopyImage,
        Self::EditPaste,
        Self::EditRotate90,
        Self::EditRotate270,
        Self::EditCopyTo,
        Self::EditMoveTo,
        Self::ViewCaption,
        Self::ViewThickFrame,
        Self::ViewMenu,
        Self::ViewStatus,
        Self::ViewControls,
        Self::ViewPreset1,
        Self::ViewPreset2,
        Self::ViewPreset3,
        Self::ViewFullscreen,
        Self::ViewSlideshow,
        Self::ViewWindowSize50,
        Self::ViewWindowSize100,
        Self::ViewWindowSize200,
        Self::ViewWindowSizeAutoFit,
        Self::ViewRefresh,
        Self::ViewAllowShrinking,
        Self::ViewKeepAspect,
        Self::ViewFillWindow,
        Self::ViewOneToOne,
        Self::ViewBestFit,
        Self::ViewPanScanIncreaseSize,
        Self::ViewPanScanDecreaseSize,
        Self::ViewPanScanIncreaseWidth,
        Self::ViewPanScanDecreaseWidth,
        Self::ViewPanScanIncreaseHeight,
        Self::ViewPanScanDecreaseHeight,
        Self::ViewPanScanMoveUp,
        Self::ViewPanScanMoveDown,
        Self::ViewPanScanMoveLeft,
        Self::ViewPanScanMoveRight,
        Self::ViewPanScanMoveUpLeft,
        Self::ViewPanScanMoveUpRight,
        Self::ViewPanScanMoveDownLeft,
        Self::ViewPanScanMoveDownRight,
        Self::ViewPanScanMoveCenter,
        Self::ViewPanScanReset,
        Self::ViewZoomIn,
        Self::ViewZoomOut,
        Self::ViewZoomReset,
        Self::ViewOntopAlways,
        Self::ViewOntopWhilePlaying,
        Self::ViewOntopNever,
        Self::ViewOptions,
        Self::SlideshowPause,
        Self::SlideshowPlayOnly,
        Self::SlideshowPauseOnly,
        Self::SlideshowRateDecrease,
        Self::SlideshowRateIncrease,
        Self::SlideshowRate250,
        Self::SlideshowRate500,
        Self::SlideshowRate1000,
        Self::SlideshowRate2000,
        Self::SlideshowRate3000,
        Self::SlideshowRate4000,
        Self::SlideshowRate5000,
        Self::SlideshowRate6000,
        Self::SlideshowRate7000,
        Self::SlideshowRate8000,
        Self::SlideshowRate9000,
        Self::SlideshowRate10000,
        Self::SlideshowRate20000,
        Self::SlideshowRate30000,
        Self::SlideshowRate40000,
        Self::SlideshowRate50000,
        Self::SlideshowRate60000,
        Self::SlideshowRateCustom,
        Self::AnimationPlayPause,
        Self::AnimationJumpForwardMedium,
        Self::AnimationJumpBackwardMedium,
        Self::AnimationJumpForwardShort,
        Self::AnimationJumpBackwardShort,
        Self::AnimationJumpForwardLong,
        Self::AnimationJumpBackwardLong,
        Self::AnimationFrameStep,
        Self::AnimationFramePrev,
        Self::AnimationFirstFrame,
        Self::AnimationLastFrame,
        Self::AnimationRateDecrease,
        Self::AnimationRateIncrease,
        Self::AnimationRateReset,
        Self::NavNext,
        Self::NavPrev,
        Self::NavHome,
        Self::NavEnd,
        Self::NavSortName,
        Self::NavSortFullPath,
        Self::NavSortSize,
        Self::NavSortDateModified,
        Self::NavSortDateCreated,
        Self::NavSortAscending,
        Self::NavSortDescending,
        Self::NavShuffle,
        Self::NavJumpTo,
        Self::HelpCommandLineOptions,
        Self::HelpAbout,
    ];
}

/// A menu slot — the implemented subset of upstream's `_VIV_MENU_*` enum
/// (viv.c:318-334). Root is the menu bar itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Root,
    File,
    /// The Edit top-level menu (#41; upstream `_VIV_MENU_EDIT`, viv.c:824
    /// — between File and View in the root order).
    Edit,
    View,
    /// The View → Preset popup (#46; upstream `_VIV_MENU_VIEW_PRESET`,
    /// viv.c:848 — between Controls and the fullscreen separator).
    ViewPreset,
    /// The View → Window Size popup (#46; upstream
    /// `_VIV_MENU_VIEW_WINDOW_SIZE`, viv.c:855 — between Slideshow and
    /// Refresh).
    ViewWindowSize,
    /// The View → Pan/Scan popup (#44; upstream `_VIV_MENU_VIEW_PANSCAN`,
    /// viv.c:870 — between Best Fit and Zoom).
    ViewPanScan,
    ViewZoom,
    /// The View → On Top popup (#46; upstream `_VIV_MENU_VIEW_ONTOP`,
    /// viv.c:889 — after the Zoom popup's separator, before Options).
    ViewOntop,
    /// The Slideshow top-level menu (#37; upstream `_VIV_MENU_SLIDESHOW`,
    /// viv.c:892 — between View and Animation in the root order).
    Slideshow,
    /// The Slideshow → Rate popup (#37; upstream `_VIV_MENU_SLIDESHOW_RATE`,
    /// viv.c:897).
    SlideshowRate,
    /// The Animation top-level menu (#38; upstream `_VIV_MENU_ANIMATION`,
    /// viv.c:920 — between Slideshow and Navigate in the root order).
    Animation,
    Navigate,
    /// The Navigate → Sort popup (#39; upstream `_VIV_MENU_NAVIGATE_SORT`,
    /// viv.c:945).
    NavigateSort,
    Help,
}

impl Slot {
    /// Variant count; indexes the Win32 half's per-slot HMENU array.
    pub(crate) const COUNT: usize = Self::Help as usize + 1;
}

/// One keyboard binding (upstream `config_key_t`'s `WORD key` with the
/// parts split; the wire/in form lives in [`crate::keys`]). The menu
/// label shows the command's FIRST registered binding (viv.c:12354-12366).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyDef {
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) shift: bool,
    /// Win32 virtual-key code (ABI-stable values, WinUser.h).
    pub(crate) vk: u16,
}

/// One row of the menu table (upstream `_viv_command_t`, viv.c:408-413):
/// what to append and into which parent slot. The enum shape makes the
/// invalid upstream combos (a localization id on a separator) unrepresentable.
/// Accelerator labels are NOT part of the row — they come from the live
/// [`crate::keys::KeyMap`] at build time (#25; upstream
/// `_viv_key_list->start[...]`, viv.c:12365-12367).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Entry {
    Separator {
        parent: Slot,
    },
    Popup {
        loc: loc::Id,
        parent: Slot,
        /// The slot this popup introduces for its children.
        slot: Slot,
    },
    Item {
        loc: loc::Id,
        parent: Slot,
        cmd: Cmd,
    },
    /// An MF_OWNERDRAW row (#38; upstream's short/long jumps, viv.c:925-928):
    /// a real command-table entry — custom-shortcut-bindable, listed on the
    /// Controls page (upstream's list skips only POPUP/SEPARATOR/DELETE
    /// rows, viv.c:8269-8290) and dispatched by id — that the MENU BAR
    /// build skips (upstream `_viv_create_menu`'s MF_OWNERDRAW filter,
    /// viv.c:12328). Its localization id names the command for the ini
    /// key and the Controls list.
    HiddenItem {
        loc: loc::Id,
        parent: Slot,
        cmd: Cmd,
    },
}

/// The command table (upstream `_viv_commands[]`, viv.c:798-965, pruned to
/// riviv's implemented commands; the unimplemented menus — Edit and the
/// dead rows inside File/View/Navigate/Help — wait for their features).
/// Order is upstream order.
pub(crate) const ENTRIES: &[Entry] = &[
    // File (viv.c:800-821).
    Entry::Popup {
        loc: loc::Id::MenuFile,
        parent: Slot::Root,
        slot: Slot::File,
    },
    Entry::Item {
        loc: loc::Id::MenuOpenFile,
        parent: Slot::File,
        cmd: Cmd::FileOpenFile,
    },
    Entry::Item {
        loc: loc::Id::MenuOpenFolder,
        parent: Slot::File,
        cmd: Cmd::FileOpenFolder,
    },
    // Upstream's Everything rows (viv.c:804/807, after Open Folder and Add
    // File, skipping the unimplemented Add Folder between them) — #22.
    // Upstream MF_OWNERDRAW-hides both from the menu (viv.c:12328 skips
    // them); riviv shows implemented commands (the Add File precedent).
    Entry::Item {
        loc: loc::Id::MenuOpenEverythingSearch,
        parent: Slot::File,
        cmd: Cmd::FileOpenEverythingSearch,
    },
    // Upstream slots Add File after the open rows (viv.c:805, after the
    // Everything row).
    Entry::Item {
        loc: loc::Id::MenuAddFile,
        parent: Slot::File,
        cmd: Cmd::FileAddFile,
    },
    Entry::Item {
        loc: loc::Id::MenuAddEverythingSearch,
        parent: Slot::File,
        cmd: Cmd::FileAddEverythingSearch,
    },
    Entry::Separator { parent: Slot::File },
    // The #42 shell verb block (viv.c:811-816): Location/Edit/Preview/
    // Print/Wallpaper visible; Close hidden (MF_OWNERDRAW upstream —
    // keyboard Ctrl+W only, the Copy Filename precedent).
    Entry::Item {
        loc: loc::Id::MenuOpenFileLocation,
        parent: Slot::File,
        cmd: Cmd::FileOpenFileLocation,
    },
    Entry::Item {
        loc: loc::Id::MenuFileEdit,
        parent: Slot::File,
        cmd: Cmd::FileEdit,
    },
    Entry::Item {
        loc: loc::Id::MenuPreview,
        parent: Slot::File,
        cmd: Cmd::FilePreview,
    },
    Entry::Item {
        loc: loc::Id::MenuPrint,
        parent: Slot::File,
        cmd: Cmd::FilePrint,
    },
    Entry::Item {
        loc: loc::Id::MenuSetDesktopWallpaper,
        parent: Slot::File,
        cmd: Cmd::FileSetDesktopWallpaper,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuClose,
        parent: Slot::File,
        cmd: Cmd::FileClose,
    },
    // Upstream separates Close from the delete/rename block (viv.c:816-
    // 820); #43's rows land here, with Properties past the block — the
    // separators survive the pruned middle (the Add tail's precedent in
    // reverse).
    Entry::Separator { parent: Slot::File },
    // The delete/rename quartet (#43; viv.c:817-820): Delete VISIBLE (the
    // live Shift probe at dispatch), the two explicit deletes hidden
    // (MF_OWNERDRAW upstream — they live on Del / Shift+Del like the #41
    // hidden rows), Rename visible.
    Entry::Item {
        loc: loc::Id::MenuDelete,
        parent: Slot::File,
        cmd: Cmd::FileDelete,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuDeleteRecycle,
        parent: Slot::File,
        cmd: Cmd::FileDeleteRecycle,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuDeletePermanently,
        parent: Slot::File,
        cmd: Cmd::FileDeletePermanently,
    },
    Entry::Item {
        loc: loc::Id::MenuRename,
        parent: Slot::File,
        cmd: Cmd::FileRename,
    },
    Entry::Item {
        loc: loc::Id::MenuProperties,
        parent: Slot::File,
        cmd: Cmd::FileProperties,
    },
    Entry::Separator { parent: Slot::File },
    Entry::Item {
        loc: loc::Id::MenuExit,
        parent: Slot::File,
        cmd: Cmd::FileExit,
    },
    // Edit (viv.c:824-830) — #41: the clipboard family. Copy Filename and
    // Paste are MF_OWNERDRAW upstream (keyboard Ctrl+Shift+C / Ctrl+V
    // only); the rotate/copy-to rows below them (viv.c:832-835+) wait for
    // #43, so the menu ends after Paste (upstream's trailing separators
    // drop with their unimplemented tails).
    Entry::Popup {
        loc: loc::Id::MenuEdit,
        parent: Slot::Root,
        slot: Slot::Edit,
    },
    Entry::Item {
        loc: loc::Id::MenuCut,
        parent: Slot::Edit,
        cmd: Cmd::EditCut,
    },
    Entry::Item {
        loc: loc::Id::MenuCopy,
        parent: Slot::Edit,
        cmd: Cmd::EditCopy,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuCopyFilename,
        parent: Slot::Edit,
        cmd: Cmd::EditCopyFilename,
    },
    Entry::Item {
        loc: loc::Id::MenuCopyImage,
        parent: Slot::Edit,
        cmd: Cmd::EditCopyImage,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuPaste,
        parent: Slot::Edit,
        cmd: Cmd::EditPaste,
    },
    // The #43 tail (viv.c:831-837): a separator, the two rotations,
    // another separator, Copy To / Move To — all visible.
    Entry::Separator { parent: Slot::Edit },
    Entry::Item {
        loc: loc::Id::MenuRotateClockwise,
        parent: Slot::Edit,
        cmd: Cmd::EditRotate90,
    },
    Entry::Item {
        loc: loc::Id::MenuRotateCounterclockwise,
        parent: Slot::Edit,
        cmd: Cmd::EditRotate270,
    },
    Entry::Separator { parent: Slot::Edit },
    Entry::Item {
        loc: loc::Id::MenuCopyTo,
        parent: Slot::Edit,
        cmd: Cmd::EditCopyTo,
    },
    Entry::Item {
        loc: loc::Id::MenuMoveTo,
        parent: Slot::Edit,
        cmd: Cmd::EditMoveTo,
    },
    // View (viv.c:839-935): the five chrome toggles (Caption/Frame
    // MF_OWNERDRAW = menu-hidden upstream), the Preset popup, fullscreen/
    // slideshow, the Window Size popup, Refresh, the three fit rows,
    // 1:1 / Best Fit, the Pan/Scan and Zoom popups, the On Top popup and
    // Options last — upstream's full order.
    Entry::Popup {
        loc: loc::Id::MenuView,
        parent: Slot::Root,
        slot: Slot::View,
    },
    // The two style-bit toggles (#46; viv.c:843-844): MF_OWNERDRAW upstream
    // — hidden rows, ini/custom-binding reachable like the #38 jump family.
    Entry::HiddenItem {
        loc: loc::Id::MenuCaption,
        parent: Slot::View,
        cmd: Cmd::ViewCaption,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuThickFrame,
        parent: Slot::View,
        cmd: Cmd::ViewThickFrame,
    },
    Entry::Item {
        loc: loc::Id::MenuMenu,
        parent: Slot::View,
        cmd: Cmd::ViewMenu,
    },
    // View → Status Bar (#46; viv.c:846, between Menu and Controls).
    Entry::Item {
        loc: loc::Id::MenuStatusBar,
        parent: Slot::View,
        cmd: Cmd::ViewStatus,
    },
    // View → Controls (#45; upstream viv.c:847 after the Status Bar row).
    Entry::Item {
        loc: loc::Id::MenuControls,
        parent: Slot::View,
        cmd: Cmd::ViewControls,
    },
    // View → Preset (#46; viv.c:848-851): Minimal/Compact/Normal.
    Entry::Popup {
        loc: loc::Id::MenuPreset,
        parent: Slot::View,
        slot: Slot::ViewPreset,
    },
    Entry::Item {
        loc: loc::Id::MenuMinimal,
        parent: Slot::ViewPreset,
        cmd: Cmd::ViewPreset1,
    },
    Entry::Item {
        loc: loc::Id::MenuCompact,
        parent: Slot::ViewPreset,
        cmd: Cmd::ViewPreset2,
    },
    Entry::Item {
        loc: loc::Id::MenuNormal,
        parent: Slot::ViewPreset,
        cmd: Cmd::ViewPreset3,
    },
    Entry::Separator { parent: Slot::View },
    Entry::Item {
        loc: loc::Id::MenuFullscreen,
        parent: Slot::View,
        cmd: Cmd::ViewFullscreen,
    },
    // View → Slideshow (viv.c:854, directly after Fullscreen) — #37.
    Entry::Item {
        loc: loc::Id::MenuSlideshow,
        parent: Slot::View,
        cmd: Cmd::ViewSlideshow,
    },
    // View → Window Size (#46; viv.c:855-859): the four sizing rows.
    Entry::Popup {
        loc: loc::Id::MenuWindowSize,
        parent: Slot::View,
        slot: Slot::ViewWindowSize,
    },
    Entry::Item {
        loc: loc::Id::MenuWindowSize50,
        parent: Slot::ViewWindowSize,
        cmd: Cmd::ViewWindowSize50,
    },
    Entry::Item {
        loc: loc::Id::MenuWindowSize100,
        parent: Slot::ViewWindowSize,
        cmd: Cmd::ViewWindowSize100,
    },
    Entry::Item {
        loc: loc::Id::MenuWindowSize200,
        parent: Slot::ViewWindowSize,
        cmd: Cmd::ViewWindowSize200,
    },
    Entry::Item {
        loc: loc::Id::MenuWindowSizeAutoFit,
        parent: Slot::ViewWindowSize,
        cmd: Cmd::ViewWindowSizeAutoFit,
    },
    // View → Refresh (#46; viv.c:860, after the Window Size popup).
    Entry::Item {
        loc: loc::Id::MenuRefresh,
        parent: Slot::View,
        cmd: Cmd::ViewRefresh,
    },
    Entry::Separator { parent: Slot::View },
    // The three fit rows (#46; viv.c:862-864 — the toggles behind
    // Allow Shrinking / Keep Aspect Ratio / Fill Window).
    Entry::Item {
        loc: loc::Id::MenuAllowShrinking,
        parent: Slot::View,
        cmd: Cmd::ViewAllowShrinking,
    },
    Entry::Item {
        loc: loc::Id::MenuKeepAspectRatio,
        parent: Slot::View,
        cmd: Cmd::ViewKeepAspect,
    },
    Entry::Item {
        loc: loc::Id::MenuFillWindow,
        parent: Slot::View,
        cmd: Cmd::ViewFillWindow,
    },
    Entry::Item {
        loc: loc::Id::MenuOneToOne,
        parent: Slot::View,
        cmd: Cmd::ViewOneToOne,
    },
    Entry::Item {
        loc: loc::Id::MenuBestFit,
        parent: Slot::View,
        cmd: Cmd::ViewBestFit,
    },
    // Upstream's separator between Best Fit and the Pan/Scan popup
    // (viv.c:867) — restored with #46's full View rebuild.
    Entry::Separator { parent: Slot::View },
    // View → Pan/Scan (#44; upstream viv.c:870-896): the popup after Best
    // Fit, six size steps, a separator, the move family (the four
    // diagonals MF_OWNERDRAW = menu-hidden, viv.c:881-884), a separator,
    // then Reset. Upstream's table interleaves the Zoom popup between the
    // size and move halves, but the built menus are the same either way.
    Entry::Popup {
        loc: loc::Id::MenuPanScan,
        parent: Slot::View,
        slot: Slot::ViewPanScan,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanIncreaseSize,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanIncreaseSize,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanDecreaseSize,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanDecreaseSize,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanIncreaseWidth,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanIncreaseWidth,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanDecreaseWidth,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanDecreaseWidth,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanIncreaseHeight,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanIncreaseHeight,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanDecreaseHeight,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanDecreaseHeight,
    },
    Entry::Separator {
        parent: Slot::ViewPanScan,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanMoveUp,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveUp,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanMoveDown,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveDown,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanMoveLeft,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveLeft,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanMoveRight,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveRight,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuPanScanMoveUpLeft,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveUpLeft,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuPanScanMoveUpRight,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveUpRight,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuPanScanMoveDownLeft,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveDownLeft,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuPanScanMoveDownRight,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveDownRight,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanMoveCenter,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanMoveCenter,
    },
    Entry::Separator {
        parent: Slot::ViewPanScan,
    },
    Entry::Item {
        loc: loc::Id::MenuPanScanReset,
        parent: Slot::ViewPanScan,
        cmd: Cmd::ViewPanScanReset,
    },
    Entry::Popup {
        loc: loc::Id::MenuZoom,
        parent: Slot::View,
        slot: Slot::ViewZoom,
    },
    Entry::Item {
        loc: loc::Id::MenuZoomIn,
        parent: Slot::ViewZoom,
        cmd: Cmd::ViewZoomIn,
    },
    Entry::Item {
        loc: loc::Id::MenuZoomOut,
        parent: Slot::ViewZoom,
        cmd: Cmd::ViewZoomOut,
    },
    Entry::Item {
        loc: loc::Id::MenuZoomReset,
        parent: Slot::ViewZoom,
        cmd: Cmd::ViewZoomReset,
    },
    Entry::Separator { parent: Slot::View },
    // View → On Top (#46; viv.c:889-892): the three radio rows after the
    // Zoom popup's separator, before Options.
    Entry::Popup {
        loc: loc::Id::MenuOnTop,
        parent: Slot::View,
        slot: Slot::ViewOntop,
    },
    Entry::Item {
        loc: loc::Id::MenuAlways,
        parent: Slot::ViewOntop,
        cmd: Cmd::ViewOntopAlways,
    },
    Entry::Item {
        loc: loc::Id::MenuWhilePlaying,
        parent: Slot::ViewOntop,
        cmd: Cmd::ViewOntopWhilePlaying,
    },
    Entry::Item {
        loc: loc::Id::MenuNever,
        parent: Slot::ViewOntop,
        cmd: Cmd::ViewOntopNever,
    },
    Entry::Item {
        loc: loc::Id::MenuOptions,
        parent: Slot::View,
        cmd: Cmd::ViewOptions,
    },
    // Slideshow (#37; upstream viv.c:892-918 — a root menu between View
    // and Navigate, its Rate submenu after Play/Pause and a separator).
    Entry::Popup {
        loc: loc::Id::MenuSlideshowMenu,
        parent: Slot::Root,
        slot: Slot::Slideshow,
    },
    Entry::Item {
        loc: loc::Id::MenuSlideshowPlayPause,
        parent: Slot::Slideshow,
        cmd: Cmd::SlideshowPause,
    },
    // The toolbar-only pair (#45; upstream viv.h:132-133 keeps them out
    // of `_viv_commands[]` — riviv registers them as hidden rows so the
    // table invariants hold, making them additionally bindable).
    Entry::HiddenItem {
        loc: loc::Id::ToolbarPlaySlideshow,
        parent: Slot::Slideshow,
        cmd: Cmd::SlideshowPlayOnly,
    },
    Entry::HiddenItem {
        loc: loc::Id::ToolbarPauseSlideshow,
        parent: Slot::Slideshow,
        cmd: Cmd::SlideshowPauseOnly,
    },
    Entry::Separator {
        parent: Slot::Slideshow,
    },
    Entry::Popup {
        loc: loc::Id::MenuSlideshowRate,
        parent: Slot::Slideshow,
        slot: Slot::SlideshowRate,
    },
    Entry::Item {
        loc: loc::Id::MenuSlideshowRateDecrease,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRateDecrease,
    },
    Entry::Item {
        loc: loc::Id::MenuSlideshowRateIncrease,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRateIncrease,
    },
    Entry::Separator {
        parent: Slot::SlideshowRate,
    },
    Entry::Item {
        loc: loc::Id::MenuRate250Milliseconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate250,
    },
    Entry::Item {
        loc: loc::Id::MenuRate500Milliseconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate500,
    },
    Entry::Item {
        loc: loc::Id::MenuRate1Second,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate1000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate2Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate2000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate3Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate3000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate4Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate4000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate5Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate5000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate6Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate6000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate7Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate7000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate8Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate8000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate9Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate9000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate10Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate10000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate20Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate20000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate30Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate30000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate40Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate40000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate50Seconds,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate50000,
    },
    Entry::Item {
        loc: loc::Id::MenuRate1Minute,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRate60000,
    },
    Entry::Item {
        loc: loc::Id::MenuRateCustom,
        parent: Slot::SlideshowRate,
        cmd: Cmd::SlideshowRateCustom,
    },
    // Animation (#38; upstream viv.c:920-937 — a root menu between
    // Slideshow and Navigate). The short/long jump quartet rides as
    // HiddenItem rows: command-table citizens, menu-bar invisible
    // (upstream's MF_OWNERDRAW, viv.c:925-928 vs the build filter at
    // viv.c:12328).
    Entry::Popup {
        loc: loc::Id::MenuAnimation,
        parent: Slot::Root,
        slot: Slot::Animation,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationPlayPause,
        parent: Slot::Animation,
        cmd: Cmd::AnimationPlayPause,
    },
    Entry::Separator {
        parent: Slot::Animation,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationJumpForward,
        parent: Slot::Animation,
        cmd: Cmd::AnimationJumpForwardMedium,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationJumpBackward,
        parent: Slot::Animation,
        cmd: Cmd::AnimationJumpBackwardMedium,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuAnimationShortJumpForward,
        parent: Slot::Animation,
        cmd: Cmd::AnimationJumpForwardShort,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuAnimationShortJumpBackward,
        parent: Slot::Animation,
        cmd: Cmd::AnimationJumpBackwardShort,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuAnimationLongJumpForward,
        parent: Slot::Animation,
        cmd: Cmd::AnimationJumpForwardLong,
    },
    Entry::HiddenItem {
        loc: loc::Id::MenuAnimationLongJumpBackward,
        parent: Slot::Animation,
        cmd: Cmd::AnimationJumpBackwardLong,
    },
    Entry::Separator {
        parent: Slot::Animation,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationFrameStep,
        parent: Slot::Animation,
        cmd: Cmd::AnimationFrameStep,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationPreviousFrame,
        parent: Slot::Animation,
        cmd: Cmd::AnimationFramePrev,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationFirstFrame,
        parent: Slot::Animation,
        cmd: Cmd::AnimationFirstFrame,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationLastFrame,
        parent: Slot::Animation,
        cmd: Cmd::AnimationLastFrame,
    },
    Entry::Separator {
        parent: Slot::Animation,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationRateDecrease,
        parent: Slot::Animation,
        cmd: Cmd::AnimationRateDecrease,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationRateIncrease,
        parent: Slot::Animation,
        cmd: Cmd::AnimationRateIncrease,
    },
    Entry::Item {
        loc: loc::Id::MenuAnimationRateReset,
        parent: Slot::Animation,
        cmd: Cmd::AnimationRateReset,
    },
    // Navigate (viv.c:952-959).
    Entry::Popup {
        loc: loc::Id::MenuNavigate,
        parent: Slot::Root,
        slot: Slot::Navigate,
    },
    Entry::Item {
        loc: loc::Id::MenuNext,
        parent: Slot::Navigate,
        cmd: Cmd::NavNext,
    },
    Entry::Item {
        loc: loc::Id::MenuPrevious,
        parent: Slot::Navigate,
        cmd: Cmd::NavPrev,
    },
    Entry::Item {
        loc: loc::Id::MenuHome,
        parent: Slot::Navigate,
        cmd: Cmd::NavHome,
    },
    Entry::Item {
        loc: loc::Id::MenuEnd,
        parent: Slot::Navigate,
        cmd: Cmd::NavEnd,
    },
    // #39 (viv.c:944-956): separator, the Sort popup (five mode radios +
    // separator + the direction pair), Shuffle, separator, Jump To.
    Entry::Separator {
        parent: Slot::Navigate,
    },
    Entry::Popup {
        loc: loc::Id::MenuSort,
        parent: Slot::Navigate,
        slot: Slot::NavigateSort,
    },
    Entry::Item {
        loc: loc::Id::MenuSortName,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortName,
    },
    Entry::Item {
        loc: loc::Id::MenuSortFullPath,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortFullPath,
    },
    Entry::Item {
        loc: loc::Id::MenuSortSize,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortSize,
    },
    Entry::Item {
        loc: loc::Id::MenuSortDateModified,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortDateModified,
    },
    Entry::Item {
        loc: loc::Id::MenuSortDateCreated,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortDateCreated,
    },
    Entry::Separator {
        parent: Slot::NavigateSort,
    },
    Entry::Item {
        loc: loc::Id::MenuSortAscending,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortAscending,
    },
    Entry::Item {
        loc: loc::Id::MenuSortDescending,
        parent: Slot::NavigateSort,
        cmd: Cmd::NavSortDescending,
    },
    Entry::Item {
        loc: loc::Id::MenuShuffle,
        parent: Slot::Navigate,
        cmd: Cmd::NavShuffle,
    },
    Entry::Separator {
        parent: Slot::Navigate,
    },
    Entry::Item {
        loc: loc::Id::MenuJumpTo,
        parent: Slot::Navigate,
        cmd: Cmd::NavJumpTo,
    },
    // Help (viv.c:958-965): upstream precedes About with Help / website /
    // donate rows riviv does not ship; the command-line options row is the
    // one #48 takes, in its upstream slot between them and About.
    Entry::Popup {
        loc: loc::Id::MenuHelp,
        parent: Slot::Root,
        slot: Slot::Help,
    },
    Entry::Item {
        loc: loc::Id::MenuCommandLineOptions,
        parent: Slot::Help,
        cmd: Cmd::HelpCommandLineOptions,
    },
    Entry::Item {
        loc: loc::Id::MenuAbout,
        parent: Slot::Help,
        cmd: Cmd::HelpAbout,
    },
];

/// The accelerator label for a key (upstream `_viv_get_key_text`,
/// viv.c:12283-12301): modifiers in Ctrl → Alt → Shift order, then the key
/// name. `vk_text` is the shell's half — `GetKeyNameTextW`'s name for the
/// vk, layout-localized exactly like upstream (`_viv_vk_to_text`,
/// viv.c:12221-12261).
pub(crate) fn key_label(key: KeyDef, vk_text: &str) -> String {
    let mut label = String::new();
    if key.ctrl {
        label.push_str("Ctrl+");
    }
    if key.alt {
        label.push_str("Alt+");
    }
    if key.shift {
        label.push_str("Shift+");
    }
    label.push_str(vk_text);
    label
}

/// The menu item text with its accelerator (`_viv_create_menu`'s
/// `string_cat_utf8(text_wbuf, "\t")` + key text, viv.c:12359-12367): the
/// tab is what right-aligns the label into Windows' accelerator column.
pub(crate) fn item_text(label: &str, key: Option<&str>) -> String {
    match key {
        Some(k) => format!("{label}\t{k}"),
        None => label.to_string(),
    }
}

/// The command whose first binding names a row's accelerator: upstream
/// remaps the visible Delete row onto the recycle-delete key — the visible
/// row itself carries no default binding (Del belongs to the hidden
/// recycle row) — in BOTH the menu-bar build (viv.c:12351-12361) and the
/// context-menu walk (viv.c:3454-3461).
pub(crate) fn hint_cmd(cmd: Cmd) -> Cmd {
    if cmd == Cmd::FileDelete {
        Cmd::FileDeleteRecycle
    } else {
        cmd
    }
}

/// One entry of the context-menu table (#49; upstream's flat
/// `_viv_context_menu_items[]`, viv.c:1053-1123 — a WORD array whose three
/// value kinds encode commands, submenu markers and separators: a fresh
/// `_VIV_MENU_*` marker pushes a submenu, its repeat pops back).
#[derive(Debug, Clone, Copy)]
pub(crate) enum ContextEntry {
    /// A command row (upstream: an id above `_VIV_MENU_COUNT`).
    Command(Cmd),
    /// Push a submenu (upstream: a `_VIV_MENU_*` marker seen fresh; the
    /// label override for the Rate popup lives here, viv.c:3498-3507 —
    /// Sort falls through to its own command-table string).
    PushPopup { slot: Slot, label: loc::Id },
    /// Pop back to the top level (upstream viv.c:3482-3487 — this arm
    /// switches `curmenu` and touches NOTHING else, not the separator
    /// dedupe state).
    PopPopup,
    /// A separator (upstream: 0).
    Separator,
}

/// The context-menu table, a verbatim port of upstream's
/// `_viv_context_menu_items[]` (viv.c:1053-1123) in table order. Only the
/// walk ([`context_rows`]) applies gates: Preview (Win8+ baseline,
/// viv.c:3418-3423) and the Menu recovery row (bar visible, viv.c:3427).
/// Upstream comments the slideshow row out (viv.c:1090) — absent here too.
pub(crate) const CONTEXT_TABLE: &[ContextEntry] = &[
    ContextEntry::Command(Cmd::NavNext),
    ContextEntry::Command(Cmd::NavPrev),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::ViewFullscreen),
    ContextEntry::Command(Cmd::SlideshowPause),
    ContextEntry::PushPopup {
        slot: Slot::SlideshowRate,
        label: loc::Id::MenuSlideshowRate,
    },
    ContextEntry::Command(Cmd::SlideshowRateDecrease),
    ContextEntry::Command(Cmd::SlideshowRateIncrease),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::SlideshowRate250),
    ContextEntry::Command(Cmd::SlideshowRate500),
    ContextEntry::Command(Cmd::SlideshowRate1000),
    ContextEntry::Command(Cmd::SlideshowRate2000),
    ContextEntry::Command(Cmd::SlideshowRate3000),
    ContextEntry::Command(Cmd::SlideshowRate4000),
    ContextEntry::Command(Cmd::SlideshowRate5000),
    ContextEntry::Command(Cmd::SlideshowRate6000),
    ContextEntry::Command(Cmd::SlideshowRate7000),
    ContextEntry::Command(Cmd::SlideshowRate8000),
    ContextEntry::Command(Cmd::SlideshowRate9000),
    ContextEntry::Command(Cmd::SlideshowRate10000),
    ContextEntry::Command(Cmd::SlideshowRate20000),
    ContextEntry::Command(Cmd::SlideshowRate30000),
    ContextEntry::Command(Cmd::SlideshowRate40000),
    ContextEntry::Command(Cmd::SlideshowRate50000),
    ContextEntry::Command(Cmd::SlideshowRate60000),
    ContextEntry::Command(Cmd::SlideshowRateCustom),
    ContextEntry::PopPopup,
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::ViewMenu),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::ViewAllowShrinking),
    ContextEntry::Command(Cmd::ViewKeepAspect),
    ContextEntry::Command(Cmd::ViewFillWindow),
    ContextEntry::Command(Cmd::ViewOneToOne),
    ContextEntry::Separator,
    ContextEntry::PushPopup {
        slot: Slot::NavigateSort,
        label: loc::Id::MenuSort,
    },
    ContextEntry::Command(Cmd::NavSortName),
    ContextEntry::Command(Cmd::NavSortFullPath),
    ContextEntry::Command(Cmd::NavSortSize),
    ContextEntry::Command(Cmd::NavSortDateModified),
    ContextEntry::Command(Cmd::NavSortDateCreated),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::NavSortAscending),
    ContextEntry::Command(Cmd::NavSortDescending),
    ContextEntry::PopPopup,
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::FileOpenFileLocation),
    ContextEntry::Command(Cmd::FileSetDesktopWallpaper),
    ContextEntry::Command(Cmd::FileEdit),
    ContextEntry::Command(Cmd::FilePrint),
    // No FilePreview row: upstream skips it on Windows 8+ (viv.c:3418-3423
    // — "this doesn't exist on Windows 8 or later"); riviv's baseline is
    // Win8+, so the walk drops it unconditionally.
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::EditRotate90),
    ContextEntry::Command(Cmd::EditRotate270),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::EditCut),
    ContextEntry::Command(Cmd::EditCopy),
    ContextEntry::Command(Cmd::EditCopyImage),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::FileDelete),
    ContextEntry::Command(Cmd::FileRename),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::FileProperties),
    ContextEntry::Command(Cmd::ViewOptions),
    ContextEntry::Separator,
    ContextEntry::Command(Cmd::FileExit),
];

/// One built row of the context menu — the flattened walk of
/// [`CONTEXT_TABLE`] (the output of upstream's build loop, viv.c:3400-3527;
/// the window shell replays Push/Pop to switch the append target exactly
/// like upstream's `curmenu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextRow {
    Command { cmd: Cmd, label: loc::Id },
    Popup { slot: Slot, label: loc::Id },
    Pop,
    Separator,
}

/// The caption a context-menu command row carries: upstream copies the
/// command's own localization string, except the slideshow-pause row takes
/// the Play/Pause string (viv.c:3442-3452).
pub(crate) fn context_label(cmd: Cmd) -> loc::Id {
    if cmd == Cmd::SlideshowPause {
        return loc::Id::MenuSlideshowPlayPause;
    }
    command_loc(cmd)
}

/// The localization id of a command's table row (upstream's
/// `_viv_command_index_from_command_id` + the row's localization id — every
/// command the context table names has exactly one ENTRIES row, the
/// registration invariant `item_commands_are_unique` pins).
fn command_loc(cmd: Cmd) -> loc::Id {
    for entry in ENTRIES {
        if let Entry::Item { loc, cmd: c, .. } | Entry::HiddenItem { loc, cmd: c, .. } = *entry
            && c == cmd
        {
            return loc;
        }
    }
    unreachable!("every Cmd variant has an ENTRIES row (tested invariant)")
}

/// Walk [`CONTEXT_TABLE`] into the rows the context menu shows under a
/// config (upstream's build loop, viv.c:3400-3527): the Menu recovery row
/// drops while the bar shows (viv.c:3427), the Preview row never ships on
/// the Win8+ baseline (viv.c:3418-3423), and separators dedupe through one
/// `was_separator` state — init true (a table-leading separator would be
/// eaten), cleared by every command/popup append, set by a separator
/// append, untouched by skipped rows AND by pops; ONE state shared across
/// submenu levels, exactly upstream. The dropped Menu row's two neighboring
/// separators therefore merge into one.
pub(crate) fn context_rows(show_menu: bool) -> Vec<ContextRow> {
    let mut rows = Vec::new();
    let mut was_separator = true;
    for entry in CONTEXT_TABLE {
        match *entry {
            ContextEntry::Command(cmd) => {
                if cmd == Cmd::FilePreview {
                    continue; // Win8+ never ships the row (viv.c:3418-3423)
                }
                if cmd == Cmd::ViewMenu && show_menu {
                    continue; // bar visible: no recovery row (viv.c:3427)
                }
                rows.push(ContextRow::Command {
                    cmd,
                    label: context_label(cmd),
                });
                was_separator = false;
            }
            ContextEntry::PushPopup { slot, label } => {
                rows.push(ContextRow::Popup { slot, label });
                was_separator = false;
            }
            ContextEntry::PopPopup => rows.push(ContextRow::Pop),
            ContextEntry::Separator => {
                if !was_separator {
                    rows.push(ContextRow::Separator);
                    was_separator = true;
                }
            }
        }
    }
    rows
}

/// The dynamic menu state read when a menu is about to open (the inputs of
/// upstream `_viv_check_menus`' CheckMenuItem calls, viv.c:7123-7132 —
/// upstream's EnableMenuItem list is copy/delete/print-style commands
/// riviv does not register, so the only enable decision here is the
/// Options placeholder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuState {
    /// View → Menu's checkmark: `config_show_menu` (viv.c:7125).
    pub(crate) show_menu: bool,
    /// View → Status Bar's checkmark (#46): `config_show_status`
    /// (viv.c:7128).
    pub(crate) show_status: bool,
    /// View → Controls' checkmark (#45): `config_show_controls`
    /// (viv.c:7126).
    pub(crate) show_controls: bool,
    /// View → Fullscreen's checkmark: `_viv_is_fullscreen` (viv.c:7132).
    pub(crate) fullscreen: bool,
    /// View → 1:1's checkmark: render size == image size (viv.c:7131).
    /// False when nothing is displayed — a blank viewer has no 1:1 state
    /// (upstream's raw size compare degenerates to 0 == 0 there; riviv
    /// guards it).
    pub(crate) one_to_one: bool,
    /// The slideshow running flag (#37; upstream viv.c:7133/7138 — BOTH
    /// View → Slideshow and Slideshow → Play/Pause carry the check).
    pub(crate) slideshow: bool,
    /// The current rate in ms (#37; the Rate submenu's radio — the preset
    /// row whose value matches, or Custom when none does, viv.c:7140-7189).
    pub(crate) slideshow_rate_ms: u32,
    /// The animation playing flag (#38; upstream viv.c:7184 — Play/Pause
    /// CHECKES while playing, the pause being the unchecked state).
    pub(crate) animation_playing: bool,
    /// The navigation sort config (#39; upstream viv.c:7188-7192 — the
    /// five mode radios check on the equal mode; an Unknown config value
    /// checks none).
    pub(crate) nav_sort: crate::playlist::SortMode,
    /// The sort direction (#39; upstream viv.c:7194-7195 — Ascending /
    /// Descending check on the flag).
    pub(crate) nav_sort_ascending: bool,
    /// The shuffle flag (#39; upstream viv.c:7186 — a plain check).
    pub(crate) shuffle: bool,
    /// Whether an image is currently shown and loadable (#41; upstream's
    /// `is_image_enabled` for the EnableMenuItem family, viv.c:7103 — the
    /// current file is set and neither a not-found nor a failed verdict
    /// stands). Gates the clipboard quartet; Paste stays ungated (upstream
    /// has no EnableMenuItem row for it).
    pub(crate) image_enabled: bool,
    /// The Allow Shrinking / Keep Aspect / Fill Window trio's config
    /// snapshot (#46; upstream viv.c:7127-7129): the Fill row reads the
    /// fullscreen or windowed `fill_window` flag per the CURRENT mode
    /// (upstream's `_viv_check_menus` quirk, viv.c:7098-7100).
    pub(crate) allow_shrinking: bool,
    pub(crate) keep_aspect: bool,
    pub(crate) fill_window: bool,
    /// The on-top mode (#46; upstream viv.c:7134-7136 — the three radio
    /// rows check on 1 / 2 / 0).
    pub(crate) ontop: i32,
}

/// Whether `cmd`'s menu item carries a check in `state` (upstream
/// `_viv_check_menus`, viv.c:7123-7132 — only toggle-ish commands do).
pub(crate) fn checked(cmd: Cmd, state: &MenuState) -> bool {
    match cmd {
        Cmd::ViewMenu => state.show_menu,
        Cmd::ViewStatus => state.show_status,
        Cmd::ViewControls => state.show_controls,
        Cmd::ViewFullscreen => state.fullscreen,
        Cmd::ViewOneToOne => state.one_to_one,
        Cmd::ViewSlideshow | Cmd::SlideshowPause => state.slideshow,
        Cmd::AnimationPlayPause => state.animation_playing,
        // The #46 fit trio (viv.c:7127-7129) and the on-top radios
        // (viv.c:7134-7136 — checked on the exact config value).
        Cmd::ViewAllowShrinking => state.allow_shrinking,
        Cmd::ViewKeepAspect => state.keep_aspect,
        Cmd::ViewFillWindow => state.fill_window,
        Cmd::ViewOntopAlways => state.ontop == 1,
        Cmd::ViewOntopWhilePlaying => state.ontop == 2,
        Cmd::ViewOntopNever => state.ontop == 0,
        // The sort radios (#39): the equal mode checks; an Unknown config
        // value checks none of the five (viv.c:7188-7192). The direction
        // pair checks on the flag (viv.c:7194-7195); Shuffle is a plain
        // check on the flag (viv.c:7186).
        cmd => {
            if let Some(mode) = cmd.sort_mode() {
                state.nav_sort == mode
            } else {
                match cmd {
                    Cmd::NavSortAscending => state.nav_sort_ascending,
                    Cmd::NavSortDescending => !state.nav_sort_ascending,
                    Cmd::NavShuffle => state.shuffle,
                    // The radio's checked row: the preset that equals the
                    // rate, or Custom when the rate is no preset (viv.c:
                    // 7140-7189's switch default).
                    Cmd::SlideshowRateCustom => {
                        !crate::slideshow::is_preset(state.slideshow_rate_ms)
                    }
                    other => other.slideshow_rate_ms() == Some(state.slideshow_rate_ms),
                }
            }
        }
    }
}

/// Whether `cmd`'s check renders as a radio dot (upstream passes
/// `MFT_RADIOCHECK` in the CheckMenuItem flags for the whole Rate submenu
/// family, viv.c:7165-7189, and the whole Sort submenu family, viv.c:7188-
/// 7195 — a display trait, not a state).
pub(crate) fn radio(cmd: Cmd) -> bool {
    cmd.slideshow_rate_ms().is_some()
        || cmd == Cmd::SlideshowRateCustom
        || cmd.sort_mode().is_some()
        || matches!(
            cmd,
            Cmd::NavSortAscending
                | Cmd::NavSortDescending
                | Cmd::ViewOntopAlways
                | Cmd::ViewOntopWhilePlaying
                | Cmd::ViewOntopNever
        )
}

/// Whether `cmd`'s menu item is selectable. Everything riviv registers is
/// always available (upstream never gates zoom/navigation on image state
/// either — its EnableMenuItem list, viv.c:7103-7121, covers clipboard/
/// delete/print/shell commands).
pub(crate) fn enabled(cmd: Cmd, state: &MenuState) -> bool {
    match cmd {
        // The clipboard quartet (#41), the #42 shell septet, and #43's
        // six visible file-management rows: upstream grays them all
        // through the same `is_image_enabled` gate (viv.c:7104-7108 +
        // 7114-7121). Paste is not in that list — an empty clipboard just
        // makes the handler a no-op — and neither are the two hidden
        // delete rows (upstream never grays them; their handlers' bare
        // current-file guard is the only defense, viv.c:7202).
        Cmd::EditCut
        | Cmd::EditCopy
        | Cmd::EditCopyFilename
        | Cmd::EditCopyImage
        | Cmd::FileOpenFileLocation
        | Cmd::FileEdit
        | Cmd::FilePreview
        | Cmd::FilePrint
        | Cmd::FileSetDesktopWallpaper
        | Cmd::FileClose
        | Cmd::FileDelete
        | Cmd::FileRename
        | Cmd::FileProperties
        | Cmd::EditRotate90
        | Cmd::EditRotate270
        | Cmd::EditCopyTo
        | Cmd::EditMoveTo => state.image_enabled,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's menu build (viv.c:12318-12343) walks the table with the
    /// root bar pre-created and creates each popup on demand — an entry can
    /// only name a parent whose popup appeared EARLIER, and each slot is
    /// introduced exactly once. Violations would append into a null menu
    /// (the row vanishes) or double-parent a slot.
    #[test]
    fn every_parent_slot_is_introduced_exactly_once_and_before_use() {
        let mut seen = [false; Slot::COUNT];
        seen[Slot::Root as usize] = true;
        for entry in ENTRIES {
            let parent = match entry {
                Entry::Separator { parent } => *parent,
                Entry::Popup { parent, .. } => *parent,
                Entry::Item { parent, .. } => *parent,
                Entry::HiddenItem { parent, .. } => *parent,
            };
            assert!(
                seen[parent as usize],
                "entry {entry:?} parents slot {parent:?} before its popup"
            );
            if let Entry::Popup { slot, .. } = entry {
                assert!(!seen[*slot as usize], "slot {slot:?} introduced twice");
                seen[*slot as usize] = true;
            }
        }
        // Every slot the enum declares is actually reachable in the table.
        assert!(seen.iter().all(|&s| s), "declared slot never introduced");
    }

    #[test]
    fn item_commands_are_unique() {
        // WM_COMMAND dispatches by id; a duplicated command would be two
        // menu rows sharing one action id (upstream's table keeps them
        // distinct, viv.c:798-965). HiddenItem rows register their command
        // the same way — they only skip the menu bar, not the command
        // table.
        let mut seen = [false; Cmd::COUNT];
        for entry in ENTRIES {
            if let Entry::Item { cmd, .. } | Entry::HiddenItem { cmd, .. } = entry {
                assert!(!seen[usize::from(cmd.id() - 1)], "{cmd:?} registered twice");
                seen[usize::from(cmd.id() - 1)] = true;
            }
        }
        // Every command the enum declares has a table row.
        assert!(seen.iter().all(|&s| s), "declared command never registered");
    }

    #[test]
    fn wm_command_ids_round_trip() {
        for cmd in Cmd::ALL {
            assert_eq!(Cmd::from_id(cmd.id()), Some(cmd));
        }
        // 0 is the separator id in Win32 menus and past-the-end is garbage
        // from a foreign send — both dispatch to nothing (upstream's
        // switch default, viv.c:1670).
        assert_eq!(Cmd::from_id(0), None);
        assert_eq!(Cmd::from_id(Cmd::COUNT as u16 + 1), None);
        assert_eq!(Cmd::from_id(u16::MAX), None);
    }

    #[test]
    fn slideshow_and_animation_command_ids_are_pinned_for_the_wire() {
        // smoke37/smoke38 post these as raw WM_COMMAND wparams; inserting a
        // command ahead of the block would silently shift every wire id.
        // The #42 shell septet runs 6-12 between the Add rows and the
        // delete block (viv.c:811-820 table order); #43's four File rows
        // (delete trio + Rename, viv.c:817-820) push Properties onward +4.
        assert_eq!(Cmd::FileOpenFileLocation.id(), 6);
        assert_eq!(Cmd::FileEdit.id(), 7);
        assert_eq!(Cmd::FilePreview.id(), 8);
        assert_eq!(Cmd::FilePrint.id(), 9);
        assert_eq!(Cmd::FileSetDesktopWallpaper.id(), 10);
        assert_eq!(Cmd::FileClose.id(), 11);
        assert_eq!(Cmd::FileProperties.id(), 16);
        assert_eq!(Cmd::FileExit.id(), 17);
        // The #41 Edit block runs 18-22 (viv.c:826-830 table order), then
        // #43's rotate/copy-to quartet 23-26 (viv.c:831-837).
        assert_eq!(Cmd::EditCut.id(), 18);
        assert_eq!(Cmd::EditCopy.id(), 19);
        assert_eq!(Cmd::EditCopyFilename.id(), 20);
        assert_eq!(Cmd::EditCopyImage.id(), 21);
        assert_eq!(Cmd::EditPaste.id(), 22);
        // The #46 View head grows to upstream's full order (viv.c:843-864),
        // +8 past the #43 Edit quartet: hidden Caption/ThickFrame (27/28),
        // Menu 29, Status 30, Controls 31, the Preset trio 32-34,
        // Fullscreen 35, Slideshow 36, the Window Size quartet 37-40,
        // Refresh 41, the fit trio 42-44, 1:1 45 and Best Fit 46.
        assert_eq!(Cmd::ViewCaption.id(), 27);
        assert_eq!(Cmd::ViewThickFrame.id(), 28);
        assert_eq!(Cmd::ViewMenu.id(), 29);
        assert_eq!(Cmd::ViewStatus.id(), 30);
        assert_eq!(Cmd::ViewControls.id(), 31);
        assert_eq!(Cmd::ViewPreset1.id(), 32);
        assert_eq!(Cmd::ViewPreset2.id(), 33);
        assert_eq!(Cmd::ViewPreset3.id(), 34);
        assert_eq!(Cmd::ViewFullscreen.id(), 35);
        assert_eq!(Cmd::ViewSlideshow.id(), 36);
        assert_eq!(Cmd::ViewWindowSize50.id(), 37);
        assert_eq!(Cmd::ViewWindowSize100.id(), 38);
        assert_eq!(Cmd::ViewWindowSize200.id(), 39);
        assert_eq!(Cmd::ViewWindowSizeAutoFit.id(), 40);
        assert_eq!(Cmd::ViewRefresh.id(), 41);
        assert_eq!(Cmd::ViewAllowShrinking.id(), 42);
        assert_eq!(Cmd::ViewKeepAspect.id(), 43);
        assert_eq!(Cmd::ViewFillWindow.id(), 44);
        assert_eq!(Cmd::ViewOneToOne.id(), 45);
        assert_eq!(Cmd::ViewBestFit.id(), 46);
        // The #44 Pan/Scan block runs 47-62 (viv.h:110-125 order).
        assert_eq!(Cmd::ViewPanScanIncreaseSize.id(), 47);
        assert_eq!(Cmd::ViewPanScanDecreaseSize.id(), 48);
        assert_eq!(Cmd::ViewPanScanIncreaseWidth.id(), 49);
        assert_eq!(Cmd::ViewPanScanDecreaseWidth.id(), 50);
        assert_eq!(Cmd::ViewPanScanIncreaseHeight.id(), 51);
        assert_eq!(Cmd::ViewPanScanDecreaseHeight.id(), 52);
        assert_eq!(Cmd::ViewPanScanMoveUp.id(), 53);
        assert_eq!(Cmd::ViewPanScanMoveDown.id(), 54);
        assert_eq!(Cmd::ViewPanScanMoveLeft.id(), 55);
        assert_eq!(Cmd::ViewPanScanMoveRight.id(), 56);
        assert_eq!(Cmd::ViewPanScanMoveUpLeft.id(), 57);
        assert_eq!(Cmd::ViewPanScanMoveUpRight.id(), 58);
        assert_eq!(Cmd::ViewPanScanMoveDownLeft.id(), 59);
        assert_eq!(Cmd::ViewPanScanMoveDownRight.id(), 60);
        assert_eq!(Cmd::ViewPanScanMoveCenter.id(), 61);
        assert_eq!(Cmd::ViewPanScanReset.id(), 62);
        // Zoom trio 63-65, the #46 on-top trio 66-68, Options 69.
        assert_eq!(Cmd::ViewZoomIn.id(), 63);
        assert_eq!(Cmd::ViewZoomOut.id(), 64);
        assert_eq!(Cmd::ViewZoomReset.id(), 65);
        assert_eq!(Cmd::ViewOntopAlways.id(), 66);
        assert_eq!(Cmd::ViewOntopWhilePlaying.id(), 67);
        assert_eq!(Cmd::ViewOntopNever.id(), 68);
        assert_eq!(Cmd::ViewOptions.id(), 69);
        // The #45 toolbar-only pair: 71/72.
        assert_eq!(Cmd::SlideshowPause.id(), 70);
        assert_eq!(Cmd::SlideshowPlayOnly.id(), 71);
        assert_eq!(Cmd::SlideshowPauseOnly.id(), 72);
        assert_eq!(Cmd::SlideshowRateDecrease.id(), 73);
        assert_eq!(Cmd::SlideshowRateIncrease.id(), 74);
        assert_eq!(Cmd::SlideshowRate250.id(), 75);
        assert_eq!(Cmd::SlideshowRate500.id(), 76);
        assert_eq!(Cmd::SlideshowRateCustom.id(), 92);
        // The #38 Animation block lands 93-106 with the #41/#44/#45/#46/
        // #42/#43 shifts.
        assert_eq!(Cmd::AnimationPlayPause.id(), 93);
        assert_eq!(Cmd::AnimationJumpForwardMedium.id(), 94);
        assert_eq!(Cmd::AnimationJumpBackwardMedium.id(), 95);
        assert_eq!(Cmd::AnimationJumpForwardShort.id(), 96);
        assert_eq!(Cmd::AnimationJumpBackwardShort.id(), 97);
        assert_eq!(Cmd::AnimationJumpForwardLong.id(), 98);
        assert_eq!(Cmd::AnimationJumpBackwardLong.id(), 99);
        assert_eq!(Cmd::AnimationFrameStep.id(), 100);
        assert_eq!(Cmd::AnimationFramePrev.id(), 101);
        assert_eq!(Cmd::AnimationFirstFrame.id(), 102);
        assert_eq!(Cmd::AnimationLastFrame.id(), 103);
        assert_eq!(Cmd::AnimationRateDecrease.id(), 104);
        assert_eq!(Cmd::AnimationRateIncrease.id(), 105);
        assert_eq!(Cmd::AnimationRateReset.id(), 106);
        assert_eq!(Cmd::NavNext.id(), 107);
        // The #39 sort/shuffle/jumpto block (menu-table order,
        // viv.c:946-956); HelpCommandLineOptions lands 120, HelpAbout
        // 121.
        assert_eq!(Cmd::NavSortName.id(), 111);
        assert_eq!(Cmd::NavSortFullPath.id(), 112);
        assert_eq!(Cmd::NavSortSize.id(), 113);
        assert_eq!(Cmd::NavSortDateModified.id(), 114);
        assert_eq!(Cmd::NavSortDateCreated.id(), 115);
        assert_eq!(Cmd::NavSortAscending.id(), 116);
        assert_eq!(Cmd::NavSortDescending.id(), 117);
        assert_eq!(Cmd::NavShuffle.id(), 118);
        assert_eq!(Cmd::NavJumpTo.id(), 119);
        // The #48 Help→Command Line Options row: 120, HelpAbout 121.
        assert_eq!(Cmd::HelpCommandLineOptions.id(), 120);
        assert_eq!(Cmd::HelpAbout.id(), 121);
    }

    #[test]
    fn key_labels_use_upstreams_modifier_order() {
        // _viv_get_key_text appends Ctrl, then Alt, then Shift (viv.c:
        // 12287-12301) before the key name. VKs: F1=0x70, Return=0x0D,
        // Right=0x27 (WinUser.h).
        let chord = KeyDef {
            ctrl: true,
            alt: true,
            shift: true,
            vk: 0x70,
        };
        assert_eq!(key_label(chord, "F1"), "Ctrl+Alt+Shift+F1");
        assert_eq!(
            key_label(
                KeyDef {
                    ctrl: false,
                    alt: true,
                    shift: false,
                    vk: 0x0d
                },
                "Enter"
            ),
            "Alt+Enter"
        );
        assert_eq!(
            key_label(
                KeyDef {
                    ctrl: true,
                    alt: false,
                    shift: false,
                    vk: u16::from(b'O')
                },
                "O"
            ),
            "Ctrl+O"
        );
        // No modifiers: the bare key name.
        assert_eq!(
            key_label(
                KeyDef {
                    ctrl: false,
                    alt: false,
                    shift: false,
                    vk: 0x27
                },
                "Right"
            ),
            "Right"
        );
    }

    #[test]
    fn item_text_appends_the_accelerator_after_a_tab() {
        // The tab routes the label into Windows' right-aligned accelerator
        // column (_viv_create_menu, viv.c:12359-12367).
        assert_eq!(
            item_text("Open File...", Some("Ctrl+O")),
            "Open File...\tCtrl+O"
        );
        assert_eq!(item_text("Best Fit", None), "Best Fit");
    }

    #[test]
    fn check_marks_mirror_the_upstream_conditions() {
        // viv.c:7125/7131/7132 — the Menu/Fullscreen/1:1 checks; viv.c:
        // 7133/7138 — both slideshow rows check with the running flag;
        // viv.c:7184 — Play/Pause checks with the playing flag; viv.c:
        // 7186-7195 — the Sort radios check with the sort config (Name +
        // Ascending here) and Shuffle with its flag. Every other item is
        // unchecked with the toggles off (the sort block: FullPath mode,
        // Descending direction, shuffle off).
        let on = MenuState {
            show_menu: true,
            show_status: true,
            show_controls: true,
            fullscreen: true,
            one_to_one: true,
            slideshow: true,
            slideshow_rate_ms: 5_000,
            animation_playing: true,
            nav_sort: crate::playlist::SortMode::Name,
            nav_sort_ascending: true,
            shuffle: true,
            image_enabled: true,
            allow_shrinking: true,
            keep_aspect: true,
            fill_window: true,
            ontop: 1,
        };
        let off = MenuState {
            show_menu: false,
            show_status: false,
            show_controls: false,
            fullscreen: false,
            one_to_one: false,
            slideshow: false,
            slideshow_rate_ms: 5_000,
            animation_playing: false,
            nav_sort: crate::playlist::SortMode::FullPath,
            nav_sort_ascending: false,
            shuffle: false,
            image_enabled: true,
            allow_shrinking: false,
            keep_aspect: false,
            fill_window: false,
            ontop: 0,
        };
        for cmd in Cmd::ALL {
            let expected_on = matches!(
                cmd,
                Cmd::ViewMenu
                    | Cmd::ViewStatus
                    | Cmd::ViewControls
                    | Cmd::ViewFullscreen
                    | Cmd::ViewOneToOne
                    | Cmd::ViewSlideshow
                    | Cmd::SlideshowPause
                    | Cmd::SlideshowRate5000
                    | Cmd::AnimationPlayPause
                    | Cmd::NavSortName
                    | Cmd::NavSortAscending
                    | Cmd::NavShuffle
                    | Cmd::ViewAllowShrinking
                    | Cmd::ViewKeepAspect
                    | Cmd::ViewFillWindow
                    | Cmd::ViewOntopAlways
            );
            assert_eq!(checked(cmd, &on), expected_on, "{cmd:?} with everything on");
            let expected_off = matches!(
                cmd,
                Cmd::SlideshowRate5000
                    | Cmd::NavSortFullPath
                    | Cmd::NavSortDescending
                    | Cmd::ViewOntopNever
            );
            assert_eq!(
                checked(cmd, &off),
                expected_off,
                "{cmd:?} with toggles off keeps the rate radio and the FullPath/Descending radios"
            );
        }
    }

    /// The On Top popup checks exactly one radio row for any config value
    /// (viv.c:7134-7136 — checked on the exact equality; an unknown ini
    /// value checks none of the three, like the sort modes' Unknown).
    #[test]
    fn the_ontop_radios_follow_the_config_value() {
        for (mode, on) in [
            (0, Cmd::ViewOntopNever),
            (1, Cmd::ViewOntopAlways),
            (2, Cmd::ViewOntopWhilePlaying),
        ] {
            let state = MenuState {
                ontop: mode,
                ..plain_state()
            };
            for cmd in [
                Cmd::ViewOntopAlways,
                Cmd::ViewOntopWhilePlaying,
                Cmd::ViewOntopNever,
            ] {
                assert_eq!(checked(cmd, &state), cmd == on, "ontop={mode} {cmd:?}");
                assert!(radio(cmd), "the on-top rows render as radios");
            }
        }
        // A garbage ini value checks none (the `== value` compares).
        let state = MenuState {
            ontop: 7,
            ..plain_state()
        };
        for cmd in [
            Cmd::ViewOntopAlways,
            Cmd::ViewOntopWhilePlaying,
            Cmd::ViewOntopNever,
        ] {
            assert!(!checked(cmd, &state));
        }
    }

    /// The Sort submenu behaves like the Rate submenu's radio invariant:
    /// exactly one mode row and exactly one direction row check for ANY
    /// config — except an Unknown (garbage ini) sort, where upstream's
    /// equal-compare chain checks no mode at all while the direction pair
    /// still follows the flag (viv.c:7188-7195).
    #[test]
    fn the_sort_radios_check_exactly_one_row_each() {
        for mode in [
            crate::playlist::SortMode::Name,
            crate::playlist::SortMode::Size,
            crate::playlist::SortMode::DateModified,
            crate::playlist::SortMode::DateCreated,
            crate::playlist::SortMode::FullPath,
        ] {
            for &ascending in &[true, false] {
                let state = MenuState {
                    nav_sort: mode,
                    nav_sort_ascending: ascending,
                    shuffle: false,
                    ..plain_state()
                };
                let modes_on: Vec<Cmd> = (Cmd::ALL)
                    .into_iter()
                    .filter(|c| c.sort_mode().is_some() && checked(*c, &state))
                    .collect();
                assert_eq!(modes_on.as_slice(), [mode_cmd(mode)], "{mode:?}");
                let dirs_on: Vec<Cmd> = (Cmd::ALL)
                    .into_iter()
                    .filter(|c| {
                        matches!(c, Cmd::NavSortAscending | Cmd::NavSortDescending)
                            && checked(*c, &state)
                    })
                    .collect();
                assert_eq!(
                    dirs_on.as_slice(),
                    if ascending {
                        &[Cmd::NavSortAscending][..]
                    } else {
                        &[Cmd::NavSortDescending][..]
                    },
                    "{mode:?}"
                );
            }
        }
        // Garbage sort value: no mode row checks, the direction pair
        // follows the flag.
        let state = MenuState {
            nav_sort: crate::playlist::SortMode::Unknown,
            nav_sort_ascending: false,
            ..plain_state()
        };
        for cmd in Cmd::ALL {
            assert_ne!(cmd.sort_mode(), Some(crate::playlist::SortMode::Unknown));
            if cmd.sort_mode().is_some() {
                assert!(
                    !checked(cmd, &state),
                    "{cmd:?} must stay off for garbage sort"
                );
            }
        }
        assert!(checked(Cmd::NavSortDescending, &state));
        assert!(!checked(Cmd::NavSortAscending, &state));
    }

    fn mode_cmd(mode: crate::playlist::SortMode) -> Cmd {
        (Cmd::ALL)
            .into_iter()
            .find(|c| c.sort_mode() == Some(mode))
            .unwrap()
    }

    #[test]
    fn the_rate_radio_checks_the_matching_preset_or_custom() {
        // viv.c:7140-7189: exactly one row of the Rate submenu is checked
        // for any rate — the equal preset, or Custom when none matches.
        // (#39 widened `radio` to the Sort family; this test counts RATE
        // rows only.)
        for &rate in &crate::slideshow::RATE_PRESETS {
            let state = MenuState {
                slideshow: true,
                slideshow_rate_ms: rate,
                ..plain_state()
            };
            let rate_cmds: Vec<Cmd> = (Cmd::ALL)
                .into_iter()
                .filter(|c| c.slideshow_rate_ms().is_some() || *c == Cmd::SlideshowRateCustom)
                .collect();
            let on: Vec<Cmd> = rate_cmds
                .into_iter()
                .filter(|c| checked(*c, &state))
                .collect();
            assert_eq!(on.len(), 1, "rate {rate} checks one radio row");
            assert_eq!(on[0].slideshow_rate_ms(), Some(rate));
        }
        // A custom rate (700 ms — not a preset) checks Custom alone.
        let state = MenuState {
            slideshow_rate_ms: 700,
            ..plain_state()
        };
        assert!(checked(Cmd::SlideshowRateCustom, &state));
        for cmd in Cmd::ALL {
            if cmd.slideshow_rate_ms().is_some() {
                assert!(!checked(cmd, &state), "{cmd:?} must stay off for 700 ms");
            }
        }
    }

    fn plain_state() -> MenuState {
        MenuState {
            show_menu: false,
            show_status: false,
            show_controls: false,
            fullscreen: false,
            one_to_one: false,
            slideshow: false,
            slideshow_rate_ms: 5_000,
            animation_playing: true,
            nav_sort: crate::playlist::SortMode::DateModified,
            nav_sort_ascending: false,
            shuffle: false,
            image_enabled: true,
            allow_shrinking: false,
            keep_aspect: false,
            fill_window: false,
            ontop: 0,
        }
    }

    #[test]
    fn the_rate_rows_map_onto_the_preset_table_in_menu_order() {
        // The 17 preset commands must be exactly the 17 presets, in the
        // submenu's walk order — ENTRIES order equals RATE_PRESETS order,
        // so the radio and the table cannot drift apart.
        let by_menu: Vec<u32> = ENTRIES
            .iter()
            .filter_map(|e| match e {
                Entry::Item {
                    cmd,
                    parent: Slot::SlideshowRate,
                    ..
                } => cmd.slideshow_rate_ms(),
                _ => None,
            })
            .collect();
        assert_eq!(by_menu, crate::slideshow::RATE_PRESETS.to_vec());
    }

    #[test]
    fn the_image_gated_commands_gate_and_only_on_the_image_flag() {
        // #41 + #42: upstream grays exactly Cut/Copy/Copy Filename/Copy
        // Image (viv.c:7104-7108) plus the shell septet Close/Edit/
        // Location/Preview/Print/Properties/Wallpaper (viv.c:7114-7116)
        // through `is_image_enabled` — Paste has NO EnableMenuItem row (an
        // empty clipboard is just a no-op). Every other riviv command
        // stays always-selectable.
        let gated_cmds = [
            Cmd::EditCut,
            Cmd::EditCopy,
            Cmd::EditCopyFilename,
            Cmd::EditCopyImage,
            Cmd::FileOpenFileLocation,
            Cmd::FileEdit,
            Cmd::FilePreview,
            Cmd::FilePrint,
            Cmd::FileSetDesktopWallpaper,
            Cmd::FileClose,
            Cmd::FileProperties,
        ];
        for cmd in Cmd::ALL {
            if gated_cmds.contains(&cmd) {
                continue;
            }
            assert!(enabled(cmd, &plain_state()), "{cmd:?}");
        }
        let mut gated = plain_state();
        gated.image_enabled = false;
        for cmd in gated_cmds {
            assert!(!enabled(cmd, &gated), "{cmd:?}");
        }
        assert!(enabled(Cmd::EditPaste, &gated), "{:?}", Cmd::EditPaste);
    }

    #[test]
    fn menu_strings_are_non_empty_in_both_languages_for_every_entry() {
        // Every localized row must resolve to real text in BOTH tables —
        // an empty caption would append a blank menu row (or, for the
        // hidden rows, a blank Controls-list entry).
        for entry in ENTRIES {
            let id = match entry {
                Entry::Separator { .. } => continue,
                Entry::Popup { loc, .. } => *loc,
                Entry::Item { loc, .. } | Entry::HiddenItem { loc, .. } => *loc,
            };
            assert!(!loc::get_for(loc::Language::English, id).is_empty());
            assert!(!loc::get_for(loc::Language::ChineseSimplified, id).is_empty());
        }
    }

    #[test]
    fn command_ids_stay_pinned_for_the_smoke_scripts() {
        // The cross-process smokes post WM_COMMAND by raw id; inserting a
        // Cmd variant shifts everything after it. This pins the
        // load-bearing anchors (the #46 baseline; #47 reads panscan/rate/
        // 1:1/Options; #42 reads the shell septet 6-12). The #43
        // insertion added 8 commands (four between Close and Properties,
        // four after Paste) and pushed COUNT to 121.
        assert_eq!(Cmd::FileOpenFileLocation.id(), 6);
        assert_eq!(Cmd::FileClose.id(), 11);
        assert_eq!(Cmd::FileDelete.id(), 12);
        assert_eq!(Cmd::FileDeleteRecycle.id(), 13);
        assert_eq!(Cmd::FileDeletePermanently.id(), 14);
        assert_eq!(Cmd::FileRename.id(), 15);
        assert_eq!(Cmd::FileProperties.id(), 16);
        assert_eq!(Cmd::FileExit.id(), 17);
        assert_eq!(Cmd::EditRotate90.id(), 23);
        assert_eq!(Cmd::EditRotate270.id(), 24);
        assert_eq!(Cmd::EditCopyTo.id(), 25);
        assert_eq!(Cmd::EditMoveTo.id(), 26);
        assert_eq!(Cmd::ViewOneToOne.id(), 45);
        assert_eq!(Cmd::ViewPanScanIncreaseSize.id(), 47);
        assert_eq!(Cmd::ViewOptions.id(), 69);
        assert_eq!(Cmd::SlideshowRate1000.id(), 77);
        assert_eq!(Cmd::AnimationRateDecrease.id(), 104);
        assert_eq!(Cmd::HelpAbout.id(), 121);
        assert_eq!(Cmd::COUNT, 121);
    }

    #[test]
    fn the_owner_draw_rows_hide_from_the_menu_bar_only() {
        // Upstream's MF_OWNERDRAW rows: present in the command table
        // (Controls list + WM_COMMAND + ini names), absent from the menu
        // bar build (viv.c:12328). The #42 Close row (viv.c:816), the
        // animation quartet (viv.c:925-928), the #41 clipboard pair —
        // Copy Filename (viv.c:828) and Paste (viv.c:830, keyboard
        // Ctrl+Shift+C / Ctrl+V upstream) — #44's four diagonal Pan/Scan
        // moves (viv.c:881-884, Ctrl+NUMPAD corners), #45's toolbar-only
        // slideshow pair (upstream keeps them out of the table entirely;
        // riviv's hidden-row superset, README Differences), and #43's two
        // explicit delete rows (viv.c:818-819, Del / Shift+Del). The
        // table marks them HiddenItem; the visible rows stay Item.
        let hidden: Vec<Cmd> = ENTRIES
            .iter()
            .filter_map(|e| match e {
                Entry::HiddenItem { cmd, .. } => Some(*cmd),
                _ => None,
            })
            .collect();
        assert_eq!(
            hidden,
            vec![
                Cmd::FileClose,
                Cmd::FileDeleteRecycle,
                Cmd::FileDeletePermanently,
                Cmd::EditCopyFilename,
                Cmd::EditPaste,
                Cmd::ViewCaption,
                Cmd::ViewThickFrame,
                Cmd::ViewPanScanMoveUpLeft,
                Cmd::ViewPanScanMoveUpRight,
                Cmd::ViewPanScanMoveDownLeft,
                Cmd::ViewPanScanMoveDownRight,
                Cmd::SlideshowPlayOnly,
                Cmd::SlideshowPauseOnly,
                Cmd::AnimationJumpForwardShort,
                Cmd::AnimationJumpBackwardShort,
                Cmd::AnimationJumpForwardLong,
                Cmd::AnimationJumpBackwardLong,
            ]
        );
    }

    /// #49: the context walk pinned against upstream's table order
    /// (`_viv_context_menu_items`, viv.c:1053-1123) with the bar hidden —
    /// the verbatim table. Any future insertion into CONTEXT_TABLE shifts
    /// this and gets caught here.
    #[test]
    fn context_rows_pin_the_upstream_table_order() {
        let c = |cmd| ContextRow::Command {
            cmd,
            label: context_label(cmd),
        };
        let pp = |slot, label| ContextRow::Popup { slot, label };
        let expected_full = vec![
            c(Cmd::NavNext),
            c(Cmd::NavPrev),
            ContextRow::Separator,
            c(Cmd::ViewFullscreen),
            c(Cmd::SlideshowPause),
            pp(Slot::SlideshowRate, loc::Id::MenuSlideshowRate),
            c(Cmd::SlideshowRateDecrease),
            c(Cmd::SlideshowRateIncrease),
            ContextRow::Separator,
            c(Cmd::SlideshowRate250),
            c(Cmd::SlideshowRate500),
            c(Cmd::SlideshowRate1000),
            c(Cmd::SlideshowRate2000),
            c(Cmd::SlideshowRate3000),
            c(Cmd::SlideshowRate4000),
            c(Cmd::SlideshowRate5000),
            c(Cmd::SlideshowRate6000),
            c(Cmd::SlideshowRate7000),
            c(Cmd::SlideshowRate8000),
            c(Cmd::SlideshowRate9000),
            c(Cmd::SlideshowRate10000),
            c(Cmd::SlideshowRate20000),
            c(Cmd::SlideshowRate30000),
            c(Cmd::SlideshowRate40000),
            c(Cmd::SlideshowRate50000),
            c(Cmd::SlideshowRate60000),
            c(Cmd::SlideshowRateCustom),
            ContextRow::Pop,
            ContextRow::Separator,
            c(Cmd::ViewMenu),
            ContextRow::Separator,
            c(Cmd::ViewAllowShrinking),
            c(Cmd::ViewKeepAspect),
            c(Cmd::ViewFillWindow),
            c(Cmd::ViewOneToOne),
            ContextRow::Separator,
            pp(Slot::NavigateSort, loc::Id::MenuSort),
            c(Cmd::NavSortName),
            c(Cmd::NavSortFullPath),
            c(Cmd::NavSortSize),
            c(Cmd::NavSortDateModified),
            c(Cmd::NavSortDateCreated),
            ContextRow::Separator,
            c(Cmd::NavSortAscending),
            c(Cmd::NavSortDescending),
            ContextRow::Pop,
            ContextRow::Separator,
            c(Cmd::FileOpenFileLocation),
            c(Cmd::FileSetDesktopWallpaper),
            c(Cmd::FileEdit),
            c(Cmd::FilePrint),
            ContextRow::Separator,
            c(Cmd::EditRotate90),
            c(Cmd::EditRotate270),
            ContextRow::Separator,
            c(Cmd::EditCut),
            c(Cmd::EditCopy),
            c(Cmd::EditCopyImage),
            ContextRow::Separator,
            c(Cmd::FileDelete),
            c(Cmd::FileRename),
            ContextRow::Separator,
            c(Cmd::FileProperties),
            c(Cmd::ViewOptions),
            ContextRow::Separator,
            c(Cmd::FileExit),
        ];
        let full = context_rows(false);
        assert_eq!(full, expected_full);
        // Bar visible: the recovery row and ONE of its flanking separators
        // drop (the dedupe eats the second) — splicing those two rows out
        // of the hidden-bar walk must equal the visible-bar walk exactly.
        let i = full
            .iter()
            .position(|r| {
                matches!(
                    r,
                    ContextRow::Command {
                        cmd: Cmd::ViewMenu,
                        ..
                    }
                )
            })
            .expect("recovery row in the hidden-bar walk");
        let mut gated = full.clone();
        gated.drain(i..i + 2);
        assert_eq!(context_rows(true), gated);
    }

    /// The Menu recovery row gates on the bar (viv.c:3427) and its removal
    /// never leaves adjacent separators (the was_separator dedupe).
    #[test]
    fn the_menu_recovery_row_gates_on_the_bar_without_leaving_a_double_separator() {
        let hidden = context_rows(false);
        let shown = context_rows(true);
        assert!(hidden.iter().any(|r| matches!(
            r,
            ContextRow::Command {
                cmd: Cmd::ViewMenu,
                ..
            }
        )));
        assert!(!shown.iter().any(|r| matches!(
            r,
            ContextRow::Command {
                cmd: Cmd::ViewMenu,
                ..
            }
        )));
        for rows in [&hidden, &shown] {
            assert!(!matches!(rows.first(), Some(ContextRow::Separator)));
            for pair in rows.windows(2) {
                assert!(
                    !(matches!(pair[0], ContextRow::Separator)
                        && matches!(pair[1], ContextRow::Separator)),
                    "adjacent separators at {:?}",
                    &rows[..2]
                );
            }
        }
    }

    /// Upstream gates the Preview row off on Windows 8+ (viv.c:3418-3423 —
    /// the verb "doesn't exist on Windows 8 or later"); riviv's baseline is
    /// Win8+, so the walk drops it unconditionally. The BAR keeps its row
    /// (the gate is context-menu-only) — #42 behavior unchanged.
    #[test]
    fn the_preview_row_never_ships_in_the_context_menu() {
        for show_menu in [false, true] {
            assert!(!context_rows(show_menu).iter().any(|r| matches!(
                r,
                ContextRow::Command {
                    cmd: Cmd::FilePreview,
                    ..
                }
            )));
        }
        assert!(ENTRIES.iter().any(|e| matches!(
            e,
            Entry::Item {
                cmd: Cmd::FilePreview,
                ..
            }
        )));
    }

    /// The slideshow-pause context row takes the Play/Pause caption
    /// (viv.c:3442-3452); every other row keeps its own command caption.
    #[test]
    fn the_slideshow_pause_row_takes_the_play_pause_caption() {
        assert_eq!(
            context_label(Cmd::SlideshowPause),
            loc::Id::MenuSlideshowPlayPause
        );
        assert_eq!(context_label(Cmd::NavNext), loc::Id::MenuNext);
        assert_eq!(context_label(Cmd::FileDelete), loc::Id::MenuDelete);
    }

    /// The accelerator hint of the visible Delete row reads the
    /// recycle-delete binding (viv.c:3457-3460 for the context walk,
    /// viv.c:12356-12361 for the bar build); every other command is
    /// identity.
    #[test]
    fn hint_cmd_remaps_only_the_visible_delete_row() {
        assert_eq!(hint_cmd(Cmd::FileDelete), Cmd::FileDeleteRecycle);
        for cmd in [
            Cmd::FileDeleteRecycle,
            Cmd::FileDeletePermanently,
            Cmd::NavNext,
            Cmd::SlideshowPause,
            Cmd::FileExit,
        ] {
            assert_eq!(hint_cmd(cmd), cmd);
        }
    }

    /// Every context row's caption resolves to real text in BOTH language
    /// tables (an empty caption would append a blank row).
    #[test]
    fn context_rows_carry_non_empty_captions_in_both_languages() {
        for row in context_rows(false) {
            let id = match row {
                ContextRow::Command { label, .. } | ContextRow::Popup { label, .. } => label,
                ContextRow::Pop | ContextRow::Separator => continue,
            };
            assert!(!loc::get_for(loc::Language::English, id).is_empty());
            assert!(!loc::get_for(loc::Language::ChineseSimplified, id).is_empty());
        }
    }

    /// The push/pop markers nest exactly one level and the pop arm leaves
    /// the dedupe state alone (viv.c:3482-3487) — which is why the
    /// top-level separator right after each pop still lands (the last row
    /// before the pop is a command, so was_separator is false across it).
    #[test]
    fn pushes_and_pops_balance_and_separators_survive_the_pops() {
        let rows = context_rows(false);
        let mut depth = 0i32;
        for (i, row) in rows.iter().enumerate() {
            match row {
                ContextRow::Popup { .. } => depth += 1,
                ContextRow::Pop => {
                    depth -= 1;
                    assert_eq!(depth, 0, "unbalanced or nested pop at row {i}");
                    assert!(
                        matches!(rows.get(i + 1), Some(ContextRow::Separator)),
                        "the separator after the pop at {i} was eaten"
                    );
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "unclosed push");
    }
}
