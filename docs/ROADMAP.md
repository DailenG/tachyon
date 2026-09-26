# Roadmap

Each phase has exit criteria that must be met, with evidence, before the next one starts.

## Phase 1: skeleton and startup gate (current)

- [x] Workspace, pinned toolchain and GPUI revision, release profile, lints
- [x] GPUI window showing Markdown as raw text (sample, files, clipboard)
- [x] Single-instance handoff: Windows named pipe, Unix socket
- [x] Startup instrumentation and `cargo xtask bench-startup`
- [x] CI on Windows (primary), Linux, macOS; cargo-deny; Windows release artifact
- [ ] **Gate:** p95 cold and warm startup measured on reference Windows hardware, and the
      direct-launch vs resident-mode decision recorded in [ADR 0004](adr/0004-startup-budget-and-gate.md)

Measured so far (release, 20 runs): Linux, Intel UHD 750, Wayland: p50 193 ms, p95 199 ms, of
which about 180 ms is inside GPUI's `open_window`. Windows: not yet measured.

## Phase 2: headless core (`tachyon-text`, `tachyon-md`, `tachyon-doc`)

- Buffer: edits, grouped undo, UTF-16 mapping, line-ending round trip, edit log
- Block segmentation, owned IR with source maps, `RefDefs`, fence-aware pre-segmenter
- Incremental reparse with the convergence rule; executor-agnostic parse scheduler
- Property test: incremental parse == full parse for random edit sequences
- Corpus of real LLM output: fences in lists, nested lists, tables, math, output truncated inside
  a fence, CRLF input
- Criterion benches: keystroke reparse on 1 MB, full parse of 1 MB and 10 MB

**Exit:** keystroke reparse p99 < 0.5 ms on a 1 MB document; every corpus file matches a full parse.

## Phase 3: block-swap editor (`tachyon-editor`, `tachyon-theme`)

- Editor view on GPUI's virtualized list; rich rendering for headings, emphasis, inline code,
  links, fences, lists, quotes, basic tables; raw rendering for the active leaf block
- Swap on cursor movement with scroll anchoring; arrow keys and clicks through rendered blocks via
  source maps; selections across rendered blocks
- IME through GPUI's input handler, verified with a Japanese or Chinese IME on Windows
- Paste path: pending blocks, then background parse streamed in, viewport first
- Frame-time overlay

**Exit:** pasting 5 MB of LLM output produces no frame over 16.6 ms, with visible text in the same
frame; typing in a 1 MB document keeps key-to-present p99 within one 60 Hz frame.
