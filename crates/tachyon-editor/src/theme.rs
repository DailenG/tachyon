//! Built-in theme. Compiled in so startup reads no configuration.

use gpui::{
    FontStyle, FontWeight, HighlightStyle, Hsla, Pixels, SharedString, StrikethroughStyle,
    UnderlineStyle, px, rgb, rgba,
};
use tachyon_md::Style;

#[derive(Clone, Debug)]
pub struct Theme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub code_background: Hsla,
    pub raw_background: Hsla,
    pub quote_bar: Hsla,
    pub rule: Hsla,
    pub selection: Hsla,
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
    pub fn dark() -> Self {
        Theme {
            background: rgb(0x1e1f22).into(),
            foreground: rgb(0xd8dade).into(),
            muted: rgb(0x80848e).into(),
            accent: rgb(0x6cb6ff).into(),
            code_background: rgb(0x2a2c31).into(),
            raw_background: rgb(0x24262a).into(),
            quote_bar: rgb(0x4b4f58).into(),
            rule: rgb(0x3d4047).into(),
            selection: rgba(0x3d6fb566).into(),
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
