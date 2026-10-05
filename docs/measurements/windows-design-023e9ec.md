# Windows design and interaction check (023e9ec)

Physical console of the Windows reference PC, no RDP. Builds: `main` at
`023e9ec feat(tachyon-editor): style existing components with the design tokens (#46)`, plus
`feat/brand-icons` (PR #45) at `4fedbb2 feat: app icons from the brand art; cargo xtask icons` for
the icon parts. Both were built with `cargo build --release --locked -p tachyon` (main 4m 20s, icons
5m 22s, both exit 0). No product code was changed.

Every test process ran with `TACHYON_INSTANCE_ID=design-check` and with `APPDATA` and
`LOCALAPPDATA` pointing at a scratch profile under `C:\temp\tdc-scratch\<test>\AppData`. That
profile held a `Tachyon\settings.toml` with `theme = "dark"` or `theme = "light"`. The exceptions
are `cargo xtask bench-startup --warm`, which sets its own `tachyon-bench-<pid>` id, and the visual
check, which leaves out `-n` on purpose so that step 6 can open files in the same private instance.
Before every batch of keys, the harness brought the window to the front and checked with
`GetForegroundWindow` that it was the expected Tachyon window. It retried when the check failed.
Before every mouse click, it checked with `WindowFromPoint` that the point was on the expected
window. No keys or clicks reached another application.

## Environment

| Item | Value |
|---|---|
| Windows | Windows 11 Pro Insider Preview 26H2, version 10.0.26340, build 26340.9577 |
| CPU | Intel Core Ultra 7 155H (16 cores, 22 threads), 31.5 GB RAM, on AC power |
| GPU | Intel Arc Graphics, driver 32.0.101.9026 (a "StarDesk Virtual Display Adapter" is also installed) |
| Monitors | One: `\\.\DISPLAY1`, primary, 3840x2160 @ 30 Hz, 96 dpi = 100 % scaling |
| System light/dark | Dark (`AppsUseLightTheme=0`, `SystemUsesLightTheme=0`) |
| High contrast | Off |
| Session | `console`, Active (no RDP) |
| Default `https` handler | Switchbar (a browser picker) |
| Commit built | `023e9ec19f2a55e83d07d89ef489195c4ee9e005` (main), `4fedbb2fc3216e31ca8db6d86859ba4d3781e0c4` (feat/brand-icons) |

## Startup

The main binary ran with the scratch profile.

Cold direct launch: `cargo xtask bench-startup --no-build --bin <main exe>` (20 runs), exit 1:

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame      1011.73   316.41   443.63   443.63
main -> first_frame        507.89   298.07   415.65   415.65
main -> platform_ready     320.88   206.61   292.02   292.02
main -> window_open        479.27   279.30   392.02   392.02

FAIL: p95 spawn -> first frame 443.63 ms (budget 50 ms, runs 2-20)
```

Resident launch: `cargo xtask bench-startup --warm --no-build --bin <main exe>`, run 1, exit 0:

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame        25.84    17.50    29.53    29.53
receipt -> first_frame       7.46     2.87     4.06     4.06

PASS: p95 spawn -> first frame 29.53 ms (budget 50 ms, runs 2-20)
```

Resident launch, run 2, exit 0:

```
metric                      first      p50      p95      max   (20 runs, ms)
spawn -> first frame        35.32    17.82    20.18    20.18
receipt -> first_frame       6.39     2.80     3.74     3.74

PASS: p95 spawn -> first frame 20.18 ms (budget 50 ms, runs 2-20)
```

- **Budget: PASS.** The 50 ms budget applies to launches into a resident instance, and both runs
  meet it (p95 29.53 ms and 20.18 ms).
- **Comparison with the previous run.** Against the previous 22.8 ms p95 (4K @ 30 Hz, DWM
  transitions off), run 2 is 2.6 ms faster and run 1 is 6.7 ms slower. The spread between the two
  runs (9.4 ms) is larger than either difference.
- **Cold direct launch.** The cold launch is far over 50 ms. It is not what the budget covers (see
  ADR 0004), but its table is included above as asked.

## Interaction

Main build, dark theme, frame log on (`TACHYON_FRAME_LOG`), real keystrokes through
`WScript.Shell.SendKeys`. The statistics are the agent's summary of the frame logs (p95 by nearest
rank). The logs are in `C:\temp\tdc-results\*-frames.log`.

