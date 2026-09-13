//! Animation frame scheduling math (pure logic, unit-tested).
//!
//! Mirrors upstream's WM_TIMER body (viv.c:3171-3292): a ~10 ms timer feeds
//! elapsed time (measured with the performance counter, viv.c:3182-3186)
//! into an accumulator; whenever the accumulator covers the current frame's
//! delay the position advances and the delay is subtracted — repeated in a
//! loop so a long stall catches up by skipping intermediate frames and only
//! painting the latest one. A single event's elapsed time is capped at one
//! second (viv.c:3189-3192: "don't elapse more than one second at a time").
//!
//! All accumulation happens in QueryPerformanceCounter ticks, not
//! milliseconds: converting each delay to ticks exactly once
//! (`delay_ms * freq / 1000`, upstream's `performance_counter_delay`)
//! means repeated event handling can never accumulate rounding drift —
//! a truncated-milliseconds accumulator would lose up to 1 ms per event.
//!
//! The streamed-loading branch (#4, landed): while the background decode
//! is still delivering frames, reaching the loaded-prefix edge zeroes the
//! accumulator and waits for the next frame (upstream resets
//! `_viv_timer_tick` and breaks, viv.c:3233-3240 — with the decode fully
//! done the same edge wraps to frame 0 instead). Wrapping is keyed on the
//! caller's `complete` flag (decode finished) rather than upstream's
//! pre-known `_viv_frame_count` because the image crate's frame iterators
//! cannot report a total up front.
//!
//! The #38 additions: the animation-rate speed table (upstream
//! `_viv_animation_rates`, viv.c:669) scales every delay the catch-up loop
//! consumes, the pause flag (upstream `_viv_animation_play`, viv.c:673)
//! gates the accumulation itself (viv.c:3195), and the frame-position
//! walkers (Frame Step / Previous / First / Last / the millisecond-budget
//! jumps, viv.c:9255-9315/10056-10103) re-anchor the timeline the way the
//! command handlers do.

/// SetTimer id for the animation timer. Any private id works (it is only
/// compared against our own WM_TIMER wparam); upstream's is a command-enum
/// value `VIV_ID_ANIMATION_TIMER` (viv.h:194).
pub(crate) const ANIMATION_TIMER_ID: usize = 1;

/// The fixed animation rates (upstream `_viv_animation_rates[]`,
/// viv.c:669): a multiplier applied as `delay × (1 / rate)` in the timer
/// loop (viv.c:3209) — 2.0 halves every delay (double speed), 0.125
/// multiplies it by eight. Index 10 is the 1.0 identity (`_VIV_ANIMATION_
/// RATE_ONE`, viv.c:671).
pub(crate) const RATE_TABLE: [f32; 21] = [
    0.125, 0.142_857, 0.166_667, 0.2, 0.25, 0.333_333, 0.5, 0.571_429, 0.666_667, 0.8, 1.0, 1.25,
    1.5, 1.75, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0,
];

/// The table index of rate 1.0 — Reset Rate's target (upstream
/// `_VIV_ANIMATION_RATE_ONE`, viv.c:671; `_viv_reset_animation_rate`,
/// viv.c:7676-7681).
pub(crate) const RATE_ONE: usize = 10;

/// Step within [`RATE_TABLE`] (upstream `_viv_increase_animation_rate`,
/// viv.c:7656-7674): Decrease/Increase move one entry and CLAMP at the
/// ends — no wrap.
pub(crate) fn rate_step(pos: usize, decrease: bool) -> usize {
    if decrease {
        pos.saturating_sub(1)
    } else {
        (pos + 1).min(RATE_TABLE.len() - 1)
    }
}

/// The playback knobs the timer loop reads — upstream's globals
/// `_viv_animation_play` (viv.c:673, default 1, reset per image at
/// viv.c:1291) and `_viv_animation_rate_pos` (viv.c:672, persists across
/// images — only the rate commands move it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Playback {
    pub(crate) playing: bool,
    /// Index into [`RATE_TABLE`] (clamped by the callers; the step helper
    /// guarantees the in-range invariant).
    pub(crate) rate_pos: usize,
}

impl Playback {
    /// The default knobs: playing at rate 1.0.
    pub(crate) const fn new() -> Self {
        Playback {
            playing: true,
            rate_pos: RATE_ONE,
        }
    }
}

