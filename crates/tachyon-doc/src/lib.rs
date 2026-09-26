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
use std::sync::Arc;

use ropey::Rope;
use tachyon_md::{self as md, DefTable, ParsedBlock};
use tachyon_text::{Buffer, Edit, EditError};

/// Jobs whose window is at most this many bytes are cheap enough to run on
/// the UI thread (tens of microseconds).
pub const SYNC_PARSE_LIMIT: usize = 32 * 1024;

/// Stale blocks larger than this are split into provisional unparsed blocks
/// so they can be shown before the background parse finishes.
pub const UNPARSED_SPLIT_THRESHOLD: usize = 64 * 1024;

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
    /// Sorted, disjoint (possibly empty) ranges awaiting a reparse.
    dirty: Vec<Range<usize>>,
    splices: Vec<Splice>,
    outstanding: Option<Outstanding>,
    /// An edit removed blocks that made definitions; rebuild the table at
    /// the next apply.
    defs_dirty: bool,
    next_block: u64,
    next_job: u64,
}

impl Document {
    /// Parses `text` completely (callers may run this off the UI thread; the
    /// document is `Send`).
    pub fn new(text: &str) -> Self {
        let buffer = Buffer::new(text);
        let (parsed, defs) = md::parse_document(&buffer.text());
        let mut doc = Document {
            buffer,
            blocks: Arc::new(Vec::new()),
            starts: vec![0],
            defs: Arc::new(defs),
            dirty: Vec::new(),
            splices: Vec::new(),
            outstanding: None,
            defs_dirty: false,
            next_block: 0,
            next_job: 0,
        };
        let blocks: Vec<Block> = parsed.into_iter().map(|p| doc.new_block(p, false)).collect();
        doc.blocks = Arc::new(blocks);
        doc.recompute_starts();
        doc
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

    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty() || self.outstanding.is_some()
    }

    /// Block-list changes since the last call, in order.
    pub fn take_splices(&mut self) -> Vec<Splice> {
        std::mem::take(&mut self.splices)
    }

    pub fn edit(&mut self, range: Range<usize>, text: &str) -> Result<Edit, EditError> {
        let edit = self.buffer.edit(range, text)?;
        self.on_edit(&edit);
        Ok(edit)
    }

    pub fn seal_undo_group(&mut self) {
        self.buffer.seal_undo_group();
    }

    pub fn undo(&mut self) -> Option<Vec<Edit>> {
        let edits = self.buffer.undo()?;
        edits.iter().for_each(|e| self.on_edit(e));
        Some(edits)
    }

    pub fn redo(&mut self) -> Option<Vec<Edit>> {
        let edits = self.buffer.redo()?;
        edits.iter().for_each(|e| self.on_edit(e));
        Some(edits)
    }