### a. Typing (1 MB document)

- **Document.** `gen-paste-doc.ps1 -Bytes 1048576` gave
  `bytes=1048848 sections=1574 sha256=68A32CD3970D5B54B25410439B04E7F520CA71C877775994C089E34BA90BEA18`.
- **Caret.** The caret was put at the end of `fn section_0() -> usize {`, a line in the first
  `rust` block, using Ctrl+F `fn section_0`, Escape, End, End.
- **Typing.** 39 characters (` typed by the windows design check run `), 60 ms apart. They landed
  at the end of that line (`typing-after.png`).

| Section | frames | keys | key_to_paint p50 / p95 / max | busy max | frames busy > 16.7 ms |
|---|---|---|---|---|---|
| typing | 39 | 39 | 1.83 / 2.69 / 4.42 ms | 2.33 ms | 0 |

**Budget: PASS.** Every key was painted within 4.42 ms.

Two earlier attempts are not reported:
- In the first, the foreground reported `hwnd=0` (no window) at key 8 and the harness stopped
  without sending more keys.
- In the second, the first End only collapsed the find selection, so the text landed mid-line.

### b. Find (5 MB document)

- **Document.** The 5 MB benchmark document, with
  `sha256=F9C26053EE214B68D61C3737E57223D8E7FBACDFA29873A800F9E36C46D777DA` (matches the expected
  value).
- **Sequence.** Ctrl+F, type `section 7815` (60 ms per key), Enter, Shift+Enter.

| From Ctrl+F to after Shift+Enter | frames | busy p50 / p95 / max | frames busy > 16.7 ms |
|---|---|---|---|
| all find actions | 16 | 2.00 / 14.88 / 14.88 ms | 0 |

- **Frame budget: PASS.** No frame's busy time went over 16.7 ms.
- **Keystroke budget: FAIL, for one key.** A query keystroke took `key_to_paint_ms=18.25`. It was
  frame 15: `busy_ms=14.88 render_ms=14.88 work=- keys=1 drawn=40 blocks=54712`, the frame that
  scrolls the view to the matches.
- **Other keys.** The other query keys took 5.18 ms at the median. Enter took 2.62 ms and
  Shift+Enter 3.28 ms.
- **Status.** The bar shows `2/2` after Enter (`find-after-enter.png`).

### c. Paste (5 MB)

**Setup.**
- The clipboard text was saved before each run and put back afterward. Every run reported
  `clipboard_restored=True` with an identical string. Only text can be restored this way:
  non-text clipboard content, such as images, would not survive this procedure. The clipboard held
  only text (`UnicodeText,System.String,Text`) at the start.
- A new window (`tachyon.exe -n`) is not empty: it opens the built-in welcome scratchpad
  (`# Tachyon`, "A scratchpad for Markdown…"). The paste goes in at the caret, before that text.

Ctrl+V alone, waiting until the frame log was quiet for 2 s (3 runs):

| Run | frames | busy max | frames busy > 16.7 ms | Ctrl+V key_to_paint | `clipboard` work on UI thread |
|---|---|---|---|---|---|
| paste-only-1 | 9 | 5.18 ms | 0 | 56.50 ms | none (no `clipboard` label) |
| paste-only-2 | 9 | 6.53 ms | 0 | 55.04 ms | none |
| paste-only-3 | 9 | 5.25 ms | 0 | 54.92 ms | none |

- **Frame budget: PASS.** The maximum frame was 5.18 to 6.53 ms, against a previous 8.6 to 12.9 ms.
- **Clipboard read.** As intended since #29, the clipboard read no longer appears on the UI thread.
- **Ctrl+V key_to_paint.** It now takes about 55 ms: the key is painted once the background read
  delivers, about 3 frames later.

Ctrl+V immediately followed by `ZQX` (`SendKeys("^vZQX")`, the keys arriving within about 30 ms):

