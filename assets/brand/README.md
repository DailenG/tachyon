# Tachyon brand assets

Every logo and icon file here is generated from the project owner's master artwork. Nothing in
this directory is drawn by hand, and nothing may be. The design source of truth is the owner's
Claude Design project, "Tachyon Design System", mirrored read-only in
[`docs/design/system/`](../../docs/design/system/) for agents without access to it.

**Do not load, parse, rasterize or draw these files inside the editor window or on its startup
path.** They are inputs to build-time icon generation and to external brand surfaces (website,
README, release artwork). The About window is the one in-app exception, decoded only when that
window opens (see [`docs/design/DESIGN_DIRECTION.md`](../../docs/design/DESIGN_DIRECTION.md)).

## How the files are made

```
docs/design/concepts/masters/*.png   owner-supplied masters, never edited
        │  python3 scripts/brand/trace.py
        ▼
symbol-navy.svg, wordmark-navy.svg    traced geometry (checked: < 0.5 % pixel mismatch)
        │  python3 scripts/brand/compose.py
        ▼
every other SVG in this directory     colourways, lockups, app icons
        │  cargo xtask icons
        ▼
crates/tachyon-platform/assets/tachyon.ico, tachyon.svg; packaging/msix/Assets/*.png
```

1. **`scripts/brand/trace.py`** traces the two flat-navy masters with potrace and writes
   `symbol-navy.svg` and `wordmark-navy.svg`. It renders each result back at the master's size
   and refuses to write anything if more than 0.5 % of the inked pixels disagree (the committed
   traces measure 0.35 % and 0.39 %, under a tenth of a pixel of average edge offset). Needs
   `magick`, `potrace` and `rsvg-convert`.
2. **`scripts/brand/fit_gradient.py`** measures the symbol's gradient from
   `masters/symbol-gradient.png` (the same shape as the navy master, 0.3 % apart). Its output is
   pasted into `compose.py`; re-run it only if that master changes. Needs `magick` and numpy.
3. **`scripts/brand/compose.py`** builds every other file from the two traces. Standard library
   only. Each number in it cites its source: a design-system token or a measurement of a concept
   sheet.
4. **`cargo xtask icons`** rasterizes the app icon art into the files the build embeds. The build
   itself only embeds those results; it never runs these scripts.

To change the logo, the owner supplies a new master and steps 1-4 are re-run. To change an icon's
layout, change the cited constant in `compose.py` and re-run steps 3-4. Never edit an SVG here
directly: the next run of `compose.py` overwrites it, and a hand edit is exactly how the brand
drifted before (the previous files were hand-drawn approximations with a DejaVu Sans Bold
wordmark, replaced in October 2026).

## Files

| File | What it is | Use it for |
| --- | --- | --- |
| `symbol-navy.svg` | The symbol, flat navy `#0B1220`. Traced master geometry | Monochrome on light surfaces |
| `symbol-gradient.svg` | The symbol in the master's gradient | The default symbol, light or dark surfaces |
| `symbol-white.svg` | The symbol, white | Monochrome on dark surfaces |
| `wordmark-navy.svg` | The italic wordmark, navy. Traced master geometry | Light surfaces |
| `wordmark-white.svg` | The wordmark, white | Dark surfaces |
| `lockup-horizontal.svg` / `-white.svg` | Gradient symbol left of the wordmark (navy / white) | Site header, README, wide spaces |
| `lockup-stacked.svg` / `-white.svg` | Gradient symbol above the wordmark (navy / white) | About window, hero, square spaces |
| `about-lockup.svg` / `-white.svg` | The stacked lockup declared at 440 px wide (twice its display width) | The About window only (`about.rs`); GPUI rasterizes an SVG at its declared size |
| `app-icon.svg` | App icon on a 256 px grid | 96 px and up; the Linux launcher icon |
| `app-icon-64.svg`, `-48`, `-40` | App icon drawn on each pixel grid, full symbol | 40-64 px |
| `app-icon-32.svg`, `-24`, `-20`, `-16` | App icon drawn on each pixel grid, simplified symbol | 16-32 px; the website favicon (`-32`) |
| `file-markdown-<n>.svg` | Markdown file icon (16, 24, 32, 48, 256 px grids) | `.md`, `.markdown` files in Explorer (MSIX) |
| `file-text-<n>.svg` | Plain-text file icon (same grids) | `.txt`, `.text`, `.log` files in Explorer (MSIX) |
| `signature.png`, `signature-light.png` | The owner's handwritten signature | See below |

`cargo xtask icons` renders each raster size from the smallest art at least that large, so a
44 px tile comes from `app-icon-48.svg` and a 150 px tile from `app-icon.svg`.

## The app icon

Owner decision (2026-10-02): the concept's dark-mode tile (concept sheet 2), navy, with a
coloured border so it holds its edge on both light and dark taskbars.

