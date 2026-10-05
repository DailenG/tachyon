# Security policy

## Supported versions

Tachyon is pre-1.0. Only the latest commit on `main` (and the latest release, once releases exist)
receives security fixes.

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/DailenG/tachyon/security/advisories/new).
Do not open a public issue.

Include the affected version or commit, the platform, reproduction steps, and a sample input file
if the issue is triggered by document content.

You will get an acknowledgment within 7 days. Fixes are released as soon as practical;
the advisory is published once a fix is available.

## Scope

In scope, most relevant for a document editor:

- Crashes, hangs or memory exhaustion triggered by opening or pasting a document (Markdown parsing,
  layout, rendering).
- The single-instance channel: named pipe `\\.\pipe\tachyon-<session>-<user>` on Windows, Unix
  socket in `$XDG_RUNTIME_DIR` (or the temp directory) on Linux/macOS. Another user or process being
  able to inject launches or read their contents is a vulnerability.
- File handling: writing outside the chosen path, following links unexpectedly, lost data on save.

Vulnerabilities in dependencies (GPUI, `pulldown-cmark`, …) should be reported upstream; tell us
too if Tachyon is affected. Known advisories are checked in CI with `cargo deny`.
