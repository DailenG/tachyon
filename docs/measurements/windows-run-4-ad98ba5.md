# Windows run 4 notes

Machine: reference Windows 11 (Core Ultra 7 155H, Intel Arc), 3840x2160 @ 30 Hz, physical console,
no RDP, AC power, heavy apps closed (confirmed by the user before the benchmarks).

## Changes on the machine since run 3

None (per the user).

## Commit

`ad98ba5 exp: Windows run 4 kit (frame log, typing test, set_placement experiments; not for merge)`

`git fetch origin --prune` also reported `origin/measure/windows-run-3` and
`origin/measure/windows-run-3-round-2` as deleted on the remote; `main` fast-forwarded
183bd54..7c9ce46.

## Builds

- Normal: `cargo build --release --locked -p tachyon` into `C:\temp\cargo-target`: exit 0,
  `Finished release profile [optimized] target(s) in 2m 38s`.
- Traced: `C:\temp\zed-trace` deleted and recreated from
  `C:\Users\dailen\.cargo\git\checkouts\zed-a70e2ad075855582\933d8d9`, robocopy exit 1,
  `gpui-trace.patch` applied (exit 0), build into `C:\temp\cargo-target-trace`: exit 0,
  `Finished release profile [optimized] target(s) in 2m 45s`. `Cargo.lock` restored; `git status`
  clean afterwards.

## Warm benchmark rows (traced build)

Script output:

```
warm-4k exit=0 trace_lines=315
warm-4k-place-hidden exit=0 trace_lines=316
warm-4k-no-transitions exit=0 trace_lines=302
warm-4k-both exit=0 trace_lines=315
```

Checks: `window.place_hidden` lines: warm-4k 0, place-hidden 10, no-transitions 0, both 10.
`tachyon-exp no_transitions applied=true` lines: warm-4k 0, place-hidden 0, no-transitions 10,
both 10. So each switch took effect only in its rows.

| Row | `spawn -> first frame` (first / p50 / p95 / max) | `receipt -> first_frame` (first / p50 / p95 / max) | Result |
|---|---|---|---|
| warm-4k | 30.65 / 23.95 / 32.29 / 32.29 | 11.50 / 4.65 / 15.64 / 15.64 | PASS: p95 spawn -> first frame 32.29 ms (budget 50 ms, runs 2-10) |
| warm-4k-place-hidden | 25.45 / 17.95 / 27.83 / 27.83 | 5.04 / 0.73 / 12.13 / 12.13 | PASS: p95 spawn -> first frame 27.83 ms (budget 50 ms, runs 2-10) |
| warm-4k-no-transitions | 27.96 / 21.97 / 22.83 / 22.83 | 5.05 / 3.33 / 5.39 / 5.39 | PASS: p95 spawn -> first frame 22.83 ms (budget 50 ms, runs 2-10) |
| warm-4k-both | 24.51 / 16.74 / 19.32 / 19.32 | 3.98 / 0.56 / 0.66 / 0.66 | PASS: p95 spawn -> first frame 19.32 ms (budget 50 ms, runs 2-10) |

Lines as printed:

```
warm-4k
spawn -> first frame        30.65    23.95    32.29    32.29
receipt -> first_frame      11.50     4.65    15.64    15.64
warm-4k-place-hidden
spawn -> first frame        25.45    17.95    27.83    27.83
receipt -> first_frame       5.04     0.73    12.13    12.13
warm-4k-no-transitions
spawn -> first frame        27.96    21.97    22.83    22.83
receipt -> first_frame       5.05     3.33     5.39     5.39
warm-4k-both
spawn -> first frame        24.51    16.74    19.32    19.32
receipt -> first_frame       3.98     0.56     0.66     0.66
```

`activate.set_placement` took (9 activations per row, in order):

- warm-4k: 22.78, 20.91, 22.01, 25.96, 22.61, 17.47, 19.39, 19.89, 24.12
- warm-4k-place-hidden: 18.28, 24.11, 16.14, 24.07, 15.47, 15.65, 13.86, 22.90, 17.30
- warm-4k-no-transitions: 23.14, 15.56, 16.94, 15.00, 23.02, 16.79, 25.05, 15.46, 20.45
- warm-4k-both: 17.10, 16.60, 21.89, 14.66, 20.99, 14.29, 21.45, 23.13, 15.18

`window.place_hidden` took (10 per row; this happens while the hidden window is prepared, before
the launch that shows it):

- warm-4k-place-hidden: 12.32, 18.40, 17.32, 21.95, 19.24, 22.59, 22.49, 17.32, 21.20, 20.02
- warm-4k-both: 11.38, 20.48, 20.42, 19.03, 20.42, 22.19, 19.75, 21.04, 21.72, 21.66

