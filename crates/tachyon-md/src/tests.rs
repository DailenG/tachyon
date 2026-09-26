use super::*;

fn blocks(src: &str) -> Vec<ParsedBlock> {
    parse(src, &DefTable::from_source(src, options()))
}

fn kinds(src: &str) -> Vec<BlockKind> {
    blocks(src).into_iter().map(|b| b.kind).collect()
}

/// Source text of each block.
fn sources(src: &str) -> Vec<String> {
    let mut at = 0;
    blocks(src)
        .iter()
        .map(|b| {
            let s = src[at..at + b.len].to_owned();
            at += b.len;
            s
        })
        .collect()
}

fn styled(ir: &BlockIr, style: Style) -> Vec<&str> {
    ir.runs.iter().filter(|r| r.style.contains(style)).map(|r| &ir.text[r.range.clone()]).collect()
}

#[test]
fn blocks_tile_the_source_and_own_their_trailing_blank_lines() {
    let src = "\n\n# Title\n\nPara one\nstill one\n\n\n- a\n- b\n\n---\n";
    assert_eq!(
        kinds(src),
        vec![
            BlockKind::Heading(1),
            BlockKind::Paragraph,
            BlockKind::List { ordered: false },
            BlockKind::Rule
        ]
    );
    assert_eq!(
        sources(src),
        vec!["\n\n# Title\n\n", "Para one\nstill one\n\n\n", "- a\n- b\n\n", "---\n"]
    );
}

#[test]
fn empty_and_blank_sources() {
    assert!(blocks("").is_empty());
    let blank = blocks("  \n\n");
    assert_eq!(blank.len(), 1);
    assert_eq!((blank[0].kind.clone(), blank[0].len), (BlockKind::Blank, 4));
}

#[test]
fn inline_syntax_is_hidden_and_styled() {
    let b = &blocks("Some **bold _both_** and `code`, ~~gone~~ [link](https://x.dev) $e^x$\n")[0];
    assert_eq!(b.ir.text, "Some bold both and code, gone link e^x");
    assert_eq!(styled(&b.ir, Style::STRONG), vec!["bold ", "both"]);
    assert_eq!(styled(&b.ir, Style::EMPHASIS), vec!["both"]);
    assert_eq!(styled(&b.ir, Style::CODE), vec!["code"]);
    assert_eq!(styled(&b.ir, Style::STRIKETHROUGH), vec!["gone"]);
    assert_eq!(styled(&b.ir, Style::MATH), vec!["e^x"]);
    assert_eq!(b.ir.links, vec![LinkSpan { visible: 30..34, dest: "https://x.dev".into() }]);
    assert_eq!(&b.ir.text[30..34], "link");
}

#[test]
fn soft_breaks_join_lines_and_hard_breaks_split_them() {
    let b = &blocks("one\ntwo  \nthree\\\nfour\n")[0];
    assert_eq!(b.ir.text, "one two\nthree\nfour");
    assert_eq!(b.ir.lines.len(), 3);
}

#[test]
fn lists_carry_markers_nesting_and_task_state() {
    let src = "3. three\n4. four\n   - [x] done\n   - [ ] open\n   -\n";
    let b = &blocks(src)[0];
    let lines: Vec<(&str, u8, Option<Marker>)> =
        b.ir.lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let end = b.ir.lines.get(i + 1).map_or(b.ir.text.len(), |n| n.start - 1);
                (&b.ir.text[l.start..end], l.indent, l.marker)
            })
            .collect();
    assert_eq!(
        lines,
        vec![
            ("three", 1, Some(Marker::Ordered(3))),
            ("four", 1, Some(Marker::Ordered(4))),
            ("done", 2, Some(Marker::Task { checked: true })),
            ("open", 2, Some(Marker::Task { checked: false })),
            ("", 2, Some(Marker::Bullet)),
        ]
    );
}

#[test]
fn code_blocks_keep_blank_lines_and_drop_the_final_newline() {
    let src = "```rust title\nfn a() {}\n\nfn b() {}\n```\n";
    let b = &blocks(src)[0];
    assert_eq!(b.kind, BlockKind::CodeBlock { fenced: true, lang: Some("rust".into()) });
    assert_eq!(b.ir.text, "fn a() {}\n\nfn b() {}");
    assert!(b.ir.lines.iter().all(|l| l.kind == LineKind::Code));
    assert_eq!(b.ir.lines.len(), 3);
}

