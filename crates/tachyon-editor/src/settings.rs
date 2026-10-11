//! User settings: a small `settings.toml` (a subset of TOML: `key = value` lines and `#` comments)
//! in the config directory. The application reads it at start-up, off the UI thread, and sets
//! [`Settings`]; saving the file from a Tachyon window applies it to every open window.

use gpui::{App, Context, Global, Pixels, px};

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

/// The text column's width, as written in `settings.toml`: a fixed logical-pixel size
/// (`"820px"`, or a bare `"820"` meaning the same thing), scaled by zoom like other sizes, or a
/// percentage of the window's own content width (`"80%"`), re-evaluated on resize and never
/// scaled by zoom again since it is already relative to the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContentWidth {
    Px(f32),
    Percent(f32),
}

/// Below this, a pixel width would not leave a usable column; values under it are rejected
/// (falling back to the default) rather than silently forced up to it, matching how every other
/// out-of-range setting here behaves (see `zoom`).
const CONTENT_WIDTH_MIN_PX: f32 = 200.;

/// The only useful percentage range: above 100 the column could never actually get that wide
/// (see `ContentWidth::resolve`), and 0 or below leaves no column at all.
const CONTENT_WIDTH_PERCENT_RANGE: (f32, f32) = (1., 100.);

impl Default for ContentWidth {
    fn default() -> Self {
        ContentWidth::Px(820.)
    }
}

impl ContentWidth {
    /// Parses a CSS-style content width: `"820px"`, a bare `"820"` (equivalent to `"820px"`), or
    /// a percentage like `"80%"`. `None` for anything malformed or out of range, so the caller
    /// can fall back to the default exactly like every other invalid setting.
    fn parse(value: &str) -> Option<ContentWidth> {
        let value = value.trim();
        if let Some(percent) = value.strip_suffix('%') {
            let percent: f32 = percent.trim().parse().ok()?;
            let (min, max) = CONTENT_WIDTH_PERCENT_RANGE;
            return (percent >= min && percent <= max).then_some(ContentWidth::Percent(percent));
        }
        let px_part = value.strip_suffix("px").unwrap_or(value).trim();
        let value: f32 = px_part.parse().ok()?;
        (value >= CONTENT_WIDTH_MIN_PX).then_some(ContentWidth::Px(value))
    }

    /// The column width for one frame: the setting's own request - a pixel size scaled by
    /// `zoom`, or a share of `window_width` (already window-relative, so `zoom` does not scale
    /// it again) - capped to `window_width` less `gap` on each side, so text keeps a minimum gap
    /// from the window frame at every width and zoom, including `100 %`. Pure arithmetic: O(1),
    /// no allocation, safe to call every frame (`Editor::render`).
    pub fn resolve(self, zoom: f32, window_width: Pixels, gap: Pixels) -> Pixels {
        let requested = match self {
            ContentWidth::Px(value) => px(value) * zoom,
            ContentWidth::Percent(percent) => window_width * (percent / 100.),
        };
        requested.min((window_width - gap * 2.).max(px(0.)))
    }
}

/// `Settings::sticky_hotkey`'s own default: see the setting's doc comment.
const DEFAULT_STICKY_HOTKEY: &str = "win+shift+n";

