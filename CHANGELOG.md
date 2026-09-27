# Changelog

All notable user-visible changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Windows: `tachyon.exe` carries its icon and version information (Explorer, Task Manager,
  pinned taskbar buttons).
- `tachyon --desktop-entry on|off` (Linux) adds Tachyon to the application launcher and "Open
  with" menus, with its icon, or removes it.
- `cargo xtask dist` packs a release archive: the binary with the README, changelog and licenses
  (`.zip` on Windows, `.tar.gz` elsewhere).
- Local images are shown in rendered text (paths relative to the document, absolute or `file:`);
  remote images still show their alt text.
- Settings (`Ctrl+,` opens `settings.toml`): theme (system, dark or light), zoom of new windows,
  and hot exit on or off. Saving the file applies the theme to open windows.
- Open recent (`Ctrl+R`): pick one of the last 30 files you opened or saved; type to filter by
  name or folder.
- Files changed by another program reload when you return to the window, unless you have unsaved
  changes; saving over such a change asks first.
- Copy as HTML (`Ctrl+Shift+C`): the selection, or the whole document, rendered as HTML. On
  Windows it is pasted as formatted text in Word, Outlook, Teams and browsers (plain-text targets
  get the Markdown); on Linux and macOS the HTML source is copied as text.
- Hot exit: Quit no longer asks about unsaved changes. Unsaved documents (scratch text and edited
  files) are backed up as you type and reopen, still unsaved, at the next start. Closing a single
  window still asks.
- Syntax highlighting in fenced code blocks (Rust, Python, JavaScript/TypeScript, C-family,
  Go, Java, C#, shells, PowerShell, SQL, JSON, TOML, YAML), in both themes, also while editing.
- Go to heading (`Ctrl+Shift+O`): type to filter the document's headings, `Enter` or a click
  jumps.
- Lists continue on `Enter` with the next bullet, number or an unchecked task box; `Enter` on an
  empty item moves it up a level or ends the list. `Tab` / `Shift+Tab` nest and un-nest list
  items (all the selected ones), renumbering ordered lists.
- Block-swap Markdown editor: the block holding the caret shows and edits its raw Markdown, every
  other block is rendered (headings, emphasis, code, links, lists, task lists, quotes, tables,
  rules, math). Files and the clipboard (`--paste`) open in it; files are read and parsed off the
  UI thread.
- Editing: arrows, word and line movement, selection (keyboard and mouse), copy/cut/paste,
  undo/redo, IME composition.
- In lists, quotes and footnotes only the item or paragraph under the caret switches to raw
  Markdown; the rest stays rendered.
- The view follows the caret's line, so typing in a long code block keeps the scroll position; it
  also stays at the caret after a long paste and jumps with `Ctrl+End` in long documents.
- `TACHYON_FRAME_LOG=<path>`: per-frame timing log (busy and render time, work by kind, key to
  paint latency) for measurements.
- Frame-time overlay, toggled with `Ctrl+Alt+F`: UI-thread time per frame, including edits,
  pastes and applied parse results that run back to back with it.
- Incremental Markdown parsing: keystrokes reparse only the affected blocks; large pastes are
  parsed in the background in 128 KiB chunks, starting at the caret, so the visible text is
  formatted first. Large pastes are also prepared off the UI thread, so a 5 MB paste no longer
  holds up a frame. Only blocks whose reference links now resolve differently are reparsed when
  definitions change.
- Save (`Ctrl+S`) and Save As (`Ctrl+Shift+S`): atomic writes that keep the file's line endings
  and permissions; the title shows unsaved changes. Closing a window or quitting with unsaved
  changes asks first. On Linux the prompt works from the keyboard (Tab/arrows, Enter, Escape).
- Single-instance handoff: a second launch forwards its files to the running instance and exits
  (named pipe on Windows, Unix socket on Linux/macOS). `-n` / `--new-instance` opts out.
- Resident mode: the instance keeps running after its last window closes so later launches open in
  about 30 ms. Default on Windows (`--no-resident` opts out), opt-in elsewhere (`--resident`).
  `--background` starts it without a window, `--autostart on|off` does that at login,
  `--status` and `--quit` report on and end it; `Ctrl+Q` quits for real. On Windows
  a resident instance keeps a hidden window ready, shown without the open animation, so a launch
  draws it within about 5 ms of the resident instance receiving it (about 23 ms from starting the
  launching process).
  `cargo xtask bench-startup --warm` measures such launches (`--gap-ms` spaces them).
- `--startup-report` and `cargo xtask bench-startup` for measuring launch-to-first-frame latency
  against the 50 ms budget.
- `Ctrl+Q` / `Cmd+Q` quits, `Ctrl+W` / `Cmd+W` closes the window.
- An app icon, and on Windows a tray icon while Tachyon runs resident: click it for a new window;
  its menu has "New window" and "Quit Tachyon".
- `Ctrl+click` (`Cmd+click` on macOS) on a link opens it: web and mail links in the browser or
  mail client, links to Markdown and text files in a Tachyon window. Other links are ignored.
- Bare `http://` and `https://` URLs in text are shown and followed as links, as on GitHub.
- Zoom: `Ctrl+=` / `Ctrl+-` step from 50 % to 300 %, `Ctrl+0` resets. Text, indents and
  spacing scale together; zoom is per window.
- A light theme. The theme follows the system appearance, including changes while running; on
  Linux the first frame is already in the right theme (Tachyon asks the desktop portal at
  start-up instead of waiting for GPUI to).
- Find (`Ctrl+F`): typed text goes to the find bar, matches are highlighted in rendered and raw
  text, `Enter` / `Shift+Enter` (`F3` / `Shift+F3`) step through them, `Escape` closes. Lowercase
  queries ignore case. A selection on one line becomes the query. Replace (`Ctrl+H`): `Tab`
  switches to the replacement, `Enter` replaces the selected match, `Ctrl+Enter` replaces all
  (one undo step).
- `Ctrl+N` opens a new window, `Ctrl+O` opens files (each in its own window), and files dropped
  onto a window open too. `Page Up` / `Page Down` move by a screen (`Shift` selects).
- File paths given on the command line are made absolute before loading.

### Fixed

- A large paste could keep a stray link: while its parse streamed in, a window that started
  inside a fenced code block read a line of code as a link definition, and a `[label]` further
  down kept linking to it after the code was parsed correctly.
- Windows: `Ctrl+V` no longer stalls the window while the clipboard is read (≈ 12 ms for 5 MB);
  the text is read on a background thread.
- Linux: bold and italic text rendered in the regular face unless IBM Plex Sans was installed.
  GPUI asks for that family and its fallback fonts come in the regular face only; Tachyon now
  picks the first installed family from a list (Noto Sans, Ubuntu, Cantarell, DejaVu Sans, ...).
- Large pastes no longer stall the window: a 5 MB paste now costs a few milliseconds on the UI
  thread before and after its background parse (it was hundreds).
- Editing a line after a paragraph-continuation line could leave a block rendered differently
  from a full reparse (look-behind now reaches the previous blank line).
- Exit with an error instead of hanging when no display server is available on Linux.
- Forwarded launches are acknowledged by the running instance; a launch is no longer lost when the
  secondary process exits before the primary has read it (seen as a flaky test on Windows CI), and
  a secondary whose primary does not reply starts standalone.
