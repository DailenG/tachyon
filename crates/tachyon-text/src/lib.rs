//! The editor's source of truth: a rope of Markdown text with a versioned edit
//! log, grouped undo/redo and UTF-8 ↔ UTF-16 offset mapping.
//!
//! All offsets are byte offsets into LF-normalized text and must lie on char
//! boundaries. This crate must never depend on GPUI.

mod line_ending;

use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;

use ropey::Rope;

pub use crate::line_ending::{LineEnding, normalize};

/// Number of edits kept for [`Buffer::edits_since`]. Background results older
/// than this cannot be rebased and are recomputed instead.
pub const EDIT_LOG_CAPACITY: usize = 4096;

/// One replacement, in the coordinates of the text *before* it was applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub new_len: usize,
}

impl Edit {
    /// The replaced range in the coordinates of the text *after* the edit.
    pub fn new_range(&self) -> Range<usize> {
        self.range.start..self.range.start + self.new_len
    }

    /// Maps an offset from before the edit to after it. Offsets inside the
    /// replaced range move to its end.
    pub fn map_offset(&self, offset: usize) -> usize {
        if offset < self.range.start {
            offset
        } else if offset >= self.range.end {
            offset - self.range.len() + self.new_len
        } else {
            self.range.start + self.new_len
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    OutOfBounds { range: Range<usize>, len: usize },
    NotCharBoundary { offset: usize },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::OutOfBounds { range, len } => {
                write!(f, "range {range:?} is out of bounds for text of length {len}")
            }
            EditError::NotCharBoundary { offset } => {
                write!(f, "offset {offset} is not on a char boundary")
            }
        }
    }
}

impl std::error::Error for EditError {}

/// Immutable, cheaply cloned view of the text at one version. Safe to send to
/// background threads.
#[derive(Clone, Debug)]
pub struct TextSnapshot {
    rope: Rope,
    version: u64,
}

impl TextSnapshot {
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// Copies `range` out as a contiguous string (parsers need `&str`).
    ///
    /// # Panics
    /// If `range` is out of bounds or not on char boundaries.
    pub fn text_for_range(&self, range: Range<usize>) -> String {
        self.rope.byte_slice(range).to_string()
    }
}

/// A replacement recorded for undo: `old` was replaced by `new` at `at`.
#[derive(Debug)]
struct Change {
    at: usize,
    old: String,
    new: String,
}

#[derive(Debug, Default)]
struct History {
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
    /// Whether the next edit joins the last undo group.
    open: bool,
}

#[derive(Debug)]
pub struct Buffer {
    rope: Rope,
    version: u64,
    log: VecDeque<Edit>,
    line_ending: LineEnding,
    history: History,
}

impl Buffer {
    /// Creates a buffer from file or clipboard text. Line endings are
    /// normalized to `\n`; the dominant original ending is kept for saving.
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(&normalize(text)),
            version: 0,
            log: VecDeque::new(),
            line_ending: LineEnding::detect(text),
            history: History::default(),
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn snapshot(&self) -> TextSnapshot {
        TextSnapshot { rope: self.rope.clone(), version: self.version }
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// The text as it should be written to disk, with the original line
    /// ending restored.
    pub fn to_saved_text(&self) -> String {
        let text = self.rope.to_string();
        match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::CrLf => text.replace('\n', "\r\n"),
        }
    }

    /// Replaces `range` with `text` (line endings normalized) and records the
    /// change in the current undo group. Leaves the buffer untouched on error.
    pub fn edit(&mut self, range: Range<usize>, text: &str) -> Result<Edit, EditError> {
        self.check_range(&range)?;
        let text = normalize(text);
        let old = self.rope.byte_slice(range.clone()).to_string();
        let edit = self.apply(range.clone(), &text);

        let history = &mut self.history;
        history.redo.clear();
        let change = Change { at: range.start, old, new: text.into_owned() };
        match history.undo.last_mut() {
            Some(group) if history.open => group.push(change),
            _ => history.undo.push(vec![change]),
        }
        history.open = true;
        Ok(edit)
    }

