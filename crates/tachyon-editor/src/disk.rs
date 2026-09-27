//! Files changed on disk behind the editor's back. The editor remembers each file's modification
//! time and size as it read or wrote them. When its window is activated it looks again: an
//! unchanged document reloads, one with unsaved changes keeps them, and Save then asks before
//! overwriting the other version.

use std::path::Path;
use std::time::SystemTime;

use gpui::{Context, Window};

use crate::editor::Editor;

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

    /// Serialized for a hot-exit backup: nanoseconds since the epoch (or `-`) and the length.
    pub(crate) fn encode(self) -> String {
        let nanos = self
            .modified
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or_else(|| "-".to_owned(), |d| d.as_nanos().to_string());
        format!("{nanos} {}", self.len)
    }

    pub(crate) fn decode(text: &str) -> Option<Self> {
        let (nanos, len) = text.trim().split_once(' ')?;
        let modified = match nanos {
            "-" => None,
            n => {
                let nanos: u64 = n.parse().ok()?;
                Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(nanos))
            }
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
            let text = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(&read_path) })
                .await;
            let _ = this.update(cx, |editor, cx| {
                if let Ok(text) = text
                    && !editor.is_modified()
                    && editor.disk_stamp == Some(known)
                {
                    let caret = editor.head();
                    editor.set_document(tachyon_doc::Document::new(&text), cx);
                    editor.disk_stamp = Some(current);
                    editor.move_to(caret, false, cx);
                }
            });
        })
        .detach();
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
        assert_eq!(DiskStamp::decode("garbage"), None);
    }
}
