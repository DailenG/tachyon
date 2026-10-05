//! Sticky notes (issue #69): quick, autosaved Markdown files in
//! `<documents_dir>/tachyon/notes/`, opened as compact always-on-top windows. This module holds
//! the parts that do not need a live window - naming a note from its own text, and the small
//! state file (`state_dir()/notes-<instance>.txt`) that remembers which notes were open, where,
//! and pinned - so both are unit-testable with no GPUI involved, the same split `backup.rs` and
//! `session.rs` use for their own formats. The window itself (the compact header in `render.rs`,
//! window creation and restore in `crates/tachyon/src/app.rs`) and the per-editor autosave,
//! close-without-asking and pin-toggle behavior below build on top of it.
//!
//! Notes are deliberately outside both existing persistence mechanisms: `session::Editor::
//! session_state` returns `None` for one (never in the regular session file), and
//! `backup::Editor::schedule_backup`/`backup_for_session_end` are no-ops for one (never a
//! hot-exit backup) - autosave here replaces both.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{App, Bounds, Context, Global, Pixels, Task, Window, bounds, point, px, size};

use crate::editor::{Editor, ToggleNotePin, write_atomically};
use crate::session::next_field;

/// The notes folder (`<documents_dir>/tachyon/notes/`), set by the application from
/// `tachyon_platform::documents_dir()`. A `Global` (rather than a compiled-in path) so a test can
/// point it at a temporary directory instead of the real one.
pub struct NotesDir(pub PathBuf);

impl Global for NotesDir {}

/// Called (via `Context::defer`, so it never runs inside another window's own update - the same
/// reason `crates/tachyon/src/app.rs`'s Quit action defers its own work) after a note is named
/// for the first time, goes back to empty, or its pin toggles: anything the notes state file
/// needs to catch up to that a plain window move/close/open does not already cover on its own.
/// Set once by the application to its own `refresh_notes_state`, the same fn-pointer-global
/// bridge `settings::RestartRegistration` uses to reach a Windows-only (or here, app-level) side
/// effect with no reverse crate dependency.
#[derive(Clone, Copy)]
pub struct NotesChanged(pub fn(&mut App));

impl Global for NotesChanged {}

fn notify_notes_changed(cx: &mut Context<Editor>) {
    if let Some(hook) = cx.try_global::<NotesChanged>().copied() {
        cx.defer(move |cx| (hook.0)(cx));
    }
}

/// A note's own state: whether it is pinned always-on-top, and the debounced autosave in flight.
/// `Editor::note` is `Some` only for a sticky note window (`Editor::make_note`); an ordinary
/// window's is `None` and every method below is then a no-op.
pub(crate) struct NoteState {
    pub(crate) pinned: bool,
    /// A file being reopened, before the background read associates it with `Editor::file`.
    /// Never used for autosave or deletion if the read fails or the window closes early.
    pending_file: Option<PathBuf>,
    autosave_task: Option<Task<()>>,
    /// The buffer version last written to disk (or deliberately not, because it was empty at
    /// the time) - guards against scheduling another write for text already saved, mirroring
    /// `Editor::backed_up_version`.
    saved_version: Option<u64>,
}

// ---------------------------------------------------------------------------------------------
// Naming: the first heading or line, sanitized, cut to length, with a timestamp fallback and a
// uniqueness suffix. Pure functions, unit-tested below with no filesystem or GPUI involved.
// ---------------------------------------------------------------------------------------------

/// How many characters of the sanitized title a note's file name keeps: a heading pasted from
/// elsewhere can be arbitrarily long, and a file name that long is unwieldy on every platform
/// this ships to.
const MAX_NAME_LEN: usize = 60;

/// The text a note's file name comes from: the first ATX heading (`#` through `######`,
/// anywhere in the text - convention puts one at the top, but nothing requires it) that has real
/// text after its marker, else the first non-empty line. Markdown markers are then stripped
/// (`strip_markdown_markers`); making the result safe as a file name happens separately
/// (`sanitize_component`), so this alone is not yet one.
pub fn title_from_text(text: &str) -> String {
    let mut first_nonempty: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if first_nonempty.is_none() {
            first_nonempty = Some(trimmed);
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with([' ', '\t']) {
            let heading = trimmed[hashes..].trim().trim_end_matches('#').trim();
            if !heading.is_empty() {
                return strip_markdown_markers(heading);
            }
        }
    }
    strip_markdown_markers(first_nonempty.unwrap_or(""))
}

