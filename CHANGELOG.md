# Changelog

All notable user-visible changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- File paths given on the command line are made absolute before loading.

### Fixed

- Large pastes no longer stall the window: a 5 MB paste now costs a few milliseconds on the UI
  thread before and after its background parse (it was hundreds).
- Editing a line after a paragraph-continuation line could leave a block rendered differently
  from a full reparse (look-behind now reaches the previous blank line).
- Exit with an error instead of hanging when no display server is available on Linux.
- Forwarded launches are acknowledged by the running instance; a launch is no longer lost when the
  secondary process exits before the primary has read it (seen as a flaky test on Windows CI), and
  a secondary whose primary does not reply starts standalone.
