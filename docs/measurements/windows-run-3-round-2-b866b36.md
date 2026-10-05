# Windows run 3 notes, round 2

Second run of the run 3 kit on the same commit, repeated on purpose. Round 1 results are on
`measure/windows-run-3` (commit 62fc0b7); this round is on `measure/windows-run-3-round-2`. Round
1's local files were moved to `C:\temp\run3-results-round1`.

Machine: reference Windows 11 (Core Ultra 7 155H, Intel Arc), 3840x2160 @ 30 Hz, physical console,
no RDP, AC power, heavy apps closed (confirmed by the user before the benchmarks).

## Changes on the machine since run 2

None (per the user), and none since round 1.

## Commit

`b866b36 exp: Windows run 3 kit (not for merge)`

## Builds

- Normal: `cargo build --release --locked -p tachyon` into `C:\temp\cargo-target`: exit 0,
  `Finished release profile [optimized] target(s) in 1.28s` (already built by round 1, same
  commit).
- Traced: `C:\temp\zed-trace` deleted and recreated from
  `C:\Users\dailen\.cargo\git\checkouts\zed-a70e2ad075855582\933d8d9`, robocopy exit 1,
  `gpui-trace.patch` applied (exit 0), build into `C:\temp\cargo-target-trace`: exit 0,
  `Finished release profile [optimized] target(s) in 1m 55s` (recompiled gpui_windows,
  gpui_platform, tachyon). `Cargo.lock` restored; `git status` clean afterward.

## Benchmark rows (traced build)

Script output:

```
direct-4k exit=1 trace_lines=359
warm-4k exit=0 trace_lines=310
```

direct-4k.txt:

```
metric                      first      p50      p95      max   (10 runs, ms)
spawn -> first frame      1061.79   336.08   391.64   391.64
main -> first_frame        442.04   320.45   369.19   369.19
main -> platform_ready     275.35   183.64   222.53   222.53
main -> window_open        424.95   295.66   343.92   343.92

FAIL: p95 spawn -> first frame 391.64 ms (budget 50 ms, runs 2-10)
```

warm-4k.txt:

```
metric                      first      p50      p95      max   (10 runs, ms)
spawn -> first frame        22.71    20.03    29.01    29.01
receipt -> first_frame       5.01     3.36     5.70     5.70

PASS: p95 spawn -> first frame 29.01 ms (budget 50 ms, runs 2-10)
```

The direct row has no `receipt -> first_frame` line (it is only printed for warm runs).

Round 1 for comparison: direct p95 445.78 ms (FAIL), warm p95 39.00 ms (PASS), warm
`receipt -> first_frame` p95 15.88 ms.

### Pre-sizing in the warm trace

As in round 1, the costly render target resize happens when the hidden window is pre-sized, not
when it is shown. Pre-size resizes (10 in the trace, in order): `renderer.resize 900x1000 took=`
3.88, 12.17, 11.13, 12.31, 13.59, 14.40, 16.56, 13.44, 14.31, 14.36 ms, inside `WM_SIZE` (depth=1)
of 4.88, 13.42, 12.16, 13.46, 15.13, 15.87, 18.07, 14.84, 16.15, 15.48 ms. Each lands about
312-345 ms before the `activate.set_placement` that shows that window. The last one prepares the
window for a launch after the trace ends.

Inside `activate.set_placement` the resize is `renderer.resize 900x1000 took=0.00` in every
activation.

`activate.set_placement` (9 activations, in order): 28.75, 16.09, 14.05, 15.51, 15.14, 15.86,
23.03, 19.34, 16.47 ms. The `WM_SIZE` (depth=0) inside each activation, same order: 1.54, 1.87,
1.66, 1.17, 1.30, 1.70, 1.18, 2.51, 1.15 ms. So about 12-27 ms of `set_placement` is outside
`WM_SIZE` (`WM_SHOWWINDOW` 0.86-13.34 ms, `WM_NCPAINT` 0.60-0.88 ms, and time not covered by
traced messages).

