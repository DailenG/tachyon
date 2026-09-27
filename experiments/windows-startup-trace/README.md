# Handoff: Windows run 4 (showing the ready window, typing latency, frame attribution)

You are a coding agent on the user's Windows PC, in their interactive desktop session. This branch
(`exp/windows-startup-trace`) is `main` plus experiment code and a measurement kit in this
directory; it is never merged. Your job: run three sets of measurements and push the text results
to a new results branch. Do not change code, do not open pull requests, do not touch `main`.

## Why

Run 3 (`docs/measurements/windows-run-3-round-2-b866b36.md` on `main`) met the startup budget
(resident launch p95 29.0 ms) but left two things open:

1. Showing the ready window (`activate.set_placement`) still takes 14-29 ms after its first frame.
   Two experiments, each behind a switch: `GPUI_TRACE_PLACE_HIDDEN=1` (the traced GPUI moves and
   sizes the hidden window when it is created, so showing it does not also move it) and
   `TACHYON_EXP_NO_TRANSITIONS=1` (the ready window has DWM's open animation turned off).
2. Phase 3's exit criterion on Windows: run 3 had one 16.9 ms paste frame and a 20.8 ms frame
   after Ctrl+Home, and typing latency was never measured. This branch's Tachyon can write a
   per-frame log (`TACHYON_FRAME_LOG=<path>`): one line per frame with its time, render time,
   editor work by kind, keys received and each key's time from arrival to the end of that frame's
   paint.

## Kit notes

- `paste-test.ps1`, `resident-check.ps1` and `typing-test.ps1` send keystrokes with SendKeys,
  which go to whatever window is in front. They dot-source `focus.ps1`: before every batch of keys
  they bring Tachyon to the foreground and check the foreground window's process id. If it is not
  Tachyon they throw `FOREGROUND CHECK FAILED before '<step>' ...` and exit with code 1 (Tachyon
  stopped, temp files removed). Each passed check prints `foreground-ok step=<step> pid=<pid>`. On
  a failure, close or minimise the window that holds the foreground and rerun that script; never
  report a run whose script failed.
- The scripts also append `marker ...` lines to frame logs so frames can be matched to actions.

## Rules

- PowerShell: never put an em dash character in any command, script or module. Do not run
  PowerShell one-liners through `bash -Command "..."`. Write scripts to `.ps1` files and run them
  with `powershell -NoProfile -ExecutionPolicy Bypass -File <script>.ps1 <args>`.
- Keep everything you create outside the repo (it lives in a Resilio-synced folder): use `C:\temp`.
- Ask the user before installing anything or changing the display mode (this run needs neither).
- Before any benchmark, confirm with the user: physical console, no RDP session connected, AC
  power, heavy apps closed, display at 3840x2160 @ 30 Hz. While scripts run, do nothing else and
  tell the user not to touch the keyboard or mouse; windows flash and keys are typed.
- The paste test replaces the Windows clipboard: tell the user and get an OK first.
- A Herdr server may be running on this machine: do not stop it and do not run `herdr` commands.
- Record errors verbatim and continue with the next step.

Paths: `$Repo` is `C:\Syncs\Resilio\Code\Github\daileng\tachyon`, `$Kit` is
`$Repo\experiments\windows-startup-trace`, results go to `C:\temp\run4-results`.

## Step 1: check out this branch

`git status` in `$Repo` must be clean (otherwise stop and ask). Then `git fetch origin --prune`,
`git checkout exp/windows-startup-trace`, `git reset --hard origin/exp/windows-startup-trace` (the
branch was rebuilt). Record `git log -1 --oneline`.

## Step 2: builds

1. Normal build: `$env:CARGO_TARGET_DIR = "C:\temp\cargo-target"`, then
   `cargo build --release --locked -p tachyon`.
2. Traced build: delete `C:\temp\zed-trace` if it exists;
   `$CargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { "$env:USERPROFILE\.cargo" }`;
   copy `$CargoHome\git\checkouts\zed-*\933d8d9` to `C:\temp\zed-trace` with
   `robocopy <source> C:\temp\zed-trace /E /NFL /NDL /NJH /NJS` (exit codes below 8 are success);
   `git -C C:\temp\zed-trace apply "$Kit\gpui-trace.patch"` (must apply); write
   `C:\temp\zed-trace\patch.toml` from `$Kit\patch-template.toml` with `ZED_TRACE` replaced by
   `C:/temp/zed-trace`; then
   ```powershell
   $env:CARGO_TARGET_DIR = "C:\temp\cargo-target-trace"
   cargo --config C:/temp/zed-trace/patch.toml build --release -p tachyon
   git checkout -- Cargo.lock
   ```
   `git status` must be clean afterwards. Create `C:\temp\run4-results`.

## Step 3: warm benchmark rows (traced build)

Set `$env:CARGO_TARGET_DIR = "C:\temp\cargo-target"`. Run `$Kit\run-trace.ps1` once per row with
`-Repo $Repo -Exe C:\temp\cargo-target-trace\release\tachyon.exe -OutDir C:\temp\run4-results -Warm`:

| `-Name` | extra arguments |
|---|---|
| `warm-4k` | |
| `warm-4k-place-hidden` | `-SetEnv GPUI_TRACE_PLACE_HIDDEN=1` |
| `warm-4k-no-transitions` | `-SetEnv TACHYON_EXP_NO_TRANSITIONS=1` |
| `warm-4k-both` | `-SetEnv "GPUI_TRACE_PLACE_HIDDEN=1;TACHYON_EXP_NO_TRANSITIONS=1"` |

Each prints `<name> exit=<code> trace_lines=<n>`; `trace_lines=0` means tracing did not work: stop
and investigate. Check: the two place-hidden rows' `.trace.txt` contain `window.place_hidden`
lines, the two no-transitions rows' contain `tachyon-exp no_transitions applied=true`.

## Step 4: typing latency (normal build)

1. `gen-paste-doc.ps1 -Repo $Repo -Out C:\temp\run4-results\doc1m.md -Bytes 1048576` (record its
   output line).
2. `typing-test.ps1 -Exe C:\temp\cargo-target\release\tachyon.exe -Doc C:\temp\run4-results\doc1m.md
   -FrameLog C:\temp\run4-results\typing-frames.log`. It types 120 characters 60 ms apart at the
   end of the document, then 120 at its start (about 20 s in total).

## Step 5: paste with frame logs (normal build)

1. `gen-paste-doc.ps1 -Repo $Repo -Out C:\temp\run4-results\paste5m.md`; expected
   `bytes=5243472 sections=7816 sha256=F9C26053EE214B68D61C3737E57223D8E7FBACDFA29873A800F9E36C46D777DA`.
2. After the user's OK for the clipboard: `paste-test.ps1` with
   `-Exe C:\temp\cargo-target\release\tachyon.exe -Doc C:\temp\run4-results\paste5m.md
   -Dir C:\temp\run4-results` for `-Run 1 -TypeAfter -FrameLog C:\temp\run4-results\paste-1-frames.log`,
   `-Run 2 -FrameLog C:\temp\run4-results\paste-2-frames.log`,
   `-Run 3 -FrameLog C:\temp\run4-results\paste-3-frames.log`.
3. For each run record from the screenshots: the overlay text in `after-paste` exactly; whether
   the view is at the end of the document with the caret visible; for run 1 whether `ZQX` is the
   last line; that `ctrl-home` shows `## Section 0` and `ctrl-end` Section 7815.

## Step 6: resident check (normal build)

`resident-check.ps1 -Exe C:\temp\cargo-target\release\tachyon.exe -Dir C:\temp\run4-results`, output
saved to `C:\temp\run4-results\resident-check.txt`. Every result line should end in `ok=True`.

## Step 7: results

1. Write `C:\temp\run4-results\notes.md`: what changed on the machine since run 3, the commit from
   step 1, each script output line (including every `foreground-ok` / failure line), the
   `spawn -> first frame` and `receipt -> first_frame` lines of the four warm rows, the paste
   observations, `resident-check.txt`, and anything unexpected. Tell the user the clipboard holds
   the 5 MB test document.
2. `git checkout -b measure/windows-run-4`, copy `C:\temp\run4-results\*.txt`,
   `C:\temp\run4-results\*.log` and `notes.md` (not the `.md` documents, not the screenshots) into
   `$Repo\measurements\windows-run-4\`, `git add measurements/windows-run-4`, commit
   `measure: Windows run 4`, `git push -u origin measure/windows-run-4`. Then `git checkout main`;
   `git status` must be clean.
3. Show the user `notes.md` in chat.
