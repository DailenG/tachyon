//! Document state: the text [`Buffer`] plus a block map that always tiles the
//! current text, kept up to date by incremental reparsing.
//!
//! Edits apply immediately: the touched blocks merge into one *stale* block
//! (current length, outdated IR) and their range is marked dirty. Reparsing
//! is split into [`ParseJob`]s so the caller decides where they run: small
//! jobs inline on the UI thread, large ones on a background executor. A job
//! owns an immutable snapshot and is `Send`; its result is applied back with
//! [`Document::apply`], which discards it if an edit touched its window in
//! the meantime.
//!
//! Invariant (checked by property tests): once no dirty ranges remain, the
//! blocks equal a from-scratch parse of the text.
//!
//! This crate must never depend on GPUI.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use ropey::Rope;
use tachyon_md::{self as md, DefTable, ParsedBlock};
use tachyon_text::{Buffer, Edit, EditError, PreparedText};

/// Jobs whose window is at most this many bytes are cheap enough to run on
/// the UI thread (tens of microseconds).
pub const SYNC_PARSE_LIMIT: usize = 32 * 1024;

/// Stale blocks larger than this are split into provisional unparsed blocks
/// so they can be shown before the background parse finishes.
pub const UNPARSED_SPLIT_THRESHOLD: usize = 64 * 1024;

/// Minimum size of those provisional blocks.
pub const UNPARSED_CHUNK: usize = 8 * 1024;

/// Within this distance of either end of the changed range, provisional
/// blocks are at least [`UNPARSED_EDGE_CHUNK`] instead. The caret lands at
/// the end of a paste, so the first frame lays out the last blocks; each is
/// laid out whole even if only part of it is visible.
pub const UNPARSED_EDGE: usize = 32 * 1024;

/// Minimum size of provisional blocks near the ends of the changed range.
pub const UNPARSED_EDGE_CHUNK: usize = 1024;

/// Window size for streaming a large dirty range back in pieces with
/// [`Document::parse_job_near`]: a few milliseconds of parsing, so the text
/// near the viewport is formatted within a frame or two of a large paste.
pub const PARSE_CHUNK: usize = 128 * 1024;

/// [`Document::find_all`] stops after this many matches.
pub const MAX_FIND_MATCHES: usize = 10_000;

/// A plain-text chunk targets this many lines or [`PLAIN_CHUNK_BYTES`], whichever comes first
/// (see [`plain_chunk_lens`]).
pub const PLAIN_CHUNK_LINES: usize = 256;

/// A plain-text chunk targets this many bytes or [`PLAIN_CHUNK_LINES`] lines, whichever comes
/// first. Purely a virtualization granularity: the visible list only lays out and shapes the
/// chunks on screen, never the whole document.
pub const PLAIN_CHUNK_BYTES: usize = 16 * 1024;

/// A single line with no `\n` for this many bytes is cut anyway (a display-only break: the
/// underlying text is unchanged). Smaller than [`PLAIN_CHUNK_BYTES`] so a pathologically long
/// line - the file is one enormous line, the reported cause of a 6-second frame and 3 GB peak on
/// a 20 MB single-line file - never grows a chunk past a small, bounded shaping cost.
pub const PLAIN_FORCED_CUT_BYTES: usize = 8 * 1024;

/// Below this many lines *and* below [`PLAIN_MIN_CHUNK_BYTES`] bytes (a quarter of
/// [`PLAIN_CHUNK_LINES`]), a plain block that fits in one chunk on its own is too small to stand
/// alone after an edit and merges with the next block instead - unless it is the document's own
/// last block, or immediately follows a forced cut, where a short remainder is normal (see
/// [`Document::on_edit_plain`] and [`plain_block_is_small`]).
const PLAIN_MIN_CHUNK_LINES: usize = PLAIN_CHUNK_LINES / 4;

/// A quarter of [`PLAIN_CHUNK_BYTES`]; see [`PLAIN_MIN_CHUNK_LINES`].
const PLAIN_MIN_CHUNK_BYTES: usize = PLAIN_CHUNK_BYTES / 4;

/// [`Document::on_edit_plain`] gives up extending its merge into further neighbours after this
/// many attempts. Never expected to matter - merging one whole neighbour always clears the
/// minimum unless that neighbour is itself an exempt short remainder right after a forced cut,
/// and a single source line has only one such remainder - but bounds the work to a small
/// constant regardless, rather than relying on that argument alone.
const PLAIN_MERGE_ATTEMPTS: usize = 4;

/// Above this size a Markdown-extension file opens as plain text instead (and the toggle back to
/// Markdown is refused): `cargo bench -p tachyon-doc` measured a full parse at 220 ms for 10 MiB;
/// well past that, the parse itself risks becoming the kind of multi-hundred-millisecond stall
/// this feature exists to avoid, and the parsed block IR (style runs, source maps) costs several
/// times the source size in memory. 16 MiB keeps that bounded while covering the overwhelming
/// majority of real Markdown files losslessly.
pub const MARKDOWN_SIZE_LIMIT: u64 = 16 * 1024 * 1024;

/// Above this size, refuse to open the file at all (a clear message document instead): measured
/// peak RSS for a plain document is roughly 4x its size (the rope, its edit log, and headroom for
/// the read buffer and chunk list), so 2 GiB already risks 8+ GiB of resident memory, more than
/// many machines have to spare for one file.
pub const PLAIN_HARD_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// Above this size, a plain document's hot-exit backup is written only on quit or close, not
/// after every pause in typing: writing tens or hundreds of megabytes 1.5 s after every edit
/// would itself compete with the UI thread's budget on a slow disk. (The saved text is always
/// built from a rope snapshot on the background executor, regardless of size - `Rope::clone` is
/// O(1), so there is no size below which that hop is not worth taking - so this limit is purely
/// about how *often* a huge document's backup runs, not where it runs.)
pub const LARGE_PLAIN_SIZE: u64 = 64 * 1024 * 1024;

/// Above this size, the find bar's scan (`Document::find_all`) moves from the UI thread to the
/// background executor: measured (`cargo run --release -p tachyon-doc --example find_bench`,
/// since removed) at ~1.9 ms/MiB for a non-matching query, the worst case (a match ends the scan
/// early), so the scan itself crosses an 8 ms frame budget around 5 MiB. A `generation` counter
/// (`FindState`) drops a background result superseded by a newer edit or query before it lands.
pub const FIND_BACKGROUND_THRESHOLD: u64 = 5 * 1024 * 1024;

/// Extensions (case-insensitive) that open as Markdown; anything else opens as plain text.
const MARKDOWN_EXTENSIONS: [&str; 6] = ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];

