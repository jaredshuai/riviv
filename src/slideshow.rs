//! Slideshow model (#37): the rate presets, the rate stepping, the
//! custom-rate composition, and the WM_TIMER advance gate — all pure
//! logic, unit-tested.
//!
//! Upstream keeps this as scattered globals + handlers: the preset table
//! `_viv_slideshow_rate_presets[]` (viv.c:711), the step walk
//! `_viv_increase_rate` (viv.c:7594-7630), the custom dialog's compose
//! (`_viv_set_custom_rate`, viv.c:7453-7483: value × unit multiplier,
//! floored at 1 ms), and the timer gate (viv.c:3143-3159: with
//! `loop_animations_once` on, an animation showing for the first time
//! holds the advance until it has looped once — the `_viv_is_slideshow_
//! timeup` flag, set by the gate and honored by the animation wrap
//! branch, viv.c:3243-3248). The status readout's unit picking
//! (viv.c:7040-7069) lands with the temp-text work (#47).

/// SetTimer id for the slideshow timer (upstream `VIV_ID_SLIDESHOW_TIMER`,
/// viv.h:191 — a command-enum value there, an app-private id here).
/// Distinct from the animation timer (anim.rs = 1) and the hide-cursor
/// timer (cursor.rs = 2).
pub(crate) const SLIDESHOW_TIMER_ID: usize = 3;

/// The 17 rate presets in menu order (viv.c:711-712).
pub(crate) const RATE_PRESETS: [u32; 17] = [
    250, 500, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000, 30000, 40000,
    50000, 60000,
];

/// Step to the neighboring preset (`_viv_increase_rate`, viv.c:7594-7630):
/// Decrease scans from the LARGEST preset for the first one strictly below
/// `current_ms`; Increase scans from the smallest for the first strictly
/// above. Already at (or past) the end in that direction → `None` (no
/// change, no wrap — the upstream walk simply finds nothing).
pub(crate) fn step_rate(current_ms: u32, decrease: bool) -> Option<u32> {
    if decrease {
        RATE_PRESETS
            .iter()
            .rev()
            .find(|&&p| p < current_ms)
            .copied()
    } else {
        RATE_PRESETS.iter().find(|&&p| p > current_ms).copied()
    }
}

/// The custom-rate unit (`config_slideshow_custom_rate_type`, config.c:79:
/// 0 = milliseconds, 1 = seconds, 2 = minutes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CustomRateUnit {
    Milliseconds,
    Seconds,
    Minutes,
}

impl CustomRateUnit {
    /// The combo-box index IS the type value (the dialog adds the three
    /// rows in type order, viv.c:7494-7496).
    pub(crate) fn type_value(self) -> i32 {
        match self {
            Self::Milliseconds => 0,
            Self::Seconds => 1,
            Self::Minutes => 2,
        }
    }

    fn multiplier(self) -> u32 {
        match self {
            Self::Milliseconds => 1,
            Self::Seconds => 1_000,
            Self::Minutes => 60_000,
        }
    }
}

/// Compose the effective rate from the dialog's value + unit
/// (`_viv_set_custom_rate`, viv.c:7463-7478): the product is computed in
/// upstream's 32-bit int (a huge typed value wraps, and MSVC wraps to a
/// negative) and then floored at 1 ms — upstream's `< 1` clamp, which
/// also catches the negative wrap results.
pub(crate) fn custom_rate(value: u32, unit: CustomRateUnit) -> u32 {
    let as_int = (value as u64 * u64::from(unit.multiplier())) as u32 as i32;
    if as_int < 1 { 1 } else { as_int as u32 }
}

/// Whether `rate_ms` is exactly one of the presets (the menu radio's
/// Custom row covers everything else, viv.c:7140-7157's switch default).
pub(crate) fn is_preset(rate_ms: u32) -> bool {
    RATE_PRESETS.contains(&rate_ms)
}

/// The slideshow WM_TIMER gate (viv.c:3143-3159): advance now, or — only
/// while `loop_animations_once` is on and the current display is an
/// animation that has not completed a full loop yet — hold the advance
/// and mark timeup (the animation wrap branch performs it later, viv.c:
/// 3243-3248). A static image never waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    Advance,
    WaitForLoop,
}

