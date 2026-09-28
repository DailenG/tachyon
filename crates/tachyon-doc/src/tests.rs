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

/// A plain document's chunk lengths equal a from-scratch chunking of its current text.
fn assert_plain_matches_full_chunking(doc: &Document) {
    let fresh = Document::new_plain(&doc.buffer().text());
    let got: Vec<usize> = doc.blocks().iter().map(Block::len).collect();
    let want: Vec<usize> = fresh.blocks().iter().map(Block::len).collect();
    assert_eq!(got, want);
    assert!(doc.blocks().iter().all(|b| b.parsed().kind == BlockKind::Plain));
}

#[test]
fn mode_for_extension_recognizes_markdown_case_insensitively() {
    use std::path::Path;
    for name in ["a.md", "a.MD", "a.Markdown", "a.mdown", "a.mkd", "a.mkdn", "a.mdx"] {
        assert_eq!(mode_for_extension(Path::new(name)), DocMode::Markdown, "{name}");
    }
    for name in ["a.txt", "a.log", "a.rs", "a", "a.tar.gz", ".gitignore"] {
        assert_eq!(mode_for_extension(Path::new(name)), DocMode::Plain, "{name}");
    }
}

#[test]
fn plain_documents_never_parse_markdown() {
    let text = "# not a heading\n\n- not a list\n\n```rust\nnot a fence\n```\n";
    let doc = Document::new_plain(text);
    assert!(doc.blocks().iter().all(|b| b.parsed().kind == BlockKind::Plain));
    assert!(!doc.is_dirty());
    assert_eq!(doc.buffer().text(), text);
}

#[test]
fn plain_chunking_splits_at_the_line_cap_and_never_schedules_a_parse_job() {
    let text = "line\n".repeat(300);
    let mut doc = Document::new_plain(&text);
    assert!(doc.blocks().len() >= 2, "300 lines exceeds the 256-line cap");
    assert_eq!(doc.blocks().iter().map(Block::len).sum::<usize>(), text.len());
    assert_eq!(doc.block_at(0), Some(0));
    assert_eq!(doc.block_at(doc.len() - 1), Some(doc.blocks().len() - 1));
    assert!(doc.parse_job().is_none(), "plain documents never parse");
    assert!(!doc.is_dirty());
    doc.edit(0..0, "x").unwrap();
    assert!(doc.parse_job().is_none(), "an edit in plain mode leaves nothing dirty either");
    assert_plain_matches_full_chunking(&doc);
}

#[test]
fn a_pathologically_long_line_is_split_by_the_forced_cut() {
    // One 50 KiB line, no real line end at all: the forced cut must still bound every chunk.
    let text = "x".repeat(50 * 1024);
    let doc = Document::new_plain(&text);
    assert!(doc.blocks().len() > 1, "one unbroken line must still be split for shaping");
    assert!(doc.blocks().iter().map(Block::len).all(|len| len <= PLAIN_FORCED_CUT_BYTES));
    assert_eq!(doc.blocks().iter().map(Block::len).sum::<usize>(), text.len());
    assert_eq!(doc.buffer().text(), text, "the forced cut never changes the text");
}

#[test]
fn edits_across_a_plain_chunk_boundary_match_a_fresh_chunking() {
    let text = "line\n".repeat(500);
    let mut doc = Document::new_plain(&text);
    let boundary = doc.block_range(0).end;
    // Insert and delete text spanning the first chunk boundary.
    doc.edit(boundary - 3..boundary + 3, "REPLACED ACROSS THE BOUNDARY\n").unwrap();
    assert_plain_matches_full_chunking(&doc);

    // A large insertion that itself must split into several new chunks.
    let at = doc.len();
    doc.edit(at..at, &"more\n".repeat(2000)).unwrap();
    assert_plain_matches_full_chunking(&doc);

    // Deleting across several chunks merges them back down.
    let mid = doc.len() / 2;
    doc.edit(mid - 200..mid + 200, "").unwrap();
    assert_plain_matches_full_chunking(&doc);
}

#[test]
fn undo_restores_the_original_plain_chunking() {
    let text = "line\n".repeat(500);
    let mut doc = Document::new_plain(&text);
    let before: Vec<usize> = doc.blocks().iter().map(Block::len).collect();
    doc.edit(0..0, "prefix\n").unwrap();
    doc.seal_undo_group();
    assert_ne!(doc.blocks().iter().map(Block::len).collect::<Vec<_>>(), before);
    doc.undo().unwrap();
    assert_eq!(doc.buffer().text(), text);
    assert_eq!(doc.blocks().iter().map(Block::len).collect::<Vec<_>>(), before);
    assert_plain_matches_full_chunking(&doc);
}

