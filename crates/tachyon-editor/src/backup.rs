//! Hot exit. While a document has unsaved changes its text is backed up to a file shortly after
//! each edit; Quit then closes windows without asking, and the next start opens them again. The
//! application enables it for its primary instance by setting [`Backups`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{Context, Global};

use crate::disk::DiskStamp;
use crate::editor::{Editor, write_atomically};

/// How long after an edit the backup is written: typing bursts share one write.
pub(crate) const BACKUP_DELAY: Duration = Duration::from_millis(1500);

/// Where unsaved documents are backed up. Each has a `<slot>.md` file with its text (original line
/// endings) and, if it belongs to a file, a `<slot>.path` file: `stamp <version>` on the first
/// line (the file's version on disk when it was read, to notice later changes; `stamp unknown` if
/// it had none), then the file's path, which may itself contain line breaks.
pub struct Backups {
    dir: PathBuf,
}

impl Global for Backups {}

/// Quit is closing the windows: they close without asking, their unsaved text backed up.
pub struct HotExit;

impl Global for HotExit {}

/// A document backed up by an earlier session.
pub struct Restored {
    pub slot: PathBuf,
    pub text: String,
    pub file: Option<PathBuf>,
    stamp: Option<DiskStamp>,
}

impl Backups {
    pub fn new(dir: PathBuf) -> Self {
        Backups { dir }
    }

    /// Where backups are written, for the About window's Environment table ("Backups"): a
    /// clickable path, so it needs the directory itself, not just the ability to write to it.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The backups left by an earlier session, oldest first. Unreadable ones are skipped.
    pub fn restore(&self) -> Vec<Restored> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut slots: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "md"))
            .collect();
        slots.sort();
        slots
            .into_iter()
            .filter_map(|slot| {
                let text = std::fs::read_to_string(&slot).ok()?;
                let meta = std::fs::read_to_string(slot.with_extension("path")).unwrap_or_default();
                let (file, stamp) = parse_meta(&meta);
                Some(Restored { slot, text, file, stamp })
            })
            .collect()
    }

    /// A new slot: named by time and a counter, so slots sort in creation order and several
    /// windows never share one.
    fn new_slot(&self) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        self.dir.join(format!("{nanos:024}-{n:04}.md"))
    }
}