/// When the overlay scrollbar is drawn. `Auto` shows a thumb only while scrolling or hovering
/// the right edge. `Never` draws nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollbarMode {
    #[default]
    Auto,
    Never,
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
    /// Show the What's new window once, after an update (`tachyon::whats_new`). The command
    /// palette's "What's new" row opens it on demand regardless of this setting.
    pub whats_new: bool,
    /// Write the whole session (open files, window bounds, caret and scroll) on Quit, and
    /// restore it at the next start with no files on the command line (issue #79, on top of hot
    /// exit's own unsaved-document restore, ADR 0006). Off: today's behavior, unsaved documents
    /// only, and the session file is neither written nor read.
    pub restore_session: bool,
    /// The text column's width (`render::Editor::render_block`, `tip_overlay`).
    pub content_width: ContentWidth,
    /// The global hotkey that opens a new sticky note (issue #69), as written in
    /// `settings.toml` (`tachyon_platform::Hotkey::parse`'s own syntax, e.g. "win+shift+n");
    /// empty means off. Windows only - `tachyon_platform::register_global_hotkey` answers
    /// `HotkeyError::Unsupported` elsewhere, which is treated the same as empty (no hotkey),
    /// not reported as an error.
    pub sticky_hotkey: String,
    /// Whether a new sticky note starts always-on-top; a per-window toggle
    /// (`Editor::toggle_note_pin`, `Ctrl+Shift+T`) can still turn it off for one note.
    pub sticky_on_top: bool,
    /// A sticky note's opacity while its window is not focused, 0.3 to 1.0; 1.0 (the
    /// default) turns translucency off. Focused, a note is always fully opaque. Normal
    /// (non-note) windows are never affected.
    pub sticky_unfocused_opacity: f32,
    /// Whether opening a file, restoring a session, or a whole-document paste starts with no
    /// block raw for editing. Off: today's behavior, the caret's block is raw immediately.
    pub start_without_active_block: bool,
    /// Overlay scrollbar. `Never` draws nothing. `Auto` shows a thumb only while scrolling or
    /// while the pointer is on the right edge.
    pub scrollbar: ScrollbarMode,
    /// Render ```mermaid fenced code blocks as diagrams (`mermaid.rs`). Off: they show as code.
    pub mermaid: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::System,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: ContentWidth::default(),
            sticky_hotkey: DEFAULT_STICKY_HOTKEY.to_owned(),
            sticky_on_top: true,
            sticky_unfocused_opacity: 1.0,
            start_without_active_block: false,
            scrollbar: ScrollbarMode::Auto,
            mermaid: true,
        }
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

# Show what's new after an update, once, the next time Tachyon starts. false only stops that
# automatic prompt; the command palette's \"What's new\" row still opens the notes any time.
whats_new = true

# Restore the whole session (open files, window position and size, caret and scroll) the next
# time Tachyon starts with no files given on the command line. false: only unsaved documents come
# back (hot exit), as before this setting existed.
restore_session = true

# Text column width: a pixel size like \"820px\" (or a bare 820), scaled by zoom, or a
# percentage of the window like \"80%\"; \"100%\" is the widest the column can get, and a
# minimum gap to the window frame always remains.
content_width = \"820px\"

# Sticky notes (issue #69): a global hotkey that opens a new one, e.g. \"win+shift+n\"
# (Windows only; empty turns it off). Linux users bind `tachyon --note` in their
# compositor instead (see the README).
sticky_hotkey = \"win+shift+n\"

# Whether a new sticky note starts always-on-top; the pin button and Ctrl+Shift+T still
# toggle it per note.
sticky_on_top = true

# A sticky note\'s opacity while unfocused, from 0.3 to 1.0; 1.0 turns this off. Focused,
# a note is always opaque.
sticky_unfocused_opacity = 1.0

# Open a file, restore a session, or paste a whole document with every block rendered. false
# (the default) starts with the caret's block raw, ready to edit. A click or keystroke edits
# either way.
start_without_active_block = false

# Overlay scrollbar: \"auto\" shows a thumb while scrolling or hovering the right edge;
# \"never\" draws none. It never reserves layout width.
scrollbar = \"auto\"

# Draw ```mermaid code blocks as diagrams; false shows them as code. Rendering happens in the
# background, so a document with no diagrams costs nothing.
mermaid = true
";

/// How the Preferences panel edits a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `true`/`false`: picking the row flips it.
    Flag,
    /// One of these values (as written in the file, quotes included): picking cycles them.
    Choice(&'static [&'static str]),
    /// Anything [`Settings::set`] accepts: picking opens a field to type it.
    Value,
}

