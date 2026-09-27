use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable, Font,
    FontStyle, FontWeight, KeyBinding, ListAlignment, ListOffset, ListState, Pixels, Point,
    PromptLevel, Task, TextLayout, TextRun, UTF16Selection, Window, actions, point, px,
};
use tachyon_doc::{BlockId, Document, PreparedInsert, Splice};
use tachyon_md::{BlockKind, ParsedBlock};

use crate::movement;
use crate::theme::Theme;

actions!(
    editor,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        Home,
        End,
        DocumentStart,
        DocumentEnd,
        PageUp,
        PageDown,
        ShiftNewline,
        Find,
        Replace,
        ReplaceAll,
        FindNext,
        FindPrevious,
        Cancel,
        SelectPageUp,
        SelectPageDown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectHome,
        SelectEnd,
        SelectAll,
        Newline,
        Tab,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Save,
        SaveAs,
        ToggleFrameStats,
        CloseWindow,
    ]
);

/// Key context the bindings below apply in.
pub const KEY_CONTEXT: &str = "Editor";

/// Default key bindings. `secondary` is Cmd on macOS and Ctrl elsewhere.
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, c),
        KeyBinding::new("alt-backspace", DeleteWordLeft, c),
        KeyBinding::new("ctrl-delete", DeleteWordRight, c),
        KeyBinding::new("alt-delete", DeleteWordRight, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("ctrl-left", WordLeft, c),
        KeyBinding::new("alt-left", WordLeft, c),
        KeyBinding::new("ctrl-right", WordRight, c),
        KeyBinding::new("alt-right", WordRight, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("cmd-left", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("cmd-right", End, c),
        KeyBinding::new("ctrl-home", DocumentStart, c),
        KeyBinding::new("cmd-up", DocumentStart, c),
        KeyBinding::new("ctrl-end", DocumentEnd, c),
        KeyBinding::new("secondary-f", Find, c),
        KeyBinding::new("ctrl-h", Replace, c),
        KeyBinding::new("cmd-alt-f", Replace, c),
        KeyBinding::new("secondary-enter", ReplaceAll, c),
        KeyBinding::new("f3", FindNext, c),
        KeyBinding::new("shift-f3", FindPrevious, c),
        KeyBinding::new("secondary-g", FindNext, c),
        KeyBinding::new("secondary-shift-g", FindPrevious, c),
        KeyBinding::new("escape", Cancel, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("shift-pageup", SelectPageUp, c),
        KeyBinding::new("shift-pagedown", SelectPageDown, c),
        KeyBinding::new("cmd-down", DocumentEnd, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("alt-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("alt-shift-right", SelectWordRight, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("secondary-a", SelectAll, c),
        KeyBinding::new("enter", Newline, c),
        KeyBinding::new("shift-enter", ShiftNewline, c),
        KeyBinding::new("tab", Tab, c),
        KeyBinding::new("secondary-c", Copy, c),
        KeyBinding::new("secondary-x", Cut, c),
        KeyBinding::new("secondary-v", Paste, c),
        KeyBinding::new("secondary-z", Undo, c),
        KeyBinding::new("secondary-shift-z", Redo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("secondary-s", Save, c),
        KeyBinding::new("secondary-shift-s", SaveAs, c),
        KeyBinding::new("ctrl-alt-f", ToggleFrameStats, c),
        KeyBinding::new("secondary-w", CloseWindow, c),
    ]
}

/// Typing pauses longer than this start a new undo group.
const UNDO_GROUP_PAUSE: Duration = Duration::from_millis(1000);

/// Maps an index into a laid-out piece of text to a document offset.
#[derive(Clone)]
pub(crate) enum TextTarget {
    /// Raw block source: `base + index`.
    Raw { base: usize },
    /// Rendered text: the index is a visible offset from `visible_base` in
    /// the block's IR, mapped back to source through its source map.
    Rendered { block_start: usize, visible_base: usize, parsed: Arc<ParsedBlock> },
}

impl TextTarget {
    pub(crate) fn offset(&self, index: usize) -> usize {
        match self {
            TextTarget::Raw { base } => base + index,
            TextTarget::Rendered { block_start, visible_base, parsed } => {
                block_start + parsed.ir.visible_to_source(visible_base + index)
            }
        }
    }
}

pub struct Editor {
    pub(crate) doc: Document,
    pub(crate) selection: Range<usize>,
    pub(crate) reversed: bool,
    pub(crate) marked: Option<Range<usize>>,
    goal_x: Option<Pixels>,
    pub(crate) list: ListState,
    pub(crate) focus: FocusHandle,
    pub(crate) theme: Theme,
    /// Block (and leaf within it) shown raw, as last laid out.
    active: Option<(BlockId, Option<usize>)>,
    /// Layout of the active block's raw text as last painted, with the
    /// document offset of its first byte.
    pub(crate) active_layout: Option<(TextLayout, usize)>,
    parse_task: Option<Task<()>>,
    /// A large paste being prepared off the UI thread; applied when ready,
    /// or right away (from `text`) before any other input.
    pending_paste: Option<PendingPaste>,
    /// The find bar, when open.
    pub(crate) find: Option<crate::find::FindState>,
    last_edit: Option<Instant>,
    /// The cursor moved without typing since the last edit.
    moved_since_edit: bool,
    pub(crate) selecting: bool,
    /// The caret should be scrolled into view at the next paint.
    reveal: bool,
    /// Block indices the list drew in the last completed frame, and the
    /// ones it is rendering in the current one.
    pub(crate) rendered: Range<usize>,
    pub(crate) rendering: Option<Range<usize>>,
    file: Option<PathBuf>,
    /// Buffer version last written to (or loaded from) `file`.
    saved_version: u64,
    /// Window title last set, to avoid resetting it every frame.
    shown_title: Option<String>,
    /// Frame-time overlay, when shown.
    pub(crate) frame_stats: Option<crate::frame_stats::FrameStats>,
    /// Per-frame timing log, when `TACHYON_FRAME_LOG` names a file.
    pub(crate) frame_log: Option<crate::frame_log::FrameLog>,
}

impl Editor {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_document(Document::new(text), window, cx)
    }

    /// Takes a document parsed elsewhere (e.g. on a background thread).
    pub fn with_document(doc: Document, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let editor = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            editor.update(cx, |editor, cx| editor.should_close(window, cx)).unwrap_or(true)
        });
        cx.on_next_frame(window, |editor, window, cx| editor.resolve_code_font(window, cx));
        let list = ListState::new(doc.blocks().len(), ListAlignment::Top, px(1000.));
        let mut editor = Editor {
            doc,
            selection: 0..0,
            reversed: false,
            marked: None,
            goal_x: None,
            list,
            focus,
            theme: Theme::dark(),
            active: None,
            active_layout: None,
            parse_task: None,
            pending_paste: None,
            find: None,
            last_edit: None,
            moved_since_edit: false,
            selecting: false,
            reveal: false,
            rendered: 0..0,
            rendering: None,
            file: None,
            saved_version: 0,
            shown_title: None,
            frame_stats: None,
            frame_log: crate::frame_log::FrameLog::from_env(),
        };
        editor.doc.take_splices();
        editor.update_active();
        editor.reparse(0, cx);
        editor
    }

    /// Replaces the whole document (file load finished, new paste).
    pub fn set_document(&mut self, doc: Document, cx: &mut Context<Self>) {
        self.saved_version = doc.buffer().version();
        self.doc = doc;
        self.doc.take_splices();
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.active = None;
        self.active_layout = None;
        self.parse_task = None;
        self.list.reset(self.doc.blocks().len());
        self.update_active();
        self.reparse(0, cx);
        cx.notify();
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Associates the editor with a file: saves go there and the current
    /// text counts as saved.
    pub fn set_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.file = Some(path);
        self.saved_version = self.doc.buffer().version();
        cx.notify();
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Edited since the last save or load.
    pub fn is_modified(&self) -> bool {
        self.doc.buffer().version() != self.saved_version
    }

    /// Window title: file name (or "Tachyon" for a scratch buffer), with a
    /// leading dot while there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| format!("{} - Tachyon", n.to_string_lossy()))
            .unwrap_or_else(|| "Tachyon".to_owned());
        if self.is_modified() { format!("• {name}") } else { name }
    }

    pub(crate) fn sync_title(&mut self, window: &mut Window) {
        let title = self.title();
        if self.shown_title.as_ref() != Some(&title) {
            window.set_window_title(&title);
            window.set_window_edited(self.is_modified());
            self.shown_title = Some(title);
        }
    }

    /// Unsaved changes: ask first. Returns whether the window may close now.
    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.flush_pending_paste(cx);
        if !self.is_modified() {
            return true;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Save changes before closing?",
            Some("Unsaved changes will be lost."),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await.ok();
            let _ = this.update_in(cx, |editor, window, cx| match choice {
                Some(0) => editor.save_then(window, cx, |window, _| window.remove_window()),
                Some(1) => window.remove_window(),
                _ => {}
            });
        })
        .detach();
        false
    }

    pub fn text(&self) -> String {
        self.doc.buffer().text()
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    /// The caret: the moving end of the selection.
    pub fn head(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    fn tail(&self) -> usize {
        if self.reversed { self.selection.end } else { self.selection.start }
    }

    /// Index of the block holding the caret; it is shown raw, entirely or
    /// (in lists, quotes and footnotes) just the leaf under the caret.
    pub fn active_block(&self) -> Option<usize> {
        self.doc.block_at(self.head())
    }

    /// The leaf of a container block (list, quote, footnote) holding the
    /// caret: its index and absolute source range. `None` means the whole
    /// active block is shown raw.
    pub fn active_leaf(&self) -> Option<(usize, Range<usize>)> {
        let index = self.active_block()?;
        let block = &self.doc.blocks()[index];
        let parsed = block.parsed();
        if block.is_stale()
            || !matches!(
                parsed.kind,
                BlockKind::List { .. } | BlockKind::BlockQuote(_) | BlockKind::Footnote(_)
            )
        {
            return None;
        }
        let start = self.doc.block_range(index).start;
        let caret = self.head() - start;
        let leaf = parsed.ir.leaves.iter().position(|leaf| {
            leaf.contains(&caret) || (caret == leaf.end && leaf.end == parsed.len)
        })?;
        let range = &parsed.ir.leaves[leaf];
        Some((leaf, start + range.start..start + range.end))
    }

    /// Picks the first installed monospace family once the window exists,
    /// off the startup path, and loads the fonts the editor draws with.
    fn resolve_code_font(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        warm_fonts(window, &self.theme, installed_code_font(window));
        if let Some(family) = installed_code_font(window)
            && self.theme.code_font != family
        {
            self.theme.code_font = family.into();
            self.list.remeasure();
            cx.notify();
        }
    }

    // ---- cursor ---------------------------------------------------------

    pub(crate) fn move_to(&mut self, offset: usize, select: bool, cx: &mut Context<Self>) {
        let offset = offset.min(self.doc.len());
        if select {
            let tail = self.tail();
            self.reversed = offset < tail;
            self.selection = offset.min(tail)..offset.max(tail);
        } else {
            self.selection = offset..offset;
            self.reversed = false;
        }
        self.marked = None;
        self.moved_since_edit = true;
        self.update_active();
        self.reveal_cursor();
        cx.notify();
    }

    fn move_by(
        &mut self,
        select: bool,
        cx: &mut Context<Self>,
        f: fn(&ropey::Rope, usize) -> usize,
    ) {
        self.goal_x = None;
        let head = self.head();
        let target = if !select && !self.selection.is_empty() {
            // Collapse a selection towards the direction of motion.
            let moved = f(self.doc.buffer().rope(), head);
            if moved < head { self.selection.start } else { self.selection.end }
        } else {
            f(self.doc.buffer().rope(), head)
        };
        self.move_to(target, select, cx);
    }

    fn vertical(&mut self, lines: isize, select: bool, cx: &mut Context<Self>) {
        let head = self.head();
        if let Some((layout, base)) = self.active_layout.clone()
            && head >= base
            && head - base <= layout.len()
            && let Some(position) = layout.position_for_index(head - base)
        {
            let line_height = layout.line_height();
            let x = self.goal_x.unwrap_or(position.x);
            let target = point(x, position.y + line_height * (lines as f32) + line_height / 2.);
            self.goal_x = Some(x);
            let bounds = layout.bounds();
            if target.y >= bounds.top() && target.y < bounds.bottom() {
                let (Ok(index) | Err(index)) = layout.index_for_position(target);
                return self.move_to(base + index, select, cx);
            }
        }
        // Leaving the laid-out block: continue by source lines.
        let target = movement::vertical(self.doc.buffer().rope(), head, lines);
        self.move_to(target, select, cx);
    }

    /// Scrolls by one viewport height and moves the caret by as many source lines as fit in it.
    fn page(&mut self, direction: isize, select: bool, cx: &mut Context<Self>) {
        let height = self.list.viewport_bounds().size.height;
        let line_height = self
            .active_layout
            .as_ref()
            .map_or(self.theme.text_size * 1.6, |(layout, _)| layout.line_height());
        let lines = ((height / line_height) as isize - 1).max(1);
        self.list.scroll_by(height * direction as f32);
        let target = movement::vertical(self.doc.buffer().rope(), self.head(), lines * direction);
        self.move_to(target, select, cx);
    }

    fn update_active(&mut self) {
        let index = self.active_block();
        let id =
            index.map(|i| (self.doc.blocks()[i].id(), self.active_leaf().map(|(leaf, _)| leaf)));
        if id == self.active {
            return;
        }
        // Both the block leaving and the block entering raw mode change height.
        if let Some((old, _)) = self.active
            && let Some(old_index) = self.doc.blocks().iter().position(|b| b.id() == old)
        {
            self.list.remeasure_items(old_index..old_index + 1);
        }
        if let Some(index) = index {
            self.list.remeasure_items(index..index + 1);
        }
        self.active = id;
        self.active_layout = None;
    }

    /// Keeps the caret in view. If its block is off screen (or not measured
    /// yet) the list scrolls to the block; the caret's line is then brought
    /// into view when the block paints (`reveal_caret_at`). Scrolling by
    /// block alone would jump to the top of a tall block on every keystroke.
    fn reveal_cursor(&mut self) {
        self.reveal = true;
        let Some(index) = self.active_block() else { return };
        if self.rendered.contains(&index) {
            return;
        }
        if index > self.rendered.end {
            // Blocks the list has not measured count as zero height, so
            // revealing one further down than the next would compute a
            // position above the current one and not scroll (a long paste,
            // Ctrl+End). Put the block at the top instead.
            self.scroll_to_caret();
        } else {
            self.list.scroll_to_reveal_item(index);
        }
    }

    /// Called while painting the caret (window coordinates). Scrolls so the
    /// caret's line is visible with a line of margin if a reveal is pending.
    /// Returns whether it scrolled.
    pub(crate) fn reveal_caret_at(&mut self, top: Pixels, line_height: Pixels) -> bool {
        if !std::mem::take(&mut self.reveal) {
            return false;
        }
        let viewport = self.list.viewport_bounds();
        let margin = line_height.min(viewport.size.height / 4.);
        let above = top - margin - viewport.top();
        let below = top + line_height + margin - viewport.bottom();
        let distance = if above < px(0.) {
            above
        } else if below > px(0.) {
            below
        } else {
            return false;
        };
        self.list.scroll_by(distance);
        true
    }

    // ---- editing --------------------------------------------------------

    /// Replaces `range` with `text` and puts the caret after it.
    /// An edit that replaces `range` as its own undo step (find and replace).
    pub(crate) fn replace_selection_with(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        self.moved_since_edit = true;
        self.replace(range, text, cx);
        self.doc.seal_undo_group();
    }

    pub(crate) fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let now = Instant::now();
        self.replace_at(now, range, Insert::Text(text), cx);
        self.charge_work("edit", now);
    }

    /// Charges the time since `started` to the frame-time overlay.
    fn charge_work(&mut self, label: &'static str, started: Instant) {
        if let Some(stats) = &mut self.frame_stats {
            stats.work(started);
        }
        if let Some(log) = &mut self.frame_log {
            log.work(label, started);
        }
    }

    fn replace_at(
        &mut self,
        now: Instant,
        range: Range<usize>,
        text: Insert,
        cx: &mut Context<Self>,
    ) {
        // Undo groups: typing runs group together; navigation, pauses, line
        // breaks and replacing a selection start new ones. An IME
        // composition (replacing its own marked text) stays one group.
        let paused = self.last_edit.is_none_or(|t| now - t > UNDO_GROUP_PAUSE);
        let composing = self.marked.as_ref() == Some(&range);
        let replaces_selection = range.len() > 1 && !composing;
        let multiline = text.as_str().contains('\n');
        if self.moved_since_edit || paused || multiline || replaces_selection {
            self.doc.seal_undo_group();
        }
        let edited = match text {
            Insert::Text(text) => self.doc.edit(range, text),
            Insert::Prepared(insert) => self.doc.edit_prepared(range, insert),
        };
        let Ok(edit) = edited else { return };
        if multiline {
            self.doc.seal_undo_group();
        }
        self.last_edit = Some(now);
        self.moved_since_edit = false;
        let caret = edit.new_range().end;
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.after_edit(cx);
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.refresh_find();
        self.reparse(self.head(), cx);
        self.update_active();
        self.reveal_cursor();
        cx.notify();
    }

    /// Offset of the first block in view.
    fn viewport_offset(&self) -> usize {
        match self.doc.blocks().len() {
            0 => 0,
            n => self.doc.block_range(self.list.logical_scroll_top().item_ix.min(n - 1)).start,
        }
    }

    /// Runs pending parse jobs: small ones inline, a large one on the
    /// background executor (one at a time; it continues when it lands). A
    /// large dirty range streams back in chunks, starting at `focus`: the
    /// caret after an edit (the view is about to reveal it), else the top of
    /// the viewport.
    fn reparse(&mut self, focus: usize, cx: &mut Context<Self>) {
        // Parse results keep the text in view where it is. After an edit the
        // list is behind the document, so there is no anchor; the caret is
        // revealed instead.
        let anchor = (!self.doc.has_splices()).then(|| self.viewport_offset());
        while self.parse_task.is_none() {
            let Some(job) = self.doc.parse_job_near(focus, tachyon_doc::PARSE_CHUNK) else {
                break;
            };
            if job.is_small() {
                let result = job.run();
                self.doc.apply(result);
                continue;
            }
            self.parse_task = Some(cx.spawn(async move |this, cx| {
                let result = cx.background_executor().spawn(async move { job.run() }).await;
                let _ = this.update(cx, |editor, cx| {
                    let started = Instant::now();
                    editor.parse_task = None;
                    // Blocks replacing the top one by a different number lose
                    // the scroll position. If the caret was on screen (just
                    // pasted), keep it there; otherwise keep the same text.
                    let caret_drawn =
                        editor.active_block().is_some_and(|i| editor.rendered.contains(&i));
                    let anchor = (!caret_drawn).then(|| editor.viewport_offset());
                    editor.doc.apply(result);
                    if editor.apply_splices(anchor) && caret_drawn {
                        editor.scroll_to_caret();
                    }
                    let focus = editor.viewport_offset();
                    editor.reparse(focus, cx);
                    editor.update_active();
                    editor.charge_work("parse", started);
                    cx.notify();
                });
            }));
        }
        self.apply_splices(anchor);
    }

    /// Mirrors block-list changes into the list. `ListState::splice` resets
    /// the scroll offset to the top of a replaced item, so an edit inside a
    /// tall block scrolled half out of view would jump; for a one-for-one
    /// replacement keep the pixel offset instead.
    ///
    /// When the top block is replaced by a different number of blocks, the
    /// position within it is lost; with `anchor` (the byte offset of the top
    /// block before the change, when the text did not change) the view moves
    /// to the block now holding that byte. Anchoring on text matters for
    /// streamed parse results: each chunk also reparses the block after it,
    /// so without it the view would climb one chunk per result.
    /// Returns whether the top block was displaced.
    fn apply_splices(&mut self, anchor: Option<usize>) -> bool {
        let splices = self.doc.take_splices();
        let top = self.list.logical_scroll_top();
        let (mut item, mut offset) = (top.item_ix, top.offset_in_item);
        let mut displaced = false;
        for splice in splices {
            let Splice { old, new_len } = splice;
            if old.contains(&item) {
                item = old.start;
                // Only a one-for-one replacement (an edit inside the block)
                // is the same content; after a split or merge the offset
                // may not fit the new item.
                if old.len() != 1 || new_len != 1 {
                    offset = px(0.);
                    displaced = true;
                }
            } else if old.end <= item {
                item = item - old.len() + new_len;
            }
            self.rendered = map_drawn(&self.rendered, &old, new_len);
            self.list.splice(old, new_len);
        }
        if displaced && let Some(index) = anchor.and_then(|at| self.doc.block_at(at)) {
            self.list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
        } else if offset > px(0.) && item < self.list.item_count() {
            self.list.scroll_to(ListOffset { item_ix: item, offset_in_item: offset });
        }
        displaced
    }

    /// Scrolls the caret's block to the top; the caret's line is brought
    /// into view when it paints.
    fn scroll_to_caret(&mut self) {
        self.reveal = true;
        if let Some(index) = self.active_block() {
            self.list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
        }
    }

    fn delete_towards(&mut self, cx: &mut Context<Self>, f: fn(&ropey::Rope, usize) -> usize) {
        let range = if self.selection.is_empty() {
            let head = self.head();
            let other = f(self.doc.buffer().rope(), head);
            head.min(other)..head.max(other)
        } else {
            self.selection.clone()
        };
        if !range.is_empty() {
            self.replace(range, "", cx);
        }
    }

    pub(crate) fn selected_text(&self) -> String {
        self.doc.buffer().rope().byte_slice(self.selection.clone()).to_string()
    }

    pub(crate) fn mouse_down(
        &mut self,
        offset: usize,
        extend: bool,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending_paste(cx);
        window.focus(&self.focus, cx);
        self.goal_x = None;
        if click_count >= 2 && !extend {
            let rope = self.doc.buffer().rope();
            let start = movement::prev_word(rope, movement::next_grapheme(rope, offset));
            let end = movement::next_word(rope, start);
            self.move_to(start, false, cx);
            self.move_to(end, true, cx);
        } else {
            self.move_to(offset, extend, cx);
        }
        self.selecting = true;
    }

    pub(crate) fn mouse_drag(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selecting {
            self.move_to(offset, true, cx);
        }
    }

    // ---- actions --------------------------------------------------------

    pub(crate) fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.finding() {
            return self.find_backspace(cx);
        }
        self.delete_towards(cx, movement::prev_grapheme);
    }
    pub(crate) fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_towards(cx, movement::next_grapheme);
    }
    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_towards(cx, movement::prev_word);
    }
    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_towards(cx, movement::next_word);
    }
    pub(crate) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(false, cx, movement::prev_grapheme);
    }
    pub(crate) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(false, cx, movement::next_grapheme);
    }
    pub(crate) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, cx);
    }
    pub(crate) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, cx);
    }
    pub(crate) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(false, cx, movement::prev_word);
    }
    pub(crate) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(false, cx, movement::next_word);
    }
    pub(crate) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(false, cx, movement::home);
    }
    pub(crate) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(false, cx, movement::end);
    }
    pub(crate) fn document_start(
        &mut self,
        _: &DocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.goal_x = None;
        self.move_to(0, false, cx);
    }
    pub(crate) fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.move_to(self.doc.len(), false, cx);
    }
    pub(crate) fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page(-1, false, cx);
    }
    pub(crate) fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page(1, false, cx);
    }
    pub(crate) fn select_page_up(
        &mut self,
        _: &SelectPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page(-1, true, cx);
    }
    pub(crate) fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page(1, true, cx);
    }
    pub(crate) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(true, cx, movement::prev_grapheme);
    }
    pub(crate) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(true, cx, movement::next_grapheme);
    }
    pub(crate) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, cx);
    }
    pub(crate) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, cx);
    }
    pub(crate) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_by(true, cx, movement::prev_word);
    }
    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_by(true, cx, movement::next_word);
    }
    pub(crate) fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(true, cx, movement::home);
    }
    pub(crate) fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_by(true, cx, movement::end);
    }
    pub(crate) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, false, cx);
        self.move_to(self.doc.len(), true, cx);
    }
    pub(crate) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.finding() {
            return self.find_enter(window, cx);
        }
        self.replace(self.selection.clone(), "\n", cx);
    }
    /// Shift+Enter: the previous match while finding, else a line break like Enter.
    pub(crate) fn shift_newline(
        &mut self,
        _: &ShiftNewline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.finding() {
            return self.find_previous(&FindPrevious, window, cx);
        }
        self.replace(self.selection.clone(), "\n", cx);
    }
    pub(crate) fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        if self.find_switch_field(cx) {
            return;
        }
        self.replace(self.selection.clone(), "    ", cx);
    }
    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text()));
        }
    }
    pub(crate) fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text()));
            self.replace(self.selection.clone(), "", cx);
        }
    }
    pub(crate) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        let started = Instant::now();
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        self.charge_work("clipboard", started);
        let Some(text) = text else { return };
        if self.finding() {
            return self.find_input(&text, false, cx);
        }
        self.moved_since_edit = true;
        if text.len() <= tachyon_doc::UNPARSED_SPLIT_THRESHOLD {
            self.replace(self.selection.clone(), &text, cx);
            return;
        }
        // Normalizing, building the rope and pre-segmenting megabytes take
        // milliseconds: do them off the UI thread, then splice the result in.
        let text: Arc<str> = text.into();
        let shared = Arc::clone(&text);
        let task = cx.spawn(async move |this, cx| {
            let insert =
                cx.background_executor().spawn(async move { PreparedInsert::new(&shared) }).await;
            let _ = this.update(cx, |editor, cx| {
                if editor.pending_paste.take().is_some() {
                    let now = Instant::now();
                    editor.replace_at(now, editor.selection.clone(), Insert::Prepared(insert), cx);
                    editor.charge_work("paste", now);
                }
            });
        });
        self.pending_paste = Some(PendingPaste { text, _task: task });
    }

    /// Applies a paste still being prepared, from its text on this thread.
    /// Called before any other input so edits keep their order.
    pub(crate) fn flush_pending_paste(&mut self, cx: &mut Context<Self>) {
        if let Some(paste) = self.pending_paste.take() {
            self.replace(self.selection.clone(), &paste.text, cx);
        }
    }
    pub(crate) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(edits) = self.doc.undo() {
            self.caret_after(&edits);
            self.moved_since_edit = true;
            self.after_edit(cx);
        }
    }
    pub(crate) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(edits) = self.doc.redo() {
            self.caret_after(&edits);
            self.moved_since_edit = true;
            self.after_edit(cx);
        }
    }

    /// Closes the window, asking first if there are unsaved changes.
    pub(crate) fn close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.should_close(window, cx) {
            window.remove_window();
        }
    }

    pub(crate) fn toggle_frame_stats(
        &mut self,
        _: &ToggleFrameStats,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.frame_stats = match self.frame_stats {
            Some(_) => None,
            None => Some(Default::default()),
        };
        cx.notify();
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.save_then(window, cx, |_, _| {});
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_path_then_save(window, cx, |_, _| {});
    }

    /// Saves to the associated file (asking for one if there is none), then
    /// runs `after` on success.
    fn save_then(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        self.flush_pending_paste(cx);
        match self.file.clone() {
            Some(path) => self.write_to(path, window, cx, after),
            None => self.prompt_path_then_save(window, cx, after),
        }
    }

    fn prompt_path_then_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let directory = self
            .file
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let name = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| "untitled.md".to_owned(), |n| n.to_string_lossy().into_owned());
        let chosen = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else { return };
            let _ =
                this.update_in(cx, |editor, window, cx| editor.write_to(path, window, cx, after));
        })
        .detach();
    }

    /// Writes the text (original line endings restored) off the UI thread.
    fn write_to(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let text = self.doc.buffer().to_saved_text();
        let version = self.doc.buffer().version();
        cx.spawn_in(window, async move |this, cx| {
            let target = path.clone();
            let written = cx
                .background_executor()
                .spawn(async move { write_atomically(&target, text.as_bytes()) })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| match written {
                Ok(()) => {
                    editor.file = Some(path);
                    editor.saved_version = version;
                    editor.sync_title(window);
                    cx.notify();
                    after(window, cx);
                }
                Err(e) => {
                    let detail = format!("{}: {e}", path.display());
                    // The answer carries no choice; dropping the receiver just ignores it.
                    drop(window.prompt(
                        PromptLevel::Critical,
                        "Could not save",
                        Some(&detail),
                        &["OK"],
                        cx,
                    ));
                }
            });
        })
        .detach();
    }

    fn caret_after(&mut self, edits: &[tachyon_text::Edit]) {
        if let Some(last) = edits.last() {
            let caret = last.new_range().end.min(self.doc.len());
            self.selection = caret..caret;
            self.reversed = false;
            self.marked = None;
        }
    }

    // ---- UTF-16 helpers for the platform input handler -------------------

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let buffer = self.doc.buffer();
        buffer.utf16_to_byte(range.start)..buffer.utf16_to_byte(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let buffer = self.doc.buffer();
        buffer.byte_to_utf16(range.start)..buffer.byte_to_utf16(range.end)
    }

    pub(crate) fn position_for_offset(&self, offset: usize) -> Option<Point<Pixels>> {
        let (layout, base) = self.active_layout.as_ref()?;
        let local = offset.checked_sub(*base).filter(|&l| l <= layout.len())?;
        layout.position_for_index(local)
    }
}

