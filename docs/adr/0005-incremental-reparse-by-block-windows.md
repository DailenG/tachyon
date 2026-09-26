# 0005: Incremental reparse by block windows

- **Status:** Accepted
- **Date:** 2026-09-25

## Context

`pulldown-cmark` parses a whole `&str` at once. Reparsing a 1 MB document per keystroke costs about
20 ms, far over the 0.5 ms keystroke budget. The editor needs a reparse whose result is *exactly*
what a full parse would give, since rendered blocks must never disagree with the source.

## Decision

The document is a list of top-level blocks that tile the text. An edit merges the blocks it
touches into one *stale* block and marks its range dirty. A reparse job parses a **window** of
whole blocks: look-behind back to the last blank line (an edit can change how preceding blocks end:
setext underlines, lazy continuation, and lines that continue a paragraph only if the *next* line
does not start a table, which reaches more than one block back; no construct continues across a
blank line into a separate block), the dirty blocks, and one block of look-ahead. The window grows
geometrically until the parse **converges**: its last block equals the old block at the same place
(same length, source hash and kind) or the window reaches the end of the text.

This is only correct if every block parses identically on its own as inside the document. The
segmenter in `tachyon-md` enforces that:

1. **Blocks start at line starts.** pulldown-cmark reports some blocks without their indentation
   (indented code); a block cut mid-line would parse differently alone. The previous block's
   *real content* end (not its container range, which can run into the next line's indentation)
   bounds the cut. Two blocks that start on the same line merge into one, so no block is ever
   empty (an empty block cannot be addressed by offset, and reparsing looped on it). A block whose
   line is indented like code (4 columns) but that is not indented code merges into the previous
   one: it continues a container that pulldown-cmark closed early (a footnote definition nested
   in another's continuation lines) and would be indented code on its own.
2. **Link reference definitions directly before a block belong to it.** After a definition the
   parser is in a paragraph-like state ("[x]: /u\n2) two" is a paragraph, "2) two" alone is a
   list), and pulldown-cmark may attribute the definition to the previous container's range.
   Definitions separated by a blank line become their own `LinkDefinition` block.
3. **Definitions are found from gaps**, not from the parser's definition map, which drops duplicate
   labels.
4. **Document-wide references go through a table.** Reference links resolve against a `DefTable`
   built from all blocks (first definition wins); footnote references resolve by prepending the
   document's footnote definitions to each window, closed off by a thematic break. Each block
   records the entry it found for every label it looked up (including "undefined"); it is reparsed
   when the table now answers differently, or, if it mentions footnotes, when the set of footnotes
   changed. A job whose window changes the definitions parses the window once more against the
   table as it will be after applying, so a paste that defines its own references is not reparsed
   block by block afterwards (for 5 MB of such text that was ≈ 7800 jobs, 2 s on the UI thread).

Jobs own an immutable snapshot (rope clone, block list `Arc`, table `Arc`) and are `Send`. The
caller runs small jobs inline and large ones on a background executor. A result is applied only if
no edit touched its window since the snapshot (checked through the buffer's edit log); otherwise the
window is marked dirty again. One job is outstanding at a time.

A large dirty range (a paste) is **streamed back in chunks**: `parse_job_near(focus, max)` takes
the dirty range nearest `focus` (the caret after an edit, else the top of the viewport) and, if it
is longer than `max`, a window of about `max` bytes starting at `focus` (moved back so it is full
size when `focus` is near the end). Such a window skips convergence and look-ahead, because the
dirty text after it is parsed by a later job whose look-behind reaches back into this window's last
block, which may have been cut short. Dirty text left before the window is parsed by a job that
converges against the window's blocks, as for any edit.

## Consequences

- Measured on a 1 MB document (`cargo bench -p tachyon-doc`): keystroke-to-clean p99 ≈ 45 µs, full
  parse ≈ 20 ms, 10 MB ≈ 220 ms, an unclosed fence reaching the end of 512 KB ≈ 7 ms. A 5 MB paste
  costs ≈ 6 ms as a plain edit (rope insert, pre-segmenting into IR-free placeholders), or 0.4 ms
  on the UI thread when prepared off it (`PreparedInsert`, ≈ 5 ms elsewhere); then ≈ 170 ms of
  background parsing unchunked (twice ≈ 80 ms: the paste defines the references it uses) and
  ≈ 7 ms to apply the result. Streamed in 128 KiB chunks it takes 38 jobs; the one at the caret is
  parsed in ≈ 4.5 ms and applied in ≈ 13 µs, the others are applied in p50 ≈ 80 µs, the last one
  ≈ 5 ms (the one deferred rebuild of the definition table).
- Documents without blank lines between blocks get larger reparse windows (slower keystrokes, same
  results).
- The invariant is checked by property tests (random documents, edits, undo, interleaved jobs,
  streamed input) against `tachyon_md::parse_document`, and by corpus tests that try every
  keystroke position. Any rule above that is removed makes those tests fail.
- The remaining per-keystroke cost is O(blocks) bookkeeping (prefix sums, `Vec` splice). Replace
  with a sum tree if documents with far more blocks need it.
- A `\r` ending an insert is dropped by the buffer (half of a CRLF split across streamed chunks).
- Known gap: nested footnote definitions on one line (`[^1]:[^1]: x`) can render differently
  after incremental edits than after a full parse, because pulldown-cmark attributes their ranges
  inconsistently. For them only termination and tiling are tested; a full reload fixes the view.
