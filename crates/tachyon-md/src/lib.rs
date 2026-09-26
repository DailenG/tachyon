//! Markdown → blocks. [`parse`] splits a window of source into top-level
//! blocks that tile it exactly and builds each block's owned [`BlockIr`].
//!
//! `pulldown-cmark` is not incremental; `tachyon-doc` gets incrementality by
//! reparsing windows of whole blocks. Everything a block's rendering depends
//! on is either inside the block or in the document-wide [`DefTable`], so a
//! block parses identically alone or as part of the whole document.
//!
//! This crate must never depend on GPUI.

mod defs;
mod ir;
mod presegment;

use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use pulldown_cmark::{
    BlockQuoteKind, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag,
    TagEnd,
};

pub use crate::defs::{DefTable, LinkTarget};
pub use crate::ir::{BlockIr, LineInfo, LineKind, LinkSpan, Marker, SourceSpan, Style, StyleRun};
pub use crate::presegment::presegment;

/// Markdown dialect: CommonMark plus the GFM extensions LLM output relies on
/// (tables, task lists, strikethrough, footnotes, alerts) and `$` math.
pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading(u8),
    CodeBlock {
        fenced: bool,
        lang: Option<String>,
    },
    BlockQuote(Option<QuoteKind>),
    List {
        ordered: bool,
    },
    Table(Vec<Alignment>),
    Rule,
    Html,
    Footnote(String),
    LinkDefinition,
    /// Whitespace-only source.
    Blank,
    /// Source not parsed yet (large insertions awaiting a background parse),
    /// shown as plain lines.
    Unparsed,
}

/// One top-level block of a parse window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedBlock {
    pub kind: BlockKind,
    /// Source bytes covered, including leading blank lines (first block only)
    /// and trailing blank lines up to the next block.
    pub len: usize,
    /// Range pulldown-cmark attributed to the block, relative to its start.
    pub content: Range<usize>,
    pub ir: BlockIr,
    /// Link reference definitions made in this block, in source order.
    pub defs: Vec<(String, LinkTarget)>,
    /// Reference labels this block's rendering looked up in the
    /// [`DefTable`], with the entry it found. The rendering is stale exactly
    /// when the table now answers differently for one of them.
    pub refs: Vec<LinkLookup>,
    /// Footnote labels defined in this block.
    pub footnotes: Vec<String>,
    /// Set when the source contains `[^`, so the rendering may depend on
    /// which footnotes the document defines: the [`DefTable::footnote_key`]
    /// it rendered against. The rendering is stale when the key changes.
    pub footnotes_seen: Option<u64>,
    /// Hash of the block's source text.
    pub source_hash: u64,
}

/// A reference label looked up in the [`DefTable`] and the entry found
/// (`None`: undefined, which is a dependency too).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkLookup {
    pub label: String,
    pub target: Option<LinkTarget>,
}

