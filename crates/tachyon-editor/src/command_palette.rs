//! The command palette (`Ctrl+Shift+P`/`Cmd+Shift+P`, also `F1`, or a right-click): every
//! user-facing action and setting in
//! one filterable list, reusing the picker overlay (`picker.rs`). `Ctrl+P`/`Cmd+P` opens the
//! same Open recent picker as a quick-open shortcut (an extra binding in `editor.rs`'s
//! `key_bindings`), so there is nothing new to build for it here.
//!
//! `COMMANDS` is the single table a new feature adds a row to: a display name and what picking
//! it does, either running a gpui action by its registered name (also used to look up and show
//! the action's current keyboard shortcut) or applying-and-persisting a setting. Nothing here
//! runs before the palette is opened, and the picker it builds is dropped when it closes.

use gpui::{App, Context, Window};

use crate::editor::{Editor, OpenCommandPalette};
use crate::picker::{Item, Pick, Picker};
use crate::settings::{ContentWidth, Settings, ThemeChoice};

/// What picking a command does.
#[derive(Clone, Copy)]
pub(crate) enum CommandEffect {
    /// Runs the gpui action registered under this name (for example `"editor::Save"` or
    /// `"tachyon::Quit"`); also the key used to look up and show the action's current shortcut.
    Action(&'static str),
    /// Sets the theme, in memory and in the settings file.
    Theme(ThemeChoice),
    /// Flips hot exit, in memory and in the settings file.
    ToggleHotExit,
    /// Flips whether the What's new window shows itself after an update, in memory and in the
    /// settings file.
    ToggleWhatsNew,
    /// Flips whether the whole session (open files, window bounds, caret and scroll) is
    /// restored at the next start, in memory and in the settings file.
    ToggleRestoreSession,
    /// Sets the text column width, in memory and in the settings file.
    ContentWidth(ContentWidth),
    /// Opens the picker of stored sticky notes.
    ReopenNote,
    /// Flips the overlay scrollbar between auto and never.
    ToggleScrollbar,
}

/// One row's name and what it does.
struct Command {
    name: &'static str,
    effect: CommandEffect,
}

/// Every command the palette lists, in the order shown when the filter is empty. A new feature
/// adds one row here.
const COMMANDS: &[Command] = &[
    Command { name: "New window", effect: CommandEffect::Action("tachyon::NewWindow") },
    Command { name: "New sticky note", effect: CommandEffect::Action("tachyon::NewNote") },
    Command { name: "Reopen sticky note...", effect: CommandEffect::ReopenNote },
    Command { name: "Open", effect: CommandEffect::Action("tachyon::Open") },
    Command { name: "Save", effect: CommandEffect::Action("editor::Save") },
    Command { name: "Save As", effect: CommandEffect::Action("editor::SaveAs") },
    Command { name: "Close window", effect: CommandEffect::Action("editor::CloseWindow") },
    Command { name: "Quit", effect: CommandEffect::Action("tachyon::Quit") },
    Command { name: "Find", effect: CommandEffect::Action("editor::Find") },
    Command { name: "Replace", effect: CommandEffect::Action("editor::Replace") },
    Command { name: "Find next", effect: CommandEffect::Action("editor::FindNext") },
    Command { name: "Find previous", effect: CommandEffect::Action("editor::FindPrevious") },
    Command { name: "Go to heading", effect: CommandEffect::Action("editor::GoToHeading") },
    Command { name: "Open recent", effect: CommandEffect::Action("editor::OpenRecent") },
    Command { name: "Copy as HTML", effect: CommandEffect::Action("editor::CopyAsHtml") },
    Command {
        name: "Toggle plain-text mode",
        effect: CommandEffect::Action("editor::ToggleTextMode"),
    },
    Command { name: "Zoom in", effect: CommandEffect::Action("editor::ZoomIn") },
    Command { name: "Zoom out", effect: CommandEffect::Action("editor::ZoomOut") },
    Command { name: "Zoom reset", effect: CommandEffect::Action("editor::ZoomReset") },
    Command { name: "Undo", effect: CommandEffect::Action("editor::Undo") },
    Command { name: "Redo", effect: CommandEffect::Action("editor::Redo") },
    Command { name: "Select all", effect: CommandEffect::Action("editor::SelectAll") },
    Command { name: "Document start", effect: CommandEffect::Action("editor::DocumentStart") },
    Command { name: "Document end", effect: CommandEffect::Action("editor::DocumentEnd") },
    Command {
        name: "Frame-time overlay",
        effect: CommandEffect::Action("editor::ToggleFrameStats"),
    },
    Command { name: "Open settings file", effect: CommandEffect::Action("tachyon::OpenSettings") },
    Command { name: "About Tachyon", effect: CommandEffect::Action("about::About") },
    Command { name: "What's new", effect: CommandEffect::Action("tachyon::WhatsNew") },
    Command { name: "Theme: System", effect: CommandEffect::Theme(ThemeChoice::System) },
    Command { name: "Theme: Light", effect: CommandEffect::Theme(ThemeChoice::Light) },
    Command { name: "Theme: Dark", effect: CommandEffect::Theme(ThemeChoice::Dark) },
    Command { name: "Hot exit", effect: CommandEffect::ToggleHotExit },
    Command { name: "Show what's new after updates", effect: CommandEffect::ToggleWhatsNew },
    Command { name: "Restore session on start", effect: CommandEffect::ToggleRestoreSession },
    Command { name: "Scrollbar", effect: CommandEffect::ToggleScrollbar },
    Command { name: "Width: 680px", effect: CommandEffect::ContentWidth(ContentWidth::Px(680.)) },
    Command { name: "Width: 820px", effect: CommandEffect::ContentWidth(ContentWidth::Px(820.)) },
    Command { name: "Width: 1100px", effect: CommandEffect::ContentWidth(ContentWidth::Px(1100.)) },
    Command {
        name: "Width: 100%",
        effect: CommandEffect::ContentWidth(ContentWidth::Percent(100.)),
    },
];

/// The settings file's value syntax for a theme choice (quoted, matching `Settings::parse`).
fn theme_value(choice: ThemeChoice) -> &'static str {
    match choice {
        ThemeChoice::System => "\"system\"",
        ThemeChoice::Light => "\"light\"",
        ThemeChoice::Dark => "\"dark\"",
    }
}