/// Lays out a sample line in every face the editor draws with, once per
/// process. A face's first use costs milliseconds (loading it and setting up
/// shaping), and in a new scratch window the first text drawn is often a
/// large paste, whose frame would pay for it.
fn warm_fonts(window: &Window, theme: &Theme, code_font: Option<&'static str>) {
    static WARMED: std::sync::Once = std::sync::Once::new();
    WARMED.call_once(|| {
        let sample = "The quick brown fox, 0123456789 ([{<*_`~|>}]).";
        let base = window.text_style().font();
        let mut faces = vec![
            base.clone(),
            Font { weight: FontWeight::BOLD, ..base.clone() },
            Font { style: FontStyle::Italic, ..base.clone() },
            Font { weight: FontWeight::BOLD, style: FontStyle::Italic, ..base.clone() },
        ];
        if let Some(family) = code_font {
            faces.push(Font { family: family.into(), ..base.clone() });
            faces.push(Font { family: family.into(), weight: FontWeight::BOLD, ..base });
        }
        for font in faces {
            let run = TextRun {
                len: sample.len(),
                font,
                color: theme.foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window.text_system().layout_line(sample, theme.text_size, &[run], None);
        }
    });
}

/// The first installed monospace candidate, looked up once per process:
/// listing the system fonts walks the whole font collection, and a resident
/// instance opens many windows.
fn installed_code_font(window: &Window) -> Option<&'static str> {
    static FONT: OnceLock<Option<&'static str>> = OnceLock::new();
    *FONT.get_or_init(|| {
        let installed = window.text_system().all_font_names();
        tachyon_platform::monospace_font_candidates()
            .iter()
            .find(|family| installed.iter().any(|name| name == *family))
            .copied()
    })
}