/// A document's editing and rendering mode. Plain text never runs Markdown parsing, has no
/// definitions table and never schedules a [`ParseJob`]: its blocks only chunk the text for the
/// virtualized list, at load with [`plain_chunk_lens`] (greedy: pack each chunk to the maximum).
/// An edit instead re-chunks only the block(s) it touched, and - if a boundary chunk of that
/// re-chunk would fall under the minimum - the next block too (see [`Document::on_edit_plain`]):
/// local by construction, never proportional to the rest of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocMode {
    Markdown,
    Plain,
}

/// The mode a file at `path` opens in, judged by its extension alone (case-insensitive): a
/// recognized Markdown extension, else plain text (including no extension at all). Callers still
/// need to apply [`MARKDOWN_SIZE_LIMIT`] once the file's size is known.
pub fn mode_for_extension(path: &Path) -> DocMode {
    let markdown = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| MARKDOWN_EXTENSIONS.iter().any(|m| ext.eq_ignore_ascii_case(m)));
    if markdown { DocMode::Markdown } else { DocMode::Plain }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u64);

#[derive(Clone, Debug)]
pub struct Block {
    id: BlockId,
    len: usize,
    parsed: Arc<ParsedBlock>,
    stale: bool,
}

impl Block {
    /// Identity across edits and reparses: kept while the block is untouched
    /// and while a reparse finds a block starting at the same offset (so the
    /// block being edited keeps its id). Key render caches by
    /// `(id, parsed().source_hash)`.
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// Current length in bytes (may differ from `parsed().len` while stale).
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Last parse of this block. Outdated while [`Block::is_stale`].
    pub fn parsed(&self) -> &ParsedBlock {
        &self.parsed
    }

    /// Shared handle to the last parse, for holding on to it beyond a borrow
    /// of the document (e.g. in UI event handlers).
    pub fn parsed_shared(&self) -> Arc<ParsedBlock> {
        Arc::clone(&self.parsed)
    }

    /// The source changed since the last parse; render it raw.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    fn matches(&self, parsed: &ParsedBlock) -> bool {
        !self.stale
            && self.len == parsed.len
            && self.parsed.source_hash == parsed.source_hash
            && self.parsed.kind == parsed.kind
    }
}

/// The block list changed: blocks at `old` indices were replaced by
/// `new_len` blocks. Drives list virtualization in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Splice {
    pub old: Range<usize>,
    pub new_len: usize,
}

/// Outcome of [`Document::apply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The result replaced its window's blocks.
    Spliced,
    /// An edit touched the window while the job ran; the window is dirty
    /// again and will be reparsed by a later job.
    Discarded,
    /// Not the outstanding job (already applied, cancelled or foreign).
    Ignored,
}

#[derive(Debug)]
struct Outstanding {
    id: u64,
    /// Buffer version the job's snapshot was taken at.
    version: u64,
    /// The job's initial window, in snapshot coordinates.
    window: Range<usize>,
}

pub struct Document {
    buffer: Buffer,
    blocks: Arc<Vec<Block>>,
    /// `starts[i]` is the offset of block `i`; the last entry is the length.
    starts: Vec<usize>,
    defs: Arc<DefTable>,
    /// Sorted, disjoint (possibly empty) ranges awaiting a reparse. Always empty in
    /// [`DocMode::Plain`]: chunking never leaves anything to reparse in the background.
    dirty: Vec<Range<usize>>,
    splices: Vec<Splice>,
    outstanding: Option<Outstanding>,
    /// An edit removed blocks that made definitions; rebuild the table at
    /// the next apply.
    defs_dirty: bool,
    next_block: u64,
    next_job: u64,
    mode: DocMode,
}

impl Document {
    /// Parses `text` as Markdown completely (callers may run this off the UI thread; the
    /// document is `Send`).
    pub fn new(text: &str) -> Self {
        Self::from_buffer(Buffer::new(text), DocMode::Markdown)
    }

    /// Builds a plain-text document from `text`: no Markdown parsing, ever, just chunked for the
    /// virtualized list (see [`plain_chunk_lens`]). For loading a file, prefer
    /// [`Document::from_buffer`] with a [`Buffer::load`]ed buffer, which never copies the whole
    /// text into one `String` the way this constructor (and `Buffer::new`) does.
    pub fn new_plain(text: &str) -> Self {
        Self::from_buffer(Buffer::new(text), DocMode::Plain)
    }

    /// Builds a document from an already-loaded buffer (its text, undo history and version are
    /// kept as is), tiling it into blocks for `mode`.
    pub fn from_buffer(buffer: Buffer, mode: DocMode) -> Self {
        let mut doc = Document {
            buffer,
            blocks: Arc::new(Vec::new()),
            starts: vec![0],
            defs: Arc::new(DefTable::default()),
            dirty: Vec::new(),
            splices: Vec::new(),
            outstanding: None,
            defs_dirty: false,
            next_block: 0,
            next_job: 0,
            mode,
        };
        let blocks: Vec<Block> = match mode {
            DocMode::Markdown => {
                let (parsed, defs) = md::parse_document(&doc.buffer.text());
                doc.defs = Arc::new(defs);
                parsed.into_iter().map(|p| doc.new_block(p, false)).collect()
            }
            DocMode::Plain => {
                let len = doc.buffer.len();
                plain_chunk_lens(doc.buffer.rope(), 0..len)
                    .into_iter()
                    .map(|len| doc.new_block(md::plain(len), false))
                    .collect()
            }
        };
        doc.blocks = Arc::new(blocks);
        doc.recompute_starts();
        doc
    }

    pub fn mode(&self) -> DocMode {
        self.mode
    }

    /// Takes the buffer back out (text, undo history and version all kept), discarding the block
    /// list. Used to rebuild it under a different mode (see [`Document::retagged`]).
    pub fn into_buffer(self) -> Buffer {
        self.buffer
    }

    /// This document's buffer re-tiled under `mode`; the text, undo history and version are
    /// unaffected, so toggling the Markdown/plain-text display mode does not touch
    /// modified/saved state. Rebuilds the whole block list, so callers of a possibly expensive
    /// `mode` (Markdown, whose parse cost is why [`MARKDOWN_SIZE_LIMIT`] exists) may want to run
    /// this off the UI thread for a large document.
    pub fn retagged(self, mode: DocMode) -> Self {
        Self::from_buffer(self.buffer, mode)
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn defs(&self) -> &DefTable {
        &self.defs
    }

    /// Byte range of block `index`.
    ///
    /// # Panics
    /// If `index` is out of bounds.
    pub fn block_range(&self, index: usize) -> Range<usize> {
        self.starts[index]..self.starts[index + 1]
    }

    /// Index of the block containing `offset`; the end of the text belongs to
    /// the last block. `None` for an empty document.
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        if self.blocks.is_empty() {
            return None;
        }
        let idx = self.starts.partition_point(|&s| s <= offset);
        Some(idx.saturating_sub(1).min(self.blocks.len() - 1))
    }

