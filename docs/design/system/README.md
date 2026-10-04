# tachyon design system

tachyon is a native markdown editor built for speed: it opens instantly, paints every keystroke inside one frame, and edits plain `.md` files. Tagline: **"Write faster. Think further."** Supporting line: **"A markdown editor that gets there before you do."**

The name comes from the theoretical faster-than-light particle. The brand idea is light speed, not electricity: never use lightning or bolt imagery. The four-point **Tachyon Spark** is the speed symbol.

Audience: modern creators and developers who want to write in pure markdown without waiting. Do not position the product around privacy or security.

## Sources

- GitHub: https://github.com/daileng/tachyon (branch `main`). Used for the real app layout (`crates/tachyon-editor/src/*.rs`, especially `theme.rs`, `about.rs`, `find.rs`, `picker.rs`), the real marketing-site structure and copy (`site/index.html`, `site/styles.css`), real app screenshots (`site/assets/screenshots/`) and the owner's signature (`assets/brand/signature*.png`). Explore it further to build more faithful product screens.
- Six uploaded concept sheets (`uploads/ChatGPT Image Oct 1, 2026 …-1…6.png`): source of truth for logo, palette, buttons, badges, pills, loaders, dividers and background patterns. The landing-page and hero images are marketing references only; the app window drawn in them is a mockup.
- Three uploaded master logo files (`uploads/Clean Navy Tachyon Wordmark.png`, `Navy Tachyon Symbol on White.png`, `Tachyon gradient speed symbol.png`): the preferred logo artwork, processed to transparent PNGs in `assets/logo/`.
- **Not used:** the repo's old `assets/brand/*.svg` logos and `docs/design/*` brand direction. They are outdated and do not match the concept; where they conflict with the sheets, the sheets win. The editor's *solid in-app color values* in `theme.rs` are still the real shipped theme and are kept as `--editor-*` tokens.

## Products

1. **Editor app** (Rust + GPUI, Windows/Linux/macOS). Single-pane block-swap editor: the block under the caret shows raw markdown, every other block renders. Overlays: find/replace bar, Command Palette, Go to heading, Open recent; separate About window. Native title bar, system fonts, no animation, solid colors only. → `ui_kits/editor/`
2. **Marketing site** (static HTML). Header, hero, performance budgets, real screenshots, block-swap demo, feature grid, install, footer. → `ui_kits/website/`

## CONTENT FUNDAMENTALS

- **Voice:** calm, confident, concise. Say what it does; skip hype. Facts and measurements over adjectives ("Opens into a resident instance in 20–35 ms").
- **Speed words:** "instant", "faster than light". "Blazing" is acceptable sparingly in body copy. Never "lightning", "lightning-fast", "electric", "supercharged".
- **Feature names (exact):** Instant · Pure Markdown · Distraction-free. Do not use "Privacy first", "Secure", "Your files, your control".
- **Person:** address the reader as "you"; the product is "Tachyon" or "it". No "we" in product UI; site copy may use "we" sparingly.
- **Casing:** sentence case for headings and body ("Speed is the feature"). Title Case for buttons and command names ("View on GitHub", "Check for Updates", "Go to heading" follows the app). Settings use "Noun: Value" ("Theme: Dark"). In running text the product is "Tachyon"; the logo itself is lowercase italic artwork.
- **Labels:** uppercase spaced eyebrows ("WHY FAST", "FASTER THAN LIGHT" in the About divider).
- **Shortcuts** in one key cap: `Ctrl+Shift+P`.
- **No emoji.** Unicode only where the app uses it: "✓" for checked picker rows, "→" in links like "Build from source →", "–" for ranges.
- **Examples:** "Write faster. Think further." / "A markdown editor that gets there before you do." / "Quit never asks. Unsaved documents come back exactly as you left them." / "Every budget below is a requirement in Tachyon's own repository, checked in CI."

## VISUAL FOUNDATIONS