    /// The next reparse to run, or `None` when nothing is dirty or a job is
    /// already outstanding (one at a time; apply or cancel it first).
    pub fn parse_job(&mut self) -> Option<ParseJob> {
        if self.outstanding.is_some() || self.dirty.is_empty() {
            return None;
        }
        if self.blocks.is_empty() {
            self.dirty.clear();
            return None;
        }
        let d = self.dirty[0].clone();
        let n = self.blocks.len();
        // One block of look-behind: an edit can change how the preceding
        // block ends (setext underline, lazy continuation, table rows). One
        // block of look-ahead anchors convergence.
        let first = self.block_at(d.start).unwrap_or(0).saturating_sub(1);
        let last = self.block_at(if d.end > d.start { d.end - 1 } else { d.start }).unwrap_or(0);
        let last = (last + 1).min(n - 1);
        let window = self.starts[first]..self.starts[last + 1];
        self.clear_dirty(&window);

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
        })
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
        let mut offset = window.start;
        let mut new_blocks = Vec::with_capacity(result.blocks.len());
        for parsed in result.blocks {
            let reuse = old.clone().find(|&i| self.starts[i] == offset).map(|i| self.blocks[i].id);
            offset += parsed.len;
            let block = match reuse {
                Some(id) => Block { id, len: parsed.len, parsed: Arc::new(parsed), stale: false },
                None => self.new_block(parsed, false),
            };
            new_blocks.push(block);
        }
        // The table only changes if the window's definitions did.
        let defs_changed = !same_definitions(&self.blocks[old.clone()], &new_blocks);
        self.splice(old, new_blocks);
        self.clear_dirty(&window);
        if defs_changed || self.defs_dirty {
            self.refresh_defs();
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
        while let Some(job) = self.parse_job() {
            let result = job.run();
            self.apply(result);
        }
    }

    fn new_block(&mut self, parsed: ParsedBlock, stale: bool) -> Block {
        let id = BlockId(self.next_block);
        self.next_block += 1;
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

    fn on_edit(&mut self, edit: &Edit) {
        for range in &mut self.dirty {
            *range = shift_range(range, edit);
        }

        let new_total = self.buffer.len();
        if self.blocks.is_empty() {
            if new_total > 0 {
                let block = self.stale_block(0..new_total, None);
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
            self.stale_block(start..start + len, Some(self.blocks[first].clone()))
        };
        self.splice(first..last + 1, replacement);
        self.mark_dirty(start..start + len);
    }

    /// Blocks for a changed range `range` (current coordinates): one stale
    /// block keeping `keep`'s identity, or provisional unparsed blocks when
    /// the range is large.
    fn stale_block(&mut self, range: Range<usize>, keep: Option<Block>) -> Vec<Block> {
        let len = range.len();
        if len > UNPARSED_SPLIT_THRESHOLD {
            let text = self.buffer.rope().byte_slice(range).to_string();
            let mut cuts = md::presegment(&text);
            cuts.insert(0, 0);
            cuts.push(text.len());
            let mut blocks: Vec<Block> = cuts
                .windows(2)
                .map(|w| self.new_block(md::unparsed(&text[w[0]..w[1]]), true))
                .collect();
            if let (Some(first), Some(keep)) = (blocks.first_mut(), keep) {
                first.id = keep.id;
            }
            return blocks;
        }
        match keep {
            Some(block) => vec![Block { len, stale: true, ..block }],
            None => {
                let text = self.buffer.rope().byte_slice(range).to_string();
                vec![self.new_block(md::unparsed(&text), true)]
            }
        }
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

    /// Rebuilds the definition table; blocks whose rendering depends on a
    /// changed definition are marked dirty.
    fn refresh_defs(&mut self) {
        self.defs_dirty = false;
        let table = DefTable::from_blocks(self.blocks.iter().map(|b| &*b.parsed));
        if table == *self.defs {
            return;
        }
        let changes = self.defs.changes(&table);
        self.defs = Arc::new(table);
        let affected: Vec<Range<usize>> = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                (changes.footnotes && b.parsed.mentions_footnotes)
                    || b.parsed
                        .refs
                        .iter()
                        .any(|r| changes.links.iter().any(|c| md::labels_match(r, c)))
            })
            .map(|(i, _)| self.block_range(i))
            .collect();
        for range in affected {
            self.mark_dirty(range);
        }
    }
}

fn same_definitions(old: &[Block], new: &[Block]) -> bool {
    let links =
        |blocks: &[Block]| blocks.iter().flat_map(|b| &b.parsed.defs).cloned().collect::<Vec<_>>();
    let notes = |blocks: &[Block]| {
        blocks.iter().flat_map(|b| &b.parsed.footnotes).cloned().collect::<Vec<_>>()
    };
    links(old) == links(new) && notes(old) == notes(new)
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

/// An immutable reparse of a window of whole blocks. `Send`; run it anywhere.
pub struct ParseJob {
    id: u64,
    rope: Rope,
    blocks: Arc<Vec<Block>>,
    defs: Arc<DefTable>,
    first: usize,
    last: usize,
    window: Range<usize>,
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
    /// reaches the end of the text.
    pub fn run(self) -> ParseResult {
        let n = self.blocks.len();
        let text_len = self.rope.len_bytes();
        let mut last = self.last;
        let mut end = self.window.end;
        loop {
            let text = self.rope.byte_slice(self.window.start..end).to_string();
            let parsed = md::parse(&text, &self.defs);
            let converged = end == text_len
                || parsed.last().is_some_and(|p| {
                    end - p.len >= self.window.start && self.blocks[last].matches(p)
                });
            if converged || last + 1 >= n {
                return ParseResult { id: self.id, window: self.window.start..end, blocks: parsed };
            }
            // Grow geometrically: an unclosed fence near the top costs
            // O(n log n) parsed bytes, not O(n²).
            let grow = (last + 1 - self.first).max(1);
            let new_last = (last + grow).min(n - 1);
            end += self.blocks[last + 1..=new_last].iter().map(|b| b.len).sum::<usize>();
            last = new_last;
        }
    }
}

pub struct ParseResult {
    id: u64,
    window: Range<usize>,
    blocks: Vec<ParsedBlock>,
}

impl ParseResult {
    /// The window the parse ended up covering (the result applies to the
    /// document only if the job's window is still intact).
    pub fn window(&self) -> Range<usize> {
        self.window.clone()
    }
}

#[cfg(test)]
mod tests;
