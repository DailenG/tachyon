//! Per-frame timing log for measurement runs (the overlay only shows a summary). With
//! `TACHYON_FRAME_LOG=<path>` every editor frame appends one line to `<path>`:
//!
//! `frame n=<n> at_ms=<ms since log start> busy_ms=<busy stretch> render_ms=<render+layout+paint>
//!  work=<label:ms,...> keys=<n> key_to_paint_ms=<ms,...> drawn=<blocks drawn> blocks=<blocks>`
//!
//! `busy_ms` counts render plus editor work that ran back to back with it (gaps under 1 ms), the
//! same accounting as the frame-time overlay. `key_to_paint_ms` is, for every key received since
//! the previous frame, the time from the key reaching the app to the end of this frame's paint.

use std::fs::File;
use std::io::Write as _;
use std::time::{Duration, Instant};

const CONTIGUOUS: Duration = Duration::from_millis(1);

pub struct FrameLog {
    file: File,
    epoch: Instant,
    frames: u64,
    started: Option<Instant>,
    work: Vec<(&'static str, Instant, Instant)>,
    keys: Vec<Instant>,
}

impl FrameLog {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("TACHYON_FRAME_LOG")?;
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        Some(Self {
            file,
            epoch: Instant::now(),
            frames: 0,
            started: None,
            work: Vec::new(),
            keys: Vec::new(),
        })
    }

    pub fn begin(&mut self) {
        self.started = Some(Instant::now());
    }

    pub fn work(&mut self, label: &'static str, started: Instant) {
        self.work.push((label, started, Instant::now()));
    }

    pub fn key(&mut self) {
        self.keys.push(Instant::now());
    }

    pub fn end(&mut self, drawn: usize, blocks: usize) {
        let Some(render_start) = self.started.take() else { return };
        let now = Instant::now();
        let mut start = render_start;
        for &(_, work_start, work_end) in self.work.iter().rev() {
            if work_end + CONTIGUOUS < start {
                break;
            }
            start = start.min(work_start);
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let work: Vec<String> =
            self.work.iter().map(|(label, s, e)| format!("{label}:{:.2}", ms(*e - *s))).collect();
        let keys: Vec<String> = self.keys.iter().map(|k| format!("{:.2}", ms(now - *k))).collect();
        self.frames += 1;
        let _ = writeln!(
            self.file,
            "frame n={} at_ms={:.1} busy_ms={:.2} render_ms={:.2} work={} keys={} key_to_paint_ms={} drawn={drawn} blocks={blocks}",
            self.frames,
            ms(render_start - self.epoch),
            ms(now - start),
            ms(now - render_start),
            if work.is_empty() { "-".to_owned() } else { work.join(",") },
            self.keys.len(),
            if keys.is_empty() { "-".to_owned() } else { keys.join(",") },
        );
        self.work.clear();
        self.keys.clear();
    }
}
