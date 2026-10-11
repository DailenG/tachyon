//! Mermaid diagrams: a ```mermaid fenced code block that is not being edited shows as a diagram
//! (ADR 0012). The renderer itself (a pure-Rust Mermaid-to-SVG library) is linked by the
//! application and installed as [`MermaidRenderer`]; this crate only schedules it and caches the
//! result, so its tests never need the real renderer.
//!
//! Nothing here runs until a Mermaid block is drawn. A diagram renders on the background
//! executor (a typical flowchart takes about 20 ms, a 60-node one about 235 ms: far too long for
//! the UI thread); until it arrives the block shows its source as code. Results are cached per
//! window by source text and theme, so scrolling past a diagram again, or typing elsewhere,
//! never renders it again. A failed render (a syntax error, or the renderer panicking on input
//! it does not handle) is cached too, and the block keeps showing its source with the error
//! underneath.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{Context, Global, Image, ImageFormat, Pixels, Size, px, size};

use crate::editor::Editor;

/// Renders Mermaid source to SVG: `dark` picks the renderer's dark palette. Installed by the
/// application; without it (every test that does not set one) Mermaid blocks stay code.
#[derive(Clone, Copy)]
pub struct MermaidRenderer(pub fn(&str, bool) -> Result<String, String>);

impl Global for MermaidRenderer {}

/// Diagrams kept per window. Well above what any one document shows; when full, the cache is
/// cleared rather than tracking use, since a re-render costs one background job.
const CACHE_LIMIT: usize = 64;

/// One block's diagram state.
#[derive(Clone)]
pub(crate) enum Diagram {
    /// Being rendered; the block shows its source meanwhile.
    Pending,
    /// Ready to draw: an SVG image GPUI rasterizes off the UI thread like any other image, and
    /// the size to show it at (smaller than the size the SVG declares; see [`oversample`]).
    Ready(Arc<Image>, Size<Pixels>),
    /// The renderer refused the source; the block shows its source and this message.
    Failed(String),
}

/// How much larger than its display size a diagram's SVG is declared. GPUI rasterizes an SVG
/// image once, at its declared size, so declaring it at twice the size it is shown at keeps it
/// sharp at up to 200 % display scaling (the About window's lockup does the same).
const OVERSAMPLE: f32 = 2.;

/// Rewrites the root `<svg>`'s `width`/`height` to [`OVERSAMPLE`] times their value (its
/// `viewBox` keeps the drawing the same) and returns the original size, to display it at. `None`
/// if the root has no positive numeric width and height.
fn oversample(svg: &str) -> Option<(String, Size<Pixels>)> {
    let root = &svg[..svg.find('>')?];
    let attr = |name: &str| -> Option<(std::ops::Range<usize>, f32)> {
        let start = root.find(&format!(" {name}=\""))? + name.len() + 3;
        let len = root[start..].find('"')?;
        let value = root[start..start + len].trim_end_matches("px").parse::<f32>().ok()?;
        (value.is_finite() && value > 0.).then_some((start..start + len, value))
    };
    let (w_range, w) = attr("width")?;
    let (h_range, h) = attr("height")?;
    let (first, second) = if w_range.start < h_range.start {
        ((w_range, w), (h_range, h))
    } else {
        ((h_range, h), (w_range, w))
    };
    let mut out = String::with_capacity(svg.len() + 16);
    out.push_str(&svg[..first.0.start]);
    out.push_str(&(first.1 * OVERSAMPLE).to_string());
    out.push_str(&svg[first.0.end..second.0.start]);
    out.push_str(&(second.1 * OVERSAMPLE).to_string());
    out.push_str(&svg[second.0.end..]);
    Some((out, size(px(w), px(h))))
}

/// A window's rendered diagrams, keyed by a hash of the source and whether the theme is dark.
#[derive(Default)]
pub(crate) struct Diagrams {
    entries: HashMap<u64, Diagram>,
}

