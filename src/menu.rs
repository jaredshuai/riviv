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
    /// View → Menu toggle (`VIV_ID_VIEW_MENU`).
    ViewMenu,
    /// View → Fullscreen (`VIV_ID_VIEW_FULLSCREEN`).
    ViewFullscreen,
    /// View → Slideshow (`VIV_ID_VIEW_SLIDESHOW`, viv.c:854) — #37: the
    /// start-only toggle (enters fullscreen first when windowed).
    ViewSlideshow,
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
    /// View → Options... (`VIV_ID_VIEW_OPTIONS`) — opens the modal Options
    /// dialog (#24; upstream viv.c:2332).
    ViewOptions,
    /// Slideshow → Play/Pause (`VIV_ID_SLIDESHOW_PAUSE`, viv.c:894) — #37:
    /// the running-state TOGGLE (no fullscreen entry, unlike F11).
    SlideshowPause,
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
        Self::FileExit,
        Self::EditCut,
        Self::EditCopy,
        Self::EditCopyFilename,
        Self::EditCopyImage,
        Self::EditPaste,
        Self::ViewMenu,
        Self::ViewFullscreen,
        Self::ViewSlideshow,
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
        Self::ViewOptions,
        Self::SlideshowPause,
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
    /// The View → Pan/Scan popup (#44; upstream `_VIV_MENU_VIEW_PANSCAN`,
    /// viv.c:870 — between Best Fit and Zoom).
    ViewPanScan,
    ViewZoom,
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
    // View (viv.c:839-935): Menu toggle, fullscreen, 1:1 / Best Fit, the
    // Zoom submenu, Options last — upstream's relative order, gaps dropped.
    Entry::Popup {
        loc: loc::Id::MenuView,
        parent: Slot::Root,
        slot: Slot::View,
    },
    Entry::Item {
        loc: loc::Id::MenuMenu,
        parent: Slot::View,
        cmd: Cmd::ViewMenu,
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

/// The dynamic menu state read when a menu is about to open (the inputs of
/// upstream `_viv_check_menus`' CheckMenuItem calls, viv.c:7123-7132 —
/// upstream's EnableMenuItem list is copy/delete/print-style commands
/// riviv does not register, so the only enable decision here is the
/// Options placeholder).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuState {
    /// View → Menu's checkmark: `config_show_menu` (viv.c:7125).
    pub(crate) show_menu: bool,
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
}

