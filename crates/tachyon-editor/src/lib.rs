//! The block-swap Markdown editor: a GPUI view over a `tachyon_doc::Document`.
//! The block holding the caret is shown and edited as raw Markdown; all other
//! blocks are rendered with their syntax hidden.

mod about;
mod backup;
mod disk;
mod editor;
mod find;
mod frame_log;
mod frame_stats;
mod links;
mod lists;
mod movement;
mod picker;
mod prompt;
mod render;
mod settings;
mod theme;

pub use crate::about::{About, AboutView, AppInfo, open_about};
pub use crate::backup::{Backups, HotExit, Restored};
pub use crate::disk::{LoadOutcome, Loaded, load_document, oversized_markdown_notice};
pub use crate::editor::{
    ClipboardReader, CloseWindow, Editor, HtmlClipboard, KEY_CONTEXT, OpenPaths, Save, SaveAs,
    key_bindings,
};
pub use crate::picker::RecentFiles;
pub use crate::prompt::keyboard_prompt;
pub use crate::settings::{DEFAULT_SETTINGS, Settings, SettingsFile, ThemeChoice};
pub use crate::theme::{AppearanceHint, Theme};

/// Registers the editor's key bindings, and applies a large paste still being prepared before
/// any keystroke that cannot simply be queued after it, so edits keep their order (a key typed
/// right after Ctrl+V lands after the pasted text) without every keystroke waiting on the
/// paste's clipboard read and re-inserting the whole paste on its own frame: plain typed text
/// queues instead (see `Editor::replace_text_in_range`), landing once the paste does.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(key_bindings());
    cx.on_action(|_: &About, cx| {
        open_about(cx);
    });
    cx.intercept_keystrokes(|event, window, cx| {
        if let Some(Some(editor)) = window.root::<Editor>() {
            editor.update(cx, |editor, cx| {
                if let Some(log) = &mut editor.frame_log {
                    log.key();
                }
                // The keymap check only runs while a paste is pending, not on every keystroke.
                if editor.paste_pending() && !can_queue_as_text(&event.keystroke, cx) {
                    editor.flush_pending_paste(cx);
                }
            });
        }
    })
    .detach();
}

/// Whether `keystroke` can only ever become plain text input, so a pending paste can be queued
/// past it (`Editor::replace_text_in_range`) instead of flushed here before dispatch: no ctrl,
/// alt, platform or function modifier (shift is fine - it does not turn a character key into an
/// action), a printable `key_char` (a control character means a bound key whose `key_char`
/// happens to be set, like Enter's or Tab's, not typed text), and no key binding claims it.
/// Checked against the whole keymap with no context, so this only ever errs toward flushing
/// (broader than any one context's bindings, never narrower) - cheaper than resolving the actual
/// dispatch path, and it only runs while a paste is pending.
fn can_queue_as_text(keystroke: &gpui::Keystroke, cx: &gpui::App) -> bool {
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return false;
    }
    let Some(key_char) = &keystroke.key_char else { return false };
    if key_char.chars().any(char::is_control) {
        return false;
    }
    let input = std::slice::from_ref(keystroke);
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    !keymap.bindings().any(|binding| binding.match_keystrokes(input) == Some(false))
}

#[cfg(test)]
mod tests;
