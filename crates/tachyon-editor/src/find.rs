//! Find and replace (`Ctrl+F`, `Ctrl+H`). While the bar is open, typed text, Backspace and paste
//! edit its active field (query or replacement; Tab switches) instead of the document. In the
//! query, Enter / Shift+Enter (and F3 / Shift+F3) step through the matches; in the replacement,
//! Enter replaces the selected match and moves to the next, Ctrl+Enter replaces all of them as one
//! undo step. Escape closes the bar. Matches are highlighted in rendered and raw blocks.
//!
//! The scan itself (`Document::find_all`) is `O(document size)`. Below
//! [`tachyon_doc::FIND_BACKGROUND_THRESHOLD`] it runs inline (`dispatch_search`); at or above it,
//! it runs on the background executor instead, so a keystroke in the query field of a huge
//! document's find bar never costs a dropped frame. `FindState::searching` is `true` meanwhile
//! (`status` shows "searching…"); `FindState::generation` is bumped by every dispatch, so a
//! result overtaken by a newer one (another keystroke, or a document edit) before it lands is
//! dropped instead of clobbering fresher matches (`apply_search_result`). Callers that would act
//! on `matches` while stale - `step_match`, `replace_current` - queue in `FindState::pending`
//! instead and run once fresh matches land.
//!
//! Ctrl+Enter (`replace_all`/`dispatch_replace_all`) never reads `find.matches`: that list is
//! capped at `MAX_FIND_MATCHES` for the display count and highlighting, and Replace All must
//! replace every match past that cap regardless (a 200 MB log with ~530k matches of
//! `status=200` silently replacing only the first 10,000, with no notice, was the bug this
//! exists to fix). It runs its own uncapped scan instead
//! (`tachyon_doc::replace_all_in_rope`/`Document::replace_all`, off the UI thread above the same
//! `FIND_BACKGROUND_THRESHOLD`), which also builds the replaced text for the span from the first
//! match to the last in the same pass, so applying it is one `Document::edit` - one undo step,
//! never one edit per match. `FindState::replacing` shows "replacing…" meanwhile;
//! `FindState::replaced` shows "Replaced <count>" once it lands, until the query changes
//! (`dispatch_search` clears it).
//!
//! An edit that arrives while a background Replace All is in flight is never queued or lost: it
//! applies immediately, in place, like any other edit would. Instead, when the scan lands,
//! `dispatch_replace_all` checks the buffer's version against the one it captured before
//! dispatching; if the version moved, the scan (and the replacement text built from it) is stale,
//! computed against text an edit already changed underneath it, and is discarded and rerun
//! from scratch against the now-current text, rather than applied or queued. That keeps the
//! typed edit's own place in the document (never reordered behind a stale replacement), at the
//! cost of redoing the scan; it was chosen over holding input (like the pending paste queue
//! does) because Replace All has no small, bounded "flush the pending thing now" fallback the
//! way a paste's clipboard read does - its own work *is* the `O(document size)` scan, so blocking
//! on it on the rare edit-mid-scan would defeat the point of running it in the background.

use std::ops::Range;

use gpui::{Context, Window};

use crate::editor::{Cancel, Editor, Find, FindNext, FindPrevious, Replace, ReplaceAll};

pub(crate) struct FindState {
    pub(crate) query: String,
    /// The replacement, when the bar is in replace mode.
    pub(crate) replacement: Option<String>,
    /// Whether typing goes to the replacement (Tab switches).
    pub(crate) editing_replacement: bool,
    /// Matches of `query` in the document, in order. Valid for `version`; while `searching` is
    /// `true` these are the previous, possibly stale, matches (kept so highlighting does not
    /// flash empty during a background scan).
    pub(crate) matches: Vec<Range<usize>>,
    /// Index into `matches` of the selected match.
    pub(crate) current: Option<usize>,
    /// Caret when the bar opened: typing the query selects the first match from here.
    origin: usize,
    /// Bytes at the end of `query` that are an uncommitted IME composition.
    composing: usize,
    /// Buffer version the most recently *dispatched* search is for (set when dispatched, not
    /// when it lands): compared against the live buffer version to decide whether a fresh search
    /// is needed at all (`Editor::refresh_find`).
    version: u64,
    /// Bumped on every dispatch; a background result carries the generation it was dispatched
    /// with, and is dropped if that no longer matches by the time it lands (`apply_search_result`).
    generation: u64,
    /// A background search is in flight for the current `generation`: `status` shows
    /// "searching…", and `step_match`/`replace_current` queue into `pending` instead of acting
    /// on the stale `matches` they would otherwise see. Independent of Replace All's own
    /// `replacing`/`replaced` below, which never read `matches` (see the module doc comment).
    pub(crate) searching: bool,
    /// An action deferred because it was requested while `searching`; run once the search lands
    /// (`Editor::apply_search_result`).
    pending: Option<PendingFindAction>,
    /// A background Replace All scan (`Editor::dispatch_replace_all`) is in flight: `status`
    /// shows "replacing…".
    pub(crate) replacing: bool,
    /// The match count Replace All last replaced, shown as "Replaced <count>" until the query
    /// changes (`dispatch_search` clears it back to `None`).
    pub(crate) replaced: Option<usize>,
}

