//! Find in the document (`Ctrl+F`). While the find bar is open, typed text, Backspace and paste
//! edit the query instead of the document; Enter / Shift+Enter (and F3 / Shift+F3) step through
//! the matches, Escape closes the bar. Matches are highlighted in rendered and raw blocks.

use std::ops::Range;

use gpui::{Context, Window};

use crate::editor::{Cancel, Editor, Find, FindNext, FindPrevious};

pub(crate) struct FindState {
    pub(crate) query: String,
    /// Matches of `query` in the document, in order.
    pub(crate) matches: Vec<Range<usize>>,
    /// Index into `matches` of the selected match.
    pub(crate) current: Option<usize>,
    /// Caret when the bar opened: typing the query selects the first match from here.
    origin: usize,
    /// Bytes at the end of `query` that are an uncommitted IME composition.
    composing: usize,
    /// Buffer version `matches` were computed for.
    version: u64,
}

impl FindState {
    /// Status shown in the find bar: `3/17`, `no matches`, or nothing for an empty query.
    pub(crate) fn status(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        let total = if self.matches.len() >= tachyon_doc::MAX_FIND_MATCHES {
            format!("{}+", self.matches.len())
        } else {
            self.matches.len().to_string()
        };
        match self.current {
            _ if self.matches.is_empty() => "no matches".to_owned(),
            Some(i) => format!("{}/{total}", i + 1),
            None => total,
        }
    }

    /// Matches overlapping `range`, with whether each is the current one.
    pub(crate) fn matches_in(
        &self,
        range: &Range<usize>,
    ) -> impl Iterator<Item = (Range<usize>, bool)> {
        let first = self.matches.partition_point(|m| m.end <= range.start);
        self.matches[first..]
            .iter()
            .enumerate()
            .take_while(move |(_, m)| m.start < range.end)
            .map(move |(i, m)| (m.clone(), self.current == Some(first + i)))
    }
}

impl Editor {
    pub(crate) fn find(&mut self, _: &Find, _: &mut Window, cx: &mut Context<Self>) {
        // A selection on one line becomes the query; otherwise the previous query stays.
        let selected = self.selected_text();
        let query = if !selected.is_empty() && !selected.contains('\n') {
            selected
        } else {
            self.find.as_ref().map(|f| f.query.clone()).unwrap_or_default()
        };
        let origin = self.selection.start;
        self.find = Some(FindState {
            query,
            matches: Vec::new(),
            current: None,
            origin,
            composing: 0,
            version: u64::MAX,
        });
        self.research(cx);
        cx.notify();
    }

    pub(crate) fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step_match(true, cx);
    }

    pub(crate) fn find_previous(
        &mut self,
        _: &FindPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_match(false, cx);
    }

    pub(crate) fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            cx.notify();
        }
    }

    /// Whether input goes to the find bar.
    pub(crate) fn finding(&self) -> bool {
        self.find.is_some()
    }

    /// Replaces the query's composition (if any) with `text`; `composing` marks it as a new
    /// uncommitted composition.
    pub(crate) fn find_input(&mut self, text: &str, composing: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        find.query.truncate(find.query.len() - find.composing);
        // The query is one line.
        let text = text.lines().next().unwrap_or("");
        find.query.push_str(text);
        find.composing = if composing { text.len() } else { 0 };
        self.research(cx);
        cx.notify();
    }

    pub(crate) fn find_backspace(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        find.composing = 0;
        find.query.pop();
        self.research(cx);
        cx.notify();
    }

    pub(crate) fn find_end_composition(&mut self) {
        if let Some(find) = &mut self.find {
            find.composing = 0;
        }
    }

    /// Searches again for the query and selects the first match from where the bar opened.
    fn research(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else { return };
        find.matches = self.doc.find_all(&find.query);
        find.version = self.doc.buffer().version();
        let origin = find.origin;
        let first = find.matches.iter().position(|m| m.start >= origin).or(
            // Wrap around.
            (!find.matches.is_empty()).then_some(0),
        );
        find.current = first;
        if let Some(m) = first.map(|i| find.matches[i].clone()) {
            self.select_range(m, cx);
        }
    }

    /// Keeps matches current after the document changed.
    pub(crate) fn refresh_find(&mut self) {
        let Some(find) = &mut self.find else { return };
        if find.version == self.doc.buffer().version() {
            return;
        }
        find.matches = self.doc.find_all(&find.query);
        find.version = self.doc.buffer().version();
        find.current = find.matches.iter().position(|m| *m == self.selection);
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.refresh_find();
        let Some(find) = &mut self.find else { return };
        if find.matches.is_empty() {
            return;
        }
        let n = find.matches.len();
        let next = if forward {
            find.matches.iter().position(|m| m.start >= self.selection.end).unwrap_or(0)
        } else {
            find.matches.iter().rposition(|m| m.end <= self.selection.start).unwrap_or(n - 1)
        };
        find.current = Some(next);
        find.origin = find.matches[next].start;
        let range = find.matches[next].clone();
        self.select_range(range, cx);
    }

    fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.move_to(range.start, false, cx);
        self.move_to(range.end, true, cx);
    }
}