/// The settings file's value syntax for one of `COMMANDS`' own content-width presets (quoted,
/// matching `Settings::parse`). Exact float patterns are safe: both sides are the same literals.
fn content_width_value(width: ContentWidth) -> &'static str {
    match width {
        ContentWidth::Px(680.) => "\"680px\"",
        ContentWidth::Px(1100.) => "\"1100px\"",
        ContentWidth::Percent(100.) => "\"100%\"",
        _ => "\"820px\"",
    }
}

/// The current shortcut for `action_name`, written the way the README and menus write keys
/// (`Ctrl+Shift+S`, `Cmd+Shift+S` on macOS, `F3`), not GPUI's `ctrl-shift-s` - `None` if
/// nothing is bound. There is no user keymap layer to override the compiled-in one, so the
/// first matching binding wins: by convention `key_bindings()` lists an action's primary binding
/// before any secondary alias (an alternate OS convention, or another key for the same chord), so
/// the first is always the one worth showing (`Redo`'s `secondary-shift-z` over its `ctrl-y`
/// alias, `Replace`'s `ctrl-h` over its macOS-only `cmd-alt-f`).
fn shortcut_for(keymap: &gpui::Keymap, action_name: &str) -> Option<String> {
    keymap.bindings().find(|binding| binding.action().name() == action_name).map(|binding| {
        binding.keystrokes().iter().map(format_keystroke).collect::<Vec<_>>().join(" ")
    })
}

/// One keystroke as `Ctrl+Alt+Shift+Key`, the README's form: `Cmd` leads on macOS
/// (`Cmd+Shift+P`), and elsewhere the platform key is `Win`, after the others.
pub(crate) fn format_keystroke(keystroke: &gpui::KeybindingKeystroke) -> String {
    let keystroke = keystroke.inner();
    let m = keystroke.modifiers;
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<&str> = Vec::new();
    if m.platform && mac {
        parts.push("Cmd");
    }
    if m.control {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push(if mac { "Option" } else { "Alt" });
    }
    if m.shift {
        parts.push("Shift");
    }
    if m.platform && !mac {
        parts.push("Win");
    }
    if m.function {
        parts.push("Fn");
    }
    let key = key_name(&keystroke.key);
    parts.push(&key);
    parts.join("+")
}

