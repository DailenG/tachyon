# 0009: Session restore on top of hot exit

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

Hot exit (ADR 0006) restores every **unsaved** document after Quit, logoff or shutdown, but not a
window that was showing a clean, saved file, and not any window's position, size, caret or scroll
position. Editors built for the same use (Notepad++, Sublime Text, VS Code) restore the whole
session on top of that, not just the unsaved half of it, and Tachyon should too (issue #79).

## Decision

- On Quit (`Ctrl+Q`, `tachyon --quit`, the tray's Quit, and - the parent's own handling of
  `WM_ENDSESSION`, `docs/adr/0008-restart-registration.md` - Windows session end), the primary
  writes one small file, `state_dir()/session-<instance>.txt`, before its windows close
  (`app::write_session_now`), atomically (a temp file, then rename - the same mechanism hot exit's
  own backups use). For each window worth remembering, in the order it was opened
  (`WindowCascade`): its file path (a clean, file-backed document) or its hot-exit backup id (an
  unsaved one - hot exit already keeps its text; the session file only needs to name the same
  slot), the window's bounds and whether it is maximized, the caret and scroll offsets, and
  Markdown-or-plain-text mode (`Editor::session_state`). An empty, unmodified scratch window
  records nothing.
- A hand-rolled format, not TOML or JSON (no such dependency exists in this workspace, and one
  version number plus a handful of numeric fields per window does not need one): `version = 1`,
  then each window as a `key = value` record, records separated by a NUL byte. NUL is never valid
  in a path on any real filesystem, so the record's last field (the path or backup id) can safely
  contain a literal `\n` with no escaping - the same trick hot exit's own `<slot>.path` file uses
  for the same reason. A corrupt, truncated, huge (over ~1 MB) or unknown-version file is treated
  as no session at all, never a panic - a state file is user input, and a bad one is a normal case
  (`tachyon_editor::session`). Capped at 100 windows, on both read and write.
- On start, a primary with no files on the command line matches the session's recorded windows,
  in order, against what still exists: a file path against the filesystem, a backup id against
  what `Backups::restore` actually finds on disk (`app::restore_sources`). A missing file is
  skipped and collected into a one-line notice shown on the first restored window (the existing
  `Editor::notice` banner, otherwise used for the oversized-Markdown fallback); a missing backup
  id (someone cleared it by hand) is silently skipped, the same as a `Ctrl+W` "Don't Save" would
  have removed it. Any hot-exit backup the session did not know about (a crash before the last
  Quit, or the setting turned on since) still comes back afterward, in `Backups::restore`'s own
  order - hot exit's guarantee is unconditional, independent of this setting. A launch **with**
  files on the command line restores the session first, then opens and focuses the new files
  (opened, and so focused, last in the same startup pass).
- Bounds are clamped to the current primary display's work area (`app::fit_to_work_area`, pure and
  unit-tested), the same shrink-to-fit `initial_bounds` already applies to a fresh window's size,
  since monitors change between sessions. Caret and scroll are reapplied after the document is
  loaded (`Editor::restore_view`), clamped to the document's current length and, for the caret, a
  UTF-8 character boundary - both may be stale if the text changed since the session was written.
  Mode is switched the same way a manual `Ctrl+Shift+M` would (`Editor::retag_to`, factored out of
  `toggle_text_mode`), silently refused above the Markdown size limit for the same reason.
- **Setting:** `restore_session = true` (default on), a settings.toml key and a command palette
  toggle row ("Restore session on start"), following `hot_exit`'s own pattern exactly. Off: the
  session file is neither written nor used, and behaviour is exactly hot exit alone, as before
  this feature existed.
- A windowless `--background` primary (login autostart) restores the session the same way hot
  exit alone already did: carried in `PendingRestore`, opened by whichever forwarded launch or
  tray click brings the first window.
- **Budget:** the session file is read on its own background thread, started alongside settings'
  own thread, and both share one 15 ms wait rather than each getting a fresh one. Restoring many
  windows must not delay the first one's first frame: the startup loop now opens only the first
  window immediately and defers every other one - the rest of a restored session, or several CLI
  files - to right after that window's first frame (the same `on_next_frame` hook that already
  carries the `--startup-report`/What's new work), since creating another platform window
  synchronously costs 40-60 ms on Windows (ADR 0004) and previously ran before the first window
  ever painted. This is a small, unconditional improvement to every multi-window launch, not only
  a session restore.

## Consequences

- A session-restored window skips the resident-mode "ready window" reuse
  (`app::take_ready_window`): that hidden, pre-made window already has its own bounds from the
  ordinary cascade, not the session's recorded ones, so reusing it would show the window in the
  wrong place. Only matters for the narrow combination of a `--background` primary's very first
  forwarded launch with a session to restore - the ready-window speedup still applies to every
  ordinary later launch.
- Caret and scroll are recorded at block granularity (the offset of the block at the top of the
  view, like the existing reparse-anchoring code already uses), not the exact pixel scroll
  position within it - close enough to reopen "where you left off", not a pixel-perfect resume.
- Turning `restore_session` off after a session already exists on disk leaves that file in place,
  unused, until the setting is turned back on; nothing deletes it. Consistent with hot exit's own
  backups, which likewise persist until saved, discarded, or restored.
