//! The editor's central invariant: however the text got to its current
//! state, and however parse jobs interleaved with edits, a clean document's
//! blocks equal a from-scratch parse of its text.

use proptest::prelude::*;
use tachyon_doc::{Applied, Block, Document};
use tachyon_md::ParsedBlock;

/// Lines that stress block boundaries: fences, lists, quotes, tables,
/// setext underlines, definitions and references, HTML, math.
const FRAGMENTS: &[&str] = &[
    "",
    "text",
    "more *em* **strong** `code`",
    "# heading",
    "===",
    "---",
    "***",
    "```",
    "~~~",
    "```rust",
    "    indented",
    "- item",
    "  - nested",
    "1. one",
    "2) two",
    "- [x] task",
    "> quote",
    "> > deep",
    "| a | b |",
    "|---|:-:|",
    "[x]: /u",
    "[y]: /v",
    "[x]",
    "[y][x]",
    "<div>",
    "</div>",
    "$$",
    "$a$",
    "[^1]",
    "[^1]: note",
    "é😀",
    "\t tab",
];

/// Nested same-line footnote definitions ("[^1]:[^1]: x") make pulldown-cmark
/// attribute ranges in ways the segmenter cannot keep identical between a
/// window and the whole document. For them only termination and tiling are
/// guaranteed, checked separately below.
const PATHOLOGICAL: &[&str] = &["[^1]:[^1]: x", "[^1]: note", "[x]: /u", "- item", "text", ""];

fn lines() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..6).prop_map(|l| l.join("\n"))
}

fn document() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..25).prop_map(|l| {
        let mut doc = l.join("\n");
        doc.push('\n');
        doc
    })
}

#[derive(Clone, Debug)]
enum Step {
    Insert {
        at: usize,
        text: String,
    },
    Delete {
        at: usize,
        len: usize,
    },
    Undo,
    /// Take a job now and run/apply it after the following steps. `max`
    /// bounds the window (streamed chunks), `focus` picks the dirty range.
    StartJob {
        focus: usize,
        max: usize,
    },
    FinishJob,
    Reparse,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (any::<usize>(), prop_oneof![lines(), "[\n#>`*|\\-=\\[\\] a]{1,4}"])
            .prop_map(|(at, text)| Step::Insert { at, text }),
        3 => (any::<usize>(), 0usize..40).prop_map(|(at, len)| Step::Delete { at, len }),
        1 => Just(Step::Undo),
        1 => (any::<usize>(), window_limit()).prop_map(|(focus, max)| Step::StartJob { focus, max }),
        1 => Just(Step::FinishJob),
        2 => Just(Step::Reparse),
    ]
}

/// Small limits force streamed chunks in these short documents.
fn window_limit() -> impl Strategy<Value = usize> {
    prop_oneof![1 => Just(usize::MAX), 3 => 1usize..64]
}

fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn parsed(doc: &Document) -> Vec<&ParsedBlock> {
    doc.blocks().iter().map(Block::parsed).collect()
}

