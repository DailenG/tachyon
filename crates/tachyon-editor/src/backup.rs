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
const BACKUP_DELAY: Duration = Duration::from_millis(1500);

/// Where unsaved documents are backed up. Each has a `<slot>.md` file with its text (original line
/// endings) and, if it belongs to a file, a `<slot>.path` file with that file's path and, on a
/// second line, the file's version on disk when it was read (to notice later changes).
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
                let mut lines = meta.lines();
                let file = lines.next().filter(|line| !line.is_empty()).map(PathBuf::from);
                let stamp = lines.next().and_then(DiskStamp::decode);
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
            let stamp = stamp.map(DiskStamp::encode).unwrap_or_default();
            write_atomically(&path_file, format!("{file}\n{stamp}").as_bytes())
        }
        None => match std::fs::remove_file(&path_file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

fn remove_slot(slot: &Path) {
    let _ = std::fs::remove_file(slot);
    let _ = std::fs::remove_file(slot.with_extension("path"));
}

impl Editor {
    /// Takes over a backup from an earlier session: its text, its file, and its slot, and counts
    /// as unsaved.
    pub fn adopt_backup(&mut self, restored: Restored, cx: &mut Context<Self>) {
        self.set_document(tachyon_doc::Document::new(&restored.text), cx);
        self.file = restored.file;
        self.disk_stamp = restored.stamp;
        // No later version equals this one, so the document stays modified until saved.
        self.saved_version = self.doc.buffer().version().wrapping_sub(1);
        self.backup_slot = Some(restored.slot);
        self.backed_up_version = Some(self.doc.buffer().version());
        cx.notify();
    }

    /// After an edit or a save: backs up unsaved text after a pause, or drops the backup of text
    /// that is saved now.
    pub(crate) fn schedule_backup(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<Backups>() {
            return;
        }
        if !self.is_modified() {
            self.discard_backup();
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

    fn write_backup(&mut self, cx: &mut Context<Self>) {
        self.backup_task = None;
        let version = self.doc.buffer().version();
        if !self.is_modified() || self.backed_up_version == Some(version) {
            return;
        }
        let Some(slot) = self.slot(cx) else { return };
        let text = self.doc.buffer().to_saved_text();
        let file = self.file.clone();
        let stamp = self.disk_stamp;
        self.backed_up_version = Some(version);
        cx.background_executor()
            .spawn(async move { write_slot(&slot, &text, file.as_deref(), stamp) })
            .detach();
    }

    /// Backs up now, on this thread (Quit). Returns whether the text is safely on disk.
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

    fn slot(&mut self, cx: &Context<Self>) -> Option<PathBuf> {
        if self.backup_slot.is_none() {
            self.backup_slot = Some(cx.try_global::<Backups>()?.new_slot());
        }
        self.backup_slot.clone()
    }
}
