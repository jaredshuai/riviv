//! The Options dialog's pure half (#24): the editable settings model, the
//! mouse-action vocabularies, and the declarative control tables the Win32
//! shell assembles (`options_dlg.rs`).
//!
//! Upstream `_viv_options` (viv.c:8553-8807) reads/writes its `config_*`
//! globals straight from the dialog controls. riviv splits that in two:
//! [`OptionsModel`] snapshots the editable subset of [`crate::config::Config`]
//! (from_config on open), the shell moves controls ⇄ model, and
//! [`OptionsModel::commit`] writes back with an [`Effects`] changelist of the
//! runtime invalidations owed (upstream's InvalidateRect calls at OK,
//! viv.c:8721-8767).
//!
//! Scope vs upstream's pages: only implemented features get controls — the
//! General page drops start-menu shortcuts and associations (#26), the View
//! page drops title-bar-format / loop-once / preload / cache (features not
//! implemented), and keeps-aspect / fill / fullscreen-fill / frame-minus
//! move here from upstream's MENU surface (viv.c:2032-2060 / 3994-3999),
//! which riviv's menu does not register. The Controls page's key-binding
//! editor lives in `keys.rs` + the hand-built area of `options_dlg.rs`
//! (#25) — a multi-control composite, not one declarative Field.

use crate::config::Config;
use crate::loc;

/// A combo item: its localized label and the config value it stands for.
/// The combo INDEX is not the value — the table is the only mapping.
pub(crate) struct ComboEntry {
    pub(crate) label: loc::Id,
    pub(crate) value: i32,
}

/// The left-click actions riviv implements (upstream values, viv.c:6368-6415:
/// 0 scroll, 1 slideshow, 2 animation pause, 3 zoom in, 4 next, 5 1:1
/// scroll, 6 move-window). Unimplemented values stay valid INI values — they
/// just have no combo row, so the combo shows blank and OK preserves them.
/// Value 1 landed with #37.
pub(crate) const LEFT_CLICK_ACTIONS: &[ComboEntry] = &[
    ComboEntry {
        label: loc::Id::ActionScroll,
        value: 0,
    },
    ComboEntry {
        label: loc::Id::ActionPlayPauseSlideshow,
        value: 1,
    },
    ComboEntry {
        label: loc::Id::ActionZoomIn,
        value: 3,
    },
    ComboEntry {
        label: loc::Id::ActionNextImage,
        value: 4,
    },
];

/// The right-click actions riviv implements (upstream values, viv.c:3349-3361:
/// 0 context menu, 1 zoom out, 2 previous).
pub(crate) const RIGHT_CLICK_ACTIONS: &[ComboEntry] = &[
    ComboEntry {
        label: loc::Id::ActionContextMenu,
        value: 0,
    },
    ComboEntry {
        label: loc::Id::ActionZoomOut,
        value: 1,
    },
    ComboEntry {
        label: loc::Id::ActionPreviousImage,
        value: 2,
    },
];

/// The wheel actions riviv implements (upstream values, viv.c:14063-14090:
/// 0 zoom, 1 next/previous by delta sign, 2 previous/next).
pub(crate) const WHEEL_ACTIONS: &[ComboEntry] = &[
    ComboEntry {
        label: loc::Id::ActionZoom,
        value: 0,
    },
    ComboEntry {
        label: loc::Id::ActionNextPrev,
        value: 1,
    },
    ComboEntry {
        label: loc::Id::ActionPrevNext,
        value: 2,
    },
];

/// The auto-size combo (upstream `VIV_ID_VIEW_WINDOW_SIZE_50 + type`,
/// viv.c:2076+ — the index IS the config value here).
pub(crate) const AUTO_ZOOM_TYPES: &[ComboEntry] = &[
    ComboEntry {
        label: loc::Id::OptionsAutoZoom50,
        value: 0,
    },
    ComboEntry {
        label: loc::Id::OptionsAutoZoom100,
        value: 1,
    },
    ComboEntry {
        label: loc::Id::OptionsAutoZoom200,
        value: 2,
    },
    ComboEntry {
        label: loc::Id::OptionsAutoZoomAutoFit,
        value: 3,
    },
];

