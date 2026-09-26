# 0001: Pin GPUI to a Zed revision

- **Status:** Accepted
- **Date:** 2026-09-25

## Context

GPUI lives in the Zed monorepo. The crates.io release (`gpui` 0.2.2) predates the split into
`gpui_platform` and per-OS crates (`gpui_windows`, `gpui_linux`, `gpui_macos`, `gpui_wgpu`), so it
lags the code Zed itself ships. The API changes between revisions without deprecation periods.

A git dependency does not inherit the upstream workspace's `[patch.crates-io]` table. Zed patches
two crates in GPUI's graph (`async-task`, `calloop`); without mirroring them we would build GPUI
against code Zed does not test.

## Decision

- Depend on `gpui` and `gpui_platform` from `https://github.com/zed-industries/zed` at one exact
  `rev`, declared once in `[workspace.dependencies]`.
- Mirror the Zed patches GPUI needs (`async-task`, `calloop`) in our `[patch.crates-io]`, pinned to
  the revisions Zed uses at that commit.
- Dependabot ignores these four dependencies.

Bumping is a dedicated PR that:

1. Updates every `rev` together and re-syncs the mirrored patches from Zed's root `Cargo.toml` at
   the new commit (add any new patch GPUI now needs).
2. Passes `cargo xtask ci` and the Windows cross-check.
3. Includes `cargo xtask bench-startup` numbers before and after.
4. Runs the app on Windows and Linux: window, text, file open, handoff.

## Consequences

- Reproducible builds and deliberate upgrades; API churn is absorbed in one place at a time.
- The first build clones the Zed repository (large) into the cargo git cache.
- We must watch Zed's patch table on every bump; a missed patch shows up as a build failure or,
  worse, subtly different executor behaviour.
- Revisit if GPUI starts publishing current releases to crates.io.
