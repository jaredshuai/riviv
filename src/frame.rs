//! View-frame pure decisions (#46): the Minimal/Compact/Normal preset →
//! five-config mapping behind the View→Preset rows and the CLI
//! `/minimal`/`/compact` switches, and the always-on-top verdict behind
//! `View→On Top` (upstream `_viv_update_ontop`'s switch, viv.c:9920-9940).
//!
//! The Win32 half lives in `window.rs`: `update_frame` applies a
//! [`FrameToggles`] to the live window (style bits, menu bar, status bar,
//! toolbar strip), and `update_ontop` pushes the [`ontop_active`] verdict
//! through SetWindowPos.

/// The five chrome toggles a preset or the five View rows flip (upstream
/// `config_show_menu`/`config_show_status`/`config_show_controls`/
/// `config_show_caption`/`config_show_thickframe`, config.c:56-57/85-86 —
/// all default 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameToggles {
    pub(crate) menu: bool,
    pub(crate) status: bool,
    pub(crate) controls: bool,
    pub(crate) caption: bool,
    pub(crate) thickframe: bool,
}

/// The three View→Preset rows (`VIV_ID_VIEW_PRESET_1/2/3`, keys '1'/'2'/'3',
/// viv.c:1990-2013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Preset {
    /// Minimal: everything off (viv.c:1990-1996).
    Minimal,
    /// Compact: only the thick frame stays — a border you can still resize
    /// (viv.c:1998-2004).
    Compact,
    /// Normal: everything on (viv.c:2006-2013).
    Normal,
}

impl Preset {
    /// The five-config combination the preset writes (viv.c:1990-2013 —
    /// all three arms assign every flag, no implicit carry-over).
    pub(crate) fn toggles(self) -> FrameToggles {
        match self {
            Self::Minimal => FrameToggles {
                menu: false,
                status: false,
                controls: false,
                caption: false,
                thickframe: false,
            },
            Self::Compact => FrameToggles {
                menu: false,
                status: false,
                controls: false,
                caption: false,
                thickframe: true,
            },
            Self::Normal => FrameToggles {
                menu: true,
                status: true,
                controls: true,
                caption: true,
                thickframe: true,
            },
        }
    }
}

/// Whether the window should sit topmost right now (upstream
/// `_viv_update_ontop`'s switch, viv.c:9926-9937): `1` = always, `2` = while
/// a slideshow runs or an animation plays (upstream: `_viv_is_slideshow ||
/// ((_viv_frame_count > 1) && (_viv_animation_play))` — the frame-count
/// guard means a static image never pins the window), anything else
/// (including the `0` default) = never.
pub(crate) fn ontop_active(config_ontop: i32, slideshow: bool, animating: bool) -> bool {
    match config_ontop {
        1 => true,
        2 => slideshow || animating,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_preset_turns_every_chrome_piece_off() {
        assert_eq!(
            Preset::Minimal.toggles(),
            FrameToggles {
                menu: false,
                status: false,
                controls: false,
                caption: false,
                thickframe: false
            }
        );
    }

    #[test]
    fn compact_preset_keeps_only_the_resize_border() {
        let t = Preset::Compact.toggles();
        assert!(t.thickframe);
        assert!(!t.menu && !t.status && !t.controls && !t.caption);
    }

    #[test]
    fn normal_preset_restores_the_full_window_chrome() {
        assert_eq!(
            Preset::Normal.toggles(),
            FrameToggles {
                menu: true,
                status: true,
                controls: true,
                caption: true,
                thickframe: true
            }
        );
    }

    #[test]
    fn ontop_always_is_topmost_whatever_plays() {
        assert!(ontop_active(1, false, false));
        assert!(ontop_active(1, true, false));
        assert!(ontop_active(1, false, true));
    }

    #[test]
    fn ontop_while_playing_follows_the_playback_flags() {
        assert!(!ontop_active(2, false, false));
        assert!(ontop_active(2, true, false));
        assert!(ontop_active(2, false, true));
        assert!(ontop_active(2, true, true));
    }

    #[test]
    fn ontop_zero_and_unknown_values_are_never_topmost() {
        assert!(!ontop_active(0, true, true));
        assert!(!ontop_active(3, true, true));
        assert!(!ontop_active(-1, false, false));
    }
}
