use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable, Font,
    FontStyle, FontWeight, KeyBinding, ListAlignment, ListOffset, ListState, Pixels, Point,
    PromptLevel, SharedString, Task, TextLayout, TextRun, UTF16Selection, Window, actions, point,
    px,
};
use tachyon_doc::{BlockId, DocMode, Document, PreparedInsert, Splice};
use tachyon_md::{BlockKind, ParsedBlock};

use crate::movement;
use crate::theme::{AppearanceHint, Theme, is_dark};

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
        ZoomIn,
        ZoomOut,
        ZoomReset,
        PageDown,
        ShiftNewline,
        Find,
        GoToHeading,
        OpenRecent,
        OpenCommandPalette,
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
        Outdent,
        CopyAsHtml,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Save,
        SaveAs,
        ToggleFrameStats,
        ToggleTextMode,
        CloseWindow,
        ToggleNotePin,
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
        KeyBinding::new("secondary-shift-o", GoToHeading, c),
        KeyBinding::new("secondary-r", OpenRecent, c),
        KeyBinding::new("secondary-p", OpenRecent, c),
        KeyBinding::new("secondary-shift-p", OpenCommandPalette, c),
        KeyBinding::new("ctrl-h", Replace, c),
        KeyBinding::new("cmd-alt-f", Replace, c),
        KeyBinding::new("secondary-enter", ReplaceAll, c),
        KeyBinding::new("f3", FindNext, c),
        KeyBinding::new("shift-f3", FindPrevious, c),
        KeyBinding::new("secondary-g", FindNext, c),
        KeyBinding::new("secondary-shift-g", FindPrevious, c),
        KeyBinding::new("escape", Cancel, c),
        KeyBinding::new("secondary-=", ZoomIn, c),
        KeyBinding::new("secondary-+", ZoomIn, c),
        KeyBinding::new("secondary-shift-=", ZoomIn, c),
        KeyBinding::new("secondary--", ZoomOut, c),
        KeyBinding::new("secondary-0", ZoomReset, c),
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
        KeyBinding::new("shift-tab", Outdent, c),
        KeyBinding::new("secondary-shift-c", CopyAsHtml, c),
        KeyBinding::new("secondary-c", Copy, c),
        KeyBinding::new("secondary-x", Cut, c),
        KeyBinding::new("secondary-v", Paste, c),
        KeyBinding::new("secondary-z", Undo, c),
        KeyBinding::new("secondary-shift-z", Redo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("secondary-s", Save, c),
        KeyBinding::new("secondary-shift-s", SaveAs, c),
        KeyBinding::new("ctrl-alt-f", ToggleFrameStats, c),
        KeyBinding::new("ctrl-shift-m", ToggleTextMode, c),
        KeyBinding::new("secondary-w", CloseWindow, c),
        KeyBinding::new("ctrl-shift-t", ToggleNotePin, c),
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
    /// The text column's width setting (`Settings::content_width`); resolved into
    /// `theme.content_width` fresh every frame from the current window size and zoom
    /// (`Editor::render`), so a resize alone re-wraps blocks without a settings round-trip.
    pub(crate) content_width: crate::settings::ContentWidth,
    /// `theme.content_width` as resolved last frame: `Editor::render` remeasures the list
    /// (`ListState::remeasure`) whenever the resolved value actually changes, whatever the
    /// cause - a window resize changing a percentage width just as much as a settings save or
    /// zoom changing a pixel one - since `ListState` caches each item's own measured size and
    /// nothing else would tell it that this block's available width, and so its wrapped height,
    /// changed underneath it.
    pub(crate) resolved_content_width: Option<Pixels>,
    /// The theme follows an [`AppearanceHint`] that GPUI's window appearance has not caught up
    /// with yet; until it does, the window's (default) appearance is ignored.
    appearance_unconfirmed: bool,
    /// Block (and leaf within it) shown raw, as last laid out.
    active: Option<(BlockId, Option<usize>)>,
    /// Whether the caret's block renders raw. Left by `Editor::cancel` without losing the
    /// caret: `Editor::active_block`/`Editor::update_active` then report no active block, so
    /// every block renders and the caret is not painted. The next click, keystroke, caret
    /// movement or edit turns it back on (`Editor::move_to`, `Editor::after_edit`),
    /// re-entering at the caret's kept offset.
    pub(crate) editing: bool,
    /// Layout of the active block's raw text as last painted, with the
    /// document offset of its first byte.
    pub(crate) active_layout: Option<(TextLayout, usize)>,
    parse_task: Option<Task<()>>,
    /// A large paste being prepared off the UI thread; applied when ready, or right away
    /// before input that cannot simply queue after it (see `flush_pending_paste`). Plain typed
    /// text queues on it instead, to land right after it (see `replace_text_in_range`).
    pending_paste: Option<PendingPaste>,
    /// The find bar, when open.
    pub(crate) find: Option<crate::find::FindState>,
    /// The open picker (Go to heading, Open recent).
    pub(crate) picker: Option<crate::picker::Picker>,
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
    /// This frame's viewport bounds and each of `rendered`'s blocks' own on-screen bounds,
    /// snapshotted from `self.list` (see `Render::render`) before the list element's own layout
    /// pass borrows its `ListState` for the frame: `render_window` reads these instead of
    /// calling `ListState::viewport_bounds`/`bounds_for_item` directly, which would re-enter
    /// that same borrow from inside the list's own `render_item` callback and panic.
    pub(crate) window_viewport: Bounds<Pixels>,
    pub(crate) window_item_bounds: std::collections::HashMap<usize, Bounds<Pixels>>,
    /// Cache of [`tachyon_doc::plain_chunk_lens`]'s split of the last oversized raw block
    /// [`crate::render::Editor::render_raw`] handled: recomputing it (a byte/line scan of the
    /// whole block) on every repaint that touches this block - a font-swap
    /// `ListState::remeasure`, the caret blinking, or any other frame with no edit in between -
    /// cost as much as the whole block regardless of how few of its chunks a window actually
    /// draws (the cause of a measured ~79 ms frame on a 15 MiB block with no blank lines).
    /// Keyed by the range and the buffer's version (an edit always changes the version); one
    /// slot, since only one block ever needs this at a time (`DocMode::Plain`'s own blocks are
    /// already small enough never to reach `RAW_SPLIT_THRESHOLD`).
    pub(crate) raw_chunk_cache: Option<(Range<usize>, u64, Vec<usize>)>,
    /// Cache of the per-line height estimate `render_rendered` builds for a windowed
    /// (`ir.lines.len() > LINE_SPLIT_THRESHOLD`) non-active block, for the same reason as
    /// `raw_chunk_cache`. Keyed by the block's own `Arc<ParsedBlock>` (kept alive here so a
    /// pointer comparison can never alias a dropped-and-reallocated one), unchanged across
    /// frames unless the block is actually reparsed.
    pub(crate) rendered_heights_cache: Option<(Arc<ParsedBlock>, Vec<Pixels>)>,
    /// Window-coordinate y of the bottom edge of the open find bar overlay, refreshed every
    /// frame it renders (see `render::find_bar`); `reveal_caret_at` keeps revealed content below
    /// it so the bar (which floats over the document without reflowing it) never hides the
    /// target. Stale while the bar is closed, but then unused.
    pub(crate) find_bar_bottom: Pixels,
    /// Whether the document caret's quad was painted during the last completed frame. False
    /// while the find bar or a picker holds typing, so only the field's own caret is visible.
    pub(crate) caret_painted: bool,
    pub(crate) file: Option<PathBuf>,
    /// The buffer's undo/redo history position ([`tachyon_text::Buffer::history_position`]) at
    /// the last save or load: `is_modified` compares against this, so undo/redo back to exactly
    /// this point clears the dirty marker, not only "no edit happened since" - an O(1) integer
    /// compare, never a whole-buffer scan, even on a huge document.
    pub(crate) saved_history_position: u64,
    /// The file's modification time and size as last read or written (see `disk`).
    pub(crate) disk_stamp: Option<crate::disk::DiskStamp>,
    /// The file changed on disk while the document had unsaved changes: Save asks first.
    pub(crate) disk_changed: bool,
    /// This document's backup file, once it has had unsaved changes (see `backup`).
    pub(crate) backup_slot: Option<PathBuf>,
    /// Buffer version last written to the backup.
    pub(crate) backed_up_version: Option<u64>,
    /// The pending backup write.
    pub(crate) backup_task: Option<Task<()>>,
    /// Window title last set, to avoid resetting it every frame.
    shown_title: Option<String>,
    /// Frame-time overlay, when shown.
    pub(crate) frame_stats: Option<crate::frame_stats::FrameStats>,
    /// Per-frame timing log, when `TACHYON_FRAME_LOG` names a file.
    pub(crate) frame_log: Option<crate::frame_log::FrameLog>,
    /// Some of the file's bytes were not valid UTF-8 and were replaced with U+FFFD when it was
    /// read: saving would change its bytes, so `save`/`save_as` warn first.
    pub(crate) lossy: bool,
    /// A one-line notice shown at the top of the view (currently only the oversized-Markdown
    /// fallback; see `disk::oversized_markdown_notice`).
    pub(crate) notice: Option<SharedString>,
    /// A one-line notice from the application rather than about this document (a sticky-note
    /// hotkey that another app already holds). Shown when `notice` is empty, and kept when the
    /// document is replaced, unlike `notice`: it is often set while a file is still loading.
    pub(crate) app_notice: Option<SharedString>,
    /// This window's rotating tip (`Settings::tips`; `tips::next_tip`), drawn faintly behind the
    /// document (`render::tip_overlay`). `None` before `refresh_tip` first runs (deferred past
    /// the first frame, see `with_document`) or while tips are turned off; picked once and kept
    /// for the rest of the window's life otherwise.
    pub(crate) tip: Option<SharedString>,
    /// The window's placement (normal rectangle, maximized) as last seen while it was on screen,
    /// kept current by bounds and visibility observers (`with_document`). The session file saves
    /// this rather than the window's state at save time, so a window that is minimized when the
    /// session is saved comes back as it was before it was minimized. `None` until the window is
    /// first seen on screen.
    pub(crate) last_placement: Option<(Bounds<Pixels>, bool)>,
    /// Fixed window title, if set: for a special document whose title should not follow the
    /// usual file-name / scratch-buffer rule (the What's new window; see `tachyon::whats_new`).
    /// Set once, right after the editor is created (`set_title_override`); nothing clears it.
    title_override: Option<String>,
    /// Sticky-note state (`notes.rs`): `Some` only for a sticky note window
    /// (`Editor::make_note`), `None` for an ordinary editor window.
    pub(crate) note: Option<crate::notes::NoteState>,
}

