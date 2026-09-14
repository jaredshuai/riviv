//! Pan/Scan view model (pure logic, unit-tested) — issue #44.
//!
//! Upstream's panscan is a SECOND zoom/pan layer that rides on top of the
//! 16-level preset curve (viv.c:715-725): a per-axis float-factor index
//! (`_viv_dst_zoom_x_pos`/`_viv_dst_zoom_y_pos`, 0..=138, identity at
//! `_VIV_DST_ZOOM_ONE` = 82) and a pan position on a 0..=1000 scale where
//! 500 is center (`_viv_dst_pos_x`/`_viv_dst_pos_y`). The paint path
//! multiplies the preset-curve render size by the factors (viv.c:4139-4140)
//! and offsets the image center by the pan term (viv.c:4153) — the
//! drag-pan clamp and the keep-centered math stay in PRE-panscan
//! coordinates upstream (viv.c:6449-6462, `_viv_get_render_size` output
//! unmultiplied), so a panscanned image deliberately overflows the
//! drag-clamped bounds (the MPC-HC panscan feel).
//!
//! The state PERSISTS across image changes and blanks — upstream's
//! `_viv_clear` (viv.c:1270-1291) resets the preset zoom/view but never
//! touches these globals; only the sixteen Pan/Scan commands move them
//! (viv.c:2246-2313).
//!
//! The status-bar readout family (`_viv_status_update_temp_pos_zoom`,
//! viv.c:11760-11801) and the cursor pixel mapping
//! (`_viv_get_src_pixel_pos/_rgb`, viv.c:15020-15110) also consume this
//! state; those land with the full status bar (#47).

/// The factor-table size (upstream `_VIV_DST_ZOOM_MAX`, viv.c:721).
pub(crate) const DST_ZOOM_MAX: usize = 139;

/// The identity index — factor 1.0 (upstream `_VIV_DST_ZOOM_ONE`, viv.c:723).
pub(crate) const DST_ZOOM_ONE: usize = 82;

/// The factor at `index` — upstream's init walk (viv.c:5186-5205): from 82
/// downward each step divides by 1.02, upward multiplies, iterated `f32`
/// exactly as the C loop runs it (the per-index value is the product of
/// `|index - 82|` successive 1.02 factors, not a `powi` — last-ulp
/// faithful to the table `_viv_dst_zoom_values`).
pub(crate) fn value_at(index: usize) -> f32 {
    let mut f = 1.0f32;
    if index <= DST_ZOOM_ONE {
        for _ in index..DST_ZOOM_ONE {
            f /= 1.02;
        }
    } else {
        for _ in DST_ZOOM_ONE..index {
            f *= 1.02;
        }
    }
    f
}

/// One axis of the panscan center term (upstream viv.c:4153:
/// `((dst_pos - 250) * (wide*2)) / 1000`): the viewport point the image
/// centers on, `len / 2` at the default 500 (plain center) and ±`len / 2`
/// at the extremes. C integer division truncates toward zero; Rust `/`
/// matches for the negative half of the range.
pub(crate) fn center_term(len: i32, pos: i32) -> i32 {
    ((pos - 250) * (len * 2)) / 1000
}

/// The panscan-scaled length of one render axis (upstream viv.c:4139:
/// `rw = (int)(rw * _viv_dst_zoom_values[...])` — int→float promote, float
/// multiply, truncation back).
pub(crate) fn scale(len: i32, index: usize) -> i32 {
    ((len as f32) * value_at(index)) as i32
}

/// Pan/Scan state (upstream `_viv_dst_pos_x/y` + `_viv_dst_zoom_x/y_pos`,
/// viv.c:719-725). Copy: it rides inside `zoom::View` and must survive that
/// model's per-image reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Panscan {
    /// Pan position per axis, 0..=1000, 500 = center (`_viv_dst_pos_x/y`).
    pub(crate) pos_x: i32,
    pub(crate) pos_y: i32,
    /// Factor-table index per axis, 0..=138 (`_viv_dst_zoom_x/y_pos`).
    pub(crate) zoom_x: usize,
    pub(crate) zoom_y: usize,
}

impl Default for Panscan {
    fn default() -> Self {
        Panscan {
            pos_x: 500,
            pos_y: 500,
            zoom_x: DST_ZOOM_ONE,
            zoom_y: DST_ZOOM_ONE,
        }
    }
}

impl Panscan {
    /// The six size/width/height steps — `_viv_dst_zoom_set` (viv.c:9999-10047):
    /// clamp each index to 0..=138, and only a REAL change repaints (the
    /// unchanged call still updates the status readout upstream, which
    /// riviv defers to #47 — so the bool is purely repaint-owed).
    pub(crate) fn step(&mut self, dx: i32, dy: i32) -> bool {
        let x = (self.zoom_x as i32 + dx).clamp(0, DST_ZOOM_MAX as i32 - 1) as usize;
        let y = (self.zoom_y as i32 + dy).clamp(0, DST_ZOOM_MAX as i32 - 1) as usize;
        let changed = (x, y) != (self.zoom_x, self.zoom_y);
        self.zoom_x = x;
        self.zoom_y = y;
        changed
    }

    /// The eight Move arrows — `_viv_dst_pos_set` (viv.c:9966-9997): clamp
    /// each axis to 0..=1000, repaint only on a real change.
    pub(crate) fn pan(&mut self, dx: i32, dy: i32) -> bool {
        let x = (self.pos_x + dx).clamp(0, 1000);
        let y = (self.pos_y + dy).clamp(0, 1000);
        let changed = (x, y) != (self.pos_x, self.pos_y);
        self.pos_x = x;
        self.pos_y = y;
        changed
    }

