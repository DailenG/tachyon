//! The block-swap Markdown editor: a GPUI view over a `tachyon_doc::Document`.
//! The block holding the caret is shown and edited as raw Markdown; all other
//! blocks are rendered with their syntax hidden.

mod editor;
mod find;
mod frame_log;
mod frame_stats;
mod movement;
mod prompt;
mod render;
mod theme;

pub use crate::editor::{CloseWindow, Editor, KEY_CONTEXT, OpenPaths, Save, SaveAs, key_bindings};
pub use crate::prompt::keyboard_prompt;
pub use crate::theme::{AppearanceHint, Theme};

/// Registers the editor's key bindings, and applies a large paste still
/// being prepared before any later keystroke is handled (so a key typed
/// right after Ctrl+V lands after the pasted text).
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(key_bindings());
    cx.intercept_keystrokes(|_, window, cx| {
        if let Some(Some(editor)) = window.root::<Editor>() {
            editor.update(cx, |editor, cx| {
                if let Some(log) = &mut editor.frame_log {
                    log.key();
                }
                editor.flush_pending_paste(cx)
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests;
