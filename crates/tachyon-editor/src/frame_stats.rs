//! Frame-time overlay (toggle with `ctrl-alt-f`): how long the editor's
//! render, layout and paint took over recent frames, against the 60 Hz budget.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frames kept for the summary.
const WINDOW: usize = 240;

/// One 60 Hz frame.
pub const FRAME_BUDGET: Duration = Duration::from_micros(16_667);

#[derive(Default)]
pub struct FrameStats {
    started: Option<Instant>,
    samples: VecDeque<Duration>,
    over_budget: usize,
}

impl FrameStats {
    /// The editor started rendering a frame.
    pub fn begin(&mut self) {
        self.started = Some(Instant::now());
    }

    /// The editor finished painting the frame begun last.
    pub fn end(&mut self) {
        let Some(started) = self.started.take() else { return };
        self.record(started.elapsed());
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
}
