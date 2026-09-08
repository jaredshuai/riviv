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
//! HMENU objects, appends the accelerator labels, and dispatches
//! WM_COMMAND through [`Cmd::from_id`].

use crate::loc;

/// A command the menu can trigger — the implemented subset of upstream's
/// `VIV_ID_*` (viv.h:71-204). Order is upstream table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmd {
    /// File → Open File... (`VIV_ID_FILE_OPEN_FILE`).
    FileOpenFile,
    /// File → Open Folder... (`VIV_ID_FILE_OPEN_FOLDER`).
    FileOpenFolder,
    /// File → Exit (`VIV_ID_FILE_EXIT`).
    FileExit,
    /// View → Menu toggle (`VIV_ID_VIEW_MENU`).
    ViewMenu,
    /// View → Fullscreen (`VIV_ID_VIEW_FULLSCREEN`).
    ViewFullscreen,
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
    /// View → Options... (`VIV_ID_VIEW_OPTIONS`) — placeholder until the
    /// Options dialog issue wires it (greyed in the meantime).
    ViewOptions,
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
        Self::FileExit,
        Self::ViewMenu,
        Self::ViewFullscreen,
        Self::ViewOneToOne,
        Self::ViewBestFit,
        Self::ViewZoomIn,
        Self::ViewZoomOut,
        Self::ViewZoomReset,
        Self::ViewOptions,
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
    Navigate,
    Help,
}

impl Slot {
    /// Variant count; indexes the Win32 half's per-slot HMENU array.
    pub(crate) const COUNT: usize = Self::Help as usize + 1;
}

/// A default key binding (upstream `_viv_default_keys[]`, viv.c:969-1129 —
/// flags are `CONFIG_KEYFLAG_CTRL/ALT/SHIFT << 8 | VK`; riviv keeps the
/// parts split). One entry per command — the FIRST key upstream registers,
/// which is the one the menu label displays (viv.c:12354-12366); the
/// keyboard path keeps answering the alternates (numpad +/-, PgUp/PgDn)
/// that upstream also registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyDef {
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) shift: bool,
    /// Win32 virtual-key code (ABI-stable values, WinUser.h).
    pub(crate) vk: u16,
}

// Win32 virtual-key codes used by the default keys (WinUser.h; kept as raw
// values so this module stays pure-logic with no windows crate dependency).
const VK_RETURN: u16 = 0x0d;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_F1: u16 = 0x70;
const VK_OEM_PLUS: u16 = 0xbb;
const VK_OEM_MINUS: u16 = 0xbd;

/// One row of the menu table (upstream `_viv_command_t`, viv.c:408-413):
/// what to append, into which parent slot, and the default key whose label
/// rides along on items. The enum shape makes the invalid upstream combos
/// (a key on a separator, a localization id on a separator) unrepresentable.
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
        key: Option<KeyDef>,
    },
}