So `set_placement` stays at 14-26 ms in every row, although p95 launch time and
`receipt -> first_frame` drop with the switches. Every trace ends in a truncated line
(`gpui-trace msg WM_SHOWWINDOW 0x0018 depth=0 at=` in warm-4k), presumably because the process was
stopped mid-write at the end of the row (inference).

## Typing latency (normal build)

```
bytes=1048848 sections=1574 sha256=68A32CD3970D5B54B25410439B04E7F520CA71C877775994C089E34BA90BEA18
foreground-ok step=typing-end pid=2948
foreground-ok step=typing-start pid=2948
```

`typing-test.ps1` exit 0. (The per-20-characters foreground checks are piped to `Out-Null` in the
kit, so only the two batch checks print; none failed.)

From `typing-frames.log` (agent summary; percentiles by nearest rank):

| Section (between markers) | frames | keys | key_to_paint_ms p50 / p95 / max | max busy_ms | frames busy > 16.7 ms |
|---|---|---|---|---|---|
| typing-end (120 chars at end of document) | 121 | 120 | 2.95 / 4.01 / 6.55 | 2.93 | 0 |
| typing-start (120 chars at start) | 120 | 120 | 2.77 / 4.16 / 6.05 | 3.38 | 0 |

The document had `blocks=11018` when loaded and `blocks=11019` once typing started. The Ctrl+End
before the first batch (key_to_paint 2.78 ms) and the Ctrl+Home between batches (2.97 ms) are
outside these sections.

## Paste with frame logs (normal build)

```
bytes=5243472 sections=7816 sha256=F9C26053EE214B68D61C3737E57223D8E7FBACDFA29873A800F9E36C46D777DA
foreground-ok step=overlay pid=43924
foreground-ok step=paste pid=43924
foreground-ok step=ctrl-home pid=43924
foreground-ok step=ctrl-end pid=43924
paste-1 exit=0
foreground-ok step=overlay pid=32608
foreground-ok step=paste pid=32608
foreground-ok step=ctrl-home pid=32608
foreground-ok step=ctrl-end pid=32608
paste-2 exit=0
foreground-ok step=overlay pid=41860
foreground-ok step=paste pid=41860
foreground-ok step=ctrl-home pid=41860
foreground-ok step=ctrl-end pid=41860
paste-3 exit=0
```

No foreground check failed.

| Run | `after-paste` overlay | View after paste | `ctrl-home` | `ctrl-end` |
|---|---|---|---|---|
| 1 (`-TypeAfter`) | `frame p50 1.3 ms · max 13.1 ms · 0/13 over 16.7 ms` | end of document, Section 7815, caret visible after `ZQX` | `## Section 0`, caret at start; overlay `frame p50 1.2 ms · max 13.1 ms · 0/14 over 16.7 ms` | Section 7815, `ZQX` last line, caret visible; overlay `frame p50 1.3 ms · max 13.1 ms · 0/15 over 16.7 ms` |
| 2 | `frame p50 1.4 ms · max 8.3 ms · 0/10 over 16.7 ms` | end of document, Section 7815, caret visible on the empty last line | `## Section 0`, caret at start; overlay `frame p50 1.4 ms · max 8.3 ms · 0/11 over 16.7 ms` | Section 7815, caret visible on the empty last line; overlay `frame p50 1.4 ms · max 8.3 ms · 0/12 over 16.7 ms` |
| 3 | `frame p50 1.3 ms · max 11.5 ms · 0/10 over 16.7 ms` | end of document, Section 7815, caret visible on the empty last line | `## Section 0`, caret at start; overlay `frame p50 1.3 ms · max 11.5 ms · 0/11 over 16.7 ms` | Section 7815, caret visible on the empty last line; overlay `frame p50 1.3 ms · max 11.5 ms · 0/12 over 16.7 ms` |

Run 1: `ZQX` is the last line. No frame over 16.7 ms in any run or screenshot (run 3 round 2, paste run 3,
had 16.9 and 20.8 ms). The caret is now visible at the end of the document in runs 2 and 3
(in run 3 round 2 it was not drawn there).

From the frame logs (between `marker paste` and `marker ctrl-home`):

| Run | frames | max busy_ms | Ctrl+V frame |
|---|---|---|---|
| 1 | 13 | 13.09 | `frame n=5 at_ms=3058.8 busy_ms=13.09 render_ms=2.89 work=clipboard:11.05,edit:8.90,edit:0.03 keys=2 key_to_paint_ms=25.34,13.09 drawn=3 blocks=685` |
| 2 | 10 | 8.28 | `frame n=5 at_ms=3074.1 busy_ms=3.04 render_ms=3.04 work=clipboard:12.22,paste:0.33 keys=1 key_to_paint_ms=24.46 drawn=3 blocks=685` |
| 3 | 10 | 11.51 | `frame n=5 at_ms=3098.1 busy_ms=2.37 render_ms=2.37 work=clipboard:11.36,paste:0.60 keys=1 key_to_paint_ms=26.25 drawn=3 blocks=685` |

