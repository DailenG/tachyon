//! Files changed on disk behind the editor's back, and reading them in the first place.
//! `load_document` is the single place a path becomes a [`Document`]: it decides Markdown vs.
//! plain text, enforces the size limits that keep a huge file from ever repeating the 18 GB,
//! frozen-window behavior this feature replaces, and reads incrementally
//! (`tachyon_text::Buffer::load`) so opening one never holds the whole file as a second copy in
//! memory. The editor remembers each file's modification time and size as it read or wrote them.
//! When its window is activated it looks again: an unchanged document reloads, one with unsaved
//! changes keeps them, and Save then asks before overwriting the other version.

use std::path::Path;
use std::time::SystemTime;

use gpui::{Context, SharedString, Window};
use tachyon_doc::{DocMode, Document, MARKDOWN_SIZE_LIMIT, PLAIN_HARD_LIMIT, mode_for_extension};
use tachyon_text::Buffer;

use crate::editor::Editor;

/// A one-line notice shown at the top of the view (see `render::mode_notice`) while a Markdown
/// file is held in plain-text mode because it is larger than [`MARKDOWN_SIZE_LIMIT`]; the toggle
/// back to Markdown is refused for the same reason (`Editor::toggle_text_mode`).
pub fn oversized_markdown_notice() -> SharedString {
    SharedString::from(format!(
        "Larger than {} for Markdown: showing as plain text. Ctrl+Shift+M is disabled above that \
         size.",
        human_size(MARKDOWN_SIZE_LIMIT)
    ))
}

/// `bytes`, rounded to whole MiB or GiB (both limits this crate cares about are round numbers in
/// one of those units).
fn human_size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= 1024 * MIB {
        format!("{} GiB", bytes / (1024 * MIB))
    } else {
        format!("{} MiB", bytes / MIB)
    }
}

/// A file read by [`load_document`], beyond the document itself.
pub struct Loaded {
    pub doc: Document,
    /// Some of the file's bytes were not valid UTF-8 (or, being NUL-sniffed as binary, might not
    /// round-trip through text at all) and were replaced with U+FFFD: saving would change the
    /// file's bytes, so `Editor::save` must warn before doing that.
    pub lossy: bool,
    /// Set when the file opened as plain text only because it is too large for Markdown.
    pub notice: Option<SharedString>,
}

/// Outcome of [`load_document`]: either a document to show, or a refusal (a file too large to
/// ever open, whatever its mode) with a message to show in its place.
pub enum LoadOutcome {
    Loaded(Box<Loaded>),
    Refused(String),
}

/// Reads `path` and builds the document it opens as. Markdown by extension, unless the file is
/// larger than [`MARKDOWN_SIZE_LIMIT`] (then plain text, with a notice) or larger than
/// [`PLAIN_HARD_LIMIT`] (then refused outright: [`LoadOutcome::Refused`], never read). Reads
/// incrementally (`Buffer::load`), so opening a huge file never holds a second full copy of it in
/// memory the way `std::fs::read_to_string` plus building a rope from the result would; invalid
/// UTF-8 and binary content (a NUL byte in the first 8 KiB, sniffed the same pass) still open,
/// decoded lossily and forced to plain text rather than parsed as Markdown regardless of
/// extension, per the "never crash on user input" rule.
pub fn load_document(path: &Path) -> std::io::Result<LoadOutcome> {
    let size = std::fs::metadata(path)?.len();
    if size > PLAIN_HARD_LIMIT {
        return Ok(LoadOutcome::Refused(format!(
            "{} is {} and Tachyon refuses files larger than {}, to avoid running out of memory. \
             Open it with a tool built for very large files instead.",
            path.display(),
            human_size(size),
            human_size(PLAIN_HARD_LIMIT)
        )));
    }
    let file = std::fs::File::open(path)?;
    let (buffer, report) = Buffer::load(file)?;
    let mut mode = mode_for_extension(path);
    let mut notice = None;
    if mode == DocMode::Markdown && buffer.len() as u64 > MARKDOWN_SIZE_LIMIT {
        mode = DocMode::Plain;
        notice = Some(oversized_markdown_notice());
    } else if mode == DocMode::Markdown && (report.lossy || report.looks_binary) {
        // Binary or garbled content: parsing it as Markdown would be pointless work over
        // replacement characters, and could not have been the user's intent.
        mode = DocMode::Plain;
    }
    let doc = Document::from_buffer(buffer, mode);
    Ok(LoadOutcome::Loaded(Box::new(Loaded { doc, lossy: report.lossy, notice })))
}

