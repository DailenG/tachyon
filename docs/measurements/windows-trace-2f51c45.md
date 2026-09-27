# Traced Windows startup runs and paste re-measure

## Machine

Same as `docs/measurements/windows-reference-d28d304.md` (Core Ultra 7 155H, Intel Arc driver 32.0.101.9026, 31.5 GB, Windows 11 Pro Insider 26H2 build 26340.9502, Balanced plan). Changes since then: the machine was restarted (language packs); Japanese and Chinese input methods are now installed. User confirmed: physical console, no RDP, AC power, heavy apps closed.

- Commit: `2f51c45 exp: re-measure the paste in the same Windows session`
- Toolchain: 1.98.1-x86_64-pc-windows-msvc
- GPUI source: `C:\Users\dailen\.cargo\git\checkouts\zed-a70e2ad075855582\933d8d9`, copied with robocopy (exit 1), `gpui-trace.patch` applied cleanly with no `--ignore-whitespace` needed.
- Traced build: `cargo --config C:/temp/zed-trace/patch.toml build --release -p tachyon`, finished in 5m 20s. `Cargo.lock` restored with `git checkout -- Cargo.lock`; `git status` clean afterwards.
- Normal build: `cargo build --release --locked -p tachyon` (target `C:\temp\cargo-target`), finished in 6m 38s; `git status` clean.

## Sanity check (traced binary, `--startup-report`)

`sanity.txt` has 1 `gpui-trace platform.new` line and 1 `gpui-trace frame#0` line. Stdout: `tachyon-startup platform_ready_us=372951 window_open_us=562735 first_frame_us=607322`.

## Display mode per run

| Run | Display |
|---|---|
| direct-4k, direct-4k-font-no-update, direct-4k-no-dcomp, warm-4k, warm-4k-no-dcomp | 3840x2160 @ 30 Hz |
| paste runs 1, 2, 3 | 3840x2160 @ 30 Hz |
| direct-1440p, warm-1440p | 2560x1440 @ 59 Hz (set by the agent with ChangeDisplaySettingsEx, not saved to the registry) |
| paste runs 4, 5, 6 | 2560x1440 @ 59 Hz |

Display restored to 3840x2160 @ 30 Hz afterwards (`now: 3840x2160 @ 30 Hz`).

## Script output lines

```
direct-4k exit=1 trace_lines=280
direct-4k-font-no-update exit=1 trace_lines=280
direct-4k-no-dcomp exit=1 trace_lines=270
warm-4k exit=1 trace_lines=172
warm-4k-no-dcomp exit=1 trace_lines=161
direct-1440p exit=1 trace_lines=280
warm-1440p exit=1 trace_lines=171
```

Generator: `bytes=5243472 sections=7816 sha256=F9C26053EE214B68D61C3737E57223D8E7FBACDFA29873A800F9E36C46D777DA` (matches).

## Trace step summary (took, ms, 10 runs each; computed from the `.trace.txt` files)

| Row | platform.new med (min-max) | dwrite.system_font_collection med | dx.create_device med | window.new med | frame#0 took med |
|---|---|---|---|---|---|
| direct-4k | 283.03 (263.86-365.71) | 129.54 | 96.24 | 115.34 | 6.06 |
| direct-4k-font-no-update | 167.21 (150.74-190.64) | 1.50 | 100.21 | 129.80 | 6.93 |
| direct-4k-no-dcomp | 283.38 (258.36-304.92) | 126.91 | 97.98 | 112.84 | 6.41 |
| direct-1440p | 385.23 (363.91-462.06) | 168.77 | 127.76 | 183.23 | 9.80 |

## Paste test (normal build)