/// Splits `src` into blocks whose `len`s sum to `src.len()` and builds their
/// IR, resolving reference links through `defs`. Empty input yields no blocks.
pub fn parse(src: &str, defs: &DefTable) -> Vec<ParsedBlock> {
    if src.is_empty() {
        return Vec::new();
    }

    // A footnote reference renders as one only if the document defines the
    // footnote, and pulldown-cmark resolves footnotes only within its input.
    // Prepend the document's footnote definitions, closed off by a thematic
    // break so nothing in the window can continue them; everything parsed
    // from this prefix is dropped and ranges are shifted back.
    let prefix = defs.footnote_prefix();
    let offset = prefix.len();
    let input: Cow<'_, str> =
        if prefix.is_empty() { Cow::Borrowed(src) } else { Cow::Owned(prefix + src) };
    let input = input.as_ref();

    let lookups = std::cell::RefCell::new(Vec::<(usize, String)>::new());
    let callback = |link: BrokenLink<'_>| {
        if let Some(at) = link.span.start.checked_sub(offset) {
            lookups.borrow_mut().push((at, link.reference.to_string()));
        }
        defs.get(&link.reference)
            .map(|t| (CowStr::from(t.dest.clone()), CowStr::from(t.title.clone())))
    };
    let mut events =
        Parser::new_with_broken_link_callback(input, options(), Some(callback)).into_offset_iter();

    let mut pending: Vec<Pending> = Vec::new();
    let mut depth = 0usize;
    let mut builder: Option<Builder<'_>> = None;
    for (event, range) in events.by_ref() {
        if range.start < offset {
            // Prefix: only keep the nesting depth right.
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth -= 1,
                _ => {}
            }
            continue;
        }
        let range = range.start - offset..range.end - offset;
        // Container ranges can run past their content (over trailing link
        // definitions); every other event marks real content.
        let container = matches!(
            event,
            Event::Start(Tag::List(_) | Tag::BlockQuote(_) | Tag::FootnoteDefinition(_))
                | Event::End(TagEnd::List(_) | TagEnd::BlockQuote(_) | TagEnd::FootnoteDefinition)
        );
        // Item ranges span nested items, so they do not grow a leaf either.
        let structural =
            container || matches!(event, Event::Start(Tag::Item) | Event::End(TagEnd::Item));
        if let Some(b) = builder.as_mut() {
            if !container {
                b.extent_end = b.extent_end.max(range.end);
            }
            b.last_offset = range.start;
        }
        let touch = (!structural).then(|| range.clone());
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    let mut b = Builder::new(src, range.start, kind_of(&tag), defs);
                    if !container {
                        b.extent_end = range.end;
                    }
                    builder = Some(b);
                }
                depth += 1;
                if let Some(b) = builder.as_mut() {
                    b.start(tag);
                    b.touch(touch);
                }
            }
            Event::End(end) => {
                depth -= 1;
                if let Some(b) = builder.as_mut() {
                    b.end(end);
                    b.touch(touch);
                }
                if depth == 0
                    && let Some(b) = builder.take()
                {
                    pending.push(b.finish(range));
                }
            }
            event if depth == 0 => {
                // Top-level leaf events: thematic breaks (and anything else
                // pulldown emits outside a container).
                let kind =
                    if matches!(event, Event::Rule) { BlockKind::Rule } else { BlockKind::Html };
                let mut b = Builder::new(src, range.start, kind, defs);
                b.extent_end = range.end;
                b.last_offset = range.start;
                b.event(event, range.clone());
                b.touch(touch);
                pending.push(b.finish(range));
            }
            event => {
                if let Some(b) = builder.as_mut() {
                    b.event(event, range);
                    b.touch(touch);
                }
            }
        }
    }

    drop(events);

    // Source no event covers is link reference definitions (the only
    // construct pulldown-cmark consumes without events). They become blocks
    // of their own, found from the gaps rather than from the parser's
    // definition map, which drops duplicate labels.
    pending.sort_by_key(|p| p.content.start);
    let mut cursor = 0;
    let mut gaps = Vec::new();
    for p in &pending {
        if p.content.start > cursor {
            gaps.push(cursor..p.content.start);
        }
        cursor = cursor.max(p.content.end);
    }
    if cursor < src.len() {
        gaps.push(cursor..src.len());
    }
    let sorted = pending.len();
    for gap in gaps {
        let text = &src[gap.clone()];
        let Some(first) = text.find(|c: char| !c.is_whitespace()) else { continue };
        let defs = gap.start + first..gap.start + text.trim_end().len();
        // Definitions are taken from the start of a paragraph, so without a
        // blank line they belong to the block that follows it. That block
        // must include them to parse the same alone: after a definition an
        // indented line is paragraph text, on its own it is code.
        let next_idx = pending[..sorted].partition_point(|p| p.content.start < gap.end);
        let next = pending[..sorted].get_mut(next_idx);
        match next {
            Some(next) if src[defs.end..next.content.start].matches('\n').count() <= 1 => {
                next.content.start = defs.start;
            }
            _ => pending.push(Pending::definitions(src, defs)),
        }
    }
    pending.sort_by_key(|p| p.content.start);
    attach_leading_definitions(src, &mut pending);

    for (at, label) in lookups.into_inner() {
        let owner = pending.partition_point(|p| p.content.start <= at).checked_sub(1);
        if let Some(owner) = owner.and_then(|i| pending.get_mut(i)) {
            owner.refs.push(label);
        }
    }

    tile(src, pending, defs)
}

