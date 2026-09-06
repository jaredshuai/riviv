//! Cursor visibility state machine (pure logic, unit-tested) — issue #8.
//!
//! Mirrors upstream's cursor globals (`_viv_is_cursor_shown` /
//! `_viv_is_hide_cursor_timer`, viv.c:709/713) and the four functions over
//! them (viv.c:14559-14640) plus the WM_TIMER arm (viv.c:3161-3169) and the
//! mouse-move dedupe (`_viv_mousemove`, viv.c:9151-9170):
//! - the cursor hides after 2 s idle (`_VIV_HIDE_CURSOR_DELAY`, viv.c:337)
//!   — but only when a viewable image is up, the window is foreground, the
//!   mouse is over it, nothing holds the capture, and fullscreen is on (or
//!   the `windowed_hide_cursor` config, default 1 upstream — riviv wires
//!   that input to `false`, a documented README deviation);
//! - any real mouse movement (a CHANGED cursor position) shows it again and
//!   restarts the cycle;
//! - the Win32 side (`ShowCursor` / `SetTimer` / `KillTimer`) is decided
//!   here as effects and performed by the window shell.

/// Idle delay before the cursor hides (upstream `_VIV_HIDE_CURSOR_DELAY`,
/// viv.c:337).
pub(crate) const HIDE_CURSOR_DELAY_MS: u32 = 2000;

/// WM_TIMER id for the hide-cursor timer (upstream `VIV_ID_HIDE_CURSOR_TIMER`;
/// riviv's animation timer is 1).
pub(crate) const HIDE_CURSOR_TIMER_ID: usize = 2;

/// The live inputs of `_viv_should_show_cursor` (viv.c:14593-14619).
/// `_viv_in_popup_menu` is always false in riviv (no context menu yet), so
/// it is omitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CursorConditions {
    /// A current file is open and neither failure flag is set (upstream:
    /// `*fd->cFileName && !file_not_found && !load_failed`).
    pub(crate) has_viewable_image: bool,
    /// `GetForegroundWindow() == hwnd`.
    pub(crate) foreground: bool,
    /// The mouse is over our window (`_viv_is_mouseover`, kept true by
    /// WM_MOUSEMOVE and false by WM_MOUSELEAVE).
    pub(crate) mouseover: bool,
    /// This thread holds the mouse capture (a drag pan in progress).
    pub(crate) captured: bool,
    /// Fullscreen is on.
    pub(crate) fullscreen: bool,
    /// The `windowed_hide_cursor` config (default 1 upstream). riviv wires
    /// `false` — the cursor never hides while windowed (README deviation).
    pub(crate) hide_when_windowed: bool,
}

/// The Win32 calls one state-machine step owes: at most one `ShowCursor`
/// polarity change, and timer start/kill flags. Upstream ordering — the
/// timer dies first, then the cursor polarity flips, then a fresh timer
/// may start — is preserved by the shell applying fields in that order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CursorEffects {
    /// `Some(v)`: `ShowCursor(v)` is due (the display-count changed).
    pub(crate) show_cursor: Option<bool>,
    /// `KillTimer(HIDE_CURSOR_TIMER_ID)` is due.
    pub(crate) kill_timer: bool,
    /// `SetTimer(HIDE_CURSOR_TIMER_ID, 2000)` is due.
    pub(crate) start_timer: bool,
}

impl CursorEffects {
    /// Fold a later step's effects into an earlier one (mouse-move does
    /// show-then-maybe-restart as one step, exactly like upstream's two
    /// back-to-back calls collapsing into a kill+start pair).
    fn merge(&mut self, later: CursorEffects) {
        if later.show_cursor.is_some() {
            self.show_cursor = later.show_cursor;
        }
        self.kill_timer |= later.kill_timer;
        self.start_timer |= later.start_timer;
    }
}

/// Cursor visibility + hide-timer state (upstream `_viv_is_cursor_shown`,
/// initialized 1, and `_viv_is_hide_cursor_timer`, initialized 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CursorVisibility {
    shown: bool,
    hide_timer: bool,
}

impl Default for CursorVisibility {
    fn default() -> Self {
        CursorVisibility::new()
    }
}

impl CursorVisibility {
    pub(crate) fn new() -> Self {
        CursorVisibility {
            shown: true,
            hide_timer: false,
        }
    }