/// An action that needs matches for the *current* text and so cannot run against a stale scan
/// still in flight; see the module doc comment.
enum PendingFindAction {
    StepMatch {
        forward: bool,
    },
    /// Enter in the replacement field: replace the selected match, then select the next one.
    ReplaceCurrentThenStep,
}

impl FindState {
    fn active_field(&mut self) -> &mut String {
        match &mut self.replacement {
            Some(replacement) if self.editing_replacement => replacement,
            _ => &mut self.query,
        }
    }

    /// Status shown in the find bar: `3/17`, `no matches`, `searching…` while a background scan
    /// of a huge document is still running, `replacing…` while a background Replace All is
    /// running, `Replaced 530,012` once one lands (kept until the query changes), or nothing for
    /// an empty query.
    pub(crate) fn status(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        if self.replacing {
            return "replacing…".to_owned();
        }
        if let Some(count) = self.replaced {
            return format!("Replaced {}", format_count(count));
        }
        if self.searching {
            return "searching…".to_owned();
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

/// `530012` -> `"530,012"`: thousands separators for `FindState::status`'s "Replaced <count>".
fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The replaced text a Replace All result carries: built on the UI thread for a small document
/// (`dispatch_replace_all`'s inline path), or off it as a [`tachyon_doc::PreparedInsert`] for a
/// large one, so applying it (`Editor::replace_selection_with_prepared`) never has to build a
/// new rope from a `&str` on the UI thread.
enum ReplaceAllText {
    Plain(String),
    Prepared(tachyon_doc::PreparedInsert),
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
            generation: 0,
            searching: false,
            pending: None,
            replacing: false,
            replaced: None,
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

    /// Enter in the replacement: replaces the selected match, then selects the next one. If a
    /// background search is needed first, both steps run together once it lands
    /// (`PendingFindAction::ReplaceCurrentThenStep`) instead of `find_next` running immediately
    /// against matches `replace_current` has not caught up to yet.
    pub(crate) fn find_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return self.picker_pick(None, cx);
        }
        let replacing = self.find.as_ref().is_some_and(|f| f.editing_replacement);
        if replacing && !self.replace_current(cx) {
            return; // deferred: queued as `ReplaceCurrentThenStep`
        }
        self.find_next(&FindNext, window, cx);
    }

    pub(crate) fn replace_all(&mut self, _: &ReplaceAll, _: &mut Window, cx: &mut Context<Self>) {
        let Some(find) = &self.find else { return };
        if find.replacing {
            return; // already running; Ctrl+Enter again while it is in flight is a no-op
        }
        let Some(replacement) = find.replacement.clone() else { return };
        let query = find.query.clone();
        self.dispatch_replace_all(query, replacement, cx);
    }

    /// Runs Replace All's own uncapped scan (`tachyon_doc::replace_all_in_rope`): inline below
    /// [`tachyon_doc::FIND_BACKGROUND_THRESHOLD`] like `dispatch_search`, else on the background
    /// executor from a cloned rope snapshot, which also builds a `PreparedInsert` from the
    /// replaced text there (`ReplaceAllText::Prepared`): splicing that in on the UI thread is
    /// O(log n) even for megabytes, instead of building a new rope from a `&str` there (measured
    /// at roughly half the UI-thread cost on a 200 MB log). See the module doc comment for why a
    /// stale result (the buffer's version moved since `version` was captured here) is discarded
    /// and rerun rather than applied or queued.
    fn dispatch_replace_all(&mut self, query: String, replacement: String, cx: &mut Context<Self>) {
        let version = self.doc.buffer().version();
        if (self.doc.len() as u64) < tachyon_doc::FIND_BACKGROUND_THRESHOLD {
            let result = self
                .doc
                .replace_all(&query, &replacement)
                .map(|(span, text, count)| (span, ReplaceAllText::Plain(text), count));
            return self.finish_replace_all(result, cx);
        }
        if let Some(find) = &mut self.find {
            find.replacing = true;
            find.replaced = None;
        }
        cx.notify();
        let rope = self.doc.buffer().rope().clone();
        let scan_query = query.clone();
        let scan_replacement = replacement.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (span, text, count) =
                        tachyon_doc::replace_all_in_rope(&rope, &scan_query, &scan_replacement)?;
                    let prepared = tachyon_doc::PreparedInsert::new(&text);
                    Some((span, ReplaceAllText::Prepared(prepared), count))
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if editor.doc.buffer().version() != version {
                    return editor.dispatch_replace_all(query, replacement, cx);
                }
                editor.finish_replace_all(result, cx);
            });
        })
        .detach();
    }

    /// Applies a Replace All result as one edit over the whole span (one undo step), or, if
    /// `query` had no matches, just clears `replacing`. Shows "Replaced <count>" until the query
    /// changes (`dispatch_search` clears it).
    fn finish_replace_all(
        &mut self,
        result: Option<(Range<usize>, ReplaceAllText, usize)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(find) = &mut self.find {
            find.replacing = false;
        }
        let Some((span, text, count)) = result else {
            cx.notify();
            return;
        };
        let start = span.start;
        match text {
            ReplaceAllText::Plain(text) => self.replace_selection_with(span, &text, cx),
            ReplaceAllText::Prepared(insert) => {
                self.replace_selection_with_prepared(span, insert, cx);
            }
        }
        self.move_to(start, false, cx);
        if let Some(find) = &mut self.find {
            find.replaced = Some(count);
        }
        cx.notify();
    }

    /// Replaces the selection if it is exactly a match. Returns whether it ran now: `false` means
    /// a background search was needed and the replacement was queued
    /// (`PendingFindAction::ReplaceCurrentThenStep`) instead - the caller (`find_enter`) must not
    /// chain `find_next` itself in that case.
    fn replace_current(&mut self, cx: &mut Context<Self>) -> bool {
        if self.refresh_find(cx) {
            if let Some(find) = &mut self.find {
                find.pending = Some(PendingFindAction::ReplaceCurrentThenStep);
            }
            return false;
        }
        self.replace_current_now(cx);
        true
    }

    fn replace_current_now(&mut self, cx: &mut Context<Self>) {
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

    /// Searches again for the query and selects the first match from where the bar opened: on
    /// opening the bar, and on every query keystroke.
    fn research(&mut self, cx: &mut Context<Self>) {
        self.dispatch_search(true, cx);
    }

    /// Keeps `find.matches` current for the buffer's present version. A no-op if they already
    /// are, including if a background search is already running for this version (dispatching
    /// again would only discard it via `dispatch_search`'s generation bump). Returns whether a
    /// background search is (now, or still) running: `true` means the caller must not act on
    /// `matches` yet - queue in `find.pending` instead.
    pub(crate) fn refresh_find(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(find) = &self.find else { return false };
        if find.version != self.doc.buffer().version() {
            self.dispatch_search(false, cx);
        }
        self.find.as_ref().is_some_and(|f| f.searching)
    }

    /// Dispatches a fresh search for `find.query`: inline below
    /// [`tachyon_doc::FIND_BACKGROUND_THRESHOLD`], else on the background executor from a cloned
    /// rope snapshot (`Buffer::rope().clone()` is O(1)). See the module doc comment for the
    /// generation/pending scheme that keeps a slow scan from clobbering fresher results.
    ///
    /// `select_first`: once matches land, select the first one at or after `find.origin`
    /// (opening the bar, or the query changed) rather than just keeping `find.current` pointed at
    /// whichever match, if any, still equals the selection (the document changed under an
    /// unmoved caret).
    fn dispatch_search(&mut self, select_first: bool, cx: &mut Context<Self>) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()) else { return };
        let version = self.doc.buffer().version();
        let background = self.doc.len() as u64 >= tachyon_doc::FIND_BACKGROUND_THRESHOLD;
        let Some(find) = self.find.as_mut() else { return };
        find.generation = find.generation.wrapping_add(1);
        find.version = version;
        find.searching = background;
        find.replaced = None;
        let generation = find.generation;
        if !background {
            let matches = self.doc.find_all(&query);
            self.apply_search_result(matches, generation, select_first, cx);
            return;
        }
        cx.notify();
        let rope = self.doc.buffer().rope().clone();
        cx.spawn(async move |this, cx| {
            let matches = cx
                .background_executor()
                .spawn(async move { tachyon_doc::find_all_in_rope(&rope, &query) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.apply_search_result(matches, generation, select_first, cx);
            });
        })
        .detach();
    }

    /// Installs a search result if it is still wanted (`generation` matches: see the module doc
    /// comment), then runs whatever queued in `find.pending` while it was in flight.
    fn apply_search_result(
        &mut self,
        matches: Vec<Range<usize>>,
        generation: u64,
        select_first: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(find) = &mut self.find else { return };
        if find.generation != generation {
            return; // superseded by a newer search; these matches are stale
        }
        find.matches = matches;
        find.searching = false;
        let pending = find.pending.take();
        let selected = if select_first {
            let origin = find.origin;
            let first = find
                .matches
                .iter()
                .position(|m| m.start >= origin)
                .or((!find.matches.is_empty()).then_some(0));
            find.current = first;
            first.map(|i| find.matches[i].clone())
        } else {
            find.current = find.matches.iter().position(|m| *m == self.selection);
            None
        };
        // `find`'s borrow ends above (its last use): free to call back into `self` below.
        match selected {
            Some(range) => self.select_range(range, cx),
            None => cx.notify(),
        }
        match pending {
            Some(PendingFindAction::StepMatch { forward }) => self.step_match_now(forward, cx),
            Some(PendingFindAction::ReplaceCurrentThenStep) => {
                self.replace_current_now(cx);
                self.step_match_now(true, cx);
            }
            None => {}
        }
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.refresh_find(cx) {
            if let Some(find) = &mut self.find {
                find.pending = Some(PendingFindAction::StepMatch { forward });
            }
            return;
        }
        self.step_match_now(forward, cx);
    }

    fn step_match_now(&mut self, forward: bool, cx: &mut Context<Self>) {
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
