# 0008: Restart registration and session-end backup

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

Windows can bring an application back after a restart or a sign-out: "Settings > Accounts >
Sign-in options > Automatically save my restartable apps and restart them when I sign back in",
and the same mechanism also reopens an app after a reboot triggered by a Windows Update install.
Both rely on the app having called `RegisterApplicationRestart` while it was running. Tachyon
never called it, so today it only comes back automatically if login autostart
(`tachyon --autostart on`) is already on - the reboot/sign-in itself does nothing.

Separately, ADR 0006 (hot exit) assumed a logoff or shutdown already preserved unsaved text
through the periodic 1.5 s typing-pause backup, but nothing actually hooked
`WM_QUERYENDSESSION`/`WM_ENDSESSION`: a session that ended less than 1.5 s after the last
keystroke could still lose it, and there was no guaranteed final write at all.

## Decision

**Registration (`tachyon_platform::register_restart`/`unregister_restart`, Windows only; no-ops
elsewhere).** `RegisterApplicationRestart("--background", RESTART_NO_CRASH | RESTART_NO_HANG)`:

- `RESTART_NO_CRASH`/`RESTART_NO_HANG` opt out of Windows Error Reporting's own crash/hang restart
  path, which already shows its own "this program has stopped working" dialog with its own restart
  offer - stacking a silent automatic restart on top would be confusing. That path is also the one
  gated by the documented "the system will only restart the application if it has been running for
  a minimum of 60 seconds" rule.
- The reboot and patch-install paths (`RESTART_NO_PATCH`, `RESTART_NO_REBOOT`) are left enabled:
  they are a different code path with no 60 s minimum and no dialog, driven by the installer's own
  `ExitWindowsEx(EWX_RESTARTAPPS)`/`InitiateShutdown(SHUTDOWN_RESTARTAPPS)` call, which is exactly
  what a Windows Update restart and the sign-in "restart my apps" setting need.
- `RegisterApplicationRestart` is a plain Kernel32 export with no packaging requirement, so an
  MSIX build needs nothing extra in its manifest.
- Sources: <https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-registerapplicationrestart>,
  <https://learn.microsoft.com/windows/win32/recovery/registering-for-application-restart>.

