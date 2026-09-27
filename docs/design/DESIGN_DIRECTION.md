# Tachyon design direction

## Purpose

Tachyon is a native Markdown editor built in Rust and GPUI. Its product goal is immediate startup and responsive editing. The visual direction should convey speed, clarity, precision, and a light touch while keeping the document central. See [the roadmap](../ROADMAP.md), [architecture](../ARCHITECTURE.md), and [contributing rules](../../CONTRIBUTING.md) for current product and engineering requirements.

## Concept references

The reference artwork lives in [`docs/design/concepts/`](concepts/). The image currently available is [`tachyon-logo-concept.png`](concepts/tachyon-logo-concept.png): the selected horizontal particle/orbit mark above an italic Tachyon wordmark on a light background. Its cyan, blue, and violet treatment and dark navy text suggest a brand direction.

These are **conceptual references, not canonical UI specifications or production-ready assets**. The image does not prescribe the editor layout, controls, typography implementation, exact colors, feature set, app icon, or website layout. Do not load artwork from `docs/design/concepts/` at runtime. Future concept images can be added to that folder and documented in its README; do not assume that mockups discussed elsewhere are present here.

Any production logo, icon, or other brand asset needs a separate implementation appropriate to its destination, with scalable geometry, small-size legibility, accessible contrast, and suitable licensing or provenance. Avoid automatically vectorizing the raster reference into an unreviewed runtime asset.

## Visual language

- Use deep navy or near-black for primary text and dark surfaces, with light neutral surfaces where appropriate.
- Use cyan → blue → violet sparingly for brand accents. Keep essential text and controls legible without relying on a gradient.
- The mark suggests a particle or burst traveling horizontally, with a restrained orbit or speed trail. Use the motif selectively.
- Favor clear typography, calm spacing, restrained corners, visible focus, and low visual weight. These are directions, not fixed token values.
- Define semantic tokens for surfaces, text, borders, accents, status, spacing, type, radius, and interaction states where the existing code supports them. Assess the current GPUI theme code before proposing changes.

## Brand intensity

| Surface | Direction |
| --- | --- |
| Website and release artwork | Expressive use of the logo and motif, with optimized assets |
| GitHub and documentation | Recognizable, restrained identity |
| About screen and onboarding | Clear brand identification |
| Working editor | Quiet chrome; the document remains the focal point |

Do not infer a sidebar, preview pane, toolbar, navigation model, settings flow, splash screen, or new feature from concept artwork. Preserve Tachyon's existing single-pane block-swap editing model unless a separate product decision changes it.

## Performance and accessibility

Follow the repository's measured startup and interaction budgets. Do not add pre-first-frame work for branding without benchmark evidence. Prefer lightweight GPUI/native primitives and compact local assets; avoid blocking remote fonts or imagery, unnecessary animation libraries, heavy blur, and elaborate splash transitions. Run relevant measurements for any implementation that changes startup or hot paths, using the process in `AGENTS.md` and `CONTRIBUTING.md`.

Check light and dark themes, contrast, keyboard focus, small windows, and reduced-motion preferences where applicable. Do not convey required information solely through gradients, motion, or color.

## How to implement this direction

1. Inspect the real application and its current theme, components, settings, and UX.
2. Identify design opportunities and conflicts with the roadmap, platform conventions, accessibility, and performance budgets.
3. Propose an incremental plan with intended tokens, components, asset destinations, milestones, validation, and measurable performance risks.
4. Implement only the approved scope; keep production assets in their application or website asset directories, separate from the concept references.
5. Validate functionality, visual behavior, and relevant performance on the target platforms.

When a concept conflicts with the product, adapt the brand to the product. Similarity to the reference image never outranks correctness, performance, established UX, accessibility, or maintainability.
