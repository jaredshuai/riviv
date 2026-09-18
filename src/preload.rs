//! Preload-next and last-image cache pure model (#40) — the decision
//! half of upstream's `_viv_preload_*` / `_viv_last_*` globals
//! (viv.c:756-764, 1326-1360, 14390-14536, 15132-15206). The Win32 shell
//! lives in `window.rs`; everything here is derivable, so the cache
//! hit/miss and adoption rules are unit-tested directly.
//!
//! Upstream model, for reference: one load thread serves foreground and
//! preload loads alike; a completed preload parks its frames in
//! `_viv_preload_frames` (never displayed until the user navigates onto
//! that file), and the previous display parks wholesale in
//! `_viv_last_frames` so navigating back skips the decode.

use crate::loader::LoadedImage;
use crate::pixels::PixelFrame;
use crate::playlist::PlaylistEntry;
use crate::surface::Surface;

/// Decode progress of a preload load (upstream `_viv_preload_state`,
/// viv.c:756 — 0 loading, 1 complete, 2 failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreloadState {
    /// Frames are still arriving (or none decoded yet).
    Loading,
    /// The full stream decoded — the image is ready to adopt.
    Complete,
    /// The decode failed at user level (or system level, degraded — the
    /// slot dies instead of the process; see the FatalSystem note in
    /// `window.rs`'s slot drain).
    Failed,
}

/// One parked preload: the in-flight (or finished) decode session plus the
/// frames it produced. Upstream spreads the same facts over
/// `_viv_preload_fd` (the file), `_viv_preload_frames` (the frames),
/// `_viv_preload_state` (the progress) and
/// `_viv_should_activate_preload_on_load` (the promotion flag).
pub(crate) struct PreloadSlot {
    pub(crate) session: crate::loadthread::LoadSession,
    /// The file being preloaded with its navigation facts (upstream
    /// `_viv_preload_fd`; the entry becomes `nav_current` on adoption).
    pub(crate) entry: PlaylistEntry,
    /// Frames decoded so far (upstream `_viv_preload_frames`) — pure
    /// memory masters (#76); the GDI-deriving `Surface` is built only
    /// when the image actually takes the display, like the drain does
    /// for replies.
    pub(crate) image: Option<LoadedImage<PixelFrame>>,
    /// Which session's first frame the parked image holds (the slot-local
    /// `displayed_from` that `apply_reply` mutates).
    pub(crate) adopted_from: Option<u64>,
    pub(crate) state: PreloadState,
    /// The user navigated onto this file while no frame was decoded yet —
    /// promote the load to the foreground when the first frame lands
    /// (upstream `_viv_should_activate_preload_on_load`, viv.c:764).
    pub(crate) activate_on_load: bool,
}

/// The previous display, parked for instant navigation back (upstream
/// `_viv_last_fd` + `_viv_last_frames` — one slot, overwritten each time a
/// new image adopts the display).
pub(crate) struct LastCache {
    /// The parked image's navigation facts (upstream `_viv_last_fd`,
    /// including the playlist id so navigation exclusion keeps working
    /// after re-activation).
    pub(crate) entry: PlaylistEntry,
    /// The WHOLE image (only fully decoded displays are cacheable —
    /// upstream refuses to move frames that are still streaming,
    /// viv.c:14436). The `Surface`s keep their memory DCs; they were built
    /// on the UI thread and stay parked on it.
    pub(crate) image: LoadedImage<Surface>,
}

/// What to do when the user opens the file a preload slot holds (upstream
/// `_viv_open_preload`'s three arms, viv.c:15132-15206).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdoptDecision {
    /// The stream finished — swap the whole cached image in and
    /// chain-preload the next neighbor (viv.c:15176-15182).
    AdoptComplete,
    /// The first frame is decoded but the stream still runs — swap the
    /// partial image in and let the session finish as the foreground load
    /// (viv.c:15166-15175; upstream flips `_viv_load_is_preload` to 0 so
    /// the remaining frames append to the display).
    AdoptPartial,
    /// No frame yet — flag the in-flight load so its first frame promotes
    /// it to the display (viv.c:15159-15164, the
    /// `_viv_should_activate_preload_on_load` arm).
    PromoteOnFirstFrame,
    /// The preload failed — cache the old display, blank, show the failed
    /// verdict, chain-preload (viv.c:15184-15202).
    AdoptFailed,
}