/// Parses a complete document and returns its blocks together with the
/// definition table incremental reparses of it must use. The table is built
/// from the blocks (as `tachyon-doc` does after every reparse); if it differs
/// from what the whole-document parse saw, the document is parsed again with
/// it so the blocks are exactly what windowed reparses would produce.
pub fn parse_document(src: &str) -> (Vec<ParsedBlock>, DefTable) {
    let initial = DefTable::from_source(src, options());
    let mut blocks = parse(src, &initial);
    let table = DefTable::from_blocks(&blocks);
    if table.links_equal(&initial) {
        // The whole document resolves its own footnotes, so the blocks
        // render as they would against `table`'s footnotes: record those.
        let key = table.footnote_key();
        for block in &mut blocks {
            if block.footnotes_seen.is_some() {
                block.footnotes_seen = Some(key);
            }
        }
        return (blocks, table);
    }
    let blocks = parse(src, &table);
    let table = DefTable::from_blocks(&blocks);
    (blocks, table)
}

/// A link reference definition leaves the parser in a paragraph-like state:
/// in "[x]: /u\n2) two" the second line cannot start a list, while alone it
/// does. pulldown-cmark may attribute such definitions to the *previous*
/// block's (container) range. Move definition lines that directly precede a
/// block (no blank line between) into that block so it parses the same alone.
fn attach_leading_definitions(src: &str, pending: &mut [Pending]) {
    for i in 1..pending.len() {
        let start = line_start(src, pending[i].content.start);
        let after_content = pending[i - 1].extent_end.min(start);
        let tail = &src[after_content..start];
        // Only lines after the last blank line touch the block.
        let tail_start = after_content + tail.rfind("\n\n").map_or(0, |at| at + 2);
        let tail = &src[tail_start..start];
        let Some(first) = tail.find(|c: char| !c.is_whitespace()) else { continue };
        // Definitions produce no events; anything else is not ours to move.
        if Parser::new_ext(tail, options()).next().is_some() {
            continue;
        }
        let defs_start = tail_start + first;
        if pending[i].kind == BlockKind::LinkDefinition {
            // Its IR shows its whole source; rebuild it over the new range.
            pending[i] = Pending::definitions(src, defs_start..pending[i].content.end);
        }
        pending[i].content.start = pending[i].content.start.min(defs_start);
        pending[i - 1].content.end = pending[i - 1].content.end.min(line_start(src, defs_start));
    }
}

/// Offset just after the `\n` ending the line that contains `offset - 1`
/// (so an offset already at a line start stays put).
fn line_end_inclusive(src: &str, offset: usize) -> usize {
    if offset == 0 || src.as_bytes()[offset - 1] == b'\n' {
        return offset;
    }
    src[offset..].find('\n').map_or(src.len(), |nl| offset + nl + 1)
}

fn line_start(src: &str, offset: usize) -> usize {
    src[..offset].rfind('\n').map_or(0, |nl| nl + 1)
}

/// A placeholder for `len` bytes of source not parsed yet (a large paste
/// waiting for its background parse). It has no IR: until the parse lands,
/// the editor shows it as raw source, which it takes from the document.
/// Costs nothing per byte, so a multi-megabyte paste stays within a frame.
pub fn unparsed(len: usize) -> ParsedBlock {
    ParsedBlock {
        kind: BlockKind::Unparsed,
        len,
        content: 0..len,
        ir: BlockIr::default(),
        defs: Vec::new(),
        refs: Vec::new(),
        footnotes: Vec::new(),
        footnotes_seen: None,
        source_hash: 0,
    }
}

/// IR showing `src` verbatim, one visible line per source line.
fn plain_lines_ir(src: &str) -> BlockIr {
    let mut ir = BlockIr::default();
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if !ir.lines.is_empty() {
            ir.text.push('\n');
        }
        let start = ir.text.len();
        ir.lines.push(LineInfo {
            start,
            kind: LineKind::Text,
            indent: 0,
            quote: 0,
            marker: None,
            leaf: 0,
        });
        ir.text.push_str(content);
        if !content.is_empty() {
            ir.map.push(SourceSpan {
                visible: start..start + content.len(),
                source: offset..offset + content.len(),
                verbatim: true,
            });
        }
        offset += line.len();
    }
    if !src.is_empty() {
        ir.leaves.push(0..src.len());
    }
    ir
}

