//! Session restore (issue #79), on top of hot exit (ADR 0006, `backup.rs`). Hot exit alone
//! remembers every *unsaved* document; this remembers the whole session - clean files too, plus
//! each window's bounds, maximized state, caret, scroll and mode - in one small file next to the
//! backups, `state_dir()/session-<instance>.txt`. The application enables it by writing the file
//! on Quit (see `tachyon::app::write_session`) and reading it at the next start
//! (`tachyon::app::restore_sources`); this module only holds the format and the per-window state
//! capture/restore, the same split `backup.rs` uses for hot exit itself.
//!
//! # Format
//! `version = 1`, then each window as its own record, separated by a NUL byte - never valid in a
//! path on any real filesystem, so the record's last field (the file path or backup slot) can
//! safely contain '\n' with no escaping, the same trick a hot-exit backup's own `.path` file
//! uses for the same reason (`backup::parse_meta`). Corrupt, truncated or unknown-version input
//! is never a panic: user input (a state file) is always a normal case, ignored like a missing
//! file would be.

use std::path::{Path, PathBuf};

use gpui::{Bounds, Context, ListOffset, Pixels, Window, bounds, point, px, size};
use tachyon_doc::DocMode;

use crate::editor::{Editor, write_atomically};

/// The only format version written or understood so far; an unrecognized one is treated as no
/// session at all, the same as a missing or corrupt file.
const SESSION_VERSION: u32 = 1;

/// However many windows a session ever records or restores: generous for real use (nobody keeps
/// hundreds of windows open) and a hard ceiling against a corrupt or hostile file forcing
/// unbounded work.
pub const MAX_SESSION_WINDOWS: usize = 100;

/// Above this, the file is treated as corrupt without even trying to parse it: a session file is
/// a few hundred bytes per window, so anything past a generous multiple of `MAX_SESSION_WINDOWS`
/// can only be damage, not a real one - never worth the read and allocation to find out.
const MAX_SESSION_FILE_BYTES: u64 = 1_000_000;

/// What a recorded window belongs to: a real file, restored directly if it still exists, or a
/// hot-exit backup slot, matched against what `Backups::restore` finds on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    File(PathBuf),
    Backup(PathBuf),
}

/// One window's session record: enough to reopen it in the same place, showing the same thing.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    pub target: Target,
    pub bounds: Bounds<Pixels>,
    pub maximized: bool,
    /// Byte offset of the caret when the session was written.
    pub caret: usize,
    /// Byte offset of the first block in view when the session was written.
    pub scroll: usize,
    pub mode: DocMode,
}

/// Writes `windows` to `path` atomically (temp file, then rename - see `write_atomically`), so a
/// crash or a concurrent read never sees a half-written file. Silently truncates to
/// `MAX_SESSION_WINDOWS`: the caller (`tachyon::app::write_session`) only ever passes the
/// currently open windows, well under that in ordinary use.
pub fn write_session(path: &Path, windows: &[WindowState]) -> std::io::Result<()> {
    let windows = &windows[..windows.len().min(MAX_SESSION_WINDOWS)];
    let mut out = format!("version = {SESSION_VERSION}\n");
    for window in windows {
        out.push('\0');
        out.push_str(&serialize_window(window));
    }
    write_atomically(path, out.as_bytes())
}

fn serialize_window(window: &WindowState) -> String {
    let (kind, path) = match &window.target {
        Target::File(path) => ("file", path),
        Target::Backup(path) => ("backup", path),
    };
    let mode = match window.mode {
        DocMode::Markdown => "markdown",
        DocMode::Plain => "plain",
    };
    format!(
        "kind = {kind}\n\
         x = {}\n\
         y = {}\n\
         w = {}\n\
         h = {}\n\
         maximized = {}\n\
         caret = {}\n\
         scroll = {}\n\
         mode = {mode}\n\
         path = {}",
        f32::from(window.bounds.origin.x),
        f32::from(window.bounds.origin.y),
        f32::from(window.bounds.size.width),
        f32::from(window.bounds.size.height),
        window.maximized,
        window.caret,
        window.scroll,
        path.display(),
    )
}