#[test]
fn unclosed_fence_runs_to_the_end_of_the_window() {
    assert_eq!(
        kinds("text\n\n```\ncode\n\n# not a heading\n"),
        vec![BlockKind::Paragraph, BlockKind::CodeBlock { fenced: true, lang: None }]
    );
}

#[test]
fn tables_have_rows_cells_and_alignment() {
    let src = "| a | b |\n|:--|--:|\n| 1 | 2 |\n";
    let b = &blocks(src)[0];
    assert_eq!(b.kind, BlockKind::Table(vec![Alignment::Left, Alignment::Right]));
    assert_eq!(b.ir.text, "a\tb\n1\t2");
    let kinds: Vec<_> = b.ir.lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![LineKind::TableRow { header: true }, LineKind::TableRow { header: false }]
    );
}

#[test]
fn quotes_nest_and_alerts_are_recognised() {
    let b = &blocks("> [!WARNING]\n> careful\n> > deeper\n")[0];
    assert_eq!(b.kind, BlockKind::BlockQuote(Some(QuoteKind::Warning)));
    let depths: Vec<u8> = b.ir.lines.iter().map(|l| l.quote).collect();
    assert_eq!(depths, vec![1, 2]);
}

#[test]
fn reference_links_resolve_through_the_document_table() {
    let target = LinkTarget { dest: "/elsewhere".into(), title: String::new() };
    let table = DefTable::new(&[("Ref".to_owned(), target)], &[]);

    let with = &parse("See [ref] and [text][REF].\n", &table)[0];
    assert_eq!(with.ir.text, "See ref and text.");
    assert_eq!(with.ir.links.len(), 2);
    assert!(with.ir.links.iter().all(|l| l.dest == "/elsewhere"));
    let found =
        |label: &str| LinkLookup { label: label.to_owned(), target: table.get(label).cloned() };
    assert_eq!(with.refs, vec![found("REF"), found("ref")]);

    let without = &parse("See [ref].\n", &DefTable::default())[0];
    assert_eq!(without.ir.text, "See [ref].");
    let missing = LinkLookup { label: "ref".to_owned(), target: None };
    assert_eq!(without.refs, vec![missing], "failed lookups are dependencies too");
}

#[test]
fn document_table_overrides_a_later_local_definition() {
    // In the full document the first definition wins; a window that only
    // sees the second one must still render the first.
    let first = LinkTarget { dest: "/first".into(), title: String::new() };
    let table = DefTable::new(&[("x".to_owned(), first)], &[]);
    let window = parse("[x]\n\n[x]: /second\n", &table);
    assert_eq!(window[0].ir.links[0].dest, "/first");
    assert_eq!(window[1].kind, BlockKind::LinkDefinition);
    assert_eq!(window[1].defs[0].1.dest, "/second");
}

#[test]
fn definitions_inside_containers_belong_to_the_container() {
    let b = blocks("> [a]: /in-quote\n> text [a]\n");
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].defs.len(), 1);
}

#[test]
fn source_map_round_trips_through_hidden_syntax() {
    let src = "Plain **bold** \\*esc\\* `code` end\n";
    let b = &blocks(src)[0];
    for span in b.ir.map.iter().filter(|s| s.verbatim) {
        assert_eq!(b.ir.text[span.visible.clone()], src[span.source.clone()]);
    }
    let bold = b.ir.text.find("bold").unwrap();
    let source = b.ir.visible_to_source(bold + 2);
    assert_eq!(&src[source..source + 2], "ld");
    assert_eq!(b.ir.source_to_visible(source), bold + 2);
    // Offsets inside hidden `**` snap to the next visible character.
    assert_eq!(b.ir.source_to_visible(src.find("**").unwrap()), bold);
}

#[test]
fn footnotes_and_html() {
    let src = "Claim[^1].\n\n[^1]: Source.\n\n<div>\n raw\n</div>\n";
    assert_eq!(
        kinds(src),
        vec![BlockKind::Paragraph, BlockKind::Footnote("1".into()), BlockKind::Html]
    );
    let b = blocks(src);
    assert_eq!(b[0].ir.text, "Claim[1].");
    assert_eq!(b[2].ir.text, "<div>\n raw\n</div>");
}