/// A block before tiling: content range still absolute in the window.
struct Pending {
    kind: BlockKind,
    content: Range<usize>,
    /// Offset the IR's source ranges are relative to.
    origin: usize,
    /// End of the block's real content (see `Builder::extent_end`).
    extent_end: usize,
    /// Leaf source ranges, absolute in the window.
    leaves: Vec<Range<usize>>,
    ir: BlockIr,
    refs: Vec<String>,
    footnotes: Vec<String>,
}

impl Pending {
    /// Appends a block that starts on this block's last line.
    fn absorb(&mut self, other: Pending) {
        let text_shift = if self.ir.text.is_empty() && self.ir.lines.is_empty() {
            0
        } else {
            self.ir.text.push('\n');
            self.ir.text.len()
        };
        let source_shift = other.origin - self.origin;
        let leaf_shift = self.leaves.len();
        let ir = other.ir;
        self.ir.text.push_str(&ir.text);
        self.ir.lines.extend(ir.lines.into_iter().map(|mut line| {
            line.start += text_shift;
            line.leaf += leaf_shift;
            line
        }));
        self.ir.runs.extend(ir.runs.into_iter().map(|mut run| {
            run.range = run.range.start + text_shift..run.range.end + text_shift;
            run
        }));
        self.ir.links.extend(ir.links.into_iter().map(|mut link| {
            link.visible = link.visible.start + text_shift..link.visible.end + text_shift;
            link
        }));
        let last_source = self.ir.map.last().map_or(0, |span| span.source.end);
        self.ir.map.extend(
            ir.map
                .into_iter()
                .map(|mut span| {
                    span.visible = span.visible.start + text_shift..span.visible.end + text_shift;
                    span.source = span.source.start + source_shift..span.source.end + source_shift;
                    span
                })
                .filter(|span| span.source.start >= last_source),
        );
        self.leaves.extend(other.leaves);
        self.refs.extend(other.refs);
        self.footnotes.extend(other.footnotes);
        self.content.end = self.content.end.max(other.content.end);
        self.extent_end = self.extent_end.max(other.extent_end);
    }

    /// Link reference definitions in `span`, shown verbatim line by line.
    fn definitions(src: &str, span: Range<usize>) -> Self {
        let ir = plain_lines_ir(&src[span.clone()]);
        Pending {
            kind: BlockKind::LinkDefinition,
            origin: span.start,
            extent_end: span.end,
            leaves: vec![span.clone()],
            content: span,
            ir,
            refs: Vec::new(),
            footnotes: Vec::new(),
        }
    }
}

/// Assigns each block the source from its start to the next block's start
/// (the first block also takes leading whitespace), then makes IR source
/// ranges relative to the block start.
///
/// A block starts at the beginning of the line holding its content, not at
/// the content itself: pulldown-cmark reports e.g. indented code without its
/// indentation, and a block cut mid-line would parse differently on its own
/// ("    code" is code, "code" is a paragraph).
/// Leading indentation of `line` in columns (tabs stop every 4), counted up
/// to 4.
fn indent_columns(line: &str) -> usize {
    let mut columns = 0;
    for byte in line.bytes() {
        match byte {
            b' ' => columns += 1,
            b'\t' => columns = (columns / 4 + 1) * 4,
            _ => break,
        }
        if columns >= 4 {
            break;
        }
    }
    columns
}

