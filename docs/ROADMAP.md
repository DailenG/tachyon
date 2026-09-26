# Roadmap

Each phase has exit criteria that must be met, with evidence, before the phase is called done. A
later phase may start early only when it does not depend on the open criterion (Phase 2's core
crates are needed whichever way the startup gate goes).

## Phase 1: skeleton and startup gate (open: needs reference Windows hardware)

- [x] Workspace, pinned toolchain and GPUI revision, release profile, lints
- [x] GPUI window showing Markdown as raw text (sample, files, clipboard)
- [x] Single-instance handoff: Windows named pipe, Unix socket
- [x] Startup instrumentation and `cargo xtask bench-startup`
- [x] CI on Windows (primary), Linux, macOS; cargo-deny; Windows release artifact
- [ ] **Gate:** p95 cold and warm startup measured on reference Windows hardware, and the
      direct-launch vs resident-mode decision recorded in [ADR 0004](adr/0004-startup-budget-and-gate.md)

Measured so far (release, 20 runs, `cargo xtask bench-startup`):

| Machine | p50 | p95 | Breakdown (p50, from `main`) |
|---|---|---|---|
| Linux, Intel UHD 750, Wayland | 193 ms | 199 ms | platform 10 ms, window open +180 ms |
| Windows, GitHub `windows-latest` runner (no GPU; not reference hardware) | 142 ms | 161 ms | platform 60 ms, window open +44 ms, first frame +23 ms; ~16 ms before `main` |

Both miss the 50 ms budget. The runner numbers only show that on Windows platform
initialization alone can exceed the budget; the gate still needs a real machine.

## Phase 2: headless core (`tachyon-text`, `tachyon-md`, `tachyon-doc`) (done)

- [x] Buffer: edits, grouped undo, UTF-16 mapping, line-ending round trip, edit log
- [x] Block segmentation, owned IR with source maps, `DefTable` (links and footnotes),
      fence-aware pre-segmenter
- [x] Incremental reparse with the convergence rule; executor-agnostic `ParseJob`s
      ([ADR 0005](adr/0005-incremental-reparse-by-block-windows.md))
- [x] Property tests: incremental parse == full parse for random edits, undo, interleaved jobs
      and streamed input (run with `PROPTEST_CASES=1000000` before changing the segmenter)
- [x] LLM-style corpus: fences in lists, nested lists, tables, alerts, references, footnotes, math,
      HTML, output truncated inside a fence, CRLF. Hand-written; extend it with captured model
      output when a rendering bug shows up.
- [x] Latency bench: `cargo bench -p tachyon-doc`

**Exit:** keystroke reparse p99 < 0.5 ms on a 1 MB document; every corpus file matches a full parse.

Measured (Linux, release): keystroke-to-clean p99 45 µs on 1 MiB (paragraph, code block and
end-of-document typing); full parse 20 ms for 1 MiB, 220 ms for 10 MiB; unclosed fence running to
the end of 512 KiB 7 ms. All corpus tests pass.

## Phase 3: block-swap editor (`tachyon-editor`) (in progress)

- [x] Editor view on GPUI's virtualized list; rich rendering for headings, emphasis, inline code,
      links, fences, lists, task lists, quotes, tables, rules, math
- [x] Raw rendering for the active block; swap on cursor movement
- [x] Swap at leaf granularity inside lists, quotes and footnotes (list item text, paragraph, code
      block); other blocks swap whole
- [x] Arrow keys, words, home/end, document start/end; selections across blocks; copy/cut/paste;
      undo/redo with typing-run grouping
- [x] Clicks and drag selection through rendered blocks via source maps (implemented; not yet
      covered by an automated test)
- [x] Scroll anchoring: the list scrolls by item offset, so blocks changing height above the
      viewport do not move it; the view follows the caret's line (not its block) and edits inside
      the top block keep their pixel offset
- [x] IME through GPUI's input handler (composition is one undo step)
- [ ] IME verified with a Japanese or Chinese IME on Windows
- [x] Paste path: large pastes show unparsed blocks and parse on the background executor; a 5 MB
      paste costs ≈ 6 ms on the UI thread plus ≈ 8 ms when the background parse is applied
- [ ] Background parse streamed back in chunks, viewport first
- [x] Frame-time overlay (`Ctrl+Alt+F`): p50/max of the editor's render+layout+paint and frames
      over 16.7 ms. On a 5 MB document while scrolling and typing: p50 1.4 ms, max 3.0 ms
- [ ] Measure a live 5 MB paste with the overlay (exit criterion)
- [x] Save / Save As with atomic writes; unsaved-changes prompt on close and quit
- [x] Keyboard support in prompts on Linux (own in-window prompt; Windows and macOS keep native
      dialogs)

**Exit:** pasting 5 MB of LLM output produces no frame over 16.6 ms, with visible text in the same
frame; typing in a 1 MB document keeps key-to-present p99 within one 60 Hz frame.
