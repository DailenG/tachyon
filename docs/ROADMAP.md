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

| Windows 11, reference (Core Ultra 7 155H, Intel Arc), 4K @ 30 Hz | 447 ms | 511 ms | platform 288 ms, window open +111 ms, first frame +21 ms |
| Same machine, 1440p @ 59 Hz | 280 ms | 324 ms | platform 180 ms, window open +67 ms, first frame +12 ms |

All miss the 50 ms budget. On the reference Windows machine GPUI's platform initialization alone
takes 180-330 ms.

Warm launches into a resident instance (`--resident`, `cargo xtask bench-startup --warm`) meet it
on Linux: p50 27.6 ms, p95 29.5 ms (first launch into a windowless instance 180 ms), but not on
the reference Windows machine: p95 150-188 ms, of which the resident instance's window open to
first frame is 86-93 ms. Next: a timing-instrumented GPUI build on that machine to attribute
platform init and per-window costs. See ADR 0004.

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
- [x] Clicks, double-click word selection and drag selection through rendered blocks via source
      maps (covered by `gpui::test` mouse tests)
- [x] Scroll anchoring: the list scrolls by item offset, so blocks changing height above the
      viewport do not move it; the view follows the caret's line (not its block) and edits inside
      the top block keep their pixel offset
- [x] IME through GPUI's input handler (composition is one undo step)
- [x] IME verified by hand on Windows with Microsoft IME (Japanese) and Microsoft Pinyin: candidate
      window at the caret, underlined composition, commit, Escape cancels, one undo step. Synthetic
      keystrokes bypass Windows IMEs, so this stays a manual check
- [x] Paste path: large pastes show unparsed blocks and parse on the background executor; a 5 MB
      paste costs ≈ 6 ms on the UI thread, then 38 chunks applied in ≤ 7 ms each (the caret's
      first, ≈ 0.1 ms); a paste that defines its own references no longer triggers a reparse of
      every block that uses them
- [x] Background parse streamed back in chunks, caret or viewport first (`parse_job_near`)
- [x] Frame-time overlay (`Ctrl+Alt+F`): p50/max of the editor's UI-thread time per frame
      (render+layout+paint plus edits, pastes and applied parse results that ran back to back
      with it) and frames over 16.7 ms. On a 5 MB document while scrolling and typing: p50
      1.4 ms, max 3.0 ms
- [x] Measure a live 5 MB paste with the overlay. Linux (Intel UHD 750, nested Hyprland): the
      paste frame takes ≈ 31 ms (clipboard read inside GPUI ≈ 7 ms, rope insert and pre-segmenting
      ≈ 8 ms, first paint of the raw placeholder under the caret ≈ 14 ms); every later frame stays
      under budget (p50 2.5 ms) while the chunks stream in, and the view stays at the caret
- [x] Paste frame within budget on Linux. A paste over 64 KiB is normalized, built into a rope
      and pre-segmented off the UI thread (`PreparedInsert`), then spliced in (0.4 ms for 5 MB
      instead of 6 ms); placeholders within 32 KiB of the caret are ≥ 1 KiB, so the first frame
      lays out about a screenful; fonts are loaded after the first frame, not by the first
      paste. Live, 5 MB from an empty scratch window: max frame 6.6-6.9 ms, 0 of 10-11 frames
      over 16.7 ms (was one ≈ 31 ms frame). A key typed right after Ctrl+V lands after the paste
- [ ] Re-measure the paste on the reference Windows machine (before: one frame of 25-40 ms at
      4K @ 30 Hz, 25-27 ms at 1440p @ 59 Hz)
- [x] Save / Save As with atomic writes; unsaved-changes prompt on close and quit
- [x] Keyboard support in prompts on Linux (own in-window prompt; Windows and macOS keep native
      dialogs)

**Exit:** pasting 5 MB of LLM output produces no frame over 16.6 ms, with visible text in the same
frame; typing in a 1 MB document keeps key-to-present p99 within one 60 Hz frame.