/// Writes a backup: the text, then the file path and version next to it (or removes a stale one).
fn write_slot(
    slot: &Path,
    text: &str,
    file: Option<&Path>,
    stamp: Option<DiskStamp>,
) -> std::io::Result<()> {
    if let Some(dir) = slot.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomically(slot, text.as_bytes())?;
    let path_file = slot.with_extension("path");
    match file.and_then(Path::to_str) {
        Some(file) => {
            let stamp = stamp.map_or_else(|| "unknown".to_owned(), DiskStamp::encode);
            write_atomically(&path_file, format!("stamp {stamp}\n{file}").as_bytes())
        }
        None => match std::fs::remove_file(&path_file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// The file and its version from a `<slot>.path` file. Backups written before versions were
/// recorded hold just the path.
fn parse_meta(meta: &str) -> (Option<PathBuf>, Option<DiskStamp>) {
    let (stamp, path) = match meta.strip_prefix("stamp ").and_then(|rest| rest.split_once('\n')) {
        Some((stamp, path)) => (DiskStamp::decode(stamp), path),
        None => (None, meta),
    };
    ((!path.is_empty()).then(|| PathBuf::from(path)), stamp)
}

fn remove_slot(slot: &Path) {
    let _ = std::fs::remove_file(slot);
    let _ = std::fs::remove_file(slot.with_extension("path"));
}

impl Editor {
    /// Takes over a backup from an earlier session: its text, its file, and its slot, and counts
    /// as unsaved.
    pub fn adopt_backup(&mut self, restored: Restored, cx: &mut Context<Self>) {
        // A restored scratch buffer stays Markdown; a restored file uses its own extension (and
        // the same oversized-Markdown fallback a fresh open would), never whatever mode the
        // window happened to be in when it was backed up (the toggle is not persisted).
        let by_extension = restored
            .file
            .as_deref()
            .map_or(tachyon_doc::DocMode::Markdown, tachyon_doc::mode_for_extension);
        let oversized = by_extension == tachyon_doc::DocMode::Markdown
            && restored.text.len() as u64 > tachyon_doc::MARKDOWN_SIZE_LIMIT;
        let mode = if oversized { tachyon_doc::DocMode::Plain } else { by_extension };
        let buffer = tachyon_text::Buffer::new(&restored.text);
        self.set_document(tachyon_doc::Document::from_buffer(buffer, mode), cx);
        if oversized {
            self.notice = Some(crate::disk::oversized_markdown_notice());
        }
        // Without a recorded version the file may have changed since: Save asks first.
        self.disk_changed = restored.file.is_some() && restored.stamp.is_none();
        self.file = restored.file;
        self.disk_stamp = restored.stamp;
        // A restored backup is never "saved": force `is_modified` regardless of the fresh
        // buffer's own history position (`u64::MAX` is never a real position - see
        // `tachyon_text::Buffer::history_position` - so this can never accidentally match).
        self.saved_history_position = u64::MAX;
        self.backup_slot = Some(restored.slot);
        self.backed_up_version = Some(self.doc.buffer().version());
        cx.notify();
    }

    /// Writes this document's unsaved-document backup immediately, without closing the window:
    /// for Windows session end (`WM_QUERYENDSESSION`/`WM_ENDSESSION` - see
    /// `tachyon_platform::windows::tray`), where the process may be killed as soon as the tray's
    /// window procedure returns, so there is no time to wait for the usual 1.5 s pause
    /// ([`Editor::schedule_backup`]). Returns whether the document is safe: `true` if there was
    /// nothing to back up (no hot exit, or no unsaved changes - the same guard
    /// [`Editor::write_backup`] itself relies on, so this never creates a backup file for an
    /// untouched document), or the write succeeded; `false` only if a write was attempted and
    /// failed, so the caller (`crates/tachyon/src/app.rs`) can report exactly which document was
    /// not preserved.
    pub fn backup_for_session_end(&mut self, cx: &mut Context<Self>) -> bool {
        // Sticky notes are never hot-exit backups: `crates/tachyon/src/app.rs`'s
        // `backup_every_window_for_session_end` calls `Editor::flush_note` for one instead.
        if self.is_note() || !cx.has_global::<Backups>() || !self.is_modified() {
            return true;
        }
        self.backup_now(cx)
    }

    /// After an edit or a save: backs up unsaved text after a pause, or drops the backup of text
    /// that is saved now. Above [`tachyon_doc::LARGE_PLAIN_SIZE`] this is a no-op: writing tens or
    /// hundreds of megabytes 1.5 s after every keystroke would itself compete with the UI
    /// thread's frame budget on a slow disk, even though the write runs off it (`write_backup`).
    /// Such a document is still backed up once, on quit or close, by `Editor::should_close`
    /// calling `backup_now` directly (never through this method), so hot exit still restores it -
    /// only the *periodic*, typing-pause backup is skipped.
    pub(crate) fn schedule_backup(&mut self, cx: &mut Context<Self>) {
        // A sticky note autosaves to its own file instead (`Editor::schedule_note_autosave`)
        // and is never a hot-exit backup.
        if self.is_note() || !cx.has_global::<Backups>() {
            return;
        }
        if !self.is_modified() {
            self.discard_backup();
            return;
        }
        if self.doc.len() as u64 > tachyon_doc::LARGE_PLAIN_SIZE {
            return;
        }
        if self.backup_task.is_some() || self.backed_up_version == Some(self.doc.buffer().version())
        {
            return;
        }
        self.backup_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BACKUP_DELAY).await;
            let _ = this.update(cx, |editor, cx| editor.write_backup(cx));
        }));
    }

    /// The saved text is built from a cloned rope snapshot (O(1): `ropey` shares nodes via
    /// `Arc`) inside the spawned task, not here: `tachyon_text::saved_text` is `O(document
    /// size)`, and this runs after every pause in typing (below `LARGE_PLAIN_SIZE`; see
    /// `schedule_backup`), so even a few-MB document should not repeatedly cost the UI thread a
    /// millisecond-plus string build and CRLF pass.
    fn write_backup(&mut self, cx: &mut Context<Self>) {
        self.backup_task = None;
        let version = self.doc.buffer().version();
        if !self.is_modified() || self.backed_up_version == Some(version) {
            return;
        }
        let Some(slot) = self.slot(cx) else { return };
        let rope = self.doc.buffer().rope().clone();
        let line_ending = self.doc.buffer().line_ending();
        let file = self.file.clone();
        let stamp = self.disk_stamp;
        self.backed_up_version = Some(version);
        cx.background_executor()
            .spawn(async move {
                let text = tachyon_text::saved_text(&rope, line_ending);
                write_slot(&slot, &text, file.as_deref(), stamp)
            })
            .detach();
    }

    /// Backs up now, on this thread (Quit): the write must finish before the process exits, so
    /// unlike `write_backup` it cannot hand the string-building work to a background task and
    /// move on. The one-time stall this risks for a huge document is accepted here - quitting
    /// already ends the frame loop - in exchange for the guarantee that hot exit has the text.
    /// Never gated by `LARGE_PLAIN_SIZE`: this is the *only* backup point such a document gets.
    /// Returns whether the text is safely on disk.
    pub(crate) fn backup_now(&mut self, cx: &mut Context<Self>) -> bool {
        self.backup_task = None;
        let Some(slot) = self.slot(cx) else { return false };
        let text = self.doc.buffer().to_saved_text();
        let written = write_slot(&slot, &text, self.file.as_deref(), self.disk_stamp).is_ok();
        if written {
            self.backed_up_version = Some(self.doc.buffer().version());
        }
        written
    }

    /// Forgets the backup: the text was saved, or deliberately discarded.
    pub(crate) fn discard_backup(&mut self) {
        self.backup_task = None;
        self.backed_up_version = None;
        if let Some(slot) = self.backup_slot.take() {
            remove_slot(&slot);
        }
    }

    /// This document's backup slot, allocating one now if it does not have one yet. Used both by
    /// the periodic and Quit-time writers (`write_backup`, `backup_now`) and, read-only in
    /// effect, by `session::Editor::session_state` to name the same slot in the session file
    /// before the write itself lands.
    pub(crate) fn slot(&mut self, cx: &Context<Self>) -> Option<PathBuf> {
        if self.backup_slot.is_none() {
            self.backup_slot = Some(cx.try_global::<Backups>()?.new_slot());
        }
        self.backup_slot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_keeps_paths_with_line_breaks_and_reads_old_backups() {
        let stamp = DiskStamp { modified: None, len: 3 };
        let meta = format!("stamp {}\n/odd\nname.md", stamp.encode());
        assert_eq!(parse_meta(&meta), (Some(PathBuf::from("/odd\nname.md")), Some(stamp)));
        assert_eq!(parse_meta("stamp unknown\n/a.md"), (Some(PathBuf::from("/a.md")), None));
        assert_eq!(parse_meta("/old/format.md"), (Some(PathBuf::from("/old/format.md")), None));
        assert_eq!(parse_meta(""), (None, None));
    }
}
