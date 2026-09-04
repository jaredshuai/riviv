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
//! M2 seams landing later: the background decode thread's
//! "next frame not decoded yet" branch (upstream resets `_viv_timer_tick`
//! and waits, viv.c:3233-3240 — with synchronous decode every frame is
//! always ready) and the animation-rate speed table (upstream
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
    /// (upstream `_viv_start_first_frame`, viv.c:14312-14317).
    pub(crate) fn new(tick_start: u64) -> Self {
        FrameScheduler {
            timer_tick: 0,
            tick_start,
        }
    }

    /// Restart from `tick_start` as if the animation had just been opened
    /// (upstream resets both fields whenever the displayed image or frame
    /// position is re-anchored, viv.c:1898/9274/10072/14315).
    pub(crate) fn restart(&mut self, tick_start: u64) {
        *self = FrameScheduler::new(tick_start);
    }

    /// Process one timer event measured at `now` (QPC ticks, `freq` ticks
    /// per second) against the per-frame `delays_ms` from `position`.
    ///
    /// When the accumulated time covers several frame delays the loop keeps
    /// advancing until the remainder is below the next delay — catch-up
    /// paints only the final position, exactly like upstream's loop
    /// (viv.c:3225-3272).
    pub(crate) fn on_timer(
        &mut self,
        now: u64,
        freq: u64,
        delays_ms: &[u32],
        position: usize,
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
                self.timer_tick -= delay_ticks;
                position += 1;
                if position == delays_ms.len() {
                    // Loop the animation (viv.c:3243-3248; upstream has a
                    // play-once slideshow variant we don't build in #3).
                    position = 0;
                }
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
        let adv = s.on_timer(99, FREQ, &delays, 0);
        assert_eq!(adv.position, 0);
        assert!(!adv.repaint);
    }

    #[test]
    fn one_full_delay_of_elapsed_time_advances_exactly_one_frame() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        let adv = s.on_timer(100, FREQ, &delays, 0);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn partial_elapsed_time_accumulates_across_events() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100, 100];
        // Two 60 ms ticks: neither reaches the delay alone, together they do
        // (60 + 60 = 120 >= 100) and the leftover 20 ms is retained.
        let adv = s.on_timer(60, FREQ, &delays, 0);
        assert_eq!(adv.position, 0);
        let adv = s.on_timer(120, FREQ, &delays, 0);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
    }

    #[test]
    fn position_wraps_back_to_the_first_frame_after_the_last() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        // Frame 1's delay elapses while sitting on the last frame.
        let adv = s.on_timer(100, FREQ, &delays, 1);
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
        let adv = s.on_timer(10_000, FREQ, &delays, 0);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        // The next event measures from the truncation point, not real time.
        let adv = s.on_timer(10_100, FREQ, &delays, adv.position);
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
        let adv = s.on_timer(350, FREQ, &delays, 0);
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
        let adv = s.on_timer(25, FREQ, &delays, 0);
        assert_eq!(adv.position, 1);
        assert!(adv.repaint);
        let adv = s.on_timer(26, FREQ, &delays, adv.position);
        assert_eq!(adv.position, 2);
    }

    #[test]
    fn restart_anchors_accumulation_to_the_given_tick() {
        let mut s = scheduler_starting_at(0);
        let delays = [100, 100];
        let _ = s.on_timer(50, FREQ, &delays, 0);
        // Reopening the image restarts the timeline: 50 ms of stale
        // accumulation must not survive into the new playback.
        s.restart(1_000);
        let adv = s.on_timer(1_050, FREQ, &delays, 0);
        assert_eq!(adv.position, 0);
        assert!(!adv.repaint);
        let adv = s.on_timer(1_100, FREQ, &delays, 0);
        assert_eq!(adv.position, 1);
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
