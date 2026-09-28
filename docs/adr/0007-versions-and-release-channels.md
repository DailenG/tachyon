# 0007: Versions and release channels

- **Status:** Accepted
- **Date:** 2026-09-28

## Context

The MSIX manifest and `Tachyon.appinstaller` (ADR-less so far, added alongside Phase 7's
distribution work) both carry a 4-part `X.Y.Z.W` version, and App Installer's only update rule is
"strictly higher `Version` wins". Until now every CI build produced `W = 0` (`cargo xtask msix`'s
default: the workspace version plus a trailing `.0`), because no release had shipped yet and
nothing needed a second, in-between build. That is a problem the moment there is an installed
MSIX at all: the project owner cannot install a newer main build over it to try a fix without
uninstalling first, since `0.1.0.0` never compares less than another `0.1.0.0`. Fixing that means
two things: a version rule with room for more than one build per product version, and a workflow
that actually produces one of those in-between builds on every merge, signed and published
somewhere App Installer can reach.

## Decision

**Product version.** `[workspace.package] version` in `Cargo.toml` (SemVer). While the project
stays `0.x`: bump the minor version for a user-visible feature, the patch version for a fix only.
A release PR bumps this version, moves the `CHANGELOG.md` `Unreleased` section under a new
`## [X.Y.Z]` heading in the same commit, and the pushed tag is `vX.Y.Z`, matching that version
exactly (`release.yml`'s `check-version` job already enforces the match).

**MSIX version.** `X.Y.Z.B`, where `X.Y.Z` is the product version above and `B` is a build number:

- **Stable** (`release.yml`, a `vX.Y.Z` tag push): `B = 0`, unchanged from today. `cargo xtask
  msix` run with no `--version` override already produces exactly this (workspace version plus a
  trailing `.0`), so `release.yml` needed no change; see the comment above its "Package the MSIX
  installer" step.
- **Nightly** (`nightly.yml`, every push to `main` that touches packaged code): `B` is that
  workflow's own `github.run_number` for the run. GitHub's run number for one workflow file only
  ever increases (a rerun of the same run keeps its number; a new run gets the next one), so
  nightlies are strictly ordered: nightly build 42 is always newer than nightly build 41. `B` is
  capped well under the 4-part version's `UInt16` limit (65535 per part; a released app would be
  years from that count of merges to `main`, and `nightly.yml`'s paths filter only counts merges
  that touch packaged code); if it were ever reached, the fix is a new major/minor product
  version, which resets nothing about `B` today but starts a fresh comparison range because `X.Y`
  is higher.

**Ordering consequence, by construction:**

- `X.Y.Z.<n>` (nightly) > `X.Y.Z.0` (stable of the same product version), for any `n >= 1`: a
  nightly always looks newer than the stable release it was built after.
- `X.Y.(Z+1).0` (a later stable release) > `X.Y.Z.<n>` (any nightly of the prior patch version):
  cutting a real release always overtakes every nightly that preceded it, because the first three
  parts already differ.
- Consequently, an install that tracks the nightly channel never gets *older* automatically: it
  either stays on a newer nightly, or jumps to a newer stable release. But it can never move
  *sideways* to the stable release that matches its own `X.Y.Z` once a nightly of that version has
  installed (e.g. nightly `0.1.0.7` next to stable `0.1.0.0`): App Installer will not offer that
  as an "update" since it is not a higher version, so switching from nightly back to that specific
  stable needs an uninstall first. This is the one deliberate asymmetry the scheme accepts, in
  exchange for making the common case (nightly always progresses) automatic.

**Nightly channel (`nightly.yml`).** A new workflow, Windows-only, triggered by every push to
`main` touching `crates/**`, `xtask/**`, `packaging/**`, `Cargo.{toml,lock}` or the workflow
itself, plus `workflow_dispatch`. It builds and signs `tachyon.exe` the same way `release.yml`
does (same pinned `Azure/trusted-signing-action`, same secret-presence check, same
`verify-signature.ps1`), packs `cargo xtask msix --version X.Y.Z.<run_number> --appinstaller-base
https://github.com/DailenG/tachyon/releases/download/nightly`, signs and verifies the MSIX, then
publishes to a single rolling prerelease tagged `nightly`: the tag is force-moved to the new
commit, the release is created if it does not exist yet or edited if it does, the previous
nightly's differently-named `.msix` asset is deleted, and the new `.msix`, `Tachyon.appinstaller`
and `SHA256SUMS.txt` are uploaded with `--clobber`. Publishing (not just signing) is skipped, with
a warning, when the Azure secrets are absent, since an unsigned build auto-updating installed
Tachyons silently would be worse than not publishing.
`GITHUB_TOKEN`'s `contents: write` is scoped to that publish job only.

Stable's `.appinstaller` still points `Uri` at
`releases/latest/download/Tachyon.appinstaller` (`cargo xtask msix`'s default), and GitHub's
`releases/latest` API/redirect excludes prereleases by construction, so the `nightly` prerelease
never becomes what a stable install's App Installer resolves against; stable and nightly are
fully independent update streams pointed at by two different, fixed URLs
(`releases/latest/download/…` and `releases/download/nightly/…`).

## Consequences

- Every merge to `main` (that touches packaged code) produces a strictly newer signed MSIX than
  the last nightly and than the current stable release, with no manual version bump: exactly the
  "install a newer test build over the old one" the project owner needed.
- A nightly install can only move forward or to a newer stable release, never back to the stable
  release with the same `X.Y.Z` it has already overtaken; that specific downgrade needs an
  uninstall. Documented in the README so it is not a surprise.
- Nightly and stable share one Azure Trusted Signing identity and one MSIX `Identity` (`Name`,
  `Publisher`), intentional: a nightly install and a stable install of Tachyon are the same
  package family and either can be the one already on a machine.
- If `nightly.yml`'s run number ever approached 65535 (it will not for a very long time at one
  run per qualifying merge), the fix is to cut a stable release, which resets the comparison range
  by bumping `X.Y.Z`, not `B` itself.
- Revisit if App Installer ever exposes an explicit channel/ring concept, or if a second
  auto-updating channel (e.g. "beta") is needed: this ADR's ordering argument generalizes to N
  channels only if each is given its own always-increasing `B` source and its own fixed URL, same
  as nightly's `run_number` and `/releases/download/nightly/` here.