- The Ctrl+V key takes 24.46-26.25 ms from arrival to paint; the clipboard read is 11.05-12.22 ms
  of that. In runs 2 and 3 `busy_ms` (3.04, 2.37) is well below the `clipboard` work in the same
  line, so the clipboard read seems not to be counted in `busy_ms` there (inference from the
  numbers; in run 1 `busy_ms` 13.09 is also below clipboard + edit).
- Largest non-paste frames: run 2 `frame n=6 ... busy_ms=8.28 render_ms=8.28 work=parse:...
  drawn=26 blocks=7966`, run 3 `frame n=6 ... busy_ms=11.51 render_ms=10.66 work=parse:...
  drawn=26 blocks=9416`: the first render after the paste while the background parse is still
  delivering blocks.
- Ctrl+Home key_to_paint: 4.81, 5.10, 3.86 ms; Ctrl+End: 5.55, 2.56, 2.43 ms (runs 1-3).
- In run 1, `ZQX` lands 13.09 ms (Z, same frame as Ctrl+V), 0.77 and 0.29 ms after arrival.

## Resident check (normal build)

Script exit 0. resident-check.txt:

```
resident-start-visible-windows expected=0 got=0 ok=True
after-launch-visible-windows expected=1 got=1 ok=True
after-launch-title-has-file expected=True got=True ok=True
after-close-visible-windows expected=0 got=0 ok=True
after-close-process-alive expected=True got=True ok=True
second-launch-visible-windows expected=1 got=1 ok=True
foreground-ok step=ctrl-q pid=38752
after-ctrl-q-process-exited expected=True got=True ok=True
```

All result lines `ok=True`. `resident-after-launch.png`: window `resident-check.md - Tachyon`,
`# Resident check` as the active raw block with the caret at its start; the block now shows an
empty second line below the heading line.

## Other observations

- No `FOREGROUND CHECK FAILED` in any script; no reruns were needed.
- No `tachyon.exe` process was left running after the scripts.
- The Windows clipboard holds the 5 MB test document (`paste5m.md`).

## Analysis (computed on Linux from the traces and frame logs)

Where the first frame and input readiness fall inside `activate.set_placement` (medians over 9
activations, ms from the start of the call; max in parentheses):

| Row | first frame | activation handled (`activate.set_focus` done) |
|---|---|---|
| warm-4k | +5.1 (15.3) | +23.9 (29.1) |
| warm-4k-place-hidden | +0.5 (12.0) | +19.8 (26.2) |
| warm-4k-no-transitions | +3.8 (5.3) | +18.8 (27.6) |
| warm-4k-both | +0.4 (3.8) | +19.6 (25.1) |

- Both switches move the first frame earlier and remove the tail: receipt -> first_frame p95
  15.6 ms (baseline) becomes 5.4 ms without transitions and 0.66 ms with both; spawn -> first frame
  p95 32.3 ms becomes 22.8 and 19.3 ms. `set_placement` itself stays 14-26 ms: the time after the
  first frame is activation, which neither switch changes. Input is handled ≈ 19-24 ms after the
  window starts showing, ≈ 35-40 ms after the launching process was spawned.
- Without transitions the window also appears at once instead of fading in; the fade is not in any
  of these numbers.
- Placing the hidden window in advance (`window.place_hidden`, 11-23 ms) moves that cost to when the
  window is prepared. It needs a GPUI change; disabling transitions does not.

Phase 3 exit criterion on Windows (normal build, 4K @ 30 Hz):

- Paste: no frame over 16.7 ms in three runs (max 13.1, 8.3, 11.5 ms); the pasted text is visible
  in the Ctrl+V frame (`drawn=3`). The slowest frames are the first renders after the paste while
  parse chunks arrive (8-12 ms). Ctrl+Home / Ctrl+End to the other end of the 5 MB document:
  2.4-5.6 ms from key to paint (run 3's 20.8 ms is not reproduced).
- Typing, 1 MiB document, 120 keys each at the end and at the start: key to paint p50 2.8-3.0 ms,
  p95 4.0-4.2 ms, max 6.6 ms; no frame busier than 3.4 ms. This is key arrival to the end of the
  frame's paint; presentation then waits for the display's next refresh (at most 33 ms at 30 Hz),
  which Tachyon does not control.
- Ctrl+V itself: key to paint 24-26 ms, of which the clipboard read inside GPUI is 11-12 ms on the
  UI thread, followed by an idle gap while the paste is prepared off the thread. It is not a
  frame (nothing is drawn in between), but it is a UI-thread stall worth moving off the thread.
