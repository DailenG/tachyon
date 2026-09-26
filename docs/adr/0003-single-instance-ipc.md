# 0003: Single-instance handoff over local IPC

- **Status:** Accepted
- **Date:** 2026-09-25

## Context

Tachyon is launched constantly (file associations, "open with", terminal). A second process per
launch pays full GPU and window startup every time; handing the launch to a running instance costs
only process creation plus a local IPC round trip. The mechanism also enables a resident mode if
cold start cannot meet the budget ([ADR 0004](0004-startup-budget-and-gate.md)).

## Decision

- **Windows:** named pipe `\\.\pipe\tachyon-<session>-<user>`, inbound only, remote clients
  rejected. The process that creates the first pipe instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`) is
  primary; `ERROR_ACCESS_DENIED` means another primary exists. The server creates the next instance
  before serving the current client so a listener always exists. The client calls
  `AllowSetForegroundWindow(ASFW_ANY)` because of Windows' foreground-lock rules.
- **Linux/macOS:** `File::try_lock` on `tachyon.lock` elects the primary, which binds
  `tachyon.sock` beside it, in `$XDG_RUNTIME_DIR` (per-user, 0700) or else the temp directory with
  the user name in the file name.
- **Wire format:** `TACHYON1\n` then NUL-terminated UTF-8 arguments, end of stream terminates, 1 MiB
  cap. Paths are made absolute by the sender.
- **Failure policy:** any error while forwarding makes the process start standalone. A launch is
  never lost.
- Implemented with std plus `windows-sys`; no IPC crate.

## Consequences

- The primary and secondary must be wire-compatible; changing the format needs a new magic.
- Non-UTF-8 paths cannot be forwarded; such launches start standalone.
- The pipe uses the default DACL and the Unix fallback directory may be shared, so another local
  user could squat the name (denial of service) in the fallback case. Revisit with an explicit
  security descriptor if this matters.