#[test]
fn a_footnote_definition_in_another_ones_continuation_stays_with_it() {
    // pulldown-cmark ends the first definition at the indented second one;
    // cut there, the second would be indented code on its own.
    let src = "[^1]: note\n\n\t[^2]: nested\n\nAfter.\n";
    assert_eq!(sources(src), vec!["[^1]: note\n\n\t[^2]: nested\n\n", "After.\n"]);
    for block in sources(src) {
        assert!(!kinds(&block).iter().any(|k| matches!(k, BlockKind::CodeBlock { .. })));
    }
    assert_eq!(
        kinds("Text.\n\n    code\n"),
        vec![BlockKind::Paragraph, BlockKind::CodeBlock { fenced: false, lang: None }]
    );
}

#[test]
fn link_definition_blocks_show_their_source_lines() {
    let src = "para\n\n[a]: /x\n[b]: /y\n";
    let b = &blocks(src)[1];
    assert_eq!(b.kind, BlockKind::LinkDefinition);
    assert_eq!(b.ir.text, "[a]: /x\n[b]: /y");
    assert_eq!(b.ir.lines.len(), 2);
    let at = b.ir.text.find("/y").unwrap();
    assert_eq!(&src[6 + b.ir.visible_to_source(at)..][..2], "/y");
}

#[test]
fn footnote_references_resolve_through_the_document_table() {
    let with = DefTable::new(&[], &["1".to_owned()]);
    let b = &parse("Claim[^1].\n", &with)[0];
    assert_eq!(b.ir.text, "Claim[1].");
    assert!(b.footnotes_seen.is_some());

    let without = &parse("Claim[^1].\n", &DefTable::default())[0];
    assert_eq!(without.ir.text, "Claim[^1].");
}

#[test]
fn footnote_prefix_resolves_even_when_the_window_ends_in_an_open_fence() {
    let table = DefTable::new(&[], &["n".to_owned()]);
    let src = "    indented\n\ntext[^n]\n\n```\nunclosed fence\n";
    let blocks = parse(src, &table);
    assert_eq!(blocks.iter().map(|b| b.len).sum::<usize>(), src.len());
    assert!(matches!(blocks[0].kind, BlockKind::CodeBlock { fenced: false, .. }));
    assert_eq!(blocks[1].ir.text, "text[n]");
    assert_eq!(blocks[2].ir.text, "unclosed fence");
    assert!(blocks.iter().all(|b| b.footnotes.is_empty()));
}

/// Source text of each leaf of the first block.
fn leaves(src: &str) -> Vec<String> {
    let b = &blocks(src)[0];
    b.ir.leaves.iter().map(|l| src[l.clone()].to_owned()).collect()
}

#[test]
fn list_items_are_leaves_with_their_markers() {
    let src = "- one\n- two\n  - nested\n-\n3. loose\n";
    assert_eq!(leaves(src), vec!["- one\n", "- two\n", "  - nested\n", "-\n"]);
    let b = &blocks(src)[0];
    let line_leaves: Vec<usize> = b.ir.lines.iter().map(|l| l.leaf).collect();
    assert_eq!(line_leaves, vec![0, 1, 2, 3]);
}

#[test]
fn leaves_cover_multi_line_content_inside_containers() {
    let src = "> first\n> para\n>\n> ```\n> code\n> ```\n";
    assert_eq!(leaves(src), vec!["> first\n> para\n", "> ```\n> code\n> ```\n"]);

    let src = "- item\n\n  ```\n  x\n\n  y\n  ```\n";
    let got = leaves(src);
    assert_eq!(got, vec!["- item\n", "  ```\n  x\n\n  y\n  ```\n"]);
    let b = &blocks(src)[0];
    assert!(b.ir.lines.iter().filter(|l| l.kind == LineKind::Code).all(|l| l.leaf == 1));
}

#[test]
fn leafy_top_level_blocks_have_one_leaf() {
    for src in ["para\ntwo\n", "# h\n", "| a |\n|---|\n| 1 |\n", "***\n", "```\nx\n```\n"] {
        let b = &blocks(src)[0];
        assert_eq!(b.ir.leaves, vec![0..src.len()], "{src:?}");
    }
}
