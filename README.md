# Tachyon

[![CI](https://github.com/DailenG/tachyon/actions/workflows/ci.yml/badge.svg)](https://github.com/DailenG/tachyon/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A GPU-accelerated native Markdown editor, built for reading and editing pasted LLM output. It aims to
start as fast as a scratchpad and edit like Typora: one pane, inline WYSIWYG, no WebView.

- **Rendering:** [GPUI](https://www.gpui.rs/) (Zed's UI framework): DirectX on Windows, Vulkan via
  wgpu on Linux, Metal on macOS.
- **Parsing:** [`pulldown-cmark`](https://github.com/pulldown-cmark/pulldown-cmark), reparsed
  incrementally per block.
- **Editing model:** *block swap*. The block under the cursor shows raw Markdown; every other block
  shows rendered rich text. Files that aren't Markdown by extension open in **plain-text mode**
  instead (no syntax hiding at all); `Ctrl+Shift+M` toggles a document between the two.
- **Platforms:** Windows first, then Linux and macOS.
- **Website:** <https://daileng.github.io/tachyon/>, published from `site/` by the Pages
  workflow.

> **Status:** 1.0. Block-swap editing, incremental parsing, everyday editing (find/replace,
> themes, zoom, links, the command palette), writing comfort (lists, code highlighting, hot exit,
> Copy as HTML), files and settings, distribution, large-file stability and Markdown-mode memory
> efficiency are done (Phases 1-9), each held to its measured budget. What remains is owner-only
> (macOS signing and notarization) or a documented measurement follow-up - see
> [ROADMAP](docs/ROADMAP.md). Download it from the
> [latest release](https://github.com/DailenG/tachyon/releases/latest).

## Highlights

- **Block swap:** the block under the caret shows raw Markdown; every other block is rendered
  rich text. `Ctrl+Shift+M` toggles a document into plain-text mode instead, for files that
  aren't Markdown.
- **Large files stay responsive:** a 200 MB log opens in about 400 MB of memory with the window
  responsive throughout; a 1 GB log
  opens in about 1.4 GB and stays responsive; files over 2 GiB are refused with a message rather
  than crashing.
- **Markdown memory tracks the text, not the worst block:** a 15 MiB Markdown file shaped like
  one giant fenced code block used to peak near 5.7 GB; it now peaks around 300 MB.
- **Speed, measured:** launch into the resident instance is p95 20-35 ms on the reference Windows
  machine (Core Ultra 7 155H, Intel Arc, 4K; a cold direct launch is 300-450 ms, which is why
  resident mode is the default on Windows); a keystroke paints within one frame (reparse p99
  about 45-55 µs against a 0.5 ms budget); a 5 MB paste never drops a frame (worst 5-9 ms).
- **Signed Windows builds:** `tachyon.exe` is signed with Azure Trusted Signing; an optional MSIX
  install auto-updates itself from GitHub Releases.
- **Everyday editing:** find and replace (Replace All is one undo step), light and dark themes
  following the system or a setting, zoom, `Ctrl+click` links, list continuation and `Tab`
  nesting, syntax highlighting in fenced code (Rust, Python, JS/TS, C family, Go, Java, C#,
  shells, PowerShell, SQL, JSON, TOML, YAML), a command palette, Go to heading, quick open, Open
  recent, hot exit, Copy as HTML, local images, external file change reload, a settings file, and
  math.

## Keyboard shortcuts

`Ctrl` is `Cmd` on macOS unless noted.

| Action | Shortcut |
|---|---|
| New window | `Ctrl+N` |
| Open | `Ctrl+O` |
| Save / Save As | `Ctrl+S` / `Ctrl+Shift+S` |
| Close window / Quit | `Ctrl+W` / `Ctrl+Q` |
| Command palette | `Ctrl+Shift+P` |
| Settings | `Ctrl+,` |
| Open recent / Quick open | `Ctrl+R` / `Ctrl+P` |
| Find | `Ctrl+F` |
| Find next / previous | `F3` / `Shift+F3` (also `Ctrl+G` / `Ctrl+Shift+G`) |
| Replace | `Ctrl+H` (`Cmd+Alt+F` on macOS) |
| Replace all, in the replace bar | `Ctrl+Enter` |
| Go to heading | `Ctrl+Shift+O` |
| Copy as HTML | `Ctrl+Shift+C` |
| Toggle plain-text mode | `Ctrl+Shift+M` (same on every platform) |
| Zoom in / out / reset | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` |
| Frame-time overlay | `Ctrl+Alt+F` (same on every platform) |
| Undo / redo | `Ctrl+Z` / `Ctrl+Shift+Z` |
| Select all | `Ctrl+A` |
| Document start / end | `Ctrl+Home` / `Ctrl+End` |
| Leave editing (click, type or move the caret to resume) | `Escape` |

## Performance budgets

| Budget | Target | Measured by |
|---|---|---|
| Launch → first frame | p95 < 50 ms | `cargo xtask bench-startup` |
| Keystroke reparse, 1 MB document | p99 < 0.5 ms | `cargo bench -p tachyon-doc` |
| Paste of 5 MB | no frame over 16.6 ms | frame-time overlay (Phase 3) |

Budgets are requirements, not goals: a change that regresses one needs a recorded decision
([ADRs](docs/adr/)).

## Install

Prebuilt archives for Windows, Linux and macOS are attached to each
[release](https://github.com/DailenG/tachyon/releases/latest): a `.zip` on Windows, a `.tar.gz`
elsewhere, plus a `SHA256SUMS.txt` to verify the download. Extract it and run `tachyon`
(`tachyon.exe` on Windows); no installer or separate runtime is required. `tachyon.exe` is signed
with Azure Trusted Signing; macOS builds are not signed or notarized yet, so Gatekeeper will warn
on first launch.

**Windows, with automatic updates:** download `Tachyon.appinstaller` from the
[latest release](https://github.com/DailenG/tachyon/releases/latest) and open it (or
`Add-AppxPackage -AppInstallerFile Tachyon.appinstaller` from PowerShell), then follow the prompt
to install. This registers Tachyon as an MSIX package (also signed with Azure Trusted Signing)
that checks for an update every time it launches and applies it in the background; a resident
Tachyon (the default) picks up the update the next time it fully quits (tray icon → "Quit
Tachyon", or `tachyon --quit`) or at the next sign-in. `tachyon` also works from any terminal, and
opening a `.md` or `.markdown` file, or a `.txt`, `.text` or `.log` file, offers Tachyon in "Open
with". Launches of a packaged app go through Windows' package activation, which adds about
35-40 ms per launch on the reference machine (p50 about 59 ms instead of about 21 ms), so the
`.zip` above is the faster choice if you would rather manage updates yourself. Uninstalling the
package does not remove the autostart entry if you turned it on; run `tachyon --autostart off`
first.

**Nightly builds:** every merge to `main` publishes a signed MSIX to a rolling
[`nightly`](https://github.com/DailenG/tachyon/releases/tag/nightly) prerelease. Install once from
[`Tachyon.appinstaller`](https://github.com/DailenG/tachyon/releases/download/nightly/Tachyon.appinstaller)
the same way as above; it then updates itself on every launch. An already-installed stable MSIX
(`1.0.0.0`) updates in place the same way, no uninstall needed, because a nightly's version is
always higher than the stable release it followed. There is no path back from nightly to the
stable release with that same version without uninstalling first (it would be a downgrade); switch
channels only if you want a tested, versioned build instead of main's latest commit.

## Building

Requires the toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml); `rustup` installs it
automatically on first build.

**Windows:** Visual Studio 2022 Build Tools with the *Desktop development with C++* workload and a
Windows 10/11 SDK. Release builds compile HLSL shaders with the SDK's `fxc.exe`.

**Linux (Debian/Ubuntu):**

```sh
sudo apt-get install libxkbcommon-x11-dev libwayland-dev libxcb1-dev libx11-xcb-dev \
  libfontconfig-dev libfreetype-dev libvulkan-dev libzstd-dev
```

Arch: `sudo pacman -S libxkbcommon-x11 wayland libxcb fontconfig freetype2 vulkan-icd-loader zstd`.

**macOS:** Xcode command-line tools.

```sh
cargo run                          # scratch window (debug)
cargo run --release -- notes.md    # open a file
cargo run --release -- --paste     # open the clipboard contents
```

A second launch forwards its files to the running instance and exits; pass `-n` to force a separate
process. On Windows the instance stays running after its last window closes, so later launches open
at once (`--no-resident` turns that off; on Linux and macOS it is opt-in with `--resident`):

```sh
tachyon --autostart on   # start in the background at login
tachyon --status         # is an instance running? does it start at login?
tachyon --quit           # end it (unsaved documents come back next time)
tachyon --desktop-entry on   # Linux: add it to the launcher and "Open with" menus
```

`cargo xtask dist` builds a release archive in `target/dist/`. `tachyon --help` lists all options. Settings live in `settings.toml` (`Ctrl+,` opens it): theme,
zoom of new windows, whether Quit keeps unsaved documents, and whether a faint rotating tip shows
behind the document.

## Development

```sh
cargo xtask ci              # fmt, clippy, tests, cargo-deny: what CI requires
cargo xtask bench-startup   # release build + startup latency vs. the 50 ms budget
cargo xtask bench-startup --warm   # launches handed to a resident instance
```

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR. Architecture:
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Decisions: [docs/adr/](docs/adr/).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
Tachyon by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