/// The blit-mode combos (both use value 0 = COLORONCOLOR "Nearest",
/// 1 = HALFTONE "Linear"; config.c:52-57).
pub(crate) const BLIT_MODES: &[ComboEntry] = &[
    ComboEntry {
        label: loc::Id::OptionsBlitNearest,
        value: 0,
    },
    ComboEntry {
        label: loc::Id::OptionsBlitLinear,
        value: 1,
    },
];

/// The combo index a config value maps to, if the table carries that value
/// (upstream `ComboBox_SetCurSel` with an out-of-range value leaves the
/// combo blank — CB_ERR — and a blank combo at OK preserves the value
/// rather than writing 0).
pub(crate) fn combo_index(entries: &[ComboEntry], value: i32) -> Option<usize> {
    entries.iter().position(|e| e.value == value)
}

/// What one control edits — the declarative link between the tables
/// (creation + read-back) and [`OptionsModel`]"'s fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Appdata,
    MultipleInstances,
    ShrinkBlitMode,
    MagFilter,
    KeepAspectRatio,
    FillWindow,
    FullscreenFillWindow,
    AutoZoom,
    AutoZoomType,
    FrameMinus,
    WindowedBg,
    FullscreenBg,
    LeftClickAction,
    RightClickAction,
    MouseWheelAction,
}

/// One dialog control, positioned in dialog units like upstream's rc
/// (c-original/res/voidImageViewer.rc — geometry cited per page).
pub(crate) enum Kind {
    /// BS_AUTOCHECKBOX — its label is the control text.
    Checkbox,
    /// A static label at (x, y, label_w, h) plus a dropdown-list combo at
    /// (combo_x, y, combo_w, drop_h) sharing the row.
    Combo(&'static [ComboEntry]),
    /// A static label plus an owner-drawn solid-color swatch button.
    ColorButton,
}

pub(crate) struct Ctrl {
    pub(crate) kind: Kind,
    /// The model field this control edits (the shell maps Field to/from
    /// control state on both the init and the OK pass).
    pub(crate) field: Field,
    /// The row's label text (checkbox caption, or the static beside a
    /// combo/color button).
    pub(crate) label: loc::Id,
    /// The label static's width in dialog units, 0 when the row has no
    /// static (a checkbox, or a combo that belongs to the row beside it —
    /// upstream rc IDD_VIEW has the auto-size combo at x=74 with the
    /// CHECKBOX carrying the row's text).
    pub(crate) label_w: i32,
    /// Row origin in dialog units.
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// The main control's size (checkbox extent, combo box, color button).
    pub(crate) w: i32,
    pub(crate) h: i32,
}

/// The three pages in tree order (upstream
/// `_viv_options_page_localization_id_array`, viv.c:736).
pub(crate) struct Page {
    pub(crate) title: loc::Id,
    pub(crate) controls: &'static [Ctrl],
}

/// General (rc IDD_GENERAL:29-53; only the two implemented rows).
pub(crate) const GENERAL: &[Ctrl] = &[
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsAppdata,
        field: Field::Appdata,
        label_w: 0,
        x: 0,
        y: 0,
        w: 186,
        h: 10,
    },
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsMultipleInstances,
        field: Field::MultipleInstances,
        label_w: 0,
        x: 0,
        y: 18,
        w: 186,
        h: 10,
    },
];