    /// Ranges still awaiting a reparse.
    pub fn dirty_ranges(&self) -> &[Range<usize>] {
        &self.dirty
    }

    /// Byte ranges of `query` in the text, in order, not overlapping, at most
    /// [`MAX_FIND_MATCHES`]. See [`find_all_in_rope`], which does the work: this is just that
    /// applied to the document's own rope.
    pub fn find_all(&self, query: &str) -> Vec<Range<usize>> {
        find_all_in_rope(self.buffer.rope(), query)
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty() || self.outstanding.is_some()
    }

    /// Whether block-list changes are waiting for [`Document::take_splices`].
    pub fn has_splices(&self) -> bool {
        !self.splices.is_empty()
    }

    /// Block-list changes since the last call, in order.
    pub fn take_splices(&mut self) -> Vec<Splice> {
        std::mem::take(&mut self.splices)
    }

    pub fn edit(&mut self, range: Range<usize>, text: &str) -> Result<Edit, EditError> {
        let edit = self.buffer.edit(range, text)?;
        self.on_edit(&edit, None);
        Ok(edit)
    }

    /// [`Document::edit`] with text prepared off the UI thread
    /// ([`PreparedInsert::new`]): the result is the same, but the rope is
    /// spliced in and the provisional blocks reuse the prepared boundaries,
    /// so a multi-megabyte paste costs well under a millisecond here.
    pub fn edit_prepared(
        &mut self,
        range: Range<usize>,
        insert: PreparedInsert,
    ) -> Result<Edit, EditError> {
        let edit = self.buffer.edit_prepared(range, insert.text)?;
        self.on_edit(&edit, Some(&insert.cuts));
        Ok(edit)
    }

    pub fn seal_undo_group(&mut self) {
        self.buffer.seal_undo_group();
    }

    /// Reverts the last undo group. Reconciles the block list after each
    /// individual change (`Buffer::undo_step`), not after the whole group:
    /// `on_edit`/`on_edit_plain` assume the buffer reflects exactly the one
    /// edit just passed to them, which only holds if each change is applied
    /// and reconciled before the next one runs. Reconciling once after the
    /// group was fully applied (its `Buffer::undo`, still used by callers
    /// that do not need this) made a plain document's re-chunk compare
    /// against a rope that had already moved past every other change in
    /// the group, so it never found the old chunk boundary it was looking
    /// for and rescanned all the way to the end of the document, once per
    /// change in the group.
    pub fn undo(&mut self) -> Option<Vec<Edit>> {
        let mut edits = Vec::new();
        loop {
            let (edit, done) = self.buffer.undo_step()?;
            self.on_edit(&edit, None);
            edits.push(edit);
            if done {
                return Some(edits);
            }
        }
    }

    /// Re-applies the last undone group. See [`Document::undo`] for why
    /// this reconciles after each change instead of the whole group.
    pub fn redo(&mut self) -> Option<Vec<Edit>> {
        let mut edits = Vec::new();
        loop {
            let (edit, done) = self.buffer.redo_step()?;
            self.on_edit(&edit, None);
            edits.push(edit);
            if done {
                return Some(edits);
            }
        }
    }

    /// The next reparse to run, covering the first dirty range whole, or
    /// `None` when nothing is dirty or a job is already outstanding (one at a
    /// time; apply or cancel it first).
    pub fn parse_job(&mut self) -> Option<ParseJob> {
        self.parse_job_near(0, usize::MAX)
    }

    /// Like [`Document::parse_job`], but takes the dirty range nearest
    /// `focus` (the top of the viewport) and, if it is longer than
    /// `max_window`, only about `max_window` bytes of it starting at `focus`.
    /// Repeated calls stream a large paste back viewport first.
    pub fn parse_job_near(&mut self, focus: usize, max_window: usize) -> Option<ParseJob> {
        if self.outstanding.is_some() || self.dirty.is_empty() {
            return None;
        }
        if self.blocks.is_empty() {
            self.dirty.clear();
            return None;
        }
        let d = self.nearest_dirty(focus);
        let part = if d.len() <= max_window {
            d.clone()
        } else {
            let start = focus.clamp(d.start, d.end - max_window);
            start..start + max_window
        };
        let n = self.blocks.len();
        // Look-behind: an edit can change how preceding blocks end (setext
        // underline, lazy continuation, table rows) and, through lines that
        // are only continuations if the next line allows it, reach further
        // back. Start after a blank line (or at the top): no construct
        // continues across one into a separate block.
        let mut first = self.block_at(part.start).unwrap_or(0).saturating_sub(1);
        while first > 0 && !self.follows_blank_line(self.starts[first]) {
            first -= 1;
        }
        let last = self.block_at(part.end.saturating_sub(1).max(part.start)).unwrap_or(0);
        // Dirty text right after the window will be parsed by a later job,
        // whose look-behind reparses this window's last block: no need to
        // converge. Otherwise one block of look-ahead anchors convergence.
        let converge = self.starts[last + 1] >= d.end;
        let last = if converge { (last + 1).min(n - 1) } else { last };
        let window = self.starts[first]..self.starts[last + 1];
        self.clear_dirty(&window);
        let will_clear_dirty = self.dirty.is_empty();

        let id = self.next_job;
        self.next_job += 1;
        self.outstanding =
            Some(Outstanding { id, version: self.buffer.version(), window: window.clone() });
        Some(ParseJob {
            id,
            rope: self.buffer.rope().clone(),
            blocks: Arc::clone(&self.blocks),
            defs: Arc::clone(&self.defs),
            first,
            last,
            window,
            converge,
            will_clear_dirty,
            defs_dirty_before: self.defs_dirty,
        })
    }

    /// The dirty range containing `focus`, else the closer of its neighbors.
    /// `self.dirty` must not be empty.
    fn nearest_dirty(&self, focus: usize) -> Range<usize> {
        let after = self.dirty.partition_point(|r| r.end < focus);
        let before = after.checked_sub(1).map(|i| &self.dirty[i]);
        let range = match (self.dirty.get(after), before) {
            (Some(next), Some(prev))
                if next.start > focus && focus - prev.end < next.start - focus =>
            {
                prev
            }
            (Some(next), _) => next,
            (None, Some(prev)) => prev,
            (None, None) => &self.dirty[0],
        };
        range.clone()
    }

