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

fn styled<'a>(ir: &'a BlockIr, style: Style) -> Vec<&'a str> {
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
    let table = DefTable::from_defs(&[("Ref".to_owned(), target)]);

    let with = &parse("See [ref] and [text][REF].\n", &table)[0];
    assert_eq!(with.ir.text, "See ref and text.");
    assert_eq!(with.ir.links.len(), 2);
    assert!(with.ir.links.iter().all(|l| l.dest == "/elsewhere"));
    assert_eq!(with.refs, vec!["REF".to_owned(), "ref".to_owned()]);

    let without = &parse("See [ref].\n", &DefTable::default())[0];
    assert_eq!(without.ir.text, "See [ref].");
    assert_eq!(without.refs, vec!["ref".to_owned()], "failed lookups are dependencies too");
}

#[test]
fn document_table_overrides_a_later_local_definition() {
    // In the full document the first definition wins; a window that only
    // sees the second one must still render the first.
    let first = LinkTarget { dest: "/first".into(), title: String::new() };
    let table = DefTable::from_defs(&[("x".to_owned(), first)]);
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