/// View (rc IDD_VIEW:68-89, rows re-ordered for the keep-aspect/fill
/// checkboxes upstream keeps on its menu).
pub(crate) const VIEW: &[Ctrl] = &[
    Ctrl {
        kind: Kind::Combo(BLIT_MODES),
        label: loc::Id::OptionsShrinkBlitMode,
        field: Field::ShrinkBlitMode,
        label_w: 74,
        x: 0,
        y: 0,
        w: 119,
        h: 30,
    },
    Ctrl {
        kind: Kind::Combo(BLIT_MODES),
        label: loc::Id::OptionsMagnifyBlitMode,
        field: Field::MagFilter,
        label_w: 74,
        x: 0,
        y: 17,
        w: 119,
        h: 30,
    },
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsKeepAspectRatio,
        field: Field::KeepAspectRatio,
        label_w: 0,
        x: 0,
        y: 36,
        w: 186,
        h: 10,
    },
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsFillWindow,
        field: Field::FillWindow,
        label_w: 0,
        x: 0,
        y: 53,
        w: 186,
        h: 10,
    },
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsFullscreenFill,
        field: Field::FullscreenFillWindow,
        label_w: 0,
        x: 0,
        y: 70,
        w: 186,
        h: 10,
    },
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsAutoZoom,
        field: Field::AutoZoom,
        label_w: 0,
        x: 0,
        y: 87,
        w: 74,
        h: 10,
    },
    Ctrl {
        kind: Kind::Combo(AUTO_ZOOM_TYPES),
        label: loc::Id::OptionsAutoZoom,
        field: Field::AutoZoomType,
        label_w: 0,
        x: 74,
        y: 85,
        w: 60,
        h: 100,
    },
    Ctrl {
        kind: Kind::Checkbox,
        label: loc::Id::OptionsFrameMinus,
        field: Field::FrameMinus,
        label_w: 0,
        x: 0,
        y: 104,
        w: 186,
        h: 10,
    },
    Ctrl {
        kind: Kind::ColorButton,
        label: loc::Id::OptionsWindowedBg,
        field: Field::WindowedBg,
        label_w: 96,
        x: 0,
        y: 122,
        w: 50,
        h: 14,
    },
    Ctrl {
        kind: Kind::ColorButton,
        label: loc::Id::OptionsFullscreenBg,
        field: Field::FullscreenBg,
        label_w: 96,
        x: 0,
        y: 140,
        w: 50,
        h: 14,
    },
];

/// Controls (rc IDD_CONTROLS:91-115; the three action combos — the
/// key-binding editor area is hand-built in `options_dlg.rs` beside this
/// table, #25).
pub(crate) const CONTROLS: &[Ctrl] = &[
    Ctrl {
        kind: Kind::Combo(LEFT_CLICK_ACTIONS),
        label: loc::Id::OptionsLeftClickAction,
        field: Field::LeftClickAction,
        label_w: 74,
        x: 0,
        y: 0,
        w: 119,
        h: 100,
    },
    Ctrl {
        kind: Kind::Combo(RIGHT_CLICK_ACTIONS),
        label: loc::Id::OptionsRightClickAction,
        field: Field::RightClickAction,
        label_w: 74,
        x: 0,
        y: 17,
        w: 119,
        h: 100,
    },
    Ctrl {
        kind: Kind::Combo(WHEEL_ACTIONS),
        label: loc::Id::OptionsMouseWheelAction,
        field: Field::MouseWheelAction,
        label_w: 74,
        x: 0,
        y: 34,
        w: 119,
        h: 100,
    },
];

pub(crate) const PAGES: [Page; 3] = [
    Page {
        title: loc::Id::OptionsGeneral,
        controls: GENERAL,
    },
    Page {
        title: loc::Id::OptionsView,
        controls: VIEW,
    },
    Page {
        title: loc::Id::OptionsControls,
        controls: CONTROLS,
    },
];

/// The editable snapshot. Combo-backed fields are `Option<i32>` of the
/// config VALUE: `None` means "the combo could not represent the current
/// value" (an ini value outside the implemented set) — commit then leaves
/// the config key alone instead of coercing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionsModel {
    pub(crate) appdata: bool,
    pub(crate) multiple_instances: bool,
    pub(crate) shrink_blit_mode: Option<i32>,
    pub(crate) mag_filter: Option<i32>,
    pub(crate) keep_aspect_ratio: bool,
    pub(crate) fill_window: bool,
    pub(crate) fullscreen_fill_window: bool,
    pub(crate) auto_zoom: bool,
    pub(crate) auto_zoom_type: Option<i32>,
    pub(crate) frame_minus: bool,
    pub(crate) windowed_bg: [u8; 3],
    pub(crate) fullscreen_bg: [u8; 3],
    pub(crate) left_click_action: Option<i32>,
    pub(crate) right_click_action: Option<i32>,
    pub(crate) mouse_wheel_action: Option<i32>,
}

