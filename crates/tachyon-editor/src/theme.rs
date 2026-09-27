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

#[derive(Clone, Debug)]
pub struct Theme {
    /// Whether this is the dark theme.
    pub dark: bool,
    /// Zoom factor the sizes below are scaled by (1.0 = 100 %).
    pub zoom: f32,
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub code_background: Hsla,
    pub raw_background: Hsla,
    pub quote_bar: Hsla,
    pub rule: Hsla,
    pub selection: Hsla,
    /// Find matches, and the selected one.
    pub find_match: Hsla,
    pub find_current: Hsla,
    pub cursor: Hsla,
    /// Code token colors: keyword, string, comment, number, function, type.
    pub syntax: [Hsla; 6],
    pub math: Hsla,
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
            background: rgb(0xffffff).into(),
            foreground: rgb(0x1f2328).into(),
            muted: rgb(0x6e7781).into(),
            accent: rgb(0x0969da).into(),
            code_background: rgb(0xeff1f3).into(),
            raw_background: rgb(0xf6f8fa).into(),
            quote_bar: rgb(0xd0d7de).into(),
            rule: rgb(0xd8dee4).into(),
            selection: rgba(0x0969da33).into(),
            find_match: rgba(0xf2cc6080).into(),
            find_current: rgba(0xf0a020c0).into(),
            cursor: rgb(0x1f2328).into(),
            syntax: [
                rgb(0xcf222e).into(),
                rgb(0x0a3069).into(),
                rgb(0x6e7781).into(),
                rgb(0x0550ae).into(),
                rgb(0x8250df).into(),
                rgb(0x953800).into(),
            ],
            math: rgb(0x8250df).into(),
            ..Self::dark()
        }
    }

    pub fn dark() -> Self {
        Theme {
            dark: true,
            zoom: 1.,
            background: rgb(0x1e1f22).into(),
            foreground: rgb(0xd8dade).into(),
            muted: rgb(0x80848e).into(),
            accent: rgb(0x6cb6ff).into(),
            code_background: rgb(0x2a2c31).into(),
            raw_background: rgb(0x24262a).into(),
            quote_bar: rgb(0x4b4f58).into(),
            rule: rgb(0x3d4047).into(),
            selection: rgba(0x3d6fb566).into(),
            find_match: rgba(0xe5c07b40).into(),
            find_current: rgba(0xe5c07baa).into(),
            cursor: rgb(0xe6e8eb).into(),
            syntax: [
                rgb(0xc678dd).into(),
                rgb(0x98c379).into(),
                rgb(0x7f848e).into(),
                rgb(0xd19a66).into(),
                rgb(0x61afef).into(),
                rgb(0xe5c07b).into(),
            ],
            math: rgb(0xd2a8ff).into(),
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
                Some(StrikethroughStyle { thickness: px(1.), color: Some(self.muted) });
        }
        if style.contains(Style::CODE) {
            h.background_color = Some(self.code_background);
        }
        if style.contains(Style::LINK) || style.contains(Style::IMAGE) {
            h.color = Some(self.accent);
            h.underline =
                Some(UnderlineStyle { thickness: px(1.), color: Some(self.accent), wavy: false });
        }
        if style.contains(Style::MATH) {
            h.color = Some(self.math);
            h.font_style = Some(FontStyle::Italic);
        }
        if style.contains(Style::HTML) || style.contains(Style::FOOTNOTE_REF) {
            h.color = Some(self.muted);
        }
        let tokens = [
            Style::KEYWORD,
            Style::STRING,
            Style::COMMENT,
            Style::NUMBER,
            Style::FUNCTION,
            Style::TYPE,
        ];
        if let Some(i) = tokens.iter().position(|&token| style.contains(token)) {
            h.color = Some(self.syntax[i]);
            if style.contains(Style::COMMENT) {
                h.font_style = Some(FontStyle::Italic);
            }
        }
        h
    }
}
