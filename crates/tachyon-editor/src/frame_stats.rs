//! Frame-time overlay (toggle with `ctrl-alt-f`): how long the editor kept
//! the UI thread busy per frame over recent frames, against the 60 Hz budget.
//! A frame's time is its render, layout and paint plus the editor work
//! (applying an edit or paste, applying a background parse result) that ran
//! back to back with it: work that ends less than [`CONTIGUOUS`] before the
//! frame (or before other such work) delays the frame. Work followed by an
//! idle gap, such as reading the clipboard before a paste lands in the next
//! frame, does not.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frames kept for the summary.
const WINDOW: usize = 240;

/// One 60 Hz frame.
pub const FRAME_BUDGET: Duration = Duration::from_micros(16_667);

/// Gaps shorter than this between pieces of UI-thread work count as none.
pub const CONTIGUOUS: Duration = Duration::from_millis(1);

#[derive(Default)]
pub struct FrameStats {
    started: Option<Instant>,
    /// Editor work since the last frame, `(start, end)` in order.
    work: Vec<(Instant, Instant)>,
    samples: VecDeque<Duration>,
    over_budget: usize,
}

impl FrameStats {
    /// The editor started rendering a frame.
    pub fn begin(&mut self) {
        self.begin_at(Instant::now());
    }

    /// The editor finished painting the frame begun last.
    pub fn end(&mut self) {
        self.end_at(Instant::now());
    }

    /// Editor work outside rendering that ran from `started` until now.
    pub fn work(&mut self, started: Instant) {
        self.work.push((started, Instant::now()));
    }

    fn begin_at(&mut self, now: Instant) {
        self.started = Some(now);
    }

    fn end_at(&mut self, now: Instant) {
        let Some(mut start) = self.started.take() else { return };
        for &(work_start, work_end) in self.work.iter().rev() {
            if work_end + CONTIGUOUS < start {
                break;
            }
            start = start.min(work_start);
        }
        self.work.clear();
        self.record(now - start);
    }

    fn record(&mut self, frame: Duration) {
        if self.samples.len() == WINDOW
            && let Some(old) = self.samples.pop_front()
            && old > FRAME_BUDGET
        {
            self.over_budget -= 1;
        }
        if frame > FRAME_BUDGET {
            self.over_budget += 1;
        }
        self.samples.push_back(frame);
    }

    /// `(p50, max, frames over budget, frames)` over the recent window.
    pub fn summary(&self) -> Option<(Duration, Duration, usize, usize)> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<Duration> = self.samples.iter().copied().collect();
        sorted.sort();
        let p50 = sorted[(sorted.len() - 1) / 2];
        let max = *sorted.last().unwrap_or(&p50);
        Some((p50, max, self.over_budget, sorted.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_tracks_the_recent_window_and_budget_misses() {
        let mut stats = FrameStats::default();
        assert!(stats.summary().is_none());
        stats.record(Duration::from_millis(40));
        for _ in 0..WINDOW - 1 {
            stats.record(Duration::from_millis(2));
        }
        let (p50, max, over, n) = stats.summary().unwrap();
        assert_eq!(
            (p50, max, over, n),
            (Duration::from_millis(2), Duration::from_millis(40), 1, WINDOW)
        );

        // The slow frame ages out of the window.
        stats.record(Duration::from_millis(3));
        let (_, max, over, n) = stats.summary().unwrap();
        assert_eq!((max, over, n), (Duration::from_millis(3), 0, WINDOW));
    }

    #[test]
    fn work_counts_toward_the_frame_it_runs_into() {
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        let mut stats = FrameStats::default();
        // Read the clipboard (8 ms), idle 5 ms, apply the paste (4 ms) right
        // before a 3 ms frame: the frame took 7 ms, not 15.
        stats.work.push((t0, t0 + ms(8)));
        stats.work.push((t0 + ms(13), t0 + ms(17)));
        stats.begin_at(t0 + ms(17));
        stats.end_at(t0 + ms(20));
        // The same work back to back, then the frame: one 23 ms stretch.
        stats.work.push((t0 + ms(30), t0 + ms(38)));
        stats.work.push((t0 + ms(38), t0 + ms(50)));
        stats.begin_at(t0 + ms(50));
        stats.end_at(t0 + ms(53));
        // Nothing carries over to the next frame.
        stats.begin_at(t0 + ms(60));
        stats.end_at(t0 + ms(62));

        assert_eq!(Vec::from(stats.samples.clone()), vec![ms(7), ms(23), ms(2)]);
        assert_eq!(stats.over_budget, 1);
    }
}
