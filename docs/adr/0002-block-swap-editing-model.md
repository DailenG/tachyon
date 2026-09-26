# 0002: Block-swap editing model

- **Status:** Accepted
- **Date:** 2026-09-25

## Context

The editor must feel like Typora (one pane, inline WYSIWYG) without WebViews, and never drop frames
on keystrokes or multi-megabyte pastes of LLM output. `pulldown-cmark` is fast but not incremental,
and it borrows from a contiguous `&str` while the buffer is a rope.

## Decision

- The raw Markdown buffer is the only source of truth. There is no separate rich-text model to keep
  in sync.
- The leaf block containing the cursor renders as raw Markdown; all other blocks render from an
  owned, block-relative IR with syntax tokens hidden.
- Reparsing is per window of top-level blocks with a convergence rule (see
  [ARCHITECTURE](../ARCHITECTURE.md#concurrency)); small windows parse synchronously, large ones on
  the background executor.
- Swapping is block-level, not inline: syntax is revealed for the whole active block.

## Consequences

- Keystrokes never wait for a parse; the active block needs no IR to draw.
- Block heights change on swap, so the view needs scroll anchoring.
- Cursor movement through rendered blocks needs source maps between visible and source ranges.
- Inline-level reveal (only the span under the cursor) is out of scope; it would need a different
  ADR.
