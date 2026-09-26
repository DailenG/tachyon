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
    /// Take a job now and run/apply it after the following steps.
    StartJob,
    FinishJob,
    Reparse,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (any::<usize>(), prop_oneof![lines(), "[\n#>`*|\\-=\\[\\] a]{1,4}"])
            .prop_map(|(at, text)| Step::Insert { at, text }),
        3 => (any::<usize>(), 0usize..40).prop_map(|(at, len)| Step::Delete { at, len }),
        1 => Just(Step::Undo),
        1 => Just(Step::StartJob),
        1 => Just(Step::FinishJob),
        2 => Just(Step::Reparse),
    ]
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
                Step::StartJob => {
                    if job.is_none() {
                        job = doc.parse_job();
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
}