impl Default for Playback {
    fn default() -> Self {
        Self::new()
    }
}

/// One frame's effective delay in ms under `rate_pos` (upstream viv.c:
/// 3209-3214): the raw delay × the reciprocal rate in f32, truncated to an
/// integer, floored at 1 — the floor doubles as upstream's `if (!delay)
/// delay = 1` for zero-delay (WebP) frames whose scaled result rounds to
/// zero.
fn scaled_delay_ms(delays_ms: &[u32], position: usize, rate_pos: usize) -> u64 {
    let scaled = (delays_ms[position] as f32 * (1.0f32 / RATE_TABLE[rate_pos])) as u32;
    u64::from(scaled.max(1))
}

/// Result of one timer event: the frame to display now and whether it
/// changed (a repaint is only needed when at least one frame boundary was
/// crossed — upstream's `invalidate` flag, viv.c:3279-3287). `looped` says
/// the advance wrapped past the last frame — the moment upstream raises
/// `_viv_frame_looped` (viv.c:3243), which the slideshow's timeup gate
/// waits for (#37).
pub(crate) struct FrameAdvance {
    pub(crate) position: usize,
    pub(crate) repaint: bool,
    pub(crate) looped: bool,
}

/// Pure timing state for one animation: upstream's
/// `_viv_timer_tick` / `_viv_animation_timer_tick_start` pair.
pub(crate) struct FrameScheduler {
    /// Accumulated play time not yet consumed by frame advances (QPC ticks).
    timer_tick: u64,
    /// Performance-counter reading when the timer last fired (the start of
    /// the interval the next event will measure).
    tick_start: u64,
}

impl FrameScheduler {
    /// A scheduler starting at `tick_start` with no accumulated time
    /// (upstream `_viv_start_first_frame`, viv.c:14312-14317) — the only
    /// anchor: playback keeps it until the image is replaced.
    pub(crate) fn new(tick_start: u64) -> Self {
        FrameScheduler {
            timer_tick: 0,
            tick_start,
        }
    }

    /// Whether the time accumulated since the anchor already covers the
    /// current frame's delay — i.e. playback is at the loaded edge where
    /// a running timer's stall branch would have been zeroing time every
    /// tick (viv.c:3233-3240). Used to re-anchor when a streamed frame
    /// arrives after that edge: without a running timer nothing zeroed
    /// the elapsed time, so the anchor must be reset to the arrival or
    /// the first timer event would credit the whole decode gap. The delay
    /// is the rate-scaled one the timer would have consumed (viv.c:3209).
    pub(crate) fn at_frame_edge(
        &self,
        now: u64,
        freq: u64,
        delays_ms: &[u32],
        position: usize,
        rate_pos: usize,
    ) -> bool {
        let elapsed = now.saturating_sub(self.tick_start).min(freq);
        let delay_ticks = ((scaled_delay_ms(delays_ms, position, rate_pos) * freq) / 1000).max(1);
        self.timer_tick + elapsed >= delay_ticks
    }

