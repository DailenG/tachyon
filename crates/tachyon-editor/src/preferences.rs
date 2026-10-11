//! The Preferences panel (`Ctrl+,`/`Cmd+,`, or the command palette's "Preferences" row): every
//! setting in one filterable list, with its current value and what it does, reusing the picker
//! overlay (`picker.rs`). Picking a row edits that setting in place: an on/off setting flips, a
//! setting with a few values steps to the next one, and any other opens a field prefilled with
//! the current value (Enter applies it, Escape goes back to the list). Every change applies to
//! every open window at once and is written to `settings.toml`, keeping its comments.
//!
//! The panel lists [`KEYS`] against the live [`Settings`], so a settings file that is empty or
//! from an older version still shows every setting there is. The last row restores every
//! default (after asking), keeping the old file as `settings.toml.bak`. "Open settings file" in
//! the command palette still opens the file itself.
//!
//! Nothing here runs until the panel opens, and the picker it builds is dropped when it closes.

use gpui::{Context, PromptLevel, Window};

use crate::editor::{Editor, OpenPreferences};
use crate::picker::{Item, Pick, Picker};
use crate::settings::{KEYS, Kind, Settings, apply_setting, description, reset_settings};

/// The panel's title, also what its picker is recognized by when a value changes under it.
pub(crate) const TITLE: &str = "Preferences";

/// What picking a Preferences row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreferencePick {
    /// Edits [`KEYS`]`[i]`.
    Edit(usize),
    /// Restores every default, after asking.
    Reset,
}

/// A value as shown in the list: quotes dropped, an empty hotkey shown as "off".
fn shown(key: &str, value: &str) -> String {
    let value = value.trim_matches('"');
    match (key, value) {
        (_, "true") => "On".to_owned(),
        (_, "false") => "Off".to_owned(),
        ("sticky_hotkey", "") => "off".to_owned(),
        _ => value.to_owned(),
    }
}

/// The panel's rows for `settings`: each setting's name and current value, its description as
/// the muted detail (also matched by the filter, so typing "sticky" or "diagram" finds rows),
/// and the reset row last.
fn items(settings: &Settings) -> Vec<Item> {
    let mut items: Vec<Item> = KEYS
        .iter()
        .enumerate()
        .map(|(i, &(key, name, _))| {
            let value = settings.value(key).unwrap_or_default();
            Item {
                label: format!("{name}: {}", shown(key, &value)),
                detail: Some(description(key)),
                indent: 0,
                shortcut: None,
                marked: false,
                pick: Pick::Preference(PreferencePick::Edit(i)),
            }
        })
        .collect();
    items.push(Item {
        label: "Reset all settings to defaults...".to_owned(),
        detail: Some("Keeps the current file as settings.toml.bak".to_owned()),
        indent: 0,
        shortcut: None,
        marked: false,
        pick: Pick::Preference(PreferencePick::Reset),
    });
    items
}

