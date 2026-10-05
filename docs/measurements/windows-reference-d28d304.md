# Tachyon measurements: Windows reference machine

## 1. Machine

- OS: Microsoft Windows 11 Pro Insider Preview 26H2, build 26340.9502
- CPU: Intel Core Ultra 7 155H (16 cores / 22 threads)
- RAM: 31.5 GB
- GPU: Intel Arc Graphics, driver 32.0.101.9026
- Other display adapters present: StarDesk Virtual Display Adapter (driver 18.43.12.322)
- Display: 3840x2160 @ 30 Hz, 100% scaling (96 DPI), single screen `\\.\DISPLAY1`. The display offers no 4K mode above 30 Hz.
- Second pass: display temporarily switched to 2560x1440 @ 59 Hz (not saved to the registry), restored to 3840x2160 @ 30 Hz afterward.
- Power: AC (battery status 2), power plan Balanced (381b4222-f694-41f0-9685-ff5bb260df2e)
- Session: physical console (session 2 `console` Active, no RDP connected). Note: the agent session first ran over RDP; the user switched to the console before any benchmark ran.
- Toolchain: 1.98.1-x86_64-pc-windows-msvc (from rust-toolchain.toml), Visual Studio Build Tools 2026, fxc from Windows Kits 10.0.26100.0
- Commit: `d28d304 perf(tachyon-doc): stream large parses back in chunks, caret first (#12)`

## 2. Startup

### Direct launch, 3840x2160 @ 30 Hz (`startup-direct.txt`)

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame       961.83   447.10   511.02   511.02
main -> first_frame        468.92   420.00   472.45   472.45
main -> platform_ready     274.16   288.06   330.24   330.24
main -> window_open        454.01   399.06   452.74   452.74

FAIL: p95 spawn -> first frame 511.02 ms (budget 50 ms, all runs)
```

### Warm launch, 3840x2160 @ 30 Hz (`startup-warm.txt`)

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame       109.22   144.73   188.19   188.19
receipt -> first_frame      84.97    93.24   129.42   129.42

FAIL: p95 spawn -> first frame 185.02 ms (budget 50 ms, all runs)
```

### Direct launch, 2560x1440 @ 59 Hz (`startup-direct-60hz.txt`)

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame       297.65   279.89   323.98   323.98
main -> first_frame        273.68   258.91   305.34   305.34
main -> platform_ready     191.40   179.68   210.62   210.62
main -> window_open        257.83   246.84   284.35   284.35

FAIL: p95 spawn -> first frame 312.06 ms (budget 50 ms, all runs)
```

### Warm launch, 2560x1440 @ 59 Hz (`startup-warm-60hz.txt`)

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame       107.22   127.14   149.59   149.59
receipt -> first_frame      84.45    86.55   101.88   101.88

FAIL: p95 spawn -> first frame 147.67 ms (budget 50 ms, all runs)
```

After-boot run: not done. (A restart later happened to finish the language install, but the startup bench was not rerun after it.)

## 3. Parser bench (`reparse-bench.txt`)

```
full parse, 1024 KiB                         p50    41.8ms  p99    54.7ms  max    54.7ms  (n=10)
full parse, 10240 KiB                        p50   516.5ms  p99   536.7ms  max   536.7ms  (n=3)
paste 5120 KiB: UI-thread part               p50    15.4ms  p99    18.2ms  max    18.2ms  (n=10)
paste 5120 KiB: applying the parse              10.3ms  (parse on background thread: 375.8ms)
paste 5120 KiB: first chunk, applying           44.2µs  (parsed in 8.8ms)
paste 5120 KiB: 38 chunks, applying each     p50   154.5µs  p99     6.4ms  max     6.4ms  (n=38)
  total chunk parse time                       365.5ms  (applying: 12.4ms)
keystroke in paragraph, 1 MiB doc            p50    29.6µs  p99    44.3µs  max   125.0µs  (n=1080)
keystroke in code block, 1 MiB doc           p50    28.1µs  p99    48.9µs  max   129.4µs  (n=1080)
streaming at end, 1 MiB doc                  p50    24.3µs  p99    48.5µs  max   866.9µs  (n=1080)
unclosed fence to EOF (half of 1 MiB)            7.2ms

PASS: keystroke p99 44.3µs (budget 500µs)
```

## 4. Paste test