    /// Whether the line before `offset` (a line start) is blank.
    fn follows_blank_line(&self, offset: usize) -> bool {
        let rope = self.buffer.rope();
        if offset == 0 {
            return true;
        }
        let line = rope.byte_to_line(offset - 1);
        rope.line(line).chars().all(char::is_whitespace)
    }

    /// Returns the outstanding job's window to the dirty set.
    pub fn cancel_job(&mut self) {
        if let Some(job) = self.outstanding.take() {
            let (window, _) = self.rebase(job.version, job.window);
            self.mark_dirty(window);
        }
    }

    pub fn apply(&mut self, result: ParseResult) -> Applied {
        let Some(job) = self.outstanding.take_if(|job| job.id == result.id) else {
            return Applied::Ignored;
        };
        // No edit at all happened since the job's snapshot: any document-
        // wide definitions refresh it computed from that snapshot still
        // matches the current document, so it can be installed instead of
        // redone here. Any edit, even one that never touched this window,
        // means some other block may have changed too, so the snapshot no
        // longer covers the whole document and the refresh must be redone.
        let unchanged_since_snapshot = job.version == self.buffer.version();
        // The result may cover more than the initial window (convergence).
        let (window, touched) = self.rebase(job.version, result.window);
        if touched {
            self.mark_dirty(window);
            return Applied::Discarded;
        }
        // Untouched: only edits elsewhere happened, so the window still
        // starts and ends on block boundaries.
        let first = self.starts.partition_point(|&s| s < window.start);
        let last_end = self.starts.partition_point(|&s| s < window.end);
        if self.starts.get(first) != Some(&window.start)
            || self.starts.get(last_end) != Some(&window.end)
            || last_end <= first
        {
            self.mark_dirty(window);
            return Applied::Discarded;
        }
        let old = first..last_end;

        // A block that starts where an old block started keeps its identity,
        // so the block being typed in stays the same block across reparses.
        // Both lists are sorted by offset: one merge pass (a large paste
        // replaces hundreds of placeholders with a hundred thousand blocks).
        let mut offset = window.start;
        let mut candidate = old.start;
        let mut new_blocks = Vec::with_capacity(result.blocks.len());
        for parsed in result.blocks {
            while candidate < old.end && self.starts[candidate] < offset {
                candidate += 1;
            }
            let reuse = (candidate < old.end && self.starts[candidate] == offset)
                .then(|| self.blocks[candidate].id);
            let len = parsed.len;
            offset += len;
            let id = reuse.unwrap_or_else(|| self.fresh_id());
            new_blocks.push(Block { id, len, parsed, stale: false });
        }
        // The table only changes if the window's definitions did.
        let defs_changed = !same_definitions(
            self.blocks[old.clone()].iter().map(|b| &*b.parsed),
            new_blocks.iter().map(|b| &*b.parsed),
        );
        let spliced = old.start..old.start + new_blocks.len();
        self.splice(old, new_blocks);
        self.clear_dirty(&window);
        self.defs_dirty |= defs_changed;
        if self.defs_dirty {
            // Rebuilding the table and checking every block is O(blocks):
            // wait until nothing else is dirty, so a paste streamed back in
            // chunks (each defining references) pays for it once. The job
            // that settles the last chunk already computed this off the UI
            // thread (see `ParseJob::settle_definitions`); use it here when
            // nothing changed since, else fall back to redoing it in place.
            if self.dirty.is_empty() {
                match result.defs_refresh {
                    Some(refresh) if unchanged_since_snapshot => {
                        self.defs_dirty = false;
                        self.defs = refresh.table;
                        for range in refresh.affected {
                            self.mark_dirty(range);
                        }
                    }
                    _ => self.refresh_defs(),
                }
            }
        } else {
            // The table stands, but the job rendered against its snapshot,
            // which an earlier result may have changed since.
            let key = self.defs.footnote_key();
            for i in spliced {
                if renders_stale(&self.blocks[i].parsed, &self.defs, key) {
                    self.mark_dirty(self.block_range(i));
                }
            }
        }
        Applied::Spliced
    }

    /// Maps a snapshot-coordinate range to current coordinates and reports
    /// whether any later edit overlapped or touched it. If the edit log no
    /// longer reaches back that far, everything counts as touched.
    fn rebase(&self, version: u64, mut range: Range<usize>) -> (Range<usize>, bool) {
        let Some(edits) = self.buffer.edits_since(version) else {
            return (0..self.buffer.len(), true);
        };
        let mut touched = false;
        for edit in edits {
            touched |= edit.range.start <= range.end && edit.range.end >= range.start;
            range = shift_range(&range, edit);
        }
        (range, touched)
    }

    /// Runs jobs inline until the document is clean. For tests, small
    /// documents and code that is already off the UI thread.
    pub fn reparse_now(&mut self) {
        self.cancel_job();
        let mut jobs = 0usize;
        while let Some(job) = self.parse_job() {
            // Every job settles its window; definition changes can re-dirty
            // dependent blocks a bounded number of times. Far more jobs than
            // blocks means the dirty set stopped shrinking.
            jobs += 1;
            debug_assert!(
                jobs <= 16 * self.blocks.len() + 1024,
                "reparse is not converging; dirty = {:?}",
                self.dirty
            );
            let result = job.run();
            self.apply(result);
        }
    }

    fn fresh_id(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        id
    }

    fn new_block(&mut self, parsed: ParsedBlock, stale: bool) -> Block {
        let id = self.fresh_id();
        Block { id, len: parsed.len, parsed: Arc::new(parsed), stale }
    }

    fn recompute_starts(&mut self) {
        self.starts.clear();
        self.starts.push(0);
        let mut at = 0;
        for block in self.blocks.iter() {
            at += block.len;
            self.starts.push(at);
        }
    }

    fn splice(&mut self, old: Range<usize>, new: Vec<Block>) {
        let new_len = new.len();
        Arc::make_mut(&mut self.blocks).splice(old.clone(), new);
        self.recompute_starts();
        self.splices.push(Splice { old, new_len });
    }

