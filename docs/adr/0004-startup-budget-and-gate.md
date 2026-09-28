# 0004: 50 ms startup budget and the Phase 1 gate

- **Status:** Accepted: resident mode with a ready window. It meets the budget on the reference
  Windows machine (p95 29.0 ms at 4K @ 30 Hz with a pre-sized ready window, 22.8 ms with its DWM
  transitions off as shipped since run 4); direct launch cannot.
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

Status of the mechanism: resident mode was first an opt-in flag, `tachyon --resident` (without
files it started with no window, for login autostart; that is `--background` now). It meets the
budget on Linux.

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

Shipped build (run 3, pre-sized ready window, traced GPUI, 4K @ 30 Hz, launches 500 ms apart;
[`docs/measurements/windows-run-3-round-2-b866b36.md`](../measurements/windows-run-3-round-2-b866b36.md)):
spawn -> first frame p50 20.0 ms, p95 29.0 ms; receipt -> first_frame p50 3.4 ms, p95 5.7 ms (a
first round on the same build: p95 39.0 and 15.9 ms). The render-target resize (4-17 ms) now
happens when the hidden window is prepared, ≈ 0.3 s before it is shown. Showing it
(`SetWindowPlacement`) still takes 14-29 ms, but the first frame is drawn 2.6-4.5 ms into it; the
rest delays the next input, not the first frame. Direct launch: p95 392 ms.

## Consequences

- Resident mode is the default on Windows since 2026-09-27; `--no-resident` opts out. Linux and
  macOS keep it opt-in (`--resident`): a resident process started from a shell would keep the
  shell busy. `--autostart on` starts `tachyon --background` at login (Windows `Run` key, XDG
  autostart on Linux). The process is shown and ended from the command line (`--status`,
  `--quit`) and with Ctrl+Q; a tray icon is not built yet.
- The budget assumes the ready window exists when a launch arrives; launches closer together
  than it takes to prepare one (≈ 20 ms plus 100 ms delay) open a window the ordinary way
  (≈ 120 ms).
- Showing the window still costs 14-29 ms after its first frame. Next: apply the window's
  placement while it is hidden, so showing it does not also move it (a GPUI change; today GPUI
  defers the placement of a hidden window until it is activated), and try disabling DWM's window
  transitions for it (`DWMWA_TRANSITIONS_FORCEDISABLED`), which may also shorten the time until
  the window visibly appears. Measure time to input readiness (activation handled), not only the
  first frame.
- Run 4 ([`docs/measurements/windows-run-4-ad98ba5.md`](../measurements/windows-run-4-ad98ba5.md),
  same run, warm p95 spawn -> first frame / receipt -> first_frame): baseline 32.3 / 15.6 ms,
  DWM transitions off 22.8 / 5.4 ms, placed while hidden 27.8 / 12.1 ms, both 19.3 / 0.66 ms.
  Transitions are now off for ready windows (`tachyon_platform::disable_window_transitions`);
  placing the window while hidden needs a GPUI change, proposed upstream in
  [zed-industries/zed#64853](https://github.com/zed-industries/zed/discussions/64853). Showing the
  window still takes 14-26 ms after its first frame (activation); input is handled ≈ 19-24 ms after
  showing starts.
- A ready window costs one window's memory and GPU buffers while idle.
- Every change on the startup path must include bench numbers (CONTRIBUTING).
- DirectWrite's font update check (≈ 130 ms per process) should still be proposed upstream: it is
  paid by every direct launch and every resident start.
