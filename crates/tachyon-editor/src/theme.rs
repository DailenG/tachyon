//! Built-in themes, dark and light, following the system appearance. Compiled in so startup
//! reads no configuration.

use std::sync::OnceLock;

use gpui::{
    App, FontStyle, FontWeight, Global, HighlightStyle, Hsla, Pixels, SharedString,
    StrikethroughStyle, UnderlineStyle, Window, WindowAppearance, px, rgb, rgba,
};
use tachyon_md::Style;

/// The system appearance, known before GPUI's windows report it (Linux: GPUI asks the desktop
/// portal asynchronously, so its first windows report light). Set it before opening the first
/// window; editors drop it once a window reports the same appearance.
pub struct AppearanceHint {
    pub dark: bool,
}

impl Global for AppearanceHint {}

/// GPUI's name for the platform's UI font.
const SYSTEM_FONT: &str = ".SystemUIFont";

/// The first installed family among `candidates`. The system fonts are listed once per process:
/// that walks the whole font collection, and a resident instance opens many windows.
pub(crate) fn installed_font(window: &Window, candidates: &[&'static str]) -> Option<&'static str> {
    static INSTALLED: OnceLock<Vec<String>> = OnceLock::new();
    let installed = INSTALLED.get_or_init(|| window.text_system().all_font_names());
    candidates.iter().find(|family| installed.iter().any(|name| name == *family)).copied()
}

/// The family for body text (see `tachyon_platform::text_font_candidates`).
fn text_font(window: &Window) -> SharedString {
    installed_font(window, tachyon_platform::text_font_candidates()).unwrap_or(SYSTEM_FONT).into()
}

pub(crate) fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

/// Background surfaces.
#[derive(Clone, Copy, Debug)]
pub struct Surfaces {
    /// The window and the rendered document.
    pub canvas: Hsla,
    /// The raw editing card, find bar, pickers and prompt.
    pub raised: Hsla,
    /// Inline and fenced code, table headers.
    pub code: Hsla,
}

/// Text colors. Each meets 4.5:1 on every surface (see the contrast test).
#[derive(Clone, Copy, Debug)]
pub struct TextColors {
    pub primary: Hsla,
    /// Labels, list markers, quotes, metadata.
    pub muted: Hsla,
    pub link: Hsla,
    /// Text on a solid `accent` fill (selected picker row, prompt button, checked task).
    pub on_accent: Hsla,
}

/// 1 px boundaries.
#[derive(Clone, Copy, Debug)]
pub struct Borders {
    /// Rules, table grid, heading underline, quote bar: decorative.
    pub subtle: Hsla,
    /// Boundaries that identify something interactive; at least 3:1 on every surface.
    pub control: Hsla,
    /// The focused element's boundary; at least 3:1 on every surface.
    pub focus: Hsla,
}

/// Editing states. The fills are the only translucent colors in the editor.
#[derive(Clone, Copy, Debug)]
pub struct Editing {
    pub selection: Hsla,
    pub find_match: Hsla,
    pub find_current: Hsla,
    /// Opaque underline marking find matches (thicker under the current one).
    pub find_underline: Hsla,
    pub caret: Hsla,
}

/// Code token colors, and math.
#[derive(Clone, Copy, Debug)]
pub struct Syntax {
    pub keyword: Hsla,
    pub string: Hsla,
    pub comment: Hsla,
    pub number: Hsla,
    pub function: Hsla,
    pub type_: Hsla,
    pub math: Hsla,
}

/// Semantic design tokens (docs/design/TOKENS_PROPOSAL.md) and type metrics. All values are
/// compiled in: choosing a theme costs nothing at startup.
#[derive(Clone, Debug)]
pub struct Theme {
    /// Whether this is the dark theme.
    pub dark: bool,
    /// Zoom factor the sizes below are scaled by (1.0 = 100 %).
    pub zoom: f32,
    pub surface: Surfaces,
    pub text: TextColors,
    pub border: Borders,
    /// The one solid brand blue: links, selected rows, prompt buttons, checked tasks.
    pub accent: Hsla,
    pub editing: Editing,
    pub syntax: Syntax,
    pub text_size: Pixels,
    pub code_size: Pixels,
    /// Heading sizes for levels 1–6.
    pub heading_sizes: [Pixels; 6],
    /// Replaced by the first installed candidate once the window is up
    /// (see `Editor::resolve_code_font`).
    pub code_font: SharedString,
    pub content_width: Pixels,
    /// Body text family. Resolved when a window is available (`Theme::for_window`).
    pub text_font: SharedString,
}

impl Theme {
    pub fn for_dark(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// The theme for the system appearance (the [`AppearanceHint`] if set, else `window`'s), with
    /// its text font resolved.
    pub fn for_window(window: &Window, cx: &App) -> Self {
        let hint = cx.try_global::<AppearanceHint>().map(|hint| hint.dark);
        Self::for_dark(hint.unwrap_or_else(|| is_dark(window.appearance()))).with_text_font(window)
    }

    /// This theme with the text font installed on the system.
    pub fn with_text_font(self, window: &Window) -> Self {
        Theme { text_font: text_font(window), ..self }
    }

    /// The dark or light theme with this theme's fonts and zoom.
    pub fn restyled(&self, dark: bool) -> Self {
        Theme {
            code_font: self.code_font.clone(),
            text_font: self.text_font.clone(),
            ..Self::for_dark(dark)
        }
        .zoomed(self.zoom)
    }

    pub fn light() -> Self {
        Theme {
            dark: false,
            surface: Surfaces {
                canvas: rgb(0xfafbfe).into(),
                raised: rgb(0xf1f5f9).into(),
                code: rgb(0xe7edf5).into(),
            },
            text: TextColors {
                primary: rgb(0x172335).into(),
                muted: rgb(0x42566c).into(),
                link: rgb(0x1356a4).into(),
                on_accent: rgb(0xffffff).into(),
            },
            border: Borders {
                subtle: rgb(0xb5c2d0).into(),
                control: rgb(0x66798f).into(),
                focus: rgb(0x1356a4).into(),
            },
            accent: rgb(0x1356a4).into(),
            editing: Editing {
                selection: rgba(0x1356a42b).into(),
                find_match: rgba(0xe8ab4138).into(),
                find_current: rgba(0xe8ab4154).into(),
                find_underline: rgb(0x87400f).into(),
                caret: rgb(0x172335).into(),
            },
            syntax: Syntax {
                keyword: rgb(0x8b2a54).into(),
                string: rgb(0x286430).into(),
                comment: rgb(0x45596e).into(),
                number: rgb(0x87400f).into(),
                function: rgb(0x235892).into(),
                type_: rgb(0x6a3b8c).into(),
                math: rgb(0x693c91).into(),
            },
            ..Self::dark()
        }
    }

    pub fn dark() -> Self {
        Theme {
            dark: true,
            zoom: 1.,
            surface: Surfaces {
                canvas: rgb(0x0e1623).into(),
                raised: rgb(0x192434).into(),
                code: rgb(0x131e2e).into(),
            },
            text: TextColors {
                primary: rgb(0xebf2fa).into(),
                muted: rgb(0xabbdd0).into(),
                link: rgb(0x8bc3ff).into(),
                on_accent: rgb(0x0e1623).into(),
            },
            border: Borders {
                subtle: rgb(0x40526a).into(),
                control: rgb(0x6f849b).into(),
                focus: rgb(0x8bc3ff).into(),
            },
            accent: rgb(0x8bc3ff).into(),
            editing: Editing {
                selection: rgba(0x8bc3ff33).into(),
                find_match: rgba(0xbe844730).into(),
                find_current: rgba(0xbe84474d).into(),
                find_underline: rgb(0xe4b17d).into(),
                caret: rgb(0xebf2fa).into(),
            },
            syntax: Syntax {
                keyword: rgb(0xf398c4).into(),
                string: rgb(0xa9d28b).into(),
                comment: rgb(0xabbdd0).into(),
                number: rgb(0xe4b17d).into(),
                function: rgb(0x8bc3ff).into(),
                type_: rgb(0xd3aaff).into(),
                math: rgb(0xc6a2ed).into(),
            },
            text_size: px(15.),
            code_size: px(13.5),
            heading_sizes: [px(28.), px(23.), px(19.), px(17.), px(15.), px(14.)],
            code_font: tachyon_platform::monospace_font_candidates()[0].into(),
            content_width: px(820.),
            text_font: SYSTEM_FONT.into(),
        }
    }

    /// This theme with its sizes scaled for `zoom` instead of its current zoom.
    pub fn zoomed(mut self, zoom: f32) -> Self {
        let factor = zoom / self.zoom;
        self.text_size *= factor;
        self.code_size *= factor;
        for size in &mut self.heading_sizes {
            *size *= factor;
        }
        self.content_width *= factor;
        self.zoom = zoom;
        self
    }

    /// A length given at 100 % zoom, at this theme's zoom.
    pub fn scaled(&self, length: Pixels) -> Pixels {
        length * self.zoom
    }

    /// Inline style for a run of rendered text.
    pub fn highlight(&self, style: Style) -> HighlightStyle {
        let mut h = HighlightStyle::default();
        if style.contains(Style::STRONG) {
            h.font_weight = Some(FontWeight::BOLD);
        }
        if style.contains(Style::EMPHASIS) {
            h.font_style = Some(FontStyle::Italic);
        }
        if style.contains(Style::STRIKETHROUGH) {
            h.strikethrough =
                Some(StrikethroughStyle { thickness: px(1.), color: Some(self.text.muted) });
        }
        if style.contains(Style::CODE) {
            h.background_color = Some(self.surface.code);
        }
        if style.contains(Style::LINK) || style.contains(Style::IMAGE) {
            h.color = Some(self.text.link);
            h.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(self.text.link),
                wavy: false,
            });
        }
        if style.contains(Style::MATH) {
            h.color = Some(self.syntax.math);
            h.font_style = Some(FontStyle::Italic);
        }
        if style.contains(Style::HTML) || style.contains(Style::FOOTNOTE_REF) {
            h.color = Some(self.text.muted);
        }
        let s = &self.syntax;
        let tokens = [
            (Style::KEYWORD, s.keyword),
            (Style::STRING, s.string),
            (Style::COMMENT, s.comment),
            (Style::NUMBER, s.number),
            (Style::FUNCTION, s.function),
            (Style::TYPE, s.type_),
        ];
        if let Some(&(token, color)) = tokens.iter().find(|&&(token, _)| style.contains(token)) {
            h.color = Some(color);
            if token == Style::COMMENT {
                h.font_style = Some(FontStyle::Italic);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, Rgba};

    use super::Theme;

    /// 8-bit sRGB channels and alpha.
    fn channels(color: Hsla) -> ([f32; 3], f32) {
        let Rgba { r, g, b, a } = Rgba::from(color);
        ([r, g, b].map(|c| (c * 255.).round()), a)
    }

    /// `color` blended over the opaque `surface` in 8-bit sRGB, as TOKENS_PROPOSAL.md measures.
    fn over(color: Hsla, surface: Hsla) -> [f32; 3] {
        let ((fg, a), (bg, _)) = (channels(color), channels(surface));
        [0, 1, 2].map(|i| (a * fg[i] + (1. - a) * bg[i]).round())
    }

    fn luminance(rgb: [f32; 3]) -> f32 {
        let [r, g, b] = rgb.map(|c| {
            let c = c / 255.;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// WCAG 2.x contrast ratio.
    fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn opaque(color: Hsla) -> [f32; 3] {
        channels(color).0
    }

    /// Every documented pair, for one theme: returns the failures.
    fn failures(theme: &Theme) -> Vec<String> {
        let t = &theme.text;
        let s = &theme.syntax;
        let texts = [
            ("text.primary", t.primary),
            ("text.muted", t.muted),
            ("text.link", t.link),
            ("syntax.keyword", s.keyword),
            ("syntax.string", s.string),
            ("syntax.comment", s.comment),
            ("syntax.number", s.number),
            ("syntax.function", s.function),
            ("syntax.type", s.type_),
            ("syntax.math", s.math),
        ];
        let surfaces = [
            ("canvas", theme.surface.canvas),
            ("raised", theme.surface.raised),
            ("code", theme.surface.code),
        ];
        let e = &theme.editing;
        let mut failures = Vec::new();
        let mut check = |what: String, ratio: f32, min: f32| {
            if ratio < min {
                failures.push(format!("{what}: {ratio:.2} < {min}"));
            }
        };
        for (surface_name, surface) in surfaces {
            let bg = opaque(surface);
            for (name, color) in texts {
                check(format!("{name} on {surface_name}"), contrast(opaque(color), bg), 4.5);
                // Text stays readable over the translucent editing highlights too.
                for (fill_name, fill) in [
                    ("selection", e.selection),
                    ("find_match", e.find_match),
                    ("find_current", e.find_current),
                ] {
                    let blended = over(fill, surface);
                    check(
                        format!("{name} over {fill_name} on {surface_name}"),
                        contrast(opaque(color), blended),
                        4.5,
                    );
                }
            }
            for (name, color) in [
                ("border.control", theme.border.control),
                ("border.focus", theme.border.focus),
                ("editing.caret", e.caret),
                ("editing.find_underline", e.find_underline),
            ] {
                check(format!("{name} on {surface_name}"), contrast(opaque(color), bg), 3.);
            }
            let under_match = over(e.find_match, surface);
            check(
                format!("find_underline over find_match on {surface_name}"),
                contrast(opaque(e.find_underline), under_match),
                3.,
            );
        }
        check(
            "text.on_accent on accent".to_owned(),
            contrast(opaque(t.on_accent), opaque(theme.accent)),
            4.5,
        );
        failures
    }

    #[test]
    fn both_themes_meet_the_documented_contrast() {
        for theme in [Theme::light(), Theme::dark()] {
            let failures = failures(&theme);
            assert!(failures.is_empty(), "dark={}: {failures:#?}", theme.dark);
        }
    }

    #[test]
    fn the_check_catches_a_failing_pair() {
        let mut theme = Theme::light();
        theme.text.muted = theme.surface.code;
        assert!(failures(&theme).iter().any(|f| f.starts_with("text.muted on code")));
    }
}