/// Every setting, in the order `DEFAULT_SETTINGS` lists them, with the name the Preferences
/// panel shows. A new setting adds a row here, a field, a `set`/`value` arm, and its commented
/// line in `DEFAULT_SETTINGS` (a test checks they agree).
pub(crate) const KEYS: &[(&str, &str, Kind)] = &[
    ("theme", "Theme", Kind::Choice(&["\"system\"", "\"light\"", "\"dark\""])),
    ("zoom", "Zoom of new windows", Kind::Value),
    ("hot_exit", "Hot exit", Kind::Flag),
    ("tips", "Tips", Kind::Flag),
    ("whats_new", "What's new after updates", Kind::Flag),
    ("restore_session", "Restore session on start", Kind::Flag),
    ("content_width", "Text column width", Kind::Value),
    ("sticky_hotkey", "Sticky note hotkey", Kind::Value),
    ("sticky_on_top", "Sticky notes on top", Kind::Flag),
    ("sticky_unfocused_opacity", "Sticky note opacity when unfocused", Kind::Value),
    ("start_without_active_block", "Open with every block rendered", Kind::Flag),
    ("scrollbar", "Scrollbar", Kind::Choice(&["\"auto\"", "\"never\""])),
    ("mermaid", "Mermaid diagrams", Kind::Flag),
];