#[test]
fn retagging_keeps_text_undo_history_and_version_across_modes() {
    let mut doc = Document::new("# Title\n\nbody\n");
    doc.edit(0..0, "x").unwrap();
    doc.seal_undo_group();
    let version = doc.buffer().version();
    let text = doc.buffer().text();

    let plain = doc.retagged(DocMode::Plain);
    assert_eq!(plain.mode(), DocMode::Plain);
    assert_eq!(plain.buffer().text(), text);
    assert_eq!(plain.buffer().version(), version);
    assert!(plain.blocks().iter().all(|b| b.parsed().kind == BlockKind::Plain));

    let mut back = plain.retagged(DocMode::Markdown);
    assert_eq!(back.mode(), DocMode::Markdown);
    assert_eq!(back.buffer().version(), version);
    back.undo().unwrap();
    back.reparse_now();
    assert_eq!(back.buffer().text(), "# Title\n\nbody\n");
    assert_matches_full_parse(&back);
}

#[test]
fn random_edits_to_a_plain_document_always_match_a_fresh_chunking() {
    // A cheap linear congruential generator: deterministic, no extra dev-dependency.
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    // Deliberately adversarial: lines short enough, and close enough to the cap, that a small
    // edit can plausibly ripple across more than one neighboring chunk.
    let mut doc = Document::new_plain(&"line\n".repeat(1000));
    for _ in 0..300 {
        let len = doc.len();
        if len == 0 {
            doc.edit(0..0, "line\n").unwrap();
            continue;
        }
        let a = (rand() as usize) % (len + 1);
        let b = (rand() as usize) % (len + 1);
        let (start, end) = (a.min(b), a.max(b));
        let insert = match rand() % 3 {
            0 => String::new(),
            1 => "x".repeat(1 + (rand() as usize) % 20),
            _ => "line\n".repeat(1 + (rand() as usize) % 5),
        };
        doc.edit(start..end, &insert).unwrap();
        assert_plain_matches_full_chunking(&doc);
    }
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

#[test]
fn a_paste_that_defines_its_references_needs_one_job() {
    let section = |n: usize| format!("See [ref-{n}] here.\n\n[ref-{n}]: /target/{n}\n\n");
    let paste: String = (0..50).map(section).collect();
    // The fillers keep the paragraph using [ref-49] out of the job's window
    // (the edit touches the last block, look-behind adds the one before).
    let mut doc =
        Document::new("# Notes\n\nUses [ref-49] before the paste.\n\nFiller.\n\nMore.\n\n");
    let end = doc.len();
    doc.edit(end..end, &paste).unwrap();

    let job = doc.parse_job().unwrap();
    assert_eq!(doc.apply(job.run()), Applied::Spliced);
    // Only the block before the paste looked up a label the paste defines.
    assert_eq!(doc.dirty_ranges(), [doc.block_range(1)]);
    doc.reparse_now();
    assert_matches_full_parse(&doc);
    assert_eq!(doc.blocks()[1].parsed().ir.links[0].dest, "/target/49");
}

#[test]
fn streamed_chunks_resolve_a_reference_defined_in_a_later_chunk() {
    // The last chunk of a streamed paste settles the document's definitions
    // off the parse job's thread (see `ParseJob::settle_definitions`); this
    // exercises that path with more than one chunk, not just the
    // single-job case above.
    let section = |n: usize| format!("See [ref-{n}] here.\n\n[ref-{n}]: /target/{n}\n\n");
    let paste: String = (0..50).map(section).collect();
    let mut doc =
        Document::new("# Notes\n\nUses [ref-49] before the paste.\n\nFiller.\n\nMore.\n\n");
    let end = doc.len();
    doc.edit(end..end, &paste).unwrap();

    // A small window forces the paste back in many chunks, like the
    // viewport-first streaming `Document::parse_job_near` does for a large
    // paste.
    let mut jobs = 0;
    while let Some(job) = doc.parse_job_near(doc.len(), 256) {
        assert_eq!(doc.apply(job.run()), Applied::Spliced);
        jobs += 1;
    }
    assert!(jobs > 1, "expected the paste to stream back in more than one chunk");
    assert_matches_full_parse(&doc);
    assert_eq!(doc.blocks()[1].parsed().ir.links[0].dest, "/target/49");
}

#[test]
fn find_all_is_smart_case_and_keeps_byte_offsets() {
    let doc = Document::new("Émile met emile. EMILE!\n");
    let starts = |q: &str| doc.find_all(q).into_iter().map(|r| r.start).collect::<Vec<_>>();
    // Lowercase query: ASCII case-insensitive; non-ASCII letters match exactly.
    assert_eq!(starts("mile"), vec![2, 12, 19]);
    assert_eq!(starts("emile"), vec![11, 18]);
    // An uppercase letter makes the search case-sensitive.
    assert_eq!(starts("EMILE"), vec![18]);
    assert_eq!(doc.find_all("Émile"), vec![0..6]);
    assert!(doc.find_all("").is_empty());
    let many = Document::new(&"a".repeat(MAX_FIND_MATCHES + 5));
    assert_eq!(many.find_all("a").len(), MAX_FIND_MATCHES);
}
