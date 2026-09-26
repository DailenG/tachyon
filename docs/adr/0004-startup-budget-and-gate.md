# 0004: 50 ms startup budget and the Phase 1 gate

- **Status:** Proposed. Waiting for Windows measurements.
- **Date:** 2026-09-25

## Context

Tachyon is meant to be an instant scratchpad: launch to first frame must be p95 < 50 ms. Most of
that time is outside our code (process creation, GPU device creation, font system setup), so the
budget has to be checked against real hardware before editor work builds on it.

Measured with `cargo xtask bench-startup` (release build, 20 runs):

| Platform | Hardware | p50 | p95 | Where the time goes |
|---|---|---|---|---|
| Linux, Wayland (Hyprland) | Intel UHD 750 | 193 ms | 199 ms | ~180 ms inside GPUI `open_window`; platform init ~10 ms |
| Windows | GitHub `windows-latest` runner, no GPU (informational) | 142 ms | 161 ms | platform init ~60 ms, window open ~44 ms, first frame ~23 ms, ~16 ms before `main` |
| Windows | reference machine | not measured | not measured | |

Both measured platforms miss the budget by 3–4×. On Linux the likely cause is GPU device creation
through wgpu/Vulkan; on the Windows runner GPUI's platform initialization alone takes longer than
the whole budget. Neither is profiled yet.

Warm launches, handed to a running instance (`cargo xtask bench-startup --warm`, release, 20 runs;
measured from spawning the second process until the running instance has drawn the new window):

| Platform | Hardware | first | p50 | p95 |
|---|---|---|---|---|
| Linux, Wayland (Hyprland) | Intel UHD 750 | 180 ms | 27.6 ms | 29.5 ms |

The first launch into a windowless resident instance is cold (GPUI creates the GPU context with
the first window); after that the context outlives its windows, so every later launch is warm even
when no window is open. The second process itself (start, hand-off, acknowledgement) costs about
1 ms of that.

## Decision (proposed)

1. Measure on reference Windows hardware, cold (first launch after boot) and warm.
2. If Windows direct launch meets p95 < 50 ms, keep direct launch and treat Linux as best effort
   until profiled.
3. Otherwise adopt **resident mode**: the primary instance stays alive after its last window closes
   and relaunches (through the single-instance channel, ADR 0003) open windows in it, skipping
   process, GPU and font start-up.

Status of the mechanism: resident mode is implemented as an opt-in flag, `tachyon --resident`
(without files it starts with no window, suitable for login autostart; Ctrl+Q quits for real). It
meets the budget on Linux. Making it the default, and adding autostart and a way to show the
process is running, waits for the Windows numbers.

## Consequences

- Resident mode means a background process and a tray or hotkey surface to design and document.
- Every change on the startup path must include bench numbers (CONTRIBUTING).
- Accept this ADR, with the chosen option and the Windows numbers, to close the Phase 1 gate.
