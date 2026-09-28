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

/// A plain document's blocks tile its text exactly, each within
/// [`PLAIN_MIN_CHUNK_BYTES`]/[`PLAIN_MIN_CHUNK_LINES`] and [`PLAIN_CHUNK_BYTES`]/
/// [`PLAIN_CHUNK_LINES`] of the maximum unless it is the document's own last block or
/// immediately follows a forced cut (a display-only break inside an over-long line, whose
/// remainder can be short too), and each ends at a real line end, a forced cut (a UTF-8 char
/// boundary with no `\n` anywhere inside it), or the end of the text. This is the invariant
/// `Document::on_edit_plain` maintains locally after every edit, undo and redo - deliberately
/// *not* a comparison against a from-scratch chunking of the same text, since the two are no
/// longer required to agree (that requirement was the bug: it forced every edit to re-derive
/// every later boundary from the document start).
fn assert_plain_chunking_is_valid(doc: &Document) {
    let rope = doc.buffer().rope();
    let total = doc.len();
    let blocks = doc.blocks();
    let n = blocks.len();
    assert!(blocks.iter().all(|b| b.parsed().kind == BlockKind::Plain));
    let mut at = 0usize;
    for (i, block) in blocks.iter().enumerate() {
        let len = block.len();
        assert!(len > 0, "block {i} is empty");
        let end = at + len;
        assert!(len <= PLAIN_CHUNK_BYTES, "block {i} ({at}..{end}) is {len} bytes, over the max");
        let lines = rope.byte_to_line(end) - rope.byte_to_line(at);
        assert!(
            lines <= PLAIN_CHUNK_LINES,
            "block {i} ({at}..{end}) is {lines} lines, over the max"
        );

        let ends_at_newline = rope.byte(end - 1) == b'\n';
        let is_doc_end = end == total;
        let is_forced_cut = !ends_at_newline
            && !is_doc_end
            && floor_char_boundary(rope, end) == end
            && find_newline(rope, at, end).is_none();
        assert!(
            ends_at_newline || is_doc_end || is_forced_cut,
            "block {i} ({at}..{end}) ends neither at a line end, the end of the text, nor a valid forced cut"
        );

        let is_last = i + 1 == n;
        let follows_forced_cut = at > 0 && rope.byte(at - 1) != b'\n';
        let big_enough = len >= PLAIN_MIN_CHUNK_BYTES || lines >= PLAIN_MIN_CHUNK_LINES;
        assert!(
            big_enough || is_last || follows_forced_cut,
            "block {i} ({at}..{end}, {len} bytes, {lines} lines) is under the minimum, and \
             neither the last block nor bounded by a forced cut"
        );
        at = end;
    }
    assert_eq!(at, total, "blocks must tile the text exactly");
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
    assert_plain_chunking_is_valid(&doc);
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
fn edits_across_a_plain_chunk_boundary_keep_the_chunking_valid() {
    let text = "line\n".repeat(500);
    let mut doc = Document::new_plain(&text);
    let boundary = doc.block_range(0).end;
    // Insert and delete text spanning the first chunk boundary.
    doc.edit(boundary - 3..boundary + 3, "REPLACED ACROSS THE BOUNDARY\n").unwrap();
    assert_plain_chunking_is_valid(&doc);

    // A large insertion that itself must split into several new chunks.
    let at = doc.len();
    doc.edit(at..at, &"more\n".repeat(2000)).unwrap();
    assert_plain_chunking_is_valid(&doc);

    // Deleting across several chunks merges them back down.
    let mid = doc.len() / 2;
    doc.edit(mid - 200..mid + 200, "").unwrap();
    assert_plain_chunking_is_valid(&doc);
}

#[test]
fn undo_restores_the_original_text_and_keeps_the_plain_chunking_valid() {
    // Undo is no longer required to reproduce the exact same chunk boundaries as before the
    // edit (only a from-scratch chunking made that promise, and requiring it is what made every
    // edit re-derive every later boundary from the document start - the bug this invariant
    // fixes); it only has to restore the text and leave the chunking valid.
    let text = "line\n".repeat(500);
    let mut doc = Document::new_plain(&text);
    doc.edit(0..0, "prefix\n").unwrap();
    doc.seal_undo_group();
    assert_ne!(doc.buffer().text(), text);
    doc.undo().unwrap();
    assert_eq!(doc.buffer().text(), text);
    assert_plain_chunking_is_valid(&doc);
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
fn random_edits_undo_and_redo_to_a_plain_document_keep_the_chunking_valid() {
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
        match rand() % 8 {
            // Undo/redo must leave the chunking within the invariant, like any other edit -
            // including when they revert or reapply more than one change in a group
            // (`Document::undo`/`redo` reconcile each change against the buffer state that one
            // change alone produced, not the group's final state; see their docs).
            0 => {
                doc.undo();
            }
            1 => {
                doc.redo();
            }
            2 => doc.seal_undo_group(),
            _ => {
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
            }
        }
        assert_plain_chunking_is_valid(&doc);
    }
}

/// Blocks a splice touched (old range or new length, whichever is larger), summed over every
/// splice: the metric a caller cares about, since GPUI's virtualized list has to remeasure and
/// relayout exactly this many items.
fn splice_cost(splices: &[Splice]) -> usize {
    splices.iter().map(|s| s.old.len().max(s.new_len)).sum()
}

/// Regression test for a stall where the first undo after typing a few characters at the top of
/// a large plain document cost time proportional to the whole document (measured on Windows: 18
/// ms on a 20 MB file's ~2500 chunks, over 2 s on a 200 MB, ~1 million line log). `Document::undo`
/// used to apply the whole undo group to the buffer first and only afterward call `on_edit` once
/// per edit in it, so by the time `on_edit_plain` reconciled edit *N* the buffer already
/// reflected every edit *after* it too; its convergence search compared a per-edit shift against
/// content that had already moved past it and never matched, so it rescanned to the end of the
/// document once per edit in the group. A timing assertion would be flaky across machines; block
/// count is not - splicing anywhere near the whole document is unambiguous evidence of the bug
/// regardless of how fast the machine running the test is.
#[test]
fn undo_and_redo_of_a_typed_group_touch_only_the_edited_region_in_a_large_plain_document() {
    let lines = 50_000;
    let mut doc = Document::new_plain(&"line some text here\n".repeat(lines));
    let total_blocks = doc.blocks().len();
    assert!(total_blocks > 100, "needs many blocks for the assertion below to be meaningful");

    // One undo group: three keystrokes at the very top, exactly the reported repro.
    doc.edit(0..0, "a").unwrap();
    doc.edit(1..1, "b").unwrap();
    doc.edit(2..2, "c").unwrap();
    doc.take_splices();

    let edits = doc.undo().unwrap();
    assert_eq!(edits.len(), 3, "one edit per keystroke in the group");
    let touched = splice_cost(&doc.take_splices());
    assert!(touched <= 8, "undo touched {touched} of {total_blocks} blocks: not bounded");
    assert_plain_chunking_is_valid(&doc);

    let edits = doc.redo().unwrap();
    assert_eq!(edits.len(), 3);
    let touched = splice_cost(&doc.take_splices());
    assert!(touched <= 8, "redo touched {touched} of {total_blocks} blocks: not bounded");
    assert_plain_chunking_is_valid(&doc);
}

/// Like the above, but for a group with edits scattered far apart (as "replace all" produces:
/// one undo group, one change per match anywhere in the document) instead of adjacent keystrokes.
#[test]
fn undo_of_a_scattered_group_touches_only_the_edited_regions_in_a_large_plain_document() {
    let lines = 50_000;
    let mut doc = Document::new_plain(&"line some text here\n".repeat(lines));
    let total_blocks = doc.blocks().len();

    // Editing from the end backward keeps each offset valid without recomputing it, while still
    // recording one undo group with changes at the start, middle and end of the document.
    let len = doc.len();
    doc.edit(len..len, "X").unwrap();
    doc.edit(len / 2..len / 2, "X").unwrap();
    doc.edit(0..0, "X").unwrap();
    doc.take_splices();

    doc.undo().unwrap();
    let touched = splice_cost(&doc.take_splices());
    assert!(touched <= 8, "scattered undo touched {touched} of {total_blocks} blocks: not bounded");
    assert_plain_chunking_is_valid(&doc);
}

/// The core regression test for the structural bug this invariant fixes: an edit that shifts
/// the line count by an amount that is not a multiple of [`PLAIN_CHUNK_LINES`] (pressing Enter,
/// the reported case) touches only the edited block and, at most, a small constant number of its
/// immediate neighbours - never every later block - regardless of how large the document is.
/// The old chunking anchored every boundary to an exact line/byte count from the document start,
/// so shifting the line count anywhere rippled every boundary after it to the end of the
/// document; this asserts that no longer happens, at a size (1,000,000 lines) where it used to.
#[test]
fn an_edit_touches_a_bounded_number_of_blocks_regardless_of_document_size() {
    for lines in [1_000usize, 50_000, 1_000_000] {
        let mut doc = Document::new_plain(&"line some text here\n".repeat(lines));
        let total_blocks = doc.blocks().len();
        doc.take_splices();

        // Enter near the very top: shifts every later line's index by one, not a multiple of
        // `PLAIN_CHUNK_LINES`.
        doc.edit(5..5, "\n").unwrap();
        let touched = splice_cost(&doc.take_splices());
        assert!(
            touched <= 8,
            "{lines}-line document: Enter near the top touched {touched} of {total_blocks} \
             blocks, not bounded"
        );
        assert_plain_chunking_is_valid(&doc);
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

/// A fresh load bigger than several [`super::LOAD_CHUNK`]s, well-structured (a blank line every
/// couple of lines), streams back in windows cut at pre-segmenter boundaries and matches a
/// direct `md::parse_document` of the same text exactly - the ground truth `load_markdown` is
/// meant to reproduce, not `Document::new` again (which would just call `load_markdown` too).
#[test]
fn a_large_well_structured_load_streams_and_matches_a_full_parse() {
    let block = "## Heading\n\nA paragraph of ordinary prose to pad out the file.\n\n";
    let text = block.repeat(LOAD_CHUNK * 3 / block.len() + 1);
    assert!(text.len() > LOAD_CHUNK * 2, "test needs several load windows");
    let doc = Document::new(&text);
    assert!(!doc.is_dirty());
    assert!(doc.blocks().iter().all(|b| !b.is_stale()));
    let (want, _) = tachyon_md::parse_document(&text);
    let got: Vec<&ParsedBlock> = doc.blocks().iter().map(Block::parsed).collect();
    assert_eq!(got, want.iter().collect::<Vec<_>>());
}

/// A fresh load that is one enormous block with no blank line anywhere (the pathological shape:
/// a huge fenced block, or - as here - one paragraph of soft-wrapped prose) has nowhere for
/// [`super::LOAD_CHUNK`]-sized windows to cut safely. `load_markdown` must fall back to one
/// direct parse of the whole thing rather than loop, and the result must still match a full
/// parse exactly.
#[test]
fn a_pathological_no_blank_line_load_falls_back_to_one_direct_parse() {
    let line = "Some prose that keeps going without a blank line anywhere in the file.\n";
    let text = line.repeat(LOAD_CHUNK * 3 / line.len() + 1);
    assert!(text.len() > LOAD_CHUNK * 2, "test needs to exceed the streaming margin");
    let doc = Document::new(&text);
    assert!(!doc.is_dirty());
    assert!(doc.blocks().iter().all(|b| !b.is_stale()));
    assert_eq!(doc.blocks().len(), 1, "no blank line anywhere collapses to one block");
    let (want, _) = tachyon_md::parse_document(&text);
    let got: Vec<&ParsedBlock> = doc.blocks().iter().map(Block::parsed).collect();
    assert_eq!(got, want.iter().collect::<Vec<_>>());
}

/// A well-structured prefix, one pathological no-blank-line stretch in the middle, then a
/// well-structured suffix again: `load_markdown` streams the prefix, falls back to one direct
/// parse for the rest once it hits the pathological stretch (rather than looping forever
/// re-widening a window that can never converge), and the result still matches a full parse -
/// including the suffix, which the fallback's one remaining window covers too.
#[test]
fn a_pathological_stretch_after_a_normal_prefix_still_parses_correctly() {
    let normal = "## Heading\n\nA short paragraph.\n\n".repeat(50);
    let line = "No blank line in this stretch at all, just prose that keeps going.\n";
    let pathological = line.repeat(LOAD_CHUNK * 3 / line.len() + 1);
    let suffix = "\n## After\n\nMore ordinary text.\n\n";
    let text = format!("{normal}{pathological}{suffix}");
    let doc = Document::new(&text);
    assert!(!doc.is_dirty());
    assert!(doc.blocks().iter().all(|b| !b.is_stale()));
    let (want, _) = tachyon_md::parse_document(&text);
    let got: Vec<&ParsedBlock> = doc.blocks().iter().map(Block::parsed).collect();
    assert_eq!(got, want.iter().collect::<Vec<_>>());
}

#[test]
fn evicting_blocks_drops_ir_and_ensure_ir_restores_it_exactly() {
    let text = "# A\n\npara one\n\npara two\n\npara three\n\n# B\n\nend\n";
    let mut doc = Document::new(text);
    assert!(!doc.is_dirty());
    let before: Vec<ParsedBlock> = doc.blocks().iter().map(|b| b.parsed().clone()).collect();
    assert!(before.len() > 1, "fixture sanity: several blocks to evict");

    // Keep only block 0 - everything else is "far from the viewport".
    doc.evict(0..1);
    for (i, block) in doc.blocks().iter().enumerate() {
        if i == 0 {
            assert!(!block.is_ir_evicted(), "the kept block keeps its ir");
            continue;
        }
        assert!(block.is_ir_evicted(), "block {i} outside keep should be evicted");
        assert!(block.parsed().ir.lines.is_empty(), "evicted ir is empty");
        // Kind, byte length, hash and whatever definitions need all survive untouched.
        assert_eq!(block.parsed().kind, before[i].kind);
        assert_eq!(block.parsed().len, before[i].len);
        assert_eq!(block.parsed().source_hash, before[i].source_hash);
        assert_eq!(block.parsed().defs, before[i].defs);
        assert_eq!(block.parsed().refs, before[i].refs);
        assert_eq!(block.parsed().footnotes, before[i].footnotes);
    }

    for i in 0..doc.blocks().len() {
        doc.ensure_ir(i);
    }
    let after: Vec<ParsedBlock> = doc.blocks().iter().map(|b| b.parsed().clone()).collect();
    assert_eq!(after, before, "restoring every evicted block reproduces the original parse");
    assert!(doc.blocks().iter().all(|b| !b.is_ir_evicted()));
}

#[test]
fn ensure_ir_re_resolves_a_forward_reference_correctly() {
    let text = "See [ref] here.\n\nmiddle\n\n[ref]: https://example.com\n";
    let mut doc = Document::new(text);
    assert!(!doc.is_dirty());
    let original_dest = doc.blocks()[0].parsed().ir.links[0].dest.clone();
    assert_eq!(original_dest, "https://example.com");

    // Evict the referencing block itself (keep only the definition's own block in view).
    doc.evict(2..3);
    assert!(doc.blocks()[0].is_ir_evicted());
    doc.ensure_ir(0);
    assert!(!doc.blocks()[0].is_ir_evicted());
    assert_eq!(doc.blocks()[0].parsed().ir.links[0].dest, original_dest);
}

#[test]
fn evict_leaves_stale_and_oversized_blocks_alone() {
    let text = "intro\n\nbody\n";
    let mut doc = Document::new(text);
    doc.edit(0..0, "x").unwrap();
    // Block 0 is now stale (pending reparse); evicting must not touch it.
    doc.evict(1..1);
    assert!(!doc.blocks()[0].is_ir_evicted(), "a stale block is never evicted");
    assert!(doc.blocks()[0].is_stale());

    let big = "y".repeat(EVICT_MAX_LEN + 1);
    let mut doc = Document::new(&big);
    doc.evict(1..1);
    assert!(
        !doc.blocks()[0].is_ir_evicted(),
        "a block over EVICT_MAX_LEN is left resident, even outside keep"
    );
}