    /// `cuts`: pre-segmenter boundaries of the inserted text alone, if known. Plain documents
    /// never reach the Markdown path below: they have no dirty ranges, definitions or parse
    /// jobs, only a local re-chunk (see [`Document::on_edit_plain`]).
    fn on_edit(&mut self, edit: &Edit, cuts: Option<&[usize]>) {
        if self.mode == DocMode::Plain {
            return self.on_edit_plain(edit);
        }
        for range in &mut self.dirty {
            *range = shift_range(range, edit);
        }

        let new_total = self.buffer.len();
        if self.blocks.is_empty() {
            if new_total > 0 {
                let block = self.stale_block(0..new_total, None, cuts.map(|c| (0, c)));
                self.splice(0..0, block);
                self.mark_dirty(0..new_total);
            }
            return;
        }

        let first = self.block_at(edit.range.start).unwrap_or(0);
        let last = if edit.range.end > edit.range.start {
            self.block_at(edit.range.end - 1).unwrap_or(first)
        } else {
            first
        };
        // Merged or removed blocks can take definitions with them; the table
        // must then be rebuilt at the next apply even if the reparsed window
        // happens to define the same things as the stale block.
        self.defs_dirty |= self.blocks[first..=last]
            .iter()
            .any(|b| !b.parsed.defs.is_empty() || !b.parsed.footnotes.is_empty());
        let start = self.starts[first];
        let len = self.starts[last + 1] - start - edit.range.len() + edit.new_len;
        let replacement = if len == 0 {
            Vec::new()
        } else {
            let cuts = cuts.map(|cuts| (edit.range.start - start, cuts));
            self.stale_block(start..start + len, Some(self.blocks[first].clone()), cuts)
        };
        self.splice(first..last + 1, replacement);
        self.mark_dirty(start..start + len);
    }

    /// A plain document's whole reaction to an edit: replaces the block(s) it touched with a
    /// fresh chunking of exactly their new combined span (`region_start..region_end`, the old
    /// touched span's start and end, the latter shifted through the edit) - never the rest of
    /// the document, since every other block's own length is untouched by an edit elsewhere (an
    /// edit only moves its *absolute offset*, which [`Document::splice`]'s
    /// [`Document::recompute_starts`] handles without touching content). That span is split, if
    /// it grew past the maximum, with [`split_evenly`] rather than packed greedily
    /// ([`plain_chunk_lens`]): the two ends of a touched span are constrained by whatever
    /// surrounds them, and a greedy pack's last, partial chunk cannot promise to clear the
    /// minimum the way an even split can (see [`split_evenly`]'s docs) - packing greedily here,
    /// re-anchored at the touched span instead of the document start, would only move the
    /// original bug's rippling from one anchor to another. If the span fits in one chunk on its
    /// own but that chunk falls under the minimum, [`plain_block_is_small`] merges it with the
    /// next block instead (bounded by [`PLAIN_MERGE_ATTEMPTS`], though one merge is normally
    /// enough) unless it is the document's own last block or immediately follows a forced cut.
    /// No dirty tracking, no definitions, no parse job: chunking is counting bytes and lines, not
    /// parsing, so it always runs inline.
    fn on_edit_plain(&mut self, edit: &Edit) {
        let new_total = self.buffer.len();
        if self.blocks.is_empty() {
            if new_total > 0 {
                let lens = plain_chunk_lens(self.buffer.rope(), 0..new_total);
                let blocks =
                    lens.into_iter().map(|len| self.new_block(md::plain(len), false)).collect();
                self.splice(0..0, blocks);
            }
            return;
        }
        let n = self.blocks.len();
        let first = self.block_at(edit.range.start).unwrap_or(0);
        let mut last = if edit.range.end > edit.range.start {
            self.block_at(edit.range.end - 1).unwrap_or(first)
        } else {
            first
        };
        let region_start = self.starts[first];
        let mut region_end = shift_offset(self.starts[last + 1], edit);
        let rope = self.buffer.rope().clone();

        for _ in 0..PLAIN_MERGE_ATTEMPTS {
            let span = region_end - region_start;
            let fits_as_one = span <= PLAIN_CHUNK_BYTES
                && rope.byte_to_line(region_end) - rope.byte_to_line(region_start)
                    <= PLAIN_CHUNK_LINES;
            if !fits_as_one || !plain_block_is_small(&rope, region_start, span) {
                break;
            }
            let is_last_block = last + 1 == n;
            let follows_forced_cut = first > 0 && rope.byte(region_start - 1) != b'\n';
            if is_last_block || follows_forced_cut {
                break;
            }
            last += 1;
            region_end = shift_offset(self.starts[last + 1], edit);
        }

        let new_lens = split_evenly(&rope, region_start..region_end);
        let mut new_blocks: Vec<Block> =
            new_lens.into_iter().map(|len| self.new_block(md::plain(len), false)).collect();
        if let Some(first_new) = new_blocks.first_mut() {
            first_new.id = self.blocks[first].id;
        }
        self.splice(first..last + 1, new_blocks);
    }

    /// Blocks for a changed range `range` (current coordinates): one stale
    /// block keeping `keep`'s identity, or provisional unparsed blocks when
    /// the range is large. `inserted`: where in `range` inserted text starts,
    /// and its pre-segmenter boundaries.
    fn stale_block(
        &mut self,
        range: Range<usize>,
        keep: Option<Block>,
        inserted: Option<(usize, &[usize])>,
    ) -> Vec<Block> {
        let len = range.len();
        if len > UNPARSED_SPLIT_THRESHOLD {
            let boundaries = self.boundaries(range, inserted);
            // Chunks of at least UNPARSED_CHUNK bytes (UNPARSED_EDGE_CHUNK
            // near the ends), cut only where the pre-segmenter allows (never
            // inside fenced code): small enough that showing one raw is
            // cheap, few enough to create quickly.
            let mut starts = vec![0];
            for cut in boundaries {
                let near_edge = cut <= UNPARSED_EDGE || len - cut <= UNPARSED_EDGE;
                let min = if near_edge { UNPARSED_EDGE_CHUNK } else { UNPARSED_CHUNK };
                if cut - starts[starts.len() - 1] >= min {
                    starts.push(cut);
                }
            }
            starts.push(len);
            let mut blocks: Vec<Block> = starts
                .windows(2)
                .map(|w| self.new_block(md::unparsed(w[1] - w[0]), true))
                .collect();
            if let (Some(first), Some(keep)) = (blocks.first_mut(), keep) {
                first.id = keep.id;
            }
            return blocks;
        }
        match keep {
            Some(block) => vec![Block { len, stale: true, ..block }],
            None => vec![self.new_block(md::unparsed(len), true)],
        }
    }

    /// Pre-segmenter boundaries of `range`, relative to its start. Boundaries
    /// computed for inserted text alone are reused when they mean the same
    /// in place: the insert starts a line and nothing before it in `range`
    /// leaves a fence open. Boundaries in the text after the insert are left
    /// out (fewer, larger provisional blocks; still correct).
    fn boundaries(&self, range: Range<usize>, inserted: Option<(usize, &[usize])>) -> Vec<usize> {
        let rope = self.buffer.rope();
        if let Some((at, cuts)) = inserted {
            let prefix = rope.byte_slice(range.start..range.start + at).to_string();
            if (prefix.is_empty() || prefix.ends_with('\n')) && !md::ends_in_fence(&prefix) {
                return cuts.iter().map(|cut| at + cut).collect();
            }
        }
        md::presegment(&rope.byte_slice(range).to_string())
    }