/// Strips the Markdown syntax a title-worthy line is likely to carry: a leading block marker
/// (block quote, bullet or ordered-list item), any leading `#`s left over (a line that looked
/// like a heading but had no text after it falls through to here as plain text), and inline
/// emphasis/code markers (`*`, `_`, `` ` ``) - removed outright rather than just trimmed, since
/// they can appear anywhere in the line (`**bold** word` becomes `bold word`).
fn strip_markdown_markers(line: &str) -> String {
    let mut rest = line.trim();
    loop {
        let after_quote = rest.trim_start_matches('>').trim_start();
        if let Some(bullet) = after_quote
            .strip_prefix("- ")
            .or_else(|| after_quote.strip_prefix("* "))
            .or_else(|| after_quote.strip_prefix("+ "))
        {
            rest = bullet.trim_start();
            continue;
        }
        let digits = after_quote.chars().take_while(char::is_ascii_digit).count();
        if digits > 0
            && let Some(marker) = after_quote[digits..].chars().next()
            && (marker == '.' || marker == ')')
            && after_quote[digits + marker.len_utf8()..].starts_with(' ')
        {
            rest = after_quote[digits + marker.len_utf8()..].trim_start();
            continue;
        }
        rest = after_quote;
        break;
    }
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    if hashes > 0 {
        rest = rest[hashes..].trim_start();
    }
    rest.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect::<String>().trim().to_owned()
}

/// Windows' reserved (invalid-in-a-path) characters; the sanitizer replaces each with a space so
/// words stay separated rather than jamming together.
const WINDOWS_INVALID_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Windows' reserved device names, matched case-insensitively against the whole component (a
/// document literally titled "CON" is rare but real; "Contacts" is unaffected).
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file-name component safe on Windows (and harmless elsewhere): invalid characters replaced
/// with a space each, then collapsed (so "a/b" becomes "a b", not "a  b"); trailing dots and
/// spaces removed (Windows silently drops them, which could otherwise make two different titles
/// collide); a reserved device name rejected outright, returning empty - the same as a title
/// that is nothing *but* invalid characters - so `base_name` falls back to its timestamp form
/// for either.
pub fn sanitize_component(raw: &str) -> String {
    let replaced: String =
        raw.chars().map(|c| if WINDOWS_INVALID_CHARS.contains(&c) { ' ' } else { c }).collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches(['.', ' ']).trim_start();
    if RESERVED_NAMES.iter().any(|name| name.eq_ignore_ascii_case(trimmed)) {
        return String::new();
    }
    trimmed.to_owned()
}

/// Cuts `s` to at most `max` bytes at a char boundary (never splitting a multi-byte character),
/// trimming any trailing space the cut exposes.
pub fn cut_to_length(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end().to_owned()
}

/// `note-YYYYMMDD-HHMMSS` in local time: the fallback name for a note with nothing usable to
/// title itself from (empty, or a title that sanitizes away to nothing).
pub fn timestamp_name(now: chrono::DateTime<chrono::Local>) -> String {
    format!("note-{}", now.format("%Y%m%d-%H%M%S"))
}

/// The note's file name (without extension or uniqueness suffix - see `unique_name`): its first
/// heading or line, stripped and sanitized, cut to `MAX_NAME_LEN` characters at a char boundary -
/// or, if that leaves nothing (an empty document, or a title that sanitizes away entirely, like
/// a lone "CON" or a line of only slashes), a timestamp.
pub fn base_name(text: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let sanitized = sanitize_component(&title_from_text(text));
    let cut = cut_to_length(&sanitized, MAX_NAME_LEN);
    if cut.is_empty() { timestamp_name(now) } else { cut }
}