/// The value after `current` in `values`, wrapping (the first if `current` is not one of them).
fn next_choice(values: &[&'static str], current: &str) -> &'static str {
    let at = values.iter().position(|v| *v == current);
    values[at.map_or(0, |i| (i + 1) % values.len())]
}

/// Text typed into the value field, in `settings.toml` syntax: a value that is not a number,
/// `true`/`false`, or already quoted is quoted, so typing `80%` or `win+shift+n` just works.
fn as_file_value(typed: &str) -> String {
    let typed = typed.trim();
    let bare = typed.parse::<f32>().is_ok() || typed == "true" || typed == "false";
    if bare || (typed.len() >= 2 && typed.starts_with('"') && typed.ends_with('"')) {
        typed.to_owned()
    } else {
        format!("\"{typed}\"")
    }
}

impl Editor {
    /// `Ctrl+,`/`Cmd+,`: opens the Preferences panel with the first row chosen.
    pub(crate) fn open_preferences(
        &mut self,
        _: &OpenPreferences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_preferences(0, cx);
    }

    /// Opens (or rebuilds, after a change) the panel with item `item` chosen (an index into its
    /// rows: `KEYS`' order, then reset), keeping whatever filter was typed.
    fn show_preferences(&mut self, item: usize, cx: &mut Context<Self>) {
        let settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
        // The list's filter: the open list's own, or the one saved when the value field opened.
        let query = match &self.picker {
            Some(open) if open.title == TITLE => match &open.editing {
                Some(edit) => edit.list_query.clone(),
                None => open.query.clone(),
            },
            _ => String::new(),
        };
        let mut picker = Picker::new(TITLE, "no settings", items(&settings), 0);
        picker.set_query(query);
        picker.select_item(item);
        self.open_picker(picker, cx);
    }

    /// Runs a Preferences row. Flags and choices change at once and the list stays open on the
    /// same row; a free-form value opens the field; reset asks first.
    pub(crate) fn pick_preference(
        &mut self,
        pick: PreferencePick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = match pick {
            PreferencePick::Edit(index) => index,
            PreferencePick::Reset => return self.confirm_reset_settings(window, cx),
        };
        let Some(&(key, name, kind)) = KEYS.get(index) else { return };
        let settings = cx.try_global::<Settings>().cloned().unwrap_or_default();
        let current = settings.value(key).unwrap_or_default();
        let next = match kind {
            Kind::Flag => (if current == "true" { "false" } else { "true" }).to_owned(),
            Kind::Choice(values) => next_choice(values, &current).to_owned(),
            Kind::Value => {
                // The list's filter, so Enter or Escape returns to the same filtered list.
                let list_query = self.picker.as_ref().map(|p| p.query.clone()).unwrap_or_default();
                let mut picker = Picker::new(TITLE, "", Vec::new(), 0);
                picker.editing = Some(crate::picker::ValueEdit { index, name, list_query });
                picker.query = current.trim_matches('"').to_owned();
                self.open_picker(picker, cx);
                return;
            }
        };
        self.apply_preference(index, &next, cx);
    }

    /// Enter in the value field: applies the typed value if [`Settings::set`] accepts it and
    /// returns to the list; otherwise keeps the field open with the reason shown.
    pub(crate) fn commit_preference_value(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else { return };
        let Some(index) = picker.editing.as_ref().map(|edit| edit.index) else { return };
        let value = as_file_value(&picker.query);
        let mut probe = cx.try_global::<Settings>().cloned().unwrap_or_default();
        match probe.set(KEYS[index].0, &value) {
            Ok(()) => self.apply_preference(index, &value, cx),
            Err(problem) => {
                picker.empty = "";
                picker.problem = Some(problem);
                cx.notify();
            }
        }
    }

    /// Escape in the value field: back to the list, on the same row, nothing changed.
    pub(crate) fn cancel_preference_value(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.picker.as_ref().and_then(|p| p.editing.as_ref()).map(|e| e.index)
        else {
            return false;
        };
        self.show_preferences(index, cx);
        true
    }

    /// Applies an already valid `value` (file syntax) for [`KEYS`]`[index]` and shows the list
    /// again with that row chosen.
    fn apply_preference(&mut self, index: usize, value: &str, cx: &mut Context<Self>) {
        let key = KEYS[index].0;
        let owned = value.to_owned();
        apply_setting(cx, key, value, move |settings| {
            // Validated by the caller; `set` cannot fail here.
            let _ = settings.set(key, &owned);
        });
        self.show_preferences(index, cx);
    }

    /// The reset row: asks, then restores every default and refreshes the open panel.
    fn confirm_reset_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            "Reset all settings to their defaults?",
            Some("The current settings file is kept as settings.toml.bak."),
            &["Reset", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let reset = answer.await.ok() == Some(0);
            let _ = this.update(cx, |editor, cx| {
                if reset {
                    reset_settings(cx);
                }
                // Back to the panel either way, on the reset row.
                editor.show_preferences(KEYS.len(), cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_has_a_row_and_a_description() {
        let defaults = Settings::default();
        for &(key, _, kind) in KEYS {
            let value = defaults.value(key).unwrap_or_else(|| panic!("{key} has no value"));
            assert!(!description(key).is_empty(), "{key} has no comment in DEFAULT_SETTINGS");
            if let Kind::Choice(values) = kind {
                assert!(values.contains(&value.as_str()), "{key}'s default {value} is a choice");
            }
            // What `value` writes, `set` reads back unchanged.
            let mut round = Settings { mermaid: !defaults.mermaid, ..Settings::default() };
            round.set(key, &value).unwrap_or_else(|e| panic!("{key} = {value}: {e}"));
            assert_eq!(round.value(key), Some(value));
        }
        // ...and every key the default file writes is listed.
        let in_file: Vec<&str> = crate::DEFAULT_SETTINGS
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once('=').map(|(k, _)| k.trim()))
            .collect();
        let listed: Vec<&str> = KEYS.iter().map(|&(key, _, _)| key).collect();
        assert_eq!(in_file, listed);
    }

    #[test]
    fn typed_values_are_quoted_unless_they_are_numbers_flags_or_quoted() {
        assert_eq!(as_file_value("1.25"), "1.25");
        assert_eq!(as_file_value(" true "), "true");
        assert_eq!(as_file_value("80%"), "\"80%\"");
        assert_eq!(as_file_value("win+shift+n"), "\"win+shift+n\"");
        assert_eq!(as_file_value("\"820px\""), "\"820px\"");
        assert_eq!(as_file_value(""), "\"\"");
    }

    #[test]
    fn choices_step_and_wrap() {
        let values = &["\"system\"", "\"light\"", "\"dark\""];
        assert_eq!(next_choice(values, "\"system\""), "\"light\"");
        assert_eq!(next_choice(values, "\"dark\""), "\"system\"");
        assert_eq!(next_choice(values, "\"bogus\""), "\"system\"");
    }
}
