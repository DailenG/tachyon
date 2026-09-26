# Agent instructions

Rules for AI coding agents working in this repository. Humans follow the same rules via
[CONTRIBUTING.md](CONTRIBUTING.md), which is authoritative if the two ever disagree.

## Project

Tachyon: native Markdown editor in Rust on GPUI (Zed's UI framework). Windows first, then Linux and
macOS. Headline requirement: launch to first frame in under 50 ms; never drop frames on keystrokes
or large pastes. Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Current phase and exit
criteria: [docs/ROADMAP.md](docs/ROADMAP.md). Decisions: [docs/adr/](docs/adr/).

## Layout

- `crates/tachyon`: binary: CLI, single-instance claim, startup sequencing, windows.
- `crates/tachyon-platform`: OS integration GPUI lacks (single-instance IPC). The only crate besides
  `main.rs` allowed to contain `#[cfg(target_os)]`/`#[cfg(windows)]` code.
- `crates/tachyon-text`: rope buffer, edit log, undo, UTF-16 mapping. No GPUI dependency, ever.
- `crates/tachyon-md`: Markdown → blocks with owned render IR. No GPUI dependency, ever.
- `crates/tachyon-doc`: document state and incremental reparse (`ParseJob`s). No GPUI dependency.
- `xtask`: `cargo xtask ci` (required checks), `cargo xtask bench-startup` (startup budget).
- `crates/tachyon-editor`: GPUI editor view (block swap, rendering, theme, input/IME). Behaviour
  tests run headless with `gpui::test`.

## Commands

```sh
cargo xtask ci                     # must pass before you claim a change is done
cargo run -- --startup-report      # needs a display; prints startup milestones then exits
cargo xtask bench-startup          # required evidence for any startup-path change
cargo bench -p tachyon-doc         # reparse latency; required evidence for parser/doc changes
PROPTEST_CASES=1000000 cargo test --release -p tachyon-doc --test incremental
                                   # run before merging any change to segmentation or reparse
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
                                   # required after touching Windows code from a non-Windows host
```

## Invariants

- Core crates never depend on `gpui`. Background work receives immutable snapshots and returns owned
  data; GPUI entities and text shaping stay on the main thread.
- Nothing is added to the startup path before the first frame unless the bench proves it fits.
- GPUI is pinned to one Zed commit in `Cargo.toml`, together with `async-task` and `calloop` patches
  that must match that commit's workspace. Never bump casually; follow ADR 0001.
- `unsafe` blocks: one operation each, preceded by `// SAFETY:` explaining why it holds.
- User input (files, pastes, IPC messages) must not panic the process.
- Every block must parse identically alone and inside the document; incremental reparse depends on
  it (ADR 0005). The property tests in `crates/tachyon-doc/tests/incremental.rs` are the arbiter.

## Working rules

- Verify behaviour by running the app, not only by compiling. On Linux, GPUI needs
  `WAYLAND_DISPLAY` or `DISPLAY`; without one the binary exits with an error by design.
- Do not run the bench or release builds casually: fat LTO takes minutes.
- Update `CHANGELOG.md` (Unreleased) for user-visible changes and write an ADR for architectural
  ones. Keep docs in sync with code in the same change.
- Never commit generated artifacts, screenshots, or scratch files. Never force-push `main`.
