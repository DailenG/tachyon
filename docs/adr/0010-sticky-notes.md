# 0010: Sticky notes

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

Post-1.0 feature request from the owner (issue #69): a quick, always-on-top scratch note that
opens from anywhere with a global hotkey, autosaves to its own file with no Save prompt, can fade
when unfocused, and reopens where it was left when the resident process starts again.

## Decision

**Opening a note.** `tachyon --note` on the command line (forwarded to a running instance like any
other launch); the global hotkey held in the `sticky_hotkey` setting, default `"win+shift+n"`;
the command palette's "New sticky note" row; and, on Windows, the tray's "New sticky note" item
above "About Tachyon". All four call the same `open_note_window(NoteSource::New, _)`.

The hotkey (`tachyon_platform::Hotkey`, parsed from settings text such as `"win+shift+n"`:
zero or more of `ctrl`/`alt`/`shift`/`win`, plus exactly one letter or digit, at least one
modifier required) is registered with `RegisterHotKey` on a dedicated Windows thread with its own
`GetMessageW` loop (`tachyon_platform::register_global_hotkey`; Windows only -
`HotkeyError::Unsupported` everywhere else). `RegisterHotKey` ties a hotkey to the registering
thread's message queue, not a window, and `UnregisterHotKey` must run on that same thread, so
dropping the returned `GlobalHotkey` posts `WM_QUIT` to the thread and joins it rather than
unregistering from wherever the drop happens to run. `on_press` runs on that thread and only sends
through a channel; `crates/tachyon/src/app.rs`'s `register_sticky_hotkey` is what actually opens a
window, back on the async loop. An empty `sticky_hotkey` turns the feature's hotkey off entirely;
`sync_sticky_hotkey`/`sync_hotkey` diff the setting against whatever is currently registered, so a
settings save that left it unchanged never unregisters and re-registers for nothing, and this is
tied to a resident primary instance the same way `RestartRegistration` is (a standalone or
secondary process registers no hotkey of its own). A hotkey already held by another application
(`HotkeyError::InUse`) shows a one-line notice ("The sticky-note hotkey (...) is already in use by
another app") in whatever window is open, or queues it for the next note or window to open if none
is; any other registration failure is only logged to stderr.

**Linux.** Wayland has no general global-hotkey API, so `sticky_hotkey` and `register_global_hotkey`
do nothing there (`HotkeyError::Unsupported`, treated the same as an empty setting: quietly no
hotkey). Instead, a compositor keybinding calls `tachyon --note` directly - see the README for a
Hyprland example. The portal `GlobalShortcuts` API is a possible later addition, tracked as an
alternative below rather than built now.

**Storage.** Each note is a Markdown file under `<documents_dir>/tachyon/notes/`
(`tachyon_platform::documents_dir()`: `FOLDERID_Documents` on Windows, `XDG_DOCUMENTS_DIR` from
`user-dirs.dirs` or `$HOME/Documents` on Linux, `$HOME/Documents` on macOS). It autosaves on the
same debounce hot exit's own backup uses after a pause in typing, on the note's own window
closing, at Quit and at a Windows session end (`flush_note`, called instead of the ordinary
backup/session paths for a note window) - never a Save prompt, and closing a note never asks.
A note's file is named once, at its first save: the sanitized, length-cut text of its first
Markdown heading or line, made unique against the notes folder's current contents, or a
`note-YYYYMMDD-HHMMSS` timestamp if there is nothing usable to title itself from; every later save
rewrites the same path, the name never changes after that first save. An empty note (including
one that never had anything typed into it) never creates a file at all, and a note that goes back
to empty after having one deletes it - the notes folder itself is only created on the very first
note ever saved.

**The window.** A note opens compact (360x360 logical pixels, shrunk and cascaded like any other
window if the display is smaller) with no native title bar: `TitlebarOptions::appears_transparent`
is set and the application draws its own compact header instead (`Editor::note_header` in
`render.rs`) - a drag area showing the note's name, a pin toggle (only where
`tachyon_platform::supports_always_on_top()` says the platform does anything with it), and a close
button. The rest of the layout is the same compact editor as an ordinary window, with a smaller
frame gap (one rem instead of one and a half) and no "Pro Tip" line (`tip_overlay` returns `None`
for a note).

**Always on top.** `sticky_on_top` (default `true`) is a new note's starting always-on-top state;
the header's pin button or `Ctrl+Shift+T` (`ToggleNotePin`) toggles it per note from there, calling
`tachyon_platform::set_always_on_top` right away rather than waiting for the next state-file
refresh. On Windows this is `SetWindowPos` with `HWND_TOPMOST`/`HWND_NOTOPMOST` and
`SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE`; `supports_always_on_top()` is `false` everywhere else,
so the header's pin button does not appear there and the toggle is a no-op if reached some other
way.

**Translucency.** `sticky_unfocused_opacity` (0.3 to 1.0, default `1.0`) is a note's opacity while
its own window is not the active one; focused, or an ordinary (non-note) window, is always fully
opaque. Below 1.0 the window is also opened with `WindowBackgroundAppearance::Transparent` instead
of `Opaque` (`note_window_options`), since requesting a transparent background unconditionally
would cost every note a compositor blend even at the fully-opaque default. This is the one
approved exception to `DESIGN_DIRECTION.md`'s "no translucency" rule (see that document).

**Restore.** A resident primary's open notes live in their own state file,
`state_dir()/notes-<instance>.txt` - the same hand-rolled, NUL-separated, path-last format as
`session.rs`, but a wholly separate file and never gated by `restore_session`: a note is
deliberately outside both the regular session file (`Editor::session_state` returns `None` for
one) and hot exit's own backups (`schedule_backup`/`backup_for_session_end` are no-ops for one).
It is rewritten whenever a note is named for the first time, moves, resizes, its pin toggles, or
the set of open notes changes (opened or closed) - except while the primary is quitting, since
Quit itself already writes every open note's state before closing them one by one, and each
note's own close arriving right after would otherwise rewrite the file with that note now gone,
one at a time, ending with an empty list. Every listed note whose file still exists reopens
unconditionally at a resident start (`existing_notes`, filtering by `Path::exists`), including a
windowless `--background` start, at its own recorded bounds (clamped to the current display) and
pin state; a note deleted or moved by hand since is silently skipped. Restored notes open after
the first window's first frame the same way the rest of a restored session does, or immediately
for a windowless `--background` primary, which has no first frame to wait for.

**Speed.** Nothing here runs before an ordinary window's first frame: the notes state file is read
on its own background thread alongside settings' own, sharing the same wait budget, and restored
notes are deferred to right after the first window's first frame exactly like the rest of a
restored session (ADR 0009); the hotkey and restart registration are both synced at that same
point, not before it.

## Consequences

- A sticky note is a small, separate feature surface: its own settings, its own state file, its
  own window chrome, but it reuses every existing mechanism it can (the debounce, the cascade
  math, the atomic-write state-file format, the first-frame sync point) rather than inventing
  parallel machinery.
- Linux and macOS get no global hotkey and no always-on-top yet; a note there behaves like an
  ordinary compact window that a compositor binding, `--note`, or the palette can open, with the
  pin button hidden since it would do nothing.
- The translucency exception is scoped narrowly (sticky notes only, off by default, unfocused
  only) precisely so it does not become precedent for translucency anywhere else in the editor.
- Revisit if the GlobalShortcuts portal or a Linux always-on-top mechanism becomes practical to
  support, or if issue #69's remaining Linux/macOS gaps get their own follow-up issue.

## Alternatives considered

- **Portal-based global hotkeys on Linux** (`org.freedesktop.portal.GlobalShortcuts`): rejected
  for this pass. It needs an async D-Bus session, a one-time user grant dialog, and per-compositor
  portal support that is not yet universal; a compositor keybinding calling `tachyon --note`
  needs none of that and was already good enough for the owner's own Hyprland setup. Left as a
  later addition rather than built now.
- **`WindowKind::PopUp`** for the note window instead of an ordinary (`Normal`) window: rejected.
  GPUI's own doc comment warns to use it sparingly, and a note is a persistent, user-facing window
  a person types in for a long time, not a transient popup - the compact custom header already
  gets the quieter chrome the issue asked about ("smaller, quieter window") without adopting a
  window kind meant for menus and tooltips.
- **Layered-window per-pixel alpha** (Windows `WS_EX_LAYERED` plus `SetLayeredWindowAttributes`)
  for translucency instead of a transparent swap chain: rejected in favor of GPUI's own
  `WindowBackgroundAppearance::Transparent`, which already exists for this purpose and keeps the
  translucency path identical across platforms instead of adding a second, Windows-only
  compositing mechanism alongside it.