    fn mark_dirty(&mut self, range: Range<usize>) {
        let idx = self.dirty.partition_point(|r| r.end < range.start);
        let mut merged = range;
        while idx < self.dirty.len() && self.dirty[idx].start <= merged.end {
            let r = self.dirty.remove(idx);
            merged = merged.start.min(r.start)..merged.end.max(r.end);
        }
        self.dirty.insert(idx, merged);
    }

    /// Removes what a parse of `window` settled from the dirty set. A dirty
    /// range needs its own block reparsed plus the block before it, so a
    /// range starting exactly at `window.start` leaves a point there (its
    /// look-behind block is outside the window), and a point at `window.end`
    /// is kept (its block is outside the window).
    fn clear_dirty(&mut self, window: &Range<usize>) {
        let len = self.buffer.len();
        let mut kept: Vec<Range<usize>> = Vec::with_capacity(self.dirty.len() + 1);
        for r in self.dirty.drain(..) {
            if r.end < window.start
                || r.start > window.end
                || (r.is_empty() && r.start == window.end && window.end < len)
            {
                kept.push(r);
                continue;
            }
            if r.start < window.start {
                kept.push(r.start..window.start);
            } else if r.start == window.start && window.start > 0 {
                kept.push(window.start..window.start);
            }
            if r.end > window.end {
                kept.push(window.end..r.end);
            }
        }
        self.dirty.clear();
        for r in kept {
            self.mark_dirty(r);
        }
    }

    /// Rebuilds the definition table and marks every block that rendered
    /// against a different one dirty (blocks parsed while the refresh was
    /// deferred may have used any snapshot).
    fn refresh_defs(&mut self) {
        self.defs_dirty = false;
        let table = DefTable::from_blocks(self.blocks.iter().map(|b| &*b.parsed));
        if table != *self.defs {
            self.defs = Arc::new(table);
        }
        let key = self.defs.footnote_key();
        let affected: Vec<Range<usize>> = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| renders_stale(&b.parsed, &self.defs, key))
            .map(|(i, _)| self.block_range(i))
            .collect();
        for range in affected {
            self.mark_dirty(range);
        }
    }
}

/// Whether `block` rendered against other definitions than `defs` (whose
/// [`DefTable::footnote_key`] is `footnote_key`): a link label it looked up
/// resolves differently, or it mentions footnotes and they changed.
fn renders_stale(block: &ParsedBlock, defs: &DefTable, footnote_key: u64) -> bool {
    block.footnotes_seen.is_some_and(|seen| seen != footnote_key)
        || block.refs.iter().any(|r| defs.get(&r.label) != r.target.as_ref())
}

fn same_definitions<'a>(
    old: impl Iterator<Item = &'a ParsedBlock> + Clone,
    new: impl Iterator<Item = &'a ParsedBlock> + Clone,
) -> bool {
    old.clone().flat_map(|b| &b.defs).eq(new.clone().flat_map(|b| &b.defs))
        && old.flat_map(|b| &b.footnotes).eq(new.flat_map(|b| &b.footnotes))
}

/// Maps a range through an edit. Ranges overlapping the edit grow to cover
/// the replacement.
fn shift_range(range: &Range<usize>, edit: &Edit) -> Range<usize> {
    let (old, new_len) = (&edit.range, edit.new_len);
    let map = |offset: usize, inside: usize| {
        if offset < old.start {
            offset
        } else if offset >= old.end && offset > old.start {
            offset - old.len() + new_len
        } else {
            inside
        }
    };
    let start = map(range.start, old.start);
    let end = map(range.end, old.start + new_len);
    start..end.max(start)
}

/// Text prepared for [`Document::edit_prepared`]: normalized, as a rope, and
/// pre-segmented. `Send`; build it off the UI thread.
pub struct PreparedInsert {
    text: PreparedText,
    cuts: Vec<usize>,
}

impl PreparedInsert {
    pub fn new(text: &str) -> Self {
        let text = PreparedText::new(text);
        let cuts = md::presegment(text.as_str());
        Self { text, cuts }
    }

    /// The text as it will be inserted.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }
}

/// An immutable reparse of a window of whole blocks. `Send`; run it anywhere.
pub struct ParseJob {
    id: u64,
    rope: Rope,
    blocks: Arc<Vec<Block>>,
    defs: Arc<DefTable>,
    first: usize,
    last: usize,
    window: Range<usize>,
    /// Extend the window until it re-synchronizes with the old blocks. Off
    /// when dirty text follows the window (a streamed chunk).
    converge: bool,
    /// Whether settling this window, if nothing else changes first, would
    /// leave the document clean: only then is it worth computing the
    /// document-wide definitions refresh in `run`, so a paste streamed back
    /// in many chunks still pays for it once, not once per chunk.
    will_clear_dirty: bool,
    /// Whether the document already owed a definitions rebuild before this
    /// job started (an earlier edit removed a block that made one).
    defs_dirty_before: bool,
}

impl ParseJob {
    /// Bytes the job will parse at least (convergence may extend it).
    pub fn window_len(&self) -> usize {
        self.window.len()
    }

    /// Whether the job is cheap enough for the UI thread.
    pub fn is_small(&self) -> bool {
        self.window.len() <= SYNC_PARSE_LIMIT
    }

    /// Parses the window, extending it until the parse re-synchronizes with
    /// the old block structure: the last reparsed block must equal the old
    /// block at the same place (same extent, source and kind), or the window
    /// reaches the end of the text. A streamed chunk is parsed as is.
    pub fn run(self) -> ParseResult {
        let n = self.blocks.len();
        let text_len = self.rope.len_bytes();
        let mut last = self.last;
        let mut end = self.window.end;
        loop {
            let text = self.rope.byte_slice(self.window.start..end).to_string();
            let parsed = md::parse(&text, &self.defs);
            let converged = !self.converge
                || end == text_len
                || parsed.last().is_some_and(|p| {
                    end - p.len >= self.window.start && self.blocks[last].matches(p)
                });
            if converged || last + 1 >= n {
                let (parsed, defs_refresh) = self.settle_definitions(&text, parsed, last);
                let blocks = parsed.into_iter().map(Arc::new).collect();
                return ParseResult {
                    id: self.id,
                    window: self.window.start..end,
                    blocks,
                    defs_refresh,
                };
            }
            // Grow geometrically: an unclosed fence near the top costs
            // O(n log n) parsed bytes, not O(n²).
            let grow = (last + 1 - self.first).max(1);
            let new_last = (last + grow).min(n - 1);
            end += self.blocks[last + 1..=new_last].iter().map(|b| b.len).sum::<usize>();
            last = new_last;
        }
    }