/// `base`, or `base (2)`, `base (3)`, ... - the first for which `exists` answers false. `exists`
/// is a plain predicate (not a directory listing itself), so this stays pure and testable; the
/// real caller (`write_note_to_disk`) checks it against a set of the notes folder's current file
/// stems.
pub fn unique_name(exists: impl Fn(&str) -> bool, base: &str) -> String {
    if !exists(base) {
        return base.to_owned();
    }
    for n in 2..10_000 {
        let candidate = format!("{base} ({n})");
        if !exists(&candidate) {
            return candidate;
        }
    }
    // Never reached in practice (it would mean 9,999 identically-named notes already exist);
    // still never loops forever or panics against a hostile or corrupt folder.
    format!("{base} ({})", std::process::id())
}

/// One `.md` file in the notes folder, for the reopen picker. Newest `modified` first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredNote {
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
}

/// `.md` files in `dir`, newest first. A missing folder is an empty list, not an error: the
/// picker says "no notes". Other directory errors reach the picker. Reads names and modification
/// times only, never file contents.
pub fn stored_notes(dir: &Path) -> std::io::Result<Vec<StoredNote>> {
    let mut notes = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(notes),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(ext) = path.extension() else { continue };
        if !ext.eq_ignore_ascii_case("md") {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_file() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        notes.push(StoredNote { path, modified });
    }
    notes.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    Ok(notes)
}
/// `age` as a short label for a picker row. Pure, so the boundaries are tested without the clock.
pub fn relative_age(age: std::time::Duration) -> String {
    let secs = age.as_secs();
    match secs {
        0..60 => "just now".to_owned(),
        60..3_600 => format!("{} min ago", secs / 60),
        3_600..86_400 => format!("{} hr ago", secs / 3_600),
        86_400..172_800 => "yesterday".to_owned(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

// ---------------------------------------------------------------------------------------------
// Writing a note's file: naming it once, rewriting it after, deleting it when it goes empty.
// ---------------------------------------------------------------------------------------------

/// The file stems (names without `.md`) already present in `dir` - what `unique_name` checks
/// against when a note is named for the first time. An unreadable or not-yet-created directory
/// answers empty (the caller is about to create it, for the first note ever saved).
fn existing_note_names(dir: &Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return HashSet::new() };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

/// What writing a note's current text to disk did.
pub(crate) enum NoteWrite {
    /// Empty text: nothing is written; `existing`, if any, was deleted.
    Empty,
    Saved(PathBuf),
    /// The write (or the delete, or creating the notes folder) failed.
    Failed,
}

/// Writes `text` for a note whose file is `existing` (`None` before its first save). Empty text
/// (whitespace-only counts as empty) deletes `existing` instead of writing - and never creates
/// the notes folder in the first place if nobody ever put text in this note. A first save
/// (`existing` is `None`) names the note (`base_name`, made unique against `dir`'s current
/// contents) and creates `dir` if this is the very first note saved at all; a later save keeps
/// the same path - the name "stays fixed", per the product decision - and just rewrites it.
pub(crate) fn write_note_to_disk(dir: &Path, existing: Option<&Path>, text: &str) -> NoteWrite {
    if text.trim().is_empty() {
        if let Some(path) = existing
            && let Err(e) = std::fs::remove_file(path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return NoteWrite::Failed;
        }
        return NoteWrite::Empty;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return NoteWrite::Failed;
    }
    let path = match existing {
        Some(path) => path.to_path_buf(),
        None => {
            let names = existing_note_names(dir);
            let base = base_name(text, chrono::Local::now());
            let name = unique_name(|candidate| names.contains(candidate), &base);
            dir.join(format!("{name}.md"))
        }
    };
    match write_atomically(&path, text.as_bytes()) {
        Ok(()) => NoteWrite::Saved(path),
        Err(_) => NoteWrite::Failed,
    }
}

// ---------------------------------------------------------------------------------------------
// The notes state file (`state_dir()/notes-<instance>.txt`): which notes are open, where, and
// pinned. Same hand-rolled format as `session.rs` (`version = 1`, then NUL-separated records,
// the path last in each record since it is the one field that may itself contain '\n') - a
// separate file because notes are explicitly outside the regular session (ADR 0009): they are
// not hot-exit backups, and are not part of what the last document window closing remembers.
// ---------------------------------------------------------------------------------------------

const NOTES_VERSION: u32 = 1;

/// However many notes the state file ever records or restores - the same generous, hostile-input
/// ceiling `session::MAX_SESSION_WINDOWS` uses for the regular session file.
pub const MAX_NOTES: usize = 100;

/// Above this, the file is treated as corrupt without even trying to parse it (see
/// `session::read_session`'s identical reasoning: a few hundred bytes per note is the expected
/// size, so anything past a generous multiple of `MAX_NOTES` can only be damage).
const MAX_NOTES_FILE_BYTES: u64 = 1_000_000;

/// One open note: enough to reopen it in the same place, pinned the same way.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteWindowState {
    pub path: PathBuf,
    pub bounds: Bounds<Pixels>,
    pub pinned: bool,
}

fn serialize_note(note: &NoteWindowState) -> String {
    format!(
        "x = {}\n\
         y = {}\n\
         w = {}\n\
         h = {}\n\
         pinned = {}\n\
         path = {}",
        f32::from(note.bounds.origin.x),
        f32::from(note.bounds.origin.y),
        f32::from(note.bounds.size.width),
        f32::from(note.bounds.size.height),
        note.pinned,
        note.path.display(),
    )
}

fn parse_note(record: &str) -> Option<NoteWindowState> {
    // 6 fields; `splitn` keeps any embedded '\n' in the last one (`path`) intact - see the
    // module doc comment.
    let mut fields = record.splitn(6, '\n');
    let x: f32 = next_field(&mut fields, "x")?.parse().ok()?;
    let y: f32 = next_field(&mut fields, "y")?.parse().ok()?;
    let w: f32 = next_field(&mut fields, "w")?.parse().ok()?;
    let h: f32 = next_field(&mut fields, "h")?.parse().ok()?;
    let pinned: bool = next_field(&mut fields, "pinned")?.parse().ok()?;
    let path = fields.next()?.strip_prefix("path")?.trim_start().strip_prefix('=')?.trim_start();
    if path.is_empty() || !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. {
        return None;
    }
    Some(NoteWindowState {
        path: PathBuf::from(path),
        bounds: bounds(point(px(x), px(y)), size(px(w), px(h))),
        pinned,
    })
}

/// Writes `notes` to `path` atomically (see `write_atomically`), truncated to `MAX_NOTES`.
pub fn write_notes_state(path: &Path, notes: &[NoteWindowState]) -> std::io::Result<()> {
    let notes = &notes[..notes.len().min(MAX_NOTES)];
    // The state folder may not exist yet: a first start at login that opens only notes creates
    // nothing else there first.
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = format!("version = {NOTES_VERSION}\n");
    for note in notes {
        out.push('\0');
        out.push_str(&serialize_note(note));
    }
    write_atomically(path, out.as_bytes())
}

fn parse_notes_state(text: &str) -> Vec<NoteWindowState> {
    let mut records = text.split('\0');
    let header = records.next().unwrap_or("");
    let version = header.lines().find_map(|line| {
        line.strip_prefix("version =").map(str::trim).and_then(|v| v.parse().ok())
    });
    if version != Some(NOTES_VERSION) {
        return Vec::new();
    }
    records.filter_map(parse_note).take(MAX_NOTES).collect()
}

/// Reads and parses the notes state file. The same "never panic on user input" degradation as
/// `session::read_session`: a missing, oversized, non-UTF-8, wrong-version or malformed file all
/// become "no notes" (or, for one bad record among otherwise good ones, just that record
/// dropped), never an error.
pub fn read_notes_state(path: &Path) -> Vec<NoteWindowState> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_NOTES_FILE_BYTES => {}
        _ => return Vec::new(),
    }
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse_notes_state(&text)
}