| Run | Display | Overlay after paste | Formatted Markdown | After-paste view | ZQX last line | ctrl-home `## Section 0` | ctrl-end Section 7815 |
|---|---|---|---|---|---|---|---|
| 1 (`-TypeAfter`) | 4K @ 30 Hz | frame p50 1.3 ms · max 12.0 ms · 0/15 over 16.7 ms | yes | end of document | yes, `ZQX` is the last line, after `[ref-7815]`, caret after it | yes | yes (ZQX still last) |
| 2 | 4K @ 30 Hz | frame p50 5.4 ms · max 11.2 ms · 0/14 over 16.7 ms | yes | top of document (Section 0) | n/a | yes | yes |
| 3 | 4K @ 30 Hz | frame p50 4.9 ms · max 12.9 ms · 0/12 over 16.7 ms | yes | top of document (Section 0) | n/a | yes | yes |
| 4 | 1440p @ 59 Hz | frame p50 1.1 ms · max 9.5 ms · 0/22 over 16.7 ms | yes | end of document | n/a | yes | yes |
| 5 | 1440p @ 59 Hz | frame p50 6.4 ms · max 17.9 ms · 1/32 over 16.7 ms (red border) | yes | top of document (Section 0) | n/a | yes | yes |
| 6 | 1440p @ 59 Hz | frame p50 1.2 ms · max 8.6 ms · 0/22 over 16.7 ms | yes | end of document | n/a | yes | yes |

## Unexpected

- Runs 2, 3 and 5: the after-paste screenshot (4 s after Ctrl+V) shows the top of the document with `Section 0` rendered as a heading, not the end; runs 1, 4 and 6 show the end. Those same three runs have a higher frame p50 (4.9-6.4 ms vs 1.1-1.3 ms), and run 5 is the only run with a frame over budget (max 17.9 ms). Ctrl+Home and Ctrl+End worked normally in all six.
- Before the valid runs 1-3, one invocation of the agent's own wrapper passed `-Runs 1,2,3` through `powershell -File` as a single value, which ran the paste once as "run 123" without `-TypeAfter`. Its screenshots were deleted and it is not reported; runs 1-3 were then run correctly.
- direct-1440p (p95 811.78 ms) is slower than direct-4k (p95 643.56 ms), the opposite of the previous measurement (511 vs 324 ms). All traced steps are uniformly slower at 1440p (platform.new, font collection, D3D11 device, window.new), not only frame-related ones.
- Warm rows are much slower than the previous untraced measurement (warm-4k p95 388.93 ms vs 188.19 ms before).
- The 4K startup rows ran after the traced build and before the normal build; the 1440p rows ran immediately after the display mode change.
- Clipboard now holds the 5 MB test document.

## Step breakdown (computed on Linux from the `.trace.txt` files)

Medians over 10 runs, ms. `took` is time inside the step; per-window steps repeat for every
window a warm (resident) instance opens.

| Step | direct-4k | direct-4k-font-no-update | warm-4k (per window) |
|---|---|---|---|
| `platform.ole_initialize` | 3.5 | 3.8 | |
| `dx.create_factory` | 13.3 | 14.0 | |
| `dx.create_device` (adapter 0, "Intel(R) Arc(TM) Graphics", flags 0x0, every run) | 97.5 | 101.7 | |
| `dwrite.system_font_collection` | 129.7 (check_for_updates=true) | 1.5 (false) | |
| `platform.direct_write` (total) | 138.9 | 10.3 | |
| `platform.drag_drop_helper` | 14.2 | 15.4 | |
| **`platform.new` (total)** | **289.1** | **168.8** | |
| `window.create_window_ex` (includes `window.renderer`) | 38.8 | 43.6 | 58.8 |
| `window.renderer` (resources 5.0, pipelines 5.9, DirectComposition 2.5) | 28.4 | 31.5 | 34.5 |
| `window.set_placement` (shows and activates the window) | 58.6 | 65.9 | 86.7 |
| **`window.new` (total)** | **117.2** | **129.9** | **158.3** |
| `frame#0` (render, layout, paint) | 6.3 | 7.0 | 0.4 |

End to end (from each row's `.txt`), p50 / p95 ms:

| Row | spawn -> first frame | main -> platform_ready | receipt -> first_frame |
|---|---|---|---|
| direct-4k | 475 / 644 | 285 / 372 | |
| direct-4k-font-no-update | 382 / 466 | 174 / 193 | |
| direct-4k-no-dcomp | 474 / 501 | 291 / 307 | |
| direct-1440p | 676 / 812 | 390 / 466 | |
| warm-4k | 269 / 389 | | 202 / 254 |
| warm-4k-no-dcomp | 307 / 392 | | 224 / 262 |
| warm-1440p | 332 / 384 | | 247 / 289 |