    /// If the window's definitions changed, the table changes once the
    /// result is applied, and every block that looked up a changed label
    /// would be parsed again, each as its own job on the UI thread (a paste
    /// that defines the references it uses: thousands). Instead parse the
    /// window once more here, against the table as it will be.
    ///
    /// When settling this window would also leave the document clean
    /// (`will_clear_dirty`) and the table needs rebuilding (the window's
    /// definitions changed, or an earlier edit already flagged one), this
    /// also builds the document-wide refresh that `Document::apply` would
    /// otherwise have to do on the UI thread: the whole new [`DefTable`]
    /// plus every block whose rendering depends on it, spanning this
    /// snapshot's blocks outside the window and the window's new ones.
    /// `apply` uses the result only if nothing else changed in the meantime.
    fn settle_definitions(
        &self,
        text: &str,
        parsed: Vec<ParsedBlock>,
        last: usize,
    ) -> (Vec<ParsedBlock>, Option<DefsRefresh>) {
        let old = self.blocks[self.first..=last].iter().map(|b| &*b.parsed);
        let defs_changed = !same_definitions(old, parsed.iter());
        let need_refresh = self.will_clear_dirty && (defs_changed || self.defs_dirty_before);
        if !defs_changed && !need_refresh {
            return (parsed, None);
        }
        let table = DefTable::from_blocks(
            self.blocks[..self.first]
                .iter()
                .map(|b| &*b.parsed)
                .chain(&parsed)
                .chain(self.blocks[last + 1..].iter().map(|b| &*b.parsed)),
        );
        let key = table.footnote_key();
        let parsed = if defs_changed && parsed.iter().any(|p| renders_stale(p, &table, key)) {
            md::parse(text, &table)
        } else {
            parsed
        };
        let defs_refresh = need_refresh.then(|| {
            let mut affected = Vec::new();
            let mut offset = 0;
            for b in &self.blocks[..self.first] {
                if renders_stale(&b.parsed, &table, key) {
                    affected.push(offset..offset + b.len);
                }
                offset += b.len;
            }
            for p in &parsed {
                if renders_stale(p, &table, key) {
                    affected.push(offset..offset + p.len);
                }
                offset += p.len;
            }
            for b in &self.blocks[last + 1..] {
                if renders_stale(&b.parsed, &table, key) {
                    affected.push(offset..offset + b.len);
                }
                offset += b.len;
            }
            DefsRefresh { table: Arc::new(table), affected }
        });
        (parsed, defs_refresh)
    }
}

/// A document-wide definitions rebuild for a window that, once applied,
/// would leave the document clean: the new table, computed by
/// [`ParseJob::run`] off the UI thread, and every block (as a byte range)
/// whose rendering depends on it. [`Document::apply`] installs it directly
/// when nothing else changed since the job's snapshot, instead of rebuilding
/// the table and re-checking every block itself.
struct DefsRefresh {
    table: Arc<DefTable>,
    affected: Vec<Range<usize>>,
}

pub struct ParseResult {
    id: u64,
    window: Range<usize>,
    /// Already behind `Arc`s: allocating them is done by whoever ran the job
    /// (usually a background thread), not by `apply` on the UI thread.
    blocks: Vec<Arc<ParsedBlock>>,
    /// The document-wide definitions refresh, if settling this window would
    /// leave the document clean and one was needed.
    defs_refresh: Option<DefsRefresh>,
}

impl ParseResult {
    /// The window the parse ended up covering (the result applies to the
    /// document only if the job's window is still intact).
    pub fn window(&self) -> Range<usize> {
        self.window.clone()
    }
}

/// Byte ranges of `query` in `rope`, in order, not overlapping, at most [`MAX_FIND_MATCHES`].
/// Smart case: case-insensitive (ASCII letters) unless `query` contains an uppercase letter. An
/// empty query matches nothing.
///
/// Scans the rope's own chunks instead of copying the whole text into a `String` first (the
/// previous implementation, `O(document size)` in extra memory): the only allocation is a
/// rolling window of at most one query length of bytes, carried across chunk boundaries so a
/// match spanning two chunks is still found.
///
/// Free (not `Document::find_all`, which just calls this on its own rope) so the editor can run
/// it on a standalone rope snapshot - `Buffer::rope().clone()`, O(1) - on a background executor
/// for a document above [`FIND_BACKGROUND_THRESHOLD`]: a `Document` borrow cannot cross an
/// `.await` onto another thread the way an owned `Rope` can.
pub fn find_all_in_rope(rope: &Rope, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let insensitive = !query.chars().any(char::is_uppercase);
    let mut needle = query.as_bytes().to_vec();
    if insensitive {
        needle.make_ascii_lowercase();
    }
    let mut matches = Vec::new();
    let mut window: Vec<u8> = Vec::new();
    let mut window_start = 0usize;
    'chunks: for piece in rope.chunks() {
        let appended_at = window.len();
        window.extend_from_slice(piece.as_bytes());
        if insensitive {
            // ASCII-only lowering keeps every byte offset where it was, and is safe on raw
            // UTF-8 bytes: 'A'-'Z' never appear inside a multi-byte sequence.
            window[appended_at..].make_ascii_lowercase();
        }
        let mut at = 0;
        while let Some(found) = find_bytes(&window[at..], &needle) {
            let start = window_start + at + found;
            matches.push(start..start + needle.len());
            if matches.len() >= MAX_FIND_MATCHES {
                break 'chunks;
            }
            at += found + needle.len();
        }
        let keep_from = window.len().saturating_sub(needle.len() - 1);
        window_start += keep_from;
        window.drain(..keep_from);
    }
    matches
}

/// First offset in `hay` where `needle` occurs, or `None`. Naive (`O(hay.len() * needle.len())`)
/// but only ever called on a bounded rolling window (`Document::find_all`), not the whole text.
fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Byte offset of the next `\n` at or after `from` and before `before`, without copying: walks
/// the rope's own chunks. `None` if there is none before `before`.
fn find_newline(rope: &Rope, from: usize, before: usize) -> Option<usize> {
    if from >= before {
        return None;
    }
    let mut at = from;
    for piece in rope.byte_slice(from..before).chunks() {
        if let Some(i) = piece.find('\n') {
            return Some(at + i);
        }
        at += piece.len();
    }
    None
}