/// Whether `cmd`'s menu item carries a check in `state` (upstream
/// `_viv_check_menus`, viv.c:7123-7132 — only toggle-ish commands do).
pub(crate) fn checked(cmd: Cmd, state: &MenuState) -> bool {
    match cmd {
        Cmd::ViewMenu => state.show_menu,
        Cmd::ViewFullscreen => state.fullscreen,
        Cmd::ViewOneToOne => state.one_to_one,
        Cmd::ViewSlideshow | Cmd::SlideshowPause => state.slideshow,
        Cmd::AnimationPlayPause => state.animation_playing,
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
        || matches!(cmd, Cmd::NavSortAscending | Cmd::NavSortDescending)
}

/// Whether `cmd`'s menu item is selectable. Everything riviv registers is
/// always available (upstream never gates zoom/navigation on image state
/// either — its EnableMenuItem list, viv.c:7103-7121, covers clipboard/
/// delete/print commands riviv does not register).
pub(crate) fn enabled(cmd: Cmd, state: &MenuState) -> bool {
    match cmd {
        // The clipboard quartet (#41): upstream grays all four through the
        // same `is_image_enabled` gate (viv.c:7104-7108). Paste is not in
        // that list — an empty clipboard just makes the handler a no-op.
        Cmd::EditCut | Cmd::EditCopy | Cmd::EditCopyFilename | Cmd::EditCopyImage => {
            state.image_enabled
        }
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
        // The #41 Edit block runs 7-11 (viv.c:826-830 table order), pushing
        // every later id by 5: ViewSlideshow 9 → 14, NavNext 51 → 56,
        // HelpAbout 64 → 69.
        assert_eq!(Cmd::EditCut.id(), 7);
        assert_eq!(Cmd::EditCopy.id(), 8);
        assert_eq!(Cmd::EditCopyFilename.id(), 9);
        assert_eq!(Cmd::EditCopyImage.id(), 10);
        assert_eq!(Cmd::EditPaste.id(), 11);
        assert_eq!(Cmd::ViewSlideshow.id(), 14);
        // The #44 Pan/Scan block runs 17-32 (viv.h:110-125 order, right
        // after ViewBestFit=16), pushing every later id by 16.
        assert_eq!(Cmd::ViewPanScanIncreaseSize.id(), 17);
        assert_eq!(Cmd::ViewPanScanDecreaseSize.id(), 18);
        assert_eq!(Cmd::ViewPanScanIncreaseWidth.id(), 19);
        assert_eq!(Cmd::ViewPanScanDecreaseWidth.id(), 20);
        assert_eq!(Cmd::ViewPanScanIncreaseHeight.id(), 21);
        assert_eq!(Cmd::ViewPanScanDecreaseHeight.id(), 22);
        assert_eq!(Cmd::ViewPanScanMoveUp.id(), 23);
        assert_eq!(Cmd::ViewPanScanMoveDown.id(), 24);
        assert_eq!(Cmd::ViewPanScanMoveLeft.id(), 25);
        assert_eq!(Cmd::ViewPanScanMoveRight.id(), 26);
        assert_eq!(Cmd::ViewPanScanMoveUpLeft.id(), 27);
        assert_eq!(Cmd::ViewPanScanMoveUpRight.id(), 28);
        assert_eq!(Cmd::ViewPanScanMoveDownLeft.id(), 29);
        assert_eq!(Cmd::ViewPanScanMoveDownRight.id(), 30);
        assert_eq!(Cmd::ViewPanScanMoveCenter.id(), 31);
        assert_eq!(Cmd::ViewPanScanReset.id(), 32);
        assert_eq!(Cmd::SlideshowPause.id(), 37);
        assert_eq!(Cmd::SlideshowRateDecrease.id(), 38);
        assert_eq!(Cmd::SlideshowRateIncrease.id(), 39);
        assert_eq!(Cmd::SlideshowRate250.id(), 40);
        assert_eq!(Cmd::SlideshowRate500.id(), 41);
        assert_eq!(Cmd::SlideshowRateCustom.id(), 57);
        // The #38 Animation block runs 37-50 upstream-side (#37's NavNext
        // was 37; the 14 Animation commands pushed it to 51) — with the
        // #41 and #44 shifts it lands 58-71.
        assert_eq!(Cmd::AnimationPlayPause.id(), 58);
        assert_eq!(Cmd::AnimationJumpForwardMedium.id(), 59);
        assert_eq!(Cmd::AnimationJumpBackwardMedium.id(), 60);
        assert_eq!(Cmd::AnimationJumpForwardShort.id(), 61);
        assert_eq!(Cmd::AnimationJumpBackwardShort.id(), 62);
        assert_eq!(Cmd::AnimationJumpForwardLong.id(), 63);
        assert_eq!(Cmd::AnimationJumpBackwardLong.id(), 64);
        assert_eq!(Cmd::AnimationFrameStep.id(), 65);
        assert_eq!(Cmd::AnimationFramePrev.id(), 66);
        assert_eq!(Cmd::AnimationFirstFrame.id(), 67);
        assert_eq!(Cmd::AnimationLastFrame.id(), 68);
        assert_eq!(Cmd::AnimationRateDecrease.id(), 69);
        assert_eq!(Cmd::AnimationRateIncrease.id(), 70);
        assert_eq!(Cmd::AnimationRateReset.id(), 71);
        assert_eq!(Cmd::NavNext.id(), 72);
        // The #39 sort/shuffle/jumpto block (menu-table order,
        // viv.c:946-956); HelpCommandLineOptions lands 85, HelpAbout 86.
        assert_eq!(Cmd::NavSortName.id(), 76);
        assert_eq!(Cmd::NavSortFullPath.id(), 77);
        assert_eq!(Cmd::NavSortSize.id(), 78);
        assert_eq!(Cmd::NavSortDateModified.id(), 79);
        assert_eq!(Cmd::NavSortDateCreated.id(), 80);
        assert_eq!(Cmd::NavSortAscending.id(), 81);
        assert_eq!(Cmd::NavSortDescending.id(), 82);
        assert_eq!(Cmd::NavShuffle.id(), 83);
        assert_eq!(Cmd::NavJumpTo.id(), 84);
        // The #48 Help→Command Line Options row takes upstream's slot
        // between the (unshipped) help rows and About: 85, pushing
        // HelpAbout to 86.
        assert_eq!(Cmd::HelpCommandLineOptions.id(), 85);
        assert_eq!(Cmd::HelpAbout.id(), 86);
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
            fullscreen: true,
            one_to_one: true,
            slideshow: true,
            slideshow_rate_ms: 5_000,
            animation_playing: true,
            nav_sort: crate::playlist::SortMode::Name,
            nav_sort_ascending: true,
            shuffle: true,
            image_enabled: true,
        };
        let off = MenuState {
            show_menu: false,
            fullscreen: false,
            one_to_one: false,
            slideshow: false,
            slideshow_rate_ms: 5_000,
            animation_playing: false,
            nav_sort: crate::playlist::SortMode::FullPath,
            nav_sort_ascending: false,
            shuffle: false,
            image_enabled: true,
        };
        for cmd in Cmd::ALL {
            let expected_on = matches!(
                cmd,
                Cmd::ViewMenu
                    | Cmd::ViewFullscreen
                    | Cmd::ViewOneToOne
                    | Cmd::ViewSlideshow
                    | Cmd::SlideshowPause
                    | Cmd::SlideshowRate5000
                    | Cmd::AnimationPlayPause
                    | Cmd::NavSortName
                    | Cmd::NavSortAscending
                    | Cmd::NavShuffle
            );
            assert_eq!(checked(cmd, &on), expected_on, "{cmd:?} with everything on");
            let expected_off = matches!(
                cmd,
                Cmd::SlideshowRate5000 | Cmd::NavSortFullPath | Cmd::NavSortDescending
            );
            assert_eq!(
                checked(cmd, &off),
                expected_off,
                "{cmd:?} with toggles off keeps the rate radio and the FullPath/Descending radios"
            );
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
            fullscreen: false,
            one_to_one: false,
            slideshow: false,
            slideshow_rate_ms: 5_000,
            animation_playing: true,
            nav_sort: crate::playlist::SortMode::DateModified,
            nav_sort_ascending: false,
            shuffle: false,
            image_enabled: true,
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
    fn only_the_clipboard_quartet_gates_and_only_on_the_image_flag() {
        // #41: upstream grays exactly Cut/Copy/Copy Filename/Copy Image
        // through `is_image_enabled` (viv.c:7104-7108) — Paste has NO
        // EnableMenuItem row (an empty clipboard is just a no-op). Every
        // other riviv command stays always-selectable.
        for cmd in Cmd::ALL {
            if matches!(
                cmd,
                Cmd::EditCut | Cmd::EditCopy | Cmd::EditCopyFilename | Cmd::EditCopyImage
            ) {
                continue;
            }
            assert!(enabled(cmd, &plain_state()), "{cmd:?}");
        }
        let mut gated = plain_state();
        gated.image_enabled = false;
        assert!(!enabled(Cmd::EditCut, &gated), "{:?}", Cmd::EditCut);
        assert!(!enabled(Cmd::EditCopy, &gated), "{:?}", Cmd::EditCopy);
        assert!(
            !enabled(Cmd::EditCopyFilename, &gated),
            "{:?}",
            Cmd::EditCopyFilename
        );
        assert!(
            !enabled(Cmd::EditCopyImage, &gated),
            "{:?}",
            Cmd::EditCopyImage
        );
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
    fn the_owner_draw_rows_hide_from_the_menu_bar_only() {
        // Upstream's MF_OWNERDRAW rows: present in the command table
        // (Controls list + WM_COMMAND + ini names), absent from the menu
        // bar build (viv.c:12328). The animation quartet (viv.c:925-928),
        // the #41 clipboard pair — Copy Filename (viv.c:828) and Paste
        // (viv.c:830), keyboard Ctrl+Shift+C / Ctrl+V upstream — and #44's
        // four diagonal Pan/Scan moves (viv.c:881-884, Ctrl+NUMPAD
        // corners). The table marks them HiddenItem; the visible rows
        // stay Item.
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
                Cmd::EditCopyFilename,
                Cmd::EditPaste,
                Cmd::ViewPanScanMoveUpLeft,
                Cmd::ViewPanScanMoveUpRight,
                Cmd::ViewPanScanMoveDownLeft,
                Cmd::ViewPanScanMoveDownRight,
                Cmd::AnimationJumpForwardShort,
                Cmd::AnimationJumpBackwardShort,
                Cmd::AnimationJumpForwardLong,
                Cmd::AnimationJumpBackwardLong,
            ]
        );
    }
}
