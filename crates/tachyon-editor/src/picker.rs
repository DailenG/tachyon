//! Pickers: a bar with a filter and a list to choose from. Go to heading (`Ctrl+Shift+O`) lists
//! the document's headings; Open recent (`Ctrl+R`/`Ctrl+P`) lists recently opened files; the
//! command palette (`Ctrl+Shift+P`, see `command_palette.rs`) lists every user-facing command.
//! Typing filters (case-insensitive substring or, failing that, subsequence, closest matches
//! first), Up / Down choose, Enter or a click picks, Escape closes. Input is routed here through
//! the find bar's entry points while a picker is open.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::{App, Context, Global, Window};
use tachyon_md::BlockKind;

use crate::command_palette::CommandEffect;
use crate::editor::{Editor, GoToHeading, OpenPaths, OpenRecent};

/// Rows shown at once; the list scrolls to keep the chosen one in view.
pub(crate) const VISIBLE_ROWS: usize = 12;

/// What picking a row does.
pub(crate) enum Pick {
    /// Moves the caret to this source offset.
    Offset(usize),
    /// Opens this file (in a new window, through [`OpenPaths`]).
    File(PathBuf),
    /// Runs a command palette entry.
    Command(CommandEffect),
}

pub(crate) struct Item {
    pub(crate) label: String,
    /// Shown muted after the label (a file's folder).
    pub(crate) detail: Option<String>,
    /// Indent level (a heading's level).
    pub(crate) indent: u8,
    /// Shown right-aligned after `detail` (a command's current keyboard shortcut).
    pub(crate) shortcut: Option<String>,
    pub(crate) pick: Pick,
}

pub(crate) struct Picker {
    pub(crate) title: &'static str,
    /// Shown when there is nothing to pick at all.
    pub(crate) empty: &'static str,
    pub(crate) query: String,
    /// Bytes at the end of `query` that are an uncommitted IME composition.
    composing: usize,
    pub(crate) items: Vec<Item>,
    /// Indices into `items` that match the query, in order.
    pub(crate) matches: Vec<usize>,
    /// Index into `matches` of the chosen row.
    pub(crate) selected: usize,
}

impl Picker {
    pub(crate) fn new(
        title: &'static str,
        empty: &'static str,
        items: Vec<Item>,
        selected: usize,
    ) -> Self {
        let mut picker = Picker {
            title,
            empty,
            query: String::new(),
            composing: 0,
            items,
            matches: Vec::new(),
            selected,
        };
        picker.filter();
        picker
    }

    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(usize, (u8, usize, usize))> = (0..self.items.len())
            .filter_map(|i| {
                let item = &self.items[i];
                let haystack = match &item.detail {
                    Some(detail) => format!("{} {detail}", item.label),
                    None => item.label.clone(),
                };
                fuzzy_rank(&haystack, &query).map(|rank| (i, rank))
            })
            .collect();
        ranked.sort_by_key(|&(_, rank)| rank);
        self.matches = ranked.into_iter().map(|(i, _)| i).collect();
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }

    /// The `rows` in view (at least one): indices into `matches`, keeping `selected` visible.
    pub(crate) fn window(&self, rows: usize) -> std::ops::Range<usize> {
        let rows = rows.max(1);
        let first = self.selected.saturating_sub(rows - 1);
        first..(first + rows).min(self.matches.len())
    }
}

