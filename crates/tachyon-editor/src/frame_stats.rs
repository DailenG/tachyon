//! Frame-time overlay (toggle with `ctrl-alt-f`): how long the editor kept
//! the UI thread busy per frame over recent frames, against the 60 Hz budget.
//! A frame's time is its render, layout and paint plus the editor work done
//! since the previous frame (applying an edit or paste, applying a
//! background parse result), which delays the frame just the same.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frames kept for the summary.
const WINDOW: usize = 240;

/// One 60 Hz frame.
pub const FRAME_BUDGET: Duration = Duration::from_micros(16_667);

#[derive(Default)]
pub struct FrameStats {
    started: Option<Instant>,
    /// Editor work since the last frame, charged to the next one.
    work: Duration,
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
        let work = std::mem::take(&mut self.work);
        self.record(started.elapsed() + work);
    }

    /// Editor work outside rendering that took `elapsed`.
    pub fn work(&mut self, elapsed: Duration) {
        self.work += elapsed;
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
    fn work_between_frames_is_charged_to_the_next_frame_only() {
        let mut stats = FrameStats::default();
        stats.work(FRAME_BUDGET);
        stats.begin();
        stats.end();
        stats.begin();
        stats.end();
        let (_, max, over, n) = stats.summary().unwrap();
        assert!(max > FRAME_BUDGET);
        assert_eq!((over, n), (1, 2));
    }
}
