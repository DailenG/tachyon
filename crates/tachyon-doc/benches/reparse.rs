//! Reparse latency against the Phase 2 budgets. `cargo bench -p tachyon-doc`
//!
//! Plain harness (no criterion): the budgets are tail latencies (p99), which
//! this reports directly.

use std::hint::black_box;
use std::time::{Duration, Instant};

use tachyon_doc::Document;

/// Keystroke-to-clean-document budget on a 1 MB document (docs/ROADMAP.md).
const KEYSTROKE_P99_BUDGET: Duration = Duration::from_micros(500);

/// LLM-style Markdown: headings, prose with inline markup, nested lists,
/// fenced code, tables, quotes and reference links.
const SECTION: &str = r#"## Section {n}

This paragraph has **bold**, *emphasis*, `inline code`, a [link](https://example.com/{n})
and a [reference][ref-{n}]. It wraps over a couple of source lines so the
parser sees soft breaks, like real model output does.

1. First step with `cargo build --release`
   - nested detail
   - another detail with ~~strikethrough~~
2. Second step

```rust
fn section_{n}() -> usize {
    let values = vec![1, 2, 3];

    values.iter().sum()
}
```

| Column | Value | Notes |
|:-------|------:|-------|
| a      | {n}   | fine  |
| b      | 42    | also fine |

> A quoted remark about section {n}, with $x^2$ math.

[ref-{n}]: https://example.com/ref/{n}

"#;

fn document(bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + SECTION.len());
    let mut n = 0;
    while text.len() < bytes {
        text.push_str(&SECTION.replace("{n}", &n.to_string()));
        n += 1;
    }
    text
}

struct Stats {
    samples: Vec<Duration>,
}

impl Stats {
    fn percentile(&self, p: f64) -> Duration {
        let mut sorted = self.samples.clone();
        sorted.sort();
        let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
        sorted[rank - 1]
    }

    fn print(&self, name: &str) {
        println!(
            "{name:<44} p50 {:>9.1?}  p99 {:>9.1?}  max {:>9.1?}  (n={})",
            self.percentile(0.5),
            self.percentile(0.99),
            self.percentile(1.0),
            self.samples.len()
        );
    }
}

fn time(runs: usize, mut f: impl FnMut()) -> Stats {
    let samples = (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    Stats { samples }
}

/// Types `keys` at `at`, reparsing inline after every key like the editor
/// does for small jobs. Returns per-keystroke latency.
fn keystrokes(doc: &mut Document, mut at: usize, keys: &str) -> Stats {
    let mut samples = Vec::with_capacity(keys.len());
    for key in keys.chars() {
        let mut buf = [0u8; 4];
        let key = key.encode_utf8(&mut buf);
        let start = Instant::now();
        doc.edit(at..at, key).expect("offset is a char boundary");
        while let Some(job) = doc.parse_job() {
            let result = job.run();
            doc.apply(result);
        }
        samples.push(start.elapsed());
        at += key.len();
    }
    Stats { samples }
}

fn main() {
    // `cargo test` runs benches with --bench in test mode; keep that fast.
    let quick = std::env::args().any(|a| a == "--test");
    let (small, large) = if quick { (16 << 10, 64 << 10) } else { (1 << 20, 10 << 20) };
    let runs = if quick { 2 } else { 10 };

    let one_mb = document(small);
    let ten_mb = document(large);
    time(runs, || drop(black_box(Document::new(black_box(&one_mb)))))
        .print(&format!("full parse, {} KiB", one_mb.len() >> 10));
    time(runs.min(3), || drop(black_box(Document::new(black_box(&ten_mb)))))
        .print(&format!("full parse, {} KiB", ten_mb.len() >> 10));

    let keys =
        "Typing a sentence into the middle of a long document. ".repeat(if quick { 1 } else { 20 });
    let middle_paragraph = one_mb.len() / 2;
    // A large paste: what runs on the UI thread (rope insert, pre-segmenting
    // into unparsed blocks) before the background parse.
    let paste = document(if quick { 256 << 10 } else { 5 << 20 });
    time(runs, || {
        let mut doc = Document::new("start\n");
        let end = doc.len();
        doc.edit(end..end, black_box(&paste)).expect("end is a char boundary");
        black_box(doc.parse_job());
    })
    .print(&format!("paste {} KiB: UI-thread part", paste.len() >> 10));
    let mut pasted = Document::new("start\n");
    let end = pasted.len();
    pasted.edit(end..end, &paste).expect("end is a char boundary");
    let job = pasted.parse_job().expect("paste is dirty");
    let start = Instant::now();
    let result = job.run();
    let background = start.elapsed();
    let start = Instant::now();
    pasted.apply(result);
    println!(
        "{:<44} {:>9.1?}  (parse on background thread: {background:.1?})",
        format!("paste {} KiB: applying the parse", paste.len() >> 10),
        start.elapsed()
    );

    let mut doc = Document::new(&one_mb);
    let at = doc.buffer().text()[middle_paragraph..]
        .find("This paragraph")
        .expect("generated text has paragraphs")
        + middle_paragraph;
    let prose = keystrokes(&mut doc, at, &keys);
    prose.print("keystroke in paragraph, 1 MiB doc");

    let mut doc = Document::new(&one_mb);
    let at = doc.buffer().text()[middle_paragraph..]
        .find("values.iter()")
        .expect("generated text has code")
        + middle_paragraph;
    keystrokes(&mut doc, at, &keys).print("keystroke in code block, 1 MiB doc");

    let mut doc = Document::new(&one_mb);
    let end = doc.len();
    keystrokes(&mut doc, end, &keys).print("streaming at end, 1 MiB doc");

    let mut doc = Document::new(&one_mb);
    let at = doc.buffer().text()[middle_paragraph..]
        .find("## Section")
        .expect("generated text has headings")
        + middle_paragraph;
    let start = Instant::now();
    // No other `~~~` in the document: the fence runs to the end.
    doc.edit(at..at, "~~~\n").expect("offset is a char boundary");
    doc.reparse_now();
    println!("{:<44} {:>9.1?}", "unclosed fence to EOF (half of 1 MiB)", start.elapsed());

    if !quick {
        let p99 = prose.percentile(0.99);
        let verdict = if p99 <= KEYSTROKE_P99_BUDGET { "PASS" } else { "FAIL" };
        println!("\n{verdict}: keystroke p99 {p99:.1?} (budget {KEYSTROKE_P99_BUDGET:?})");
    }
}
