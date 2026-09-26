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

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use pulldown_cmark::{
    BlockQuoteKind, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag,
    TagEnd,
};

pub use crate::defs::{DefTable, LinkTarget, labels_match};
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
    /// Reference labels this block's rendering looked up in the [`DefTable`].
    pub refs: Vec<String>,
    /// Hash of the block's source text.
    pub source_hash: u64,
}

/// Splits `src` into blocks whose `len`s sum to `src.len()` and builds their
/// IR, resolving reference links through `defs`. Empty input yields no blocks.
pub fn parse(src: &str, defs: &DefTable) -> Vec<ParsedBlock> {
    if src.is_empty() {
        return Vec::new();
    }

    let lookups = std::cell::RefCell::new(Vec::<(usize, String)>::new());
    let callback = |link: BrokenLink<'_>| {
        lookups.borrow_mut().push((link.span.start, link.reference.to_string()));
        defs.get(&link.reference)
            .map(|t| (CowStr::from(t.dest.clone()), CowStr::from(t.title.clone())))
    };
    let mut events =
        Parser::new_with_broken_link_callback(src, options(), Some(callback)).into_offset_iter();

    let mut pending: Vec<Pending> = Vec::new();
    let mut depth = 0usize;
    let mut builder: Option<Builder<'_>> = None;
    for (event, range) in events.by_ref() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    builder = Some(Builder::new(src, range.start, kind_of(&tag), defs));
                }
                depth += 1;
                if let Some(b) = builder.as_mut() {
                    b.start(tag);
                }
            }
            Event::End(end) => {
                depth -= 1;
                if let Some(b) = builder.as_mut() {
                    b.end(end);
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
                b.event(event, range.clone());
                pending.push(b.finish(range));
            }
            event => {
                if let Some(b) = builder.as_mut() {
                    b.event(event, range);
                }
            }
        }
    }

    let mut defs_found: Vec<(Range<usize>, String, LinkTarget)> = events
        .reference_definitions()
        .iter()
        .map(|(label, def)| {
            (
                def.span.clone(),
                label.to_owned(),
                LinkTarget {
                    dest: def.dest.to_string(),
                    title: def.title.as_deref().unwrap_or_default().to_owned(),
                },
            )
        })
        .collect();
    drop(events);
    defs_found.sort_by_key(|(span, ..)| span.start);

    for (span, label, target) in defs_found {
        match pending
            .iter_mut()
            .find(|p| p.content.start <= span.start && span.end <= p.content.end)
        {
            Some(owner) => owner.defs.push((label, target)),
            None => pending.push(Pending::definition(src, span, label, target)),
        }
    }
    pending.sort_by_key(|p| p.content.start);

    for (at, label) in lookups.into_inner() {
        if let Some(owner) = pending.iter_mut().rev().find(|p| p.content.start <= at) {
            owner.refs.push(label);
        }
    }

    tile(src, pending)
}

/// A block before tiling: content range still absolute in the window.
struct Pending {
    kind: BlockKind,
    content: Range<usize>,
    ir: BlockIr,
    defs: Vec<(String, LinkTarget)>,
    refs: Vec<String>,
}

impl Pending {
    fn definition(src: &str, span: Range<usize>, label: String, target: LinkTarget) -> Self {
        let raw = src[span.clone()].trim_end_matches('\n');
        let ir = BlockIr {
            text: raw.to_owned(),
            lines: vec![LineInfo {
                start: 0,
                kind: LineKind::Text,
                indent: 0,
                quote: 0,
                marker: None,
            }],
            runs: Vec::new(),
            map: vec![SourceSpan { visible: 0..raw.len(), source: 0..raw.len(), verbatim: true }],
            links: vec![LinkSpan { visible: 0..raw.len(), dest: target.dest.clone() }],
        };
        Pending {
            kind: BlockKind::LinkDefinition,
            content: span,
            ir,
            defs: vec![(label, target)],
            refs: Vec::new(),
        }
    }
}

/// Assigns each block the source from its content start to the next block's
/// content start (the first block also takes leading whitespace), then makes
/// IR source ranges relative to the block start.
fn tile(src: &str, pending: Vec<Pending>) -> Vec<ParsedBlock> {
    if pending.is_empty() {
        return vec![ParsedBlock {
            kind: BlockKind::Blank,
            len: src.len(),
            content: 0..0,
            ir: BlockIr::default(),
            defs: Vec::new(),
            refs: Vec::new(),
            source_hash: hash(src),
        }];
    }
    let starts: Vec<usize> = pending
        .iter()
        .enumerate()
        .map(|(i, p)| if i == 0 { 0 } else { p.content.start })
        .chain(std::iter::once(src.len()))
        .collect();
    pending
        .into_iter()
        .enumerate()
        .map(|(i, mut p)| {
            let (start, end) = (starts[i], starts[i + 1]);
            // The block's IR was built with offsets relative to its content
            // start; shift them to be relative to the tiled block start.
            let shift = p.content.start - start;
            if shift > 0 {
                for span in &mut p.ir.map {
                    span.source = span.source.start + shift..span.source.end + shift;
                }
            }
            let content_end = p.content.end.min(end);
            p.refs.sort();
            p.refs.dedup();
            ParsedBlock {
                kind: p.kind,
                len: end - start,
                content: p.content.start - start..content_end - start,
                ir: p.ir,
                defs: p.defs,
                refs: p.refs,
                source_hash: hash(&src[start..end]),
            }
        })
        .collect()
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
    kind: BlockKind,
    defs: &'a DefTable,
    ir: BlockIr,
    refs: Vec<String>,
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
            kind,
            defs,
            ir: BlockIr::default(),
            refs: Vec::new(),
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
            self.open_line(LineKind::Text);
        }
        Pending { kind: self.kind, content: range, ir: self.ir, defs: Vec::new(), refs: self.refs }
    }

    fn style(&self) -> Style {
        self.styles.iter().fold(Style::PLAIN, |acc, &s| acc | s)
    }

    fn open_line(&mut self, kind: LineKind) {
        if !self.ir.text.is_empty() || !self.ir.lines.is_empty() {
            self.ir.text.push('\n');
        }
        self.ir.lines.push(LineInfo {
            start: self.ir.text.len(),
            kind,
            indent: self.lists.len() as u8,
            quote: self.quote,
            marker: self.pending_marker.take(),
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
            Tag::Paragraph => self.open_line(LineKind::Text),
            Tag::Heading { level, .. } => self.open_line(LineKind::Heading(level as u8)),
            Tag::CodeBlock(_) => {
                self.open_line(LineKind::Code);
                self.in_verbatim_block = true;
            }
            Tag::HtmlBlock => {
                self.open_line(LineKind::Html);
                self.in_verbatim_block = true;
            }
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
            Tag::FootnoteDefinition(_) => self.line_open = false,
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