- **Tile:** navy, faintly lighter at its centre (design-system tokens navy-750 `#141D33` to
  navy-900 `#0B1220`), corner radius 22 % of the tile.
- **Border:** the brand gradient (cyan `#00D1FF`, blue `#2563FF`, violet `#7C3AED`, top left to
  bottom right) at 40 px and up; one solid Tachyon Blue `#2563FF` pixel at 16-32 px, where a
  one-pixel gradient reads as mud.
- **Mark:** the gradient symbol, 98 % of the tile's inside at 40 px and up, with a soft glow at
  96 px and up. At 16-32 px, the simplified symbol from concept sheet 2: the short free-standing
  speed line is dropped, the spike's thin outer tips are cropped, and every part is thickened by
  about half a pixel so nothing vanishes.

Always review icon changes at native pixels, on a light (`#F3F3F3`) and a dark (`#202020`)
background, at 16, 20, 24, 32, 40, 48 and 64 px, and compare with concept sheet 2.

## File-type icons

What Windows shows on the files Tachyon is registered for (`packaging/msix/AppxManifest.xml`,
`<uap:Logo>` on each `FileTypeAssociation`; `cargo xtask icons` renders `FileMarkdown*.png` and
`FileText*.png` with their `targetsize-<n>` siblings). Before these, Windows put the app tile on
every `.md` file, so a document looked like the app itself.

- **Shape:** a white document page with a folded top-right corner, the Windows convention for
  files, edged in neutral-400 `#94A3B8` so it holds its edge on white and on dark Explorer.
- **Markdown** (`file-markdown-*`): a heading line (neutral-500) above text lines (neutral-300),
  and the gradient symbol as the app's badge.
- **Plain text** (`file-text-*`): even text lines and the navy symbol.
- The two differ in shape (the heading line) as well as colour, so neither relies on colour
  alone. At 16 and 24 px there is room only for the page and the symbol, so there they differ by
  the symbol's colour; Explorer's file name shows the extension anyway.
- The symbol uses the same cuts as the app icon: simplified up to 32 px, full from 48 px.

The portable zip build registers no file types, so these appear only with the MSIX package.

## Colours

The brand gradient used in UI (buttons, rules, borders) is the design-system token: cyan
`#00D1FF` → blue `#2563FF` → violet `#7C3AED`. The **symbol's own gradient** is measured from the
owner's gradient master instead (16 stops along a line tilted 10° upward, violet peak `#622FFC`),
because it is the owner's artwork and the token's violet is lighter than the master's. The
gradient never appears in editor UI; it is confined to artwork.

## Clear space and constraints

- Keep clear space of at least a quarter of the symbol's height around the symbol and lockups.
- Use the full symbol at 40 px and up only; below that, use the `app-icon-16` … `-32` art.
- Use the navy wordmark on light surfaces and the white one on dark surfaces. Never recolour
  either, and never typeset the wordmark in a font: it exists only as these outlines.
- Do not stretch, rotate, re-letter, add effects, animate, put in the editor chrome, or replace
  the native window title bar with any of these.

## The owner's personal signature

`signature.png` and `signature-light.png` are the owner's own handwritten signature, unrelated to
the logo (different provenance and licensing; see below). The About window showed them for 1.0
and 1.x. Since then the About window shows the stacked logo instead (concept sheet 5); the
signature files are kept here, unused, so the owner can revert to them: put back the two
`include_bytes!` constants and the `img` element in `crates/tachyon-editor/src/about.rs`
(see that file's history).

| Asset | What it is |
| --- | --- |
| `signature.png` | The original scan: dark-navy ink on transparent, 500×250. For light themes. |
| `signature-light.png` | The same artwork with its ink recoloured to the dark theme's `text.primary` (`#EBF2FA`); alpha untouched. For dark themes. |

`signature-light.png` was prepared once with ImageMagick (extract `signature.png`'s alpha
channel, flood a solid `#EBF2FA` fill, recombine as that fill's colour with the extracted alpha),
not regenerated by any build script.

## Provenance and licensing

The symbol and wordmark are traced from masters the project owner supplied
(`docs/design/concepts/masters/`), as part of the concept work in `docs/design/concepts/`. No font
is involved: the wordmark is the owner's lettering, outlined. The generated SVGs fall under the
repository's MIT OR Apache-2.0 terms. No remote resources, scripts, text elements or font lookups
are used; the only effects are the app icon's gradient fills and its blur glow, which are
rendered at build time.

`signature.png` is Dailen Gunter's own handwritten signature, provided by him for this project;
it is not derived from the concept artwork or the logo, and is used with his explicit approval as
the project owner. `signature-light.png` is a colour-only derivative of it (see above). Neither
carries the repository's MIT OR Apache-2.0 licence: they are personal artwork.