/// The `_viv_open_preload` arm selection: the state plus whether a first
/// frame has been decoded yet (upstream `_viv_preload_frame_loaded_count`).
pub(crate) fn adopt_decision(state: PreloadState, has_first_frame: bool) -> AdoptDecision {
    match state {
        PreloadState::Complete => AdoptDecision::AdoptComplete,
        PreloadState::Failed => AdoptDecision::AdoptFailed,
        PreloadState::Loading if has_first_frame => AdoptDecision::AdoptPartial,
        PreloadState::Loading => AdoptDecision::PromoteOnFirstFrame,
    }
}

/// What the slot-drain tail does with a slot a navigation waits on
/// (`activate_on_load`), from the batch's terminal net state — upstream's
/// should_activate arms in the reply handler: the first frame
/// (viv.c:2916-2924) promotes the load to the foreground, a completion
/// with a parked image (viv.c:2824-2829) promotes AND fires the next
/// preload itself, and a failure (viv.c:2808-2819) blanks with the failed
/// verdict. A slot nobody waits on never touches the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainAdoption {
    /// Leave the slot parked — nothing terminal for a waiting navigation.
    None,
    /// Swap the parked image in. `keep_session`: the stream still runs
    /// (Loading) and continues as the foreground load (upstream flips
    /// `_viv_load_is_preload` to 0 at the first frame, viv.c:2920); a
    /// finished stream (Complete) has no reply left, so the next preload
    /// fires from the completion arm itself (viv.c:2824-2826/2877-2879).
    Promote { keep_session: bool },
    /// The load failed while a navigation waited — the display blanks with
    /// the failed verdict and the next preload chains (viv.c:2808-2819).
    FailActivation,
}

/// The drain-tail decision for a waiting slot: whether its reply batch
/// promotes the parked image, blanks on a failure, or does nothing.
pub(crate) fn drain_adoption(
    activate_on_load: bool,
    state: PreloadState,
    has_image: bool,
) -> DrainAdoption {
    if !activate_on_load {
        return DrainAdoption::None;
    }
    match state {
        PreloadState::Failed => DrainAdoption::FailActivation,
        PreloadState::Loading if has_image => DrainAdoption::Promote { keep_session: true },
        PreloadState::Complete if has_image => DrainAdoption::Promote {
            keep_session: false,
        },
        PreloadState::Loading | PreloadState::Complete => DrainAdoption::None,
    }
}

/// Whether the status bar's PRELOAD part shows (upstream gate viv.c:11210:
/// a preload load is in flight, still decoding its first frame, and nobody
/// is waiting to adopt it — `_viv_load_is_preload && _viv_preload_state ==
/// 0 && !_viv_should_activate_preload_on_load && !_viv_load_image_terminate
/// && !_viv_preload_frame_loaded_count`). A terminated slot is gone from
/// the window state entirely, so the terminate clause is structural here.
pub(crate) fn indicator_visible(
    state: PreloadState,
    has_first_frame: bool,
    activate_on_load: bool,
) -> bool {
    state == PreloadState::Loading && !has_first_frame && !activate_on_load
}

