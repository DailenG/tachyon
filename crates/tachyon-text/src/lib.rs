//! The editor's source of truth: a rope of Markdown text with a versioned edit
//! log, grouped undo/redo and UTF-8 ↔ UTF-16 offset mapping.
//!
//! All offsets are byte offsets into LF-normalized text and must lie on char
//! boundaries. This crate must never depend on GPUI.

mod line_ending;
mod load;

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read};
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

/// Text made ready for [`Buffer::edit_prepared`]: line endings normalized
/// (as [`Buffer::edit`] does) and the rope built. `Send`; build it anywhere.
#[derive(Clone, Debug)]
pub struct PreparedText {
    text: String,
    rope: Rope,
}

impl PreparedText {
    pub fn new(text: &str) -> Self {
        let text = normalize(text.strip_suffix('\r').unwrap_or(text)).into_owned();
        let rope = Rope::from_str(&text);
        Self { text, rope }
    }

    /// The normalized text, as it will be inserted.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// A replacement recorded for undo: `old` was replaced by `new` at `at`.
#[derive(Debug)]
struct Change {
    at: usize,
    old: String,
    new: String,
}

/// One undo group with a globally unique, ever-increasing id: assigned when the group is first
/// pushed to [`History::undo`] and carried along as it moves between the undo and redo stacks
/// (an undone group moves to `redo` keeping its id; a redone one moves back to `undo` keeping
/// it), never reused. Because a fresh edit clears `redo` (see [`Buffer::record`]), no id is ever
/// assigned twice to different content: [`Buffer::history_position`] compares these ids to tell
/// "undo/redo landed back on the version last saved" from "still a different edit", in O(1)
/// without reading the buffer's text.
#[derive(Debug)]
struct Group {
    id: u64,
    changes: Vec<Change>,
}

#[derive(Debug, Default)]
struct History {
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// Whether the next edit joins the last undo group.
    open: bool,
    /// A group popped from `undo` or `redo` but not yet fully replayed by
    /// [`Buffer::undo_step`]/[`Buffer::redo_step`]: the changes plus which
    /// stack the group returns to once every one of them has been applied.
    /// Lets a caller (`tachyon_doc::Document`) react to each change against
    /// the buffer state it actually produces - the same interleaving
    /// `Buffer::edit` gives a typed keystroke - instead of the whole
    /// group's final state, which is what applying the group first and
    /// reacting after used to do.
    replay: Option<Replay>,
    /// The id the next newly started group receives; strictly increasing, never reused.
    next_id: u64,
}

#[derive(Debug)]
struct Replay {
    id: u64,
    group: Vec<Change>,
    /// Index of the next change [`Buffer::undo_step`]/[`Buffer::redo_step`]
    /// applies: counts down from `group.len()` for a group undone (changes
    /// revert newest first), up from `0` for one redone (oldest first).
    cursor: usize,
    is_undo: bool,
}

/// What [`Buffer::load`] found while decoding: whether any invalid UTF-8 was replaced with
/// U+FFFD, and whether a NUL byte turned up in the first few KiB (a strong binary-file signal).
/// Either means saving without confirmation would silently change the file's bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
    pub lossy: bool,
    pub looks_binary: bool,
}