/// What identifies a version of a file on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl DiskStamp {
    /// The file's current stamp; `None` if it cannot be read (deleted, no permission).
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(DiskStamp { modified: metadata.modified().ok(), len: metadata.len() })
    }

    /// Serialized for a hot-exit backup: signed nanoseconds from the epoch (`none` if the file
    /// system has no modification time) and the length.
    pub(crate) fn encode(self) -> String {
        let nanos = self.modified.map_or_else(
            || "none".to_owned(),
            |t| match t.duration_since(SystemTime::UNIX_EPOCH) {
                Ok(after) => after.as_nanos().to_string(),
                Err(before) => format!("-{}", before.duration().as_nanos()),
            },
        );
        format!("{nanos} {}", self.len)
    }

    pub(crate) fn decode(text: &str) -> Option<Self> {
        let (nanos, len) = text.trim().split_once(' ')?;
        let from_nanos = |n: &str| n.parse::<u64>().ok().map(std::time::Duration::from_nanos);
        let modified = match nanos {
            "none" => None,
            n => Some(match n.strip_prefix('-') {
                Some(before) => SystemTime::UNIX_EPOCH.checked_sub(from_nanos(before)?)?,
                None => SystemTime::UNIX_EPOCH.checked_add(from_nanos(n)?)?,
            }),
        };
        Some(DiskStamp { modified, len: len.parse().ok()? })
    }
}

impl Editor {
    /// On window activation: reloads the file if it changed on disk and the document has no
    /// unsaved changes; otherwise notes the change so Save asks first.
    pub(crate) fn check_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), Some(known)) = (self.file.clone(), self.disk_stamp) else { return };
        cx.spawn_in(window, async move |this, cx| {
            let read_path = path.clone();
            let current =
                cx.background_executor().spawn(async move { DiskStamp::of(&read_path) }).await;
            let Some(current) = current.filter(|current| *current != known) else { return };
            let reload = this.update(cx, |editor, _| {
                // Still the same file and version as when the check started.
                let same = editor.file.as_deref() == Some(path.as_path())
                    && editor.disk_stamp == Some(known);
                if same && editor.is_modified() {
                    editor.disk_changed = true;
                }
                same && !editor.is_modified()
            });
            if !matches!(reload, Ok(true)) {
                return;
            }
            let read_path = path.clone();
            let outcome =
                cx.background_executor().spawn(async move { load_document(&read_path) }).await;
            let _ = this.update(cx, |editor, cx| {
                let still_current = editor.file.as_deref() == Some(path.as_path())
                    && !editor.is_modified()
                    && editor.disk_stamp == Some(known);
                if !still_current {
                    return;
                }
                // A file that grew too large to reopen, or a transient read error (permissions,
                // the file briefly gone mid-write): leave the window showing what it last had
                // rather than replacing it with a refusal or losing the file association.
                let Ok(LoadOutcome::Loaded(loaded)) = outcome else { return };
                let caret = editor.head();
                editor.set_loaded(*loaded, cx);
                editor.disk_stamp = Some(current);
                editor.file = Some(path);
                editor.move_to(caret, false, cx);
            });
        })
        .detach();
    }

    /// Installs a document [`load_document`] read, along with whether it was decoded lossily
    /// (so `Editor::save` warns) and any mode notice.
    pub fn set_loaded(&mut self, loaded: Loaded, cx: &mut Context<Self>) {
        self.set_document(loaded.doc, cx);
        self.lossy = loaded.lossy;
        self.notice = loaded.notice;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_round_trip_through_backups() {
        let stamp = DiskStamp {
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(1_234_567_891)),
            len: 42,
        };
        assert_eq!(DiskStamp::decode(&stamp.encode()), Some(stamp));
        let unknown = DiskStamp { modified: None, len: 7 };
        assert_eq!(DiskStamp::decode(&unknown.encode()), Some(unknown));
        let before_epoch = DiskStamp {
            modified: Some(SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(86_400)),
            len: 1,
        };
        assert_eq!(DiskStamp::decode(&before_epoch.encode()), Some(before_epoch));
        assert_eq!(DiskStamp::decode("garbage"), None);
    }
}
