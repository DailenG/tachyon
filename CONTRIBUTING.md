# Contributing to Tachyon

Tachyon's reason to exist is speed: launch in under 50 ms and never drop a frame while typing or
pasting. Every rule below protects that or keeps the code maintainable once it grows past one
person's memory.

## Setup

1. Install the platform prerequisites from the [README](README.md#building). `rustup` picks up the
   pinned toolchain from `rust-toolchain.toml`.
2. Enable the repository hooks (runs `cargo xtask ci` before each push):

   ```sh
   git config core.hooksPath .githooks
   ```

3. Optional, for the dependency check that CI enforces: `cargo install cargo-deny --locked`.

## Workflow

- `main` is protected: all changes land through pull requests with a green **CI result** check.
- Branch names: `feat/…`, `fix/…`, `perf/…`, `refactor/…`, `docs/…`, `ci/…`, `chore/…`.
- PRs are squash-merged, so the **PR title** becomes the commit on `main`. Use
  [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) with the crate as scope:
  `feat(platform): forward launches over a named pipe`, `perf(md): reuse the parse buffer`.
  Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`.
- Keep PRs to one concern. Mechanical changes (renames, formatting) go in their own PR.
- Update `CHANGELOG.md` under **Unreleased** for anything a user could notice.

## Required checks

`cargo xtask ci` runs what CI requires: `cargo fmt --check`, clippy with `-D warnings`, tests, and
`cargo deny check`. CI additionally builds on Windows, Linux and macOS and uploads a Windows
release binary as an artifact.

Windows is the primary platform. On Linux or macOS, type-check it before pushing platform code:

```sh
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

## Architecture rules

These are enforced in review; [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains why.

- **GPUI stays at the edge.** Core crates (`tachyon-text`, `tachyon-md`, `tachyon-doc`) must not
  depend on `gpui`. They are tested headless and compile in seconds.
- **Incremental equals full.** Changes to segmentation or reparsing must keep the property tests in
  `crates/tachyon-doc/tests/incremental.rs` green at `PROPTEST_CASES=1000000`, and include
  `cargo bench -p tachyon-doc` numbers ([ADR 0005](docs/adr/0005-incremental-reparse-by-block-windows.md)).
- **Platform code is quarantined.** `#[cfg(target_os = …)]` / `#[cfg(windows)]` only in
  `tachyon-platform` and `crates/tachyon/src/main.rs`. Each backend exposes the same functions;
  no trait objects for OS dispatch.
- **The UI thread never blocks.** No file, network or IPC I/O and no unbounded work on the main
  thread. Use GPUI's background executor and hand back owned results.
- **No new startup work.** Anything not needed for the first frame runs after it. Changes on the
  startup path include `cargo xtask bench-startup` numbers before and after in the PR.
- **GPUI is pinned.** Bumping the Zed revision is its own PR following
  [ADR 0001](docs/adr/0001-pin-gpui-to-a-zed-revision.md).

## Code rules

- `unsafe` requires a `// SAFETY:` comment stating the invariant (clippy
  `undocumented_unsafe_blocks` is denied), one unsafe operation per block.
- No `unwrap()` outside `#[test]` code (clippy `unwrap_used` is denied). `expect("…")` only for
  true invariants, with the invariant as the message. User input (files, pasted text, IPC messages)
  must never cause a panic.
- Errors reach the user or stderr with context; never swallow them silently. Degrade instead of
  failing when a launch can still succeed (see the single-instance fallback).
- Hot paths (keystroke, paste, layout, paint) do not allocate per event without a measured reason.
- No `dbg!`, `todo!` or `unimplemented!` (denied by lints). No commented-out code.
- New dependencies need a reason in the PR. Prefer std; core crates stay small. `cargo deny` must
  pass (licenses must be compatible with MIT OR Apache-2.0; no copyleft).

## Tests

- Test behaviour a user or caller would notice: boundaries, invariants, error paths, state
  transitions. The central invariant of the editor will be *incremental parse == full parse*,
  checked with property tests.
- Do not test wording, wiring, or that a function forwards its arguments.
- Tests must be deterministic and safe to run in parallel with the rest of the suite (unique names
  for IPC endpoints, temp directories, no shared global state).
- UI and platform changes also need a manual run of the app; say what you exercised in the PR.

## Architecture decisions

Changes to architecture, invariants, budgets or platform strategy get an ADR in
[`docs/adr/`](docs/adr/) (copy `0000-template.md`, next free number). Superseded ADRs are kept and
marked, never deleted.

## Releases

Versions follow SemVer, starting at `0.x` while the editor is incomplete. A release moves the
**Unreleased** changelog section under a version heading, bumps `version` in the workspace
`Cargo.toml`, and tags `vX.Y.Z` on `main`. Pushing that tag runs
[`.github/workflows/release.yml`](.github/workflows/release.yml), which builds `cargo xtask dist`
on Windows, Linux and macOS and publishes a GitHub Release with the three archives, the MSIX
installer and its `.appinstaller` (Windows only, see below), a `SHA256SUMS.txt`, and notes taken
from the matching `CHANGELOG.md` section; the tag must match the workspace version or the
workflow fails before building. `workflow_dispatch` runs the same build as a dry run (artifacts
uploaded, no release), useful for checking the packaging steps before cutting a tag; its optional
`msix_version` and `appinstaller_base` inputs let that dry run produce a signed MSIX with a
different version and a different `.appinstaller` target, for testing that App Installer offers
an update. The Windows build is signed with Azure Trusted Signing, which needs the
`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TRUSTED_SIGNING_ACCOUNT`,
`AZURE_TRUSTED_SIGNING_PROFILE` and `AZURE_TRUSTED_SIGNING_ENDPOINT` repository secrets; without
them the workflow warns and ships an unsigned `tachyon.exe` (and MSIX) instead of failing.

**MSIX.** `cargo xtask msix` (Windows only; needs `makeappx.exe` from the Windows 10/11 SDK)
packs the signed `tachyon.exe` and `packaging/msix/Assets` (regenerated by `cargo xtask icons`,
see below) with `packaging/msix/AppxManifest.xml` into `target/dist/Tachyon_<version>_x64.msix`,
and writes `target/dist/Tachyon.appinstaller` pointing App Installer at it. The manifest's
`Publisher` must match the Azure Trusted Signing certificate's subject exactly, or the MSIX will
not install; if that certificate is ever reissued with a different subject, update
`packaging/msix/AppxManifest.xml` in the same PR as whatever changed it.

