use proptest::prelude::*;
use tachyon_md::{DefTable, options, parse};

/// Lines that exercise block structure boundaries: fences, lists, quotes,
/// tables, setext underlines, definitions, HTML, math and escapes.
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
    "- [ ] task",
    "> quote",
    "> > deep",
    "| a | b |",
    "|---|:-:|",
    "[x]: /u",
    "[x]",
    "[y][x]",
    "<div>",
    "</div>",
    "$$",
    "$a$",
    "\\*",
    "&amp;",
    "[^1]",
    "[^1]: note",
    "é😀中",
    "\t tab",
];

fn document() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..30).prop_map(|lines| {
        let mut doc = lines.join("\n");
        doc.push('\n');
        doc
    })
}

proptest! {
    /// Any input parses without panicking into blocks that tile the source,
    /// with well-formed IR: sorted, in-bounds maps whose verbatim spans match
    /// the source exactly.
    #[test]
    fn parse_output_is_well_formed(src in prop_oneof![document(), "\\PC{0,200}"]) {
        let blocks = parse(&src, &DefTable::from_source(&src, options()));
        prop_assert_eq!(blocks.iter().map(|b| b.len).sum::<usize>(), src.len());

        let mut start = 0;
        for block in &blocks {
            let source = &src[start..start + block.len];
            prop_assert!(block.content.start <= block.content.end && block.content.end <= block.len);
            let ir = &block.ir;
            for line in &ir.lines {
                prop_assert!(line.start <= ir.text.len());
            }
            let mut last_visible = 0;
            let mut last_source = 0;
            for span in &ir.map {
                prop_assert!(span.visible.start >= last_visible && span.visible.end <= ir.text.len());
                prop_assert!(span.source.start >= last_source && span.source.end <= block.len);
                if span.verbatim {
                    prop_assert_eq!(&ir.text[span.visible.clone()], &source[span.source.clone()]);
                }
                last_visible = span.visible.end;
                last_source = span.source.end;
            }
            for run in &ir.runs {
                prop_assert!(run.range.end <= ir.text.len() && !run.range.is_empty());
            }
            start += block.len;
        }
    }
}