fn tile(src: &str, pending: Vec<Pending>, defs: &DefTable) -> Vec<ParsedBlock> {
    if pending.is_empty() {
        return vec![ParsedBlock {
            kind: BlockKind::Blank,
            len: src.len(),
            content: 0..0,
            ir: BlockIr::default(),
            defs: Vec::new(),
            refs: Vec::new(),
            footnotes: Vec::new(),
            footnotes_seen: None,
            source_hash: hash(src),
        }];
    }
    // Blocks start at the beginning of their content's line. Two blocks
    // on one line (an empty footnote definition followed by another) become
    // one block: a mid-line cut would parse differently on its own. So does
    // a block indented like code that is not code: it continues a container
    // (a footnote definition nested in another's continuation lines), and
    // alone it would be indented code.
    let mut starts: Vec<usize> = Vec::with_capacity(pending.len() + 1);
    let mut merged: Vec<Pending> = Vec::with_capacity(pending.len());
    let mut previous_end = 0;
    for p in pending {
        let start =
            if merged.is_empty() { 0 } else { line_start(src, p.content.start).max(previous_end) };
        // Real content, not the container range: a list's range can run into
        // the indentation of the next block's first line.
        previous_end = previous_end.max(p.extent_end);
        match merged.last_mut() {
            // The first block's start is 0, not its content's line.
            Some(last)
                if starts
                    .last()
                    .is_some_and(|&s| start <= s.max(line_start(src, last.content.start)))
                    || (indent_columns(&src[start..]) >= 4
                        && !matches!(p.kind, BlockKind::CodeBlock { fenced: false, .. })) =>
            {
                last.absorb(p)
            }
            _ => {
                starts.push(start);
                merged.push(p);
            }
        }
    }
    let pending = merged;
    let footnote_key = defs.footnote_key();
    starts.push(src.len());
    pending
        .into_iter()
        .enumerate()
        .map(|(i, mut p)| {
            let (start, end) = (starts[i], starts[i + 1]);
            // The block's IR was built with offsets relative to its content
            // start; shift them to be relative to the tiled block start.
            let shift = p.origin - start;
            if shift > 0 {
                for span in &mut p.ir.map {
                    span.source = span.source.start + shift..span.source.end + shift;
                }
            }
            // Leaves as whole source lines, so list and quote markers are
            // part of their raw text. Leaves sharing a line merge.
            let mut leaves: Vec<Range<usize>> = Vec::with_capacity(p.leaves.len());
            let mut remap = Vec::with_capacity(p.leaves.len());
            for leaf in &p.leaves {
                let s = line_start(src, leaf.start.clamp(start, end)).max(start);
                // An empty leaf (an empty list item) is its marker line.
                let content_end = if leaf.is_empty() { s + 1 } else { leaf.end };
                let e = line_end_inclusive(src, content_end.clamp(s, end)).min(end);
                let (s, e) = (s - start, e - start);
                match leaves.last_mut() {
                    Some(last) if s < last.end => last.end = last.end.max(e),
                    _ => leaves.push(s..e),
                }
                remap.push(leaves.len() - 1);
            }
            for line in &mut p.ir.lines {
                line.leaf = remap.get(line.leaf).copied().unwrap_or(0);
            }
            p.ir.leaves = leaves;
            let content_end = p.content.end.min(end);
            p.refs.sort();
            p.refs.dedup();
            let refs = p
                .refs
                .into_iter()
                .map(|label| LinkLookup { target: defs.get(&label).cloned(), label })
                .collect();
            ParsedBlock {
                kind: p.kind,
                len: end - start,
                content: p.content.start - start..content_end - start,
                ir: p.ir,
                defs: block_defs(&src[start..end]),
                refs,
                footnotes: p.footnotes,
                footnotes_seen: src[start..end].contains("[^").then_some(footnote_key),
                source_hash: hash(&src[start..end]),
            }
        })
        .collect()
}

/// Link reference definitions made in one block's source. Parsed from the
/// block alone (it parses identically alone), so duplicates of labels
/// defined elsewhere are still reported.
fn block_defs(block_src: &str) -> Vec<(String, LinkTarget)> {
    if !block_src.contains("]:") {
        return Vec::new();
    }
    let parser = Parser::new_ext(block_src, options());
    let mut defs: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(label, def)| {
            let target = LinkTarget {
                dest: def.dest.to_string(),
                title: def.title.as_deref().unwrap_or_default().to_owned(),
            };
            (def.span.start, label.to_owned(), target)
        })
        .collect();
    defs.sort_by_key(|(start, ..)| *start);
    defs.into_iter().map(|(_, label, target)| (label, target)).collect()
}

