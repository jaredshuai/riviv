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
    /// Navigate → Next (`VIV_ID_NAV_NEXT`).
    NavNext,
    /// Navigate → Previous (`VIV_ID_NAV_PREV`).
    NavPrev,
    /// Navigate → Home (`VIV_ID_NAV_HOME`).
    NavHome,
    /// Navigate → End (`VIV_ID_NAV_END`).
    NavEnd,
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
        Self::ViewMenu,
        Self::ViewFullscreen,
        Self::ViewSlideshow,
        Self::ViewOneToOne,
        Self::ViewBestFit,
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
        Self::NavNext,
        Self::NavPrev,
        Self::NavHome,
        Self::NavEnd,
        Self::HelpAbout,
    ];
}

/// A menu slot — the implemented subset of upstream's `_VIV_MENU_*` enum
/// (viv.c:318-334). Root is the menu bar itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Root,
    File,
    View,
    ViewZoom,
    /// The Slideshow top-level menu (#37; upstream `_VIV_MENU_SLIDESHOW`,
    /// viv.c:892 — between View and Animation in the root order).
    Slideshow,
    /// The Slideshow → Rate popup (#37; upstream `_VIV_MENU_SLIDESHOW_RATE`,
    /// viv.c:897).
    SlideshowRate,
    Navigate,
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
}

/// The command table (upstream `_viv_commands[]`, viv.c:798-965, pruned to
/// riviv's implemented commands; the unimplemented menus — Edit, Animation
/// and the dead rows inside File/View/Navigate/Help — wait for their
/// features). Order is upstream order.
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
    // Help (viv.c:962-965): upstream precedes About with Help /
    // command-line options / website / donate rows riviv does not ship.
    Entry::Popup {
        loc: loc::Id::MenuHelp,
        parent: Slot::Root,
        slot: Slot::Help,
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
}

/// Whether `cmd`'s menu item carries a check in `state` (upstream
/// `_viv_check_menus`, viv.c:7123-7132 — only toggle-ish commands do).
pub(crate) fn checked(cmd: Cmd, state: &MenuState) -> bool {
    match cmd {
        Cmd::ViewMenu => state.show_menu,
        Cmd::ViewFullscreen => state.fullscreen,
        Cmd::ViewOneToOne => state.one_to_one,
        Cmd::ViewSlideshow | Cmd::SlideshowPause => state.slideshow,
        // The radio's checked row: the preset that equals the rate, or
        // Custom when the rate is no preset (viv.c:7140-7189's switch
        // default).
        Cmd::SlideshowRateCustom => !crate::slideshow::is_preset(state.slideshow_rate_ms),
        cmd => cmd.slideshow_rate_ms() == Some(state.slideshow_rate_ms),
    }
}

/// Whether `cmd`'s check renders as a radio dot (upstream passes
/// `MFT_RADIOCHECK` in the CheckMenuItem flags for the whole Rate submenu
/// family, viv.c:7165-7189 — a display trait, not a state).
pub(crate) fn radio(cmd: Cmd) -> bool {
    cmd.slideshow_rate_ms().is_some() || cmd == Cmd::SlideshowRateCustom
}

/// Whether `cmd`'s menu item is selectable. Everything riviv registers is
/// always available (upstream never gates zoom/navigation on image state
/// either — its EnableMenuItem list, viv.c:7103-7121, covers clipboard/
/// delete/print commands riviv does not register).
pub(crate) fn enabled(_cmd: Cmd) -> bool {
    true
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
        // distinct, viv.c:798-965).
        let mut seen = [false; Cmd::COUNT];
        for entry in ENTRIES {
            if let Entry::Item { cmd, .. } = entry {
                assert!(!seen[usize::from(cmd.id() - 1)], "{cmd:?} registered twice");
                seen[usize::from(cmd.id() - 1)] = true;
            }
        }
        // Every command the enum declares has a menu row.
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
    fn slideshow_command_ids_are_pinned_for_the_wire() {
        // smoke37 posts these as raw WM_COMMAND wparams; inserting a command
        // ahead of the slideshow block would silently shift every wire id.
        assert_eq!(Cmd::ViewSlideshow.id(), 9);
        assert_eq!(Cmd::SlideshowPause.id(), 16);
        assert_eq!(Cmd::SlideshowRateDecrease.id(), 17);
        assert_eq!(Cmd::SlideshowRateIncrease.id(), 18);
        assert_eq!(Cmd::SlideshowRate250.id(), 19);
        assert_eq!(Cmd::SlideshowRate500.id(), 20);
        assert_eq!(Cmd::SlideshowRateCustom.id(), 36);
        assert_eq!(Cmd::NavNext.id(), 37);
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
        // 7133/7138 — both slideshow rows check with the running flag.
        // Every non-rate item is unchecked with the toggles off.
        let on = MenuState {
            show_menu: true,
            fullscreen: true,
            one_to_one: true,
            slideshow: true,
            slideshow_rate_ms: 5_000,
        };
        let off = MenuState {
            show_menu: false,
            fullscreen: false,
            one_to_one: false,
            slideshow: false,
            slideshow_rate_ms: 5_000,
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
            );
            assert_eq!(checked(cmd, &on), expected_on, "{cmd:?} with everything on");
            assert_eq!(
                checked(cmd, &off),
                cmd.slideshow_rate_ms() == Some(5_000),
                "{cmd:?} with toggles off keeps only the rate radio"
            );
        }
    }

    #[test]
    fn the_rate_radio_checks_the_matching_preset_or_custom() {
        // viv.c:7140-7189: exactly one row of the Rate submenu is checked
        // for any rate — the equal preset, or Custom when none matches.
        for &rate in &crate::slideshow::RATE_PRESETS {
            let state = MenuState {
                slideshow: true,
                slideshow_rate_ms: rate,
                ..plain_state()
            };
            let rate_cmds: Vec<Cmd> = Cmd::ALL.into_iter().filter(|c| radio(*c)).collect();
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
    fn every_registered_item_is_selectable() {
        // #24 wired the Options dialog: nothing riviv registers is ever
        // greyed (upstream gates only clipboard/delete/print-style
        // commands, none registered).
        for cmd in Cmd::ALL {
            assert!(enabled(cmd), "{cmd:?}");
        }
    }

    #[test]
    fn menu_strings_are_non_empty_in_both_languages_for_every_entry() {
        // Every localized row must resolve to real text in BOTH tables —
        // an empty caption would append a blank menu row.
        for entry in ENTRIES {
            let id = match entry {
                Entry::Separator { .. } => continue,
                Entry::Popup { loc, .. } => *loc,
                Entry::Item { loc, .. } => *loc,
            };
            assert!(!loc::get_for(loc::Language::English, id).is_empty());
            assert!(!loc::get_for(loc::Language::ChineseSimplified, id).is_empty());
        }
    }
}
