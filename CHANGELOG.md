# Changelog

All notable user-visible changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- An About window: the owner's signature artwork (a light-ink variant for the dark theme),
  version (and the full MSIX package version, for a packaged install), and an Environment table
  (platform, install kind, resident or standalone, update channel derived from the packaged
  version, and clickable Settings/Backups paths and project website). Opens from the command
  palette's "About Tachyon" row, the Windows tray's "About Tachyon" item above "Quit Tachyon", or
  `tachyon --about` on the command line (forwarded to a running instance like other launches).
  Reused if already open; `Escape` or its Close button closes it. The signature is embedded in
  the binary (`assets/brand/signature.png`, `signature-light.png`) and decoded only when the
  window opens, never on the startup path.
- Command palette (`Ctrl+Shift+P`/`Cmd+Shift+P`): every user-facing action (New window, Open,
  Save/Save As, Close window, Quit, Find/Replace/Find next/previous, Go to heading, Open recent,
  Copy as HTML, Toggle plain-text mode, Zoom in/out/reset, Undo/Redo, Select all, Document
  start/end, the frame-time overlay, Open settings file, About Tachyon) plus the theme and hot
  exit settings, which apply at once and persist to `settings.toml` without disturbing its
  comments or other keys. Reuses the picker overlay: type to filter (case-insensitive substring,
  then subsequence, closest match first), Up/Down/Enter/Escape as in the other pickers; each row
  shows its current keyboard shortcut, read from the real keymap. `Ctrl+P`/`Cmd+P` is a
  quick-open shortcut for the same Open recent list. Nothing is built until the palette opens.
  The palette, Go to heading and Open recent now open a fifth of the way down the window instead
  of against its top edge.
- Markdown mode no longer costs memory and frame time proportional to a whole oversized block:
  a fresh open streams the parse back in 4 MiB windows instead of copying the whole file into one
  `String` first; a run of verbatim text split across parser events (a long fenced code block,
  one source-map span per line before) now merges into one span per unbroken run instead of one
  per line; the active block's raw source, or a rendered block's line list, above a size
  threshold is drawn as a window of segments/lines near the viewport instead of the whole block,
  with the rest reserved as plain height so scrolling and the scrollbar stay correct; and an
  off-screen block's parsed content is dropped and re-derived on demand when it scrolls back into
  view, so a well-structured multi-megabyte document's memory stays close to what is on screen
  instead of growing with everything that was ever visible. A 15 MiB Markdown file shaped like a
  giant fenced code block, one no-blank-line paragraph, or one unwrapped line - which used to risk
  gigabytes of memory and multi-second, unresponsive frames on open - now peaks at 300-480 MB with
  no frame over 16.7 ms opening it.
- Plain-text mode: any file that is not Markdown by extension (`.md`, `.markdown`, `.mdown`,
  `.mkd`, `.mkdn`, `.mdx`; anything else) opens as literal text, syntax hidden nowhere - `#`, `*`
  and the rest stay exactly what you typed. `Ctrl+Shift+M` toggles the current document between
  plain and Markdown, keeping its text and undo history; toggling to Markdown is refused, with a
  notice, above 16 MiB (a full parse at that size risks a visible stall), and a Markdown file that
  large opens as plain text automatically with the same notice.
- Large and unusual files no longer risk crashing or freezing Tachyon. Files are streamed in from
  disk in 1 MiB chunks instead of read into one `String` first, so opening a huge file no longer
  doubles its peak memory; invalid UTF-8 is replaced with U+FFFD instead of failing to open
  (Save / Save As then asks "Save Anyway" or "Cancel" first, since saving would replace the
  original bytes for good), and files over 2 GiB are refused outright with a message rather than
  risking exhausting memory. A single pathologically long line (no `\n` for a long stretch, as in
  some logs and single-line minified files) is split into ≤ 8 KiB display-only chunks so it never
  costs a full layout at once; the caret still moves through it, with arrow keys, exactly as if it
  were one continuous line. A document over 64 MiB is backed up (hot exit) only on quit or close,
  not after every pause in typing, and both backups and saves build the text to write off the UI
  thread from a cloned snapshot of the buffer. Find on a document over 5 MiB runs on the
  background executor instead of the keystroke's frame, showing "searching…" until it lands.