fn hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn kind_of(tag: &Tag<'_>) -> BlockKind {
    match tag {
        Tag::Paragraph => BlockKind::Paragraph,
        Tag::Heading { level, .. } => BlockKind::Heading(*level as u8),
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => BlockKind::CodeBlock {
            fenced: true,
            lang: info.split_whitespace().next().map(str::to_owned),
        },
        Tag::CodeBlock(CodeBlockKind::Indented) => {
            BlockKind::CodeBlock { fenced: false, lang: None }
        }
        Tag::BlockQuote(kind) => BlockKind::BlockQuote(kind.map(|k| match k {
            BlockQuoteKind::Note => QuoteKind::Note,
            BlockQuoteKind::Tip => QuoteKind::Tip,
            BlockQuoteKind::Important => QuoteKind::Important,
            BlockQuoteKind::Warning => QuoteKind::Warning,
            BlockQuoteKind::Caution => QuoteKind::Caution,
        })),
        Tag::List(start) => BlockKind::List { ordered: start.is_some() },
        Tag::Table(aligns) => BlockKind::Table(
            aligns
                .iter()
                .map(|a| match a {
                    pulldown_cmark::Alignment::None => Alignment::None,
                    pulldown_cmark::Alignment::Left => Alignment::Left,
                    pulldown_cmark::Alignment::Center => Alignment::Center,
                    pulldown_cmark::Alignment::Right => Alignment::Right,
                })
                .collect(),
        ),
        Tag::FootnoteDefinition(label) => BlockKind::Footnote(label.to_string()),
        // HTML blocks and anything unexpected render as raw source.
        _ => BlockKind::Html,
    }
}

/// Builds one top-level block's IR from its events.
struct Builder<'a> {
    src: &'a str,
    /// Absolute offset of the block's content start in `src`.
    origin: usize,
    /// End of the furthest non-container event: where the block's real
    /// content ends, as opposed to its (possibly longer) container range.
    extent_end: usize,
    /// Start of the event being processed.
    last_offset: usize,
    /// Leaf source ranges (absolute), grown by the events inside them.
    leaves: Vec<Range<usize>>,
    kind: BlockKind,
    defs: &'a DefTable,
    ir: BlockIr,
    refs: Vec<String>,
    footnotes: Vec<String>,
    styles: Vec<Style>,
    links: Vec<(usize, String)>,
    lists: Vec<Option<u64>>,
    quote: u8,
    line_open: bool,
    pending_marker: Option<Marker>,
    /// Kind of the line being built, for continuation lines.
    line_kind: LineKind,
    /// Inside a code or HTML block, a newline has been seen but the next line
    /// is opened only when more text follows (drops the trailing newline).
    deferred_newline: bool,
    in_verbatim_block: bool,
    table_cell: usize,
}

impl<'a> Builder<'a> {
    fn new(src: &'a str, origin: usize, kind: BlockKind, defs: &'a DefTable) -> Self {
        Builder {
            src,
            origin,
            extent_end: origin,
            last_offset: origin,
            leaves: Vec::new(),
            kind,
            defs,
            ir: BlockIr::default(),
            refs: Vec::new(),
            footnotes: Vec::new(),
            styles: Vec::new(),
            links: Vec::new(),
            lists: Vec::new(),
            quote: 0,
            line_open: false,
            pending_marker: None,
            line_kind: LineKind::Text,
            deferred_newline: false,
            in_verbatim_block: false,
            table_cell: 0,
        }
    }

    fn finish(mut self, range: Range<usize>) -> Pending {
        if self.pending_marker.is_some() {
            // Empty list item: still show its marker.
            self.begin_leaf();
            self.open_line(LineKind::Text);
        }
        Pending {
            kind: self.kind,
            content: range,
            origin: self.origin,
            extent_end: self.extent_end.max(self.origin),
            leaves: self.leaves,
            ir: self.ir,
            refs: self.refs,
            footnotes: self.footnotes,
        }
    }

    fn style(&self) -> Style {
        self.styles.iter().fold(Style::PLAIN, |acc, &s| acc | s)
    }

    fn begin_leaf(&mut self) {
        self.leaves.push(self.last_offset..self.last_offset);
    }

    /// Grows the current leaf over an event's source range. Only while a
    /// line is open: events between leaves (a task marker or `**` before a
    /// tight item's text) belong to the next leaf, not the previous one.
    fn touch(&mut self, range: Option<Range<usize>>) {
        if !self.line_open {
            return;
        }
        if let (Some(range), Some(leaf)) = (range, self.leaves.last_mut()) {
            leaf.start = leaf.start.min(range.start);
            leaf.end = leaf.end.max(range.end);
        }
    }

