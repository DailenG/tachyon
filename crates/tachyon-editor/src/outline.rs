//! Jump to a heading (`Ctrl+Shift+O`): a bar listing the document's headings. Typing filters them
//! (case-insensitive substring), Up / Down choose, Enter or a click jumps, Escape closes. Input is
//! routed here through the find bar's entry points while the list is open.

use gpui::{Context, Window};
use tachyon_md::BlockKind;

use crate::editor::{Editor, GoToHeading};

/// Rows shown at once; the list scrolls to keep the chosen one in view.
pub(crate) const VISIBLE_ROWS: usize = 12;

pub(crate) struct Heading {
    /// Source offset of the heading's text.
    pub(crate) offset: usize,
    pub(crate) level: u8,
    pub(crate) title: String,
}

pub(crate) struct Outline {
    pub(crate) query: String,
    /// Bytes at the end of `query` that are an uncommitted IME composition.
    composing: usize,
    pub(crate) headings: Vec<Heading>,
    /// Indices into `headings` that match the query, in document order.
    pub(crate) matches: Vec<usize>,
    /// Index into `matches` of the chosen heading.
    pub(crate) selected: usize,
}

impl Outline {
    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = (0..self.headings.len())
            .filter(|&i| query.is_empty() || self.headings[i].title.to_lowercase().contains(&query))
            .collect();
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    /// The rows in view: indices into `matches`, keeping `selected` visible.
    pub(crate) fn window(&self) -> std::ops::Range<usize> {
        let first = self.selected.saturating_sub(VISIBLE_ROWS - 1);
        first..(first + VISIBLE_ROWS).min(self.matches.len())
    }
}

impl Editor {
    pub(crate) fn go_to_heading(
        &mut self,
        _: &GoToHeading,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find = None;
        let caret = self.selection.start;
        let mut headings = Vec::new();
        for (index, block) in self.doc.blocks().iter().enumerate() {
            let BlockKind::Heading(level) = block.parsed().kind else { continue };
            let ir = &block.parsed().ir;
            let start = self.doc.block_range(index).start;
            let title = ir.text.lines().next().unwrap_or("").trim().to_owned();
            headings.push(Heading { offset: start + ir.visible_to_source(0), level, title });
        }
        // Start at the heading the caret is under.
        let selected = headings.iter().rposition(|h| h.offset <= caret).unwrap_or(0);
        let mut outline =
            Outline { query: String::new(), composing: 0, headings, matches: Vec::new(), selected };
        outline.filter();
        self.outline = Some(outline);
        cx.notify();
    }

    pub(crate) fn outline_input(&mut self, text: &str, composing: bool, cx: &mut Context<Self>) {
        let Some(outline) = &mut self.outline else { return };
        outline.query.truncate(outline.query.len() - outline.composing);
        let text = text.lines().next().unwrap_or("");
        outline.query.push_str(text);
        outline.composing = if composing { text.len() } else { 0 };
        outline.selected = 0;
        outline.filter();
        cx.notify();
    }

    pub(crate) fn outline_backspace(&mut self, cx: &mut Context<Self>) {
        let Some(outline) = &mut self.outline else { return };
        outline.composing = 0;
        outline.query.pop();
        outline.filter();
        cx.notify();
    }

    pub(crate) fn outline_end_composition(&mut self) {
        if let Some(outline) = &mut self.outline {
            outline.composing = 0;
        }
    }

    /// Up / Down while the list is open: moves the choice, wrapping. Returns whether it did.
    pub(crate) fn outline_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(outline) = &mut self.outline else { return false };
        let n = outline.matches.len();
        if n > 0 {
            outline.selected = (outline.selected as isize + delta).rem_euclid(n as isize) as usize;
            cx.notify();
        }
        true
    }

    /// Jumps to the chosen heading (or match `row`) and closes the list.
    pub(crate) fn outline_jump(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        let Some(outline) = self.outline.take() else { return };
        let row = row.unwrap_or(outline.selected);
        if let Some(heading) = outline.matches.get(row).map(|&i| &outline.headings[i]) {
            self.move_to(heading.offset, false, cx);
        }
        cx.notify();
    }
}