    /// Ends the current undo group; the next edit starts a new one. The editor
    /// decides grouping policy (pauses, cursor jumps, word boundaries).
    pub fn seal_undo_group(&mut self) {
        self.history.open = false;
    }

    /// Reverts the last undo group. Returns the edits applied, in order.
    pub fn undo(&mut self) -> Option<Vec<Edit>> {
        self.history.open = false;
        let group = self.history.undo.pop()?;
        let edits =
            group.iter().rev().map(|c| self.apply(c.at..c.at + c.new.len(), &c.old)).collect();
        self.history.redo.push(group);
        Some(edits)
    }

    /// Re-applies the last undone group. Returns the edits applied, in order.
    pub fn redo(&mut self) -> Option<Vec<Edit>> {
        self.history.open = false;
        let group = self.history.redo.pop()?;
        let edits = group.iter().map(|c| self.apply(c.at..c.at + c.old.len(), &c.new)).collect();
        self.history.undo.push(group);
        Some(edits)
    }

    /// Edits applied after `version`, oldest first, or `None` if they have
    /// fallen out of the log (or `version` is in the future).
    pub fn edits_since(&self, version: u64) -> Option<impl Iterator<Item = &Edit>> {
        let oldest = self.version - self.log.len() as u64;
        if version < oldest || version > self.version {
            return None;
        }
        Some(self.log.iter().skip((version - oldest) as usize))
    }

    /// UTF-16 code-unit offset of a byte offset (IME and platform text APIs
    /// use UTF-16). Offsets past the end clamp to the end.
    pub fn byte_to_utf16(&self, byte: usize) -> usize {
        let char_idx = self.rope.byte_to_char(byte.min(self.len()));
        self.rope.char_to_utf16_cu(char_idx)
    }

    /// Byte offset of a UTF-16 offset. An offset inside a surrogate pair maps
    /// to the start of that char; offsets past the end clamp to the end.
    pub fn utf16_to_byte(&self, utf16: usize) -> usize {
        let char_idx = self.rope.utf16_cu_to_char(utf16.min(self.rope.len_utf16_cu()));
        self.rope.char_to_byte(char_idx)
    }

    fn check_range(&self, range: &Range<usize>) -> Result<(), EditError> {
        let len = self.len();
        if range.start > range.end || range.end > len {
            return Err(EditError::OutOfBounds { range: range.clone(), len });
        }
        for offset in [range.start, range.end] {
            if self.rope.char_to_byte(self.rope.byte_to_char(offset)) != offset {
                return Err(EditError::NotCharBoundary { offset });
            }
        }
        Ok(())
    }