/// Opens files dropped onto an editor window. Set by the application, which owns windows; without
/// it drops are ignored.
pub struct OpenPaths(pub std::rc::Rc<OpenPathsFn>);

/// Handler type for [`OpenPaths`].
pub type OpenPathsFn = dyn Fn(Vec<PathBuf>, &mut App);

impl gpui::Global for OpenPaths {}

/// A large paste whose text is being prepared off the UI thread.
struct PendingPaste {
    text: Arc<str>,
    /// Applies the prepared text when ready; dropped (cancelled) when the
    /// paste is applied from `text` instead.
    _task: Task<()>,
}

/// Text to insert: as is, or prepared off the UI thread.
enum Insert<'a> {
    Text(&'a str),
    Prepared(PreparedInsert),
}

impl Insert<'_> {
    fn as_str(&self) -> &str {
        match self {
            Insert::Text(text) => text,
            Insert::Prepared(insert) => insert.as_str(),
        }
    }
}

/// Maps the block indices drawn last frame through a splice replacing `old`
/// with `new_len` blocks. Replaced blocks that were drawn count as that many
/// of their replacements (the first ones): a drawn block split in two is
/// still near the viewport, but thousands of blocks replacing one are not
/// all in view.
pub(crate) fn map_drawn(drawn: &Range<usize>, old: &Range<usize>, new_len: usize) -> Range<usize> {
    let shift = |ix: usize| ix - old.len() + new_len;
    let start = if drawn.start < old.start {
        drawn.start
    } else if drawn.start >= old.end {
        shift(drawn.start)
    } else {
        old.start
    };
    let end = if drawn.end <= old.start {
        drawn.end
    } else if drawn.end > old.end {
        shift(drawn.end)
    } else {
        let replaced = drawn.end - drawn.start.max(old.start);
        old.start + replaced.min(new_len)
    };
    start..end.max(start)
}