For comparison, the direct row's `window.set_placement` (window creation path, not activation)
takes 46.68-71.75 ms (round 1: 33.35-45.32 ms) and includes a
`renderer.resize took=3.52-6.52`.

## Resident check (normal build)

Script exit 0. resident-check.txt (written as UTF-8 directly this time):

```
resident-start-visible-windows expected=0 got=0 ok=True
after-launch-visible-windows expected=1 got=1 ok=True
after-launch-title-has-file expected=True got=True ok=True
after-close-visible-windows expected=0 got=0 ok=True
after-close-process-alive expected=True got=True ok=True
second-launch-visible-windows expected=1 got=1 ok=True
after-ctrl-q-process-exited expected=True got=True ok=True
```

All lines `ok=True`. `resident-after-launch.png`: same as round 1. Window titled
`resident-check.md - Tachyon` showing the line `# Resident check` with the caret at its start,
displayed as raw Markdown source (the `#` is visible, normal body size), not as a rendered heading.
With the caret in the only block, that block is the active raw block, which matches the block-swap
design.

## Paste test (normal build)

`gen-paste-doc.ps1` output matched the expected values exactly:

```
bytes=5243472 sections=7816 sha256=F9C26053EE214B68D61C3737E57223D8E7FBACDFA29873A800F9E36C46D777DA
```

All three `paste-test.ps1` runs exited 0. Unlike round 1, Tachyon received the keystrokes in every
run: all nine screenshots differ from one another, and each shows the pasted document.

| Run | `after-paste` overlay | View after paste | `ctrl-home` | `ctrl-end` |
|---|---|---|---|---|
| 1 (`-TypeAfter`) | `frame p50 1.3 ms · max 11.3 ms · 0/13 over 16.7 ms` | end of document, Section 7815, caret visible after `ZQX` | `## Section 0` (caret at start) | Section 7815, `ZQX` last line, caret visible |
| 2 | `frame p50 1.5 ms · max 9.6 ms · 0/9 over 16.7 ms` | end of document, Section 7815; last block (`[ref-7815]: ...`) active, caret not drawn in the screenshot | `## Section 0` (caret at start) | Section 7815, same as after-paste |
| 3 | `frame p50 2.5 ms · max 16.9 ms · 1/12 over 16.7 ms` (red border) | end of document, Section 7815; last block active, caret not drawn in the screenshot | `## Section 0` (caret at start) | Section 7815, same as after-paste |

Run 1: `ZQX` is the last line of the document.

Overlay in the later screenshots (it accumulates as the run continues):

- Run 1: ctrl-home `frame p50 1.3 ms · max 11.3 ms · 0/14 over 16.7 ms`, ctrl-end
  `frame p50 1.6 ms · max 11.3 ms · 0/15 over 16.7 ms`.
- Run 2: ctrl-home `frame p50 1.5 ms · max 9.6 ms · 0/10 over 16.7 ms`, ctrl-end
  `frame p50 2.1 ms · max 9.6 ms · 0/11 over 16.7 ms`.
- Run 3: ctrl-home `frame p50 2.8 ms · max 20.8 ms · 2/13 over 16.7 ms`, ctrl-end
  `frame p50 2.8 ms · max 20.8 ms · 2/14 over 16.7 ms` (red border on all three).

Run 3 is the only run with frames over 16.7 ms: one frame (16.9 ms) during the paste, a second one
(20.8 ms) by the Ctrl+Home screenshot.

Screenshot sha256:

```
5c6a96f0a597e0a86b762675c5891be2e7841b11d394cd0afac34fa788124507  paste-1-after-paste.png
e8e270c7c08eedfca3e417416aad36e75b128e13bf200e356111fbc3f3f90787  paste-1-ctrl-end.png
e20bfd231013b28d27cc10a1f9edeabb451ecfec6d3d1459d0a7431c9e3b33f0  paste-1-ctrl-home.png
47b47568317d955e7ca34283425342cf22f12aa24edc9e23d55518821317cbba  paste-2-after-paste.png
180ac16ca191255cc2541af8afa6218d57069fdfb50557d8696138e1d53a1773  paste-2-ctrl-end.png
e7ff02a861d3d0ef4f93a984c3a402bb520a4b939c7a1a1a2111f075eacca646  paste-2-ctrl-home.png
752b52f086581a4d8f3f7b18908b09efaf9b832032b642ff5dec14b8176ca67c  paste-3-after-paste.png
751848804d230387e462c49a663ba08c268b86400f1822ed0f45c93909ea2ef0  paste-3-ctrl-end.png
e53bfd2cefb9da1812dd61ab46ac88505b4f90ee8517c29588ff4dc7df43674a  paste-3-ctrl-home.png
```