    fn open_line(&mut self, kind: LineKind) {
        if self.leaves.is_empty() {
            self.begin_leaf();
        }
        if !self.ir.text.is_empty() || !self.ir.lines.is_empty() {
            self.ir.text.push('\n');
        }
        self.ir.lines.push(LineInfo {
            start: self.ir.text.len(),
            kind,
            indent: self.lists.len() as u8,
            quote: self.quote,
            marker: self.pending_marker.take(),
            leaf: self.leaves.len() - 1,
        });
        self.line_kind = kind;
        self.line_open = true;
        self.deferred_newline = false;
        self.table_cell = 0;
    }

    fn ensure_line(&mut self) {
        if self.deferred_newline {
            let kind = self.line_kind;
            self.open_line(kind);
        } else if !self.line_open {
            // Inline content outside a paragraph: the text of a tight list
            // item, a leaf of its own.
            self.begin_leaf();
            self.open_line(LineKind::Text);
        }
    }

    /// Appends visible text taken from `source` (absolute range).
    fn push(&mut self, visible: &str, source: Range<usize>, extra: Style) {
        self.ensure_line();
        let start = self.ir.text.len();
        self.ir.text.push_str(visible);
        let end = self.ir.text.len();
        self.add_run(start..end, self.style() | extra);

        let raw = &self.src[source.clone()];
        let (source, verbatim) = if raw == visible {
            (source, true)
        } else if let Some(at) = raw.find(visible).filter(|_| !visible.is_empty()) {
            let s = source.start + at;
            (s..s + visible.len(), true)
        } else {
            (source, false)
        };
        let source = source.start - self.origin..source.end - self.origin;
        if let Some(last) = self.ir.map.last()
            && last.source.end > source.start
        {
            // Overlapping source (e.g. several events from one range): keep
            // the map monotonic by recording only the visible text.
            return;
        }
        self.ir.map.push(SourceSpan { visible: start..end, source, verbatim });
    }

    fn push_synthetic(&mut self, text: &str, extra: Style) {
        self.ensure_line();
        let start = self.ir.text.len();
        self.ir.text.push_str(text);
        let end = self.ir.text.len();
        self.add_run(start..end, self.style() | extra);
    }

    fn add_run(&mut self, range: Range<usize>, style: Style) {
        if style.is_plain() || range.is_empty() {
            return;
        }
        if let Some(last) = self.ir.runs.last_mut()
            && last.style == style
            && last.range.end == range.start
        {
            last.range.end = range.end;
            return;
        }
        self.ir.runs.push(StyleRun { range, style });
    }

    /// Verbatim block text (code, HTML): split into lines, dropping the final
    /// newline.
    fn push_verbatim(&mut self, text: &str, source: Range<usize>) {
        // Per-line source offsets are only known when the event text is the
        // source text; otherwise every line maps to the whole event range.
        let exact = self.src[source.clone()] == *text;
        let mut offset = source.start;
        for piece in text.split_inclusive('\n') {
            let (line, newline) = match piece.strip_suffix('\n') {
                Some(line) => (line, true),
                None => (piece, false),
            };
            if !line.is_empty() {
                let line_source = if exact { offset..offset + line.len() } else { source.clone() };
                self.push(line, line_source, Style::PLAIN);
            } else {
                self.ensure_line();
            }
            if newline {
                self.deferred_newline = true;
            }
            offset += piece.len();
        }
    }

