//! The block-swap Markdown editor: a GPUI view over a `tachyon_doc::Document`.
//! The block holding the caret is shown and edited as raw Markdown; all other
//! blocks are rendered with their syntax hidden.

mod editor;
mod movement;
mod prompt;
mod render;
mod theme;

pub use crate::editor::{CloseWindow, Editor, KEY_CONTEXT, Save, SaveAs, key_bindings};
pub use crate::prompt::keyboard_prompt;
pub use crate::theme::Theme;

/// Registers the editor's key bindings.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(key_bindings());
}

#[cfg(test)]
mod tests;
