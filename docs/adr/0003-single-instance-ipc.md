# 0003: Single-instance handoff over local IPC

- **Status:** Accepted
- **Date:** 2026-09-25

## Context

Tachyon is launched constantly (file associations, "open with", terminal). A second process per
launch pays full GPU and window startup every time; handing the launch to a running instance costs
only process creation plus a local IPC round trip. The mechanism also enables a resident mode if
cold start cannot meet the budget ([ADR 0004](0004-startup-budget-and-gate.md)).

## Decision

- **Windows:** named pipe `\\.\pipe\tachyon-<session>-<user>`, duplex (for the reply), remote
  clients rejected. The process that creates the first pipe instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`) is
  primary; `ERROR_ACCESS_DENIED` means another primary exists. The server creates the next instance
  before serving the current client so a listener always exists. The client calls
  `AllowSetForegroundWindow(ASFW_ANY)` because of Windows' foreground-lock rules.
- **Linux/macOS:** `File::try_lock` on `tachyon.lock` elects the primary, which binds
  `tachyon.sock` beside it, in `$XDG_RUNTIME_DIR` (per-user, 0700) or else the temp directory with
  the user name in the file name.
- **Wire format:** `TACHYON2\n`, body length as `u32` little-endian, then NUL-terminated UTF-8
  arguments; body capped at 1 MiB. The request is length-framed because Windows pipes have no
  half-close. Paths are made absolute by the sender.
- **Delivery acknowledgement:** the primary replies `OK` only after the application accepted
  (queued) the launch. The secondary treats the launch as forwarded only after the reply; without
  it (primary gone, hung, quitting, or stalled by another client) it starts standalone. On Unix the
  whole request is bounded by one 2 s deadline and the secondary's write and read by 2 s timeouts;
  on Windows the secondary's exchange runs on a helper thread with an overall timeout, since
  synchronous pipe I/O cannot time out.
- **Access control:** the pipe has a protected DACL granting access only to its owner and SYSTEM
  (`D:P(A;;GA;;;OW)(A;;GA;;;SY)`); the default DACL would let any local user open it for reading and
  occupy the listener. The Unix socket is created with mode 0600, so only its owner can connect.
- **Failure policy:** any error while forwarding makes the process start standalone. A launch is
  never lost; in the rare case of a reply lost after delivery it may open twice.
- Implemented with std plus `windows-sys`; no IPC crate.

## Consequences

- The primary and secondary must be wire-compatible; changing the format needs a new magic.
- Non-UTF-8 paths cannot be forwarded; such launches start standalone.
- A process of the *same* user that connects to the Windows pipe and never writes stalls the
  listener; later launches time out and start standalone. Another user who creates the pipe name
  first makes launches start standalone (no handoff) but cannot receive them.