pub(crate) fn timer_gate(
    loop_animations_once: bool,
    is_animation: bool,
    looped_once: bool,
) -> Gate {
    if loop_animations_once && is_animation && !looped_once {
        Gate::WaitForLoop
    } else {
        Gate::Advance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is pinned verbatim — every preset row of the Rate submenu
    /// reads its milliseconds out of this order (viv.c:711-712).
    #[test]
    fn presets_match_the_upstream_table() {
        assert_eq!(
            RATE_PRESETS,
            [
                250, 500, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000,
                30000, 40000, 50000, 60000
            ]
        );
    }

    #[test]
    fn stepping_lands_on_the_neighbor_preset() {
        // From each preset, Decrease/Increase find the adjacent entries
        // (the upstream scans run strict inequalities, so an exact match
        // never returns the current value itself).
        assert_eq!(step_rate(5000, true), Some(4000));
        assert_eq!(step_rate(5000, false), Some(6000));
        assert_eq!(step_rate(250, false), Some(500));
        assert_eq!(step_rate(60_000, true), Some(50_000));
    }

    #[test]
    fn stepping_clamps_at_the_table_ends() {
        // Below the fastest / above the slowest preset the walk finds
        // nothing — the rate stays put (no wrap, viv.c:7594-7630).
        assert_eq!(step_rate(250, true), None);
        assert_eq!(step_rate(60_000, false), None);
        // A custom rate past the slow end is equally stuck.
        assert_eq!(step_rate(90_000, false), None);
    }

    #[test]
    fn stepping_snaps_a_custom_rate_to_the_neighbors() {
        // Between presets the strict-inequality scans find the surrounding
        // entries, not the nearest in menu distance.
        assert_eq!(step_rate(700, true), Some(500));
        assert_eq!(step_rate(700, false), Some(1000));
        assert_eq!(step_rate(15_000, true), Some(10_000));
        assert_eq!(step_rate(15_000, false), Some(20_000));
    }

    #[test]
    fn custom_rate_composes_value_times_unit() {
        // Defaults config ships: 3 × seconds = 3000 (config.c:78-79).
        assert_eq!(custom_rate(3, CustomRateUnit::Seconds), 3_000);
        assert_eq!(custom_rate(250, CustomRateUnit::Milliseconds), 250);
        assert_eq!(custom_rate(2, CustomRateUnit::Minutes), 120_000);
    }

    #[test]
    fn custom_rate_floors_at_one_millisecond() {
        // A zero/empty edit field composes 0 → clamped to 1 (viv.c:7475-
        // 7478: `if (config_slideshow_rate < 1) config_slideshow_rate = 1`).
        assert_eq!(custom_rate(0, CustomRateUnit::Seconds), 1);
        assert_eq!(custom_rate(0, CustomRateUnit::Milliseconds), 1);
    }

    #[test]
    fn custom_rate_wraps_like_the_c_int_before_the_floor() {
        // 200000 × 60000 = 12e9: the low 32 bits have the sign bit set, so
        // the C int assignment wraps NEGATIVE and the `< 1` clamp floors
        // it to 1 (MSVC wrap semantics; the clamp is the only guarantee
        // either build gives).
        assert_eq!(custom_rate(200_000, CustomRateUnit::Minutes), 1);
        // 100000 × 60000 = 6e9 wraps to a positive value below 2^31 — it
        // passes through untouched (6e9 − 2^32).
        assert_eq!(custom_rate(100_000, CustomRateUnit::Minutes), 1_705_032_704);
    }

    #[test]
    fn preset_membership_drives_the_radio() {
        // viv.c:7140-7157: the switch maps every preset to its row;
        // anything else (including 3000 composed from a custom dialog
        // pass) is the Custom row.
        assert!(is_preset(250));
        assert!(is_preset(60_000));
        assert!(!is_preset(700));
        assert!(!is_preset(0));
    }

    #[test]
    fn the_timer_gate_waits_only_for_a_first_loop() {
        // viv.c:3143-3159: without loop-once the gate never holds; with
        // it, an un-looped animation holds exactly once.
        assert_eq!(timer_gate(false, true, false), Gate::Advance);
        assert_eq!(timer_gate(true, false, false), Gate::Advance);
        assert_eq!(timer_gate(true, true, true), Gate::Advance);
        assert_eq!(timer_gate(true, true, false), Gate::WaitForLoop);
    }
}
