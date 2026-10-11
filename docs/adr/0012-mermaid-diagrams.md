# 0012: Mermaid diagrams: a bundled pure-Rust renderer, run lazily off the UI thread

- **Status:** Accepted
- **Date:** 2026-10-11

## Context

The owner asked for Mermaid diagram support as a high-priority feature, on condition that it
cause no measurable performance variance (or be discussed first). Mermaid's reference
implementation is JavaScript run in a browser, which Tachyon will never embed. Options considered:

- **A pure-Rust renderer linked into the binary** (`mermaid-rs-renderer` 0.3.1, MIT, no browser,
  Node or system dependencies; 23 diagram types, Mermaid source to SVG).
- **The same renderer as a separate helper process, downloaded when enabled.** Keeps the base
  install small and isolates renderer crashes, but adds Tachyon's first network code, an
  offline failure mode, and a per-version download.
- **Source only:** no rendering.

Measured on Linux/Wayland, two release builds of `main` (fat LTO) differing only in the renderer
being linked but never called, with `cargo xtask bench-startup`, alternating builds:

| | Without | With (unused) |
| --- | --- | --- |
| Executable | 23.87 MB | 26.46 MB (+2.59 MB, +10.8%) |
| Cold start, median (2 × 20 launches) | 223.8 / 219.3 ms | 225.3 / 221.4 ms |
| Warm launch, median (3 × 30) | 29.2 / 29.7 / 29.2 ms | 29.2 / 29.0 / 29.4 ms |
| Warm launch, p95 | 35.7 / 33.8 / 32.2 ms | 30.8 / 32.2 / 31.9 ms |
| Resident memory after opening a file | ~115.9 MB | ~116.4 MB |

Startup and memory are within run-to-run noise: executable pages are mapped on demand, and the
renderer does no work at load (its font database and text measurement initialize on the first
diagram). Rendering itself is not cheap: about 20-25 ms for a typical flowchart, about 235 ms for
a 60-node graph, and under 1 ms for small sequence or state diagrams.

## Decision

- Bundle the renderer in the `tachyon` binary crate. `tachyon-editor` never depends on it: the
  application installs a plain function pointer (`tachyon_editor::MermaidRenderer`) that the
  editor calls. Editor tests use a stand-in.
- A fenced code block whose info string is `mermaid` (any case), when not being edited, shows its
  diagram. While the block holds the caret it is raw source, like every other block (ADR 0002).
- Rendering runs on GPUI's background executor, only when such a block is drawn. Until the result
  arrives, and permanently if the source does not render, the block shows its code; a failure
  adds the renderer's one-line error beneath it. A renderer panic is caught and treated as a
  failure (user input must not take the process down).
- Results are cached per window, keyed by source text and light/dark theme (at most 64; full
  means cleared, since a re-render is one background job). Typing elsewhere, scrolling back, or
  repainting never renders a diagram again.
- The SVG's declared size is doubled before GPUI rasterizes it (once, at its declared size) and
  shown at the original size, so diagrams stay sharp at up to 200 % display scaling.
- Diagrams use the renderer's own light or dark palette with a transparent background, so they sit
  on the editor's canvas in either theme.
- The `mermaid` setting (default `true`) turns this off; the Preferences panel and the command
  palette toggle it. Off, Mermaid blocks are ordinary code and the renderer is never called.
- The renderer's font-measurement disk cache (about 84 KB under the user cache directory) is left
  on: it makes the first diagram after a restart faster and costs nothing until a diagram is drawn.

## Consequences

- No startup or typing cost for documents without diagrams; +2.6 MB of executable.
- A diagram appears a moment after its block: instantly for small ones, up to a quarter second for
  large graphs, without ever blocking a frame.
- Release builds use `panic = "abort"`, so `catch_unwind` cannot contain a renderer panic there;
  the fuzz pass over malformed input (empty, truncated, unclosed, negative values, deep nesting)
  found errors but no panics. A renderer panic on some future input would abort the process. If
  that is ever observed, move rendering into a helper process (the alternative above) rather than
  changing the global panic strategy.
- Diagram fidelity is the renderer's, not Mermaid.js's: layouts differ in detail, and unsupported
  syntax shows as code with an error.
- Revisit if the renderer's binary cost grows materially, if a renderer panic is reported, or if
  a lighter renderer with comparable coverage appears.