    /// `_viv_should_show_cursor` (viv.c:14593-14619): show UNLESS every hide
    /// condition holds at once.
    pub(crate) fn should_show(&self, c: &CursorConditions) -> bool {
        !(c.has_viewable_image
            && c.foreground
            && c.mouseover
            && !c.captured
            && (c.fullscreen || c.hide_when_windowed))
    }

    /// `_viv_show_cursor` (viv.c:14559-14571): kill the timer, show if hidden.
    pub(crate) fn show(&mut self) -> CursorEffects {
        let mut e = CursorEffects::default();
        if self.hide_timer {
            self.hide_timer = false;
            e.kill_timer = true;
        }
        if !self.shown {
            self.shown = true;
            e.show_cursor = Some(true);
        }
        e
    }

    /// `_viv_hide_cursor` (viv.c:14573-14585): kill the timer, hide if shown.
    fn hide(&mut self) -> CursorEffects {
        let mut e = CursorEffects::default();
        if self.hide_timer {
            self.hide_timer = false;
            e.kill_timer = true;
        }
        if self.shown {
            self.shown = false;
            e.show_cursor = Some(false);
        }
        e
    }

    /// `_viv_start_hide_cursor_timer` (viv.c:14633-14641): arm once.
    fn start_hide_timer(&mut self) -> CursorEffects {
        let mut e = CursorEffects::default();
        if !self.hide_timer {
            self.hide_timer = true;
            e.start_timer = true;
        }
        e
    }

    /// `_viv_update_show_cursor` (viv.c:14621-14628): reconcile with the
    /// conditions — show, or (re)arm the hide timer.
    pub(crate) fn update(&mut self, c: &CursorConditions) -> CursorEffects {
        if self.should_show(c) {
            self.show()
        } else {
            self.start_hide_timer()
        }
    }

    /// The WM_TIMER hide-cursor arm (viv.c:3161-3169): fire only while the
    /// timer is armed AND the conditions still say hide. When the conditions
    /// reverted, upstream leaves the timer RUNNING (the flag stays set and
    /// the next fire no-ops again) — replicated here on purpose.
    pub(crate) fn timer_fired(&mut self, c: &CursorConditions) -> CursorEffects {
        if self.hide_timer && !self.should_show(c) {
            self.hide()
        } else {
            CursorEffects::default()
        }
    }

