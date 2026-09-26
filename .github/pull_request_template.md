## What and why

<!-- The problem this solves and the approach taken. Link issues: Fixes #123 -->

## Verification

<!-- What you ran and observed. "Tests pass" alone is not verification for UI or perf changes. -->

- [ ] `cargo xtask ci` passes locally
- [ ] Exercised the changed behaviour by running the app (UI/platform changes)
- [ ] Startup-affecting change: `cargo xtask bench-startup` before/after numbers below

## Checklist

- [ ] `CHANGELOG.md` updated under **Unreleased** (user-visible changes)
- [ ] Docs/ADRs updated if architecture, invariants or budgets changed
- [ ] New `unsafe` has a `// SAFETY:` comment stating the invariant
- [ ] Windows behaviour considered (primary platform)
