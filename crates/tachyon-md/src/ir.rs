//! Owned, render-ready representation of one block. Every offset is relative
//! to the block: `text` offsets for visible ranges, block-source offsets for
//! source ranges. Nothing here borrows the document, so blocks survive edits
//! elsewhere unchanged.

use std::ops::{BitOr, BitOrAssign, Range};

/// Inline style flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style(u16);

impl Style {
    pub const PLAIN: Style = Style(0);
    pub const STRONG: Style = Style(1 << 0);
    pub const EMPHASIS: Style = Style(1 << 1);
    pub const STRIKETHROUGH: Style = Style(1 << 2);
    pub const CODE: Style = Style(1 << 3);
    pub const LINK: Style = Style(1 << 4);
    pub const IMAGE: Style = Style(1 << 5);
    pub const MATH: Style = Style(1 << 6);
    pub const HTML: Style = Style(1 << 7);
    pub const FOOTNOTE_REF: Style = Style(1 << 8);

    pub fn contains(self, other: Style) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_plain(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for Style {
    type Output = Style;
    fn bitor(self, rhs: Style) -> Style {
        Style(self.0 | rhs.0)
    }
}

impl BitOrAssign for Style {
    fn bitor_assign(&mut self, rhs: Style) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleRun {
    pub range: Range<usize>,
    pub style: Style,
}

/// List item marker shown at the start of an item's first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Bullet,
    Ordered(u64),
    Task { checked: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Text,
    Heading(u8),
    Code,
    Html,
    TableRow { header: bool },
    Rule,
}

/// One visible line. Lines are separated by `\n` in [`BlockIr::text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineInfo {
    /// Offset of the line's first byte in [`BlockIr::text`].
    pub start: usize,
    pub kind: LineKind,
    /// List nesting depth.
    pub indent: u8,
    /// Block quote nesting depth.
    pub quote: u8,
    pub marker: Option<Marker>,
    /// Index into [`BlockIr::leaves`] of the leaf block this line belongs to.
    pub leaf: usize,
}

/// Correspondence between visible text and block source. Visible text not
/// covered by any span is synthetic (soft breaks, table cell separators).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub visible: Range<usize>,
    pub source: Range<usize>,
    /// Visible and source text are byte-identical, so offsets inside the span
    /// map one-to-one. Otherwise (escapes, entities) only the span ends map.
    pub verbatim: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub visible: Range<usize>,
    pub dest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockIr {
    pub text: String,
    pub lines: Vec<LineInfo>,
    /// Non-plain runs, sorted and non-overlapping.
    pub runs: Vec<StyleRun>,
    /// Sorted by visible offset, non-overlapping.
    pub map: Vec<SourceSpan>,
    pub links: Vec<LinkSpan>,
    /// Source ranges (block-relative, whole lines, including list and quote
    /// markers) of the leaf blocks: paragraphs, headings, code blocks,
    /// tables, rules, list item text. Inside lists and quotes the editor
    /// swaps a single leaf to raw Markdown instead of the whole block.
    pub leaves: Vec<Range<usize>>,
}

impl BlockIr {
    /// Maps a visible offset to a block-source offset for cursor placement
    /// when the block switches to raw mode. Offsets in synthetic text snap to
    /// the nearest preceding mapped position.
    pub fn visible_to_source(&self, visible: usize) -> usize {
        let idx = self.map.partition_point(|s| s.visible.start <= visible);
        let Some(span) = idx.checked_sub(1).map(|i| &self.map[i]) else {
            return self.map.first().map_or(0, |s| s.source.start);
        };
        if visible >= span.visible.end {
            span.source.end
        } else if span.verbatim {
            span.source.start + (visible - span.visible.start)
        } else {
            span.source.start
        }
    }

    /// Maps a block-source offset to a visible offset (for placing the caret
    /// in rendered mode). Offsets inside hidden syntax snap to the next
    /// visible position.
    pub fn source_to_visible(&self, source: usize) -> usize {
        for span in &self.map {
            if source < span.source.start {
                return span.visible.start;
            }
            if source < span.source.end {
                return if span.verbatim {
                    span.visible.start + (source - span.source.start)
                } else {
                    span.visible.start
                };
            }
        }
        self.text.len()
    }
}
