# 0011: The Claude Design system is the design source, and brand assets are traced from masters

- **Status:** Accepted
- **Date:** 2026-10-02

## Context

By 1.2.0 the shipped brand had drifted visibly from the owner's concept. The logo files in
`assets/brand/` were hand-drawn Bézier approximations of a raster concept, with the wordmark set
in DejaVu Sans Bold and sheared to look italic. At 16-32 px the icon used a cropped glyph that
read as a "3", on a slate-bordered navy tile, and the `.ico` stopped at 64 px, so Explorer's large
views and Alt+Tab upscaled it. Each step had seemed reasonable, but no step compared the result
with the concept, and the design rules lived only in prose (`docs/design/DESIGN_DIRECTION.md`)
that described one early concept image.

The owner has since built a design system in Claude Design ("Tachyon Design System") from six
concept sheets and three master logo files, and decided (2026-10-02) that it is the design to use.
Most contributors and agents cannot reach Claude Design: subagents spawned by a session do not
get the DesignSync tool, and a human contributor may have no account.

## Decision

1. **The Claude Design project is the visual source of truth.** A read-only, verbatim copy of its
   README and tokens lives in `docs/design/system/`, with `SNAPSHOT.md` recording when it was
   copied, how to refresh it (only from a session with Claude Design access, one way, never
   writing back), and every place the repository deliberately departs from it.
   `DESIGN_DIRECTION.md` keeps the speed rules, which decide those departures. Where the two
   disagree, the speed rules win on speed and the design system wins on everything else.
2. **Logo and icon files are generated, never drawn.** `scripts/brand/trace.py` traces the owner's
   flat-navy masters (`docs/design/concepts/masters/`) with potrace, renders each trace back, and
   refuses to write it if more than 0.5 % of the inked pixels differ from the master.
   `scripts/brand/fit_gradient.py` measures the symbol's gradient from the gradient master.
   `scripts/brand/compose.py` builds every colorway, lockup and icon size from the two traces,
   each constant citing a design-system token or a measurement of a concept sheet.
   `cargo xtask icons` rasterizes the icons. The application build only embeds those results.
3. **Each icon size has art drawn on its own pixel grid** (16, 20, 24, 32, 40, 48, 64 and a
   256-unit grid), and every raster is rendered from the smallest art at least that large.
4. **`tachyon.ico` holds 32-bit DIB images from 16 to 128 px plus one 256 px PNG.** The tray and
   window icons are created only from the DIBs (`ico_image` skips the PNG), so creating an icon
   still never needs an image codec. Explorer uses the PNG.

## Consequences

- The brand can be regenerated exactly, and checked: a trace that drifts from its master fails
  loudly instead of shipping.
- Changing the logo now means the owner supplies a new master. Nobody, human or agent, can
  "touch up" a path, and the README and AGENTS.md say so.
- The snapshot can go stale. `SNAPSHOT.md` dates it, and a pull request that relies on something
  it does not cover says so, rather than guessing from concept images.
- The executable grows by 272 KB (10.00 MB to 10.28 MB, measured on the CI release build). The
  `.ico` goes from 43 KB to 179 KB, and the binary holds it twice: once as the executable's icon
  resource (`crates/tachyon/build.rs`, for Explorer) and once in the tray code
  (`include_bytes!` in `tray.rs`). Of the 136 KB per copy, 105 KB is the 96 and 128 px DIBs
  (sharp window icons at 300 and 400 % scaling) and 31 KB the 256 px PNG. The icon is not read
  on the startup path beyond what Windows already did for the old one. If size matters more, the
  tray could load its icons from the executable's own resource instead of a second copy, which
  would halve the growth.
- Regenerating needs potrace, ImageMagick, rsvg-convert, Python 3 and numpy, on a maintainer's
  machine only; CI and the normal build need none of them.
- Revisit if the owner commissions hand-built vector masters (for example from a designer): they
  would replace the traced files and `trace.py` would be retired, with `compose.py` unchanged.
