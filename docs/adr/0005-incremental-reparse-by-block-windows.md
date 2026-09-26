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
whole blocks: one block of look-behind (an edit can change how the preceding block ends: setext
underlines, lazy continuation), the dirty blocks, and one block of look-ahead. The window grows
geometrically until the parse **converges**: its last block equals the old block at the same place
(same length, source hash and kind) or the window reaches the end of the text.

This is only correct if every block parses identically on its own as inside the document. The
segmenter in `tachyon-md` enforces that:

1. **Blocks start at line starts.** pulldown-cmark reports some blocks without their indentation
   (indented code); a block cut mid-line would parse differently alone. The previous block's
   *real content* end (not its container range, which can run into the next line's indentation)
   bounds the cut. Two blocks that start on the same line merge into one, so no block is ever
   empty (an empty block cannot be addressed by offset, and reparsing looped on it).
2. **Link reference definitions directly before a block belong to it.** After a definition the
   parser is in a paragraph-like state ("[x]: /u\n2) two" is a paragraph, "2) two" alone is a
   list), and pulldown-cmark may attribute the definition to the previous container's range.
   Definitions separated by a blank line become their own `LinkDefinition` block.
3. **Definitions are found from gaps**, not from the parser's definition map, which drops duplicate
   labels.
4. **Document-wide references go through a table.** Reference links resolve against a `DefTable`
   built from all blocks (first definition wins); footnote references resolve by prepending the
   document's footnote definitions to each window, closed off by a thematic break. When the table
   changes, blocks that looked up a changed label (or mention footnotes) are reparsed.

Jobs own an immutable snapshot (rope clone, block list `Arc`, table `Arc`) and are `Send`. The
caller runs small jobs inline and large ones on a background executor. A result is applied only if
no edit touched its window since the snapshot (checked through the buffer's edit log); otherwise the
window is marked dirty again. One job is outstanding at a time.

## Consequences

- Measured on a 1 MB document (`cargo bench -p tachyon-doc`): keystroke-to-clean p99 ≈ 45 µs, full
  parse ≈ 20 ms, 10 MB ≈ 220 ms, an unclosed fence reaching the end of 512 KB ≈ 7 ms.
- The invariant is checked by property tests (random documents, edits, undo, interleaved jobs,
  streamed input) against `tachyon_md::parse_document`, and by corpus tests that try every
  keystroke position. Any rule above that is removed makes those tests fail.
- The remaining per-keystroke cost is O(blocks) bookkeeping (prefix sums, `Vec` splice). Replace
  with a sum tree if documents with far more blocks need it.
- A `\r` ending an insert is dropped by the buffer (half of a CRLF split across streamed chunks).
- Known gap: nested footnote definitions on one line (`[^1]:[^1]: x`) can render differently
  after incremental edits than after a full parse, because pulldown-cmark attributes their ranges
  inconsistently. For them only termination and tiling are tested; a full reload fixes the view.