| Run | busy max | frames busy > 16.7 ms | Ctrl+V / Z key_to_paint | `clipboard` work on UI thread | Outcome |
|---|---|---|---|---|---|
| paste-typeafter-1 | 45.90 ms | 1 | 30.74 / 26.81 ms | 11.55 ms | **process crashed** |
| paste-typeafter-2 | 22.70 ms | 1 | 26.99 / 22.70 ms | 10.92 ms | **process crashed** |

- **What happens.** Typing before the background read finishes applies the paste synchronously:
  the frame with the keys contains `work=clipboard:11.55,edit:10.45` (run 1) and
  `work=clipboard:10.92,edit:8.32` (run 2).
- **Frames over budget.** Run 1's 45.90 ms frame was a later render, `render_ms=45.90`, with
  `blocks=21029` while the parse was streaming. Run 2's 22.70 ms frame was the Ctrl+V frame itself.
- **Key order: correct.** The typed keys land after the pasted text in every run: `ZQX# Tachyon`
  directly after `[ref-7815]: https://example.com/ref/7815` (`paste-typeafter-2-settled.png`,
  `crash-d300-7180-10s.png`).

**Crash.** When keys follow Ctrl+V immediately, `tachyon.exe` terminates with access violation
`0xC0000005` about 4 s after the paste.
- **Trigger.** It crashed 5 times out of 5 with no delay (paste-typeafter-1, paste-typeafter-2, a
  probe, and 2 dedicated repro runs: `EXITED at_ms_after_ctrl_v=4604` and `=4097`). With `ZQX` sent
  300 ms or 3000 ms after Ctrl+V, or with no keys at all, it crashed 0 times in 5 runs (2 + 3).
- **Where.** Every crash is at the same fault offset `0x00000000005ed5c9` in `tachyon.exe`
  (Application Error event 1000; WER `AppCrash_tachyon.exe_…` reports in
  `C:\ProgramData\Microsoft\Windows\WER\ReportArchive`). The PDB resolves it only to a far-off
  symbol (`core::core_arch::x86::xsave::_xgetbv+0xB4C09`, no line info), so the function is not
  identified.
- **State at the time.** In each crash, the frame log stops while the background parse is still
  delivering blocks (last `blocks=25381` to `32634`, against `54719` when it completes).

**Paste budget: FAIL.** Ctrl+V alone meets it. Ctrl+V followed immediately by typing goes over
16.7 ms, puts the clipboard read back on the UI thread, and crashes.

## Visual check

Main build, `sample.md` (headings, bold, italic, inline code, link, `$x^2$`, "tachyon" three
times, ordered list with a nested bullet, open and checked task, quote, table, a `rust` block with
comment, keyword, string and number, horizontal rule).
- **Window sizes.** The default window has a 900x1000 client area; the small one is 480x360,
  reached with SetWindowPos and verified with GetClientRect (`client=480x360 dpi=96`).
- **Screenshots.** They cover the DWM extended frame bounds and are in
  `C:\temp\tachyon-design-check\`, named `<theme>-<default|480x360>-<state>.png`. The states are
  `1-open`, `2-selection`, `3-find`, `4-replace`, `5-headings` and `6-recent`: 24 files.

Colors sampled from the screenshots:

| Surface / element | Dark | Light | Expected |
|---|---|---|---|
| Canvas | `#0E1623` | `#FAFBFE` | `#0E1623` / `#FAFBFE` |
| Raised (raw card, bars, pickers) | `#192434` | `#F1F5F9` | tokens `#192434` / `#F1F5F9` |
| Selected picker row fill | `#8BC3FF` | `#1356A4` | accent `#8BC3FF` / `#1356A4` |
| Text on the selected row | `#0E1623` (dark) | `#FFFFFF` | "light text" per the check description |
| Selection fill (behind text) | `#30445D` | `#CCDAEB` | alpha accent |
| Current find match fill | `#4B413A` | `#EEDDBC` | stronger than the others |
| Other find match fill | `#383638` | `#EFE5D1` | |
| Find underline, current | 2 px (rows 162 to 163) | 2 px | thicker |
| Find underline, others | 1 px | 1 px | |
| Title bar | `#1A2030` | `#1A2030` | |