/// A key's display name: letters upper-cased, named keys capitalised (`F3`, `Home`, `PageUp`).
fn key_name(key: &str) -> String {
    match key {
        "pageup" => "PageUp".to_owned(),
        "pagedown" => "PageDown".to_owned(),
        "escape" => "Esc".to_owned(),
        _ => {
            let mut chars = key.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// Builds the palette's rows from `COMMANDS`: each action's live shortcut from the keymap, and
/// each setting's current value marked (a checkmark on the active theme; the live value in Hot
/// exit's own name). The checkmark is a separate `Item::marked` flag, never folded into `label`
/// itself: see that field's doc comment for why (folding it in would make the three theme rows
/// rank differently, and so reorder, depending only on which one happens to be current).
fn command_items(cx: &App) -> Vec<Item> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
    COMMANDS
        .iter()
        .map(|command| {
            let (label, marked, shortcut) = match command.effect {
                CommandEffect::Action("tachyon::NewNote") => (
                    command.name.to_owned(),
                    false,
                    tachyon_platform::global_hotkey_label(&settings.sticky_hotkey),
                ),
                CommandEffect::Action(action_name) => {
                    (command.name.to_owned(), false, shortcut_for(&keymap, action_name))
                }
                CommandEffect::Theme(choice) => {
                    (command.name.to_owned(), settings.theme == choice, None)
                }
                CommandEffect::ToggleHotExit => (
                    format!("{}: {}", command.name, if settings.hot_exit { "On" } else { "Off" }),
                    false,
                    None,
                ),
                CommandEffect::ToggleWhatsNew => (
                    format!("{}: {}", command.name, if settings.whats_new { "On" } else { "Off" }),
                    false,
                    None,
                ),
                CommandEffect::ToggleRestoreSession => (
                    format!(
                        "{}: {}",
                        command.name,
                        if settings.restore_session { "On" } else { "Off" }
                    ),
                    false,
                    None,
                ),
                CommandEffect::ToggleScrollbar => (
                    format!(
                        "{}: {}",
                        command.name,
                        if settings.scrollbar == crate::settings::ScrollbarMode::Auto {
                            "Auto"
                        } else {
                            "Off"
                        }
                    ),
                    false,
                    None,
                ),
                CommandEffect::ReopenNote => (command.name.to_owned(), false, None),
                CommandEffect::ContentWidth(width) => {
                    (command.name.to_owned(), settings.content_width == width, None)
                }
            };
            Item {
                label,
                detail: None,
                indent: 0,
                shortcut,
                marked,
                pick: Pick::Command(command.effect),
            }
        })
        .collect()
}

impl Editor {
    /// `Ctrl+Shift+P`/`Cmd+Shift+P`: opens the command palette, built fresh from `COMMANDS` each
    /// time (nothing is created while it is closed).
    pub(crate) fn open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = command_items(cx);
        self.open_picker(Picker::new("Command Palette", "no commands", items, 0), cx);
    }

    /// Runs a command chosen from the palette: dispatches its action on this window through the
    /// same node-dispatch path a keystroke would (not `App::dispatch_action`'s active-window
    /// lookup, which the picker itself, already focused in this window, does not need), or
    /// applies and persists its setting.
    pub(crate) fn run_command(
        &mut self,
        effect: &CommandEffect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match *effect {
            CommandEffect::Action(name) => {
                if let Ok(action) = cx.build_action(name, None) {
                    window.dispatch_action(action, cx);
                }
            }
            CommandEffect::Theme(choice) => {
                crate::settings::apply_setting(cx, "theme", theme_value(choice), move |settings| {
                    settings.theme = choice;
                });
            }
            CommandEffect::ToggleHotExit => {
                let current = cx.try_global::<Settings>().is_none_or(|s| s.hot_exit);
                let next = !current;
                let value = if next { "true" } else { "false" };
                crate::settings::apply_setting(cx, "hot_exit", value, move |settings| {
                    settings.hot_exit = next;
                });
            }
            CommandEffect::ToggleWhatsNew => {
                let current = cx.try_global::<Settings>().is_none_or(|s| s.whats_new);
                let next = !current;
                let value = if next { "true" } else { "false" };
                crate::settings::apply_setting(cx, "whats_new", value, move |settings| {
                    settings.whats_new = next;
                });
            }
            CommandEffect::ToggleRestoreSession => {
                let current = cx.try_global::<Settings>().is_none_or(|s| s.restore_session);
                let next = !current;
                let value = if next { "true" } else { "false" };
                crate::settings::apply_setting(cx, "restore_session", value, move |settings| {
                    settings.restore_session = next;
                });
            }
            CommandEffect::ReopenNote => self.open_note_picker(cx),
            CommandEffect::ToggleScrollbar => {
                let current = cx
                    .try_global::<Settings>()
                    .is_none_or(|s| s.scrollbar == crate::settings::ScrollbarMode::Auto);
                let next = !current;
                let value = if next { "\"auto\"" } else { "\"never\"" };
                crate::settings::apply_setting(cx, "scrollbar", value, move |settings| {
                    settings.scrollbar = if next {
                        crate::settings::ScrollbarMode::Auto
                    } else {
                        crate::settings::ScrollbarMode::Never
                    };
                });
            }

            CommandEffect::ContentWidth(width) => {
                crate::settings::apply_setting(
                    cx,
                    "content_width",
                    content_width_value(width),
                    move |settings| {
                        settings.content_width = width;
                    },
                );
            }
        }
    }
}