fn key(source: &str, dark: bool) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

/// Whether a fenced code block's info string names Mermaid (`mermaid`, any case).
pub(crate) fn is_mermaid(lang: Option<&str>) -> bool {
    lang.is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid"))
}

/// Runs the renderer, turning a panic into an error (a malformed diagram is user input and must
/// not take the process down), and prepares the SVG for display.
fn render_guarded(
    render: fn(&str, bool) -> Result<String, String>,
    source: &str,
    dark: bool,
) -> Result<(String, Size<Pixels>), String> {
    std::panic::catch_unwind(|| render(source, dark))
        .unwrap_or_else(|_| Err("the diagram could not be drawn".to_owned()))
        .and_then(|svg| oversample(&svg).ok_or_else(|| "the diagram has no size".to_owned()))
}

impl Editor {
    /// The diagram for a Mermaid block's `source`, starting a background render the first time
    /// it is asked for. `None` while Mermaid rendering is off or no renderer is installed: the
    /// block is then ordinary code.
    pub(crate) fn mermaid_diagram(
        &mut self,
        source: &str,
        cx: &mut Context<Self>,
    ) -> Option<Diagram> {
        if !cx.try_global::<crate::Settings>().is_none_or(|s| s.mermaid) {
            return None;
        }
        let render = cx.try_global::<MermaidRenderer>()?.0;
        let dark = self.theme.dark;
        let key = key(source, dark);
        if let Some(diagram) = self.diagrams.entries.get(&key) {
            return Some(diagram.clone());
        }
        if self.diagrams.entries.len() >= CACHE_LIMIT {
            self.diagrams.entries.clear();
        }
        self.diagrams.entries.insert(key, Diagram::Pending);
        let source = source.to_owned();
        cx.spawn(async move |editor, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { render_guarded(render, &source, dark) })
                .await;
            let diagram = match result {
                Ok((svg, shown)) => Diagram::Ready(
                    Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())),
                    shown,
                ),
                Err(error) => Diagram::Failed(error),
            };
            let _ = editor.update(cx, |editor, cx| {
                // A clear since this started (cache full, or the setting turned off) drops it.
                if let Some(slot) = editor.diagrams.entries.get_mut(&key) {
                    *slot = diagram;
                    // The block's height changes from code to diagram.
                    editor.list.remeasure();
                    cx.notify();
                }
            });
        })
        .detach();
        Some(Diagram::Pending)
    }

    /// Drops every cached diagram, so none is shown from a stale setting or theme.
    pub(crate) fn clear_diagrams(&mut self) {
        self.diagrams.entries.clear();
    }

    /// Each cached diagram's state, for tests: "pending", "ready" or "failed".
    #[cfg(test)]
    pub(crate) fn diagram_states(&self) -> Vec<&'static str> {
        self.diagrams
            .entries
            .values()
            .map(|d| match d {
                Diagram::Pending => "pending",
                Diagram::Ready(..) => "ready",
                Diagram::Failed(_) => "failed",
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversample_doubles_the_declared_size_and_reports_the_shown_one() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="236.5" height="80" viewBox="0 0 236.5 80"><rect width="236.5"/></svg>"#;
        let (out, shown) = oversample(svg).expect("sized");
        assert_eq!(shown, size(px(236.5), px(80.)));
        assert!(
            out.starts_with(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="473" height="160" viewBox="0 0 236.5 80">"#
            ),
            "{out}"
        );
        assert!(out.ends_with(r#"<rect width="236.5"/></svg>"#), "only the root changes: {out}");
    }

    #[test]
    fn oversample_refuses_a_root_without_a_usable_size() {
        assert!(oversample(r#"<svg viewBox="0 0 1 1"></svg>"#).is_none());
        assert!(oversample(r#"<svg width="0" height="5"></svg>"#).is_none());
        assert!(oversample(r#"<svg width="NaN" height="5"></svg>"#).is_none());
        assert!(oversample("").is_none());
    }
}