proptest! {

    #[test]
    fn incremental_parse_equals_full_parse(initial in document(), steps in prop::collection::vec(step(), 1..25)) {
        let mut doc = Document::new(&initial);
        let mut job = None;
        for step in steps {
            let text = doc.buffer().text();
            match step {
                Step::Insert { at, text: insert } => {
                    let at = floor_boundary(&text, at % (text.len() + 1));
                    doc.edit(at..at, &insert).unwrap();
                    doc.seal_undo_group();
                }
                Step::Delete { at, len } => {
                    let at = floor_boundary(&text, at % (text.len() + 1));
                    let end = floor_boundary(&text, at + len).max(at);
                    doc.edit(at..end, "").unwrap();
                    doc.seal_undo_group();
                }
                Step::Undo => { doc.undo(); }
                Step::StartJob { focus, max } => {
                    if job.is_none() {
                        job = doc.parse_job_near(focus % (text.len() + 1), max);
                    }
                }
                Step::FinishJob => {
                    if let Some(j) = job.take() {
                        let outcome = doc.apply(j.run());
                        prop_assert_ne!(outcome, Applied::Ignored);
                    }
                }
                Step::Reparse => {
                    job = None;
                    doc.reparse_now();
                }
            }
            // Blocks always tile the text, clean or not.
            prop_assert_eq!(doc.blocks().iter().map(Block::len).sum::<usize>(), doc.len());
        }
        if let Some(j) = job.take() {
            doc.apply(j.run());
        }
        doc.reparse_now();

        let fresh = Document::new(&doc.buffer().text());
        prop_assert!(!doc.is_dirty());
        prop_assert!(doc.blocks().iter().all(|b| !b.is_stale()));
        prop_assert_eq!(parsed(&doc), parsed(&fresh));
    }

    /// Streaming an LLM answer: text arrives in small chunks at the end, and
    /// the document is reparsed after every chunk.
    #[test]
    fn streamed_output_equals_full_parse(text in document(), chunk in 1usize..24) {
        let mut doc = Document::new("");
        let mut at = 0;
        while at < text.len() {
            let end = floor_boundary(&text, (at + chunk).min(text.len())).max(at + 1);
            let end = if text.is_char_boundary(end) { end } else { text.len() };
            doc.edit(doc.len()..doc.len(), &text[at..end]).unwrap();
            doc.reparse_now();
            at = end;
        }
        let fresh = Document::new(&text);
        prop_assert_eq!(parsed(&doc), parsed(&fresh));
    }

    /// Pathological input still reparses to a clean document whose blocks
    /// tile the text without empty blocks.
    #[test]
    fn pathological_footnotes_terminate(
        initial in prop::collection::vec(prop::sample::select(PATHOLOGICAL), 0..12),
        edits in prop::collection::vec((any::<usize>(), prop::sample::select(PATHOLOGICAL)), 1..8),
    ) {
        let mut doc = Document::new(&initial.join("\n"));
        for (at, insert) in edits {
            let text = doc.buffer().text();
            let at = floor_boundary(&text, at % (text.len() + 1));
            doc.edit(at..at, &format!("{insert}\n")).unwrap();
            doc.reparse_now();
            prop_assert!(!doc.is_dirty());
            prop_assert!(doc.blocks().iter().all(|b| !b.is_empty()));
            prop_assert_eq!(doc.blocks().iter().map(Block::len).sum::<usize>(), doc.len());
        }
    }
}

/// A paste large enough to be split into unparsed blocks, streamed back in
/// chunks around a moving viewport, parses like a full parse.
///
/// Not in `proptest!`: that applies `PROPTEST_CASES` over any configured
/// count, and each case streams more than 64 KiB in up to a few hundred
/// chunks (every chunk carries the document's footnote definitions), so a
/// million cases would take hours. Edits interleaved with small chunks are
/// covered by `incremental_parse_equals_full_parse`.
#[test]
fn streamed_chunks_of_a_large_paste_equal_full_parse() {
    let config =
        ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() };
    let strategy = (
        prop::collection::vec(document(), 1..8),
        prop::collection::vec(any::<usize>(), 1..8),
        1024usize..(64 * 1024),
    );
    let result =
        proptest::test_runner::TestRunner::new(config).run(&strategy, |(parts, focuses, max)| {
            let mut paste = String::new();
            while paste.len() <= tachyon_doc::UNPARSED_SPLIT_THRESHOLD {
                for part in &parts {
                    paste.push_str(part);
                    paste.push('\n');
                }
            }
            let mut doc = Document::new("# before\n\n");
            doc.edit(doc.len()..doc.len(), &paste).unwrap();
            let mut focuses = focuses.into_iter().cycle();
            let mut jobs = 0;
            while let Some(job) =
                doc.parse_job_near(focuses.next().unwrap_or(0) % (doc.len() + 1), max)
            {
                prop_assert_eq!(doc.apply(job.run()), Applied::Spliced);
                jobs += 1;
                prop_assert!(jobs < 100_000, "streaming does not terminate");
            }
            let fresh = Document::new(&doc.buffer().text());
            prop_assert!(!doc.is_dirty());
            prop_assert_eq!(parsed(&doc), parsed(&fresh));
            Ok(())
        });
    if let Err(e) = result {
        panic!("{e}");
    }
}