/// The recorded notes whose file still exists, in the same order - what the application's
/// restore actually reopens. A note deleted or moved since by hand is silently skipped, the same
/// way a session-recorded hot-exit backup id nobody can find any more is (`app::restore_sources`).
pub fn existing_notes(notes: Vec<NoteWindowState>) -> Vec<NoteWindowState> {
    notes.into_iter().filter(|note| note.path.exists()).collect()
}

// ---------------------------------------------------------------------------------------------
// Per-editor behavior: making a window a note, autosaving it, closing it without a prompt,
// toggling its pin. The window itself (bounds, chrome, restore) is
// `crates/tachyon/src/app.rs`'s job; this is everything that only needs the document and its
// own file.
// ---------------------------------------------------------------------------------------------

impl Editor {
    /// Marks this window as a sticky note: autosave replaces hot exit's own backup
    /// (`backup::Editor::schedule_backup`'s own guard), closing never prompts
    /// (`Editor::close_window`'s branch), and it is left out of the ordinary session file
    /// (`session::Editor::session_state`'s own guard). Meant to be called once, right after the
    /// editor is created (`crates/tachyon/src/app.rs`'s `open_note_window`), before its first
    /// frame - the same convention `Editor::set_title_override` follows.
    pub fn make_note(&mut self, pinned: bool) {
        self.note = Some(NoteState {
            pinned,
            pending_file: None,
            autosave_task: None,
            saved_version: None,
        });
    }

