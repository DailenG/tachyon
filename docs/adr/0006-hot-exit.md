# 0006: Hot exit: Quit keeps unsaved documents instead of asking

- **Status:** Accepted
- **Date:** 2026-09-27

## Context

Tachyon is a scratchpad for pasted LLM output: most documents are never saved to a file. Until
now Quit (`Ctrl+Q`, the tray menu, `tachyon --quit`) asked "Save changes before closing?" for each
unsaved window, so quitting meant either a string of prompts or losing scratch text. A resident
instance on Windows also ends at logoff or shutdown, where no prompt can be answered and the text
was lost. Editors built for the same use (Notepad on Windows 11, Sublime Text, VS Code) keep
unsaved documents across restarts.

## Decision

- The primary instance backs up every document with unsaved changes to
  `tachyon_platform::state_dir()/backups/<instance id>/` (`<slot>.md` with the text in its original
  line endings, `<slot>.path` with its file, if any), 1.5 s after the last edit, off the UI thread.
  A backup is removed as soon as its document is saved or deliberately discarded.
- Quit closes windows without asking, writing each unsaved document's backup first (on the UI
  thread, so it is on disk before the window goes). If a backup cannot be written, that window
  asks as before.
- The next start of the primary opens the backups again as unsaved documents, with their file
  paths, in place of the scratch window (or before the files on the command line). A background
  start (`--background`) opens them with the first launch.
- Closing one window (`Ctrl+W`, the close button) still asks: that is a decision about one
  document.
- Separate processes (`-n`) and instances without a state directory keep the old behavior, so two
  processes never restore the same backups.

## Consequences

- Quitting is instant and loses nothing, and so is a logoff or crash, up to the last 1.5 s of
  typing.
- Unsaved text now persists on disk in the user's state directory until it is saved or
  discarded. That is the point, but it is also a place where private text stays; closing the
  window with "Don't Save" removes it.
- A restored document belonging to a file remembers the file's version (modification time and
  size) from its backup, so Save asks before overwriting changes made to the file meanwhile.
- Revisit if users expect Quit to ask (an opt-out setting would need configuration, which startup
  deliberately does not read yet).