Contrast computed from those samples (WCAG):
- **Primary text:** on the current match 8.80:1 (dark) and 11.82:1 (light); on the other matches
  10.62 and 12.65; on the selection 8.82 and 11.14.
- **Muted text:** on the selection 5.17 and 5.32; on the canvas 9.43 and 7.30.
- **Code comments:** 8.72 (dark) and 6.13 (light).
- **Text on a solid accent fill:** 9.80 and 7.25.

Every one of these is at or above 4.5:1.

Per screenshot, both themes:
- **1-open.** The colors match: near-black navy with light text, and near-white with deep-navy
  text. The raw card (the heading, which holds the caret) has a 1 px border. There are no
  gradients, shadows or blur. The code block has syntax colors: pink keyword, green string, orange
  number, italic muted comment, blue function name. The link is blue and underlined, and math is
  violet. The checked task is a solid accent box with a check; the open task is an outlined box.
  All text is readable.
- **2-selection.** The paragraph is shown raw, `This` is selected, and the text inside the
  selection is readable.
- **3-find.** The bar shows `1/3` with three amber-underlined matches, two in the raw card and one
  in the quote. The current match has the 2 px underline and the stronger fill, so it can be told
  apart by shape as well as color, though 1 px against 2 px is a small difference. Text inside
  the highlights is readable.
- **4-replace.** `Find speed 1/1 Replace velocity`, with the caret in Replace after Tab, and the
  match in the code string highlighted.
- **5-headings.** The third row ("Closing heading") is selected after one Down, as a solid accent
  row. The selected row is obvious.
- **6-recent.** Two entries (`other-b.md`, `other-a.md` with paths), the first selected.
- **480x360.** Nothing in the find bar, replace bar or pickers is clipped: labels, status `1/3`
  and `1/1`, and the recent paths (`C:\temp\tdc-scratch\visual-*\docs`) all fit. Body text wraps.
  The bars and pickers float over the document and cover the text under them; see Problems found.

Native title bar: the title bar is dark (`#1A2030`) in both themes. On the light theme that means
a dark title bar above a near-white canvas (`light-default-1-open.png`). It follows the system
setting (dark) rather than the Tachyon theme.

The caret did not blink in any capture: it appears, steady, in every screenshot taken at arbitrary
times. It was not sampled continuously over time.

## Keyboard

Main build, dark theme, 480x360, keys only (`kb-*.png`).