/// The nearest char boundary at or before `byte_idx` (ropey panics on a slice or byte read that
/// splits a character). UTF-8 continuation bytes have their top two bits `10`.
fn floor_char_boundary(rope: &Rope, mut byte_idx: usize) -> usize {
    while byte_idx > 0 && byte_idx < rope.len_bytes() && (rope.byte(byte_idx) & 0xC0) == 0x80 {
        byte_idx -= 1;
    }
    byte_idx
}

/// Length of the next plain-text chunk starting at absolute offset `start`, never reading past
/// `cap` (`0` only when `start >= cap`): cut after a `\n`, targeting [`PLAIN_CHUNK_BYTES`] bytes
/// or [`PLAIN_CHUNK_LINES`] lines, whichever comes first, or `cap` itself if reached first. A
/// line with no `\n` within [`PLAIN_FORCED_CUT_BYTES`] of its own start is cut there instead - a
/// display-only break (the underlying bytes are unchanged, and the next chunk continues the same
/// source line) that keeps a single pathologically long line from ever making the virtualized
/// list lay out or shape more than a bounded amount of text. Scans at most
/// `min(PLAIN_CHUNK_BYTES, cap - start)` bytes, never the rest of the document, and never copies
/// `rope`'s text.
///
/// `cap` needs no precondition (unlike a plain byte offset picked at random, it is always itself
/// a valid chunk boundary in the caller's own terms: the end of the document, or the start/end of
/// a block the caller is not touching) - this only ever *shrinks* the chunk that would otherwise
/// be returned, cutting at `cap` instead of the usual stopping point, and `cap` is exactly as
/// valid a place to cut as whatever `cap` itself bounds.
fn plain_chunk_len_capped(rope: &Rope, start: usize, cap: usize) -> usize {
    if start >= cap {
        return 0;
    }
    let mut pos = start;
    let mut lines = 0usize;
    loop {
        let force_at = floor_char_boundary(rope, (pos + PLAIN_FORCED_CUT_BYTES).min(cap));
        match find_newline(rope, pos, force_at) {
            Some(nl) => {
                lines += 1;
                let len_so_far = nl + 1 - start;
                if len_so_far >= PLAIN_CHUNK_BYTES || lines >= PLAIN_CHUNK_LINES || nl + 1 >= cap {
                    return len_so_far;
                }
                pos = nl + 1;
            }
            None => return force_at - start,
        }
    }
}

/// Chunk lengths tiling `range` exactly, packed greedily to the maximum:
/// [`plain_chunk_len_capped`] repeatedly from `range.start`, capped at `range.end`. Used for a
/// fresh span with nothing past its own end to answer to - the whole document at load, or new
/// content mid-document before [`Document::on_edit_plain`] decides how much of what surrounds it
/// to fold in - never for re-chunking an existing span in place (see [`split_evenly`]).
fn plain_chunk_lens(rope: &Rope, range: Range<usize>) -> Vec<usize> {
    let mut lens = Vec::new();
    let mut at = range.start;
    while at < range.end {
        let len = plain_chunk_len_capped(rope, at, range.end);
        lens.push(len);
        at += len;
    }
    lens
}

/// The block `start..start+len` is too small to stand on its own (under [`PLAIN_MIN_CHUNK_BYTES`]
/// *and* under [`PLAIN_MIN_CHUNK_LINES`] - either alone is enough to clear the minimum, matching
/// how a chunk is cut on whichever of lines or bytes hits its cap first). Whether it is actually
/// allowed to be that small anyway (the document's last block, or right after a forced cut) is
/// for the caller to judge; this only measures.
fn plain_block_is_small(rope: &Rope, start: usize, len: usize) -> bool {
    if len >= PLAIN_MIN_CHUNK_BYTES {
        return false;
    }
    let lines = rope.byte_to_line(start + len) - rope.byte_to_line(start);
    lines < PLAIN_MIN_CHUNK_LINES
}

/// Splits `range` into the fewest chunks that each respect [`PLAIN_CHUNK_BYTES`] and
/// [`PLAIN_CHUNK_LINES`], cut at *even* line indices (`Rope::line_to_byte`) rather than packed
/// greedily to the maximum like [`plain_chunk_lens`]. Used to re-chunk a span already bounded on
/// both ends by whatever [`Document::on_edit_plain`] decided surrounds it - a block that grew
/// past the maximum, alone or after merging with a too-small neighbour - where the two ends may
/// themselves need to clear [`PLAIN_MIN_CHUNK_BYTES`]/[`PLAIN_MIN_CHUNK_LINES`]: a greedy pack's
/// last, partial chunk cannot promise that (it is sized only by where it happens to run out of
/// room, not by what is left over), while halving a range that is at most twice the maximum
/// always leaves both halves at least half the maximum - comfortably over a quarter. Halving is
/// recursive because one side of an uneven split (very unequal line lengths) may still be over
/// the maximum. Still cut only at line ends, or, once no further line boundary can subdivide a
/// piece (a single source line, or part of one, spans it entirely), at a forced cut - the same
/// display-only break as [`plain_chunk_len_capped`], delegated to it directly.
fn split_evenly(rope: &Rope, range: Range<usize>) -> Vec<usize> {
    let mut lens = Vec::new();
    split_evenly_into(rope, range, &mut lens);
    lens
}

fn split_evenly_into(rope: &Rope, range: Range<usize>, lens: &mut Vec<usize>) {
    let total = range.len();
    if total == 0 {
        return;
    }
    let start_line = rope.byte_to_line(range.start);
    let end_line = rope.byte_to_line(range.end);
    if total <= PLAIN_CHUNK_BYTES && end_line - start_line <= PLAIN_CHUNK_LINES {
        lens.push(total);
        return;
    }
    let mid_line = start_line + (end_line - start_line) / 2;
    let mid = rope.line_to_byte(mid_line.min(end_line));
    if mid <= range.start || mid >= range.end {
        let mut at = range.start;
        while at < range.end {
            let len = plain_chunk_len_capped(rope, at, range.end);
            lens.push(len);
            at += len;
        }
        return;
    }
    split_evenly_into(rope, range.start..mid, lens);
    split_evenly_into(rope, mid..range.end, lens);
}

/// Maps `offset` (in the text *before* `edit`) to its position after, like [`shift_range`] for a
/// single point. Only meaningful for `offset >= edit.range.end` (a position [`Document`] never
/// calls this with any other for): positions inside the edited range have no single new position.
fn shift_offset(offset: usize, edit: &Edit) -> usize {
    offset - edit.range.len() + edit.new_len
}

#[cfg(test)]
mod tests;
