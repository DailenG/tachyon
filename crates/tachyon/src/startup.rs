//! Startup milestones, measured from entry into `main`. Process creation and
//! loader time before `main` are covered by `cargo xtask bench-startup`, which
//! measures from `spawn` to the report line.

use std::io::Write;
use std::time::{Duration, Instant};

use gpui::Global;

pub struct Startup {
    main: Instant,
    marks: Vec<(&'static str, Duration)>,
    report: bool,
    finished: bool,
}

impl Global for Startup {}

impl Startup {
    pub fn begin() -> Self {
        Self { main: Instant::now(), marks: Vec::with_capacity(4), report: false, finished: false }
    }

    pub fn set_report(&mut self, report: bool) {
        self.report = report;
    }

    pub fn mark(&mut self, name: &'static str) {
        if !self.finished {
            self.marks.push((name, self.main.elapsed()));
        }
    }

    /// Records the first rendered frame. Returns whether the process should
    /// exit because it was launched only to report timings.
    pub fn finish(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.mark("first_frame");
        self.finished = true;
        if !self.report {
            return false;
        }
        let mut line = String::from("tachyon-startup");
        for (name, at) in &self.marks {
            line.push_str(&format!(" {name}_us={}", at.as_micros()));
        }
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
        true
    }
}
