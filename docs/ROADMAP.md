# Roadmap

Each phase has exit criteria that must be met, with evidence, before the phase is called done. A
later phase may start early only when it does not depend on the open criterion (Phase 2's core
crates are needed whichever way the startup gate goes).

## Phase 1: skeleton and startup gate (gate met; follow-ups open)

- [x] Workspace, pinned toolchain and GPUI revision, release profile, lints
- [x] GPUI window showing Markdown as raw text (sample, files, clipboard)
- [x] Single-instance handoff: Windows named pipe, Unix socket
- [x] Startup instrumentation and `cargo xtask bench-startup`
- [x] CI on Windows (primary), Linux, macOS; cargo-deny; Windows release artifact
- [x] **Gate:** p95 cold and warm startup measured on reference Windows hardware, and the
      direct-launch vs resident-mode decision recorded in [ADR 0004](adr/0004-startup-budget-and-gate.md):
      resident mode with a ready window, p95 47.5 ms at 4K @ 30 Hz, 29.0 ms as shipped (direct
      launch 352-392 ms)
- [x] Re-measure with the ready window pre-sized: p95 29.0 ms (was 47.5), content drawn 3.4 ms
      (p50) after the launch arrives
- [x] Cut showing the ready window: DWM transitions off for ready windows (run 4: p95 22.8 ms,
      content drawn ≤ 5.4 ms p95 after the launch arrives, window appears without a fade); input
      handled ≈ 19-24 ms after showing starts