/// `key`'s description: its comment in `DEFAULT_SETTINGS`, joined into one line.
pub(crate) fn description(key: &str) -> String {
    let lines: Vec<&str> =
        default_comment(key).into_iter().map(|line| line.trim_start_matches('#').trim()).collect();
    lines.join(" ").replace("\\\"", "\"")
}

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
            if let Err(problem) = settings.set(key.trim(), value.trim()) {
                problems.push(format!("{number}: {problem}"));
            }
        }
        (settings, problems)
    }

    /// Sets one setting from its `settings.toml` spelling (`value` as written after `=`, quotes
    /// optional). The file parser and the Preferences panel both go through this, so a value is
    /// accepted or rejected the same way in either place. `Err` describes the valid values.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let unquoted = value.trim_matches('"');
        let flag = |name: &str| match unquoted {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("{name} is true or false")),
        };
        match key {
            "theme" => {
                self.theme = match unquoted {
                    "system" => ThemeChoice::System,
                    "dark" => ThemeChoice::Dark,
                    "light" => ThemeChoice::Light,
                    _ => return Err("theme is \"system\", \"dark\" or \"light\"".into()),
                }
            }
            "zoom" => match unquoted.parse::<f32>() {
                Ok(zoom) if (0.5..=3.).contains(&zoom) => self.zoom = zoom,
                _ => return Err("zoom is a number from 0.5 to 3.0".into()),
            },
            "hot_exit" => self.hot_exit = flag(key)?,
            "tips" => self.tips = flag(key)?,
            "whats_new" => self.whats_new = flag(key)?,
            "restore_session" => self.restore_session = flag(key)?,
            "content_width" => match ContentWidth::parse(unquoted) {
                Some(width) => self.content_width = width,
                None => {
                    return Err(format!(
                        "content_width is a pixel width of at least {CONTENT_WIDTH_MIN_PX}px (e.g. \"820px\") or a percentage from 1% to 100% (e.g. \"80%\")"
                    ));
                }
            },
            "sticky_hotkey" => {
                if unquoted.is_empty() || tachyon_platform::Hotkey::parse(unquoted).is_some() {
                    self.sticky_hotkey = unquoted.to_owned();
                } else {
                    return Err(
                        "sticky_hotkey is empty (off) or a combination like \"win+shift+n\"".into(),
                    );
                }
            }
            "sticky_on_top" => self.sticky_on_top = flag(key)?,
            "sticky_unfocused_opacity" => match unquoted.parse::<f32>() {
                Ok(opacity) if (0.3..=1.0).contains(&opacity) => {
                    self.sticky_unfocused_opacity = opacity;
                }
                _ => return Err("sticky_unfocused_opacity is a number from 0.3 to 1.0".into()),
            },
            "start_without_active_block" => self.start_without_active_block = flag(key)?,
            "scrollbar" => {
                self.scrollbar = match unquoted {
                    "auto" => ScrollbarMode::Auto,
                    "never" => ScrollbarMode::Never,
                    _ => return Err("scrollbar is \"auto\" or \"never\"".into()),
                }
            }
            "mermaid" => self.mermaid = flag(key)?,
            other => return Err(format!("unknown setting `{other}`")),
        }
        Ok(())
    }

    /// One setting's current value in `settings.toml` syntax (strings quoted, as
    /// `DEFAULT_SETTINGS` writes them): what the Preferences panel shows and what it writes.
    /// `None` for an unknown key.
    pub fn value(&self, key: &str) -> Option<String> {
        let quoted = |s: &str| format!("\"{s}\"");
        let flag = |b: bool| if b { "true" } else { "false" }.to_owned();
        Some(match key {
            "theme" => quoted(match self.theme {
                ThemeChoice::System => "system",
                ThemeChoice::Dark => "dark",
                ThemeChoice::Light => "light",
            }),
            "zoom" => self.zoom.to_string(),
            "hot_exit" => flag(self.hot_exit),
            "tips" => flag(self.tips),
            "whats_new" => flag(self.whats_new),
            "restore_session" => flag(self.restore_session),
            "content_width" => quoted(&match self.content_width {
                ContentWidth::Px(value) => format!("{value}px"),
                ContentWidth::Percent(percent) => format!("{percent}%"),
            }),
            "sticky_hotkey" => quoted(&self.sticky_hotkey),
            "sticky_on_top" => flag(self.sticky_on_top),
            "sticky_unfocused_opacity" => self.sticky_unfocused_opacity.to_string(),
            "start_without_active_block" => flag(self.start_without_active_block),
            "scrollbar" => quoted(match self.scrollbar {
                ScrollbarMode::Auto => "auto",
                ScrollbarMode::Never => "never",
            }),
            "mermaid" => flag(self.mermaid),
            _ => return None,
        })
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

/// Windows: ties hot exit's `hot_exit` setting to `RegisterApplicationRestart`/
/// `UnregisterApplicationRestart` (issue #80 - `tachyon_platform::register_restart`,
/// `tachyon_platform::unregister_restart`). Set only by a resident primary instance
/// (`crates/tachyon/src/app.rs`'s `Lifecycle`); a standalone (`-n`) or secondary process never
/// sets it, so it never registers itself for a restart nothing would be around to use hot exit's
/// backups after. Plain `fn` pointers (no captured state, like `RecentFilesOs`), so the struct
/// stays `Copy` and cheap to store as a global. Absent everywhere but the primary (and, in
/// practice, everywhere but Windows: `tachyon_platform`'s Linux and macOS implementations are
/// no-ops, so setting this there would just cost two pointless calls).
#[derive(Clone, Copy)]
pub struct RestartRegistration {
    pub register: fn(),
    pub unregister: fn(),
}

impl Global for RestartRegistration {}

/// Registers or unregisters this process for an OS restart, matching the current `hot_exit`
/// setting: called once after the first window's first frame (or immediately for a windowless
/// `--background` primary, which has no first frame to wait for) and again by [`apply_to_windows`]
/// whenever the setting changes (a settings-file save or the command palette's toggle). A no-op
/// if [`RestartRegistration`] was never set - not a resident primary instance, so there is
/// nothing to tie to the setting.
pub fn sync_restart_registration(cx: &mut App) {
    let Some(restart) = cx.try_global::<RestartRegistration>().copied() else { return };
    let hot_exit = cx.try_global::<Settings>().is_none_or(|s| s.hot_exit);
    if hot_exit { (restart.register)() } else { (restart.unregister)() }
}

/// Ties the `sticky_hotkey` setting to registering/unregistering a system-wide hotkey
/// (issue #69), the same `RestartRegistration` pattern: a `Copy` struct of one plain `fn`
/// pointer, set only by a resident primary instance (`crates/tachyon/src/app.rs`'s
/// `Lifecycle`) - a standalone (`-n`) or secondary process never registers a global hotkey
/// of its own. `sync`'s own implementation (`crates/tachyon/src/app.rs`) diffs the setting
/// against whatever is currently registered, so calling this when nothing changed is cheap.
#[derive(Clone, Copy)]
pub struct StickyHotkeyRegistration {
    pub sync: fn(&mut App),
}

impl Global for StickyHotkeyRegistration {}

/// Called once after the first window's first frame (or immediately for a windowless
/// `--background` primary), and again whenever `sticky_hotkey` changes (a settings-file
/// save; see `apply_to_windows`). A no-op if `StickyHotkeyRegistration` was never set - not
/// a resident primary instance, so there is nothing to tie the setting to.
pub fn sync_sticky_hotkey(cx: &mut App) {
    if let Some(reg) = cx.try_global::<StickyHotkeyRegistration>().copied() {
        (reg.sync)(cx);
    }
}

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
/// command palette's settings commands and the Preferences panel, which apply immediately rather
/// than waiting for a save.
pub(crate) fn apply_setting(
    cx: &mut Context<Editor>,
    key: &'static str,
    value: impl Into<String>,
    mutate: impl FnOnce(&mut Settings),
) {
    let mut settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
    mutate(&mut settings);
    cx.set_global(settings);
    cx.defer(apply_to_windows);
    let Some(path) = cx.try_global::<SettingsFile>().map(|file| file.0.clone()) else { return };
    let value = value.into();
    cx.background_executor()
        .spawn(async move {
            let text =
                std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_SETTINGS.to_owned());
            let updated = set_setting_line(&text, key, &value);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = crate::editor::write_atomically(&path, updated.as_bytes());
        })
        .detach();
}