/// Reads and parses the session file. A missing file, one over `MAX_SESSION_FILE_BYTES`, invalid
/// UTF-8, an unrecognized version, or any individual malformed record all degrade to "no session"
/// (or, for one bad record among otherwise good ones, just that record dropped) rather than an
/// error: a state file is user input, and a bad one is a normal case to ignore, never to panic
/// on.
pub fn read_session(path: &Path) -> Vec<WindowState> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_SESSION_FILE_BYTES => {}
        _ => return Vec::new(),
    }
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse_session(&text)
}

fn parse_session(text: &str) -> Vec<WindowState> {
    let mut records = text.split('\0');
    let header = records.next().unwrap_or("");
    let version = header.lines().find_map(|line| {
        line.strip_prefix("version =").map(str::trim).and_then(|v| v.parse().ok())
    });
    if version != Some(SESSION_VERSION) {
        return Vec::new();
    }
    records.filter_map(parse_window).take(MAX_SESSION_WINDOWS).collect()
}

/// The next field's value: `record`'s next line must read `expected = ...`, else the whole
/// record is malformed (`None`) and dropped by `parse_session`'s `filter_map`.
fn next_field<'a>(fields: &mut impl Iterator<Item = &'a str>, expected: &str) -> Option<&'a str> {
    let (key, value) = fields.next()?.split_once('=')?;
    (key.trim() == expected).then(|| value.trim())
}

fn parse_window(record: &str) -> Option<WindowState> {
    // 10 fields; `splitn` keeps any embedded '\n' in the last one (`path`) intact rather than
    // splitting on it - see the module doc comment.
    let mut fields = record.splitn(10, '\n');
    let kind = next_field(&mut fields, "kind")?;
    let x: f32 = next_field(&mut fields, "x")?.parse().ok()?;
    let y: f32 = next_field(&mut fields, "y")?.parse().ok()?;
    let w: f32 = next_field(&mut fields, "w")?.parse().ok()?;
    let h: f32 = next_field(&mut fields, "h")?.parse().ok()?;
    let maximized: bool = next_field(&mut fields, "maximized")?.parse().ok()?;
    let caret: usize = next_field(&mut fields, "caret")?.parse().ok()?;
    let scroll: usize = next_field(&mut fields, "scroll")?.parse().ok()?;
    let mode = match next_field(&mut fields, "mode")? {
        "markdown" => DocMode::Markdown,
        "plain" => DocMode::Plain,
        _ => return None,
    };
    let path = fields.next()?.strip_prefix("path")?.trim_start().strip_prefix('=')?.trim_start();
    if path.is_empty() || !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. {
        return None;
    }
    let target = match kind {
        "file" => Target::File(PathBuf::from(path)),
        "backup" => Target::Backup(PathBuf::from(path)),
        _ => return None,
    };
    Some(WindowState {
        target,
        bounds: bounds(point(px(x), px(y)), size(px(w), px(h))),
        maximized,
        caret,
        scroll,
        mode,
    })
}

/// Same walk-back-over-continuation-bytes rule as `tachyon_doc`'s own (private) char-boundary
/// clamp: a session-recorded caret may no longer land on one if the text changed since (edited
/// elsewhere before the next start, or shorter after a session written before a deletion).
fn floor_char_boundary(rope: &ropey::Rope, mut byte_idx: usize) -> usize {
    while byte_idx > 0 && byte_idx < rope.len_bytes() && (rope.byte(byte_idx) & 0xC0) == 0x80 {
        byte_idx -= 1;
    }
    byte_idx
}