impl OptionsModel {
    /// The checkbox half of the field-typed accessors: exactly one family
    /// (bool / value / color) handles each Field, pinned by the round-trip
    /// test below — the Win32 shell loops over the control table through
    /// these instead of restating the mapping (derivable logic lives here,
    /// tested, not in the unsafe shell). `None` = not a bool field.
    pub(crate) fn get_bool(&self, field: Field) -> Option<bool> {
        Some(match field {
            Field::Appdata => self.appdata,
            Field::MultipleInstances => self.multiple_instances,
            Field::KeepAspectRatio => self.keep_aspect_ratio,
            Field::FillWindow => self.fill_window,
            Field::FullscreenFillWindow => self.fullscreen_fill_window,
            Field::AutoZoom => self.auto_zoom,
            Field::FrameMinus => self.frame_minus,
            _ => return None,
        })
    }

    pub(crate) fn set_bool(&mut self, field: Field, value: bool) {
        match field {
            Field::Appdata => self.appdata = value,
            Field::MultipleInstances => self.multiple_instances = value,
            Field::KeepAspectRatio => self.keep_aspect_ratio = value,
            Field::FillWindow => self.fill_window = value,
            Field::FullscreenFillWindow => self.fullscreen_fill_window = value,
            Field::AutoZoom => self.auto_zoom = value,
            Field::FrameMinus => self.frame_minus = value,
            _ => {}
        }
    }

    /// The combo half (the raw config VALUE, not the combo index).
    pub(crate) fn get_value(&self, field: Field) -> Option<i32> {
        match field {
            Field::ShrinkBlitMode => self.shrink_blit_mode,
            Field::MagFilter => self.mag_filter,
            Field::AutoZoomType => self.auto_zoom_type,
            Field::LeftClickAction => self.left_click_action,
            Field::RightClickAction => self.right_click_action,
            Field::MouseWheelAction => self.mouse_wheel_action,
            _ => None,
        }
    }

    pub(crate) fn set_value(&mut self, field: Field, value: i32) {
        match field {
            Field::ShrinkBlitMode => self.shrink_blit_mode = Some(value),
            Field::MagFilter => self.mag_filter = Some(value),
            Field::AutoZoomType => self.auto_zoom_type = Some(value),
            Field::LeftClickAction => self.left_click_action = Some(value),
            Field::RightClickAction => self.right_click_action = Some(value),
            Field::MouseWheelAction => self.mouse_wheel_action = Some(value),
            _ => {}
        }
    }

    /// The color-swatch fields.
    pub(crate) fn get_color(&self, field: Field) -> Option<[u8; 3]> {
        match field {
            Field::WindowedBg => Some(self.windowed_bg),
            Field::FullscreenBg => Some(self.fullscreen_bg),
            _ => None,
        }
    }

    pub(crate) fn set_color(&mut self, field: Field, value: [u8; 3]) {
        match field {
            Field::WindowedBg => self.windowed_bg = value,
            Field::FullscreenBg => self.fullscreen_bg = value,
            _ => {}
        }
    }

    /// Snapshot the editable subset on open (upstream WM_INITDIALOG's
    /// CheckDlgButton/ComboBox_SetCurSel block, viv.c:7938-7990).
    pub(crate) fn from_config(config: &Config) -> OptionsModel {
        let to_bool = |v: i32| v != 0;
        OptionsModel {
            appdata: to_bool(config.appdata),
            multiple_instances: to_bool(config.multiple_instances),
            shrink_blit_mode: Some(config.shrink_blit_mode),
            mag_filter: Some(config.mag_filter),
            keep_aspect_ratio: to_bool(config.keep_aspect_ratio),
            fill_window: to_bool(config.fill_window),
            fullscreen_fill_window: to_bool(config.fullscreen_fill_window),
            auto_zoom: to_bool(config.auto_zoom),
            auto_zoom_type: Some(config.auto_zoom_type),
            frame_minus: to_bool(config.frame_minus),
            windowed_bg: config.windowed_bg(),
            fullscreen_bg: config.fullscreen_bg(),
            left_click_action: Some(config.left_click_action),
            right_click_action: Some(config.right_click_action),
            mouse_wheel_action: Some(config.mouse_wheel_action),
        }
    }

