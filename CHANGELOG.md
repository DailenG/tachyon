# Changelog

All notable user-visible changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- GPUI window showing Markdown as raw text: the built-in sample, files given on the command line
  (read off the UI thread, CRLF normalized), or the clipboard with `--paste`.
- Single-instance handoff: a second launch forwards its files to the running instance and exits
  (named pipe on Windows, Unix socket on Linux/macOS). `-n` / `--new-instance` opts out.
- `--startup-report` and `cargo xtask bench-startup` for measuring launch-to-first-frame latency
  against the 50 ms budget.
- `Ctrl+Q` / `Cmd+Q` quits, `Ctrl+W` / `Cmd+W` closes the window.

### Fixed

- Exit with an error instead of hanging when no display server is available on Linux.