/// `window`'s placement for the session file: GPUI's save/restore pair `window_bounds` (the normal,
/// unmaximized rectangle, in the coordinates a window is created with, so it lands exactly where it
/// was; `bounds` is offset from those on Windows), and whether it is maximized. The flag also asks
/// `is_maximized` (`IsZoomed` on Windows), which on the test machine reported a title-bar-button
/// maximize that `window_bounds` missed.
fn current_placement(window: &Window) -> (Bounds<Pixels>, bool) {
    let (bounds, maximized) = match window.window_bounds() {
        gpui::WindowBounds::Maximized(bounds) => (bounds, true),
        gpui::WindowBounds::Windowed(bounds) | gpui::WindowBounds::Fullscreen(bounds) => {
            (bounds, false)
        }
    };
    (bounds, maximized || window.is_maximized())
}

impl Editor {
    /// This window's session record, or `None` if there is nothing worth remembering: an
    /// untouched scratch buffer (no file, never modified - the sample, a fresh New Window, the
    /// What's new notes unless saved). A modified document - whether or not it has a file, since
    /// hot exit backs up either kind the same way - is recorded by its backup slot (allocating
    /// one now if typing has not yet paused long enough to have written one - see
    /// `Editor::slot`; `Editor::should_close`'s own `backup_now`, run moments later as each
    /// window closes, is what actually writes it, so this only needs to fix the path the
    /// session file will name); a clean, file-backed document is recorded by its path directly.
    /// Records the window's placement, if it is on screen (shown and not minimized): see
    /// `Editor::last_placement`.
    pub(crate) fn note_placement(&mut self, window: &Window) {
        if window.is_visible() {
            self.last_placement = Some(current_placement(window));
        }
    }

    pub fn session_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<WindowState> {
        self.flush_pending_paste(cx);
        let target = if self.is_modified() {
            Target::Backup(self.slot(cx)?)
        } else {
            Target::File(self.file.clone()?)
        };
        // `window_bounds`, not `bounds` and `is_maximized`: GPUI's own save/restore pair. It gives
        // the normal (unmaximized) rectangle in the coordinates `WindowBounds` takes when a window
        // is created, plus whether it is maximized, so a restored window lands exactly where it
        // was. `bounds` is offset from those coordinates on Windows (each restore drifted 15 px
        // down at 125 %), and for a maximized window it is the maximized rectangle, not the one
        // to restore to.
        // The placement last seen on screen (`note_placement`), not the window's state right
        // now: saving at a Windows session end otherwise recorded maximized windows as normal.
        let (bounds, maximized) = self.last_placement.unwrap_or_else(|| current_placement(window));
        Some(WindowState {
            target,
            bounds,
            maximized,
            caret: self.head(),
            scroll: self.viewport_offset(),
            mode: self.doc.mode(),
        })
    }

    /// Reapplies a session-recorded caret, scroll position and mode, once the document itself is
    /// in place (a restored backup, right away; a freshly loaded file, once the read finishes -
    /// see `tachyon::app::load_file`). Switches mode first if it differs from what the document
    /// opened with, refused above `MARKDOWN_SIZE_LIMIT` exactly like a manual `Ctrl+Shift+M`
    /// would be (a session file predates knowing today's size, so a document that grew past the
    /// limit since is treated the same as a fresh open of it would be) - silently, rather than
    /// with the toggle's own notice, since the caller may still set its own (a missing-file
    /// notice takes the one line a window gets). Caret and scroll are then clamped to the
    /// document's current length (and, for the caret, a char boundary): both may be stale if the
    /// text changed since the session was written.
    pub fn restore_view(
        &mut self,
        caret: usize,
        scroll: usize,
        mode: DocMode,
        cx: &mut Context<Self>,
    ) {
        if self.doc.mode() != mode
            && !(mode == DocMode::Markdown
                && self.doc.len() as u64 > tachyon_doc::MARKDOWN_SIZE_LIMIT)
        {
            self.retag_to(mode, cx);
        }
        let len = self.doc.len();
        let caret = floor_char_boundary(self.doc.buffer().rope(), caret.min(len));
        self.move_to(caret, false, cx);
        if let Some(index) = self.doc.block_at(scroll.min(len)) {
            self.list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
        }
        cx.notify();
    }