/// Whether a dying display moves into the last cache (upstream
/// `viv_copy_current_image_to_last_image`'s gate, viv.c:14436-14443: the
/// feature is on AND the whole image decoded — a partial frame set must
/// keep streaming into the display, and upstream frees the old cache
/// without refilling instead). A blank display never caches (the caller
/// clears the slot — upstream's vacuous 0==0 count compare copies the
/// empty fd over the cache, emptying it).
pub(crate) fn cacheable(cache_last: bool, decode_complete: bool) -> bool {
    cache_last && decode_complete
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- adopt decision (upstream _viv_open_preload's three arms) ----

    #[test]
    fn a_completed_preload_swaps_in_whole() {
        assert_eq!(
            adopt_decision(PreloadState::Complete, false),
            AdoptDecision::AdoptComplete
        );
    }

    #[test]
    fn a_completed_preload_with_frames_still_swaps_in_whole() {
        // The has_first_frame clause is upstream's loading-only branch
        // (frame_loaded_count); a completed stream always adopts fully.
        assert_eq!(
            adopt_decision(PreloadState::Complete, true),
            AdoptDecision::AdoptComplete
        );
    }

    #[test]
    fn a_loading_preload_with_a_first_frame_switches_immediately() {
        // viv.c:15166-15175: state 0 with frame_loaded_count set — "switch
        // now", the remaining frames follow as the foreground load.
        assert_eq!(
            adopt_decision(PreloadState::Loading, true),
            AdoptDecision::AdoptPartial
        );
    }

    #[test]
    fn a_loading_preload_without_frames_waits_for_the_first_one() {
        // viv.c:15155-15164: still loading — set
        // _viv_should_activate_preload_on_load and wait.
        assert_eq!(
            adopt_decision(PreloadState::Loading, false),
            AdoptDecision::PromoteOnFirstFrame
        );
    }

    #[test]
    fn a_failed_preload_blanks_with_the_failed_verdict() {
        assert_eq!(
            adopt_decision(PreloadState::Failed, false),
            AdoptDecision::AdoptFailed
        );
        assert_eq!(
            adopt_decision(PreloadState::Failed, true),
            AdoptDecision::AdoptFailed
        );
    }

    // ---- drain-tail adoption for a navigation-waiting slot (upstream
    // viv.c:2808-2829/2916-2924) ----

    #[test]
    fn a_first_frame_while_a_navigation_waits_promotes_and_keeps_the_stream() {
        // viv.c:2916-2924: should_activate + first frame — the load becomes
        // the foreground (remaining frames follow as the display's stream).
        assert_eq!(
            drain_adoption(true, PreloadState::Loading, true),
            DrainAdoption::Promote { keep_session: true }
        );
    }

    #[test]
    fn a_completion_landing_in_the_same_drain_drops_the_finished_stream() {
        // viv.c:2824-2829: COMPLETE + should_activate adopts the parked
        // image and fires the next preload from the completion arm itself.
        // Keeping the finished session would strand the Loading status and
        // the preload chain (cubic P1): no reply is left to end it.
        assert_eq!(
            drain_adoption(true, PreloadState::Complete, true),
            DrainAdoption::Promote {
                keep_session: false
            }
        );
    }

    #[test]
    fn a_failure_while_a_navigation_waits_blanks_regardless_of_parked_frames() {
        // viv.c:2808-2819: FAILED + should_activate clears the display with
        // the failed verdict — partial frames never adopt (Codex P1).
        assert_eq!(
            drain_adoption(true, PreloadState::Failed, false),
            DrainAdoption::FailActivation
        );
        assert_eq!(
            drain_adoption(true, PreloadState::Failed, true),
            DrainAdoption::FailActivation
        );
    }

    #[test]
    fn a_waiting_slot_without_its_first_frame_yet_does_nothing() {
        assert_eq!(
            drain_adoption(true, PreloadState::Loading, false),
            DrainAdoption::None
        );
    }

    #[test]
    fn a_slot_nobody_waits_on_never_touches_the_display() {
        // Background terminal replies only record the slot's state; the
        // display hears nothing (the same split as the reply stash).
        assert_eq!(
            drain_adoption(false, PreloadState::Complete, true),
            DrainAdoption::None
        );
        assert_eq!(
            drain_adoption(false, PreloadState::Failed, true),
            DrainAdoption::None
        );
        assert_eq!(
            drain_adoption(false, PreloadState::Loading, true),
            DrainAdoption::None
        );
    }

    // ---- status-bar PRELOAD indicator (upstream viv.c:11210 gate) ----

    #[test]
    fn the_preload_indicator_shows_only_while_the_first_frame_decodes() {
        assert!(indicator_visible(PreloadState::Loading, false, false));
    }

    #[test]
    fn the_preload_indicator_hides_once_a_frame_is_parked() {
        // Upstream hides it at the first-frame stash (viv.c:2942's
        // _viv_status_update after _viv_preload_frame_loaded_count = 1).
        assert!(!indicator_visible(PreloadState::Loading, true, false));
    }

    #[test]
    fn the_preload_indicator_hides_when_a_navigation_waits_on_it() {
        // Promoted preload: the main part shows "Loading..." instead
        // (upstream's should_activate clause in both the preload gate and
        // the Loading gate, viv.c:11210/11356).
        assert!(!indicator_visible(PreloadState::Loading, false, true));
    }

    #[test]
    fn the_preload_indicator_hides_after_terminal_replies() {
        assert!(!indicator_visible(PreloadState::Complete, false, false));
        assert!(!indicator_visible(PreloadState::Failed, false, false));
    }

    // ---- last-cache eligibility (upstream viv.c:14436-14443 gate) ----

    #[test]
    fn a_fully_decoded_display_moves_into_the_last_cache() {
        assert!(cacheable(true, true));
    }

    #[test]
    fn a_still_streaming_display_is_not_cacheable() {
        // Upstream refuses to move frames that later replies must append
        // to (frame_count != frame_loaded_count).
        assert!(!cacheable(true, false));
    }

    #[test]
    fn cache_last_disabled_never_caches() {
        assert!(!cacheable(false, true));
    }
}