| Check | Result | Screenshot |
|---|---|---|
| Ctrl+F opens the find bar, Escape closes it | PASS | `kb-01-find-open.png`, `kb-02-find-closed.png` |
| Ctrl+H, type the query, Tab moves to Replace, Tab moves back | PASS | `kb-03`, `kb-04`, `kb-05` |
| Enter (in Replace) replaces one match | PASS: `The word TACH`, count 1/3 → 1/2 | `kb-06-after-enter-replace-one.png` |
| Ctrl+Enter replaces all | PASS: `no matches` | `kb-07-after-ctrl-enter-replace-all.png` |
| Ctrl+Z undoes replace-all in one step | PASS: after one Ctrl+Z the 2 replaced matches are back (search count `1/2`); the earlier single replace stays | `kb-08`, `kb-09` |
| Ctrl+Shift+O, Down, Down, Up, Enter | PASS: "Lists and tasks" selected; Enter jumps there and shows it raw | `kb-10`, `kb-11`, `kb-12` |
| Ctrl+Shift+O, then Escape | PASS | `kb-13-headings-after-escape.png` |
| Ctrl+R | PASS: opens ("no recent files" in this fresh profile; a filled list is in `*-6-recent.png`) | `kb-14`, `kb-15` |
| Ctrl+W on a modified document | PASS: native dialog (class `#32770`, title "Warning": "Save changes before closing?", Save / Don't Save / Cancel), in the foreground; Escape canceled and the window stayed | `kb-16-ctrl-w-dialog.png` |

States where keyboard focus is unclear:
- **Replace bar.** The only sign of which field is active (Find or Replace) is the text caret.
  Neither field has a focus border or fill (`kb-04` against `kb-05`).
- **Find bar open.** While the find bar is open, a caret is drawn both in the bar field and in the
  document at the current match (`dark-default-3-find.png`, `kb-03`). Two carets make it
  ambiguous where typing will go (it goes to the bar).

## Tray and icons

Branch `feat/brand-icons` (`4fedbb2`), same private id and scratch profile.

- **Background start.** `tachyon.exe --background` shows no window (`visible_windows=0`) and a
  "Tachyon" icon in the overflow flyout (`tray-flyout.png`, `tray-icon-button.png`,
  `tray-icon-16px.png`). PASS.
- **Menu.** Right-click shows "New window", a separator and "Quit Tachyon"
  (`tray-menu-first.png`). PASS.
  - The menu is light (native default) while the system and the app are dark.
  - UI Automation did not expose the menu items, so they were read from the screenshot and clicked
    by position after checking that the point was on the Tachyon process's own `#32768` menu.
- **New window.** A window titled "Tachyon" opened (`tray-new-window.png`). PASS.
- **Quit Tachyon.** The process exited with code 0, and the icon was gone from the flyout
  (`tray-flyout-after-quit.png`). PASS.
- **Left-click.** Left-clicking the icon of a new `--background` instance opened a window
  (`tray-left-click-window.png`), and Quit ended it. PASS.
- **Title-bar and taskbar icons.** Both show the new mark, a light-blue arrow on a navy rounded
  square (`titlebar-icon.png`, `taskbar-button.png`; the taskbar button is named
  "Tachyon - 1 running window").
- **Explorer.** A copy of the icons-branch exe shows the new mark in Explorer (`explorer-exe.png`).
  The exe holds one icon group with 32 px and 16 px images (`exe-icon-icons-*.png`). The `main` exe
  still has the old purple gradient icon (`exe-icon-main-*.png`); expected, since PR #45 is not
  merged.
- **Version resource.** FileDescription `Tachyon`, ProductName `Tachyon`, FileVersion and
  ProductVersion `0.1.0`, Company `Tachyon`, Copyright `MIT OR Apache-2.0`, OriginalFilename
  `tachyon.exe`, on both branches. This was read with `FileVersionInfo`, the same resource that
  Properties > Details shows; the Properties dialog itself was not opened.
- **16 px legibility at 100 %.** The mark can be recognized, but at 16 px the arrow reads much like
  a "3", and the navy tile is low-contrast against the dark flyout (`tray-icon-16px.png`). It is
  legible, but only just.

During the tray test, a Chrome notification toast ("AutoElevate Request", unrelated to Tachyon)
covered the overflow flyout. The click guard refused to click it. The user dismissed it and the
test was rerun.

## Links

Main build, `links.md` containing `[site](https://example.com)` and a bare
`https://example.com/docs`. The caret started in the last paragraph.

- **Ctrl+click on `site`.** The default `https` handler opened with `https://example.com/`.
  Because the handler is the Switchbar browser picker, a picker window titled
  `https://example.com/` appeared instead of a browser tab. The caret stayed in the last paragraph
  (`links-1-after-ctrl-click-site.png`). PASS.
- **Ctrl+click on the bare URL.** The picker switched to `https://example.com/docs`, and the caret
  stayed (`links-2-after-ctrl-click-bare.png`). PASS.
- **Plain click on `site`.** The caret moved into the link text and that paragraph went raw
  (`links-3-after-plain-click-site.png`). PASS.
- **Plain click on the bare URL.** The caret moved into the URL, and no URL was opened
  (`links-4-after-plain-click-bare.png`). PASS.
- **Cleanup.** No browser tab was opened. The picker window was closed with `WM_CLOSE`.
- **Unidentified window.** During the plain clicks, the window snapshot counted one new top-level
  window. It was not identified: it did not have a URL title, and no browser appeared.

## Problems found

1. **Crash on paste followed by typing (main, 023e9ec).** Ctrl+V of the 5 MB document followed
   within about 30 ms by typed keys ends `tachyon.exe` with `0xC0000005` at offset `0x5ed5c9`,
   about 4 s later. It happened 5 times out of 5, and 0 times out of 5 with a delay of 300 ms or
   more or with no typing. See `paste-typeafter-*-frames.log`, `crash-d0-*-frames.log` and the WER
   reports.
2. **Paste budget exceeded when typing right after Ctrl+V.** Frames reached 45.90 ms and 22.70 ms,
   and the 11 ms clipboard read ran on the UI thread (`work=clipboard:11.55` and `:10.92`).
3. **Ctrl+V key_to_paint about 55 ms** in the paste-only runs (the frames themselves were at most
   6.53 ms).
4. **One find keystroke painted in 18.25 ms**, over one frame (5 MB document, frame that scrolls
   to the matches).
5. **Light title bar mismatch.** `light-default-1-open.png`: the native title bar stays dark
   (`#1A2030`) on the light theme.
6. **Selected row text in dark.** `dark-default-5-headings.png`, `dark-default-6-recent.png`: the
   selected picker row is light blue `#8BC3FF` with dark text `#0E1623`, not "solid blue with light
   text" as described. It is readable (9.80:1). This matches the token `on_accent`.
7. **Find bar over the raw card.** `kb-01-find-open.png` (480x360): the find bar overlaps the top
   of the raw card holding `# Design check`.
8. **Replace bar over content.** `dark-480x360-4-replace.png`, `light-480x360-4-replace.png`: the
   Replace bar covers the table's last row and the code comment line. The document does not scroll
   the covered text into view.
9. **Replace field focus.** `kb-04`, `kb-05`: the active Replace or Find field is shown only by the
   caret, with no focus border.
10. **Two carets.** `dark-default-3-find.png`, `kb-03`: a document caret and a find-field caret are
    drawn at the same time.
11. **Tray menu is light.** `tray-menu-first.png`: the tray context menu is light while the system
    is dark.
12. **16 px tray icon.** `tray-icon-16px.png`: it reads like a "3" and has low contrast on the dark
    flyout.
13. **Picker caret colors.** `light-default-6-recent.png`: the picker and find-field caret has
    color fringes (sampled pixel pairs `#712335`/`#1776B8` in light and `#EBC79C`/`#19249C` in
    dark). In Open recent (light) it looks maroon.
14. **Paste window is not empty.** A new window (`-n`, no file) contains the welcome scratchpad,
    so the paste test's "empty scratch window" is not empty. The pasted text goes in before the
    welcome text.

## Not verified

- **Properties > Details dialog.** It was not opened. The same version resource was read with
  `FileVersionInfo`.
- **Caret blinking over time.** It was not sampled continuously; the caret was steady in every
  capture.
- **Screen readers.** Not tested beyond noting that UI Automation exposes no items for the tray
  menu.
- **Other scaling factors or monitors.** Only 100 % on the single 3840x2160 monitor was tested.
  Settings were not changed.
- **Browser tab opening.** The default handler is a browser picker, so no real browser tab was
  opened. The check stops at the handler receiving the right URL.
- **The unidentified window** during the plain link clicks.
- **The crashing function.** Only the faulting offset is known; the PDB gives no line information
  for it.
- **Non-text clipboard content.** Only text was on the clipboard. Non-text content such as images
  would not have been restored.

## Artifacts

- **Screenshots** (`C:\temp\tachyon-design-check\`, 94 files, not committed):
  - Visual: `dark-*` and `light-*` (24).
  - Keyboard: `kb-01` to `kb-16b` (17).
  - Typing: `typing-before.png`, `typing-after.png`, `probe-end-*` (the End-key probe).
  - Find: `find-after-query.png`, `find-after-enter.png`, `find-after-shift-enter.png`.
  - Paste: `paste-only-*`, `paste-typeafter-*`, `probe-typeafter-*`, `crash-d300-*`,
    `crash-d3000-*`.
  - Tray and icons: `tray-*`, `titlebar-icon.png`, `taskbar-button.png`, `explorer-exe.png`,
    `exe-icon-*`.
  - Links: `links-*`.
- **Frame logs and bench output:** `C:\temp\tdc-results\`.
- **Harness scripts** (PowerShell, run with `-File`): `C:\temp\tdc-scripts\`.