    /// Move Center (viv.c:2302-2307): the position is written directly and
    /// the window invalidated UNCONDITIONALLY — no change check, unlike
    /// the pan/step setters.
    pub(crate) fn center(&mut self) -> bool {
        self.pos_x = 500;
        self.pos_y = 500;
        true
    }

    /// Pan/Scan → Reset (viv.c:2309-2313): position to center with an
    /// unconditional invalidate, then the factor indices through the
    /// change-checked `_viv_dst_zoom_set(ONE, ONE)` — a repaint is owed
    /// regardless of whether the indices moved.
    pub(crate) fn reset(&mut self) -> bool {
        self.pos_x = 500;
        self.pos_y = 500;
        self.zoom_x = DST_ZOOM_ONE;
        self.zoom_y = DST_ZOOM_ONE;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_index_is_one_and_neighbors_step_two_percent() {
        assert_eq!(value_at(DST_ZOOM_ONE), 1.0);
        // 1.02 and 1/1.02 are exact single steps of the init walk.
        assert_eq!(value_at(DST_ZOOM_ONE + 1), 1.0f32 * 1.02);
        assert_eq!(value_at(DST_ZOOM_ONE - 1), 1.0f32 / 1.02);
    }

    #[test]
    fn table_ends_match_the_iterated_walk() {
        // viv.c:5186-5205: values[0] is 82 successive /1.02, values[138]
        // is 56 successive *1.02 — pin the exact iterated products.
        let mut down = 1.0f32;
        for _ in 0..DST_ZOOM_ONE {
            down /= 1.02;
        }
        assert_eq!(value_at(0), down);
        let mut up = 1.0f32;
        for _ in DST_ZOOM_ONE..DST_ZOOM_MAX - 1 {
            up *= 1.02;
        }
        assert_eq!(value_at(DST_ZOOM_MAX - 1), up);
        // ...and the table is monotonic non-decreasing across the pivot.
        assert!(value_at(0) < value_at(1));
        assert!(value_at(DST_ZOOM_MAX - 2) < value_at(DST_ZOOM_MAX - 1));
    }

    #[test]
    fn scale_truncates_like_the_c_cast() {
        // (int)(rw * factor): 101 * 1.02 = 103.02 -> 103; 103 * (1/1.02)
        // = 100.98.. -> 100.
        assert_eq!(scale(101, DST_ZOOM_ONE + 1), 103);
        assert_eq!(scale(103, DST_ZOOM_ONE - 1), 100);
        assert_eq!(scale(400, DST_ZOOM_ONE), 400);
    }

    #[test]
    fn center_term_is_plain_center_at_five_hundred() {
        // The term collapses to len/2 at the default position (this is
        // why the pre-#44 paint math was center-identical).
        assert_eq!(center_term(400, 500), 200);
        assert_eq!(center_term(401, 500), 200);
    }

    #[test]
    fn center_term_spans_half_the_viewport_and_truncates_toward_zero() {
        // pos 0 -> -len/2, pos 1000 -> +3*len/2 (the image center rides
        // a full viewport wide; viv.c:234 "doubled offsets to match
        // mpchc"). Odd lengths: C truncation toward zero on the negative
        // side (-802*250/1000 = -200.5 -> -200, not -201).
        assert_eq!(center_term(401, 0), -200);
        assert_eq!(center_term(401, 1000), 601);
        assert_eq!(center_term(400, 0), -200);
        assert_eq!(center_term(400, 1000), 600);
    }

    #[test]
    fn step_clamps_to_the_table_bounds() {
        let mut p = Panscan::default();
        for _ in 0..DST_ZOOM_MAX * 2 {
            p.step(1, 0);
        }
        assert_eq!(p.zoom_x, DST_ZOOM_MAX - 1);
        for _ in 0..DST_ZOOM_MAX * 2 {
            p.step(0, -1);
        }
        assert_eq!(p.zoom_y, 0);
    }

    #[test]
    fn step_reports_change_only_when_an_index_moved() {
        let mut p = Panscan::default();
        assert!(p.step(1, 1)); // 82 -> 83
        assert!(!p.step(0, 0)); // no-op clamps to itself
        let mut pinned = Panscan {
            zoom_x: 0,
            zoom_y: DST_ZOOM_MAX - 1,
            ..Panscan::default()
        };
        assert!(!pinned.step(-1, 1)); // both clamps bite
    }

    #[test]
    fn pan_clamps_to_zero_through_a_thousand() {
        let mut p = Panscan::default();
        for _ in 0..300 {
            p.pan(5, -5);
        }
        assert_eq!((p.pos_x, p.pos_y), (1000, 0));
        assert!(!p.pan(5, -5)); // both clamped: no change
        p.pan(-1000, 1000);
        assert_eq!((p.pos_x, p.pos_y), (0, 1000));
    }

    #[test]
    fn pan_reports_change_only_when_a_position_moved() {
        let mut p = Panscan::default();
        assert!(p.pan(5, 0));
        assert!(!p.pan(0, 0));
    }

    #[test]
    fn center_and_reset_always_report_a_repaint() {
        // viv.c:2302-2313: both write the position directly and invalidate
        // unconditionally — even from the already-centered state.
        let mut p = Panscan::default();
        assert!(p.center());
        assert_eq!((p.pos_x, p.pos_y), (500, 500));
        p.pan(-200, 100);
        p.step(9, -9);
        assert!(p.reset());
        assert_eq!(p, Panscan::default());
        // ...and reset from the fresh state still repaints.
        assert!(p.reset());
    }

    #[test]
    fn default_is_centered_identity() {
        assert_eq!(
            Panscan::default(),
            Panscan {
                pos_x: 500,
                pos_y: 500,
                zoom_x: DST_ZOOM_ONE,
                zoom_y: DST_ZOOM_ONE,
            }
        );
    }
}