    /// Reserves this file identity while the background read runs, without assigning a save path.
    pub fn reserve_note_file(&mut self, path: PathBuf) {
        if let Some(note) = self.note.as_mut() {
            note.pending_file = Some(path);
        }
    }

    /// Releases the reservation if a read failed; the blank note stays unnamed.
    pub fn clear_pending_note_file(&mut self) {
        if let Some(note) = self.note.as_mut() {
            note.pending_file = None;
        }
    }

    /// Applies the note's pin to its OS window (`tachyon_platform::set_always_on_top`) and
    /// returns whether the window now matches it; `true` for an ordinary window or where the
    /// platform has no always-on-top. Called after the note's first frame (retried from there
    /// until it sticks, see `open_note_window`) and on every activation change.
    pub fn apply_note_pin(&self, window: &Window) -> bool {
        if !tachyon_platform::supports_always_on_top() {
            return true;
        }
        let Some(note) = self.note.as_ref() else { return true };
        tachyon_platform::set_always_on_top(window, note.pinned)
    }

    /// A sticky note's whole-window translucency where the OS provides it (Windows; see
    /// `tachyon_platform::supports_window_opacity`): `sticky_unfocused_opacity` while the window
    /// is not active, opaque while it is. Called when activation changes, when settings are saved,
    /// and after the note's first frame. Elsewhere `render` draws the content translucent instead.
    pub fn apply_note_opacity(&self, window: &Window, cx: &App) {
        if !self.is_note() || !tachyon_platform::supports_window_opacity() {
            return;
        }
        let unfocused = cx
            .try_global::<crate::settings::Settings>()
            .map_or(1.0, |settings| settings.sticky_unfocused_opacity);
        let opacity = if window.is_window_active() { 1.0 } else { unfocused };
        tachyon_platform::set_window_opacity(window, opacity);
    }

    pub fn is_note(&self) -> bool {
        self.note.is_some()
    }

    /// Whether this window owns `path`, including while its background read is in flight.
    pub fn is_note_file(&self, path: &Path) -> bool {
        self.note.as_ref().is_some_and(|note| {
            self.file.as_deref() == Some(path) || note.pending_file.as_deref() == Some(path)
        })
    }

    pub fn note_pinned(&self) -> bool {
        self.note.as_ref().is_some_and(|note| note.pinned)
    }

    /// This note's record for the notes state file, or `None` if it has never been named (an
    /// empty note with no first save yet) - nothing worth remembering, the same way an untouched
    /// scratch window is skipped by `session::Editor::session_state`.
    pub fn note_window_state(&self) -> Option<NoteWindowState> {
        self.note.as_ref()?;
        let path = self.file.clone()?;
        let bounds = self.last_placement.map_or_else(Bounds::default, |(bounds, _)| bounds);
        Some(NoteWindowState { path, bounds, pinned: self.note_pinned() })
    }

    /// `Ctrl+Shift+T`: flips this note's always-on-top state - a no-op, like the header's own
    /// pin button not appearing, for an ordinary window (`self.note` is `None`) - and applies it
    /// to the real OS window right away (`tachyon_platform::set_always_on_top`), unlike the pin
    /// *record* in the notes state file, which only catches up through the usual refresh hook.
    pub(crate) fn toggle_note_pin(
        &mut self,
        _: &ToggleNotePin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(note) = self.note.as_mut() else { return };
        note.pinned = !note.pinned;
        let pinned = note.pinned;
        tachyon_platform::set_always_on_top(window, pinned);
        notify_notes_changed(cx);
        cx.notify();
    }

