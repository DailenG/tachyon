# Tachyon concept references

The owner's concept artwork, supplied 2026-10-01. The concept sheets show the intended look; the
masters are the logo itself. Neither is a runtime asset: never load anything in this directory
from the app or the website. The rules drawn from these images are in the design system snapshot,
[`../system/README.md`](../system/README.md), and where the repository adapts them, in
[`../system/SNAPSHOT.md`](../system/SNAPSHOT.md).

## Masters (`masters/`)

The production logo is traced from these files (`scripts/brand/trace.py`; see
[`assets/brand/README.md`](../../../assets/brand/README.md)). They are kept lossless and
byte-for-byte as supplied apart from metadata stripping and recompression, which leave every
pixel identical. Do not edit, crop, or re-export them: the traces are checked against them pixel
for pixel.

| File | What it is |
| --- | --- |
| `masters/symbol-navy.png` | The symbol, flat navy on white, 1254 × 1254. Trace source for the symbol's shape |
| `masters/symbol-gradient.png` | The same symbol in its gradient, 1254 × 1254. Source of the gradient's colors (`scripts/brand/fit_gradient.py`) |
| `masters/wordmark-navy.png` | The italic wordmark, flat navy on white, 1254 × 1254. Trace source for the wordmark |

## Concept sheets

Stored as WebP at quality 90 (the originals are 1.1-1.3 MB PNGs each; these are references, not
trace sources). They inform the visual language. The landing-page and hero sheets show a mock-up
editor with a sidebar, preview pane and toolbar: those are **not** features. Tachyon stays a
single-pane block-swap editor (ADR 0002).

| File | What it shows | Used for |
| --- | --- | --- |
| `sheet-1-brand-identity.webp` | Stacked and horizontal logos, symbol, monochrome logos, palette, type | Lockup proportions (`scripts/brand/compose.py`), palette tokens |
| `sheet-2-app-icon.webp` | Light and dark app icon tiles, monochrome tile, 16-128 px sizes | App icon (dark tile chosen), simplified 16-32 px mark |
| `sheet-3-landing-light.webp` | Light landing page: header, hero, download buttons, feature row | Website layout and tone |
| `sheet-4-hero-dark.webp` | Dark hero / release artwork | Release and social artwork |
| `sheet-5-about-window.webp` | About window: stacked logo, tagline, divider, info rows, buttons | The About window |
| `sheet-6-ui-elements.webp` | Buttons, badges, pills, feature icons, loaders, dividers, patterns, decorative accents | Website components |

`tachyon-logo-concept.png` is the earlier single-logo concept (PR #41), superseded by the sheets
above and kept for history.