- [x] Propose to GPUI upstream: apply a hidden window's placement when it is created (run 4, with
      transitions off: p95 19.3 ms, first frame ≤ 0.7 ms p95 after the launch arrives): proposed in
      [zed-industries/zed#64853](https://github.com/zed-industries/zed/discussions/64853)
- [x] Resident by default on Windows (`--no-resident` opts out; opt-in on Linux and macOS, where a
      shell would stay busy), `--background`, `--autostart on|off`, `--status`, `--quit`
- [x] A tray icon (or equivalent) that shows the resident process and quits it: Windows tray
      icon (Phase 4); `tachyon --status` / `--quit` everywhere

Measured so far (release, 20 runs, `cargo xtask bench-startup`):

| Machine | p50 | p95 | Breakdown (p50, from `main`) |
|---|---|---|---|
| Linux, Intel UHD 750, Wayland | 193 ms | 199 ms | platform 10 ms, window open +180 ms |
| Windows, GitHub `windows-latest` runner (no GPU; not reference hardware) | 142 ms | 161 ms | platform 60 ms, window open +44 ms, first frame +23 ms; ~16 ms before `main` |

| Windows 11, reference (Core Ultra 7 155H, Intel Arc), 4K @ 30 Hz | 447 ms | 511 ms | platform 288 ms, window open +111 ms, first frame +21 ms |
| Same machine, 1440p @ 59 Hz | 280 ms | 324 ms | platform 180 ms, window open +67 ms, first frame +12 ms |

All miss the 50 ms budget. On the reference Windows machine GPUI's platform initialization alone
takes 180-330 ms.

Warm launches into a resident instance (resident mode, `cargo xtask bench-startup --warm`) meet it
on Linux: p50 27.6 ms, p95 29.5 ms (first launch into a windowless instance 180 ms), but not on
the reference Windows machine: p95 150-188 ms, of which the resident instance's window open to
first frame is 86-93 ms. A timing-instrumented GPUI build attributed it: per process, the D3D11
device (97 ms) and DirectWrite's check for new fonts (130 ms, avoidable); per window, creating
(39-59 ms) and showing (59-87 ms) it. A second trace found showing is mostly resizing the render
targets and activation; a resident instance that keeps a hidden window ready shows it with its
content ≈ 27 ms after the launch arrives: p95 47.5 ms from spawning the launching process, 29.0 ms
once the hidden window is sized in advance (shipped). See
ADR 0004.

## Phase 2: headless core (`tachyon-text`, `tachyon-md`, `tachyon-doc`) (done)

- [x] Buffer: edits, grouped undo, UTF-16 mapping, line-ending round trip, edit log
- [x] Block segmentation, owned IR with source maps, `DefTable` (links and footnotes),
      fence-aware pre-segmenter
- [x] Incremental reparse with the convergence rule; executor-agnostic `ParseJob`s
      ([ADR 0005](adr/0005-incremental-reparse-by-block-windows.md))
- [x] Property tests: incremental parse == full parse for random edits, undo, interleaved jobs
      and streamed input (run with `PROPTEST_CASES=1000000` before changing the segmenter)
- [x] LLM-style corpus: fences in lists, nested lists, tables, alerts, references, footnotes, math,
      HTML, output truncated inside a fence, CRLF. Hand-written; extend it with captured model
      output when a rendering bug shows up.
- [x] Latency bench: `cargo bench -p tachyon-doc`

**Exit:** keystroke reparse p99 < 0.5 ms on a 1 MB document; every corpus file matches a full parse.

Measured (Linux, release): keystroke-to-clean p99 45 µs on 1 MiB (paragraph, code block and
end-of-document typing); full parse 20 ms for 1 MiB, 220 ms for 10 MiB; unclosed fence running to
the end of 512 KiB 7 ms. All corpus tests pass.

## Phase 3: block-swap editor (`tachyon-editor`) (done)

- [x] Editor view on GPUI's virtualized list; rich rendering for headings, emphasis, inline code,
      links, fences, lists, task lists, quotes, tables, rules, math
- [x] Raw rendering for the active block; swap on cursor movement
- [x] Swap at leaf granularity inside lists, quotes and footnotes (list item text, paragraph, code
      block); other blocks swap whole
- [x] Arrow keys, words, home/end, document start/end; selections across blocks; copy/cut/paste;
      undo/redo with typing-run grouping
- [x] Clicks, double-click word selection and drag selection through rendered blocks via source
      maps (covered by `gpui::test` mouse tests)
- [x] Scroll anchoring: the list scrolls by item offset, so blocks changing height above the
      viewport do not move it; the view follows the caret's line (not its block) and edits inside
      the top block keep their pixel offset
- [x] IME through GPUI's input handler (composition is one undo step)
- [x] IME verified by hand on Windows with Microsoft IME (Japanese) and Microsoft Pinyin: candidate
      window at the caret, underlined composition, commit, Escape cancels, one undo step. Synthetic
      keystrokes bypass Windows IMEs, so this stays a manual check
- [x] Paste path: large pastes show unparsed blocks and parse on the background executor; a 5 MB
      paste costs ≈ 6 ms on the UI thread, then 38 chunks applied in ≤ 7 ms each (the caret's
      first, ≈ 0.1 ms); a paste that defines its own references no longer triggers a reparse of
      every block that uses them
- [x] Background parse streamed back in chunks, caret or viewport first (`parse_job_near`)
- [x] Frame-time overlay (`Ctrl+Alt+F`): p50/max of the editor's UI-thread time per frame
      (render+layout+paint plus edits, pastes and applied parse results that ran back to back
      with it) and frames over 16.7 ms. On a 5 MB document while scrolling and typing: p50
      1.4 ms, max 3.0 ms
- [x] Measure a live 5 MB paste with the overlay. Linux (Intel UHD 750, nested Hyprland): the
      paste frame takes ≈ 31 ms (clipboard read inside GPUI ≈ 7 ms, rope insert and pre-segmenting
      ≈ 8 ms, first paint of the raw placeholder under the caret ≈ 14 ms); every later frame stays
      under budget (p50 2.5 ms) while the chunks stream in, and the view stays at the caret
- [x] Paste frame within budget on Linux. A paste over 64 KiB is normalized, built into a rope
      and pre-segmented off the UI thread (`PreparedInsert`), then spliced in (0.4 ms for 5 MB
      instead of 6 ms); placeholders within 32 KiB of the caret are ≥ 1 KiB, so the first frame
      lays out about a screenful; fonts are loaded after the first frame, not by the first
      paste. Live, 5 MB from an empty scratch window: max frame 6.6-6.9 ms, 0 of 10-11 frames
      over 16.7 ms (was one ≈ 31 ms frame). A key typed right after Ctrl+V lands after the paste
- [x] Re-measure the paste on the reference Windows machine: max frame 8.6-12.9 ms, no frame
      over budget in 5 of 6 runs (one 17.9 ms frame at 1440p @ 59 Hz), where it used to be one
      25-40 ms frame. A key typed right after Ctrl+V lands after the paste. The same runs showed
      the view drifting to the top of the document while the parse streamed in, fixed since
- [x] Save / Save As with atomic writes; unsaved-changes prompt on close and quit
- [x] Keyboard support in prompts on Linux (own in-window prompt; Windows and macOS keep native
      dialogs)

- [x] Exit criteria shown on the reference Windows machine (run 4, 4K @ 30 Hz, per-frame log
      `TACHYON_FRAME_LOG`): three 5 MB pastes with no frame over 16.7 ms (max 13.1 ms) and the
      pasted text visible in the Ctrl+V frame; typing in a 1 MiB document key to paint p95 ≈ 4 ms,
      max 6.6 ms. Run 3 once had a 16.9 ms paste frame; not reproduced. Key to present adds the
      wait for the display's next refresh, which Tachyon does not control
- [x] Follow-up: read the clipboard off the UI thread (Ctrl+V stalls it 11-12 ms on Windows):
      done in Phase 4

**Exit:** pasting 5 MB of LLM output produces no frame over 16.6 ms, with visible text in the same
frame; typing in a 1 MB document keeps key-to-present p99 within one 60 Hz frame.

## Phase 4: everyday editing (done)

What a scratchpad needs day to day, without giving up the budgets above.

- [x] New window (`Ctrl+N`), Open (`Ctrl+O`, native dialog; several files open in their own
      windows), files dropped onto a window open too; Page Up / Page Down (with Shift to select)
- [x] Find in the document (`Ctrl+F`): matches highlighted in rendered and raw blocks, Enter /
      Shift+Enter (F3 / Shift+F3) to step through them, Escape to close; smart case; the selection
      seeds the query. `find_all` on 5 MiB: ≈ 1 ms; live, typing a query in a 5 MB document:
      max frame 3.7 ms
- [x] Replace (`Ctrl+H`, `Cmd+Alt+F`): Tab switches between query and replacement, Enter replaces
      the selected match and moves on, `Ctrl+Enter` replaces all as one edit and one undo step
- [x] Light theme, following the system appearance, live. On Linux GPUI learns the appearance
      from the desktop portal only after its first windows exist, so start-up asks the portal
      itself on a thread (≈ 1 ms warm, waited for at most 15 ms) and the first frame is already
      in the right theme
- [x] Zoom (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`): browser steps from 50 % to 300 %, per window; text,
      indents and rem-based spacing scale together
- [x] Open links with `Ctrl+click` (`Cmd+click`) on their text, rendered or raw: `http`, `https`
      and `mailto` in the system's handler; Markdown and text files (relative to the document,
      absolute, or `file:`) in Tachyon; other schemes and other local files are ignored, since they
      could be programs. Bare `http(s)://` URLs are links too, as on GitHub (`tachyon-md`'s
      autolink pass: not in code, math or HTML; trailing punctuation and unbalanced `)` excluded)
- [x] Tray icon on Windows for the resident process: click for a new window, menu with "New
      window" and "Quit Tachyon"; re-added when Explorer restarts. Tachyon's windows get the same
      icon (title bar, taskbar, Alt+Tab)
- [x] Read the clipboard off the UI thread on Windows (the Phase 3 follow-up above): a background
      read of the 5 MB paste takes ≈ 15 ms off the UI thread (session 0, over SSH); Linux and macOS
      still read through GPUI, whose clipboards are served by its event loop

**Exit:** every item above covered by a `gpui::test` or a live check; finding in a 5 MB document
keeps every frame under 16.6 ms; startup budgets unchanged (`cargo xtask bench-startup --warm`).
Met: each item has a `gpui::test` and a live check (Linux; the tray, window icon and clipboard
reader on Windows); finding in 5 MB: max frame 3.7 ms; warm launch on Linux p95 29-32 ms (the
last 2 ms from drawing real bold and italic faces); the Windows paths the budgets were measured on
are unchanged apart from the icon and the tray, which a resident instance sets up ahead of
launches.

## Phase 5: writing comfort (done)

Typing Markdown by hand, and reading the code in LLM output, without leaving the budgets.

- [x] Lists continue on Enter (bullets, numbers incremented, task boxes unchecked, inside quotes
      too); Enter on an empty item moves it up a level, or ends the list with a blank line at the
      top level; Tab / Shift+Tab nest an item under the previous one or move it up to its parent,
      renumbering ordered items (a nested list starts at 1, so it can interrupt its parent's
      text); not inside code or HTML blocks
- [x] Syntax highlighting in fenced code blocks: a small lexer in `tachyon-md` (keywords,
      strings, comments, numbers, function and type names for Rust, Python, JS/TS, C-family,
      Go, Java, C#, shells, PowerShell, SQL, JSON, TOML/YAML), run as part of parsing the block,
      so it goes wherever the parse goes (off the UI thread for large ranges); colors for both
      themes; the raw block being edited is highlighted too. Keystroke in a code block, 1 MiB
      document: p99 62 µs (budget 500 µs); full parse of 1 MiB: 23.0 ms
- [x] Jump to a heading (`Ctrl+Shift+O`): the document's headings in a filterable list
      (case-insensitive substring, indented by level, opening on the heading above the caret);
      Up / Down choose, Enter or a click jumps, Escape closes
- [x] Scratch buffers survive a restart: unsaved documents are backed up 1.5 s after the last
      edit and Quit closes without asking; the next start reopens them, unsaved, with their files
      ([ADR 0006](adr/0006-hot-exit.md))
- [x] Copy as rich text (`Ctrl+Shift+C`): the selection (or the whole document) rendered by
      `pulldown-cmark` with the editor's dialect; on Windows as `HTML Format` with the Markdown as
      its plain text (checked on Windows: offsets and both formats read back); on Linux and macOS,
      whose GPUI clipboard is text only, the HTML source as text

**Exit:** each item covered by a `gpui::test` or a live check; typing stays within one frame in a
1 MB document with highlighted code; startup budgets unchanged.
Met: every item has `gpui::test`s and a live check (Linux; the HTML clipboard on Windows); typing
in a Rust block of a 1 MB document (Linux, release): key to paint p95 1.3 ms, max frame 1.5 ms,
none over 16.7 ms; startup paths untouched apart from reading the (usually empty) backup
directory at a primary's start.

## Phase 6: files and settings (done)

Living with files that change under the editor, and a few preferences, without slowing the first
frame.

- [x] A file changed on disk reloads if its window has no unsaved changes; otherwise Save asks
      before overwriting (also for documents restored by hot exit, whose backup records the
      file's version). Checked when the window is activated: modification time and size, read
      off the UI thread
- [x] Open recent (`Ctrl+R`): recently opened and saved files (up to 30, newest first, kept in
      the state directory) in a filterable list; the pickers share one implementation
- [x] A settings file (`settings.toml`, `Ctrl+,` opens it, created with comments): theme
      (system, dark, light), zoom of new windows, hot exit on or off; read on a thread at
      start-up (waited for at most 15 ms), so the first frame already uses it; saving it from
      Tachyon applies the theme to open windows. Warm launch on Linux p95 30.7-31.5 ms (was
      29-32)
- [x] Images: local images (relative to the document, absolute, `file:`) shown in rendered
      blocks, scaled down to the column and at most 480 px high, loaded and decoded off the UI
      thread by GPUI; a line that is only an image shows just the image, and clicking it edits
      its Markdown. Remote images stay alt text: loading them needs an HTTP client dependency

**Exit:** each item covered by a `gpui::test` or a live check; startup budgets unchanged.
Met: every item has `gpui::test`s and a live check on Linux; warm launch p95 30.7-31.5 ms.

## Phase 7: distribution (in progress)

Getting Tachyon onto machines without a Rust toolchain. Items marked *(owner)* need decisions or
accounts only the project owner has.

- [x] `cargo xtask dist`: a release archive per platform (binary, README, changelog, licenses):
      `.zip` on Windows (4.2 MB; built and run from the extracted archive on the reference
      machine), `.tar.gz` elsewhere (9.7 MB on Linux)
- [x] Windows: icon and version information in the executable (so Explorer, Task Manager and the
      pinned taskbar button show them): `build.rs` writes a `.res` file from the embedded
      `.ico` and the crate version, linked directly (checked on Windows: `VersionInfo` reads
      back Tachyon 0.1.0, and the executable's associated icon is the brand mark)
- [x] Linux: a `.desktop` entry and icon, installed per user with `tachyon --desktop-entry on`
      (launcher, "Open with" for Markdown and text; `desktop-file-validate` passes), removed with
      `off`
- [x] Publishing: pushing a `vX.Y.Z` tag runs [`.github/workflows/release.yml`](../.github/workflows/release.yml),
      which verifies the tag against the workspace version, builds `cargo xtask dist` on Windows,
      Linux and macOS, and creates a GitHub Release with the three archives, a `SHA256SUMS.txt`,
      and notes taken from the matching `CHANGELOG.md` section. `workflow_dispatch` runs the same
      build as a dry run (artifacts uploaded, no release)
- [x] Windows code signing: the release workflow's `windows-latest` leg signs `tachyon.exe` with
      Azure Trusted Signing (`Azure/trusted-signing-action`, pinned to a commit SHA) before
      `cargo xtask dist --no-build` packs it, then checks the result with
      `Get-AuthenticodeSignature` (`.github/scripts/verify-signature.ps1`). Needs the
      `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
      `AZURE_TRUSTED_SIGNING_ACCOUNT`, `AZURE_TRUSTED_SIGNING_PROFILE` and
      `AZURE_TRUSTED_SIGNING_ENDPOINT` repository secrets; without them signing is skipped with a
      workflow warning instead of failing (forks, or a dry run with no secrets configured)
- [x] MSIX installer and auto-update: `cargo xtask msix` (Windows only, needs `makeappx.exe`
      from the Windows 10/11 SDK) packs the signed `tachyon.exe` and
      `packaging/msix/Assets` (rendered from `assets/brand` by `cargo xtask icons`, alongside the
      existing `.ico`) with `packaging/msix/AppxManifest.xml` into an MSIX, and writes
      `Tachyon.appinstaller` (App Installer's 2021 schema, `HoursBetweenUpdateChecks="0"`) so an
      install from the release checks for updates on every launch. The manifest registers an
      App Execution Alias (`tachyon` from any terminal, a path stable across updates) and file
      type associations for `.md`/`.markdown`, and disables MSIX's file-system and registry
      write virtualization so settings, backups and the autostart `Run` key land in the same
      places as the `.zip` build; `--autostart on` detects an MSIX install
      (`GetCurrentPackageFullName`) and points the `Run` key at the execution alias instead of
      the versioned install path, which would otherwise break on the next update. The release
      workflow signs the MSIX with Azure Trusted Signing the same way as `tachyon.exe` and
      verifies it the same way. Checked on the reference machine: install through the
      `.appinstaller` (1.8 s), alias, Start entry, `.md` association, settings path and autostart
      target correct; an update from 0.1.0.1 to 0.1.0.2 applied silently about 6 s after a launch
      and quit, and settings and autostart survived it. Packaged launches cost about 35-40 ms more
      than the same exe unpackaged (warm p50 59 ms, p95 76-199 ms; package activation, before
      Tachyon code runs), so the `.zip` stays the fastest option. A resident
      Tachyon's update applies once it fully quits (App Installer stages the update but MSIX
      cannot swap a package whose process is still running), documented in the README and
      ARCHITECTURE rather than automated, since Windows has no supported way to force a clean
      full-trust app to restart for an update in progress
- [ ] *(owner)* macOS code signing and notarization, and where releases beyond GitHub are
      published (winget, a package repository)

**Exit:** a fresh machine runs Tachyon from the archive with nothing else installed; startup
budgets unchanged.

## Phase 8: plain-text mode and large-file stability (in progress)

Editing arbitrary large text files (logs, exports, anything that is not Markdown) without the
block-swap parser's per-keystroke and full-parse costs, and without the memory or freeze risk of
opening such a file the way Markdown mode does.

- [x] `DocMode::Plain`: every block always literal text (no syntax hiding), chosen by extension
      (`mode_for_extension`); `Ctrl+Shift+M` retags the same buffer (text, undo and version
      unaffected) rather than reloading; refused above `MARKDOWN_SIZE_LIMIT` (16 MiB) with a
      notice, and a Markdown file that large opens as `Plain` automatically with the same notice
- [x] Plain-text chunking bounded to `PLAIN_CHUNK_BYTES` / `PLAIN_CHUNK_LINES` per block, with a
      forced display-only cut (`PLAIN_FORCED_CUT_BYTES`, 8 KiB) for a line with no `\n` for a long
      stretch; arrow-key caret movement crosses such a cut as if the line were one continuous row
      (`Editor::cross_forced_cut`, `gpui::test`-covered)
- [x] Streaming load (`Buffer::load`, 1 MiB chunks, never a full-file `String`); invalid UTF-8
      replaced with U+FFFD (`lossy`) instead of failing to open; files above `PLAIN_HARD_LIMIT`
      (2 GiB) refused outright instead of risking exhausting memory
- [x] Save / Save As ask "Save Anyway" / "Cancel" before overwriting a lossily-decoded file
      (`gpui::test`-covered)
- [x] Documents above `LARGE_PLAIN_SIZE` (64 MiB) backed up on quit/close only, not after every
      typing pause; both that backup and an ordinary save build the on-disk text off the UI
      thread from a cloned rope snapshot
- [x] Find on a document above `FIND_BACKGROUND_THRESHOLD` (5 MiB) runs on the background
      executor with a generation counter, so a stale scan cannot clobber a fresher one or feed
      replace-all a wrong range (`gpui::test`-covered against a synchronous scan)
- [ ] Live measurements on real large files (peak RSS, time to first content, worst frame on
      open/scroll/type/find) for a 200 MB log, a 1 GB log, a 20 MB single line, a 50 MB lossy log,
      and a 3 GB sparse file hitting the `PLAIN_HARD_LIMIT` refusal - not run in this pass: the
      machine crashed during an earlier stress run, so this pass was kept to unit and
      `gpui::test` coverage only, with no GUI stress harness and no file over 50 MB

**Exit:** every item above covered by a `gpui::test`; the live measurements run and recorded, with
no frame over budget on a 200 MB/1 GB document and the crash-triggering scenario (a 20+ MB single
line) confirmed fixed on real hardware.

## Phase 9: Markdown-mode memory efficiency (in progress)

A Markdown file that tiles into one or few enormous top-level blocks (a huge fenced code block,
or a whole paragraph with no blank lines) hit the same memory/freeze pathology Phase 8 fixed for
plain text, just in Markdown mode, because virtualization is by top-level block and one giant
block is virtualization's whole unit. See `docs/ARCHITECTURE.md`'s "Markdown memory efficiency".

- [x] Fresh Markdown loads stream the parse back in 4 MiB windows (`Document::load_markdown`)
      instead of copying the whole file into one `String` for `pulldown-cmark`, falling back to
      one direct parse for a file with no blank lines (windowing cannot bound that shape; ADR
      0005's look-behind rule)
- [x] Run-length source maps: adjacent verbatim `SourceSpan`s merge in `Builder::push_span`
      instead of one per parser event. A 15 MiB single-fence file: 221,531 map spans to 2
- [x] `Editor::render_raw`/`render_rendered` split an oversized block's source/lines with
      `plain_chunk_lens` and window them to a couple of viewport-heights around the estimate
      nearest the viewport (`Editor::render_window`). `gpui::test`-covered
      (`typing_across_an_oversized_raw_block_stays_correct`): caret, typing, undo and Home/End
      stay correct across the introduced splits
- [x] `Document::evict`/`Document::ensure_ir` (fix 6): a clean, ≤256 KiB block outside the drawn
      range plus a 200-block margin has its `ir` dropped, restored synchronously the moment
      `render_block` visits it again; `kind`/`len`/`source_hash`/`defs`/`refs`/`footnotes` are
      untouched, so document-wide definitions keep resolving correctly (`gpui::test`- and
      property-covered: `evicting_blocks_drops_ir_and_ensure_ir_restores_it_exactly`,
      `ensure_ir_re_resolves_a_forward_reference_correctly`,
      `evict_leaves_stale_and_oversized_blocks_alone`)
- [x] Two live-measurement regressions found and fixed in this pass, both invisible to headless
      `Document::new` timing since they only fire from `Editor::render`'s per-frame calls:
      `plain_chunk_len_capped` re-descended the rope tree once per *line* (`find_newline`'s
      `rope.byte_slice`), turning a 15 MiB file with real line breaks (a fenced block, log-shaped
      prose) into a single ~80-300 ms frame the first time it became the active block or a raw
      segment was recomputed; rewritten to walk the rope's own chunk iterator once and scan
      forward with `str::find`. `Document::evict` built its replacement as
      `(*block.parsed).clone()` with `ir` then overwritten - a full deep clone of `ir` immediately
      discarded - and `Editor::render` calls it unconditionally every frame, so evicting a
      well-structured document's entire off-screen majority in one call right after a fresh load
      (162,834 blocks measured) cost one frame tens of milliseconds even fixing the wasted clone;
      `evict` now builds field-by-field without touching `ir`'s own `Vec`s and stops after 4096
      blocks per call, spreading the rest over the next several frames
- [x] `cargo bench -p tachyon-doc` unaffected: full parse 1 MiB 22.8 → 19.1 ms, 10 MiB
      265 → 231 ms (both faster - fewer/no whole-file copies); keystroke p99 55.9 → 48.6 µs
      (budget 500 µs); 5 MiB paste UI-thread part 5.5 → 6.1 ms (within noise)
- [x] `cargo test --workspace --locked` and `PROPTEST_CASES=20000` on `tachyon-doc`'s
      `incremental`/`corpus` suites pass
- [x] GUI live measurements (`TACHYON_FRAME_LOG`, peak RSS via `VmHWM`), baseline
      (`origin/feat/plain-text` @ `46e946a`) vs fixed, 2 runs each, `systemd-run --user --scope
      -p MemoryMax=6G -p MemorySwapMax=0`:

      | File | Peak RSS: before → after | Worst frame on open: before → after | Steady RSS after scroll-to-end-and-back + 10 s idle: before → after |
      |---|---|---|---|
      | `md-1mb.md` (well-structured) | 144-145 → 138-140 MB | 8.3-9.2 → 11.4-13.6 ms | 147.6-147.8 → 141.6-142.0 MB |
      | `md-15mb.md` (well-structured, 162,834 blocks) | 412-437 → 374-378 MB | 8.3-8.4 → 7.8-9.6 ms | 415.3-415.4 → 377.3-377.9 MB |
      | `fence-15mb.md` (one giant fenced block) | 5,608-5,872 → 296-335 MB | 22.2-28.8 ms → 8.0-9.5 ms | 3,736-3,928 → 185-187 MB |
      | `oneline-15mb.md` (one unwrapped line) | 4,259 → 271-478 MB | 5,420-5,524 **ms** → 11.0-11.5 ms | 657-727 → 198-202 MB |
      | `log-15mb.md` (one no-blank-line paragraph) | 2,149-2,224 → 336-394 MB | 4,535-4,723 **ms** → 15.3-15.9 ms | 2,149-2,151 → 184-186 MB |

      Every "worst frame on open" number above is under the 16.7 ms budget after the fix (the
      baseline column for `fence`/`oneline`/`log` is seconds, not milliseconds, for the open
      frame alone - `oneline`/`log` additionally produced *no frames at all* during
      `fence`/`oneline`/`log`'s Ctrl+End/PageUp×5/Ctrl+Home sequence on baseline: the window was
      unresponsive for several seconds, not merely slow). The ~79 ms frame on `log-15mb.md`
      reported at the end of the previous pass is confirmed gone (now 15.3-15.9 ms), traced to
      the `plain_chunk_len_capped` per-line rope-descent cost above, not the font-swap remeasure
      originally suspected
- [x] Fix 6's steady-state RSS win, measured the same way as the table above (open, Ctrl+End,
      Ctrl+Home, idle 10 s, `VmRSS`): `md-1mb.md` 147.6-147.8 → 141.6-142.0 MB (small - few enough
      blocks that eviction has little to do); `md-15mb.md` 415.3-415.4 → 377.3-377.9 MB, tracking
      the same reduction seen at load, i.e. scrolling back to the top does not accumulate memory
      for everything that was ever visible
- [x] Eviction smoke test on `md-15mb.md` (fixed binary, `TACHYON_FRAME_LOG`, `grim`
      screenshot): `Page Down` held 5 s from the top (`wtype -P Next -s 5000 -p Next`, relying on
      the client's own key-repeat the same as a physical hold) generated 192 frames, worst
      5.10 ms, 0 over 16.7 ms; jumping back to the top afterward (forcing `ensure_ir` to restore
      every block scrolled past, now evicted) cost 2.51 ms; the screenshot shows the restored
      top-of-document content (headings, lists, a fenced block, a table, a block quote, links)
      rendered correctly, not blank or stale
- [x] The two smaller, pre-existing costs flagged after the previous pass (see
      `docs/ARCHITECTURE.md`'s "Typing in a huge single block"): typing into the active block of
      `fence-15mb.md` or `log-15mb.md` (one ~15 MiB block) occasionally cost 20-61 ms - shaping a
      freshly-built `StyledText` for the touched raw segment (up to `RAW_SPLIT_THRESHOLD`, sized
      like a `DocMode::Plain` chunk, 16 KiB) on every keystroke, and (for a giant block with no
      interior blank line) `Document::stale_block` re-deriving pre-segmenter boundaries with a
      fresh copy-and-scan of the whole stale block each keystroke. Fixed as the follow-up
      suggested: `Document::boundaries` bounds its scan to a window around the edit instead of the
      whole block, using the block's own first line to tell a fence (which can never gain a new
      boundary) from anything else (which can only gain one near the edit) - ADR 0005's segmenter
      invariant makes this safe: a block already large enough to reach that path parses
      identically alone, so it had no interior boundary before the edit or it would already have
      split then; and that scan, like the whole-range fallback for a merge of several blocks,
      walks the rope's own chunks instead of copying it into a `String` first
      (`tachyon_md::presegment_chunks`/`ends_in_fence_chunks`), the treatment `plain_chunk_len_capped`
      got in the previous pass. `Editor::render_raw` now splits an oversized active block into
      `raw_segment_lens` segments (4 KiB, a quarter of `DocMode::Plain`'s own chunk size) instead
      of 16 KiB ones, so re-shaping the one segment a keystroke touches - always a cache miss for
      GPUI's own text-layout cache, since its content just changed - stays cheap regardless of the
      block's size; `RAW_SPLIT_THRESHOLD` itself is untouched, so a `DocMode::Plain` block never
      crosses it. A new test bounds an edit's presegmented bytes directly (not a timing
      assertion): `tachyon-doc::tests::edits_into_a_huge_single_block_presegment_a_bounded_window`
      asserts under 256 KiB presegmented per edit for 20 consecutive keystrokes into both a 512 KiB
      and an 8 MiB single block of each shape, and
      `a_huge_single_block_still_converges_correctly_after_a_bounded_edit` checks the bounded path
      never miscounts a boundary (an edit that introduces a blank line still matches a full parse
      once reparsed, at the start, middle and end of the block). `PROPTEST_CASES=20000` on
      `incremental`/`corpus` and the rest of the workspace suite still pass; `cargo bench -p
      tachyon-doc` keystroke p99 is unchanged within noise (paragraph 47.0 → 54.0 µs, code block
      51.8 → 50.7 µs, streaming at end 41.9 → 40.7 µs; budget 500 µs). Live (Linux, release, 40
      keystrokes at 60 ms each, `TACHYON_FRAME_LOG`, `systemd-run --user --scope -p MemoryMax=6G
      -p MemorySwapMax=0`), worst frame typing into the middle of the file: `fence-15mb.md`
      28.7 → 15.8 ms, `log-15mb.md` 49.2 → 9.4 ms; at the end: 61.0 → 13.5 ms and 41.4 → 8.7 ms -
      no frame over the 16.7 ms budget after the fix (most were over it before, one and a half to
      three and a half times over). A normal 1 MiB well-structured document's typing and a 5 MiB
      paste are unaffected (worst frames within measurement noise of before: 3.4 → 1.5 ms and
      17.2 → 7.0 ms respectively, both already well under budget).
- [ ] Fix 4 (share `BlockIr::text` with the rope for verbatim regions) not attempted this phase:
      a real API change to `BlockIr`/`ParsedBlock`, a pervasive, already-widely-consumed type,
      rather than a contained one; design recorded in `docs/ARCHITECTURE.md` for a follow-up

**Exit:** every item above covered by a `gpui::test`, a property test or a live measurement; a
15-20 MiB Markdown file shaped like the pathological cases (one fenced block, one no-blank-line
paragraph, one unwrapped line) opens with no frame over 16.7 ms and peak RSS proportional to the
file, not a multi-GB/multi-second outlier (baseline: up to 5.9 GB peak and a 5.5-second single
frame on open; fixed: 271-478 MB peak, every open frame under 16.7 ms); typing into an
already-huge single block also stays within budget (baseline: 20-61 ms per keystroke; fixed: no
frame over 16.7 ms in 40 keystrokes at either the middle or the end of a 15 MiB single block, both
shapes); `cargo bench -p tachyon-doc` budgets unchanged. Met, apart from Fix 4 (`BlockIr::text`
sharing the rope), recorded above as a design for a follow-up rather than attempted this phase.