/// Restores every setting to its default, in memory and on disk: the settings file is replaced
/// by [`DEFAULT_SETTINGS`] (every key, each with its comment), the previous file kept beside it
/// as `settings.toml.bak` so a reset is never a loss.
pub(crate) fn reset_settings(cx: &mut Context<Editor>) {
    cx.set_global(Settings::default());
    cx.defer(apply_to_windows);
    let Some(path) = cx.try_global::<SettingsFile>().map(|file| file.0.clone()) else { return };
    cx.background_executor()
        .spawn(async move {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(old) = std::fs::read(&path)
                && old != DEFAULT_SETTINGS.as_bytes()
            {
                let _ = crate::editor::write_atomically(&backup_path(&path), &old);
            }
            let _ = crate::editor::write_atomically(&path, DEFAULT_SETTINGS.as_bytes());
        })
        .detach();
}

/// Where [`reset_settings`] keeps the file it replaced: the same name with `.bak` appended.
pub(crate) fn backup_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    name.into()
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
        // Appended with a blank line and the key's own comment from `DEFAULT_SETTINGS`, so it
        // never reads as belonging to whatever comment the file happens to end with (a comment
        // left behind when its own line was deleted, for example).
        if !out.is_empty() {
            out.push('\n');
        }
        for comment in default_comment(key) {
            out.push_str(comment);
            out.push('\n');
        }
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(value);
        out.push('\n');
    }
    out
}

/// The comment lines directly above `key`'s line in `DEFAULT_SETTINGS`, in order; empty for a key
/// the default file does not have.
fn default_comment(key: &str) -> Vec<&'static str> {
    let mut comment = Vec::new();
    for line in DEFAULT_SETTINGS.lines() {
        if line.starts_with('#') {
            comment.push(line);
        } else if line.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            return comment;
        } else {
            comment.clear();
        }
    }
    Vec::new()
}

