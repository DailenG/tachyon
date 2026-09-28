# Tachyon brand assets

These are original SVG constructions informed by the selected conceptual logo in `docs/design/concepts/tachyon-logo-concept.png` (PR #41). The particle, orbit, and tapered wake were drawn as new Bézier shapes; the raster was not auto-traced. These assets are for build-time icon generation and external brand surfaces. **Do not load, parse, rasterize, or draw them inside the editor window or on its startup path.** The application's `.ico` and launcher SVG in `crates/tachyon-platform/assets/` are generated from `app-icon-small.svg` (16–32 px, the same art as the website favicon) and `app-icon.svg` (40–64 px) by `cargo xtask icons`; the build only embeds those results.

## Files and use

| Asset | Intended use | Minimum rendered size |
| --- | --- | --- |
| `mark.svg` / `mark-flat.svg` | Full particle/orbit mark, gradient or one solid brand blue | 64 px wide |
| `mark-small.svg` | Simplified, solid mark for compact surfaces | 16 px wide; reviewed at 16, 20, 24, 32 px |
| `mark-mono.svg` | Single `currentColor` tintable mark | 24 px wide; use the small mark geometry if smaller |
| `app-icon.svg` / `app-icon-flat.svg` | Square icon on a bounded dark tile | 48 px square |
| `app-icon-small.svg` | Square icon with simplified mark | 32 px square |
| `wordmark.svg` | Outlined wordmark; never relies on a runtime font | 120 px wide |
| `lockup-horizontal.svg` / `lockup-horizontal-flat.svg` | Mark plus outlined wordmark | 240 px wide |

The gradient assets have flat SVG fallbacks with the same geometry. Full-size icons have **two** gradient stops, violet `#6645E8` to cyan `#21C8ED`; the flat mark is `#396BF2`. The tile is near-black navy `#101A2D` with a solid `#6E879F` edge for visibility on dark and light taskbars. The wordmark is `#101A2D`. Tiny tile marks use solid `#8CCBFF`. The gradient is confined to artwork, never editor UI.

## Clear space and constraints

- Keep a clear zone at least one quarter of the mark's height around the full mark and lockup. For square icons, keep the authored tile padding and do not crop it.
- Use the complete full mark only above its minimum size. Use the simplified mark (`app-icon-small.svg`) at 16–32 px. Inspect target rasterizations at native pixels after any geometry change.
- Do not put the dark wordmark on a dark surface without a separately reviewed high-contrast colour variant; do not recolour the multi-colour logo as if it were a syntax token.
- Do not stretch, rotate, add effects, add animation, place in the editor chrome, or replace the native window title bar.
- For monochrome icon panels use `mark-mono.svg` and set `currentColor` explicitly to a contrasting solid tint; the SVG itself is not intended as an independent preview without a CSS/currentColor context.

## Provenance and licensing

The vector geometry was authored specifically for Tachyon for this asset set. The wordmark glyph outlines are derived from **DejaVu Sans Bold**, with an SVG shear for the italic stance. Source installed as `DejaVuSans-Bold.ttf`, DejaVu fonts, under the Bitstream Vera font licence; the licence notice is included verbatim in [`FONT-LICENSE.txt`](FONT-LICENSE.txt). No font file is bundled or loaded by the application. The wordmark is a new outlined treatment inspired by the concept, not a claim to reproduce the raster lettering exactly. The repository's MIT OR Apache-2.0 terms apply to the new mark geometry; the font-derived outlines retain their font licence notice.

The raster concept is reference material only and retains its own provenance; it is not embedded in these SVGs. No remote resources, filters, scripts, masks, text elements, or font lookups are used.
