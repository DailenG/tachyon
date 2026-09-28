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

#[cfg(test)]
use std::cell::Cell;

// Test-only instrumentation for `Document::boundaries`: how many bytes of rope content it has
// handed to a presegment scan (full, bounded-window or fence-peek) since the last
// `take_presegment_bytes` call. Lets a test assert that an edit into a huge single block does
// bounded work without timing anything (see `tests::edits_into_a_huge_single_block_...`).
#[cfg(test)]
thread_local! {
    static PRESEGMENT_BYTES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn record_presegment_bytes(n: usize) {
    PRESEGMENT_BYTES.with(|c| c.set(c.get() + n));
}

/// Resets and returns the running total [`record_presegment_bytes`] has recorded.
#[cfg(test)]
pub(crate) fn take_presegment_bytes() -> usize {
    PRESEGMENT_BYTES.with(|c| c.replace(0))
}

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

/// Above this size with no reusable inserted-text boundaries (an ordinary keystroke; only
/// [`PreparedInsert`]'s paste path carries those), [`Document::boundaries`] bounds its scan to a
/// window around the edit instead of the whole range: see its own doc comment.
const BOUNDARY_FULL_SCAN_LIMIT: usize = 256 * 1024;

/// How far to each side of the edit (and its own inserted text) [`Document::boundaries`]
/// presegments when it falls back to a bounded scan: enough to catch a blank line the edit
/// creates near either edge, while keeping the scan a small, fixed cost regardless of how large
/// the rest of the block is.
const BOUNDARY_SCAN_MARGIN: usize = 64 * 1024;

/// How far [`block_shape`] looks for the end of a block's first line before giving up on
/// telling whether it opens a fence: real Markdown lines are nowhere near this long, so this
/// only ever gives up on one pathologically long line, which a bounded scan could not help
/// either way.
const BOUNDARY_FENCE_PEEK_LIMIT: usize = 8 * 1024;

/// Window size for streaming a large dirty range back in pieces with
/// [`Document::parse_job_near`]: a few milliseconds of parsing, so the text
/// near the viewport is formatted within a frame or two of a large paste.
pub const PARSE_CHUNK: usize = 128 * 1024;

/// Window size [`Document::load_markdown`] streams a fresh Markdown load back in. Larger than
/// [`PARSE_CHUNK`]: nothing needs to reach the UI a chunk at a time here (the document is not
/// shown until the whole load finishes), so a bigger window trades a little of the memory bound
/// for a lot less fixed per-window overhead (definitions bookkeeping, splicing, rebasing) -
/// `cargo bench -p tachyon-doc`'s "full parse" numbers are the budget this is tuned against.
/// Still small next to a pathological file: a 200 MB single-block file peaks around this many
/// bytes of transient buffer, not 200 MB.
const LOAD_CHUNK: usize = 4 * 1024 * 1024;

/// Extra bytes past [`LOAD_CHUNK`] [`Document::load_markdown`] scans for a pre-segmenter
/// boundary (`md::presegment`: a cut after a blank line, outside fenced code - the same
/// fence-aware scan a large paste already uses) before giving up on that placeholder and
/// treating the rest of the text as one pathological block instead. Ordinary Markdown has a
/// blank line every few lines at most, so this is found almost immediately; only a file that
/// tiles into one enormous block (a huge fenced block, or a long run with no blank line at all)
/// ever reads this far without finding one.
const LOAD_SCAN_MARGIN: usize = 64 * 1024;

/// [`Document::evict`] leaves a block's `ir` alone above this size: re-deriving it again once
/// the block scrolls back into view (`Document::ensure_ir`, on the UI thread, synchronously)
/// must never risk a frame budget on its own. `cargo bench -p tachyon-doc`'s full-parse rate
/// (~20 ms/MiB) bounds this comfortably under a frame at this size. The common case - one giant
/// block *is* the whole document - already needs no eviction (it is always the block holding
/// the caret, so always kept); this only excludes the rarer shape of a few large blocks
/// scattered through an otherwise well-structured file, where dropping their `ir` would buy
/// little next to what the file's many small blocks' `ir` already costs.
pub const EVICT_MAX_LEN: usize = 256 * 1024;

/// [`Document::evict`] drops at most this many blocks' `ir` per call: dropping an evicted
/// block's *old* `ir` (a real `String` and several `Vec`s, each its own free) is the expensive
/// part, not building the cheap replacement, so evicting everything outside `keep` in one call -
/// tens or hundreds of thousands of blocks, the first time a well-structured multi-megabyte
/// document's whole off-screen majority leaves `keep` at once, right after a fresh load - cost a
/// single frame tens of milliseconds even with a cheap replacement. `Editor::render` calls
/// `evict` every frame regardless, so capping one call's work just spreads the same total drop
/// cost over the next several frames instead of paying it all in the one right after load.
const EVICT_BATCH_LIMIT: usize = 4096;

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

/// [`raw_segment_lens`]'s target byte size, playing [`PLAIN_CHUNK_BYTES`]'s role for
/// `Editor::render_raw`'s split of an oversized *active* block instead of [`DocMode::Plain`]'s
/// own chunking: shaping one segment on every keystroke should cost a small, fixed amount
/// regardless of how large the rest of the block is (measured: 20-58 ms/keystroke into a
/// many-megabyte single block, shaping a full `PLAIN_CHUNK_BYTES` segment every time). Kept
/// well under `PLAIN_CHUNK_BYTES` deliberately: `RAW_SPLIT_THRESHOLD` (`tachyon-editor`) stays at
/// `PLAIN_CHUNK_BYTES * 2` so a `DocMode::Plain` block, always at most `PLAIN_CHUNK_BYTES`, never
/// crosses it and starts splitting into several of these on every repaint.
const RAW_SEGMENT_TARGET_BYTES: usize = 4 * 1024;

/// [`raw_segment_lens`]'s target line count, in the same ratio to [`RAW_SEGMENT_TARGET_BYTES`]
/// as [`PLAIN_CHUNK_LINES`] is to [`PLAIN_CHUNK_BYTES`].
const RAW_SEGMENT_TARGET_LINES: usize = 64;

/// [`raw_segment_lens`]'s forced-cut distance, half its target byte size like
/// [`PLAIN_FORCED_CUT_BYTES`] is half of [`PLAIN_CHUNK_BYTES`].
const RAW_SEGMENT_FORCED_CUT_BYTES: usize = 2 * 1024;

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
    /// `parsed.ir` was replaced with an empty placeholder because this block was far from the
    /// viewport (see [`Document::evict`]); `parsed.kind`/`len`/`source_hash`/`defs`/`refs`/
    /// `footnotes`/`footnotes_seen` are still current and correct - only the bulk rendered form
    /// is gone. [`Document::ensure_ir`] restores it on demand.
    ir_evicted: bool,
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

    /// Whether [`Document::evict`] replaced this block's `ir` with an empty placeholder;
    /// [`Document::ensure_ir`] restores it before anything reads `parsed().ir`.
    pub fn is_ir_evicted(&self) -> bool {
        self.ir_evicted
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
        match mode {
            DocMode::Markdown => doc.load_markdown(),
            DocMode::Plain => {
                let len = doc.buffer.len();
                let blocks = plain_chunk_lens(doc.buffer.rope(), 0..len)
                    .into_iter()
                    .map(|len| doc.new_block(md::plain(len), false))
                    .collect();
                doc.blocks = Arc::new(blocks);
                doc.recompute_starts();
            }
        }
        doc
    }

    /// Parses a fresh Markdown load the same way a large paste's dirty range is settled (ADR
    /// 0005), instead of one `md::parse_document` call over the whole text: tile the buffer into
    /// placeholder blocks and mark it all dirty, then apply [`LOAD_CHUNK`]-sized reparse windows
    /// in order until nothing is left dirty. Each window copies only its own bytes out of the
    /// rope (`ParseJob::run`), never the whole file, so a fresh open's peak extra memory is
    /// bounded by the window size, not the file size.
    ///
    /// Placeholder boundaries are [`md::presegment`] cuts (a blank line outside fenced code),
    /// not a blind byte count: `parse_job_near`'s look-behind (`follows_blank_line`) only trusts
    /// a boundary like that on its first check, so a window never needs to walk back into the
    /// whole of the previous one. A blind byte-count cut can land inside an open fence or
    /// mid-paragraph, which look-behind cannot trust; it then keeps walking back to the start of
    /// the file, so *every* later window re-parses everything parsed so far - quadratic in the
    /// number of windows, and worse than the single copy this was meant to avoid. A file that
    /// tiles into one enormous CommonMark block (a huge fenced code block, or one paragraph with
    /// no blank lines) has no such boundary anywhere: once a placeholder's own scan
    /// ([`LOAD_SCAN_MARGIN`] past [`LOAD_CHUNK`]) finds none, the rest of the text becomes one
    /// final placeholder and is parsed directly in one window (`in_tail`, below) rather than
    /// streamed - the same single copy that shape always needed, without paying for doomed
    /// windows that would otherwise discover it one chunk at a time. Forward references (a
    /// definition used before it appears) resolve correctly through the same deferred mechanism
    /// a self-referencing paste already relies on: blocks parsed before the whole file's
    /// definitions are known are re-marked dirty and reparsed once `Document::apply`'s deferred
    /// refresh (triggered when nothing else is dirty) finds they rendered against a stale table,
    /// never by re-scanning the whole text up front.
    fn load_markdown(&mut self) {
        let len = self.buffer.len();
        if len == 0 {
            self.blocks = Arc::new(Vec::new());
            self.recompute_starts();
            return;
        }
        let rope = self.buffer.rope().clone();
        let mut lens = Vec::new();
        let mut at = 0usize;
        let mut pathological = false;
        while len - at > LOAD_CHUNK {
            let mark = at + LOAD_CHUNK;
            let scan_limit = floor_char_boundary(&rope, (mark + LOAD_SCAN_MARGIN).min(len));
            // Zero-allocation pre-check: a safe cut needs a blank line at or past `mark`: if
            // there is none at all in range, there is certainly no fence-aware presegment
            // boundary either, so skip that copy entirely - the dominant saving for a file with
            // no blank line anywhere (a huge fenced block, or a long run of soft-wrapped prose).
            if !has_blank_line(&rope, mark, scan_limit) {
                pathological = true;
                break;
            }
            let slice = rope.byte_slice(at..scan_limit).to_string();
            match md::presegment(&slice).into_iter().find(|&b| b >= LOAD_CHUNK) {
                Some(cut) => {
                    lens.push(cut);
                    at += cut;
                }
                None => {
                    pathological = true;
                    break;
                }
            }
        }
        lens.push(len - at);
        let tail_start = at;
        let placeholders: Vec<Block> = lens
            .into_iter()
            .map(|chunk_len| self.new_block(md::unparsed(chunk_len), true))
            .collect();
        self.blocks = Arc::new(placeholders);
        self.recompute_starts();
        self.mark_dirty(0..len);
        let mut jobs = 0usize;
        loop {
            let in_tail = pathological && self.dirty.first().is_some_and(|d| d.start >= tail_start);
            let max_window = if in_tail { usize::MAX } else { LOAD_CHUNK };
            let Some(job) = self.parse_job_near(0, max_window) else { break };
            jobs += 1;
            debug_assert!(
                jobs <= 16 * self.blocks.len() + 1024,
                "initial load is not converging; dirty = {:?}",
                self.dirty
            );
            let result = job.run();
            self.apply(result);
        }
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

    /// Drops `ir` (replacing it with an empty placeholder) for every block whose index is
    /// outside `keep` (typically the visible blocks plus a margin, unioned with the caret's own
    /// block), is not stale, not already evicted, and at most [`EVICT_MAX_LEN`] bytes.
    /// `kind`/`len`/`source_hash`/`defs`/`refs`/`footnotes`/`footnotes_seen` stay exactly as
    /// parsed - the [`DefTable`] and every staleness check only ever read those, never `ir`, so
    /// definitions keep resolving document-wide exactly as before. [`Document::ensure_ir`]
    /// re-derives the dropped `ir` alone, when the block is needed again. A no-op in
    /// [`DocMode::Plain`], which has no IR to drop in the first place.
    pub fn evict(&mut self, keep: Range<usize>) {
        if self.mode == DocMode::Plain || self.blocks.is_empty() {
            return;
        }
        let keep = keep.start.min(self.blocks.len())..keep.end.min(self.blocks.len());
        let evictable = |block: &Block| {
            !block.stale
                && !block.ir_evicted
                && block.len <= EVICT_MAX_LEN
                && !block.parsed.ir.lines.is_empty()
        };
        if !(0..self.blocks.len()).any(|i| !keep.contains(&i) && evictable(&self.blocks[i])) {
            return;
        }
        let blocks = Arc::make_mut(&mut self.blocks);
        let mut evicted_count = 0usize;
        for (i, block) in blocks.iter_mut().enumerate() {
            if evicted_count >= EVICT_BATCH_LIMIT {
                break;
            }
            if keep.contains(&i) || !evictable(block) {
                continue;
            }
            // Built field-by-field instead of `(*block.parsed).clone()` then overwriting `ir`:
            // that would deep-clone `ir` (its `text`, and every line/run/map/link/leaf `Vec`)
            // only to immediately throw the clone away. A block's own parse never needs that -
            // this is exactly the "several thousand blocks evicted in one frame, right after a
            // large load" case, where the wasted clones alone cost tens of milliseconds.
            let p = &block.parsed;
            let evicted = ParsedBlock {
                kind: p.kind.clone(),
                len: p.len,
                content: p.content.clone(),
                ir: md::BlockIr::default(),
                defs: p.defs.clone(),
                refs: p.refs.clone(),
                footnotes: p.footnotes.clone(),
                footnotes_seen: p.footnotes_seen,
                source_hash: p.source_hash,
            };
            block.parsed = Arc::new(evicted);
            block.ir_evicted = true;
            evicted_count += 1;
        }
    }

    /// Restores `ir` for block `index` if [`Document::evict`] dropped it: a plain re-derivation
    /// of its own bytes against the current [`DefTable`], touching no other block - every block
    /// parses identically alone or in context (ADR 0005), which is exactly why this is safe
    /// without any of the look-behind/convergence a real edit needs. Nothing about a merely
    /// evicted block's source or its place in the document changed while it was evicted: a real
    /// edit instead marks it (and whatever it affects) stale, which the normal reparse-job path
    /// already handles - this only ever runs for a *clean* block, so its preserved `refs`/
    /// `footnotes_seen` are still exactly what the current [`DefTable`] would produce. A no-op
    /// if the block is not evicted (including every block in [`DocMode::Plain`]).
    pub fn ensure_ir(&mut self, index: usize) {
        let Some(block) = self.blocks.get(index) else { return };
        if !block.ir_evicted {
            return;
        }
        let range = self.block_range(index);
        let src = self.buffer.rope().byte_slice(range).to_string();
        let mut fresh = md::parse(&src, &self.defs);
        debug_assert_eq!(
            fresh.len(),
            1,
            "an evicted block's own bytes must parse alone as exactly one block (ADR 0005)"
        );
        let Some(parsed) = fresh.pop() else { return };
        let blocks = Arc::make_mut(&mut self.blocks);
        blocks[index].parsed = Arc::new(parsed);
        blocks[index].ir_evicted = false;
    }

    /// Byte ranges of `query` in the text, in order, not overlapping, at most
    /// [`MAX_FIND_MATCHES`]. See [`find_all_in_rope`], which does the work: this is just that
    /// applied to the document's own rope.
    pub fn find_all(&self, query: &str) -> Vec<Range<usize>> {
        find_all_in_rope(self.buffer.rope(), query)
    }

    /// Everything [`Document::find_all`] leaves out for `Replace All`: every match, not just up
    /// to [`MAX_FIND_MATCHES`], and the replaced span ready to splice in with one
    /// [`Document::edit`] - one undo step regardless of the match count. See
    /// [`replace_all_in_rope`], which does the work.
    pub fn replace_all(
        &self,
        query: &str,
        replacement: &str,
    ) -> Option<(Range<usize>, String, usize)> {
        replace_all_in_rope(self.buffer.rope(), query, replacement)
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
            new_blocks.push(Block { id, len, parsed, stale: false, ir_evicted: false });
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
        Block { id, len: parsed.len, parsed: Arc::new(parsed), stale, ir_evicted: false }
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
                let block = self.stale_block(0..new_total, None, true, 0, edit.new_len, cuts);
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
        let single_source = first == last;
        let start = self.starts[first];
        let len = self.starts[last + 1] - start - edit.range.len() + edit.new_len;
        let replacement = if len == 0 {
            Vec::new()
        } else {
            self.stale_block(
                start..start + len,
                Some(self.blocks[first].clone()),
                single_source,
                edit.range.start - start,
                edit.new_len,
                cuts,
            )
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

    /// Blocks for a changed range `range` (current coordinates): one stale block keeping
    /// `keep`'s identity, or provisional unparsed blocks when the range is large.
    /// `single_source`: whether `range` is exactly the one pre-existing block the edit touched,
    /// with nothing else merged in (see [`Document::boundaries`]). `edit_at`/`edit_len`: where
    /// in `range` the edit's own new text lies. `cuts`: that new text's own pre-segmenter
    /// boundaries, if already known ([`PreparedInsert`]).
    fn stale_block(
        &mut self,
        range: Range<usize>,
        keep: Option<Block>,
        single_source: bool,
        edit_at: usize,
        edit_len: usize,
        cuts: Option<&[usize]>,
    ) -> Vec<Block> {
        let len = range.len();
        if len > UNPARSED_SPLIT_THRESHOLD {
            let boundaries = self.boundaries(range, single_source, edit_at, edit_len, cuts);
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

    /// Pre-segmenter boundaries of `range`, relative to its start. `single_source`: whether
    /// `range` is exactly the one pre-existing block the edit touched, with nothing else merged
    /// in - only then can the fast path below assume the whole range shares one fence state (a
    /// merge of several original blocks may include one that is itself a fence, or a container
    /// able to wrap one).
    ///
    /// Boundaries computed for the edit's own inserted text (`cuts`, from [`PreparedInsert`]) are
    /// reused when they mean the same in place: the insert starts a line and nothing before it
    /// in `range` leaves a fence open. Boundaries in the text after the insert are left out
    /// (fewer, larger provisional blocks; still correct).
    ///
    /// Otherwise, above [`BOUNDARY_FULL_SCAN_LIMIT`], presegmenting the whole range - even
    /// copy-free - costs real time on every keystroke into a block that large (`Document::edit`
    /// never carries `cuts`: only [`PreparedInsert`]'s paste path does). A `single_source` block
    /// parses identically alone (ADR 0005's segmenter invariant), so one already big enough to
    /// reach here had *no* interior boundary before this edit (else it would have been split
    /// then), and any *new* boundary this edit creates can only appear near where it happened:
    /// the rest of `range` is provably unchanged since the last time this same argument applied
    /// to it. Whether a bounded window around the edit can be presegmented on its own instead of
    /// the whole range depends on whether the fence/HTML state a window elsewhere in `range`
    /// would need to start from is trivially known without scanning up to it: a fence never
    /// re-opens inside one CommonMark block once closed - a line that could would end the block
    /// for real - so a block whose first line opens one is fence-open throughout, and gets no
    /// boundaries at all ([`block_shape`]'s `Fence`); a block whose first line opens neither a
    /// fence nor a container (a list item or block quote, whose own nested content can open a
    /// fence at a column a top-level line never could) nor an HTML block is fence-and-HTML-free
    /// throughout, and a bounded window anywhere in it is safe (`Plain`). An HTML block's first
    /// line does *not* similarly make the rest of `range` safe to assume: unlike a fence, most
    /// HTML block types (`tachyon_md::presegment`'s module doc comment has the full CommonMark
    /// rules) end at a specific pattern - a blank line for some, a line containing `-->`, `?>`,
    /// `>` or `]]>` for others - that a bounded window checks for on every line it scans just
    /// like a real fence's own closing marker, so - unlike genuine `Plain` content - the state
    /// a bounded window elsewhere in `range` would need to start from is "still inside the HTML
    /// block opened at `range.start`, watching for *its* end pattern", which a window starting
    /// fresh partway through cannot represent; a fence-marker-shaped line the edit adds there
    /// would misread as a real fence the same way one swallowed by an HTML block already did
    /// once (`tachyon_md::presegment`'s module doc comment). A container, an HTML block, or a
    /// first line too long to read within [`BOUNDARY_FENCE_PEEK_LIMIT`] therefore falls back to
    /// the full scan below (`Unknown`).
    fn boundaries(
        &self,
        range: Range<usize>,
        single_source: bool,
        edit_at: usize,
        edit_len: usize,
        cuts: Option<&[usize]>,
    ) -> Vec<usize> {
        let rope = self.buffer.rope();
        if let Some(cuts) = cuts {
            let prefix_end = range.start + edit_at;
            let starts_line = edit_at == 0 || rope.byte(prefix_end - 1) == b'\n';
            #[cfg(test)]
            record_presegment_bytes(prefix_end - range.start);
            if starts_line
                && !md::ends_in_fence_chunks(rope.byte_slice(range.start..prefix_end).chunks(), 0)
            {
                return cuts.iter().map(|cut| edit_at + cut).collect();
            }
        }
        if single_source && range.len() > BOUNDARY_FULL_SCAN_LIMIT {
            match block_shape(rope, &range) {
                BlockShape::Fence => return Vec::new(),
                BlockShape::Plain => {
                    let lo = edit_at.saturating_sub(BOUNDARY_SCAN_MARGIN);
                    let hi = (edit_at + edit_len + BOUNDARY_SCAN_MARGIN).min(range.len());
                    #[cfg(test)]
                    record_presegment_bytes(hi - lo);
                    return md::presegment_chunks(
                        rope.byte_slice(range.start + lo..range.start + hi).chunks(),
                        lo,
                    );
                }
                BlockShape::Unknown => {}
            }
        }
        #[cfg(test)]
        record_presegment_bytes(range.len());
        md::presegment_chunks(rope.byte_slice(range.clone()).chunks(), 0)
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

/// Byte ranges of `query` in `rope`, in order, not overlapping, at most [`MAX_FIND_MATCHES`] (see
/// [`find_all_in_rope_unbounded`] for the same scan without that cap, for `Replace All`). Smart
/// case: case-insensitive (ASCII letters) unless `query` contains an uppercase letter. An empty
/// query matches nothing.
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
    find_all_in_rope_bounded(rope, query, Some(MAX_FIND_MATCHES))
}

/// Like [`find_all_in_rope`], but never stops at [`MAX_FIND_MATCHES`]: that cap exists only for
/// the display count and highlighting, and `Replace All` must still replace every match past it
/// (a 200 MB log with ~530k matches of `status=200` silently replacing only the first 10,000,
/// with no notice, was the bug this exists to fix). Same scan, same bounded extra memory; only
/// the stopping condition differs. See [`replace_all_in_rope`], which is what `Replace All`
/// actually calls.
pub fn find_all_in_rope_unbounded(rope: &Rope, query: &str) -> Vec<Range<usize>> {
    find_all_in_rope_bounded(rope, query, None)
}

fn find_all_in_rope_bounded(rope: &Rope, query: &str, limit: Option<usize>) -> Vec<Range<usize>> {
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
            if limit.is_some_and(|limit| matches.len() >= limit) {
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

/// The span from the first match of `query` in `rope` to the last, `replacement` spliced in at
/// each one, and how many matches it covers: everything `Replace All` needs to apply as one
/// [`Document::edit`] (one undo step), computed in a single pass so it can run entirely off the
/// UI thread. `None` if `query` is empty or has no matches.
///
/// Two allocations scale with the span, not with the match count: the matched span extracted
/// from `rope` once (`text`) and the replaced text built from it (`replaced`). Building
/// `replaced` directly from `matches` (rather than, say, one document edit per match) is what
/// keeps a multi-million-match Replace All to roughly one extra copy of the span instead of
/// millions of tiny ones.
pub fn replace_all_in_rope(
    rope: &Rope,
    query: &str,
    replacement: &str,
) -> Option<(Range<usize>, String, usize)> {
    let matches = find_all_in_rope_unbounded(rope, query);
    let span = matches.first()?.start..matches.last()?.end;
    let text = rope.byte_slice(span.clone()).to_string();
    let mut replaced = String::with_capacity(text.len());
    let mut at = span.start;
    for m in &matches {
        replaced.push_str(&text[at - span.start..m.start - span.start]);
        replaced.push_str(replacement);
        at = m.end;
    }
    Some((span, replaced, matches.len()))
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

/// Whether any line in `[from, before)` is blank (all whitespace): walks the rope's own chunks
/// via [`find_newline`], never copying. `Document::load_markdown` uses this as a cheap
/// pre-check before a presegment scan's copy - a safe cut needs a blank line, so if there is
/// none at all in range there is certainly no presegment boundary either.
fn has_blank_line(rope: &Rope, from: usize, before: usize) -> bool {
    let mut pos = from;
    while let Some(nl) = find_newline(rope, pos, before) {
        if rope.byte_slice(pos..nl).chars().all(char::is_whitespace) {
            return true;
        }
        pos = nl + 1;
    }
    false
}

/// What [`Document::boundaries`]'s bounded-scan fast path can assume about a `single_source`
/// block's interior fence state, judged from its own first line alone (`range.start` truly is
/// this block's own start whenever `single_source` holds, and stays so across every stale
/// transition an edit inside it makes, since none of them ever touch byte 0 of the block).
enum BlockShape {
    /// `range`'s first line opens a fence (and does not open an HTML block instead - see
    /// `block_shape`). ADR 0005's segmenter invariant means a `single_source` block parses
    /// identically alone, so one whose first line opens a fence never closes and reopens one
    /// inside itself - closing it for real would end the block. Every interior blank line is
    /// therefore inside that fence, so `range` can never gain a boundary.
    Fence,
    /// `range`'s first line neither opens a fence or an HTML block, nor could start a construct
    /// (a list item, a block quote) whose own nested content might open one at a raw column a
    /// top-level line never could. Fence state is `None` throughout `range`, so a bounded window
    /// anywhere in it is safe to presegment on its own.
    Plain,
    /// Anything else: a container, an HTML block, or a first line too long to read within
    /// [`BOUNDARY_FENCE_PEEK_LIMIT`]. [`Document::boundaries`] falls back to the full scan.
    Unknown,
}

fn block_shape(rope: &Rope, range: &Range<usize>) -> BlockShape {
    let peek_limit = (range.start + BOUNDARY_FENCE_PEEK_LIMIT).min(range.end);
    let Some(nl) = find_newline(rope, range.start, peek_limit) else { return BlockShape::Unknown };
    let first_line = rope.byte_slice(range.start..nl + 1).to_string();
    // An HTML block, unlike a fence, ends at its own pattern - a blank line for some types, a
    // line containing a specific closing sequence for others (`tachyon_md::presegment`'s module
    // doc comment has the full CommonMark rules) - rather than swallowing every line
    // indefinitely: an edit elsewhere in `range` (Fence's own reasoning does not apply) can
    // still create a genuinely new boundary there, which only the full scan below is guaranteed
    // to find - a bounded window starting fresh partway through the block would not know it is
    // still "inside HTML" and could misread a fence-marker-shaped line the same way a swallowed
    // one already did once. Checked before `ends_in_fence`, which also reports true for an
    // HTML-opening line (an HTML block swallows fence markers exactly like a fence swallows
    // blank lines), so this must not fall through to the `Fence` arm below.
    if md::opens_html_block(&first_line) {
        return BlockShape::Unknown;
    }
    if md::ends_in_fence(&first_line) {
        return BlockShape::Fence;
    }
    if opens_container(&first_line) {
        return BlockShape::Unknown;
    }
    BlockShape::Plain
}

/// Whether `line`, after up to three leading spaces (the same indentation `presegment`'s own
/// fence detection allows), starts a list item or a block quote marker.
fn opens_container(line: &str) -> bool {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let bytes = rest.as_bytes();
    match bytes.first() {
        Some(b'>') => true,
        Some(b'-' | b'+' | b'*') => {
            bytes.get(1).is_none_or(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        }
        Some(b'0'..=b'9') => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            digits <= 9
                && matches!(bytes.get(digits), Some(b'.' | b')'))
                && bytes.get(digits + 1).is_none_or(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        }
        _ => false,
    }
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
/// `rope`'s text. `cap` is the whole document for most callers; `Editor::render_raw`/
/// `render_rendered` also call [`plain_chunk_lens`] with `cap` set to one Markdown block's own
/// end, to bound how much of one oversized block is laid out at once without reading into the
/// next block's text.
///
/// `cap` needs no precondition (unlike a plain byte offset picked at random, it is always itself
/// a valid chunk boundary in the caller's own terms: the end of the document, or the start/end of
/// a block the caller is not touching) - this only ever *shrinks* the chunk that would otherwise
/// be returned, cutting at `cap` instead of the usual stopping point, and `cap` is exactly as
/// valid a place to cut as whatever `cap` itself bounds.
/// Unlike the byte-offset-based `find_newline`, this walks the rope's own chunk iterator once
/// (`rope.byte_slice(start..cap).chunks()`, one `O(log n)` descent total) and advances through it
/// sequentially, checking each chunk's own bytes with `str::find` instead of re-descending the
/// rope tree once per line: a naive per-line `rope.byte_slice(pos..force_at)` (as `find_newline`
/// does) costs one tree traversal *per line*, which for a file with many short real lines (not
/// the pathologically-long-single-line shape this forced cut defends against) turned a 15 MiB,
/// ~200,000-line scan into a 300 ms frame - proportional to line count times tree depth, not to
/// bytes scanned. `floor_char_boundary` (also `O(log n)`) only runs when a forced cut is actually
/// about to happen (a line already past `PLAIN_FORCED_CUT_BYTES` with no `\n`), not per line.
fn plain_chunk_len_capped(
    rope: &Rope,
    start: usize,
    cap: usize,
    target_bytes: usize,
    target_lines: usize,
    forced_cut_bytes: usize,
) -> usize {
    if start >= cap {
        return 0;
    }
    let mut lines = 0usize;
    let mut line_start = start;
    let mut abs = start;
    for piece in rope.byte_slice(start..cap).chunks() {
        let mut offset = 0usize;
        while offset < piece.len() {
            let force_at = (line_start + forced_cut_bytes).min(cap);
            if abs >= force_at {
                return floor_char_boundary(rope, force_at) - start;
            }
            match piece[offset..].find('\n') {
                Some(rel) => {
                    let nl = abs + rel;
                    if nl >= force_at {
                        return floor_char_boundary(rope, force_at) - start;
                    }
                    lines += 1;
                    let len_so_far = nl + 1 - start;
                    if len_so_far >= target_bytes || lines >= target_lines || nl + 1 >= cap {
                        return len_so_far;
                    }
                    offset += rel + 1;
                    abs = nl + 1;
                    line_start = abs;
                }
                None => {
                    abs += piece.len() - offset;
                    offset = piece.len();
                    if abs >= force_at {
                        return floor_char_boundary(rope, force_at) - start;
                    }
                }
            }
        }
    }
    // Reached `cap` with the last line still unterminated: force a cut only if it is itself
    // already past the threshold, exactly like the loop above; otherwise the whole remainder (at
    // most `cap - start`, itself always a valid cut point) is one chunk.
    let force_at = (line_start + forced_cut_bytes).min(cap);
    if abs >= force_at { floor_char_boundary(rope, force_at) - start } else { cap - start }
}

/// Chunk lengths tiling `range` exactly, packed greedily to the maximum:
/// [`plain_chunk_len_capped`] repeatedly from `range.start`, capped at `range.end`. Used for a
/// fresh span with nothing past its own end to answer to - the whole document at load, or new
/// content mid-document before [`Document::on_edit_plain`] decides how much of what surrounds it
/// to fold in - never for re-chunking an existing span in place (see [`split_evenly`]). Also used
/// by `tachyon-editor` to bound how much of one oversized Markdown block is laid out at once
/// (`range` is that block's own byte range, `plain_chunk_len_capped`'s `cap` its end), hence `pub`.
pub fn plain_chunk_lens(rope: &Rope, range: Range<usize>) -> Vec<usize> {
    chunk_lens_targeting(rope, range, PLAIN_CHUNK_BYTES, PLAIN_CHUNK_LINES, PLAIN_FORCED_CUT_BYTES)
}

/// Like [`plain_chunk_lens`], but targeting [`RAW_SEGMENT_TARGET_BYTES`]/
/// [`RAW_SEGMENT_TARGET_LINES`] instead of [`DocMode::Plain`]'s own chunk size:
/// `Editor::render_raw` uses this, once a block is already over `RAW_SPLIT_THRESHOLD` and being
/// split, so re-shaping the one segment a keystroke actually touches costs a small, fixed amount
/// regardless of how large the rest of the block is, instead of up to a whole `PLAIN_CHUNK_BYTES`
/// segment's worth of monospace text every time.
pub fn raw_segment_lens(rope: &Rope, range: Range<usize>) -> Vec<usize> {
    chunk_lens_targeting(
        rope,
        range,
        RAW_SEGMENT_TARGET_BYTES,
        RAW_SEGMENT_TARGET_LINES,
        RAW_SEGMENT_FORCED_CUT_BYTES,
    )
}

fn chunk_lens_targeting(
    rope: &Rope,
    range: Range<usize>,
    target_bytes: usize,
    target_lines: usize,
    forced_cut_bytes: usize,
) -> Vec<usize> {
    let mut lens = Vec::new();
    let mut at = range.start;
    while at < range.end {
        let len = plain_chunk_len_capped(
            rope,
            at,
            range.end,
            target_bytes,
            target_lines,
            forced_cut_bytes,
        );
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
            let len = plain_chunk_len_capped(
                rope,
                at,
                range.end,
                PLAIN_CHUNK_BYTES,
                PLAIN_CHUNK_LINES,
                PLAIN_FORCED_CUT_BYTES,
            );
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