/// Writes `bytes` to a temporary file next to `path` and renames it over
/// `path`, so a crash or full disk never leaves a truncated file. Follows a
/// symlink to its target and keeps the target's permissions.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let temp = path.with_file_name(format!(".{}.tachyon-save", name.to_string_lossy()));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Ok(metadata) = std::fs::metadata(&path) {
            std::fs::set_permissions(&temp, metadata.permissions())?;
        }
        std::fs::rename(&temp, &path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.doc.buffer().rope().byte_slice(range).to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.find_end_composition();
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending_paste(cx);
        if self.finding() {
            return self.find_input(text, false, cx);
        }
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending_paste(cx);
        if self.finding() {
            return self.find_input(text, true, cx);
        }
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let start = range.start;
        self.replace(range, text, cx);
        let inserted = self.head() - start;
        self.marked = (inserted > 0).then(|| start..start + inserted);
        if let Some(selected) = new_selected_range_utf16 {
            // Relative to the start of the marked text, in UTF-16.
            let base16 = self.doc.buffer().byte_to_utf16(start);
            let selected = self.range_from_utf16(&(base16 + selected.start..base16 + selected.end));
            self.selection = selected;
            self.reversed = false;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.position_for_offset(range.start)?;
        let end = self.position_for_offset(range.end).unwrap_or(start);
        let line_height = self.active_layout.as_ref()?.0.line_height();
        Some(Bounds::from_corners(start, point(end.x.max(start.x), end.y + line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let (layout, base) = self.active_layout.as_ref()?;
        let (Ok(index) | Err(index)) = layout.index_for_position(point);
        Some(self.doc.buffer().byte_to_utf16(base + index))
    }
}