Generator: `bytes=5243472 sections=7816 sha256=` (hash empty: `Get-FileHash` not recognized in this PowerShell, see section 6). Hash via `sha256sum`: `f9c26053ee214b68d61c3737e57223d8e7fbacdfa29873a800f9e36c46d777da`. Matches the expected hash.

Overlay numbers are read from the `after-paste` screenshot. All runs had a red border (at least one frame over budget).

| Run | Display | Overlay after paste | Formatted Markdown | Caret after paste | Ctrl+Home | Ctrl+End |
|---|---|---|---|---|---|---|
| 1 | 4K @ 30 Hz | frame p50 1.8 ms · max 24.9 ms · 1/12 over 16.7 ms | yes | end of doc (last block `[ref-7815]` shown as raw source); caret bar not visible in shot | yes, `## Section 0`, caret visible | yes, Section 7815; caret bar not visible in shot |
| 2 | 4K @ 30 Hz | frame p50 2.8 ms · max 38.4 ms · 1/15 over 16.7 ms | yes | same as run 1 | yes, caret visible | yes; caret bar not visible |
| 3 | 4K @ 30 Hz | frame p50 2.3 ms · max 40.3 ms · 1/14 over 16.7 ms | yes | same as run 1 | yes, caret visible | yes; caret bar not visible |
| 4 | 1440p @ 59 Hz | frame p50 1.5 ms · max 26.1 ms · 1/22 over 16.7 ms | yes | same as run 1 | not inspected | not inspected |
| 5 | 1440p @ 59 Hz | frame p50 1.6 ms · max 25.1 ms · 1/22 over 16.7 ms | yes | same as run 1 | not inspected | not inspected |
| 6 | 1440p @ 59 Hz | frame p50 1.9 ms · max 27.1 ms · 1/23 over 16.7 ms | yes | same as run 1 | yes, caret visible | yes; caret bar not visible |

In every run exactly one frame went over 16.7 ms (the paste frame). Keys were delivered on the first try in every run; no click was needed.

## 5. IME

Japanese (Microsoft IME), tested by hand by the user after a restart (`Language.Basic~~~ja-JP: Installed`).

- Candidate window next to the caret: yes
- Composing text underlined: yes
- Enter inserted 日本語: yes
- Escape removed the uncommitted text: yes
- One Ctrl+Z removed the whole committed word: yes
- Anything odd (flicker, caret position, lost keystrokes): nothing odd

Chinese (Microsoft Pinyin), tested by hand by the user (`Language.Basic~~~zh-CN: Installed`). Steps: `nihao`, Space; `zhongwen`, Escape; Ctrl+Z.

- Candidate window next to the caret: yes
- Composing text underlined: yes
- Space inserted 你好: yes
- Escape removed the uncommitted text: yes
- One Ctrl+Z removed the whole committed word: yes
- Anything odd: nothing odd

Earlier scripted attempts are discarded: keystrokes injected with `keybd_event` bypassed IME composition, in Tachyon and in a plain WinForms TextBox alike (raw `nihongo` inserted; a Pinyin retry with scan codes also inserted raw `nihao` in the TextBox), even with the IME reported open in hiragana (`open=1 convmode=0x9`).

## 6. Errors and unexpected

- `gen-paste-doc.ps1`: `Get-FileHash : The term 'Get-FileHash' is not recognized as the name of a cmdlet, function, script file, or operable program.` Output line was `bytes=5243472 sections=7816 sha256=`. The file hash was checked with `sha256sum` instead and matches.
- Warm bench: the summary table p95 (188.19 ms at 30 Hz, 149.59 ms at 59 Hz) differs from the FAIL line (185.02 ms, 147.67 ms). The direct bench shows the same thing at 59 Hz (table 323.98 ms, FAIL line 312.06 ms). At 30 Hz the two agree (511.02 ms).
- Direct launch is much slower at 4K @ 30 Hz (p95 511 ms) than at 1440p @ 59 Hz (p95 324 ms). platform_ready also drops (330 to 211 ms), so not all of the difference comes from vsync.
- The caret bar did not show in the after-paste or Ctrl+End screenshots, but did show after Ctrl+Home. Possibly the blink phase at capture time; the active (raw-source) block at the end suggests the caret is there.
- The display only supports 30 Hz at 4K. A StarDesk virtual display adapter is installed.
- Build output was moved from `C:\cargo-target` to `C:\temp\cargo-target`, and results from `C:\tachyon-results` to `C:\temp\tachyon-results`, after all measurements had run (user request).