- **Color:** Navy `#0B1220` text on Light Background `#F8FAFC`. Brand gradient Cyan `#00D1FF` → Blue `#2563FF` → Violet `#7C3AED`, used sparingly: logo, primary buttons, one gradient headline phrase, key accents (progress fill, spark divider). **Never a page or section background.** Solid Tachyon Blue is the working accent (links, soft buttons, selected pills, eyebrows).
- **Dark mode:** deep navy surfaces from `#0B1220` (`#101A2D`, `#111A2E`, `#141D33`), borders `#1A2542`/`#223055`, text `#E9ECFF`; subtle cyan-blue glow on elevated items (`--shadow-md/lg` in dark, `drop-shadow` glow on the symbol). Toggle with `data-theme="dark"` on any ancestor.
- **Editor theme:** the app uses its own solid tokens (`--editor-*`, `--syntax-*`) from `theme.rs`: accent `#1356A4` / `#8BC3FF`, no gradients, no shadows, no transparency except selection and find highlights.
- **Type:** Inter. Headings semibold (600) at −1% tracking; display −2%. Body regular 16px / 150%. Eyebrows 12px 600 uppercase +16% (accent dividers +24%). Code: system monospace. The editor uses the system UI font at 15px (H1 28, H2 23, H3 19). **The wordmark is always the supplied logo file, never typeset in Inter.**
- **Spacing:** 4px base (4 … 96). Generous whitespace; sections 88px vertical; containers max 1180px.
- **Radii:** 4 (editor rows, inline code, checkboxes), 8 (editor bars, pickers), 10–14 (buttons by size), 16 (cards), 24+ (app-icon tiles, hero frames), pill for badges/pills/tracks.
- **Cards:** white surface, 1px `--border-subtle`, radius 16, soft blue-tinted shadow (`--shadow-sm/md`). No colored left borders.
- **Shadows:** soft and blue-tinted (`rgba(37,99,255,…)`), never grey-black heavy. Primary buttons carry a blue glow (`--shadow-glow-primary`).
- **Backgrounds:** light and airy, plain `#F8FAFC` or white. Decoration is limited to the supplied low-contrast patterns (orbit, speed lines, grid dots) and accents (speed streak, motion lines, comet, orbit ring, Spark) placed at edges, never behind body text. No photography. Imagery is the real app screenshot in a 3px gradient frame.
- **Motion:** fast and quiet. `--ease-out` cubic-bezier(.16,1,.3,1), 120–180ms. Hover lifts primary buttons/cards by 1px; transitions on color/border only. Loaders animate; everything goes static under `prefers-reduced-motion`. **The editor has no animation at all** (no fades, no blinking caret).
- **Hover:** primary gradient extends toward violet + 1px lift; secondary border turns blue; soft gets a deeper tint; links go to `--blue-700`; nav text goes navy.
- **Press:** primary becomes deep blue (`#1D45C8 → #1E37A0`) with no glow or lift; secondary/soft go one tint darker. No scale/shrink.
- **Focus:** 2px accent outline, 2px offset (web); 1px focus border in the editor.
- **Transparency & blur:** only the sticky site header (90% light + 8px blur). Not in the editor.
- **Layout:** sticky 68px site header; editor is one centred column ≤ 820px with overlays anchored top-centre.

## ICONOGRAPHY

- **Lucide** line icons (stroke 2, round caps), loaded from CDN (`lucide@0.469.0`) by the `Icon` component, colored with brand tokens (usually `--accent`, sometimes violet or navy). The repo ships no icon set of its own; the brand brief specifies Lucide.
- **Speed = `sparkle`** (Lucide's four-point star) in UI, or the gradient **Tachyon Spark** image (`assets/decor/tachyon-spark.png`) for brand moments. Never `zap`/bolt.
- Common: `download`, `github`, `arrow-right`, `arrow-down`, `file-text` (Pure Markdown), `eye-off` (Distraction-free), `code`, `feather`, `search`, `command`, `check`.
- Feature icons sit in a soft rounded tile (`FeatureIcon`).
- No emoji. Unicode "✓" only in editor picker rows (as the app does).
- The editor itself draws no icons in its chrome (native title bar; text-only overlays).

## Index

- `styles.css` — entry point (imports only) → `tokens/fonts.css`, `colors.css`, `typography.css`, `spacing.css`, `base.css`
- `assets/logo/` — `symbol-gradient.png`, `symbol-navy.png`, `symbol-white.png`, `wordmark-navy.png`, `wordmark-white.png`, `app-icon-light.png`, `app-icon-dark.png`
- `assets/decor/` — `tachyon-spark.png`, `speed-streak.png`, `motion-lines.png`, `comet-accent.png`, `orbit-ring.png`, `divider-*.png`
- `assets/patterns/` — `orbit.png`, `speed-lines.png`, `grid-dots.png`
- `assets/brand/` — `signature.png`, `signature-light.png` (About window only)
- `site/assets/` — real app screenshots and OG image from the repo
- `guidelines/` — foundation specimen cards (Colors, Type, Spacing, Brand)
- `components/` — React primitives (below)
- `ui_kits/editor/` — editor app recreation · `ui_kits/website/` — marketing site
- `SKILL.md`, `github.md`, `thumbnail.html`

## Components

- `components/brand/` — **Logo**, **Spark**, **Icon**
- `components/actions/` — **Button**
- `components/display/` — **Badge**, **Pill**, **FeatureIcon**, **FeatureCard**, **Divider**
- `components/feedback/` — **Spinner**, **LoadingDots**, **ProgressBar**
- `components/editor/` — **FindBar**, **Picker**, **TaskCheckbox**, **Kbd**

Inventory comes from the brand sheets (buttons, badges, pills, feature icons, loaders, dividers, UI card) and the app source (find bar, picker, task checkbox; key caps from the site).

### Intentional additions

- **Icon** — wrapper that loads Lucide and renders any glyph in brand colors.
- **Logo** / **Spark** — wrappers around the supplied raster files so consumers never retype the wordmark. Pages outside the project root set `window.TACHYON_ASSET_BASE` (e.g. `'../../'`).

## Caveats

- Inter is loaded from Google Fonts; no font binaries were supplied.
- Logo files are raster PNGs (processed from the uploads). Vector masters are needed for print and very large sizes.
- The About window follows `about.rs` (signature, version, environment table) with the supplied wordmark and the "FASTER THAN LIGHT" divider added per the brief.
