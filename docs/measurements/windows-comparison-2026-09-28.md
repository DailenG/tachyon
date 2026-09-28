# Editor comparison on Windows (Tachyon 1.0.0, 2026-09-28)

Tachyon 1.0.0 measured against Windows Notepad, Notepad++, Typora, Obsidian, MarkText and VS Code
on one Windows 11 laptop, every app with the same scripts, one app at a time. Where an app could not
do a step, the table records that instead of a number. This is the source of the comparison table on
the [website](https://daileng.github.io/tachyon/#compare). Runs that were redone because an
app's own session restore reopened an earlier test file, or because the test harness misfired, were
replaced, not averaged in.

## Machine
- Windows 11 Pro 26H2, build 26300.9457, x64.
- Intel Core i7-7820HQ (4 cores / 8 threads). 32 GB RAM, 24 GB free at start.
- Intel HD Graphics 630 drives the panel (NVIDIA Quadro M1200 is idle, hybrid).
- One 1920x1080 display at 59 Hz, 125 % scaling. Dark mode. OS file cache warm (not a reboot).
- **All apps ran elevated.** The test shell runs at High integrity, so every app inherited it.
  Notepad++ and VS Code show "[Administrator]" in their titles. This applied to every app
  equally. Obsidian's blank window also happened when started unelevated through Explorer, so
  it is not an elevation effect (see Obsidian below).

## App versions
| App | Version | Source |
|---|---|---|
| Tachyon | 1.0.0 (MSIX package 1.0.0.8, nightly; `tachyon --version` = 1.0.0) | installed nightly (updated itself from 0.1.0.7 since 0002) |
| Windows Notepad | 11.2607.14.0 (Microsoft.WindowsNotepad) | built in |
| Notepad++ | 8.9.8.1 x64 | winget `Notepad++.Notepad++` |
| Typora | 1.14.10 | winget: `Typora.Typora` does not exist ("No package found"); installed `appmakes.Typora` instead |
| Obsidian | 1.13.7 | winget `Obsidian.Obsidian` |
| MarkText | 0.19.1 | winget `MarkText.MarkText` |
| VS Code | 1.139.1 (user setup) | winget `Microsoft.VisualStudioCode` |
| Edge (tab test) | 153.0.4234.32 | preinstalled |

## Test files (`work\`)
- `small.md`: 4,541 bytes of realistic Markdown: release notes with headings, lists, a task
  list, a blockquote, three fenced code blocks and two tables.
- `big.md`: 20,972,680 bytes. It is `small.md` repeated 4,537 times; every heading gets a copy
  number (`## 17. Highlights`), then a final line `ENDMARKER 97531`.
- `big.log`: 209,715,369 bytes, 1,603,727 lines, generated with a fixed seed. Each line is
  about 120 chars: ISO timestamp, level, component, id and 6-10 random words. The last line
  holds `ENDMARKER 97531`.
- `other.md`: a 3-line note, used as the "already running" document for warm launches.
- `vault\`: copies of all four files, for Obsidian.

## Method (identical for every app)
- **Watchdog:** a separate PowerShell process polled every 250 ms. It summed working set and
  private bytes over each app's whole process tree (all processes with the app's image name,
  plus descendants such as conhost and copilot-runtime). It would kill the tree above 6 GB WS
  or 8 GB commit. It fired twice, both times on MarkText (below).
- **Launch timing:** `Start-Process <exe> "<full path>"`.
  - Obsidian cannot take a file path, so it got `obsidian://open?path=<vault path>`.
  - Tachyon ran without `TACHYON_INSTANCE_ID`, i.e. its normal default instance.
  - The clock starts right before Start-Process (Stopwatch). The window list was polled every
    10 ms with EnumWindows: visible, not minimized, not cloaked, not owned, at least 150x100.
  - Stop = first such window of the app whose title contains the file name. Obsidian shows the
    note name without extension (`small - vault - Obsidian`). Every result in the tables
    matched on the **title**. The time of the first window of any title is kept in the raw data
    (`first_window_ms`).
- **Cold:** before each run, every instance was closed.
  - Tachyon: `tachyon --quit`. Notepad: Ctrl+W per tab. Others: WM_CLOSE, then any leftovers
    killed after 15 s.
  - Session restore was switched off so the previous file did not come back:
    - Notepad++: `RememberLastSession=no`.
    - MarkText: `startUpAction=blank`. It was `restoreAll`, which reopened the previous run's
      files, so all MarkText numbers were re-measured after the change.
    - VS Code: `window.restoreWindows=none`, `workbench.startupEditor=none`, `files.hotExit=off`.
    - Obsidian: `workspace.json` deleted.
    - Notepad: its TabState entries created by this test were deleted.
  - 2 s pause between runs; 5 runs; the median is reported.
- **Warm:** the app was started once with `other.md` and left running. Each run closes the test
  file's tab/window (Ctrl+W), then launches the app's command line with the file again. 10
  runs, median and p95. For Tachyon this is the resident instance.
- **Memory:** after launch the script waits until the tree's CPU use stays below 50 ms per 1 s,
  then 10 s more. Then it sums **private working set** (Win32_PerfRawData_PerfProc_Process
  WorkingSetPrivate) over the whole tree. Private bytes (commit) is also given.
- **Responsiveness and load time (big.log, big.md):**
  - From the first window, the script pressed Ctrl+End every 500 ms and captured the window
    every 250 ms. OCR (Windows.Media.Ocr) looked for `ENDMARKER 97531`. **Load time** = launch
    to the first capture showing it.
  - Then Ctrl+Home, 3 s pause, one Ctrl+End. **Ctrl+End time** = to the first 250 ms capture
    showing the marker. **The resolution is one capture interval.** "~300 ms" means "not in the
    capture taken right after the key, present in the next one".
  - Then 20 characters (`quick brown fox jump`) with SendKeys at about 60 ms each (measured
    mean gap 65-80 ms). Captures after every second key, then every 100 ms up to 3 s, then
    every 250 ms up to 30 s. **Typed lag** = last key sent to the first capture where OCR reads
    the whole string. Resolution is 100 ms.
  - Then 25 undos, and the app was closed with Don't Save.
- **Idle Tachyon vs Edge tab:** described in its own section.

## Cold launch (median of 5; process start to a visible window titled with the file)
| App | small.md | big.md | big.log |
|---|---|---|---|
| Tachyon | **199 ms** | **216 ms** | **214 ms** |
| Notepad | 436 ms | 2.22 s | 5.70 s |
| Notepad++ | 372 ms | 405 ms | 663 ms |
| Typora | 1.42 s | 1.58 s (shows "too large to render") | 1.18 s (shows "This file cannot be opened in Typora") |
| Obsidian | 934 ms (vault without big.md, see notes) | cannot: vault with big.md crashes the renderer | cannot: .log is not a note; Obsidian hands it to Windows "Open with" |
| MarkText | 1.50 s | 1.57 s (window titled at once; content never finished, see responsiveness) | cannot: opens an empty "Untitled-1" instead |
| VS Code | 1.46 s | 1.56 s | 1.48 s |

## Warm launch (median / p95 of 10; app already running, open the file again via its command line)
| App | small.md | big.md | big.log |
|---|---|---|---|
| Tachyon (resident) | **55 / 102 ms** | **60 / 88 ms** | **52 / 73 ms** |
| Notepad | 120 / 133 ms | 1.90 / 1.93 s | 5.37 / 5.59 s |
| Notepad++ | 87 / 104 ms | 118 / 147 ms | 371 / 388 ms |
| Typora | 1.05 / 1.19 s | 1.20 / 1.30 s ("too large") | 858 / 905 ms ("cannot be opened") |
| Obsidian | 221 / 277 ms (vault without big.md) | cannot (see above) | cannot: no window titled big.log within 240 s |
| MarkText | 557 / 615 ms | cannot: no window titled big.md within 240 s | cannot: no window within 240 s |
| VS Code | 473 / 559 ms | 972 ms / 1.38 s | 2.54 / 3.83 s |

Raw values per run are in [`windows-comparison-2026-09-28.jsonl`](windows-comparison-2026-09-28.jsonl), one JSON record per run. Example, Tachyon small.md cold:
236, 199, 185, 289, 186 ms.

## Memory (whole process tree, private working set, 10 s after loading; commit in brackets)
| App | small.md | big.log | processes |
|---|---|---|---|
| Tachyon | **18.2 MB** (68 MB) | 252 MB (303 MB) | 1 |
| Notepad | 35.3 MB (44 MB) | 900 MB (913 MB) | 2 |
| Notepad++ | 16.0 MB (21 MB) | 233 MB (238 MB) | 1 |
| Typora | 220 MB (342 MB) | 210 MB (332 MB): file not opened | 4 |
| Obsidian | 178 MB (292 MB) (vault without big.md) | 150 MB (259 MB): file not opened | 4 |
| MarkText | 177 MB (273 MB) | 139 MB (231 MB): file not opened (Untitled-1) | 5 |
| VS Code | 1,002 MB (1,406 MB) | 1,062 MB (1,437 MB) | 11-12, including copilot-runtime and a conhost |

## Load time for big.log (launch until the end of the file is on screen after Ctrl+End)
| App | big.log | big.md (for reference) |
|---|---|---|
| Tachyon | **1.80 s** | **0.96 s** |
| Notepad | 28.1 s (title appears at 5.7 s; Ctrl+End only reached the end at 28.1 s) | 5.16 s |
| Notepad++ | 1.46 s | 1.11 s |
| Typora | cannot open (.log) | refuses: "The file is too large to render in Typora." |
| Obsidian | cannot open (.log) | cannot (vault crash) |
| MarkText | cannot open (Untitled-1) | **killed at 6.06 GB** WS (5.9 GB commit) after about 3 min, still not showing the end |
| VS Code | 5.06 s | 2.93 s |

## Responsiveness after loading (Ctrl+End from the top, then 20 typed characters)
| App | big.log: Ctrl+End | big.log: typed text visible | big.md: Ctrl+End | big.md: typed text visible |
|---|---|---|---|---|
| Tachyon | ~316 ms (2nd capture) | **64 ms**, no visible lag | ~282 ms (2nd capture) | **73 ms**, no visible lag |
| Notepad | ~307 ms | **8.1 s** after the last key; 3 s after it showed only "quick br" | 1.34 s | **never complete**: after 30 s it showed "quick  fox", so keys were dropped |
| Notepad++ | 22 ms (1st capture) | 65 ms, no visible lag | 36 ms | **4.2 s** (repeat run: 4.2 s); nothing appeared while typing |
| Typora | n/a (cannot open) | n/a | n/a (too large) | n/a |
| Obsidian | n/a | n/a | n/a | n/a |
| MarkText | n/a | n/a | killed at 6.06 GB | n/a |
| VS Code | ~375 ms | 75 ms, no visible lag | ~321 ms | 108 ms, no visible lag |

The tester kept the screenshots behind every number (final states, load and typing frames); they
are not published here.

## Idle resident Tachyon vs one Edge tab
- **Tachyon:** default instance, installed MSIX, resident mode. I opened `small.md`, closed every
  window, and `--status` said `instance: running`, with 0 windows.
  - Reading A, 30 s later: **16.2 MB** private WS, **65.9 MB** commit (private bytes).
  - Reading B, 5 min later, untouched: **16.2 MB** private WS, **65.8 MB** commit.
  - These include the hidden pre-built window.
- **Edge 153:** started fresh (no Edge processes before), `--new-window
  https://github.com/DailenG/tachyon`. I waited until Edge's CPU dropped below 50 ms/s
  (13 s), then 10 s more, and opened Browser Task Manager.
  - Shift+Esc did not open it from the automation. I used the ... menu > More tools > Browser
    task manager.
  - The row `Tab: GitHub - DailenG/tachyon: ...`: Memory **102,272K** (the same in two readings
    10 s apart).

| | Private working set | Commit (private bytes) | Edge "Memory" column |
|---|---|---|---|
| Tachyon idle, reading A | 16.2 MB | 65.9 MB | |
| Tachyon idle, reading B | 16.2 MB | 65.8 MB | |
| Edge tab github.com/DailenG/tachyon | | | 102,272 K = 99.9 MiB |

- Caveat: Edge's "Memory" column is its own memory-footprint figure, not the same counter as
  the Windows private working set.
- Unusual: the default Edge profile on this machine is signed in and has extensions. It opened
  two extra tabs by itself (a 1Password welcome tab and "What's new in Edge"). Only the tachyon
  tab's row is reported above.
- Cross-check in a clean throwaway profile (`--user-data-dir` under work\, since deleted): the
  tab read 128,780K and then 116,136K. That profile still picked up extensions and extra tabs
  from sync.
- Edge was closed afterwards. 10 msedge processes stayed 30 s after WM_CLOSE (background
  mode) and were killed.

## Unfair or unusual things noticed
- **Typora** will not open `.log` files ("This file cannot be opened in Typora"). It refuses the
  20 MB big.md ("The file is too large to render in Typora" with an "Open in..." button). Its
  big-file launch times are therefore times to that message, not to content.
  - Every cold launch shows a "License Info / trial" nag. The script clicked "Not Now" after
    the title matched, outside the timed window.
  - Unregistered 15-day trial (started during this test).
- **Obsidian:**
  - With the spec vault (big.md inside), Obsidian's renderer climbs to about 4.4-4.5 GB and
    then dies. The window stays a blank dark rectangle; the tree drops to about 70 MB.
    Watchdog peak 4,490 MB WS, which is below the kill limit, so it was not killed.
    Launch to title in that vault: 1.04 / 1.10 s (2 runs), but nothing renders.
  - Removing big.md from the vault fixes it. So Obsidian's small.md numbers come from a vault
    with small.md, big.log and other.md (tagged `-novaultbigmd` in the raw data).
  - `big.log` is not a note type. `obsidian://open` opens a blank "New tab" and hands the
    file to the Windows "Select an app to open this .log file" dialog.
- **MarkText:**
  - Given `big.log` on the command line, it opens an empty "Untitled-1" window.
  - Given `big.md`, the window gets the title within 1.6 s, but the tree grew until the
    watchdog **killed it at 6.06 GB WS / 5.9 GB commit**, both times it was tried (16:52 and
    17:34). Its cold-launch number for big.md is only "title shown".
  - With session restore on (its default was `restoreAll`), MarkText reopened the previous
    runs' files. The first MarkText series was discarded and re-measured with
    `startUpAction=blank`.
- **Notepad (Windows 11):**
  - It shows a titled window within 2.2 s (big.md) or 5.7 s (big.log) but keeps loading for
    about 80 s on big.log. Ctrl+End only reached the end at 28 s.
  - Typing after that lagged 8 s on big.log. On big.md characters were lost: 20 sent,
    "quick  fox" shown.
  - Its tab-session restore reopened earlier test files once. Those responsiveness runs were
    discarded; the Notepad numbers above are the clean `-r3` runs.
- **Notepad++:** loads big.log in about 1.5 s. It uses 233 MB, about the same as Tachyon's
  252 MB. It is the fastest at Ctrl+End (first capture). On big.md (Markdown lexer active),
  typed text only appeared **4.2 s** after the last key, in two runs.
- **VS Code:**
  - Opened in Restricted Mode (workspace trust banner). A Copilot chat panel opens by default.
  - The tree includes a `copilot-runtime` process and a conhost. That is part of VS Code's
    default install, so it was counted.
  - First run showed a sign-in / theme welcome, dismissed once by hand-scripted clicks before
    measuring.
  - big.md peaked at 5.7 GB WS / 6.2 GB commit during warm runs. That is under the limit, so
    not killed.
  - Warm big.log opens took 2.0-3.8 s. Cold took 1.4-1.6 s, because a fresh window is titled
    before the file is read.
- **Tachyon:**
  - Launch-to-title is fast on every file because the window appears before the content finishes.
    For big.log the end of the file was on screen at 1.80 s after launch. For big.md, at 0.96 s.
  - Its hot exit keeps unsaved edits in `%LOCALAPPDATA%\Tachyon\backups\tachyon`. Early typing
    runs were closed without Don't Save, so a 200 MB backup was restored into the next run.
    Those runs were discarded and re-measured after the harness answered Don't Save. The
    backups folder is now empty.
- **SendKeys pacing:** the target was 60 ms; the measured mean gap was 65-80 ms, because each
  SendWait blocks until the app processes the key. Slow apps therefore got their keys slightly
  more slowly, which does not favour Tachyon.
- **First window vs titled window:** Notepad, Notepad++, Typora, MarkText and VS Code all show
  an untitled window 300-900 ms before the title contains the file name. The tables use the
  titled time. `first_window_ms` is in the raw data.
