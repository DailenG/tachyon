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
  shows rendered rich text.
- **Platforms:** Windows first, then Linux and macOS.

> **Status:** early. Block-swap editing and saving work (the block under the caret is raw
> Markdown, the rest is rendered), on top of an incremental parser. The startup budget is met by
> resident mode (default on Windows), and Phase 3's paste and typing criteria are met on
> the reference Windows machine. Phase 4 added find and replace, a light theme, zoom, links you
> can follow with `Ctrl+click` and a Windows tray icon. See [ROADMAP](docs/ROADMAP.md).

## Performance budgets

| Budget | Target | Measured by |
|---|---|---|
| Launch → first frame | p95 < 50 ms | `cargo xtask bench-startup` |
| Keystroke reparse, 1 MB document | p99 < 0.5 ms | `cargo bench -p tachyon-doc` |
| Paste of 5 MB | no frame over 16.6 ms | frame-time overlay (Phase 3) |

Budgets are requirements, not goals: a change that regresses one needs a recorded decision
([ADRs](docs/adr/)).

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
```

`tachyon --help` lists all options. Settings live in `settings.toml` (`Ctrl+,` opens it): theme,
zoom of new windows, and whether Quit keeps unsaved documents.

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