    /// Process one timer event measured at `now` (QPC ticks, `freq` ticks
    /// per second) against the per-frame `delays_ms` from `position`.
    /// `delays_ms.len()` is the loaded-prefix length; `complete` says the
    /// decode stream has ended, making that prefix the full frame set.
    /// `stop_at_loop` is the slideshow's held-advance gate (`loop_once &&
    /// timeup`, computed by the caller like upstream's inline test): when
    /// set, the FIRST wrap breaks the catch-up loop before the subtraction
    /// and repaint bookkeeping — upstream calls `_viv_next` and `break`s
    /// right there (viv.c:3245-3250), the incoming image replacing
    /// whatever the leftover accumulator said.
    ///
    /// `playback` carries the pause flag and the rate table position
    /// (upstream reads `_viv_animation_play` / `_viv_animation_rate_pos`
    /// as globals, viv.c:3195/3209): while paused the anchor still tracks
    /// the clock — each event measures and discards its elapsed time — but
    /// nothing accumulates or advances, and the pre-pause partial
    /// accumulator survives the resume (upstream's `_viv_animation_pause`
    /// flips the flag and nothing else, viv.c:9250-9253).
    ///
    /// When the accumulated time covers several frame delays the loop keeps
    /// advancing until the remainder is below the next delay — catch-up
    /// paints only the final position, exactly like upstream's loop
    /// (viv.c:3225-3272). At the loaded-prefix edge a completed animation
    /// wraps to frame 0; an in-flight one zeroes the accumulator and waits
    /// for the next streamed frame (viv.c:3233-3240).
    // The parameter list deliberately mirrors the upstream timer body's
    // inputs one-for-one (clock, frequency, frame set, position, completion,
    // the slideshow gate, the playback knobs) — bundling further would
    // obscure that mapping.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn on_timer(
        &mut self,
        now: u64,
        freq: u64,
        delays_ms: &[u32],
        position: usize,
        complete: bool,
        stop_at_loop: bool,
        playback: Playback,
    ) -> FrameAdvance {
        let elapsed = now.saturating_sub(self.tick_start);
        self.tick_start = now;
        // Don't elapse more than one second at a time (viv.c:3189-3192):
        // after a long stall the animation catches up by at most one second
        // of frames instead of fast-forwarding to real time.
        let elapsed = elapsed.min(freq);
        if !playback.playing {
            // Paused (viv.c:3195): the anchor moved above, the time dies
            // here — no accumulation, no advance, no repaint.
            return FrameAdvance {
                position,
                repaint: false,
                looped: false,
            };
        }
        self.timer_tick += elapsed;

        let mut position = position;
        let mut repaint = false;
        let mut looped = false;
        loop {
            // The rate-scaled delay with its 1 ms floor (viv.c:3209-3214);
            // the ticks conversion's max(1) guards the degenerate freq <
            // 1000 case where the quotient would be zero and the catch-up
            // loop below would never terminate.
            let delay_ms = scaled_delay_ms(delays_ms, position, playback.rate_pos);
            let delay_ticks = ((delay_ms * freq) / 1000).max(1);
            if self.timer_tick >= delay_ticks {
                if position + 1 == delays_ms.len() {
                    if complete {
                        // Loop the animation (viv.c:3243-3248; upstream has a
                        // play-once slideshow variant we don't build in #3).
                        position = 0;
                        looped = true;
                        if stop_at_loop {
                            // The gated wrap breaks out BEFORE the
                            // subtraction/repaint bookkeeping (viv.c:3245-
                            // 3250): the held advance replaces the display,
                            // the leftover accumulator dies with the old
                            // image.
                            break;
                        }
                    } else {
                        // The next frame is still decoding: ignore this tick,
                        // zero the accumulator, and wait at the prefix edge
                        // (viv.c:3233-3240) so playback resumes from the
                        // moment the frame arrives instead of fast-forwarding.
                        self.timer_tick = 0;
                        break;
                    }
                } else {
                    position += 1;
                }
                self.timer_tick -= delay_ticks;
                repaint = true;
            } else {
                break;
            }
        }
        FrameAdvance {
            position,
            repaint,
            looped,
        }
    }
}

/// GIF delay normalization: image delivers the PropertyTagFrameDelay value
/// exactly as centiseconds × 10 ms (gif 0.14 delay unit is 10 ms; image
/// multiplies by 10 into the `Delay` ms ratio), and a zero/absent delay
/// falls back to 100 ms — upstream viv.c:10749-10753 ("just use a value of
/// 0 for bad data" → ×10 → 0 → 100 fallback).
pub(crate) fn gif_delay_ms(image_reported_ms: u32) -> u32 {
    if image_reported_ms == 0 {
        100
    } else {
        image_reported_ms
    }
}

/// Frame Step's position walk (upstream `_viv_frame_step`, viv.c:9264-9272):
/// advance one frame with the streaming edge guard — `complete` unlocks the
/// wrap to frame 0 at the last frame, an in-flight decode only advances
/// within the loaded prefix. `None` = the guard held the advance at the
/// loaded edge (no move, no repaint). A single-frame `delays_ms` always
/// holds (the caller's `frame_count > 1` check, viv.c:9264).
pub(crate) fn step_position(delays_ms: &[u32], position: usize, complete: bool) -> Option<usize> {
    if delays_ms.len() <= 1 {
        // The caller's `frame_count > 1` check (viv.c:9264): a static image
        // never steps.
        return None;
    }
    let next = position + 1;
    if complete || next < delays_ms.len() {
        Some(if next == delays_ms.len() { 0 } else { next })
    } else {
        None
    }
}