    /// Applies a validated, normalized replacement and logs it.
    fn apply(&mut self, range: Range<usize>, text: &str) -> Edit {
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.remove(start..end);
        self.rope.insert(start, text);

        let edit = Edit { range, new_len: text.len() };
        if self.log.len() == EDIT_LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(edit.clone());
        self.version += 1;
        edit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_input_is_normalized_and_restored_on_save() {
        let mut buffer = Buffer::new("# Title\r\n\r\nBody\r\n");
        assert_eq!(buffer.text(), "# Title\n\nBody\n");
        assert_eq!(buffer.line_ending(), LineEnding::CrLf);

        buffer.edit(buffer.len()..buffer.len(), "pasted\r\nline\n").unwrap();
        assert_eq!(buffer.text(), "# Title\n\nBody\npasted\nline\n");
        assert_eq!(buffer.to_saved_text(), "# Title\r\n\r\nBody\r\npasted\r\nline\r\n");
    }

    #[test]
    fn invalid_edits_leave_the_buffer_untouched() {
        let mut buffer = Buffer::new("héllo");
        assert_eq!(
            buffer.edit(2..3, "x"),
            Err(EditError::NotCharBoundary { offset: 2 }),
            "offset 2 is inside 'é'"
        );
        assert_eq!(buffer.edit(4..9, ""), Err(EditError::OutOfBounds { range: 4..9, len: 6 }));
        assert_eq!(buffer.version(), 0);
        assert_eq!(buffer.text(), "héllo");
    }

    #[test]
    fn edit_reports_old_and_new_ranges() {
        let mut buffer = Buffer::new("hello world");
        let edit = buffer.edit(6..11, "Tachyon!").unwrap();
        assert_eq!(edit, Edit { range: 6..11, new_len: 8 });
        assert_eq!(edit.new_range(), 6..14);
        assert_eq!(buffer.text(), "hello Tachyon!");
        assert_eq!((edit.map_offset(3), edit.map_offset(8), edit.map_offset(11)), (3, 14, 14));
    }

    #[test]
    fn undo_and_redo_follow_groups() {
        let mut buffer = Buffer::new("");
        buffer.edit(0..0, "a").unwrap();
        buffer.edit(1..1, "b").unwrap();
        buffer.seal_undo_group();
        buffer.edit(2..2, "c").unwrap();

        assert_eq!(buffer.undo().unwrap(), vec![Edit { range: 2..3, new_len: 0 }]);
        assert_eq!(buffer.text(), "ab");
        assert_eq!(buffer.undo().unwrap().len(), 2);
        assert_eq!(buffer.text(), "");
        assert!(buffer.undo().is_none());

        buffer.redo().unwrap();
        assert_eq!(buffer.text(), "ab");
        buffer.edit(2..2, "X").unwrap();
        assert!(buffer.redo().is_none(), "a new edit discards the redo stack");
        assert_eq!(buffer.text(), "abX");
    }

    #[test]
    fn undo_seals_the_group_so_later_edits_do_not_join_it() {
        let mut buffer = Buffer::new("");
        buffer.edit(0..0, "a").unwrap();
        buffer.undo().unwrap();
        buffer.edit(0..0, "b").unwrap();
        buffer.edit(1..1, "c").unwrap();
        buffer.undo().unwrap();
        assert_eq!(buffer.text(), "");
    }

    #[test]
    fn edits_since_covers_undo_and_expires_with_the_log() {
        let mut buffer = Buffer::new("abc");
        let v0 = buffer.version();
        buffer.edit(0..1, "").unwrap();
        buffer.undo().unwrap();
        let since: Vec<_> = buffer.edits_since(v0).unwrap().cloned().collect();
        assert_eq!(since, vec![Edit { range: 0..1, new_len: 0 }, Edit { range: 0..0, new_len: 1 }]);
        assert_eq!(buffer.edits_since(buffer.version()).unwrap().count(), 0);
        assert!(buffer.edits_since(buffer.version() + 1).is_none());

        for _ in 0..EDIT_LOG_CAPACITY {
            buffer.edit(0..0, "x").unwrap();
        }
        assert!(buffer.edits_since(v0).is_none());
        assert_eq!(buffer.edits_since(buffer.version() - 10).unwrap().count(), 10);
    }

    #[test]
    fn utf16_mapping_handles_surrogate_pairs() {
        // "a" (1 byte, 1 cu), "é" (2, 1), "😀" (4, 2), "中" (3, 1)
        let buffer = Buffer::new("aé😀中");
        let pairs = [(0, 0), (1, 1), (3, 2), (7, 4), (10, 5)];
        for (byte, utf16) in pairs {
            assert_eq!(buffer.byte_to_utf16(byte), utf16, "byte {byte}");
            assert_eq!(buffer.utf16_to_byte(utf16), byte, "utf16 {utf16}");
        }
        assert_eq!(buffer.utf16_to_byte(3), 3, "inside a surrogate pair maps to the char start");
        assert_eq!(buffer.utf16_to_byte(99), 10);
        assert_eq!(buffer.byte_to_utf16(99), 5);
    }
}