/// Ranks `query_lower` (already lowercased) against `haystack` for the picker's filter: `None`
/// if it does not match at all. Lower sorts first. A case-insensitive substring match beats a
/// subsequence match (every query character appears in order, not necessarily contiguous, so
/// abbreviations like "tpm" still find "Toggle plain-text mode"); within each kind, a match
/// starting right at a word boundary beats one in the middle of a word, then an earlier or
/// tighter match beats a later or looser one, then a shorter haystack beats a longer one (a more
/// specific match). An empty query matches everything, tied, so a stable sort keeps the
/// original order (document order for headings, most-recent-first for files).
fn fuzzy_rank(haystack: &str, query_lower: &str) -> Option<(u8, usize, usize)> {
    if query_lower.is_empty() {
        return Some((0, 0, 0));
    }
    let hay_lower = haystack.to_lowercase();
    if let Some(pos) = hay_lower.find(query_lower) {
        let at_word_start = pos == 0 || !hay_lower.as_bytes()[pos - 1].is_ascii_alphanumeric();
        return Some((if at_word_start { 0 } else { 1 }, pos, hay_lower.len()));
    }
    let mut rest = hay_lower.chars();
    let mut skipped = 0usize;
    for q in query_lower.chars() {
        loop {
            match rest.next() {
                Some(c) if c == q => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
    }
    Some((2, skipped, hay_lower.len()))
}

/// Recently opened files, most recent first, kept in a file (one path per line). Set by the
/// application; without it Open recent lists nothing.
pub struct RecentFiles {
    path: PathBuf,
    /// Serializes read-modify-write updates from several windows.
    lock: Arc<Mutex<()>>,
}

impl Global for RecentFiles {}

/// How many files Open recent remembers.
const RECENT_LIMIT: usize = 30;

impl RecentFiles {
    pub fn new(path: PathBuf) -> Self {
        RecentFiles { path, lock: Arc::new(Mutex::new(())) }
    }

    fn read(path: &Path) -> Vec<PathBuf> {
        std::fs::read_to_string(path)
            .map(|text| text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default()
    }

    /// Moves `file` to the front of the list, off the UI thread.
    pub(crate) fn note(file: &Path, cx: &App) {
        let Some(recent) = cx.try_global::<RecentFiles>() else { return };
        let Some(entry) = file.to_str().map(str::to_owned) else { return };
        let (path, lock) = (recent.path.clone(), Arc::clone(&recent.lock));
        cx.background_executor()
            .spawn(async move {
                let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut files = Self::read(&path);
                files.retain(|f| f.to_str() != Some(entry.as_str()));
                files.insert(0, PathBuf::from(&entry));
                files.truncate(RECENT_LIMIT);
                let text: String =
                    files.iter().filter_map(|f| f.to_str()).map(|f| format!("{f}\n")).collect();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = crate::editor::write_atomically(&path, text.as_bytes());
            })
            .detach();
    }
}

impl Editor {
    pub(crate) fn go_to_heading(
        &mut self,
        _: &GoToHeading,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caret = self.selection.start;
        let heading_indices: Vec<(usize, u8)> = self
            .doc
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(index, block)| match block.parsed().kind {
                BlockKind::Heading(level) => Some((index, level)),
                _ => None,
            })
            .collect();
        let mut items = Vec::with_capacity(heading_indices.len());
        for (index, level) in heading_indices {
            // A heading's own `ir` may have been dropped if it was far from the viewport
            // (`Document::evict`, report fix 6); restore it here, since the picker needs every
            // heading's text document-wide, regardless of scroll position.
            self.doc.ensure_ir(index);
            let ir = &self.doc.blocks()[index].parsed().ir;
            let start = self.doc.block_range(index).start;
            items.push(Item {
                label: ir.text.lines().next().unwrap_or("").trim().to_owned(),
                detail: None,
                indent: level.saturating_sub(1),
                shortcut: None,
                pick: Pick::Offset(start + ir.visible_to_source(0)),
            });
        }
        // Start at the heading the caret is under.
        let selected = items
            .iter()
            .rposition(|item| matches!(item.pick, Pick::Offset(offset) if offset <= caret))
            .unwrap_or(0);
        self.open_picker(Picker::new("Go to heading", "no headings", items, selected), cx);
    }

    pub(crate) fn open_recent(&mut self, _: &OpenRecent, _: &mut Window, cx: &mut Context<Self>) {
        let files =
            cx.try_global::<RecentFiles>().map(|r| RecentFiles::read(&r.path)).unwrap_or_default();
        let items = files
            .into_iter()
            .filter(|file| self.file.as_deref() != Some(file.as_path()))
            .map(|file| Item {
                label: file.file_name().map_or_else(
                    || file.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
                detail: file.parent().map(|dir| dir.display().to_string()),
                indent: 0,
                shortcut: None,
                pick: Pick::File(file),
            })
            .collect();
        self.open_picker(Picker::new("Open recent", "no recent files", items, 0), cx);
    }

    pub(crate) fn open_picker(&mut self, picker: Picker, cx: &mut Context<Self>) {
        self.find = None;
        self.picker = Some(picker);
        cx.notify();
    }

    pub(crate) fn picker_input(&mut self, text: &str, composing: bool, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else { return };
        picker.query.truncate(picker.query.len() - picker.composing);
        let text = text.lines().next().unwrap_or("");
        picker.query.push_str(text);
        picker.composing = if composing { text.len() } else { 0 };
        picker.selected = 0;
        picker.filter();
        cx.notify();
    }

    pub(crate) fn picker_backspace(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else { return };
        picker.composing = 0;
        picker.query.pop();
        picker.filter();
        cx.notify();
    }

    pub(crate) fn picker_end_composition(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.composing = 0;
        }
    }

    /// Up / Down while a picker is open: moves the choice, wrapping. Returns whether it did.
    pub(crate) fn picker_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(picker) = &mut self.picker else { return false };
        let n = picker.matches.len();
        if n > 0 {
            picker.selected = (picker.selected as isize + delta).rem_euclid(n as isize) as usize;
            cx.notify();
        }
        true
    }

    /// Picks the chosen row (or match `row`) and closes the picker.
    pub(crate) fn picker_pick(
        &mut self,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.picker.take() else { return };
        let row = row.unwrap_or(picker.selected);
        let Some(&index) = picker.matches.get(row) else {
            cx.notify();
            return;
        };
        match &picker.items[index].pick {
            Pick::Offset(offset) => self.move_to(*offset, false, cx),
            Pick::File(file) => {
                let file = file.clone();
                cx.defer(move |cx: &mut App| {
                    if let Some(open) = cx.try_global::<OpenPaths>().map(|o| o.0.clone()) {
                        open(vec![file], cx);
                    }
                });
            }
            Pick::Command(effect) => self.run_command(effect, window, cx),
        }
        cx.notify();
    }
}