/// The command table (upstream `_viv_commands[]`, viv.c:798-965, pruned to
/// riviv's implemented commands; the unimplemented menus — Edit, Slideshow,
/// Animation and the dead rows inside File/View/Navigate/Help — wait for
/// their features). Order is upstream order.
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
        key: Some(KeyDef {
            ctrl: true,
            alt: false,
            shift: false,
            vk: b'O' as u16,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuOpenFolder,
        parent: Slot::File,
        cmd: Cmd::FileOpenFolder,
        key: Some(KeyDef {
            ctrl: true,
            alt: false,
            shift: false,
            vk: b'B' as u16,
        }),
    },
    Entry::Separator { parent: Slot::File },
    Entry::Item {
        loc: loc::Id::MenuExit,
        parent: Slot::File,
        cmd: Cmd::FileExit,
        key: Some(KeyDef {
            ctrl: true,
            alt: false,
            shift: false,
            vk: b'Q' as u16,
        }),
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
        // Upstream registers no default key for VIV_ID_VIEW_MENU
        // (absent from _viv_default_keys).
        key: None,
    },
    Entry::Separator { parent: Slot::View },
    Entry::Item {
        loc: loc::Id::MenuFullscreen,
        parent: Slot::View,
        cmd: Cmd::ViewFullscreen,
        key: Some(KeyDef {
            ctrl: false,
            alt: true,
            shift: false,
            vk: VK_RETURN,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuOneToOne,
        parent: Slot::View,
        cmd: Cmd::ViewOneToOne,
        key: Some(KeyDef {
            ctrl: true,
            alt: true,
            shift: false,
            vk: b'0' as u16,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuBestFit,
        parent: Slot::View,
        cmd: Cmd::ViewBestFit,
        key: None,
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
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: VK_OEM_PLUS,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuZoomOut,
        parent: Slot::ViewZoom,
        cmd: Cmd::ViewZoomOut,
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: VK_OEM_MINUS,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuZoomReset,
        parent: Slot::ViewZoom,
        cmd: Cmd::ViewZoomReset,
        key: Some(KeyDef {
            ctrl: true,
            alt: false,
            shift: false,
            vk: b'0' as u16,
        }),
    },
    Entry::Separator { parent: Slot::View },
    Entry::Item {
        loc: loc::Id::MenuOptions,
        parent: Slot::View,
        cmd: Cmd::ViewOptions,
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: b'O' as u16,
        }),
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
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: VK_RIGHT,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuPrevious,
        parent: Slot::Navigate,
        cmd: Cmd::NavPrev,
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: VK_LEFT,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuHome,
        parent: Slot::Navigate,
        cmd: Cmd::NavHome,
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: VK_HOME,
        }),
    },
    Entry::Item {
        loc: loc::Id::MenuEnd,
        parent: Slot::Navigate,
        cmd: Cmd::NavEnd,
        key: Some(KeyDef {
            ctrl: false,
            alt: false,
            shift: false,
            vk: VK_END,
        }),
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
        key: Some(KeyDef {
            ctrl: true,
            alt: false,
            shift: false,
            vk: VK_F1,
        }),
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
}

/// Whether `cmd`'s menu item carries a check in `state` (upstream
/// `_viv_check_menus`, viv.c:7123-7132 — only toggle-ish commands do).
pub(crate) fn checked(cmd: Cmd, state: &MenuState) -> bool {
    match cmd {
        Cmd::ViewMenu => state.show_menu,
        Cmd::ViewFullscreen => state.fullscreen,
        Cmd::ViewOneToOne => state.one_to_one,
        _ => false,
    }
}

/// Whether `cmd`'s menu item is selectable. Everything riviv registers is
/// always available (upstream never gates zoom/navigation on image state
/// either — its EnableMenuItem list, viv.c:7103-7121, covers clipboard/
/// delete/print commands riviv does not register); the sole exception is
/// the Options placeholder, greyed until the Options issue wires it.
pub(crate) fn enabled(cmd: Cmd) -> bool {
    cmd != Cmd::ViewOptions
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
    fn default_keys_are_globally_unique() {
        // One physical chord must not trigger two commands — upstream's
        // key list would deliver the key to whichever command matched
        // first (config.c's key ownership), so the table must never
        // register a duplicate.
        let mut seen: Vec<KeyDef> = Vec::new();
        for entry in ENTRIES {
            if let Entry::Item { key: Some(key), .. } = entry {
                assert!(
                    !seen.contains(key),
                    "key {key:?} bound to more than one command"
                );
                seen.push(*key);
            }
        }
    }

    #[test]
    fn key_labels_use_upstreams_modifier_order() {
        // _viv_get_key_text appends Ctrl, then Alt, then Shift (viv.c:
        // 12287-12301) before the key name.
        let chord = KeyDef {
            ctrl: true,
            alt: true,
            shift: true,
            vk: VK_F1,
        };
        assert_eq!(key_label(chord, "F1"), "Ctrl+Alt+Shift+F1");
        assert_eq!(
            key_label(
                KeyDef {
                    ctrl: false,
                    alt: true,
                    shift: false,
                    vk: VK_RETURN
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
                    vk: VK_RIGHT
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
        // viv.c:7125/7131/7132 — the Menu/Fullscreen/1:1 checks; every
        // other item is unchecked regardless of state.
        let on = MenuState {
            show_menu: true,
            fullscreen: true,
            one_to_one: true,
        };
        let off = MenuState {
            show_menu: false,
            fullscreen: false,
            one_to_one: false,
        };
        for cmd in Cmd::ALL {
            assert_eq!(
                checked(cmd, &on),
                matches!(cmd, Cmd::ViewMenu | Cmd::ViewFullscreen | Cmd::ViewOneToOne)
            );
            assert!(!checked(cmd, &off));
        }
    }

    #[test]
    fn options_is_the_only_disabled_item() {
        // The Options placeholder stays greyed until its issue wires the
        // dialog; everything else is always selectable (upstream gates
        // only clipboard/delete/print-style commands, none registered).
        for cmd in Cmd::ALL {
            assert_eq!(enabled(cmd), cmd != Cmd::ViewOptions, "{cmd:?}");
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
