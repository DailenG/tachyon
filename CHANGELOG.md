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
- Incremental Markdown parsing: keystrokes reparse only the affected blocks; large pastes are
  parsed in the background.
- Save (`Ctrl+S`) and Save As (`Ctrl+Shift+S`): atomic writes that keep the file's line endings
  and permissions; the title shows unsaved changes. Closing a window or quitting with unsaved
  changes asks first.
- Single-instance handoff: a second launch forwards its files to the running instance and exits
  (named pipe on Windows, Unix socket on Linux/macOS). `-n` / `--new-instance` opts out.
- `--startup-report` and `cargo xtask bench-startup` for measuring launch-to-first-frame latency
  against the 50 ms budget.
- `Ctrl+Q` / `Cmd+Q` quits, `Ctrl+W` / `Cmd+W` closes the window.
- File paths given on the command line are made absolute before loading.

### Fixed

- Exit with an error instead of hanging when no display server is available on Linux.
- Forwarded launches are acknowledged by the running instance; a launch is no longer lost when the
  secondary process exits before the primary has read it (seen as a flaky test on Windows CI), and
  a secondary whose primary does not reply starts standalone.