## Other observations

- The paste test worked this round with the unchanged kit. The kit still discards the
  `AppActivate` result, so why focus reached Tachyon this time is not established. Probably the
  foreground was not held by another app when the runs started (round 1 had the agent's Chrome
  window in front), but this is an inference, not measured.
- Runs 2 and 3 show the last block active but no caret in the after-paste and ctrl-end
  screenshots, while run 1 (which typed `ZQX` last) shows the caret. Possibly the caret blink
  phase at capture time (inference, not checked).
- In the direct row, `window.set_placement` was 46.68-71.75 ms, longer than round 1's
  33.35-45.32 ms, but direct p95 spawn -> first frame was lower (391.64 vs 445.78 ms).
- A `Get-FileHash` call in the agent's own wrapper failed in Windows PowerShell
  (`The term 'Get-FileHash' is not recognized as the name of a cmdlet, function, script file, or
  operable program.`); hashes were taken with `sha256sum` instead. Not part of the kit.
- No `tachyon.exe` process was left running after the paste runs.
- The Windows clipboard holds the 5 MB test document (`paste5m.md`).

## Analysis (computed on Linux from `warm-4k.trace.txt`)

Round 2 is the valid round. Round 1 (`measure/windows-run-3`, 62fc0b7) sent every paste keystroke
to Chrome, because `paste-test.ps1` ignored the result of `AppActivate` and Windows' foreground
lock kept Chrome in front, so its paste results are void; its benchmark and resident-check results
are valid and slightly slower (warm p95 39.0 ms, direct p95 445.8 ms). The kit now checks the
foreground window before every batch of keys and fails loudly (`exp/windows-startup-trace`).

What happens inside `activate.set_placement` for the ready window (ms from the start of the call;
depth-0 messages; first four activations):

| Activation | `set_placement` | frame#0 drawn at | `WM_SHOWWINDOW` | `WM_NCPAINT` | `WM_SIZE` (resize 0.00) | untraced |
|---|---|---|---|---|---|---|
| 1 (first in the process) | 28.75 | +4.5 (took 3.72) | 13.34 | 0.72 | 1.54 at +25.0 | 13.15 |
| 2 | 16.09 | +3.2 | 1.08 | 0.62 | 1.87 at +12.1 | 12.52 |
| 3 | 14.05 | +2.7 | 0.99 | 0.60 | 1.66 at +10.5 | 10.80 |
| 4 | 15.51 | +2.7 | 1.25 | 0.69 | 1.17 at +12.3 | 12.40 |

- The first frame is drawn inside `WM_SHOWWINDOW`, 2.6-4.5 ms into the call; the rest of
  `set_placement` (10-25 ms) comes after it. It no longer delays the first frame
  (`receipt -> first_frame` p50 3.36 ms, p95 5.70 ms); it delays the next input and frames.
- `WM_SIZE` still arrives near the end although the size is unchanged: `SetWindowPlacement` also
  moves the window from where it was created (`CW_USEDEFAULT`) to its centered position.
- The untraced time is spent inside `SetWindowPlacement` between messages (messages under 0.3 ms
  are not printed; the rest is user32 and DWM work while showing the window).
- The first activation in a process is slower: its first frame takes 3.72 ms (0.05-0.07 ms later).

The caret was missing in paste runs 2 and 3 not because of blinking (Tachyon's caret does not
blink) but because a caret after a document's final newline had no position in the raw block's
layout; fixed separately.