/// Zoom levels `Ctrl+=` and `Ctrl+-` step through, as in browsers.
const ZOOM_STEPS: [f32; 13] = [0.5, 0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2., 2.5, 3.];

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
        // Deferred past construction like `resolve_code_font` above, but through a spawned task
        // rather than `on_next_frame`: unlike a font warm-up tied to an actual paint, nothing
        // here needs a frame to have been drawn, and a task the executor runs at its next turn
        // keeps this reliably observable in tests (`run_until_parked`) instead of depending on
        // a redraw the test harness has no other reason to schedule.
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |editor, cx| editor.refresh_tip(cx));
        })
        .detach();
        cx.observe_window_appearance(window, |editor, window, cx| {
            editor.follow_appearance(window, cx);
        })
        .detach();
        cx.observe_window_activation(window, |editor, window, cx| {
            if window.is_window_active() {
                editor.check_disk(window, cx);
            }
            editor.apply_note_opacity(window, cx);
        })
        .detach();
        cx.observe_window_bounds(window, |editor, window, _| editor.note_placement(window))
            .detach();
        cx.observe_window_visibility(window, |editor, _, window, _| editor.note_placement(window))
            .detach();
        let list = ListState::new(doc.blocks().len(), ListAlignment::Top, px(1000.));
        let reported = is_dark(window.appearance());
        let hint = cx.try_global::<AppearanceHint>().map(|hint| hint.dark);
        let settings = cx.try_global::<crate::Settings>().cloned().unwrap_or_default();
        let mut editor = Editor {
            doc,
            selection: 0..0,
            reversed: false,
            marked: None,
            goal_x: None,
            list,
            focus,
            theme: Theme::for_dark(settings.dark(hint.unwrap_or(reported)))
                .with_text_font(window)
                .zoomed(settings.zoom),
            content_width: settings.content_width,
            resolved_content_width: None,
            appearance_unconfirmed: settings.theme == crate::ThemeChoice::System
                && hint.is_some_and(|dark| dark != reported),
            active: None,
            editing: true,
            active_layout: None,
            parse_task: None,
            pending_paste: None,
            find: None,
            picker: None,
            last_edit: None,
            moved_since_edit: false,
            selecting: false,
            reveal: false,
            rendered: 0..0,
            rendering: None,
            window_viewport: Bounds::default(),
            window_item_bounds: std::collections::HashMap::new(),
            raw_chunk_cache: None,
            rendered_heights_cache: None,
            find_bar_bottom: px(0.),
            caret_painted: false,
            file: None,
            saved_history_position: 0,
            disk_stamp: None,
            disk_changed: false,
            backup_slot: None,
            backed_up_version: None,
            backup_task: None,
            shown_title: None,
            frame_stats: None,
            frame_log: crate::frame_log::FrameLog::from_env(),
            lossy: false,
            notice: None,
            app_notice: None,
            tip: None,
            last_placement: None,
            title_override: None,
            note: None,
        };
        editor.doc.take_splices();
        editor.update_active();
        editor.reparse(0, cx);
        editor
    }

    /// Replaces the whole document (file load finished, new paste).
    pub fn set_document(&mut self, doc: Document, cx: &mut Context<Self>) {
        self.saved_history_position = doc.buffer().history_position();
        self.doc = doc;
        self.doc.take_splices();
        self.lossy = false;
        self.notice = None;
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.active = None;
        self.active_layout = None;
        self.editing = true;
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
        crate::picker::RecentFiles::note(&path, cx);
        self.disk_stamp = crate::disk::DiskStamp::of(&path);
        self.disk_changed = false;
        self.file = Some(path);
        self.saved_history_position = self.doc.buffer().history_position();
        cx.notify();
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Edited since the last save or load.
    pub fn is_modified(&self) -> bool {
        self.doc.buffer().history_position() != self.saved_history_position
    }

    /// Window title: `title_override` if one was set and the document has no file yet, else the
    /// file name (or "Tachyon" for a scratch buffer), with a leading dot while there are unsaved
    /// changes. Save As on an overridden document (the What's new window) gives it a file, so
    /// this reverts to the ordinary rule from then on rather than keeping a stale fixed title.
    pub fn title(&self) -> String {
        if self.is_note() {
            return self
                .file
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Sticky note".to_owned());
        }
        if self.file.is_none()
            && let Some(title) = &self.title_override
        {
            return title.clone();
        }
        let name = self
            .file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| format!("{} - Tachyon", n.to_string_lossy()))
            .unwrap_or_else(|| "Tachyon".to_owned());
        if self.is_modified() { format!("• {name}") } else { name }
    }

    /// Overrides the ordinary file-name/scratch-buffer window title with a fixed one: for a
    /// special document that should keep its own name rather than being mistaken for an
    /// untitled scratch buffer (the What's new window). Meant to be called once, right after
    /// the editor is created, before its first frame.
    pub fn set_title_override(&mut self, title: impl Into<String>) {
        self.title_override = Some(title.into());
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
        // A sticky note saves itself and never asks, however it is closed: its own close button,
        // Alt+F4, the taskbar or the Windows close area.
        if self.is_note() {
            self.flush_note(cx);
            return true;
        }
        if !self.is_modified() {
            self.discard_backup();
            return true;
        }
        // Quit with hot exit: the backup keeps the text for the next start.
        if cx.has_global::<crate::HotExit>() && self.backup_now(cx) {
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
                Some(1) => {
                    editor.discard_backup();
                    window.remove_window();
                }
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

    pub(crate) fn tail(&self) -> usize {
        if self.reversed { self.selection.end } else { self.selection.start }
    }

    /// Index of the block holding the caret; it is shown raw, entirely or
    /// (in lists, quotes and footnotes) just the leaf under the caret. `None` while not
    /// editing (`Editor::cancel` left edit mode): every block then renders, though the caret
    /// keeps its offset.
    pub fn active_block(&self) -> Option<usize> {
        if !self.editing {
            return None;
        }
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
    /// Switches between the dark and light theme when the system appearance changes.
    pub(crate) fn follow_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = cx.try_global::<crate::Settings>().map(|s| s.theme);
        if theme.is_some_and(|theme| theme != crate::ThemeChoice::System) {
            return;
        }
        let reported = is_dark(window.appearance());
        if self.appearance_unconfirmed {
            if reported != self.theme.dark {
                return;
            }
            // GPUI knows the system appearance now; later windows can trust it.
            self.appearance_unconfirmed = false;
            if cx.has_global::<AppearanceHint>() {
                cx.remove_global::<AppearanceHint>();
            }
        }
        if reported == self.theme.dark {
            return;
        }
        self.theme = self.theme.restyled(reported);
        // The native title bar is DWM's, not GPUI's: it keeps the mode set when the window
        // opened until told otherwise. The tray icon's context menu is process-wide, not
        // per-window, but it only ever shows one theme at a time, so the last window to follow
        // the system appearance still leaves it in the right state.
        tachyon_platform::set_title_bar_dark(window, reported);
        tachyon_platform::set_popup_menu_dark(reported);
        cx.notify();
    }

    /// Applies changed settings: the theme, the rotating tip and the text column width now
    /// (zoom applies to new windows).
    pub(crate) fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_note_opacity(window, cx);
        let settings = cx.try_global::<crate::Settings>().cloned().unwrap_or_default();
        let dark = settings.dark(is_dark(window.appearance()));
        self.appearance_unconfirmed = false;
        if dark != self.theme.dark {
            self.theme = self.theme.restyled(dark);
            cx.notify();
        }
        if settings.content_width != self.content_width {
            self.content_width = settings.content_width;
            // `Editor::render` remeasures the list itself once the resolved width actually
            // changes (it also covers a plain window resize, which never reaches here); just
            // notifying is enough to get that next render.
            cx.notify();
        }
        self.set_tips_shown(settings.tips, cx);
    }

    /// Picks this window's tip the first time it runs (deferred past construction, see
    /// `with_document`), reading whether `Settings::tips` allows it directly since nothing has
    /// fetched settings yet at that point. `apply_settings` handles the same on/off switch again
    /// on every settings save, through `set_tips_shown` directly with the settings it already
    /// read, rather than calling back into this.
    pub(crate) fn refresh_tip(&mut self, cx: &mut Context<Self>) {
        let show = cx.try_global::<crate::Settings>().is_none_or(|s| s.tips);
        self.set_tips_shown(show, cx);
    }

    /// Turns the rotating tip on or off (`Settings::tips`): a tip already picked
    /// (`tips::next_tip`) keeps its text for the rest of the window's life - this never re-rolls
    /// it, only shows or hides whichever one this window already has.
    fn set_tips_shown(&mut self, show: bool, cx: &mut Context<Self>) {
        if !show {
            if self.tip.take().is_some() {
                cx.notify();
            }
        } else if self.tip.is_none() {
            self.tip = crate::tips::next_tip();
            cx.notify();
        }
    }

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
        self.flush_pending_paste(cx);
        self.editing = true;
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
        self.flush_pending_paste(cx);
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
        self.flush_pending_paste(cx);
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
            if let Some(target) = self.cross_forced_cut(base, layout.len(), head, lines) {
                return self.move_to(target, select, cx);
            }
        }
        // Leaving the laid-out block: continue by source lines.
        let target = movement::vertical(self.doc.buffer().rope(), head, lines);
        self.move_to(target, select, cx);
    }

    /// `vertical`'s fallback (leaving the active block's laid-out bounds) normally continues by
    /// real source lines (`movement::vertical`, via `rope.byte_to_line`/`line_to_byte`): correct
    /// because a block boundary is ordinarily a real line boundary too. Not so for a plain-text
    /// block split by [`tachyon_doc::PLAIN_FORCED_CUT_BYTES`] - a display-only cut in the middle
    /// of one source line with no `\n` at the boundary. There, `rope.byte_to_line` sees the whole
    /// multi-chunk line as line 0 throughout, so `movement::vertical` would jump straight to the
    /// *next real line*, skipping every remaining chunk of the current one - the reported 20 MB
    /// single-line pathology, applied to Up/Down instead of open time.
    ///
    /// Detects that case (the block being left ends, or begins, without a `\n`) and crosses into
    /// the neighboring chunk instead, at the same byte offset from its start (clamped to its
    /// length) - exact for a uniform forced cut (the common case: one very long line of similar
    /// content), and always a valid, in-bounds offset otherwise. `None` if this is an ordinary
    /// line boundary (or plain mode is not active, or there is no neighboring block), so the
    /// caller falls back to `movement::vertical` as before.
    fn cross_forced_cut(
        &self,
        base: usize,
        block_len: usize,
        head: usize,
        lines: isize,
    ) -> Option<usize> {
        if self.doc.mode() != DocMode::Plain {
            return None;
        }
        let within = head - base;
        let rope = self.doc.buffer().rope();
        if lines > 0 {
            let end = base + block_len;
            if end >= self.doc.len() || rope.byte(end - 1) == b'\n' {
                return None;
            }
            let next = self.doc.block_range(self.doc.block_at(end)?);
            Some((next.start + within).min(next.end))
        } else {
            if base == 0 || rope.byte(base - 1) == b'\n' {
                return None;
            }
            let prev = self.doc.block_range(self.doc.block_at(base - 1)?);
            Some(prev.start + within.min(prev.end - prev.start))
        }
    }

    /// Scrolls by one viewport height and moves the caret by as many source lines as fit in it.
    fn page(&mut self, direction: isize, select: bool, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
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

    pub(crate) fn update_active(&mut self) {
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

    /// Called while painting the caret (window coordinates). Scrolls so the caret's line is
    /// visible with a line of margin if a reveal is pending. While the find bar is open it floats
    /// over the top of the document without reflowing it, so its bottom edge (`find_bar_bottom`)
    /// is treated as the effective top of the viewport, keeping the revealed line below it;
    /// pickers close on Enter and need no such inset. Returns whether it scrolled.
    pub(crate) fn reveal_caret_at(&mut self, top: Pixels, line_height: Pixels) -> bool {
        if !std::mem::take(&mut self.reveal) {
            return false;
        }
        let viewport = self.list.viewport_bounds();
        let margin = line_height.min(viewport.size.height / 4.);
        let top_bound = if self.find.is_some() {
            viewport.top().max(self.find_bar_bottom + crate::render::OVERLAY_MARGIN)
        } else {
            viewport.top()
        };
        let above = top - margin - top_bound;
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

    /// Like [`Editor::replace_selection_with`], but for text prepared off the UI thread
    /// ([`PreparedInsert`]): splicing it in is O(log n) even for megabytes of text, instead of
    /// building a new rope from a `&str` on this thread (see [`tachyon_doc::Document::edit_prepared`]'s
    /// doc comment). Used by Replace All's background path (`find::dispatch_replace_all`), which
    /// already has to build the replaced text off thread and so builds the `PreparedInsert` from
    /// it there too - measured (a 200 MB plain-text log, ~1.08 million matches, the whole file
    /// rewritten in one edit) at roughly a third the UI-thread cost of
    /// [`Editor::replace_selection_with`] on the same edit (223 ms -> 72 ms worst frame).
    pub(crate) fn replace_selection_with_prepared(
        &mut self,
        range: Range<usize>,
        insert: PreparedInsert,
        cx: &mut Context<Self>,
    ) {
        self.moved_since_edit = true;
        let now = Instant::now();
        self.replace_at(now, range, Insert::Prepared(insert), cx);
        self.charge_work("edit", now);
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
        self.editing = true;
        self.schedule_backup(cx);
        self.schedule_note_autosave(cx);
        self.goal_x = None;
        self.refresh_find(cx);
        self.reparse(self.head(), cx);
        self.update_active();
        self.reveal_cursor();
        cx.notify();
    }

    /// Offset of the first block in view.
    pub(crate) fn viewport_offset(&self) -> usize {
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
        self.flush_pending_paste(cx);
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
        modifiers: gpui::Modifiers,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending_paste(cx);
        window.focus(&self.focus, cx);
        let extend = modifiers.shift;
        if modifiers.secondary() && !extend && click_count == 1 && self.follow_link(offset, cx) {
            return;
        }
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

    /// A click that reached this handler missed every block's own text - `with_mouse` stops
    /// propagation once it handles one, so this only ever runs for a margin, a gap between
    /// blocks, the window's side gutters, or the space below the document. Below the *last*
    /// block of the whole document, puts the caret at the end (like [`Editor::document_end`]);
    /// otherwise finds the block nearest the click by its on-screen bounds (`Editor::item_bounds`)
    /// and the position nearest the click inside it (`nearest_offset_in_block`), then handles it
    /// exactly like an ordinary click there. Does nothing at all if no block's bounds are known
    /// yet (nothing has ever completed a frame): there is no "nearest" to fall back to.
    pub(crate) fn mouse_down_in_margin(
        &mut self,
        position: Point<Pixels>,
        modifiers: gpui::Modifiers,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.doc.blocks().is_empty() {
            return self.mouse_down(0, modifiers, click_count, window, cx);
        }
        let mut nearest: Option<(usize, Bounds<Pixels>, Pixels)> = None;
        let mut bottom_most: Option<(usize, Pixels)> = None;
        for index in self.rendered.clone() {
            let Some(bounds) = self.item_bounds(index) else { continue };
            let distance = if position.y < bounds.top() {
                bounds.top() - position.y
            } else if position.y > bounds.bottom() {
                position.y - bounds.bottom()
            } else {
                px(0.)
            };
            if nearest.is_none_or(|(_, _, d)| distance < d) {
                nearest = Some((index, bounds, distance));
            }
            if bottom_most.is_none_or(|(_, b)| bounds.bottom() > b) {
                bottom_most = Some((index, bounds.bottom()));
            }
        }
        let Some((index, bounds, _)) = nearest else { return };
        if let Some((bottom_index, bottom)) = bottom_most
            && bottom_index + 1 == self.doc.blocks().len()
            && position.y >= bottom
        {
            return self.mouse_down(self.doc.len(), modifiers, click_count, window, cx);
        }
        let offset = self.nearest_offset_in_block(index, bounds, position, window);
        self.mouse_down(offset, modifiers, click_count, window, cx);
    }

    /// `index`'s current on-screen bounds: the last completed frame's snapshot
    /// (`window_item_bounds`, a cheap map lookup) when it already has it, else the list's own
    /// layout state directly. The snapshot lags by one frame right after `set_document` or a new
    /// window (it is taken from the *previous* frame's drawn range before this frame draws
    /// anything), so a click in the first frame or two would otherwise find nothing even though
    /// the list itself already has real bounds by then. Calling `ListState::bounds_for_item`
    /// directly is only unsafe from inside the list's own layout pass (`render_item`/
    /// `render_block`, already holding the same borrow for the frame), never from a mouse event
    /// handler like this one.
    fn item_bounds(&self, index: usize) -> Option<Bounds<Pixels>> {
        self.window_item_bounds.get(&index).copied().or_else(|| self.list.bounds_for_item(index))
    }

    /// Whether `index` currently renders as an editing card with its own inset
    /// (`render_raw_segment`'s border and padding around the source; skipped in `DocMode::Plain`,
    /// where a chunk is never drawn as a card). Matches `render_block`'s own choice, at block
    /// granularity: a container's single raw leaf, inside a list, quote or footnote, is not
    /// distinguished from the rest of that container here.
    fn has_raw_card(&self, index: usize) -> bool {
        if self.doc.mode() == DocMode::Plain {
            return false;
        }
        let Some(block) = self.doc.blocks().get(index) else { return false };
        let active = self.active_block() == Some(index);
        let leaf = if active { self.active_leaf() } else { None };
        block.is_stale() || (active && leaf.is_none())
    }

    /// The horizontal extent of `index`'s own text inside its row `bounds`: the centered content
    /// column `render_block` builds (`div().w_full().max_w(content_width).px_4()`), plus the raw
    /// editing card's own inset (`render_raw_segment`) when `has_raw_card`. Mirrors that layout
    /// without a stored one, since a non-active block's lines keep none. `window` supplies the
    /// current rem size (`px_4` is `1rem`, which follows zoom).
    fn text_area(&self, index: usize, bounds: Bounds<Pixels>, window: &Window) -> (Pixels, Pixels) {
        let rem = window.rem_size();
        let column_width = bounds.size.width.min(self.theme.content_width);
        let column_left = bounds.left() + (bounds.size.width - column_width) / 2.;
        let mut left = column_left + rem;
        let mut width = (column_width - rem * 2.).max(px(0.));
        if self.has_raw_card(index) {
            let inset = self.theme.scaled(crate::render::RAW_INSET) - px(1.);
            left += inset;
            width = (width - inset * 2.).max(px(0.));
        }
        (left, width)
    }

    /// The position closest to `position` inside block `index`, whose row is `bounds`: the
    /// source line nearest the click's y, then the visual row nearest it within that line (a
    /// long line wraps into several, estimated from `text_area`'s width and an average glyph
    /// width, the same idea `render_raw`'s own windowing uses for a segment's row count), then
    /// the character nearest the click's x within that row (the same average-glyph-width
    /// estimate). A rendered, non-active block keeps no per-glyph layout the way the active
    /// block's own [`TextLayout`] does, and shaping one just to hit-test a rare margin click
    /// would cost as much as drawing it. Walks the target line's own rope slice only up to the
    /// chosen column, never copying it, so a click beside an enormous single-line block costs
    /// only that column, not the whole line. Falls back to the block's start if it is empty
    /// (should not happen for a block `mouse_down_in_margin` already found bounds for).
    fn nearest_offset_in_block(
        &self,
        index: usize,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        window: &Window,
    ) -> usize {
        let range = self.doc.block_range(index);
        if range.is_empty() {
            return range.start;
        }
        let code = matches!(
            self.doc.blocks()[index].parsed().kind,
            BlockKind::CodeBlock { .. } | BlockKind::Html
        );
        let text_size = if code { self.theme.code_size } else { self.theme.text_size };
        let line_height = text_size * if code { 1.45 } else { 1.6 };
        let (text_left, wrap_width) = self.text_area(index, bounds, window);
        let average_char_width = text_size * 0.55;
        let chars_per_row = ((wrap_width / average_char_width).floor().max(1.)) as usize;

        let rope = self.doc.buffer().rope();
        let first_line = rope.byte_to_line(range.start);
        let last_line = rope.byte_to_line(range.end.saturating_sub(1).max(range.start));
        let relative_y = (position.y - bounds.top()).max(px(0.));

        let mut consumed = px(0.);
        let mut line_start = range.start;
        let mut line_end = range.end;
        let mut row_in_line = 0usize;
        for line in first_line..=last_line {
            let start = rope.line_to_byte(line).max(range.start);
            let mut end = rope.line_to_byte(line + 1).min(range.end);
            if end > start && rope.byte(end - 1) == b'\n' {
                end -= 1;
            }
            let rows = rope.byte_slice(start..end).len_chars().div_ceil(chars_per_row).max(1);
            let height = line_height * rows as f32;
            if relative_y < consumed + height || line == last_line {
                line_start = start;
                line_end = end;
                row_in_line = (((relative_y - consumed) / line_height) as usize).min(rows - 1);
                break;
            }
            consumed += height;
        }

        let column_in_row = ((position.x - text_left).max(px(0.)) / average_char_width) as usize;
        let line_len = rope.byte_slice(line_start..line_end).len_chars();
        let column = (row_in_line * chars_per_row + column_in_row).min(line_len);

        let mut offset = line_start;
        for ch in rope.byte_slice(line_start..line_end).chars().take(column) {
            offset += ch.len_utf8();
        }
        offset
    }

    // ---- actions --------------------------------------------------------

    pub(crate) fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.bar_open() {
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
        if !self.picker_step(-1, cx) {
            self.vertical(-1, false, cx);
        }
    }
    pub(crate) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.picker_step(1, cx) {
            self.vertical(1, false, cx);
        }
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
        // Flushed here (not just inside `move_to`): `self.doc.len()` below must see the pending
        // paste's text, not the document from before it landed.
        self.flush_pending_paste(cx);
        self.goal_x = None;
        self.move_to(self.doc.len(), false, cx);
    }
    pub(crate) fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        let next = ZOOM_STEPS.iter().find(|&&z| z > self.theme.zoom + 0.001);
        self.set_zoom(next.copied().unwrap_or(self.theme.zoom), cx);
    }
    pub(crate) fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        let next = ZOOM_STEPS.iter().rev().find(|&&z| z < self.theme.zoom - 0.001);
        self.set_zoom(next.copied().unwrap_or(self.theme.zoom), cx);
    }
    pub(crate) fn zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1., cx);
    }
    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if zoom == self.theme.zoom {
            return;
        }
        self.theme = self.theme.clone().zoomed(zoom);
        self.goal_x = None;
        self.list.remeasure();
        self.reveal_cursor();
        cx.notify();
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
        // Flushed here for the same reason as `document_end`: `self.doc.len()` below.
        self.flush_pending_paste(cx);
        self.move_to(0, false, cx);
        self.move_to(self.doc.len(), true, cx);
    }
    pub(crate) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        // Enter can continue or end a list, which is not simple to queue like plain typed text
        // (see `replace_text_in_range`): flush a pending paste first, like the edits below.
        self.flush_pending_paste(cx);
        if self.bar_open() {
            return self.find_enter(window, cx);
        }
        if self.list_newline(cx) {
            return;
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
        self.flush_pending_paste(cx);
        if self.bar_open() {
            return self.find_previous(&FindPrevious, window, cx);
        }
        self.replace(self.selection.clone(), "\n", cx);
    }
    pub(crate) fn tab(&mut self, _: &Tab, _: &mut Window, cx: &mut Context<Self>) {
        // Tab has no key_char an editor keystroke could ever queue as text (`lib::init`'s
        // intercept already flushes for it), but it also indents list items and switches
        // find-bar fields below, neither of which is simple to queue: flush explicitly too.
        self.flush_pending_paste(cx);
        if self.find_switch_field(cx) || self.list_indent(false, cx) {
            return;
        }
        self.replace(self.selection.clone(), "    ", cx);
    }
    /// Shift+Tab: outdents list items (and switches find-bar fields).
    pub(crate) fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        if !self.find_switch_field(cx) {
            self.list_indent(true, cx);
        }
    }
    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text()));
        }
    }
    /// Copies the selection (or, with none, the document) rendered as HTML: as rich text where
    /// the application can put HTML on the clipboard (`HtmlClipboard`), with the Markdown as its
    /// plain-text form; elsewhere the HTML source as text.
    pub(crate) fn copy_as_html(
        &mut self,
        _: &CopyAsHtml,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending_paste(cx);
        let markdown = if self.selection.is_empty() { self.text() } else { self.selected_text() };
        let html = tachyon_md::to_html(&markdown);
        let rich = cx.try_global::<HtmlClipboard>().map(|writer| writer.0);
        if !rich.is_some_and(|write| write(window, &html, &markdown)) {
            cx.write_to_clipboard(ClipboardItem::new_string(html));
        }
    }
    pub(crate) fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text()));
            self.replace(self.selection.clone(), "", cx);
        }
    }
    pub(crate) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        if !self.bar_open()
            && let Some(read) = cx.try_global::<ClipboardReader>().map(|reader| reader.0)
        {
            return self.paste_read_off_thread(read, cx);
        }
        let started = Instant::now();
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        self.charge_work("clipboard", started);
        let Some(text) = text else { return };
        if self.bar_open() {
            return self.find_input(&text, false, cx);
        }
        self.moved_since_edit = true;
        if text.len() <= tachyon_doc::UNPARSED_SPLIT_THRESHOLD {
            self.replace(self.selection.clone(), &text, cx);
            return;
        }
        let text: Arc<str> = text.into();
        let shared = Arc::clone(&text);
        let task = cx.spawn(async move |this, cx| prepare_paste(this, shared, cx).await);
        self.pending_paste =
            Some(PendingPaste { text: PasteText::Read(text), queued: String::new(), _task: task });
    }

    /// Pastes with the clipboard read on a background thread, then continues like `paste`.
    /// `claim` races this task against `flush_pending_paste`: whichever moves it off
    /// `CLAIM_IDLE` first is the only one that calls `read` (see the `CLAIM_*` docs above
    /// `PasteText`).
    fn paste_read_off_thread(&mut self, read: fn() -> Option<String>, cx: &mut Context<Self>) {
        self.moved_since_edit = true;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let claim = Arc::new(AtomicU8::new(CLAIM_IDLE));
        let claim_for_background = Arc::clone(&claim);
        let reading = cx.background_executor().spawn(async move {
            let won = claim_for_background
                .compare_exchange(CLAIM_IDLE, CLAIM_BACKGROUND, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
            // If this lost the race, `flush_pending_paste` already claimed the read and is doing
            // it on the UI thread right now: touching the clipboard here too is the two-thread
            // race that used to crash on Windows, so this task does nothing further.
            if won {
                let _ = sender.send(read());
            }
        });
        let task = cx.spawn(async move |this, cx| {
            reading.await;
            let text = this.update(cx, |editor, cx| editor.take_read_paste(cx)).ok().flatten();
            if let Some(text) = text {
                prepare_paste(this, text, cx).await;
            }
        });
        self.pending_paste = Some(PendingPaste {
            text: PasteText::Reading { receiver, read, claim },
            queued: String::new(),
            _task: task,
        });
    }

    /// The clipboard text read for the pending paste. Pastes it right away if it is small (and
    /// returns `None`); returns it to be prepared off the UI thread if it is large.
    fn take_read_paste(&mut self, cx: &mut Context<Self>) -> Option<Arc<str>> {
        let paste = self.pending_paste.as_mut()?;
        let PasteText::Reading { receiver, .. } = &paste.text else { return None };
        let Some(text) = receiver.try_recv().ok().flatten() else {
            let queued = self.pending_paste.take().map_or_else(String::new, |paste| paste.queued);
            self.apply_queued(queued, cx);
            return None;
        };
        if text.len() <= tachyon_doc::UNPARSED_SPLIT_THRESHOLD {
            let queued = self.pending_paste.take().map_or_else(String::new, |paste| paste.queued);
            self.replace(self.selection.clone(), &text, cx);
            self.apply_queued(queued, cx);
            return None;
        }
        let text: Arc<str> = text.into();
        paste.text = PasteText::Read(Arc::clone(&text));
        Some(text)
    }

    /// Applies text queued on a pending paste (`replace_text_in_range`) at the caret the paste
    /// left, as its own edit and so its own undo step; a no-op when nothing was queued.
    fn apply_queued(&mut self, queued: String, cx: &mut Context<Self>) {
        if !queued.is_empty() {
            self.replace(self.selection.clone(), &queued, cx);
        }
    }

    /// Whether a paste is still being read or prepared (see `flush_pending_paste`).
    pub(crate) fn paste_pending(&self) -> bool {
        self.pending_paste.is_some()
    }

    /// Applies a paste still being read or prepared, on this thread, followed by any text
    /// queued on it. Called before any other input that cannot simply queue after the paste
    /// (see `replace_text_in_range`), so edits keep their order.
    ///
    /// Claims the read itself (`CLAIM_IDLE` -> `CLAIM_UI`) if `paste_read_off_thread`'s
    /// background task has not started yet, and then reads the clipboard exactly as `paste`
    /// would have. If the background task already claimed it, this must not read a second time
    /// (see the `CLAIM_*` docs above `PasteText`): it blocks on the background task's single
    /// result instead. That block is bounded in practice, not by a timeout: the background task,
    /// once it wins the claim, does nothing but call `read` (the same synchronous read this
    /// thread would otherwise do itself), so the wait is at most `read`'s own cost (Windows:
    /// `read_clipboard_text`'s ~50 ms `OpenClipboard` deadline plus ~12 ms to copy 5 MB), and
    /// usually much less, since the background task had a head start of however long the keys
    /// took to arrive after Ctrl+V. That is cheaper than what this used to do on a lost race:
    /// read the clipboard a second time, concurrently with the background task's own read.
    pub(crate) fn flush_pending_paste(&mut self, cx: &mut Context<Self>) {
        let Some(paste) = self.pending_paste.take() else { return };
        let text: Option<Arc<str>> = match paste.text {
            PasteText::Read(text) => Some(text),
            PasteText::Reading { receiver, read, claim } => {
                let started = Instant::now();
                let won = claim
                    .compare_exchange(CLAIM_IDLE, CLAIM_UI, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok();
                let text = if won { read() } else { receiver.recv().ok().flatten() };
                self.charge_work("clipboard", started);
                text.map(Into::into)
            }
        };
        if let Some(text) = text {
            self.replace(self.selection.clone(), &text, cx);
        }
        self.apply_queued(paste.queued, cx);
    }
    pub(crate) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        if let Some(edits) = self.doc.undo() {
            self.caret_after(&edits);
            self.moved_since_edit = true;
            self.after_edit(cx);
        }
    }
    pub(crate) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
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
        if self.is_note() {
            self.close_note(window, cx);
            return;
        }
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

    /// `Ctrl+Shift+M`: switches between Markdown and plain-text display and editing, keeping the
    /// text, undo history and modified/saved state (retagging the same buffer, not reloading).
    /// Not persisted: the next launch reopens the file by its extension again. Refused, with the
    /// same notice the automatic Markdown-to-plain fallback shows, above `MARKDOWN_SIZE_LIMIT`:
    /// switching a document that large to Markdown risks the same multi-hundred-millisecond
    /// parse stall the size limit exists to avoid, and this action has no background path for it
    /// (see the doc comment on `Document::retagged`).
    pub(crate) fn toggle_text_mode(
        &mut self,
        _: &ToggleTextMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending_paste(cx);
        let target = match self.doc.mode() {
            DocMode::Plain => DocMode::Markdown,
            DocMode::Markdown => DocMode::Plain,
        };
        if target == DocMode::Markdown && self.doc.len() as u64 > tachyon_doc::MARKDOWN_SIZE_LIMIT {
            self.notice = Some(crate::disk::oversized_markdown_notice());
            cx.notify();
            return;
        }
        self.retag_to(target, cx);
        cx.notify();
    }

    /// Retags the document to `mode` (`Document::retagged`), keeping the caret at the same
    /// offset (clamped to the new length): the shared mechanics behind `toggle_text_mode` (a
    /// manual `Ctrl+Shift+M`) and `session::restore_view` (a session-recorded mode). Callers
    /// check `MARKDOWN_SIZE_LIMIT` themselves first - refusing is a per-caller decision (a
    /// notice for the manual toggle, a silent skip for a session written before the document
    /// grew) - this always retags.
    pub(crate) fn retag_to(&mut self, mode: DocMode, cx: &mut Context<Self>) {
        let caret = self.head().min(self.doc.len());
        let placeholder = std::mem::replace(&mut self.doc, Document::new_plain(""));
        self.doc = placeholder.retagged(mode);
        self.doc.take_splices();
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.active = None;
        self.active_layout = None;
        self.editing = true;
        self.notice = None;
        self.list.reset(self.doc.blocks().len());
        self.update_active();
        self.reparse(0, cx);
        self.move_to(caret, false, cx);
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_lossy_then(window, cx, |editor, window, cx| {
            editor.save_then(window, cx, |_, _| {});
        });
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_lossy_then(window, cx, |editor, window, cx| {
            editor.prompt_path_then_save(window, cx, |_, _| {});
        });
    }

    /// If this document was decoded lossily (`self.lossy`: some bytes were not valid UTF-8, or it
    /// looked binary, and were replaced with U+FFFD when it was read - see `disk::load_document`),
    /// asks before `save`/`save_as` writes it, since that replaces the original bytes for good;
    /// otherwise runs `then` right away. Uses the same in-window-or-native prompt as the
    /// unsaved-changes-on-close prompt (`should_close`) - native on Windows/macOS, in-window on
    /// Linux, chosen once at startup (`tachyon_platform::has_native_prompts`).
    ///
    /// Asked once per Save/Save As, not once per overwrite attempt: gated here, before
    /// `write_to`'s own disk-changed-on-conflict prompt, rather than inside `write_to` itself,
    /// which `confirm_overwrite` can call again for the same logical save.
    fn confirm_lossy_then(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.lossy {
            return then(self, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Save this file?",
            Some(
                "Some bytes in this file were not valid text and were replaced with \u{FFFD} \
                 when it was opened. Saving keeps the replacement text, not the original bytes.",
            ),
            &["Save Anyway", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await.ok() == Some(0) {
                let _ = this.update_in(cx, |editor, window, cx| then(editor, window, cx));
            }
        })
        .detach();
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
            Some(path) => self.write_to(path, false, window, cx, after),
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
            let _ = this.update_in(cx, |editor, window, cx| {
                editor.write_to(path, false, window, cx, after)
            });
        })
        .detach();
    }

    /// Writes the text (original line endings restored) off the UI thread. Unless `force`, asks
    /// first if the file changed on disk since it was read or last written. The rope is cloned
    /// (O(1): `ropey` shares nodes via `Arc`) and the line-ending conversion itself
    /// (`tachyon_text::saved_text`, `O(document size)`) runs on the background executor, not
    /// here, so a save of a huge document never stalls a frame building the string to write.
    fn write_to(
        &mut self,
        path: PathBuf,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        use crate::disk::DiskStamp;
        let rope = self.doc.buffer().rope().clone();
        let line_ending = self.doc.buffer().line_ending();
        let history_position = self.doc.buffer().history_position();
        // Only the file this document came from has a version to protect.
        let same_file = !force && self.file.as_ref() == Some(&path);
        if same_file && self.disk_changed {
            // Known (or possibly) changed on disk already: ask before writing anything.
            return self.confirm_overwrite(path, window, cx, after);
        }
        let known = self.disk_stamp.filter(|_| same_file);
        cx.spawn_in(window, async move |this, cx| {
            let target = path.clone();
            let written = cx
                .background_executor()
                .spawn(async move {
                    // The file is unchanged, or gone (saving recreates it).
                    let unchanged = || {
                        known.is_none_or(|known| {
                            DiskStamp::of(&target).is_none_or(|current| current == known)
                        })
                    };
                    if !unchanged() {
                        return Ok(None);
                    }
                    let text = tachyon_text::saved_text(&rope, line_ending);
                    // Checked again right before the new file replaces the old one.
                    let written = write_atomically_if(&target, text.as_bytes(), unchanged)?;
                    io::Result::Ok(written.then(|| DiskStamp::of(&target)))
                })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| match written {
                Ok(None) => editor.confirm_overwrite(path, window, cx, after),
                Ok(Some(stamp)) => {
                    editor.settings_saved(&path, cx);
                    editor.disk_stamp = stamp;
                    editor.disk_changed = false;
                    editor.file = Some(path);
                    editor.saved_history_position = history_position;
                    // The bytes now on disk are exactly what the document holds (U+FFFD in
                    // place of whatever did not decode the first time): the round trip is
                    // lossless from here on, so later saves need not ask again.
                    editor.lossy = false;
                    editor.schedule_backup(cx);
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

    /// The file changed on disk since it was read: asks before overwriting it.
    fn confirm_overwrite(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("{name} changed on disk"),
            Some("Another program changed the file since it was opened. Saving replaces its changes."),
            &["Overwrite", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await.ok() == Some(0) {
                let _ = this.update_in(cx, |editor, window, cx| {
                    editor.write_to(path, true, window, cx, after)
                });
            }
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
        let base = Font { family: theme.text_font.clone(), ..window.text_style().font() };
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
                color: theme.text.primary,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window.text_system().layout_line(sample, theme.text_size, &[run], None);
        }
    });
}

/// The first installed monospace candidate.
fn installed_code_font(window: &Window) -> Option<&'static str> {
    crate::theme::installed_font(window, tachyon_platform::monospace_font_candidates())
}

/// Opens files dropped onto an editor window. Set by the application, which owns windows; without
/// it drops are ignored.
pub struct OpenPaths(pub std::rc::Rc<OpenPathsFn>);

/// Handler type for [`OpenPaths`].
pub type OpenPathsFn = dyn Fn(Vec<PathBuf>, &mut App);

impl gpui::Global for OpenPaths {}

/// A large paste whose text is being prepared off the UI thread.
struct PendingPaste {
    text: PasteText,
    /// Plain text typed while this paste was pending (`replace_text_in_range`), queued to apply
    /// right after it lands, as its own undo step; painted only once it does (see
    /// `Editor::apply_queued`).
    queued: String,
    /// Applies the prepared text when ready; dropped (cancelled) when the
    /// paste is applied from `text` instead.
    _task: Task<()>,
}

/// States for [`PasteText::Reading`]'s `claim`: exactly one of the background task spawned by
/// `Editor::paste_read_off_thread` and `Editor::flush_pending_paste` may read the clipboard for a
/// given paste. Both race to move `claim` off `CLAIM_IDLE` with a `compare_exchange`; whichever
/// loses must not call `read` at all. That is what keeps two threads from ever being inside
/// `read_clipboard_text`'s `OpenClipboard`/`GlobalLock`/`CloseClipboard` span for the same paste
/// at once. Before this, a large paste plus a fast keystroke read the clipboard on both threads,
/// and on Windows the background read then faulted inside the `GlobalLock`ed text a few seconds
/// later (crash dumps in the Windows design check that followed #47).
const CLAIM_IDLE: u8 = 0;
const CLAIM_BACKGROUND: u8 = 1;
const CLAIM_UI: u8 = 2;

/// The text of a pending paste.
enum PasteText {
    /// Still being read from the clipboard: `claim` decides which thread performs the single
    /// read (see the `CLAIM_*` docs above); `read` is called by whichever thread wins it.
    Reading {
        receiver: std::sync::mpsc::Receiver<Option<String>>,
        read: fn() -> Option<String>,
        claim: Arc<AtomicU8>,
    },
    Read(Arc<str>),
}

/// Normalizing, building the rope and pre-segmenting megabytes take milliseconds: does that off
/// the UI thread, then splices the result in, unless the paste was applied meanwhile.
async fn prepare_paste(this: gpui::WeakEntity<Editor>, text: Arc<str>, cx: &mut gpui::AsyncApp) {
    let insert = cx.background_executor().spawn(async move { PreparedInsert::new(&text) }).await;
    let _ = this.update(cx, |editor, cx| {
        if let Some(pending) = editor.pending_paste.take() {
            let now = Instant::now();
            editor.replace_at(now, editor.selection.clone(), Insert::Prepared(insert), cx);
            editor.charge_work("paste", now);
            editor.apply_queued(pending.queued, cx);
        }
    });
}

/// Puts HTML on the clipboard as rich text, with a plain-text form; returns whether it did. Set by
/// the application (`tachyon_platform::write_clipboard_html`); without it, or when it fails, Copy
/// as HTML copies the HTML source as text.
pub struct HtmlClipboard(pub fn(&Window, &str, &str) -> bool);

impl gpui::Global for HtmlClipboard {}

/// Reads the clipboard's text off the UI thread. Set by the application where the platform
/// allows that (`tachyon_platform::clipboard_text_reader`); without it the clipboard is read
/// through GPUI on the UI thread, which takes ≈ 12 ms for 5 MB on Windows. Never called from two
/// threads at once for the same paste: see the `CLAIM_*` docs above `PasteText`.
pub struct ClipboardReader(pub fn() -> Option<String>);

impl gpui::Global for ClipboardReader {}

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
    write_atomically_if(path, bytes, || true).map(drop)
}

/// Like [`write_atomically`], but asks `still_ok` right before the new file replaces the old one
/// and leaves the old one if it says no (returning `false`): the latest point a concurrent change
/// by another program can be noticed.
pub(crate) fn write_atomically_if(
    path: &Path,
    bytes: &[u8],
    still_ok: impl FnOnce() -> bool,
) -> io::Result<bool> {
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
        if !still_ok() {
            return Ok(false);
        }
        std::fs::rename(&temp, &path).map(|()| true)
    })();
    if !matches!(result, Ok(true)) {
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
        if self.bar_open() {
            self.flush_pending_paste(cx);
            return self.find_input(text, false, cx);
        }
        // A plain committed character (or string) at the current selection: not an IME
        // composition (`marked` is `None`) or an explicit-range replace, so it is the common
        // case of typing right after Ctrl+V. Queue it on the pending paste instead of waiting
        // for its clipboard read and applying the whole paste synchronously on this keystroke's
        // frame; it is painted once the paste lands, right after it, as its own undo step (see
        // `apply_queued`).
        if range_utf16.is_none()
            && self.marked.is_none()
            && let Some(pending) = self.pending_paste.as_mut()
        {
            pending.queued.push_str(text);
            return;
        }
        self.flush_pending_paste(cx);
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
        if self.bar_open() {
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