/// Builds the text as it should be written to disk (original line ending restored) from a rope
/// and line ending alone, without a [`Buffer`]: `Buffer::rope().clone()` is O(1) (`ropey` shares
/// nodes across the clone via reference counting), so a caller can snapshot a buffer on the UI
/// thread and run this - `Rope::to_string` plus the CRLF pass, `O(document size)` - on a
/// background executor instead. [`Buffer::to_saved_text`] is this on its own rope.
pub fn saved_text(rope: &Rope, line_ending: LineEnding) -> String {
    let text = rope.to_string();
    match line_ending {
        LineEnding::Lf => text,
        LineEnding::CrLf => text.replace('\n', "\r\n"),
    }
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

    /// Builds a buffer by reading `reader` in bounded chunks, so the whole input is never held
    /// as one contiguous `String`: peak memory is the rope's own storage plus one read-sized
    /// buffer, instead of a full copy of the text plus the rope built from it (`Buffer::new` on
    /// text already read into memory). Line endings are normalized and the dominant one detected
    /// in the same streamed pass; invalid UTF-8 is replaced with U+FFFD rather than failing, so a
    /// binary or foreign-encoded file still opens (see [`LoadReport`]).
    pub fn load(reader: impl Read) -> io::Result<(Self, LoadReport)> {
        Self::load_with_chunk(reader, load::READ_CHUNK)
    }

    /// [`Buffer::load`] with an explicit read-buffer size, so tests can exercise chunk-boundary
    /// edge cases (a split multi-byte UTF-8 sequence, the binary sniff window) without a
    /// multi-megabyte input.
    fn load_with_chunk(mut reader: impl Read, chunk_size: usize) -> io::Result<(Self, LoadReport)> {
        let mut builder = ropey::RopeBuilder::new();
        let mut read_buf = vec![0u8; chunk_size];
        let mut chunk: Vec<u8> = Vec::with_capacity(chunk_size + 8);
        let mut carry: Vec<u8> = Vec::new();
        let mut decoded = String::new();
        let mut decoder = load::Decoder::default();
        let mut sniffed = 0usize;
        let mut looks_binary = false;
        loop {
            let n = reader.read(&mut read_buf)?;
            if n == 0 {
                break;
            }
            if sniffed < load::BINARY_SNIFF_LEN {
                let take = (load::BINARY_SNIFF_LEN - sniffed).min(n);
                looks_binary |= read_buf[..take].contains(&0);
                sniffed += take;
            }
            chunk.clear();
            chunk.extend_from_slice(&carry);
            chunk.extend_from_slice(&read_buf[..n]);
            carry.clear();
            let mut rest: &[u8] = &chunk;
            loop {
                match std::str::from_utf8(rest) {
                    Ok(s) => {
                        decoder.push(&mut decoded, s);
                        break;
                    }
                    Err(e) => {
                        let valid_up_to = e.valid_up_to();
                        // SAFETY: `from_utf8` reports `rest` is valid UTF-8 up to `valid_up_to`,
                        // so this prefix is exactly that valid text.
                        let valid = unsafe { std::str::from_utf8_unchecked(&rest[..valid_up_to]) };
                        decoder.push(&mut decoded, valid);
                        match e.error_len() {
                            Some(len) => {
                                decoder.lossy = true;
                                decoded.push('\u{FFFD}');
                                rest = &rest[valid_up_to + len..];
                            }
                            None => {
                                // An incomplete sequence at the end of this read: it may
                                // complete with the next one, so carry it over instead of
                                // treating it as invalid yet.
                                carry.extend_from_slice(&rest[valid_up_to..]);
                                break;
                            }
                        }
                    }
                }
            }
            if !decoded.is_empty() {
                builder.append(&decoded);
                decoded.clear();
            }
        }
        if !carry.is_empty() {
            // A multi-byte sequence truncated at end of file: genuinely invalid.
            decoder.lossy = true;
            decoded.push('\u{FFFD}');
        }
        let line_ending = decoder.finish(&mut decoded);
        let lossy = decoder.lossy;
        if !decoded.is_empty() {
            builder.append(&decoded);
        }
        let buffer = Buffer {
            rope: builder.finish(),
            version: 0,
            log: VecDeque::new(),
            line_ending,
            history: History::default(),
        };
        Ok((buffer, LoadReport { lossy, looks_binary }))
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
        saved_text(&self.rope, self.line_ending)
    }

    /// Replaces `range` with `text` (line endings normalized) and records the
    /// change in the current undo group. Leaves the buffer untouched on error.
    ///
    /// A `\r` at the very end of `text` is dropped: it is almost always the
    /// first half of a `\r\n` split across two inserts (streamed output), and
    /// turning it into `\n` would double the line break.
    pub fn edit(&mut self, range: Range<usize>, text: &str) -> Result<Edit, EditError> {
        self.check_range(&range)?;
        let text = normalize(text.strip_suffix('\r').unwrap_or(text));
        let old = self.rope.byte_slice(range.clone()).to_string();
        let edit = self.apply(range.clone(), &text);
        self.record(Change { at: range.start, old, new: text.into_owned() });
        Ok(edit)
    }

    /// [`Buffer::edit`] with text prepared elsewhere (usually off the UI
    /// thread): splicing the prepared rope in is O(log n), where inserting
    /// megabytes of text costs milliseconds.
    pub fn edit_prepared(
        &mut self,
        range: Range<usize>,
        prepared: PreparedText,
    ) -> Result<Edit, EditError> {
        self.check_range(&range)?;
        let old = self.rope.byte_slice(range.clone()).to_string();
        let start = self.rope.byte_to_char(range.start);
        let end = self.rope.byte_to_char(range.end);
        self.rope.remove(start..end);
        let tail = self.rope.split_off(start);
        self.rope.append(prepared.rope);
        self.rope.append(tail);
        let edit = self.log_edit(Edit { range: range.clone(), new_len: prepared.text.len() });
        self.record(Change { at: range.start, old, new: prepared.text });
        Ok(edit)
    }

    fn record(&mut self, change: Change) {
        let history = &mut self.history;
        history.redo.clear();
        match history.undo.last_mut() {
            Some(group) if history.open => group.changes.push(change),
            _ => {
                history.next_id += 1;
                history.undo.push(Group { id: history.next_id, changes: vec![change] });
            }
        }
        history.open = true;
    }

    /// A stable position in the buffer's linear undo/redo history: the id of the group last
    /// applied (undone groups sitting on `redo` do not count), or `0` if none has ever been
    /// applied, or every group has been undone back past the start. Comparing two positions is
    /// an integer equality check - no text is read - so callers like `Editor::is_modified` can
    /// afford it on every keystroke, even on a huge buffer.
    ///
    /// A fresh edit always discards `redo` (see [`Buffer::record`]), so an id, once it stops
    /// being reachable, is never assigned to different content: two positions compare equal only
    /// when undo/redo landed back on the exact edit group, never merely on the same *count* of
    /// groups after the history has since diverged. Retyping the same text by hand instead of
    /// redoing it is a new group with a new id, so it compares unequal even though the text
    /// matches - deliberate: it is a fresh edit, not a return to a known point in the history.
    ///
    /// Mid-replay (a group partly applied by [`Buffer::undo_step`]/[`Buffer::redo_step`]) this is
    /// `u64::MAX`, a value no save ever captures: [`Buffer::undo`]/[`Buffer::redo`] always finish
    /// a group in one call, so callers that only use those never observe it.
    pub fn history_position(&self) -> u64 {
        if self.history.replay.is_some() {
            return u64::MAX;
        }
        self.history.undo.last().map_or(0, |group| group.id)
    }

    /// Ends the current undo group; the next edit starts a new one. The editor
    /// decides grouping policy (pauses, cursor jumps, word boundaries).
    pub fn seal_undo_group(&mut self) {
        self.history.open = false;
    }

    /// Reverts the last undo group, applying every change in one call.
    /// Returns the edits applied, in order. Equivalent to calling
    /// [`Buffer::undo_step`] until it reports the group's last step;
    /// prefer that when a caller needs to react (re-chunk, reparse) to each
    /// change against the buffer state it individually produced, rather
    /// than the group's final state.
    pub fn undo(&mut self) -> Option<Vec<Edit>> {
        let mut edits = Vec::new();
        loop {
            let (edit, done) = self.undo_step()?;
            edits.push(edit);
            if done {
                return Some(edits);
            }
        }
    }

    /// Re-applies the last undone group, applying every change in one call.
    /// Returns the edits applied, in order. Equivalent to calling
    /// [`Buffer::redo_step`] until it reports the group's last step; see
    /// its docs for why a caller may prefer that.
    pub fn redo(&mut self) -> Option<Vec<Edit>> {
        let mut edits = Vec::new();
        loop {
            let (edit, done) = self.redo_step()?;
            edits.push(edit);
            if done {
                return Some(edits);
            }
        }
    }

    /// Reverts one change of the last undo group and returns its edit and
    /// whether it was the group's last change (the group has now moved to
    /// the redo stack), or `None` if there is nothing left to undo. Call in
    /// a loop until `true`; do not interleave with
    /// [`Buffer::edit`]/[`Buffer::redo_step`] before a group reports `true`.
    ///
    /// Applying one change at a time - instead of the whole group, then
    /// reporting all its edits together - lets a caller like
    /// `tachyon_doc::Document` re-chunk or reparse after each change
    /// against the buffer state that change actually produced, the same
    /// interleaving a typed keystroke gets from [`Buffer::edit`]. Reacting
    /// only after the whole group had already been replayed made a plain
    /// document's re-chunk (`on_edit_plain`'s convergence search, which
    /// compares a per-change shift against the current rope) compare against
    /// a rope that had already moved past every other change in the group,
    /// so it never converged and rescanned the entire document once per
    /// change in the group instead of once per edited region.
    pub fn undo_step(&mut self) -> Option<(Edit, bool)> {
        self.history.open = false;
        let mut replay = match self.history.replay.take() {
            Some(replay) => replay,
            None => {
                let group = self.history.undo.pop()?;
                let cursor = group.changes.len();
                Replay { id: group.id, group: group.changes, cursor, is_undo: true }
            }
        };
        debug_assert!(replay.is_undo, "redo_step left a group only partly replayed");
        replay.cursor -= 1;
        let c = &replay.group[replay.cursor];
        let edit = self.apply(c.at..c.at + c.new.len(), &c.old);
        let done = replay.cursor == 0;
        if done {
            self.history.redo.push(Group { id: replay.id, changes: replay.group });
        } else {
            self.history.replay = Some(replay);
        }
        Some((edit, done))
    }

    /// Re-applies one change of the last undone group and returns its edit
    /// and whether it was the group's last change (the group has now moved
    /// back to the undo stack), or `None` if there is nothing left to redo.
    /// See [`Buffer::undo_step`] for why a caller may prefer this over
    /// [`Buffer::redo`].
    pub fn redo_step(&mut self) -> Option<(Edit, bool)> {
        self.history.open = false;
        let mut replay = match self.history.replay.take() {
            Some(replay) => replay,
            None => {
                let group = self.history.redo.pop()?;
                Replay { id: group.id, group: group.changes, cursor: 0, is_undo: false }
            }
        };
        debug_assert!(!replay.is_undo, "undo_step left a group only partly replayed");
        let c = &replay.group[replay.cursor];
        let edit = self.apply(c.at..c.at + c.old.len(), &c.new);
        replay.cursor += 1;
        let done = replay.cursor == replay.group.len();
        if done {
            self.history.undo.push(Group { id: replay.id, changes: replay.group });
        } else {
            self.history.replay = Some(replay);
        }
        Some((edit, done))
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
        self.log_edit(Edit { range, new_len: text.len() })
    }

    fn log_edit(&mut self, edit: Edit) -> Edit {
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
    fn a_prepared_edit_matches_a_plain_one_and_undoes_the_same() {
        let text = "pasted\r\nline\n\u{e9}\r";
        let mut plain = Buffer::new("ab\u{1f600}cd\n");
        let mut prepared = Buffer::new("ab\u{1f600}cd\n");
        let at = 2..6; // replaces the emoji
        let edit = plain.edit(at.clone(), text).unwrap();
        assert_eq!(prepared.edit_prepared(at, PreparedText::new(text)).unwrap(), edit);
        assert_eq!(prepared.text(), plain.text());
        assert_eq!(prepared.version(), plain.version());
        assert_eq!(prepared.undo(), plain.undo());
        assert_eq!(prepared.text(), "ab\u{1f600}cd\n");
        assert_eq!(prepared.redo(), plain.redo());
        assert_eq!(prepared.text(), plain.text());
        let mut emoji = Buffer::new("\u{1f600}");
        assert!(emoji.edit_prepared(1..1, PreparedText::new("x")).is_err(), "inside the emoji");
        assert_eq!(emoji.text(), "\u{1f600}");
    }

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
    fn crlf_split_across_inserts_is_one_line_break() {
        let mut buffer = Buffer::new("");
        for chunk in ["line one\r", "\nline two\r", "\n"] {
            buffer.edit(buffer.len()..buffer.len(), chunk).unwrap();
        }
        assert_eq!(buffer.text(), "line one\nline two\n");
    }

    #[test]
    fn load_matches_new_for_a_variety_of_inputs_at_small_chunk_sizes() {
        let inputs = [
            "",
            "no line breaks",
            "a\r\nb\r\nc\n",
            "line one\r\nline two\r\nend without break",
            "unicode: héllo \u{1f600} wörld\n",
            "trailing lone cr\r",
        ];
        for text in inputs {
            for chunk_size in [1, 2, 3, 7] {
                let (loaded, report) =
                    Buffer::load_with_chunk(io::Cursor::new(text.as_bytes()), chunk_size).unwrap();
                assert!(!report.lossy, "{text:?} at chunk {chunk_size}");
                assert!(!report.looks_binary, "{text:?} at chunk {chunk_size}");
                let direct = Buffer::new(text);
                assert_eq!(loaded.text(), direct.text(), "{text:?} at chunk {chunk_size}");
                assert_eq!(
                    loaded.line_ending(),
                    direct.line_ending(),
                    "{text:?} at chunk {chunk_size}"
                );
            }
        }
    }

    #[test]
    fn load_replaces_invalid_utf8_even_when_split_across_reads() {
        // `b"caf\xC3\xA9"` is "café"; splitting the 2-byte 'é' across a read boundary must not
        // count it as invalid, but a lone continuation byte must.
        let valid = b"caf\xC3\xA9 after\n";
        let (buffer, report) = Buffer::load_with_chunk(io::Cursor::new(valid), 4).unwrap();
        assert!(!report.lossy);
        assert_eq!(buffer.text(), "café after\n");

        let invalid = b"before \xff\xfe after\n";
        let (buffer, report) = Buffer::load_with_chunk(io::Cursor::new(invalid), 3).unwrap();
        assert!(report.lossy);
        assert_eq!(buffer.text(), "before \u{FFFD}\u{FFFD} after\n");
    }

    #[test]
    fn load_flags_a_nul_byte_in_the_first_8_kib_as_binary() {
        let mut bytes = vec![b'a'; 100];
        bytes[50] = 0;
        let (_, report) = Buffer::load(io::Cursor::new(bytes)).unwrap();
        assert!(report.looks_binary);

        let (_, report) = Buffer::load(io::Cursor::new(vec![b'a'; 100])).unwrap();
        assert!(!report.looks_binary);
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
    fn history_position_returns_to_the_same_value_on_undo_and_redo_but_not_on_a_retyped_edit() {
        let mut buffer = Buffer::new("");
        let start = buffer.history_position();
        buffer.edit(0..0, "a").unwrap();
        let after_a = buffer.history_position();
        assert_ne!(after_a, start, "a new group is a new position");
        buffer.seal_undo_group();
        buffer.edit(1..1, "b").unwrap();
        let after_b = buffer.history_position();
        assert_ne!(after_b, after_a);

        buffer.undo().unwrap();
        assert_eq!(buffer.text(), "a");
        assert_eq!(buffer.history_position(), after_a, "undo lands back on the same group");

        buffer.redo().unwrap();
        assert_eq!(buffer.text(), "ab");
        assert_eq!(buffer.history_position(), after_b, "redo lands back on the same group");

        buffer.undo().unwrap();
        buffer.undo().unwrap();
        assert_eq!(buffer.text(), "");
        assert_eq!(buffer.history_position(), start, "fully undone matches the starting position");

        // Retyping the same text by hand, instead of redoing it, is a new group: same text, a
        // different position (the caller decides whether that counts as "modified"; here it
        // deliberately does not match `after_b`, even though the buffer holds the same "ab").
        buffer.edit(0..0, "a").unwrap();
        buffer.seal_undo_group();
        buffer.edit(1..1, "b").unwrap();
        assert_eq!(buffer.text(), "ab");
        assert_ne!(buffer.history_position(), after_b, "a hand-retyped edit is a new group");
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