    /// Shows a one-line notice at the top of the view (`render::mode_notice`), the same banner
    /// `disk::oversized_markdown_notice` uses - here, the session restore's missing-files list.
    pub fn set_notice(&mut self, notice: gpui::SharedString, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WindowState {
        WindowState {
            target: Target::File(PathBuf::from("/home/user/notes.md")),
            bounds: bounds(point(px(12.), px(34.)), size(px(900.), px(1000.))),
            maximized: false,
            caret: 42,
            scroll: 7,
            mode: DocMode::Markdown,
        }
    }

    #[test]
    fn round_trips_file_and_backup_windows() {
        let backup = WindowState {
            target: Target::Backup(PathBuf::from("/state/backups/tachyon/000123-0001.md")),
            bounds: bounds(point(px(0.), px(0.)), size(px(500.), px(600.))),
            maximized: true,
            caret: 3,
            scroll: 0,
            mode: DocMode::Plain,
        };
        let windows = vec![sample(), backup];
        let text = {
            let mut out = format!("version = {SESSION_VERSION}\n");
            for w in &windows {
                out.push('\0');
                out.push_str(&serialize_window(w));
            }
            out
        };
        assert_eq!(parse_session(&text), windows);
    }

    #[test]
    fn a_path_with_embedded_newlines_round_trips() {
        let odd = WindowState { target: Target::File(PathBuf::from("/odd\nname.md")), ..sample() };
        let text = format!("version = {SESSION_VERSION}\n\0{}", serialize_window(&odd));
        assert_eq!(parse_session(&text), vec![odd]);
    }

    #[test]
    fn an_unknown_version_is_ignored() {
        let text = format!("version = 2\n\0{}", serialize_window(&sample()));
        assert_eq!(parse_session(&text), Vec::new());
    }

    #[test]
    fn a_truncated_record_is_dropped_without_panicking() {
        let full = serialize_window(&sample());
        let cut = &full[..full.len() / 2];
        let text = format!("version = {SESSION_VERSION}\n\0{cut}");
        assert_eq!(parse_session(&text), Vec::new());
    }

    #[test]
    fn garbage_is_ignored_without_panicking() {
        assert_eq!(parse_session("not a session file at all\0\0garbage\n\nmore"), Vec::new());
        assert_eq!(parse_session(""), Vec::new());
    }

    #[test]
    fn one_bad_record_does_not_lose_the_rest() {
        let good = sample();
        let text = format!(
            "version = {SESSION_VERSION}\n\0kind = file\nnonsense\0{}",
            serialize_window(&good)
        );
        assert_eq!(parse_session(&text), vec![good]);
    }

    #[test]
    fn more_than_the_cap_is_truncated() {
        let text = {
            let mut out = format!("version = {SESSION_VERSION}\n");
            for _ in 0..MAX_SESSION_WINDOWS + 10 {
                out.push('\0');
                out.push_str(&serialize_window(&sample()));
            }
            out
        };
        assert_eq!(parse_session(&text).len(), MAX_SESSION_WINDOWS);
    }

    #[test]
    fn a_huge_file_is_ignored_without_reading_it() {
        let dir = std::env::temp_dir().join(format!("tachyon-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge.txt");
        // Sparse: does not actually allocate the bytes on disk, only its reported length.
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_SESSION_FILE_BYTES + 1).unwrap();
        assert_eq!(read_session(&path), Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_yields_no_windows() {
        assert_eq!(read_session(Path::new("/does/not/exist/session.txt")), Vec::new());
    }

    #[test]
    fn writes_and_reads_back_through_a_real_file() {
        let dir =
            std::env::temp_dir().join(format!("tachyon-session-test-rw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.txt");
        let windows = vec![sample()];
        write_session(&path, &windows).unwrap();
        assert_eq!(read_session(&path), windows);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
