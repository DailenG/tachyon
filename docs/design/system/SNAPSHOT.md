# Design system snapshot

The files beside this one are a **read-only copy** of the project owner's Claude Design project,
"Tachyon Design System" (claude.ai/design, project id `9785dd01-b9a6-4c92-89c1-91ae48499346`).
That project is Tachyon's visual source of truth. This copy exists so agents and contributors who
cannot reach Claude Design still work from the same rules. Everything here except this file is a
verbatim copy: never edit those files to change the design. Change the Claude Design project and
refresh the copy.

| | |
| --- | --- |
| Copied | 2026-10-02, from the project as last updated 2026-10-02 16:12 UTC |
| Copied by | an agent session with Claude Design access (DesignSync `get_file`, read only) |
| Files | `README.md`, `styles.css`, `tokens/base.css`, `tokens/colors.css`, `tokens/fonts.css`, `tokens/spacing.css`, `tokens/typography.css` |

**Not copied:** the project's React component prototypes (`components/`), specimen cards
(`guidelines/`), UI kit mock-ups (`ui_kits/`), raster assets (`assets/`, `uploads/`) and build
files. The README above describes them. The six concept sheets and three logo masters it calls
`uploads/…` are in [`../concepts/`](../concepts/) as the owner's originals. The production logo
files are vectors traced from those masters, in [`assets/brand/`](../../../assets/brand/), not the
project's raster `assets/logo/` PNGs.

## Where this repository deliberately differs

The design system was written before some owner decisions, and a few of its statements cannot
apply to a native app with a 50 ms startup budget. Where this table and the copied files
disagree, this table wins. Do not "fix" the repository back to the copied text.

| Design system says | Repository does | Why |
| --- | --- | --- |
| The repo's `assets/brand/*.svg` and `docs/design/*` are outdated; don't use them | Both were rebuilt from the design system in October 2026 and are current | The statement described the old hand-drawn files, now deleted |
| Logo files are raster PNGs; vector masters are needed | `assets/brand/` holds vectors traced from the owner's masters (`scripts/brand/`) | Icons and print need vectors |
| Inter from Google Fonts (`tokens/fonts.css`) | Website: Inter self-hosted from the site's own domain. App: system fonts only | Owner decision 2026-10-02 (no third-party round trips on the site); bundled fonts cost app startup |
| Lucide icons loaded from a CDN | Website: Lucide glyphs inlined as SVG, no CDN | Same reason |
| About window: signature at the top, wordmark below | About window: stacked lockup, no signature (signature files kept for a revert) | Owner decision 2026-10-02, following concept sheet 5 |
| App icon tiles: light and dark (`app-icon-light/-dark.png`) | One icon: the dark navy tile with a gradient border | Owner decision 2026-10-02 |
| Motion: 120-180 ms transitions, animated loaders | Website only. The editor has no animation at all | Speed budget (ADR 0004); the design system agrees for the editor |

## Known contrast exceptions (owner decision, 2026-10-03)

The website uses the design system's colors exactly, including four that miss the text-contrast
minimums in `docs/design/DESIGN_DIRECTION.md` (4.5:1 for text, 3:1 for large text). The owner
reviewed the measured values and chose the design over the minimums. Do not "fix" these without
the owner's go-ahead; an accessible alternative was built and reverted (PR #120 history).

| Element | Measured | Accessible alternative, if ever wanted |
| --- | --- | --- |
| Primary button: white text on cyan → blue | 2.2-3.6:1 under the label | Blue → violet (`#2563FF` → `#7C3AED`), 4.9:1 and up |
| "Think further." gradient, light mode | 1.8:1 at the cyan start | Start at Cyan 600 `#0099C7` (3:1) |
| Badge: white on `#2563FF` → `#4F7DFF` | 3.6:1 at the light end | Solid `#2563FF` |
| `--text-muted` on `--surface-sunken`, light mode | 4.3:1 | `--text-body` there |

## Refreshing the copy

Only a session that can reach Claude Design can refresh it. Subagents spawned by a session
usually cannot (the DesignSync tool is not passed to them), so do this from the main session.

1. List the project's files (DesignSync `list_files`) and read each file in the table above
   (`get_file`). Never use DesignSync write methods for this; the copy flows one way.
2. Write each file's content here byte for byte. Do not reformat or annotate it.
3. Update the "Copied" row above and the "Files" row if the set changed.
4. Read the diff. If the design changed in a way that affects shipped code (tokens used by
   `site/styles.css`, brand rules used by `scripts/brand/compose.py`), make that code change in
   the same pull request, or open an issue for it.

If you are an agent without Claude Design access and you need something the copy does not cover,
say so in your pull request or ask the owner. Do not guess the design from the concept images,
and do not redraw brand artwork.
