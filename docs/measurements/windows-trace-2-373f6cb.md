# Traced Windows per-window runs (run 2)

## Machine

Same machine as run 1 (`docs/measurements/windows-trace-2f51c45.md`). User reports nothing changed since run 1: no new software, no input method changes, no restarts. User confirmed: physical console, no RDP connected, AC power, heavy apps closed, display at 3840x2160 @ 30 Hz. All rows ran at 3840x2160 @ 30 Hz; the display mode was not changed.

- Commit: `373f6cb exp: trace window messages; ready-window experiment; run 2 handoff`
- GPUI source: `C:\Users\dailen\.cargo\git\checkouts\zed-a70e2ad075855582\933d8d9`. The old `C:\temp\zed-trace` was deleted, a fresh copy made with robocopy (exit 1), and `gpui-trace.patch` applied cleanly without `--ignore-whitespace`.
- Traced build: `cargo --config C:/temp/zed-trace/patch.toml build --release -p tachyon`, finished in 2m 24s. `Cargo.lock` restored with `git checkout -- Cargo.lock`; `git status` clean afterwards.
- Sanity check (`--startup-report`, exit 0): `sanity.txt` has 7 `gpui-trace msg` lines and 1 `gpui-trace frame#0` line. Stdout: `tachyon-startup platform_ready_us=213931 window_open_us=320349 first_frame_us=340748`.

## Script output lines

```
direct-4k exit=1 trace_lines=358
warm-4k exit=1 trace_lines=311
warm-4k-ready exit=0 trace_lines=287
```

## First-frame lines (columns: first, p50, p95, max; 10 runs, ms)

```
direct-4k      spawn -> first frame       342.85   347.18   352.09   352.09
direct-4k      FAIL: p95 spawn -> first frame 352.09 ms (budget 50 ms, runs 2-10)
warm-4k        spawn -> first frame       134.41   118.33   127.86   127.86
warm-4k        receipt -> first_frame     113.21   101.57   107.97   107.97
warm-4k        FAIL: p95 spawn -> first frame 127.86 ms (budget 50 ms, runs 2-10)
warm-4k-ready  spawn -> first frame        45.25    41.32    47.51    47.51
warm-4k-ready  receipt -> first_frame      22.63    25.39    30.44    30.44
warm-4k-ready  PASS: p95 spawn -> first frame 47.51 ms (budget 50 ms, runs 2-10)
```

direct-4k has no `receipt -> first_frame` line (direct launches have no resident instance).

## `tachyon-exp ready_window_open_us` lines in `.trace.txt`

| Row | Lines |
|---|---|
| direct-4k | 0 |
| warm-4k | 0 |
| warm-4k-ready | 10 |

As expected: only `warm-4k-ready` has them.

## Span summary (took, ms; computed from the `.trace.txt` files)

| Row | Span | n | min | median | max |
|---|---|---|---|---|---|
| warm-4k | activate.set_placement | 10 | 0.00 | 0.00 | 0.00 |
| warm-4k | activate.set_active_window | 10 | 0.01 | 0.01 | 0.05 |
| warm-4k | activate.set_focus | 10 | 0.00 | 0.00 | 0.01 |
| warm-4k | window.create_window_ex | 10 | 14.75 | 18.10 | 22.59 |
| warm-4k | frame#0 | 9 | 0.14 | 0.50 | 1.95 |
| warm-4k-ready | activate.set_placement | 9 | 47.29 | 60.31 | 79.51 |
| warm-4k-ready | activate.set_active_window | 9 | 0.01 | 0.01 | 0.01 |
| warm-4k-ready | activate.set_focus | 9 | 0.00 | 0.00 | 0.05 |
| warm-4k-ready | window.create_window_ex | 10 | 16.17 | 17.03 | 22.42 |
| warm-4k-ready | frame#0 | 10 | 0.06 | 0.07 | 3.79 |

## Unexpected

- `warm-4k-ready` passes the budget (p95 47.51 ms, exit 0). It is the only row to do so.
- In `warm-4k`, `activate.set_placement` took 0.00 ms in all 10 runs. Run 1 reported 59-87 ms for showing and activating the window. In `warm-4k-ready` the same span takes 47-80 ms, yet receipt -> first_frame is only 22-30 ms. The traces don't show whether that placement happens before the launch (while preparing the hidden window) or after the first frame. See the `at=` values in `warm-4k-ready.trace.txt`.
- Counts differ by one: `warm-4k` has 9 `frame#0` lines against 10 windows, and `warm-4k-ready` has 9 `activate.*` lines of each kind against 10 `window.create_window_ex` lines.
- The results are faster than run 1 on every row (direct-4k p95 352.09 ms vs 643.56 ms; warm-4k p95 127.86 ms vs 388.93 ms). The machine was not restarted in between, per the user. The only differences on this side are the new branch commit and a warm benchmark that waits 500 ms between launches.
- No errors occurred.

## Analysis (computed on Linux from the `.trace.txt` files)

Where a warm launch's window time goes (medians per window, ms; depth-0 window messages):

| | warm-4k (window opened on launch) | warm-4k-ready (hidden window shown on launch) |
|---|---|---|
| `window.new` on the launch path | 79.9 (create 18.3, `SetWindowPlacement` 52.6) | none (21.3, done in advance) |
| `activate.set_placement` on the launch path | 0.0 | 60.3 |
| `WM_WINDOWPOSCHANGED` (contains the render-target resize in `WM_SIZE`) | 17.6 | 20.0 |
| `WM_ACTIVATE` (contains `WM_IME_SETCONTEXT`, 0.6-3.4 ms) | 15.2 | 16.0 |
| receipt -> first_frame p50 / p95 | 101.6 / 108.0 | 25.4 / 30.4 |
| spawn -> first frame p50 / p95 | 118.3 / 127.9 | 41.3 / 47.5 |

Order of events while the ready window is shown (second launch; `at` in ms since the resident
process started):

```
1283.6  activate.set_placement begins (takes 55.9 ms)
1284.0    WM_WINDOWPOSCHANGED 24.8 ms, of which WM_SIZE 22.5 ms (render targets resized)
1310.8    WM_SHOWWINDOW 0.7 ms
1310.9    frame#0 drawn (0.07 ms): the window appears with its content
1317.2    WM_NCACTIVATE 2.1 ms
1321.9    WM_ACTIVATE 11.0 ms, of which WM_IME_SETCONTEXT 3.4 ms
1336.0    WM_SIZE 1.1 ms
```

- The first frame is drawn as the window is shown, about 27 ms after the launch arrives, before
  activation finishes; `receipt -> first_frame` is an honest measure of when the content appears.
  (As in every mode, it marks the start of the frame that draws the window, not the end of its
  presentation.)
- Input-method activation costs 0.6-3.4 ms; it is not the per-window cost.
- The ready window's final size is only applied when it is shown, so its render targets are
  resized then (≈ 20 ms). Resizing the hidden window in advance should remove that from the launch.
- Run 1 and run 2 differ by up to 2x on the same machine (warm p95 389 ms vs 128 ms, direct p95
  644 ms vs 352 ms) with no reported change in between; compare rows within one run.