/// Applies changed settings to every open window: the theme, and (Windows) the native title
/// bar's and popup menus' dark/light mode, neither of which is GPUI's to draw and so does not
/// follow `apply_settings`'s own repaint; also [`sync_restart_registration`], which has no
/// per-window effect but belongs with every other side effect of a settings change.
fn apply_to_windows(cx: &mut App) {
    sync_restart_registration(cx);
    sync_sticky_hotkey(cx);
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
    fn a_missing_key_is_appended_with_its_own_comment_not_under_a_stray_one() {
        // The winlab file: `tips = true` deleted by hand, its comment left as the last line.
        let text = "theme = \"dark\"\n\n# Show a faint rotating tip behind the document.\n";
        let updated = set_setting_line(text, "restore_session", "false");
        let expected_tail =
            "# Show a faint rotating tip behind the document.\n\n# Restore the whole session";
        assert!(updated.contains(expected_tail), "{updated}");
        assert!(updated.ends_with("restore_session = false\n"), "{updated}");
        assert!(!Settings::parse(&updated).0.restore_session);
    }

    #[test]
    fn the_default_file_parses_to_the_defaults() {
        assert_eq!(Settings::parse(DEFAULT_SETTINGS), (Settings::default(), Vec::new()));
    }

    #[test]
    fn values_comments_and_problems() {
        let (settings, problems) = Settings::parse(
            "theme = \"light\"  # always\nzoom = 1.25\nhot_exit = false\ntips = false\n\
             whats_new = false\nrestore_session = false\nzoom = 9\ncolor = red\nnonsense\n",
        );
        assert_eq!(
            settings,
            Settings {
                theme: ThemeChoice::Light,
                zoom: 1.25,
                hot_exit: false,
                tips: false,
                whats_new: false,
                restore_session: false,
                content_width: ContentWidth::default(),
                ..Default::default()
            }
        );
        assert_eq!(
            problems,
            [
                "7: zoom is a number from 0.5 to 3.0",
                "8: unknown setting `color`",
                "9: expected `key = value`",
            ]
        );
    }

    #[test]
    fn an_invalid_restore_session_value_falls_back_to_the_default_and_is_reported() {
        let (settings, problems) = Settings::parse("restore_session = maybe\n");
        assert!(settings.restore_session, "falls back to the default (on)");
        assert_eq!(problems, ["1: restore_session is true or false"]);
    }

    #[test]
    fn content_width_parses_pixels_and_a_bare_number() {
        assert_eq!(ContentWidth::parse("820px"), Some(ContentWidth::Px(820.)));
        assert_eq!(ContentWidth::parse("680px"), Some(ContentWidth::Px(680.)));
        assert_eq!(ContentWidth::parse("820"), Some(ContentWidth::Px(820.)), "a bare number");
    }

    #[test]
    fn content_width_parses_percentages() {
        assert_eq!(ContentWidth::parse("80%"), Some(ContentWidth::Percent(80.)));
        assert_eq!(ContentWidth::parse(" 100% "), Some(ContentWidth::Percent(100.)));
    }

    #[test]
    fn content_width_rejects_invalid_values() {
        for invalid in ["abc", "-5px", "0%", "250%", "", "820em"] {
            assert_eq!(ContentWidth::parse(invalid), None, "{invalid:?} should be invalid");
        }
    }

    #[test]
    fn content_width_enforces_its_range_bounds() {
        // Right at the edges of the accepted range: still valid.
        assert_eq!(ContentWidth::parse("200px"), Some(ContentWidth::Px(200.)), "the px minimum");
        assert_eq!(ContentWidth::parse("1%"), Some(ContentWidth::Percent(1.)));
        assert_eq!(ContentWidth::parse("100%"), Some(ContentWidth::Percent(100.)));
        // Just outside them: rejected, not silently pulled back in.
        assert_eq!(ContentWidth::parse("199px"), None, "just under the px minimum");
        assert_eq!(ContentWidth::parse("101%"), None, "just over the percent maximum");
    }

    #[test]
    fn an_invalid_content_width_falls_back_to_the_default_without_failing_the_file() {
        let (settings, problems) = Settings::parse("content_width = \"250%\"\n");
        assert_eq!(settings.content_width, ContentWidth::default());
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn content_width_resolve_scales_pixels_by_zoom_and_percent_by_window_width() {
        let gap = px(16.);
        assert_eq!(
            ContentWidth::Px(820.).resolve(2., px(4000.), gap),
            px(1640.),
            "a pixel width scales with zoom"
        );
        assert_eq!(
            ContentWidth::Percent(50.).resolve(2., px(2000.), gap),
            px(1000.),
            "a percentage is a share of the window, not scaled again by zoom"
        );
    }

    #[test]
    fn content_width_resolve_always_leaves_the_frame_gap_even_at_100_percent() {
        let gap = px(16.);
        let window_width = px(1400.);
        let resolved = ContentWidth::Percent(100.).resolve(1., window_width, gap);
        assert_eq!(resolved, window_width - gap * 2.);
        assert!(
            resolved + gap * 2. <= window_width,
            "the column plus both gaps never exceeds the window: {resolved:?}"
        );
    }
}