    /// Write back into the live config, reporting the runtime invalidations
    /// owed (upstream's IDOK block, viv.c:8641-8786: the InvalidateRect
    /// calls fire for filter/color changes; the rest takes effect at the
    /// next read). `repaint` also covers the fit inputs (keep-aspect/fill),
    /// whose render-size change upstream picks up through the same
    /// InvalidateRect discipline on its View-menu toggles (viv.c:2032-2051).
    /// `None` combo fields leave their key alone (an ini value outside the
    /// implemented set — the blank combo must not coerce it to 0).
    pub(crate) fn commit(&self, config: &mut Config) -> Effects {
        // Both verdicts compare against the config as it stands NOW —
        // before this method writes anything.
        let repaint = repaint_filters_colors(config, self);
        let refit = fit_inputs_changed(config, self);
        if let Some(v) = self.shrink_blit_mode {
            config.shrink_blit_mode = v;
        }
        if let Some(v) = self.mag_filter {
            config.mag_filter = v;
        }
        if let Some(v) = self.auto_zoom_type {
            config.auto_zoom_type = v;
        }
        if let Some(v) = self.left_click_action {
            config.left_click_action = v;
        }
        if let Some(v) = self.right_click_action {
            config.right_click_action = v;
        }
        if let Some(v) = self.mouse_wheel_action {
            config.mouse_wheel_action = v;
        }
        config.appdata = i32::from(self.appdata);
        config.multiple_instances = i32::from(self.multiple_instances);
        config.keep_aspect_ratio = i32::from(self.keep_aspect_ratio);
        config.fill_window = i32::from(self.fill_window);
        config.fullscreen_fill_window = i32::from(self.fullscreen_fill_window);
        config.auto_zoom = i32::from(self.auto_zoom);
        config.frame_minus = i32::from(self.frame_minus);
        config.windowed_background_color_r = i32::from(self.windowed_bg[0]);
        config.windowed_background_color_g = i32::from(self.windowed_bg[1]);
        config.windowed_background_color_b = i32::from(self.windowed_bg[2]);
        config.fullscreen_background_color_r = i32::from(self.fullscreen_bg[0]);
        config.fullscreen_background_color_g = i32::from(self.fullscreen_bg[1]);
        config.fullscreen_background_color_b = i32::from(self.fullscreen_bg[2]);
        Effects { repaint, refit }
    }
}

/// The filter/color/fit repaint decision against the config as it stands
/// when commit starts (call before writing those fields).
fn fit_inputs_changed(config: &Config, model: &OptionsModel) -> bool {
    model.keep_aspect_ratio != (config.keep_aspect_ratio != 0)
        || model.fill_window != (config.fill_window != 0)
        || model.fullscreen_fill_window != (config.fullscreen_fill_window != 0)
}

fn repaint_filters_colors(config: &Config, model: &OptionsModel) -> bool {
    let filters = model
        .shrink_blit_mode
        .is_some_and(|v| v != config.shrink_blit_mode)
        || model.mag_filter.is_some_and(|v| v != config.mag_filter);
    let colors =
        model.windowed_bg != config.windowed_bg() || model.fullscreen_bg != config.fullscreen_bg();
    filters || fit_inputs_changed(config, model) || colors
}