    fn resolve(&mut self, link_type: LinkType, dest: &str, id: &str) -> String {
        match link_type {
            LinkType::Reference
            | LinkType::ReferenceUnknown
            | LinkType::Collapsed
            | LinkType::CollapsedUnknown
            | LinkType::Shortcut
            | LinkType::ShortcutUnknown
                if !id.is_empty() =>
            {
                self.refs.push(id.to_owned());
                self.defs.get(id).map_or_else(|| dest.to_owned(), |t| t.dest.clone())
            }
            _ => dest.to_owned(),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.begin_leaf();
                self.open_line(LineKind::Text);
            }
            Tag::Heading { level, .. } => {
                self.begin_leaf();
                self.open_line(LineKind::Heading(level as u8));
            }
            Tag::CodeBlock(_) => {
                self.begin_leaf();
                self.open_line(LineKind::Code);
                self.in_verbatim_block = true;
            }
            Tag::HtmlBlock => {
                self.begin_leaf();
                self.open_line(LineKind::Html);
                self.in_verbatim_block = true;
            }
            Tag::Table(_) => self.begin_leaf(),
            Tag::BlockQuote(_) => {
                self.quote += 1;
                self.line_open = false;
            }
            Tag::List(start) => {
                self.lists.push(start);
                self.line_open = false;
            }
            Tag::Item => {
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let marker = Marker::Ordered(*n);
                        *n += 1;
                        marker
                    }
                    _ => Marker::Bullet,
                };
                self.pending_marker = Some(marker);
                self.line_open = false;
            }
            Tag::TableHead => self.open_line(LineKind::TableRow { header: true }),
            Tag::TableRow => self.open_line(LineKind::TableRow { header: false }),
            Tag::TableCell => {
                if self.table_cell > 0 {
                    self.push_synthetic("\t", Style::PLAIN);
                }
                self.table_cell += 1;
            }
            Tag::Emphasis => self.styles.push(Style::EMPHASIS),
            Tag::Strong => self.styles.push(Style::STRONG),
            Tag::Strikethrough => self.styles.push(Style::STRIKETHROUGH),
            Tag::Link { link_type, dest_url, id, .. } => {
                let dest = self.resolve(link_type, &dest_url, &id);
                self.ensure_line();
                self.links.push((self.ir.text.len(), dest));
                self.styles.push(Style::LINK);
            }
            Tag::Image { link_type, dest_url, id, .. } => {
                let dest = self.resolve(link_type, &dest_url, &id);
                self.ensure_line();
                self.links.push((self.ir.text.len(), dest));
                self.styles.push(Style::IMAGE);
            }
            Tag::FootnoteDefinition(label) => {
                self.footnotes.push(label.to_string());
                self.line_open = false;
            }
            _ => {}
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::FootnoteDefinition => self.line_open = false,
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                self.in_verbatim_block = false;
                self.deferred_newline = false;
                self.line_open = false;
            }
            TagEnd::BlockQuote(_) => {
                self.quote = self.quote.saturating_sub(1);
                self.line_open = false;
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.line_open = false;
            }
            TagEnd::Item => {
                if self.pending_marker.is_some() {
                    // Empty item: its marker line is a leaf.
                    self.begin_leaf();
                    self.open_line(LineKind::Text);
                }
                self.line_open = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                self.styles.pop();
                if let Some((start, dest)) = self.links.pop() {
                    let end = self.ir.text.len();
                    self.ir.links.push(LinkSpan { visible: start..end, dest });
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Text(text) => {
                if self.in_verbatim_block {
                    self.push_verbatim(&text, range);
                } else {
                    self.push(&text, range, Style::PLAIN);
                }
            }
            Event::Code(code) => self.push(&code, range, Style::CODE),
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                self.push(&math, range, Style::MATH)
            }
            Event::Html(html) => {
                if self.in_verbatim_block {
                    self.push_verbatim(&html, range);
                } else {
                    self.push(&html, range, Style::HTML);
                }
            }
            Event::InlineHtml(html) => self.push(&html, range, Style::HTML),
            Event::FootnoteReference(label) => {
                self.push_synthetic(&format!("[{label}]"), Style::FOOTNOTE_REF)
            }
            Event::SoftBreak => self.push_synthetic(" ", Style::PLAIN),
            Event::HardBreak => {
                let kind = self.line_kind;
                self.open_line(kind);
            }
            Event::Rule => {
                self.begin_leaf();
                self.open_line(LineKind::Rule);
                self.line_open = false;
            }
            Event::TaskListMarker(checked) => {
                let task = Marker::Task { checked };
                if self.pending_marker.is_some() {
                    self.pending_marker = Some(task);
                } else if let Some(line) = self.ir.lines.last_mut()
                    && line.marker.is_some()
                {
                    line.marker = Some(task);
                }
            }
            Event::Start(_) | Event::End(_) => {}
        }
    }
}

#[cfg(test)]
mod tests;
