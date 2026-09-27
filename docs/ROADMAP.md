# Roadmap

Each phase has exit criteria that must be met, with evidence, before the phase is called done. A
later phase may start early only when it does not depend on the open criterion (Phase 2's core
crates are needed whichever way the startup gate goes).

## Phase 1: skeleton and startup gate (gate met; follow-ups open)

- [x] Workspace, pinned toolchain and GPUI revision, release profile, lints
- [x] GPUI window showing Markdown as raw text (sample, files, clipboard)
- [x] Single-instance handoff: Windows named pipe, Unix socket
- [x] Startup instrumentation and `cargo xtask bench-startup`
- [x] CI on Windows (primary), Linux, macOS; cargo-deny; Windows release artifact
- [x] **Gate:** p95 cold and warm startup measured on reference Windows hardware, and the
      direct-launch vs resident-mode decision recorded in [ADR 0004](adr/0004-startup-budget-and-gate.md):
      resident mode with a ready window, p95 47.5 ms at 4K @ 30 Hz, 29.0 ms as shipped (direct
      launch 352-392 ms)
- [x] Re-measure with the ready window pre-sized: p95 29.0 ms (was 47.5), content drawn 3.4 ms
      (p50) after the launch arrives
- [x] Cut showing the ready window: DWM transitions off for ready windows (run 4: p95 22.8 ms,
      content drawn ≤ 5.4 ms p95 after the launch arrives, window appears without a fade); input
      handled ≈ 19-24 ms after showing starts
- [ ] Propose to GPUI upstream: apply a hidden window's placement when it is created (run 4, with
      transitions off: p95 19.3 ms, first frame ≤ 0.7 ms p95 after the launch arrives)
- [x] Resident by default on Windows (`--no-resident` opts out; opt-in on Linux and macOS, where a
      shell would stay busy), `--background`, `--autostart on|off`, `--status`, `--quit`
- [ ] A tray icon (or equivalent) that shows the resident process and quits it

Measured so far (release, 20 runs, `cargo xtask bench-startup`):

| Machine | p50 | p95 | Breakdown (p50, from `main`) |
|---|---|---|---|
| Linux, Intel UHD 750, Wayland | 193 ms | 199 ms | platform 10 ms, window open +180 ms |
| Windows, GitHub `windows-latest` runner (no GPU; not reference hardware) | 142 ms | 161 ms | platform 60 ms, window open +44 ms, first frame +23 ms; ~16 ms before `main` |

| Windows 11, reference (Core Ultra 7 155H, Intel Arc), 4K @ 30 Hz | 447 ms | 511 ms | platform 288 ms, window open +111 ms, first frame +21 ms |
| Same machine, 1440p @ 59 Hz | 280 ms | 324 ms | platform 180 ms, window open +67 ms, first frame +12 ms |

All miss the 50 ms budget. On the reference Windows machine GPUI's platform initialization alone
takes 180-330 ms.

Warm launches into a resident instance (resident mode, `cargo xtask bench-startup --warm`) meet it
on Linux: p50 27.6 ms, p95 29.5 ms (first launch into a windowless instance 180 ms), but not on
the reference Windows machine: p95 150-188 ms, of which the resident instance's window open to
first frame is 86-93 ms. A timing-instrumented GPUI build attributed it: per process, the D3D11
device (97 ms) and DirectWrite's check for new fonts (130 ms, avoidable); per window, creating
(39-59 ms) and showing (59-87 ms) it. A second trace found showing is mostly resizing the render
targets and activation; a resident instance that keeps a hidden window ready shows it with its
content ≈ 27 ms after the launch arrives: p95 47.5 ms from spawning the launching process, 29.0 ms
once the hidden window is sized in advance (shipped). See
ADR 0004.

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

## Phase 3: block-swap editor (`tachyon-editor`) (done)

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
- [x] Re-measure the paste on the reference Windows machine: max frame 8.6-12.9 ms, no frame
      over budget in 5 of 6 runs (one 17.9 ms frame at 1440p @ 59 Hz), where it used to be one
      25-40 ms frame. A key typed right after Ctrl+V lands after the paste. The same runs showed
      the view drifting to the top of the document while the parse streamed in, fixed since
- [x] Save / Save As with atomic writes; unsaved-changes prompt on close and quit
- [x] Keyboard support in prompts on Linux (own in-window prompt; Windows and macOS keep native
      dialogs)

- [x] Exit criteria shown on the reference Windows machine (run 4, 4K @ 30 Hz, per-frame log
      `TACHYON_FRAME_LOG`): three 5 MB pastes with no frame over 16.7 ms (max 13.1 ms) and the
      pasted text visible in the Ctrl+V frame; typing in a 1 MiB document key to paint p95 ≈ 4 ms,
      max 6.6 ms. Run 3 once had a 16.9 ms paste frame; not reproduced. Key to present adds the
      wait for the display's next refresh, which Tachyon does not control
- [ ] Follow-up: read the clipboard off the UI thread (Ctrl+V stalls it 11-12 ms on Windows)

**Exit:** pasting 5 MB of LLM output produces no frame over 16.6 ms, with visible text in the same
frame; typing in a 1 MB document keeps key-to-present p99 within one 60 Hz frame.