- The app icon (tray, windows, taskbar, executable, Linux launcher) is the new Tachyon mark; the
  raster icons use the simplified mark (the same art as the website's favicon) at 16–32 px. `cargo xtask icons`
  regenerates the icon from `assets/brand/`.
- Windows: `tachyon.exe` carries its icon and version information (Explorer, Task Manager,
  pinned taskbar buttons).
- `tachyon --desktop-entry on|off` (Linux) adds Tachyon to the application launcher and "Open
  with" menus, with its icon, or removes it.
- `cargo xtask dist` packs a release archive: the binary with the README, changelog and licenses
  (`.zip` on Windows, `.tar.gz` elsewhere).
- Pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`: it checks the tag against the
  workspace version, builds `cargo xtask dist` on Windows, Linux and macOS, signs `tachyon.exe`
  with Azure Trusted Signing before packing it, and publishes a GitHub Release with the three
  archives, a `SHA256SUMS.txt`, and notes from the matching changelog section. `workflow_dispatch`
  runs the same build as a dry run that uploads the archives as workflow artifacts without
  creating a release; signing is skipped with a warning when the Azure secrets are not configured.
- Windows: a signed MSIX installer (`cargo xtask msix`) and a `Tachyon.appinstaller` published
  with each GitHub Release, so `Add-AppxPackage -AppInstallerFile` (or just opening the
  `.appinstaller`) installs Tachyon and checks for updates on every launch. The package registers
  an App Execution Alias (`tachyon` from any terminal), `.md`/`.markdown` and (as a separate
  `plaintext` association, since plain-text mode opens them too) `.txt`/`.text`/`.log` file
  associations, and
  disables MSIX's write virtualization so settings, backups and autostart use the same real
  locations as the `.zip` build; `--autostart on` targets the execution alias instead of the
  versioned install path when run from an MSIX install, so it survives updates. The release
  workflow signs and verifies the MSIX the same way as `tachyon.exe`.
- Versioning: the MSIX version is now `X.Y.Z.B`, with stable releases keeping `B = 0` and a new
  nightly channel ([`.github/workflows/nightly.yml`](.github/workflows/nightly.yml)) using its
  own always-increasing build number instead, so every merge to `main` produces a signed MSIX
  strictly newer than the last nightly and the current stable release; an installed nightly picks
  it up automatically ([ADR 0007](docs/adr/0007-versions-and-release-channels.md)).
- Local images are shown in rendered text (paths relative to the document, absolute or `file:`);
  remote images still show their alt text.
- Settings (`Ctrl+,` opens `settings.toml`): theme (system, dark or light), zoom of new windows,
  hot exit on or off, and whether the rotating tip (below) shows. Saving the file applies the
  theme and the tip setting to open windows.
- A faint "Pro Tip" line (`tips = true`, on by default) behind the document, near the bottom of
  the window: one tip per window, naming a real bound key (`Ctrl+Shift+P` opens the command
  palette, `Ctrl+Shift+M` switches Markdown and plain text, `Escape` leaves editing, `Ctrl+R`
  opens a recent file, `Ctrl+F` finds text, `Ctrl+Shift+O` jumps to a heading) or `Ctrl+click`
  to open a link, rotating through the list as windows open. It sits behind the document (typed
  text paints over it) and never captures clicks or becomes selectable.
- Open recent (`Ctrl+R`): pick one of the last 30 files you opened or saved; type to filter by
  name or folder.
- Recent files reach two OS-native surfaces, not just `Open recent`: on Windows, right-clicking
  the taskbar or Start icon shows a "Recent" jump-list category (newest first, up to 10) and a
  "New window" task; on Linux and BSD, opened and saved files are added to the freedesktop
  recently-used list (`~/.local/share/recently-used.xbel`), which GTK and Qt file choosers read
  as their own "Recent" list. Both update off the UI thread whenever `Open recent`'s own list
  changes (open, save, or a backup restored). The portable `.exe` sets an explicit
  AppUserModelID (`DailenG.Tachyon`) so the jump list attaches to the right taskbar icon; an
  MSIX install already has one from its package identity.
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

### Changed

- Clicking below the last block now puts the caret at the document's end instead of leaving it
  wherever it was; clicking beside or between blocks (margins, window gutters, gaps) now goes to
  the nearest block, at the position closest to the click, instead of doing nothing. `Escape`,
  when no find bar, picker or prompt is open, now leaves edit mode: every block renders (the
  active block's own raw view returns to rendered) and the caret stops painting, though it keeps
  its offset; the next click, keystroke, caret movement or edit resumes editing right there.
  `Escape` also collapses a selection to a caret. Plain text has no raw/rendered distinction, so
  `Escape` does nothing there.

- New light and dark palettes from the design direction: navy-tinted surfaces and deep-navy text,
  one brand-blue accent, and syntax colors that all meet 4.5:1 contrast (muted text and code
  comments were below it on the editing card and code backgrounds).
- Find matches are underlined, and the current match has a thicker underline and a stronger fill
  (it used to look like a plain selection). The editing card, find bar, pickers, prompt and task
  boxes have visible 1 px control borders; the selected picker row is solid accent. The find bar,
  pickers and prompt fit windows down to 480 × 360, and the Linux prompt no longer dims the
  document behind a translucent backdrop.
- The find, replace and picker fields have a 1 px border that turns accent on the field receiving
  typing, and a painted caret (the caret character showed colour fringes on Windows). While the
  find bar or a picker takes typing, the document caret is hidden, so only one caret shows.
- The find bar no longer covers the match it points at: revealing a match keeps it below the bar,
  and at the top of the document the text moves down under an open bar.
- Windows: the native title bar follows Tachyon's theme (the `theme` setting, or the system
  appearance with `theme = "system"`) instead of the system dark-mode setting; it was dark above a
  light document.

### Fixed

- Typing into the active block of a Markdown file shaped like one giant fenced code block or one
  no-blank-line paragraph (many megabytes, no blank line anywhere to split it) no longer costs
  20-61 ms on the keystroke's frame. `Document::stale_block` no longer copies and rescans the
  whole stale block on every keystroke to look for a new pre-segmenter boundary: a block that
  large already had none before the edit (ADR 0005's segmenter invariant), so which of "definitely
  a fence" (no boundary is ever possible) or "definitely not" (any new boundary can only appear
  near the edit) it is is told from its own first line, and only a small window around the edit is
  rescanned either way - and that rescan itself now walks the rope's own chunks instead of copying
  the range into a `String` first. `Editor::render_raw` also splits an oversized active block into
  ~4 KiB segments instead of ~16 KiB ones, so re-shaping the one segment a keystroke touches stays
  cheap regardless of the block's own size. Measured (Linux, release, a 15 MiB single-block file,
  40 keystrokes at 60 ms each): worst frame at the middle 28.7 ms → 15.8 ms (fenced) and
  49.2 ms → 9.4 ms (paragraph); at the end of the file 61.0 ms → 13.5 ms (fenced) and
  41.4 ms → 8.7 ms (paragraph); no frame over budget after the fix, versus most frames over it
  before. Normal-file typing, a 5 MB paste and `cargo bench -p tachyon-doc`'s keystroke p99 are
  unaffected.
- A large paste's streamed background parse could invent a footnote: the presegmenter that picks
  provisional chunk boundaries tracked fenced code but not `<div>`-style HTML blocks, so a fence
  marker swallowed as literal HTML content could desync its notion of "inside a fence" from the
  real parser, and a later boundary it proposed as safe (after what looked like a blank line
  outside any fence) could land inside a fence that was genuinely still open. A window starting
  there then read a `[^label]: text`-shaped line of code as a real footnote definition and
  resolved a `[^label]` reference further down the same window against it - unlike the equivalent
  bug already fixed for link reference definitions, nothing rechecked the reference once the
  window's start was reparsed correctly, because the document-wide footnote set it depends on
  never actually changed (the phantom definition never left that one mis-windowed parse). The
  presegmenter now recognizes an open HTML block exactly like an open fence and never proposes a
  cut inside either; typing into a huge single block that opens with an HTML tag (the same shape
  the keystroke-cost fix above targets) now always rescans the whole block rather than risking
  the same misread through that fix's bounded-window shortcut, which does not hold for a
  construct that, unlike a fence, ends at the next blank line.
- Table grid lines are 1 px: neighbouring cells drew two lines side by side (2 px at 100 %, and
  visibly heavier than card and field borders at 150 %).
- Windows: the tray icon's context menu ("New window", "Quit Tachyon") now follows Tachyon's
  resolved theme instead of always rendering light, matching the native title bar
  (`set_popup_menu_dark`; an undocumented `uxtheme.dll` mode switch, Windows 10 1903+, the same one
  Windows Terminal and Notepad++ use, since `TrackPopupMenuEx` has no documented dark-mode option).
- Typing right after pasting a large text no longer stalls a frame: the typed text is queued and
  inserted right after the paste lands, instead of the whole paste being inserted on the
  keystroke's frame (25-72 ms for 5 MB).
- The last step of a large paste's background parse no longer costs a frame: the document-wide
  link and footnote table is rebuilt on the parse thread (it took up to 17 ms on the UI thread for
  a 5 MB paste on Windows).
- Windows: pasting a large text and typing right away could crash Tachyon a few seconds later.
  The clipboard is now read once per paste, on one thread; a key typed before the background read
  finishes waits for that read instead of reading the clipboard again.
- The in-window prompt (Linux) ignored the `theme` setting and followed the system appearance.
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
- Plain-text mode: undoing or redoing more than one change in a single undo group (for example,
  a burst of typed keystrokes, or "Replace All") no longer costs time proportional to the whole
  document. Each change in the group is now re-chunked against the buffer state it individually
  produced, the same as typing, instead of the whole group being applied first and every change
  in it separately triggering a re-chunk of everything from the edit to the end of the document.
- Plain-text mode: any edit that changed the line count by an amount that is not a multiple of
  256 (pressing `Enter`, pasting or deleting lines) re-chunked and re-rendered the entire rest of
  the document, not just the edited part (487 ms for one `Enter` near the top of a 1,000,000-line
  log). Chunk boundaries no longer have to match a from-scratch chunking of the whole document;
  an edit now re-chunks only the block(s) it touched, splitting one that grew past the maximum
  and merging one that fell under the minimum with its next neighbour, so the cost and the number
  of blocks touched no longer depend on the document's size.
- Replace All on a document large enough for background find (over 5 MiB) silently replaced only
  the first 10,000 matches - the display highlight cap - with no notice, instead of every match
  (up to ~530,000 on a 200 MB log, or 2.7 million on a 1 GB one). It now scans and replaces every
  match, off the UI thread, as one undo step; the find bar shows "replacing…" while it runs and
  "Replaced <count>" once it lands.
- Forwarded launches are acknowledged by the running instance; a launch is no longer lost when the
  secondary process exits before the primary has read it (seen as a flaky test on Windows CI), and
  a secondary whose primary does not reply starts standalone.
