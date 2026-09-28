//! User settings: a small `settings.toml` (a subset of TOML: `key = value` lines and `#` comments)
//! in the config directory. The application reads it at start-up, off the UI thread, and sets
//! [`Settings`]; saving the file from a Tachyon window applies it to every open window.

use gpui::{App, Context, Global};

use crate::editor::Editor;
use crate::theme::is_dark;

/// Which theme to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow the system appearance.
    System,
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Zoom of new windows, 0.5 to 3.0.
    pub zoom: f32,
    /// Keep unsaved documents when quitting (ADR 0006).
    pub hot_exit: bool,
    /// Show a faint rotating tip behind the document (`render::tip_overlay`).
    pub tips: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: ThemeChoice::System, zoom: 1., hot_exit: true, tips: true }
    }
}

impl Global for Settings {}

/// Written when the settings file is opened and does not exist yet.
pub const DEFAULT_SETTINGS: &str = "\
# Tachyon settings. Saving this file applies them to open windows.

# \"system\" follows the system appearance; or \"dark\", \"light\".
theme = \"system\"

# Zoom of new windows, from 0.5 to 3.0.
zoom = 1.0

# Keep unsaved documents when quitting, and reopen them at the next start.
hot_exit = true

# Show a faint rotating tip behind the document.
tips = true
";

impl Settings {
    /// Parses a settings file. Unknown keys and invalid values are skipped (reported in the
    /// second result, `line: problem`); missing keys keep their defaults.
    pub fn parse(text: &str) -> (Settings, Vec<String>) {
        let mut settings = Settings::default();
        let mut problems = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let number = index + 1;
            let Some((key, value)) = line.split_once('=') else {
                problems.push(format!("{number}: expected `key = value`"));
                continue;
            };
            let value = value.trim();
            let unquoted = value.trim_matches('"');
            match key.trim() {
                "theme" => match unquoted {
                    "system" => settings.theme = ThemeChoice::System,
                    "dark" => settings.theme = ThemeChoice::Dark,
                    "light" => settings.theme = ThemeChoice::Light,
                    _ => problems
                        .push(format!("{number}: theme is \"system\", \"dark\" or \"light\"")),
                },
                "zoom" => match value.parse::<f32>() {
                    Ok(zoom) if (0.5..=3.).contains(&zoom) => settings.zoom = zoom,
                    _ => problems.push(format!("{number}: zoom is a number from 0.5 to 3.0")),
                },
                "hot_exit" => match value {
                    "true" => settings.hot_exit = true,
                    "false" => settings.hot_exit = false,
                    _ => problems.push(format!("{number}: hot_exit is true or false")),
                },
                "tips" => match value {
                    "true" => settings.tips = true,
                    "false" => settings.tips = false,
                    _ => problems.push(format!("{number}: tips is true or false")),
                },
                other => problems.push(format!("{number}: unknown setting `{other}`")),
            }
        }
        (settings, problems)
    }

    /// Whether the theme is dark, given what the system prefers.
    pub fn dark(&self, system_dark: bool) -> bool {
        match self.theme {
            ThemeChoice::System => system_dark,
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
        }
    }
}

/// The settings file's path, so saving it applies the settings. Set by the application.
pub struct SettingsFile(pub std::path::PathBuf);

impl Global for SettingsFile {}

impl Editor {
    /// After a save: if it was the settings file, applies it to every window.
    pub(crate) fn settings_saved(&self, path: &std::path::Path, cx: &mut Context<Self>) {
        if cx.try_global::<SettingsFile>().is_none_or(|file| file.0 != path) {
            return;
        }
        let (settings, _problems) = Settings::parse(&self.text());
        cx.set_global(settings);
        // Deferred: this runs inside one window's update, and every window is updated.
        cx.defer(apply_to_windows);
    }
}

/// Applies an in-memory settings change to every open window and writes it to the settings
/// file, keeping the file's comments and other keys (see `set_setting_line`). Used by the
/// command palette's settings commands, which apply immediately rather than waiting for a save.
pub(crate) fn apply_setting(
    cx: &mut Context<Editor>,
    key: &'static str,
    value: &'static str,
    mutate: impl FnOnce(&mut Settings),
) {
    let mut settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
    mutate(&mut settings);
    cx.set_global(settings);
    cx.defer(apply_to_windows);
    let Some(path) = cx.try_global::<SettingsFile>().map(|file| file.0.clone()) else { return };
    cx.background_executor()
        .spawn(async move {
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_SETTINGS.to_owned());
            let updated = set_setting_line(&text, key, value);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = crate::editor::write_atomically(&path, updated.as_bytes());
        })
        .detach();
}

/// Rewrites `key`'s line in a settings file's text to `key = value`, keeping any trailing
/// comment on that line and every other line untouched; appends the line if `key` is missing.
fn set_setting_line(text: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len() + key.len() + value.len() + 4);
    let mut found = false;
    for line in text.split_inclusive('\n') {
        let (content, newline) = line.strip_suffix('\n').map_or((line, ""), |c| (c, "\n"));
        let code = content.split('#').next().unwrap_or("");
        let matches_key = !found && code.split_once('=').is_some_and(|(k, _)| k.trim() == key);
        if matches_key {
            found = true;
            let comment = &content[code.len()..];
            out.push_str(key);
            out.push_str(" = ");
            out.push_str(value);
            if !comment.is_empty() {
                out.push(' ');
                out.push_str(comment);
            }
        } else {
            out.push_str(content);
        }
        out.push_str(newline);
    }
    if !found {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(value);
        out.push('\n');
    }
    out
}

/// Applies changed settings to every open window: the theme, and (Windows) the native title
/// bar's and popup menus' dark/light mode, neither of which is GPUI's to draw and so does not
/// follow `apply_settings`'s own repaint.
fn apply_to_windows(cx: &mut App) {
    for window in cx.windows() {
        if let Some(editor) = window.downcast::<Editor>() {
            let _ = editor.update(cx, |editor, window, cx| {
                editor.apply_settings(window, cx);
                let settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
                let dark = settings.dark(is_dark(window.appearance()));
                tachyon_platform::set_title_bar_dark(window, dark);
                tachyon_platform::set_popup_menu_dark(dark);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_parses_to_the_defaults() {
        assert_eq!(Settings::parse(DEFAULT_SETTINGS), (Settings::default(), Vec::new()));
    }

    #[test]
    fn values_comments_and_problems() {
        let (settings, problems) = Settings::parse(
            "theme = \"light\"  # always\nzoom = 1.25\nhot_exit = false\ntips = false\nzoom = 9\ncolour = red\nnonsense\n",
        );
        assert_eq!(
            settings,
            Settings { theme: ThemeChoice::Light, zoom: 1.25, hot_exit: false, tips: false }
        );
        assert_eq!(
            problems,
            [
                "5: zoom is a number from 0.5 to 3.0",
                "6: unknown setting `colour`",
                "7: expected `key = value`",
            ]
        );
    }
}
