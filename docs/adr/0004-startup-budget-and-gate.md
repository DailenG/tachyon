# 0004: 50 ms startup budget and the Phase 1 gate

- **Status:** Accepted: resident mode with a ready window. It meets the budget on the reference
  Windows machine (p95 47.5 ms at 4K @ 30 Hz); direct launch cannot.
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
| Windows 11, 3840x2160 @ 30 Hz | reference: Core Ultra 7 155H, Intel Arc | 447 ms | 511 ms | platform init 288 ms, window open +111 ms, first frame +21 ms, ~27 ms before `main` |
| Windows 11, 2560x1440 @ 59 Hz | same machine | 280 ms | 324 ms | platform init 180 ms, window open +67 ms, first frame +12 ms |

Every measured platform misses the budget, the reference Windows machine by 6-10×. On Linux the
likely cause is GPU device creation through wgpu/Vulkan. On Windows, GPUI's platform
initialization alone (D3D11 device, DirectWrite, before the first window) takes 180-330 ms, three
to five times what it took on the GPU-less CI runner, and it is slower at 4K @ 30 Hz than at
1440p @ 59 Hz. Neither is profiled yet. Raw results:
[`docs/measurements/windows-reference-d28d304.md`](../measurements/windows-reference-d28d304.md).

Warm launches, handed to a running instance (`cargo xtask bench-startup --warm`, release, 20 runs;
measured from spawning the second process until the running instance has drawn the new window):

| Platform | Hardware | first | p50 | p95 |
|---|---|---|---|---|
| Linux, Wayland (Hyprland) | Intel UHD 750 | 180 ms | 27.6 ms | 29.5 ms |
| Windows 11, 3840x2160 @ 30 Hz | reference (above) | 109 ms | 145 ms | 188 ms |
| Windows 11, 2560x1440 @ 59 Hz | reference (above) | 107 ms | 127 ms | 150 ms |

On Windows the resident instance itself needs 86-93 ms (p50) from receiving a launch to the new
window's first frame, against 27 ms on Linux: opening a window in GPUI's Windows backend creates a
swap chain, render pipelines and a DirectComposition tree per window, and sizes its buffers to the
window.

The first launch into a windowless resident instance is cold (GPUI creates the GPU context with
the first window); after that the context outlives its windows, so every later launch is warm even
when no window is open. The second process itself (start, hand-off, acknowledgement) costs about
1 ms of that.

## Decision

Steps 1-3 below were the plan; the measurements under them led to the decision at the end of this
section.

1. Measure on reference Windows hardware, cold (first launch after boot) and warm.
2. If Windows direct launch meets p95 < 50 ms, keep direct launch and treat Linux as best effort
   until profiled.
3. Otherwise adopt **resident mode**: the primary instance stays alive after its last window closes
   and relaunches (through the single-instance channel, ADR 0003) open windows in it, skipping
   process, GPU and font start-up.

Status of the mechanism: resident mode is implemented as an opt-in flag, `tachyon --resident`
(without files it starts with no window, suitable for login autostart; Ctrl+Q quits for real). It
meets the budget on Linux.

Windows results (step 1): direct launch misses the budget by 6-10×, resident mode by 3-4×, so
step 2 is ruled out and step 3 alone does not close the gate. Next: attribute the Windows time with
a timing-instrumented GPUI build (each platform-init and window-open step, and the first frames)
on the reference machine, then decide which costs Tachyon can avoid (window options, deferred work,
a resident instance that keeps a window ready) and which need a GPUI change.

Windows results (step 2, traced GPUI, 4K @ 30 Hz, medians;
[`docs/measurements/windows-trace-2f51c45.md`](../measurements/windows-trace-2f51c45.md)):

| Cost | ms | Paid by |
|---|---|---|
| D3D11 device on the Intel Arc (the right adapter; the virtual display adapter is not involved) | 97 | every process |
| DirectWrite system font collection with `bCheckForUpdates = true` | 130 | every process |
| DXGI factory, OLE, drag-and-drop helper | 31 | every process |
| `CreateWindowEx`, including GPUI's renderer (≈ 13 ms of swap chain, pipelines, DirectComposition) | 39-59 | every window |
| `SetWindowPlacement`, which shows and activates the window | 59-87 | every window |
| First frame | 6 | every window |

- Checking for newly installed fonts is avoidable: with the check off, platform init drops from
  289 to 169 ms and launch p50 from 475 to 382 ms. It needs a one-line GPUI change. Not taken yet:
  it only helps direct launch, which cannot reach 50 ms anyway (device plus one window ≈ 215 ms),
  and a patched GPUI copy is a maintenance cost ADR 0001 avoids. Propose it upstream instead.
- Disabling DirectComposition changes nothing measurable.
- A warm launch is dominated by the window itself: receipt to first frame p50 202 ms here, 93 ms
  a week earlier on the same machine before Japanese and Chinese input methods were installed.
  Showing and activating the window is the largest and most variable step. [INFERENCE] Text
  Services Framework setup on focus is the likely addition; not yet measured.

So resident mode can only meet the budget if the per-window cost goes: next, trace inside
`SetWindowPlacement` (window messages, focus, input-method activation), and try a resident
instance that keeps a created, hidden window ready and only shows it on launch.

Windows results (step 3, traced GPUI, 4K @ 30 Hz, launches 500 ms apart;
[`docs/measurements/windows-trace-2-373f6cb.md`](../measurements/windows-trace-2-373f6cb.md)):

| Launch | spawn -> first frame p50 / p95 | receipt -> first_frame p50 / p95 |
|---|---|---|
| direct | 347 / 352 ms | |
| resident, window opened on launch | 118 / 128 ms | 102 / 108 ms |
| resident, hidden window kept ready | **41 / 47.5 ms** | 25 / 30 ms |

- Showing a window is mostly the render-target resize to its final size (`WM_SIZE` inside
  `WM_WINDOWPOSCHANGED`, ≈ 20 ms) and activation (`WM_ACTIVATE`, ≈ 15 ms, of which input-method
  setup is 0.6-3.4 ms). A ready window is drawn and appears as soon as it is shown, before
  activation finishes: its content is visible ≈ 27 ms after the launch arrives.
- The second process (start, hand-off, acknowledgement) costs ≈ 16 ms of the 41 ms.

**Decision:** the startup path is resident mode, and on Windows the resident instance keeps one
hidden window ready (created 100 ms after each launch's first frame, already sized, so the resize
does not happen while it is shown). Direct launch remains for a process that is not resident (the
first launch, `--new-instance`, a resident instance that is not running). Wayland maps windows
opened hidden, so Linux keeps no ready window; its resident launches already take ≈ 27 ms.
The ready window is enabled per platform (`tachyon_platform::keeps_hidden_windows_hidden`) where
measured; macOS is unmeasured.

## Consequences

- Resident mode means a background process and a tray or hotkey surface to design and document;
  it is still opt-in (`--resident`). Making it the default (autostart at login, a way to see and
  quit the process) is the next startup work.
- The margin is small (47.5 ms against 50) and assumes the ready window exists when a launch
  arrives; launches closer together than it takes to prepare one (≈ 20 ms plus 100 ms delay) open
  a window the ordinary way. Pre-sizing the ready window is expected to take ≈ 20 ms off, still to
  be measured on the reference machine.
- A ready window costs one window's memory and GPU buffers while idle.
- Every change on the startup path must include bench numbers (CONTRIBUTING).
- DirectWrite's font update check (≈ 130 ms per process) should still be proposed upstream: it is
  paid by every direct launch and every resident start.