/// What the shell owes after an OK commit (upstream viv.c:8721-8767: a
/// whole-client InvalidateRect per changed filter/color).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Effects {
    /// A whole-client repaint is owed (filter/fit/color change).
    pub(crate) repaint: bool,
    /// The fit inputs changed (keep-aspect/fill): the pan offset must be
    /// re-anchored against the new render size before that repaint —
    /// upstream's FILL WINDOW menu command pairs its flip with
    /// `_viv_on_size()` for exactly this (viv.c:2032-2051); riviv exposes
    /// the toggle through Options, so OK performs the pairing.
    pub(crate) refit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_round_trips_through_exactly_one_accessor_family() {
        // The Field⇄model mapping the Win32 shell leans on: each control
        // table entry is served by one family, and a set reads back out.
        let mut m = OptionsModel::from_config(&Config::default());
        let mut bools = 0;
        let mut values = 0;
        let mut colors = 0;
        for page in &PAGES {
            for ctrl in page.controls {
                if let Some(v) = m.get_bool(ctrl.field) {
                    bools += 1;
                    let flipped = !v;
                    m.set_bool(ctrl.field, flipped);
                    assert_eq!(m.get_bool(ctrl.field), Some(flipped), "{:?}", ctrl.field);
                } else if let Some(v) = m.get_value(ctrl.field) {
                    values += 1;
                    m.set_value(ctrl.field, v.wrapping_add(1));
                    assert_eq!(
                        m.get_value(ctrl.field),
                        Some(v.wrapping_add(1)),
                        "{:?}",
                        ctrl.field
                    );
                } else if let Some(c) = m.get_color(ctrl.field) {
                    colors += 1;
                    m.set_color(ctrl.field, [c[0] ^ 1, c[1] ^ 2, c[2] ^ 3]);
                    assert_ne!(m.get_color(ctrl.field), Some(c), "{:?}", ctrl.field);
                } else {
                    panic!("field {:?} has no accessor", ctrl.field);
                }
            }
        }
        assert_eq!((bools, values, colors), (7, 6, 2));
    }

    #[test]
    fn action_tables_carry_only_implemented_values_in_upstream_order() {
        // The values are upstream's action numbering; the ORDER is the
        // combo order (upstream's AddString order, viv.c:8225-8252). The
        // left-click slideshow row landed with #37.
        let values = |t: &[ComboEntry]| t.iter().map(|e| e.value).collect::<Vec<_>>();
        assert_eq!(values(LEFT_CLICK_ACTIONS), vec![0, 1, 3, 4]);
        assert_eq!(values(RIGHT_CLICK_ACTIONS), vec![0, 1, 2]);
        assert_eq!(values(WHEEL_ACTIONS), vec![0, 1, 2]);
        assert_eq!(values(AUTO_ZOOM_TYPES), vec![0, 1, 2, 3]);
        assert_eq!(values(BLIT_MODES), vec![0, 1]);
    }

    #[test]
    fn combo_index_maps_values_and_rejects_the_unimplemented() {
        assert_eq!(combo_index(LEFT_CLICK_ACTIONS, 0), Some(0));
        assert_eq!(combo_index(LEFT_CLICK_ACTIONS, 1), Some(1));
        assert_eq!(combo_index(LEFT_CLICK_ACTIONS, 4), Some(3));
        // The animation/1:1/move values have no row (not implemented):
        // the combo shows blank and OK must preserve them.
        assert_eq!(combo_index(LEFT_CLICK_ACTIONS, 2), None);
        assert_eq!(combo_index(LEFT_CLICK_ACTIONS, 5), None);
        assert_eq!(combo_index(AUTO_ZOOM_TYPES, 7), None);
        assert_eq!(combo_index(BLIT_MODES, 2), None);
    }

    #[test]
    fn every_model_field_has_exactly_one_control() {
        // The tables are the single source for both creation and the OK
        // read-back: a Field missing here means the setting is unreachable
        // in the dialog, a duplicate means two controls fight over it.
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for page in &PAGES {
            for c in page.controls {
                assert!(
                    seen.insert(format!("{:?}", c.field)),
                    "duplicate field {:?}",
                    c.field
                );
            }
        }
        assert_eq!(seen.len(), 15);
    }

    #[test]
    fn every_control_has_a_localized_label_in_both_languages() {
        for page in &PAGES {
            assert!(!loc::get_for(loc::Language::English, page.title).is_empty());
            for c in page.controls {
                assert!(
                    !loc::get_for(loc::Language::English, c.label).is_empty(),
                    "{:?}",
                    c.label
                );
                assert!(
                    !loc::get_for(loc::Language::ChineseSimplified, c.label).is_empty(),
                    "{:?}",
                    c.label
                );
                if let Kind::Combo(entries) = c.kind {
                    assert!(!entries.is_empty());
                    for e in entries {
                        assert!(!loc::get_for(loc::Language::English, e.label).is_empty());
                        assert!(
                            !loc::get_for(loc::Language::ChineseSimplified, e.label).is_empty()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_row_sits_on_its_own_line_in_dialog_units() {
        // The shell stacks controls straight from these y values: no two
        // rows share a line, and nothing exceeds the 214-dlu page height
        // (upstream's IDC_PAGEPLACEHOLDER, rc:67). A combo's `h` is its
        // OPEN dropdown extent (like the rc's 30/100), not its row height,
        // so only `y` participates here.
        for page in &PAGES {
            let mut ys: Vec<i32> = page.controls.iter().map(|c| c.y).collect();
            ys.sort_unstable();
            ys.dedup();
            assert_eq!(ys.len(), page.controls.len(), "two rows share a y");
            assert!(*ys.last().unwrap() < 214, "row below the page area");
        }
    }

    #[test]
    fn commit_round_trips_every_field_and_flags_a_repaint() {
        let mut config = Config::default();
        let model = OptionsModel {
            appdata: true,
            multiple_instances: true,
            shrink_blit_mode: Some(0),
            mag_filter: Some(1),
            keep_aspect_ratio: false,
            fill_window: true,
            fullscreen_fill_window: false,
            auto_zoom: true,
            auto_zoom_type: Some(2),
            frame_minus: true,
            windowed_bg: [10, 20, 30],
            fullscreen_bg: [1, 2, 3],
            left_click_action: Some(3),
            right_click_action: Some(2),
            mouse_wheel_action: Some(1),
        };
        let effects = model.commit(&mut config);
        assert!(effects.repaint, "filter/fit/color changes repaint");
        assert_eq!(config.appdata, 1);
        assert_eq!(config.multiple_instances, 1);
        assert_eq!(config.shrink_blit_mode, 0);
        assert_eq!(config.mag_filter, 1);
        assert_eq!(config.keep_aspect_ratio, 0);
        assert_eq!(config.fill_window, 1);
        assert_eq!(config.fullscreen_fill_window, 0);
        assert_eq!(config.auto_zoom, 1);
        assert_eq!(config.auto_zoom_type, 2);
        assert_eq!(config.frame_minus, 1);
        assert_eq!(config.windowed_background_color_r, 10);
        assert_eq!(config.fullscreen_background_color_b, 3);
        assert_eq!(config.left_click_action, 3);
        assert_eq!(config.right_click_action, 2);
        assert_eq!(config.mouse_wheel_action, 1);
        // ...and a clean round trip back out of the mutated config.
        assert_eq!(OptionsModel::from_config(&config), model);
    }

    #[test]
    fn commit_of_unchanged_settings_owes_nothing() {
        let config = Config::default();
        let mut c2 = config.clone();
        let effects = OptionsModel::from_config(&config).commit(&mut c2);
        assert!(!effects.repaint);
        assert_eq!(c2, config);
    }

    #[test]
    fn none_combo_fields_preserve_unrepresented_ini_values() {
        // An ini hand-edited to an unimplemented action (5 = 1:1 scroll)
        // or an out-of-range type (7): the blank combo yields None and OK
        // must NOT coerce the key to 0.
        let mut config = Config {
            left_click_action: 5,
            auto_zoom_type: 7,
            shrink_blit_mode: 9,
            ..Config::default()
        };
        let model = OptionsModel {
            left_click_action: None,
            auto_zoom_type: None,
            shrink_blit_mode: None,
            ..OptionsModel::from_config(&config)
        };
        let effects = model.commit(&mut config);
        assert!(!effects.repaint);
        assert_eq!(config.left_click_action, 5);
        assert_eq!(config.auto_zoom_type, 7);
        assert_eq!(config.shrink_blit_mode, 9);
    }
}