    /// `_viv_mousemove` (viv.c:9151-9170): a real movement (the cursor
    /// POSITION changed since the last look — the caller owns that dedupe)
    /// shows the cursor and, if the conditions say hide, restarts the 2 s
    /// cycle. A stationary "move" is upstream's dedupe no-op.
    pub(crate) fn mouse_moved(&mut self, moved: bool, c: &CursorConditions) -> CursorEffects {
        if !moved {
            return CursorEffects::default();
        }
        let mut e = self.show();
        if !self.should_show(c) {
            e.merge(self.start_hide_timer());
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The conditions that let the cursor hide (upstream's every-gate-at-once).
    const HIDING: CursorConditions = CursorConditions {
        has_viewable_image: true,
        foreground: true,
        mouseover: true,
        captured: false,
        fullscreen: true,
        hide_when_windowed: false,
    };

    #[test]
    fn fresh_state_is_shown_with_no_timer() {
        // _viv_is_cursor_shown = 1, _viv_is_hide_cursor_timer = 0 (viv.c:709/713).
        assert_eq!(
            CursorVisibility::new(),
            CursorVisibility {
                shown: true,
                hide_timer: false
            }
        );
    }

    #[test]
    fn hides_only_when_every_condition_holds() {
        // viv.c:14593-14619 — flip any single gate and the cursor stays.
        let v = CursorVisibility::new();
        assert!(!v.should_show(&HIDING));
        for mutate in [
            CursorConditions {
                has_viewable_image: false,
                ..HIDING
            },
            CursorConditions {
                foreground: false,
                ..HIDING
            },
            CursorConditions {
                mouseover: false,
                ..HIDING
            },
            CursorConditions {
                captured: true,
                ..HIDING
            },
            CursorConditions {
                fullscreen: false,
                ..HIDING
            },
        ] {
            assert!(v.should_show(&mutate), "should show with {mutate:?}");
        }
    }

    #[test]
    fn windowed_mode_never_hides_by_default() {
        // riviv's wiring: hide_when_windowed = false (upstream config default
        // is 1 — the documented deviation). With fullscreen off, every other
        // gate may hold and the cursor still shows.
        let v = CursorVisibility::new();
        let windowed = CursorConditions {
            fullscreen: false,
            hide_when_windowed: false,
            ..HIDING
        };
        assert!(v.should_show(&windowed));
        // ...while the upstream default WOULD hide.
        let upstream_default = CursorConditions {
            hide_when_windowed: true,
            ..windowed
        };
        assert!(!v.should_show(&upstream_default));
    }

    #[test]
    fn update_arms_the_two_second_cycle_when_hiding() {
        let mut v = CursorVisibility::new();
        assert_eq!(
            v.update(&HIDING),
            CursorEffects {
                start_timer: true,
                ..Default::default()
            }
        );
        // update is edge-managed: the second call does not re-arm.
        assert_eq!(v.update(&HIDING), CursorEffects::default());
    }

    #[test]
    fn timer_fire_after_idle_hides_and_kills_the_timer() {
        // The full cycle: update arms, the timer fires with the conditions
        // still hiding -> hide (ShowCursor(FALSE) + KillTimer, viv.c:3161-3169).
        let mut v = CursorVisibility::new();
        v.update(&HIDING);
        assert_eq!(
            v.timer_fired(&HIDING),
            CursorEffects {
                show_cursor: Some(false),
                kill_timer: true,
                start_timer: false,
            }
        );
        // A second fire with nothing re-armed is inert.
        assert_eq!(v.timer_fired(&HIDING), CursorEffects::default());
    }

    #[test]
    fn timer_fire_with_conditions_reverted_is_a_noop_that_leaves_the_timer_armed() {
        // Upstream quirk (viv.c:3161-3169): the timer is never killed by its
        // own fire when should_show came back true — it keeps firing no-ops
        // until a show/update path kills it.
        let mut v = CursorVisibility::new();
        v.update(&HIDING);
        let reverted = CursorConditions {
            mouseover: false,
            ..HIDING
        };
        assert_eq!(v.timer_fired(&reverted), CursorEffects::default());
        // The flag stayed armed, so a later fire (conditions hiding again)
        // still delivers the hide.
        assert_eq!(v.timer_fired(&HIDING).show_cursor, Some(false));
    }

    #[test]
    fn mouse_move_shows_and_restarts_the_cycle() {
        // _viv_mousemove (viv.c:9151-9170): movement -> show (the timer is
        // ALREADY dead — the idle hide killed it) -> conditions still
        // hiding -> a fresh timer arms. Net effects: one ShowCursor(TRUE)
        // and a SetTimer, no kill.
        let mut v = CursorVisibility::new();
        v.update(&HIDING); // armed
        v.timer_fired(&HIDING); // hidden, timer dead
        assert_eq!(
            v.mouse_moved(true, &HIDING),
            CursorEffects {
                show_cursor: Some(true),
                kill_timer: false,
                start_timer: true,
            }
        );
        // Now shown + re-armed: the next fire hides again.
        assert_eq!(v.timer_fired(&HIDING).show_cursor, Some(false));
    }

    #[test]
    fn stationary_mouse_message_does_nothing() {
        // The dedupe: WM_MOUSEMOVE with an UNCHANGED cursor position must not
        // show the cursor or touch the cycle (viv.c:9155-9158).
        let mut v = CursorVisibility::new();
        v.update(&HIDING);
        v.timer_fired(&HIDING); // hidden
        assert_eq!(v.mouse_moved(false, &HIDING), CursorEffects::default());
        assert!(!v.shown, "still hidden — nothing was shown");
    }

    #[test]
    fn show_is_idempotent() {
        // ShowCursor is a display COUNT upstream guards with _viv_is_cursor_
        // shown (viv.c:14569-14571): two shows owe exactly one ShowCursor.
        let mut v = CursorVisibility::new();
        v.hide();
        let first = v.show();
        assert_eq!(first.show_cursor, Some(true));
        assert_eq!(v.show().show_cursor, None);
    }

    #[test]
    fn conditions_change_to_showing_kills_the_cycle() {
        // E.g. blanking the image (has_viewable_image false) or leaving the
        // window with the mouse: update() must show + kill the timer.
        let mut v = CursorVisibility::new();
        v.update(&HIDING);
        let blanked = CursorConditions {
            has_viewable_image: false,
            ..HIDING
        };
        assert_eq!(
            v.update(&blanked),
            CursorEffects {
                kill_timer: true,
                ..Default::default()
            }
        );
    }
}