Called once, after the first window's first frame (or immediately for a windowless `--background`
primary, which has no first frame to wait for) - never on the startup path - and only for a
resident primary instance (`tachyon_editor::RestartRegistration`, set by `crates/tachyon/src/app.rs`
only when `Lifecycle::is_primary && Lifecycle::resident`): a standalone (`-n`) or secondary process
holds no hot-exit backups of its own, so registering it for restart would bring back a process with
nothing to reopen. Tied to the `hot_exit` setting (`tachyon_editor::sync_restart_registration`,
called at that first-frame point and again by `apply_to_windows` whenever the setting changes -
settings-file save or the command palette's toggle): registered while it is on, unregistered the
moment it is turned off. The relaunch command line is bare `--background` (never the executable
path - `RegisterApplicationRestart`'s own documentation says the OS prepends it), which is already
exactly what login autostart uses: a windowless resident primary that reopens hot exit's backups
once a window is asked for. The existing single-instance claim already drops a forwarded
`--background` launch with nothing to open when a primary is already running
(`crates/tachyon/src/main.rs`'s `claim_instance`), so autostart and this registration racing at
sign-in never opens a duplicate window.

**Session-end backup (`tachyon_platform::TrayEvent::EndSession`).** The tray's hidden window
(`crates/tachyon-platform/src/windows/tray.rs`) now handles two more messages, following
Microsoft's own shutdown guidance rather than answering and hoping the write finishes in time:

- `WM_QUERYENDSESSION`: registers a shutdown block reason (`ShutdownBlockReasonCreate`, "Tachyon
  is saving unsaved documents") unconditionally, then answers `TRUE` at once - Windows measures
  how quickly each top-level window responds to this specific message, so refusing or delaying it
  risks its own "close programs" prompt before the application even reaches `WM_ENDSESSION`. The
  platform layer has no cheap way to know *from here* whether there is actually anything unsaved
  without a synchronous round trip to the application on every single query, so it always
  registers the reason; if it turns out nothing needed saving, `WM_ENDSESSION`'s own call below
  finishes at once and clears it well before Windows would ever show it to the user.
- `WM_ENDSESSION` with `wParam != 0` (the session is actually ending, not cancelled by another
  application refusing `WM_QUERYENDSESSION`): calls the tray's ordinary `on_event(TrayEvent::EndSession)`
  synchronously, which blocks this window procedure - and so this message - until the application
  has written every window's backup or a timeout passes. The reason registered above is destroyed
  afterwards either way (`wParam == 0` too, so a cancelled session end never leaves a stale one
  registered). The process may be killed as soon as `WM_ENDSESSION` returns, so the write cannot
  be deferred to the usual 1.5 s typing-pause delay.

`crates/tachyon/src/app.rs`'s `show_tray` intercepts `TrayEvent::EndSession` in the tray callback
itself (rather than forwarding it through the ordinary async event channel, the way `Open`/`About`/
`Quit` are): it sends a one-shot reply channel to the application's own async loop and blocks on it
with a bounded timeout (`END_SESSION_BACKUP_TIMEOUT`, 30 s). Thirty seconds, not a few, because the
shutdown block reason above means the wait is no longer racing Windows' own ~5 s `HungAppTimeout`
silently - if it runs long, the user sees exactly why ("Tachyon is preventing shutdown", with the
reason) and can choose to wait or force it, rather than the process just appearing hung. If the
wait itself times out, the tray callback logs it (`eprintln!`) before letting `WM_ENDSESSION`
return, so a genuinely wedged UI thread is at least visible somewhere durable instead of silently
producing an incomplete backup.

The async loop runs `save_for_session_end` (every open window's backup through
`Editor::backup_for_session_end`, which returns whether the document ended up safe, then the
session file) and replies once done. Any window reported `false` has its title logged to stderr
(`tachyon: could not back up "<title>" before session end`) - there is no time left for a
user-facing notice this late, but a full disk or a state directory that went away should not fail
completely silently. This is a best effort, not an unconditional guarantee: a wedged UI thread, a
persistently failing write, or a very large number of windows can still exhaust the 30 s budget.

**Correction after the first reboot on the test machine (1.0.0.13).** The tray path above never
ran in time: nothing unsaved was written and the session file stayed 7 minutes old. GPUI handles
`WM_ENDSESSION` on its own windows, on the UI thread, by running the app's quit observers and then
calling `std::process::exit(0)` (`gpui_windows` `handle_end_session`), so the process can end
before the tray thread's request is ever serviced. The save therefore also runs from an
`on_app_quit` observer (`crates/tachyon/src/app.rs`), whenever the quit did not come from
Tachyon's own Quit action. GPUI runs these observers synchronously with every window still open
(it clears them only afterwards), so the observer does the work in its body rather than in the
returned future, which only gets a short timeout. The tray handler stays: it registers the
shutdown block reason and covers a session end that reaches the tray window first. Running the
save twice only rewrites the same files.

## Consequences

- Turning on "Automatically save my restartable apps and restart them when I sign back in" now
  actually brings Tachyon back, with its unsaved documents, after a reboot, a Windows Update
  restart, or signing out and back in - provided hot exit (on by default) is also on.
- A session end (logoff, sign-out, shutdown, restart) now blocks shutdown, with a visible reason,
  for up to 30 s while every open window gets a final, synchronous backup of whatever was not yet
  on disk - closing the gap ADR 0006 assumed was already closed, though as a best effort rather
  than an unconditional guarantee: a failed write or an exhausted wait is reported to stderr, not
  silently swallowed, but does not retry or block shutdown further.
- Two more integration points for a future session-restore feature (issue #79) to hook: the same
  `TrayMessage::EndingSession` arm is the one place a session file would also need writing before
  the process can be killed, and `RestartRegistration`'s pattern (a `Copy` struct of plain `fn`
  pointers, set only by the resident primary) is reusable for tying another setting to another
  Windows-only side effect.
- No effect on Linux or macOS (`tachyon_platform`'s implementations there are no-ops); no effect on
  a standalone (`-n`) or secondary process, or when hot exit is off.
- Revisit if a future Windows App SDK migration replaces `RegisterApplicationRestart` with
  `AppInstance.Restart`, or if `WM_ENDSESSION`'s window ever needs to also flush a session file
  (issue #79) in the same synchronous step.
