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
//! M2 seams landing later: the animation-rate speed table (upstream
//! `_viv_animation_rates`, a menu/keyboard feature).

/// SetTimer id for the animation timer. Any private id works (it is only
/// compared against our own WM_TIMER wparam); upstream's is a command-enum
/// value `VIV_ID_ANIMATION_TIMER` (viv.h:194).
pub(crate) const ANIMATION_TIMER_ID: usize = 1;

/// Result of one timer event: the frame to display now and whether it
/// changed (a repaint is only needed when at least one frame boundary was
/// crossed — upstream's `invalidate` flag, viv.c:3279-3287).
pub(crate) struct FrameAdvance {
    pub(crate) position: usize,
    pub(crate) repaint: bool,
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

    /// Process one timer event measured at `now` (QPC ticks, `freq` ticks
    /// per second) against the per-frame `delays_ms` from `position`.
    /// `delays_ms.len()` is the loaded-prefix length; `complete` says the
    /// decode stream has ended, making that prefix the full frame set.
    ///
    /// When the accumulated time covers several frame delays the loop keeps
    /// advancing until the remainder is below the next delay — catch-up
    /// paints only the final position, exactly like upstream's loop
    /// (viv.c:3225-3272). At the loaded-prefix edge a completed animation
    /// wraps to frame 0; an in-flight one zeroes the accumulator and waits
    /// for the next streamed frame (viv.c:3233-3240).
    pub(crate) fn on_timer(
        &mut self,
        now: u64,
        freq: u64,
        delays_ms: &[u32],
        position: usize,
        complete: bool,
    ) -> FrameAdvance {
        let elapsed = now.saturating_sub(self.tick_start);
        self.tick_start = now;
        // Don't elapse more than one second at a time (viv.c:3189-3192):
        // after a long stall the animation catches up by at most one second
        // of frames instead of fast-forwarding to real time.
        let elapsed = elapsed.min(freq);
        self.timer_tick += elapsed;

        let mut position = position;
        let mut repaint = false;
        loop {
            // Upstream floors a zero delay to 1 ms so a zero-delay frame
            // still advances at a bounded rate (viv.c:3211-3214). GIF never
            // delivers zero here (the loader's 100 ms fallback), WebP can.
            let delay_ms = delays_ms[position].max(1) as u64;
            // Exact per-delay conversion (viv.c:3216); the max(1) guards the
            // degenerate freq < 1000 case where the quotient would be zero
            // and the catch-up loop below would never terminate.
            let delay_ticks = ((delay_ms * freq) / 1000).max(1);
            if self.timer_tick >= delay_ticks {
                if position + 1 == delays_ms.len() {
                    if complete {
                        // Loop the animation (viv.c:3243-3248; upstream has a
                        // play-once slideshow variant we don't build in #3).
                        position = 0;
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
        FrameAdvance { position, repaint }
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
        let adv = s.on_timer(99, FREQ, &delays, 0, true);
        assert_eq!(adv.position, 0);
        assert!(!adv.repaint);
    }

    #[test]
    fn one_full_delay_of_elapsed_time_advances_exactly_one_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        let adv = s.on_timer(100, FREQ, &delays, 0, true);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn partial_elapsed_time_accumulates_across_events() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        // Two 60 ms ticks: neither reaches the delay alone, together they do
        // (60 + 60 = 120 >= 100) and the leftover 20 ms is retained.
        let adv = s.on_timer(60, FREQ, &delays, 0, true);
        assert_eq!(adv.position, 0);
        let adv = s.on_timer(120, FREQ, &delays, 0, true);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn position_wraps_back_to_the_first_frame_after_the_last() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // Frame 1's delay elapses while sitting on the last frame.
        let adv = s.on_timer(100, FREQ, &delays, 1, true);
        assert_eq!(adv.position, 0);
        assert!(adv.repaint);
    }

    #[test]
    fn a_long_stall_elapses_at_most_one_second_per_event() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        // 10 seconds of wall time in one event: only 1 s is credited, so the
        // animation advances ten 100 ms frames (back to frame 1) instead of
        // a hundred.
        let adv = s.on_timer(10_000, FREQ, &delays, 0, true);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        // The next event measures from the truncation point, not real time.
        let adv = s.on_timer(10_100, FREQ, &delays, adv.position, true);
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
        let adv = s.on_timer(350, FREQ, &delays, 0, true);
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
        let adv = s.on_timer(25, FREQ, &delays, 0, true);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        let adv = s.on_timer(26, FREQ, &delays, adv.position, true);
        assert_eq!(adv.position, 2);
    }

    #[test]
    fn the_loaded_prefix_edge_waits_for_the_next_streamed_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // Sitting on the only loaded frame with decode still in flight: any
        // amount of elapsed time is discarded and the frame is held
        // (upstream viv.c:3233-3240).
        let adv = s.on_timer(250, FREQ, &delays, 1, false);
        assert_eq!(adv.position, 1);
        assert!(!adv.repaint);
        // The accumulator was zeroed, not banked: another long event while
        // still waiting must not fast-forward once the frame lands.
        let adv = s.on_timer(1_000, FREQ, &delays, 1, false);
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
        let adv = s.on_timer(150, FREQ, &delays, 0, false);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        let adv = s.on_timer(160, FREQ, &delays, adv.position, false);
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
        let adv = s.on_timer(150, FREQ, &delays, 1, true);
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
        let adv = s.on_timer(100, FREQ, &delays, 0, true);
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
}
