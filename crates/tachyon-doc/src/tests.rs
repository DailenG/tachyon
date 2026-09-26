use super::*;
use tachyon_md::BlockKind;

fn kinds(doc: &Document) -> Vec<BlockKind> {
    doc.blocks().iter().map(|b| b.parsed().kind.clone()).collect()
}

fn ids(doc: &Document) -> Vec<BlockId> {
    doc.blocks().iter().map(Block::id).collect()
}

/// Blocks equal a from-scratch parse of the current text.
fn assert_matches_full_parse(doc: &Document) {
    let fresh = Document::new(&doc.buffer().text());
    assert!(!doc.is_dirty());
    let got: Vec<&ParsedBlock> = doc.blocks().iter().map(Block::parsed).collect();
    let want: Vec<&ParsedBlock> = fresh.blocks().iter().map(Block::parsed).collect();
    assert_eq!(got, want);
    assert!(doc.blocks().iter().all(|b| !b.is_stale()));
}

#[test]
fn edits_mark_blocks_stale_until_reparsed_and_keep_ids() {
    let mut doc = Document::new("# Title\n\nfirst para\n\nsecond para\n");
    let before = ids(&doc);
    let at = doc.buffer().text().find("first").unwrap();
    doc.edit(at..at, "the ").unwrap();

    assert!(doc.blocks()[1].is_stale());
    assert_eq!(doc.blocks()[1].len(), "the first para\n\n".len());
    assert!(doc.is_dirty());

    doc.reparse_now();
    assert_eq!(ids(&doc), before, "untouched and edited blocks keep their identity");
    assert_eq!(doc.blocks()[1].parsed().ir.text, "the first para");
    assert_matches_full_parse(&doc);
}

#[test]
fn typing_a_blank_line_splits_a_paragraph() {
    let mut doc = Document::new("one\ntwo\n");
    doc.take_splices();
    doc.edit(4..4, "\n").unwrap();
    doc.reparse_now();
    assert_eq!(kinds(&doc), vec![BlockKind::Paragraph, BlockKind::Paragraph]);
    let splices = doc.take_splices();
    assert_eq!(splices.last(), Some(&Splice { old: 0..1, new_len: 2 }));
    assert_matches_full_parse(&doc);
}

#[test]
fn opening_a_fence_swallows_the_rest_and_closing_it_restores_blocks() {
    let text = "intro\n\n# A\n\npara\n\n- item\n";
    let mut doc = Document::new(text);
    doc.edit(7..7, "```\n").unwrap();
    doc.reparse_now();
    assert_eq!(kinds(&doc).len(), 2);
    assert!(matches!(kinds(&doc)[1], BlockKind::CodeBlock { .. }));
    assert_matches_full_parse(&doc);

    let end = doc.len();
    doc.edit(end..end, "```\n").unwrap();
    doc.reparse_now();
    assert_matches_full_parse(&doc);
}

#[test]
fn setext_underline_changes_the_previous_block() {
    let mut doc = Document::new("Title\n# heading\n");
    let at = "Title\n".len();
    doc.edit(at..at + "# heading".len(), "===").unwrap();
    doc.reparse_now();
    assert_eq!(kinds(&doc), vec![BlockKind::Heading(1)]);
    assert_matches_full_parse(&doc);
}

#[test]
fn result_is_discarded_when_an_edit_touches_its_window() {
    let mut doc = Document::new("aaa\n\nbbb\n\nccc\n");
    doc.edit(0..0, "x").unwrap();
    let job = doc.parse_job().unwrap();
    doc.edit(1..1, "y").unwrap();
    assert_eq!(doc.apply(job.run()), Applied::Discarded);
    assert!(doc.is_dirty());
    doc.reparse_now();
    assert_eq!(doc.blocks()[0].parsed().ir.text, "xyaaa");
    assert_matches_full_parse(&doc);
}

#[test]
fn result_applies_after_edits_elsewhere() {
    let mut doc = Document::new("aaa\n\nbbb\n\nccc\n\nddd\n\neee\n");
    let end = doc.len();
    doc.edit(end..end, "\nfff\n").unwrap();
    let job = doc.parse_job().unwrap();
    doc.edit(0..0, "zz").unwrap();
    assert_eq!(doc.apply(job.run()), Applied::Spliced);
    doc.reparse_now();
    assert_matches_full_parse(&doc);
}

#[test]
fn only_the_outstanding_job_applies() {
    let mut doc = Document::new("a\n");
    doc.edit(0..0, "b").unwrap();
    let job = doc.parse_job().unwrap();
    assert!(doc.parse_job().is_none(), "one job at a time");
    doc.cancel_job();
    let replacement = doc.parse_job().unwrap();
    assert_eq!(doc.apply(job.run()), Applied::Ignored);
    assert_eq!(doc.apply(replacement.run()), Applied::Spliced);
    assert!(!doc.is_dirty());
}

#[test]
fn adding_a_definition_rerenders_earlier_references() {
    let mut doc = Document::new("See [docs].\n\nmiddle\n");
    assert_eq!(doc.blocks()[0].parsed().ir.text, "See [docs].");
    let end = doc.len();
    doc.edit(end..end, "\n[docs]: https://example.com\n").unwrap();
    doc.reparse_now();
    assert_eq!(doc.blocks()[0].parsed().ir.text, "See docs.");
    assert_eq!(doc.blocks()[0].parsed().ir.links[0].dest, "https://example.com");
    assert_matches_full_parse(&doc);
}

#[test]
fn large_paste_shows_unparsed_blocks_until_parsed() {
    let mut doc = Document::new("start\n");
    let chunk = "## Section\n\nSome *text* here.\n\n```\ncode\n\nmore\n```\n\n";
    let paste = chunk.repeat(UNPARSED_SPLIT_THRESHOLD / chunk.len() + 1);
    doc.edit(doc.len()..doc.len(), &paste).unwrap();

    assert!(doc.blocks().len() >= UNPARSED_SPLIT_THRESHOLD / UNPARSED_CHUNK / 2);
    assert!(doc.blocks().iter().all(|b| b.is_stale()));
    assert!(doc.blocks().iter().any(|b| b.parsed().kind == BlockKind::Unparsed));
    let job = doc.parse_job().unwrap();
    assert!(!job.is_small());
    doc.apply(job.run());
    doc.reparse_now();
    assert_matches_full_parse(&doc);
}

#[test]
fn undo_restores_the_original_parse() {
    let text = "# T\n\n- a\n- b\n\n> q\n";
    let mut doc = Document::new(text);
    doc.edit(0..doc.len(), "").unwrap();
    assert!(doc.blocks().is_empty());
    doc.seal_undo_group();
    doc.edit(0..0, "new").unwrap();
    doc.seal_undo_group();
    doc.undo().unwrap();
    doc.undo().unwrap();
    doc.reparse_now();
    assert_eq!(doc.buffer().text(), text);
    assert_matches_full_parse(&doc);
}

#[test]
fn block_lookup_by_offset() {
    let doc = Document::new("a\n\nb\n");
    assert_eq!(doc.block_range(0), 0..3);
    assert_eq!((doc.block_at(0), doc.block_at(2), doc.block_at(3)), (Some(0), Some(0), Some(1)));
    assert_eq!(doc.block_at(doc.len()), Some(1));
    assert_eq!(Document::new("").block_at(0), None);
}
