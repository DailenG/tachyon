//! Find and replace (`Ctrl+F`, `Ctrl+H`). While the bar is open, typed text, Backspace and paste
//! edit its active field (query or replacement; Tab switches) instead of the document. In the
//! query, Enter / Shift+Enter (and F3 / Shift+F3) step through the matches; in the replacement,
//! Enter replaces the selected match and moves to the next, Ctrl+Enter replaces all of them as one
//! undo step. Escape closes the bar. Matches are highlighted in rendered and raw blocks.

use std::ops::Range;

use gpui::{Context, Window};

use crate::editor::{Cancel, Editor, Find, FindNext, FindPrevious, Replace, ReplaceAll};

pub(crate) struct FindState {
    pub(crate) query: String,
    /// The replacement, when the bar is in replace mode.
    pub(crate) replacement: Option<String>,
    /// Whether typing goes to the replacement (Tab switches).
    pub(crate) editing_replacement: bool,
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
    fn active_field(&mut self) -> &mut String {
        match &mut self.replacement {
            Some(replacement) if self.editing_replacement => replacement,
            _ => &mut self.query,
        }
    }

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
        self.open_find(false, cx);
    }

    pub(crate) fn replace_bar(&mut self, _: &Replace, _: &mut Window, cx: &mut Context<Self>) {
        self.open_find(true, cx);
    }

    fn open_find(&mut self, replace: bool, cx: &mut Context<Self>) {
        // Keyboard shortcuts for this always have a modifier, so `lib::init`'s intercept
        // already flushes a pending paste first; flush here too so opening the bar some other
        // way (not a keystroke) still sees the query and selection after the paste, not before.
        self.flush_pending_paste(cx);
        // A selection on one line becomes the query; otherwise the previous query stays.
        let selected = self.selected_text();
        let query = if !selected.is_empty() && !selected.contains('\n') {
            selected
        } else {
            self.find.as_ref().map(|f| f.query.clone()).unwrap_or_default()
        };
        self.picker = None;
        let origin = self.selection.start;
        let previous = self.find.as_ref().and_then(|f| f.replacement.clone());
        let replacement = replace.then(|| previous.unwrap_or_default());
        self.find = Some(FindState {
            query,
            replacement,
            editing_replacement: false,
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
        if self.find.take().is_some() | self.picker.take().is_some() {
            cx.notify();
        }
    }

    /// Whether input goes to the find bar.
    pub(crate) fn bar_open(&self) -> bool {
        self.find.is_some() || self.picker.is_some()
    }

    /// Replaces the active field's composition (if any) with `text`; `composing` marks it as a
    /// new uncommitted composition.
    pub(crate) fn find_input(&mut self, text: &str, composing: bool, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return self.picker_input(text, composing, cx);
        }
        let Some(find) = &mut self.find else { return };
        let editing_query = !find.editing_replacement;
        let composed = find.composing;
        let field = find.active_field();
        field.truncate(field.len() - composed);
        // Both fields are one line.
        let text = text.lines().next().unwrap_or("");
        field.push_str(text);
        find.composing = if composing { text.len() } else { 0 };
        if editing_query {
            self.research(cx);
        }
        cx.notify();
    }

    pub(crate) fn find_backspace(&mut self, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return self.picker_backspace(cx);
        }
        let Some(find) = &mut self.find else { return };
        find.composing = 0;
        let editing_query = !find.editing_replacement;
        find.active_field().pop();
        if editing_query {
            self.research(cx);
        }
        cx.notify();
    }

    /// Tab while the bar is in replace mode: switches between query and replacement.
    pub(crate) fn find_switch_field(&mut self, cx: &mut Context<Self>) -> bool {
        if self.picker.is_some() {
            // Tab does nothing in a picker, and must not reach the document.
            return true;
        }
        let Some(find) = &mut self.find else { return false };
        if find.replacement.is_none() {
            return false;
        }
        find.editing_replacement = !find.editing_replacement;
        find.composing = 0;
        cx.notify();
        true
    }

    /// Enter in the replacement: replaces the selected match, then selects the next one.
    pub(crate) fn find_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return self.picker_pick(None, cx);
        }
        let replacing = self.find.as_ref().is_some_and(|f| f.editing_replacement);
        if replacing {
            self.replace_current(cx);
        }
        self.find_next(&FindNext, window, cx);
    }

    pub(crate) fn replace_all(&mut self, _: &ReplaceAll, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh_find();
        let Some(find) = &self.find else { return };
        let Some(replacement) = find.replacement.clone() else { return };
        let (Some(first), Some(last)) = (find.matches.first(), find.matches.last()) else {
            return;
        };
        // One edit over the span of all matches: one undo step, one reparse.
        let span = first.start..last.end;
        let text = self.doc.buffer().rope().byte_slice(span.clone()).to_string();
        let mut replaced = String::with_capacity(text.len());
        let mut at = span.start;
        for m in &find.matches {
            replaced.push_str(&text[at - span.start..m.start - span.start]);
            replaced.push_str(&replacement);
            at = m.end;
        }
        let start = span.start;
        self.replace_selection_with(span, &replaced, cx);
        self.move_to(start, false, cx);
    }

    /// Replaces the selection if it is exactly a match.
    fn replace_current(&mut self, cx: &mut Context<Self>) {
        self.refresh_find();
        let Some(find) = &self.find else { return };
        let Some(replacement) = find.replacement.clone() else { return };
        if !find.matches.contains(&self.selection) {
            return;
        }
        let selection = self.selection.clone();
        self.replace_selection_with(selection, &replacement, cx);
    }

    pub(crate) fn find_end_composition(&mut self) {
        self.picker_end_composition();
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
