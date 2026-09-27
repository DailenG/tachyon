//! Built-in themes, dark and light, following the system appearance. Compiled in so startup
//! reads no configuration.

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

pub(crate) fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// Whether this is the dark theme.
    pub dark: bool,
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
    pub math: Hsla,
    pub text_size: Pixels,
    pub code_size: Pixels,
    /// Heading sizes for levels 1–6.
    pub heading_sizes: [Pixels; 6],
    /// Replaced by the first installed candidate once the window is up
    /// (see `Editor::resolve_code_font`).
    pub code_font: SharedString,
    pub content_width: Pixels,
}

impl Theme {
    pub fn for_dark(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// The theme for the system appearance: the [`AppearanceHint`] if set, else `window`'s.
    pub fn for_window(window: &Window, cx: &App) -> Self {
        let hint = cx.try_global::<AppearanceHint>().map(|hint| hint.dark);
        Self::for_dark(hint.unwrap_or_else(|| is_dark(window.appearance())))
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
            math: rgb(0x8250df).into(),
            ..Self::dark()
        }
    }

    pub fn dark() -> Self {
        Theme {
            dark: true,
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
            math: rgb(0xd2a8ff).into(),
            text_size: px(15.),
            code_size: px(13.5),
            heading_sizes: [px(28.), px(23.), px(19.), px(17.), px(15.), px(14.)],
            code_font: tachyon_platform::monospace_font_candidates()[0].into(),
            content_width: px(820.),
        }
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
        h
    }
}