    /// After an edit, on the same debounce hot exit's own backup uses (`backup::BACKUP_DELAY`):
    /// writes this note's file once typing pauses. A no-op for an ordinary window (`self.note`
    /// is `None`) or once nothing has changed since the last write.
    pub(crate) fn schedule_note_autosave(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.note.as_ref() else { return };
        if note.autosave_task.is_some() {
            return;
        }
        let version = self.doc.buffer().version();
        if note.saved_version == Some(version) {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::backup::BACKUP_DELAY).await;
            let _ = this.update(cx, |editor, cx| editor.write_note_after_pause(cx));
        });
        if let Some(note) = self.note.as_mut() {
            note.autosave_task = Some(task);
        }
    }

    fn write_note_after_pause(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.note.as_mut() else { return };
        note.autosave_task = None;
        let version = self.doc.buffer().version();
        if note.saved_version == Some(version) {
            return;
        }
        self.flush_note(cx);
    }

    /// Writes (or deletes, if it has gone empty) this note's file right now, synchronously -
    /// unlike hot exit's own backup, this never defers the string build to a background task (a
    /// note is always small), and it also runs from places a background task cannot outlive
    /// (`Editor::close_note`; `crates/tachyon/src/app.rs`'s Quit and session-end paths). Returns
    /// whether the note ended up safe (written, deleted, or nothing to do) - `false` only if a
    /// write or delete was attempted and failed, mirroring
    /// `backup::Editor::backup_for_session_end`.
    pub fn flush_note(&mut self, cx: &mut Context<Self>) -> bool {
        if self.note.is_none() {
            return true;
        }
        if let Some(note) = self.note.as_mut() {
            note.autosave_task = None;
        }
        let Some(dir) = cx.try_global::<NotesDir>().map(|dir| dir.0.clone()) else {
            // Nowhere to save: `documents_dir()` failed at start-up. Not this note's fault, and
            // there is nothing more it can do about it.
            return true;
        };
        let version = self.doc.buffer().version();
        let text = self.doc.buffer().to_saved_text();
        let existing = self.file.clone();
        match write_note_to_disk(&dir, existing.as_deref(), &text) {
            NoteWrite::Empty => {
                let had_file = self.file.take().is_some();
                self.disk_stamp = None;
                if let Some(note) = self.note.as_mut() {
                    note.saved_version = Some(version);
                }
                if had_file {
                    notify_notes_changed(cx);
                }
                cx.notify();
                true
            }
            NoteWrite::Saved(path) => {
                let named_now = self.file.as_deref() != Some(path.as_path());
                self.set_note_file(path, cx);
                if let Some(note) = self.note.as_mut() {
                    note.saved_version = Some(version);
                }
                if named_now {
                    notify_notes_changed(cx);
                }
                true
            }
            NoteWrite::Failed => false,
        }
    }

    /// Like `set_file`, but for a note: does not add it to Open Recent or the Windows jump list
    /// (`picker::RecentFiles::note`) - a sticky note is not a document the user "opened", and
    /// should not clutter either list.
    fn set_note_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.disk_stamp = crate::disk::DiskStamp::of(&path);
        self.disk_changed = false;
        self.file = Some(path);
        self.saved_history_position = self.doc.buffer().history_position();
        cx.notify();
    }

    /// Installs a note's file read from disk at start-up (`crates/tachyon/src/app.rs`'s
    /// restore, `load_note_file`), once its content is already in place (`Editor::set_loaded`):
    /// like `set_note_file`, without Open Recent or jump-list clutter.
    pub fn adopt_note_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.clear_pending_note_file();
        self.set_note_file(path, cx);
    }

    /// `Editor::close_window`'s note branch: never asks (autosave already keeps the file
    /// current; there is nothing a prompt would protect here) - flushing first so text from
    /// inside the last debounce window is not lost.
    pub(crate) fn close_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_paste(cx);
        self.flush_note(cx);
        window.remove_window();
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(y, mo, d, h, mi, s).single().expect("valid test timestamp")
    }

    #[test]
    fn title_from_text_prefers_the_first_heading() {
        assert_eq!(title_from_text("Intro line\n\n# Real Title\n\nbody\n"), "Real Title");
        assert_eq!(title_from_text("## Sub heading\ntext\n"), "Sub heading");
    }

    #[test]
    fn title_from_text_falls_back_to_the_first_non_empty_line() {
        assert_eq!(title_from_text("\n\nJust a line\n\nmore\n"), "Just a line");
        assert_eq!(
            title_from_text("#no-space-so-not-a-heading\nrest\n"),
            "no-space-so-not-a-heading"
        );
    }

    #[test]
    fn title_from_text_strips_markdown_markers() {
        assert_eq!(title_from_text("# **Bold** _idea_\n"), "Bold idea");
        assert_eq!(title_from_text("- a list item title\n"), "a list item title");
        assert_eq!(title_from_text("1. first step\n"), "first step");
        assert_eq!(title_from_text("> quoted title\n"), "quoted title");
        assert_eq!(title_from_text("`code` word\n"), "code word");
    }

    #[test]
    fn sanitize_component_replaces_windows_invalid_characters() {
        assert_eq!(sanitize_component("a/b:c*d?e"), "a b c d e");
    }

    #[test]
    fn sanitize_component_trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_component("My Note...   "), "My Note");
    }

    #[test]
    fn sanitize_component_rejects_reserved_device_names_case_insensitively() {
        assert_eq!(sanitize_component("CON"), "");
        assert_eq!(sanitize_component("con"), "");
        assert_eq!(sanitize_component("Nul"), "");
        assert_eq!(sanitize_component("Contacts"), "Contacts", "not merely a prefix match");
    }

    #[test]
    fn cut_to_length_never_splits_a_multibyte_character() {
        let s = format!("{}é", "a".repeat(59));
        assert_eq!(s.len(), 61, "the é starts exactly at the 60-byte cut point");
        let cut = cut_to_length(&s, 60);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
        assert_eq!(cut, "a".repeat(59));
    }

    #[test]
    fn base_name_falls_back_to_a_timestamp_when_nothing_usable_remains() {
        let now = local(2026, 9, 28, 14, 5, 9);
        assert_eq!(base_name("", now), "note-20260928-140509");
        assert_eq!(base_name("///\n\n***\n", now), "note-20260928-140509");
    }

    #[test]
    fn base_name_uses_the_sanitized_title_when_there_is_one() {
        let now = local(2026, 1, 1, 0, 0, 0);
        assert_eq!(base_name("# Grocery list\n\nMilk\n", now), "Grocery list");
    }

    #[test]
    fn unique_name_adds_a_numbered_suffix() {
        let existing = ["Draft", "Draft (2)"];
        assert_eq!(unique_name(|c| existing.contains(&c), "Draft"), "Draft (3)");
        assert_eq!(unique_name(|c| existing.contains(&c), "Other"), "Other");
    }

    fn sample_bounds() -> Bounds<Pixels> {
        bounds(point(px(12.), px(34.)), size(px(360.), px(360.)))
    }

    fn sample_note() -> NoteWindowState {
        NoteWindowState {
            path: PathBuf::from("/home/user/tachyon/notes/Draft.md"),
            bounds: sample_bounds(),
            pinned: true,
        }
    }

    #[test]
    fn notes_state_round_trips() {
        let notes = vec![sample_note(), NoteWindowState { pinned: false, ..sample_note() }];
        let text = {
            let mut out = format!("version = {NOTES_VERSION}\n");
            for n in &notes {
                out.push('\0');
                out.push_str(&serialize_note(n));
            }
            out
        };
        assert_eq!(parse_notes_state(&text), notes);
    }

    #[test]
    fn a_path_with_an_embedded_newline_round_trips() {
        let odd = NoteWindowState { path: PathBuf::from("/odd\nname.md"), ..sample_note() };
        let text = format!("version = {NOTES_VERSION}\n\0{}", serialize_note(&odd));
        assert_eq!(parse_notes_state(&text), vec![odd]);
    }

    #[test]
    fn an_unknown_version_notes_file_is_ignored() {
        let text = format!("version = 2\n\0{}", serialize_note(&sample_note()));
        assert_eq!(parse_notes_state(&text), Vec::new());
    }

    #[test]
    fn a_corrupt_notes_file_is_ignored_without_panicking() {
        assert_eq!(parse_notes_state("not a notes file\0garbage\n\nmore"), Vec::new());
        assert_eq!(parse_notes_state(""), Vec::new());
        let full = serialize_note(&sample_note());
        let cut = &full[..full.len() / 2];
        assert_eq!(parse_notes_state(&format!("version = {NOTES_VERSION}\n\0{cut}")), Vec::new());
    }

    #[test]
    fn writes_and_reads_notes_state_back_through_a_real_file() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-notes-state-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("notes.txt");
        let notes = vec![sample_note()];
        write_notes_state(&path, &notes).expect("write");
        assert_eq!(read_notes_state(&path), notes);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_huge_notes_file_is_ignored_without_reading_it() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-notes-huge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge.txt");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_NOTES_FILE_BYTES + 1).unwrap();
        assert_eq!(read_notes_state(&path), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_notes_skips_one_whose_file_is_gone() {
        let dir = std::env::temp_dir()
            .join(format!("tachyon-notes-existing-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let present = dir.join("present.md");
        std::fs::write(&present, "hi").unwrap();
        let gone = dir.join("gone.md");
        let kept = NoteWindowState { path: present, bounds: sample_bounds(), pinned: false };
        let dropped = NoteWindowState { path: gone, bounds: sample_bounds(), pinned: true };
        assert_eq!(existing_notes(vec![kept.clone(), dropped]), vec![kept]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_note_to_disk_names_the_note_and_then_keeps_the_same_path() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-notes-write-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = match write_note_to_disk(&dir, None, "# Grocery list\n\nMilk\n") {
            NoteWrite::Saved(path) => path,
            _ => panic!("expected a save"),
        };
        assert_eq!(path.file_stem().and_then(|s| s.to_str()), Some("Grocery list"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Grocery list\n\nMilk\n");

        // The heading changes, but the file the note was first saved as stays the same.
        match write_note_to_disk(&dir, Some(&path), "# Renamed\n\nMilk\n") {
            NoteWrite::Saved(saved) => assert_eq!(saved, path),
            _ => panic!("expected a save"),
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Renamed\n\nMilk\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_note_to_disk_makes_a_second_note_with_the_same_title_unique() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-notes-unique-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = match write_note_to_disk(&dir, None, "# Draft\n") {
            NoteWrite::Saved(path) => path,
            _ => panic!("expected a save"),
        };
        let second = match write_note_to_disk(&dir, None, "# Draft\n\nmore\n") {
            NoteWrite::Saved(path) => path,
            _ => panic!("expected a save"),
        };
        assert_ne!(first, second);
        assert_eq!(second.file_stem().and_then(|s| s.to_str()), Some("Draft (2)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_note_to_disk_deletes_an_emptied_file_and_never_creates_the_folder_for_nothing() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-notes-empty-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Never typed into: no folder appears.
        assert!(matches!(write_note_to_disk(&dir, None, ""), NoteWrite::Empty));
        assert!(!dir.exists());

        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.md");
        std::fs::write(&path, "was here").unwrap();
        assert!(matches!(write_note_to_disk(&dir, Some(&path), "   \n"), NoteWrite::Empty));
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stored_notes_lists_markdown_files_and_skips_a_missing_folder() {
        let dir = std::env::temp_dir().join(format!("tachyon-notes-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("older.md"), "old\n").unwrap();
        std::fs::write(dir.join("newer.md"), "new\n").unwrap();
        std::fs::write(dir.join("skip.txt"), "no\n").unwrap();
        let mut names: Vec<_> = stored_notes(&dir)
            .unwrap()
            .into_iter()
            .map(|note| note.path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["newer", "older"]);
        assert!(stored_notes(&dir.join("missing")).unwrap().is_empty());
        assert!(stored_notes(&dir.join("skip.txt")).is_err(), "not a directory is not 'no notes'");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relative_age_uses_the_documented_boundaries() {
        use std::time::Duration;
        assert_eq!(relative_age(Duration::from_secs(0)), "just now");
        assert_eq!(relative_age(Duration::from_secs(59)), "just now");
        assert_eq!(relative_age(Duration::from_secs(60)), "1 min ago");
        assert_eq!(relative_age(Duration::from_secs(3_600)), "1 hr ago");
        assert_eq!(relative_age(Duration::from_secs(86_400)), "yesterday");
        assert_eq!(relative_age(Duration::from_secs(86_400 * 3)), "3 days ago");
    }
}