/// Previous Frame's position walk (upstream `_viv_frame_prev`, viv.c:9297-
/// 9304): retreat one frame, wrapping from frame 0 to the LAST LOADED
/// frame — unlike the forward walk there is no streaming guard (a partial
/// prefix still lets you page back through what has arrived). Caller
/// guarantees `delays_ms.len() > 1`.
pub(crate) fn prev_position(delays_ms: &[u32], position: usize) -> usize {
    if position > 0 {
        position - 1
    } else {
        delays_ms.len() - 1
    }
}

/// The millisecond-budget jump (upstream `_viv_frame_skip`, viv.c:10056-
/// 10103): walk frames in `direction_ms`'s sign until the RAW frame delays
/// consumed exceed the budget — the jump size is time, not frames (a
/// 1000 ms medium jump crosses ten 100 ms frames or one 1000 ms one). The
/// forward walk reuses Frame Step's streaming edge guard (a blocked step
/// consumes the current frame's delay without moving, viv.c:10064-10076);
/// the backward walk wraps within the loaded prefix with no guard
/// (viv.c:10084-10093). Delay scaling does NOT apply (viv.c:10076/10093
/// read `_viv_frames[...].delay` raw — the jump covers the same wall of
/// frames whatever the playback rate).
///
/// Deviation from upstream: each consumed delay floors at 1 ms. Upstream's
/// loop subtracts the raw delay, so a run of zero-delay frames (possible
/// in WebP; the GIF loader floors to 100 ms) spends no budget and would
/// spin forever — riviv terminates at one millisecond per frame.
pub(crate) fn skip_position(
    delays_ms: &[u32],
    position: usize,
    complete: bool,
    direction_ms: i32,
) -> usize {
    let mut pos = position;
    let mut budget = direction_ms;
    if budget > 0 {
        while budget > 0 {
            if complete || pos + 1 < delays_ms.len() {
                pos += 1;
                if pos == delays_ms.len() {
                    pos = 0;
                }
            }
            budget -= delays_ms[pos].max(1) as i32;
        }
    } else if budget < 0 {
        while budget < 0 {
            pos = if pos > 0 {
                pos - 1
            } else {
                delays_ms.len() - 1
            };
            budget += delays_ms[pos].max(1) as i32;
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    /// freq = 1000 makes one tick one millisecond, so tests read like the
    /// delays they assert against.
    const FREQ: u64 = 1000;

    fn scheduler_starting_at(tick: u64) -> FrameScheduler {
        FrameScheduler::new(tick)
    }

    #[test]
    fn elapsed_time_below_current_delay_keeps_the_current_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        // 99 ms accumulated: below the 100 ms first-frame delay.
        let adv = s.on_timer(99, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        assert!(!adv.repaint);
    }

    #[test]
    fn one_full_delay_of_elapsed_time_advances_exactly_one_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        let adv = s.on_timer(100, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn partial_elapsed_time_accumulates_across_events() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        // Two 60 ms ticks: neither reaches the delay alone, together they do
        // (60 + 60 = 120 >= 100) and the leftover 20 ms is retained.
        let adv = s.on_timer(60, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        let adv = s.on_timer(120, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn position_wraps_back_to_the_first_frame_after_the_last() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // Frame 1's delay elapses while sitting on the last frame.
        let adv = s.on_timer(100, FREQ, &delays, 1, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        assert!(adv.repaint);
        // The wrap is the loop-completion moment upstream marks
        // `_viv_frame_looped` (viv.c:3243) — reported once per wrap, and
        // only for completed animations (the streaming edge does not wrap).
        assert!(adv.looped);
        let adv = s.on_timer(250, FREQ, &delays, 0, false, false, Playback::new());
        assert!(!adv.looped);
    }

    #[test]
    fn the_gated_wrap_stops_the_catch_up_at_the_first_loop() {
        // The slideshow's held advance (loop_once + timeup): upstream
        // wraps, calls _viv_next and BREAKS out of the catch-up loop right
        // there (viv.c:3245-3250) — 250 ms over [100, 100] from frame 1
        // covers the wrap plus frame 0's delay again, but the gated stop
        // leaves the position at 0 instead of advancing to 1 (cubic
        // round 1).
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        let adv = s.on_timer(250, FREQ, &delays, 1, true, true, Playback::new());
        assert_eq!(adv.position, 0);
        assert!(adv.looped);
        assert!(!adv.repaint, "the break skips the wrap step's repaint mark");
        // Without the gate the same elapsed time catch-ups past the wrap.
        let mut s = scheduler_starting_at(0);
        let adv = s.on_timer(250, FREQ, &delays, 1, true, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn a_long_stall_elapses_at_most_one_second_per_event() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        // 10 seconds of wall time in one event: only 1 s is credited, so the
        // animation advances ten 100 ms frames (back to frame 1) instead of
        // a hundred.
        let adv = s.on_timer(10_000, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        // The next event measures from the truncation point, not real time.
        let adv = s.on_timer(
            10_100,
            FREQ,
            &delays,
            adv.position,
            true,
            false,
            Playback::new(),
        );
        assert_eq!(adv.position, 2);
        assert!(adv.repaint);
    }

    #[test]
    fn catch_up_after_a_stall_paints_only_the_latest_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100, 100];
        // 350 ms elapsed covers three and a half frames: the position lands
        // on frame 3 and the two intermediate frames are skipped without
        // their own repaints (upstream counts them as frames_skipped,
        // viv.c:3220/3257-3260).
        let adv = s.on_timer(350, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 3);
        assert!(adv.repaint);
    }

    #[test]
    fn a_zero_delay_frame_advances_at_the_one_millisecond_floor() {
        let mut s = scheduler_starting_at(0);
        // WebP may carry zero-duration frames; upstream floors them to 1 ms
        // (viv.c:3211-3214) so the loop cannot spin unbounded within one
        // event: 25 ms advances exactly 25 one-millisecond frames.
        let delays = [0, 0, 0];
        let adv = s.on_timer(25, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        let adv = s.on_timer(
            26,
            FREQ,
            &delays,
            adv.position,
            true,
            false,
            Playback::new(),
        );
        assert_eq!(adv.position, 2);
    }

    #[test]
    fn the_loaded_prefix_edge_waits_for_the_next_streamed_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // Sitting on the only loaded frame with decode still in flight: any
        // amount of elapsed time is discarded and the frame is held
        // (upstream viv.c:3233-3240).
        let adv = s.on_timer(250, FREQ, &delays, 1, false, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(!adv.repaint);
        // The accumulator was zeroed, not banked: another long event while
        // still waiting must not fast-forward once the frame lands.
        let adv = s.on_timer(1_000, FREQ, &delays, 1, false, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(!adv.repaint);
    }

    #[test]
    fn a_partial_advance_into_the_edge_stalls_after_repainting() {
        let mut s = scheduler_starting_at(0);
        let delays = [50, 100];
        // 150 ms covers frame 0's 50 ms (advance, repaint) and then meets
        // frame 1's 100 ms exactly at the not-yet-loaded edge: the advance
        // stops there, the repaint of the partial advance is kept, and the
        // leftover 100 ms is discarded rather than banked.
        let adv = s.on_timer(150, FREQ, &delays, 0, false, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        let adv = s.on_timer(
            160,
            FREQ,
            &delays,
            adv.position,
            false,
            false,
            Playback::new(),
        );
        assert_eq!(adv.position, 1);
        assert!(!adv.repaint);
    }

    #[test]
    fn the_prefix_edge_wraps_once_the_decode_completes() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // Same stall conditions, but the stream has ended: the edge is the
        // whole frame set, so it wraps like a fully-decoded animation.
        // 150 ms = the last frame's delay (wraps to 0) + 50 ms below
        // frame 0's delay (stays there).
        let adv = s.on_timer(150, FREQ, &delays, 1, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        assert!(adv.repaint);
    }

    #[test]
    fn frames_arriving_after_completion_are_ignored_by_the_scheduler() {
        // The caller never grows delays past completion; this pins the
        // contract that `complete` freezes the frame set (the wrap branch
        // must keep using the slice length it was given).
        let mut s = scheduler_starting_at(0);
        let delays = [100];
        let adv = s.on_timer(100, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        assert!(adv.repaint);
    }

    #[test]
    fn gif_delay_zero_falls_back_to_100ms() {
        assert_eq!(gif_delay_ms(0), 100);
    }

    #[test]
    fn gif_delay_in_centiseconds_passes_through_times_ten() {
        // The image crate already delivers cs × 10 ms (gif 0.14 delay unit
        // is 10 ms); the loader passes that through this normalization.
        assert_eq!(gif_delay_ms(10), 10);
        assert_eq!(gif_delay_ms(1_000), 1_000);
    }

    /// The playing rate's table position for a helper: reads best as the
    /// multiplier it applies.
    fn playing_at(rate_pos: usize) -> Playback {
        Playback {
            playing: true,
            rate_pos,
        }
    }

    #[test]
    fn the_rate_table_is_pinned_against_upstream() {
        // viv.c:669 verbatim — 21 fixed rates, index 10 the identity.
        assert_eq!(RATE_TABLE[RATE_ONE], 1.0);
        assert_eq!(RATE_TABLE.len(), 21);
        assert_eq!(
            RATE_TABLE,
            [
                0.125, 0.142_857, 0.166_667, 0.2, 0.25, 0.333_333, 0.5, 0.571_429, 0.666_667, 0.8,
                1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0
            ]
        );
    }

    #[test]
    fn rate_stepping_clamps_at_the_table_ends() {
        // viv.c:7656-7674: one entry per press, no wrap — the ends stick.
        assert_eq!(rate_step(RATE_ONE, true), 9);
        assert_eq!(rate_step(RATE_ONE, false), 11);
        assert_eq!(rate_step(0, true), 0);
        assert_eq!(rate_step(RATE_TABLE.len() - 1, false), RATE_TABLE.len() - 1);
    }

    #[test]
    fn a_faster_rate_shortens_the_delay_and_a_slower_one_lengthens_it() {
        // viv.c:3209: delay × (1/rate) truncated to an integer — 100 ms at
        // 8× is 12.5 → 12 ms, at 0.125× it is 800 ms.
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        let adv = s.on_timer(12, FREQ, &delays, 0, true, false, playing_at(20));
        assert_eq!(adv.position, 1, "12 ms covers the 12 ms scaled delay");
        let mut s = scheduler_starting_at(0);
        let adv = s.on_timer(799, FREQ, &delays, 0, true, false, playing_at(0));
        assert_eq!(adv.position, 0, "799 ms is below the 800 ms scaled delay");
        let adv = s.on_timer(800, FREQ, &delays, 0, true, false, playing_at(0));
        assert_eq!(adv.position, 1);
    }

    #[test]
    fn a_scaled_delay_that_truncates_to_zero_floors_at_one_millisecond() {
        // viv.c:3211-3214: a 1 ms frame at 8× scales to 0.125 → DWORD 0 →
        // floored back to 1, so the catch-up loop still terminates.
        let mut s = scheduler_starting_at(0);
        let delays = [1, 1];
        let adv = s.on_timer(3, FREQ, &delays, 0, true, false, playing_at(20));
        assert_eq!(adv.position, 1, "three 1 ms events advance three frames");
    }

    #[test]
    fn a_paused_animation_discards_time_and_keeps_the_partial_accumulator() {
        // viv.c:3195: paused, the timer still re-anchors every event — the
        // elapsed time is measured and dropped. Resume continues from the
        // partial accumulator that existed at the pause (viv.c:9250-9253
        // flips nothing else).
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // 60 ms of play, then pause: 40 ms short of the advance.
        let adv = s.on_timer(60, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        let paused = Playback {
            playing: false,
            ..Playback::new()
        };
        // A long paused gap (10 s) must not bank against the frame.
        let adv = s.on_timer(10_060, FREQ, &delays, 0, true, false, paused);
        assert_eq!(adv.position, 0);
        assert!(!adv.repaint);
        // Resume: the pre-pause 60 ms plus 39 ms of play still holds...
        let adv = s.on_timer(10_099, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 0);
        // ...and the 41st resumed millisecond crosses the boundary.
        let adv = s.on_timer(10_100, FREQ, &delays, 0, true, false, Playback::new());
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn frame_step_advances_one_frame_and_wraps_when_complete() {
        let delays = [100, 100, 100];
        assert_eq!(step_position(&delays, 0, true), Some(1));
        assert_eq!(step_position(&delays, 2, true), Some(0), "the wrap");
        // In-flight decode: within the loaded prefix it advances, at the
        // edge it holds (viv.c:9266-9268).
        assert_eq!(step_position(&delays[..2], 0, false), Some(1));
        assert_eq!(step_position(&delays[..2], 1, false), None);
        // A static image never steps (viv.c:9264).
        assert_eq!(step_position(&delays[..1], 0, true), None);
    }

    #[test]
    fn previous_frame_retreats_and_wraps_to_the_last_loaded_frame() {
        // viv.c:9297-9304: wrap to loaded_count - 1, no streaming guard.
        let delays = [100, 100, 100];
        assert_eq!(prev_position(&delays, 2), 1);
        assert_eq!(prev_position(&delays, 1), 0);
        assert_eq!(prev_position(&delays, 0), 2);
        // The wrap lands on the LOADED prefix's last frame mid-decode.
        assert_eq!(prev_position(&delays[..2], 0), 1);
    }

    #[test]
    fn a_forward_jump_spends_the_budget_on_raw_delays() {
        // viv.c:10062-10077: 1000 ms over 100 ms frames crosses ten frames
        // (the frame whose delay overflows the budget IS entered — the
        // subtraction runs after the move).
        let delays = [100; 12];
        assert_eq!(skip_position(&delays, 0, true, 1_000), 10);
        // 250 ms over 100 ms frames: enters frame 2 with 50 ms left,
        // enters frame 3 overshooting — lands on 3.
        assert_eq!(skip_position(&delays, 0, true, 250), 3);
        // The walk wraps: 500 ms from frame 10 enters 11, 0, 1, 2, 3 —
        // five 100 ms frames — and lands on 3.
        assert_eq!(skip_position(&delays, 10, true, 500), 3);
        // Heterogeneous delays make the time-budget shape visible: with
        // [400, 100] a 1000 ms jump from frame 0 re-enters frame 0's 400
        // after the wrap and lands back on 0 (400+100+400+100), and even a
        // 400 ms budget overshoots through 1→0→1→0.
        let mixed = [400, 100];
        assert_eq!(skip_position(&mixed, 0, true, 1_000), 0);
        assert_eq!(skip_position(&mixed, 0, true, 400), 0);
    }

    #[test]
    fn a_backward_jump_wraps_within_the_loaded_prefix() {
        // viv.c:10082-10094: the retreat wraps to loaded_count - 1 and the
        // delay added is the NEW position's.
        let delays = [100; 12];
        assert_eq!(skip_position(&delays, 2, true, -1_000), 4);
        // -250 ms enters 11, 10, 9 — the third retreat flips the budget.
        assert_eq!(skip_position(&delays, 0, true, -250), 9);
        // In-flight decode: the backward walk has no edge guard — it pages
        // back through the two loaded frames freely.
        assert_eq!(skip_position(&delays[..2], 1, false, -250), 0);
    }

    #[test]
    fn a_forward_jump_holds_at_the_streaming_edge_like_frame_step() {
        // viv.c:10064-10068: with the decode in flight the guard blocks the
        // advance at the edge, but the (blocked) steps still consume budget
        // — a 250 ms jump at a 100 ms edge frame ends without moving.
        let delays = [100, 100];
        assert_eq!(skip_position(&delays, 1, false, 250), 1);
    }

    #[test]
    fn a_zero_delay_run_terminates_the_jump_walk() {
        // The deviation floor: upstream's raw subtraction would spin on
        // zero-delay frames (WebP); each frame costs at least 1 ms here, so
        // a 5 ms budget crosses exactly five frames.
        let delays = [0, 0, 0, 0, 0, 0];
        assert_eq!(skip_position(&delays, 0, true, 5), 5);
        assert_eq!(skip_position(&delays, 2, true, -3), 5);
    }

    #[test]
    fn a_zero_millisecond_jump_is_a_no_op() {
        let delays = [100, 100];
        assert_eq!(skip_position(&delays, 1, true, 0), 1);
    }

    #[test]
    fn at_frame_edge_scales_the_delay_by_the_rate() {
        // The re-anchor probe must agree with the timer's scaled delay
        // (viv.c:3209): at 2× (table index 14) a 100 ms frame's edge
        // arrives at 50 ms.
        let s = scheduler_starting_at(0);
        let delays = [100, 100];
        assert!(!s.at_frame_edge(49, FREQ, &delays, 0, 14));
        assert!(s.at_frame_edge(50, FREQ, &delays, 0, 14));
    }
}
