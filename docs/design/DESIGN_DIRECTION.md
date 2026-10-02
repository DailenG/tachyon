# Tachyon design direction

## Purpose

Tachyon is a native Markdown editor built in Rust and GPUI. Its product goal is immediate startup and responsive editing. The visual direction should convey speed, clarity, precision, and a light touch while keeping the document central. See [the roadmap](../ROADMAP.md), [architecture](../ARCHITECTURE.md), and [contributing rules](../../CONTRIBUTING.md) for current product and engineering requirements.

## Speed comes first

Speed is the product. Every visual decision is judged first by its cost to startup and interaction, and only then by how well it expresses the brand. A design element that costs measurable startup or interaction time is a **second-class decision**: it is left out unless a benchmark shows it stays inside the budgets and the project owner approves it. When a design choice and speed conflict, ship without the design choice.

The budgets are in the [README](../../README.md#performance-budgets) and [ADR 0004](../adr/0004-startup-budget-and-gate.md): launch to first frame p95 under 50 ms, a keystroke painted within one frame, and no frame over 16.6 ms during a 5 MB paste.

Classify every proposed element before designing it in detail:

| Class | What it means | Examples |
| --- | --- | --- |
| **Free** | Compiled-in values that add no runtime work | Colors, spacing, radii, font sizes and weights of system fonts, border widths, token structure |
| **Measure** | Probably neutral, but touches startup, window creation, or a per-frame path | Icon sizes embedded in the binary, title-bar color, new highlight or overlay rendering, anything drawn on every frame |
| **Deprioritized** | Costs speed by nature; not planned | See the list below |

### Deprioritized because they compromise speed

These are not planned for the application. Proposing one needs benchmark evidence within budget and explicit owner approval; the default answer is no.

- **Bundled or custom fonts in the app**, including a brand typeface for UI text. Loading, registering, and warming up a font costs startup time and binary size. The editor uses system fonts; the wordmark exists only as outlined vector artwork.
- **Remote assets**: web fonts, remote images, or anything fetched over the network for branding.
- **Gradients in the editor chrome.** The cyan → blue → violet gradient belongs to brand assets (icons, README, website, release artwork), not to UI surfaces, text, controls, or states.
- **Shadows, blur, translucency, and backdrop effects** (for example acrylic or mica materials) on editor surfaces. (Sticky notes are an approved, owner-reviewed exception: `sticky_unfocused_opacity`, off by default, applied only while a note's own window is unfocused, and measured against the startup and per-frame budgets like any other Measure-class change - see [ADR 0010](../adr/0010-sticky-notes.md). Nowhere else in the editor.)
- **Animation and transitions**: splash screens, fades, hover or focus transitions, animated or blinking carets, animated logos. Window open and close animations are deliberately turned off (ADR 0004).
- **Brand imagery inside the app**: logos or artwork on the editing canvas, empty states, or
  overlays; raster images loaded at startup. (The About window's signature artwork is an
  approved exception - see below - decoded only when that window opens, never on the startup
  path.)
- **Anything added before the first frame for branding**: reading theme or asset files, decoding images, or rasterizing vector artwork at startup. Icons are prepared at build time.
- **Custom-drawn window chrome** replacing the native title bar. Owner-approved exception
  (issue #69, ADR 0010): sticky notes only, whose compact header (drag area, pin, close) is what
  keeps a note small. Normal windows keep the native title bar.
- **New screens justified only by branding**, such as onboarding or a splash. These would also
  be new features, which need a separate product decision. (An About window is approved for
  1.0 - see the Brand intensity table below - not as a precedent for other branding-only screens.)

## Source of truth

The visual design is defined by the owner's Claude Design project, "Tachyon Design System",
mirrored read-only in [`docs/design/system/`](system/). Read its
[`README.md`](system/README.md) (voice, colour, type, spacing, iconography) and tokens before any
UI or brand work, then [`SNAPSHOT.md`](system/SNAPSHOT.md), which lists where this repository
deliberately departs from it (for example, the editor never animates and the website self-hosts
its font). This document adds the speed rules that decide every such departure. Where the
snapshot and this document disagree on speed, this document wins; on anything else, the design
system wins.

## Concept references

The owner's concept sheets and logo masters live in [`docs/design/concepts/`](concepts/) (see its
[README](concepts/README.md)). The sheets show the intended look; the masters are the logo itself.
Do not load anything from that directory at runtime, and do not infer features from the mock-up
editor drawn in the landing-page sheets.

Production logo and icon files are **traced from the masters, never redrawn**: `scripts/brand/`
traces them, checks each trace against its master pixel for pixel, and composes every colourway,
lockup and icon size from the traces (see [`assets/brand/README.md`](../../assets/brand/README.md)).
The previous hand-drawn approximations, with a wordmark set in DejaVu Sans Bold, are how the brand
drifted from the concept; they were replaced in October 2026.

## Visual language

- Use deep navy or near-black for primary text and dark surfaces, with light neutral surfaces where appropriate.
- Use cyan → blue → violet sparingly for brand accents, and as a gradient only in brand assets. In the editor, use one solid accent for links, selection, and selected items. Keep essential text and controls legible without relying on color alone.
- The mark suggests a particle or burst traveling horizontally, with a restrained orbit or speed trail. Use the motif in brand assets, not in the editor.
- Favor clear system typography, calm spacing, restrained corners, visible focus, and low visual weight. These are directions, not fixed token values.
- Define semantic tokens for surfaces, text, borders, accents, spacing, type, radius, and interaction states where the existing code supports them. Assess the current GPUI theme code before proposing changes. Tokens are compile-time values, so they are free; keep them that way.

## Brand intensity

| Surface | Direction |
| --- | --- |
| Website and release artwork | Expressive use of the logo, gradient, and motif, with optimized assets |
| GitHub and documentation | Recognizable, restrained identity |
| App icons (window, taskbar, tray, launcher, executable) | The gradient symbol on the navy tile with a gradient border (owner decision, 2026-10-02); a simplified symbol and solid border at 16–32 px; prepared at build time from `assets/brand/` |
| Working editor | Quiet chrome; solid colors only; the document remains the focal point |
| About window | Approved for 1.0: the owner's personal signature artwork (`assets/brand/signature.png`, `signature-light.png`), theme-appropriate, decoded only when the window opens |
| Onboarding | Not planned (see the deprioritized list) |

Do not infer a sidebar, preview pane, toolbar, navigation model, settings flow, splash screen, or new feature from concept artwork. Preserve Tachyon's existing single-pane block-swap editing model unless a separate product decision changes it.

## Performance and accessibility

Follow the repository's measured startup and interaction budgets. Do not add pre-first-frame work for branding. Changes classed as **Measure** include before-and-after numbers in their pull request: `cargo xtask bench-startup` (cold and `--warm`) for startup and window creation, a Windows console run for window, icon, or title-bar changes, and frame-log measurements (typing key-to-paint, find in a 5 MB document, a 5 MB paste) for rendering changes. Record the release archive size (`cargo xtask dist`) when embedded assets change.

Check light and dark themes, contrast, keyboard focus, small windows, and reduced-motion preferences where applicable. Minimum contrast: 4.5:1 for text on every surface (including muted text and code comments), 3:1 for control borders, the focus indicator, and the caret; text over selection and find highlights must stay at 4.5:1. Do not convey required information solely through gradients, motion, or color. Because the editor has no animation, reduced-motion preferences are respected by default; keep it that way.

## How to implement this direction

1. Inspect the real application and its current theme, components, settings, and UX.
2. Classify each proposed element as Free, Measure, or Deprioritized, and drop deprioritized elements unless the owner has approved them with benchmark evidence.
3. Identify design opportunities and conflicts with the roadmap, platform conventions, accessibility, and performance budgets.
4. Propose an incremental plan with intended tokens, components, asset destinations, milestones, validation, and measured performance risks.
5. Implement only the approved scope; keep production assets in their application or website asset directories, separate from the concept references.
6. Validate functionality, visual behavior, and relevant performance on the target platforms.

When a concept conflicts with the product, adapt the brand to the product. Similarity to the reference image never outranks speed, correctness, established UX, accessibility, or maintainability, and speed comes first among those.
