//! The rotating "Pro Tip" line drawn faintly behind the document (`Settings::tips`; rendered in
//! `render::tip_overlay`). Built from the real key bindings (`editor::key_bindings`) so the key
//! shown always matches what is actually bound, instead of a copy that can drift from it; the one
//! tip about a mouse gesture (not in the keymap) uses the same secondary-key label
//! `gpui::Modifiers::secondary_key()` resolves to.

use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::KeybindingKeystroke;

use crate::editor::{self, Cancel, Find, GoToHeading, OpenRecent, ToggleTextMode};

/// "Cmd" on macOS, "Ctrl" everywhere else: what `secondary-` keybindings and
/// `Modifiers::secondary_key()` both resolve to, spelled out for tip text that names a gesture
/// with no keymap entry to read it from (`Ctrl+click`).
#[cfg(target_os = "macos")]
const SECONDARY: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const SECONDARY: &str = "Ctrl";

/// One key-bound tip: the action whose bound key opens the sentence, and the rest of it.
struct KeyTip {
    matches: fn(&dyn gpui::Action) -> bool,
    rest: &'static str,
}

const KEY_TIPS: &[KeyTip] = &[
    KeyTip {
        matches: |a| a.as_any().is::<ToggleTextMode>(),
        rest: "switches between Markdown and plain text",
    },
    KeyTip { matches: |a| a.as_any().is::<Cancel>(), rest: "leaves editing" },
    KeyTip { matches: |a| a.as_any().is::<OpenRecent>(), rest: "opens a recent file" },
    KeyTip { matches: |a| a.as_any().is::<Find>(), rest: "finds text in the document" },
    KeyTip { matches: |a| a.as_any().is::<GoToHeading>(), rest: "jumps to a heading" },
];

/// "Ctrl+Shift+M" from a bound keystroke's modifiers and key, in the product's own casing (the
/// keymap's own `Display` impl instead prints macOS symbols or lowercase hyphenated words, which
/// read like debug output rather than UI copy).
fn format_keystroke(keystroke: &KeybindingKeystroke) -> String {
    let modifiers = keystroke.modifiers();
    let mut parts = Vec::with_capacity(4);
    if modifiers.control {
        parts.push("Ctrl".to_owned());
    }
    if modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if modifiers.platform {
        parts.push(if cfg!(target_os = "macos") { "Cmd" } else { "Win" }.to_owned());
    }
    if modifiers.shift {
        parts.push("Shift".to_owned());
    }
    parts.push(key_label(keystroke.key()));
    parts.join("+")
}

/// A single key named for display: `"m"` -> `"M"`, `"escape"` -> `"Escape"`.
fn key_label(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Every tip whose action still has a bound key (skipped otherwise, rather than showing a stale
/// or missing one), plus the fixed mouse-gesture tip, each as "Pro Tip: <label> <rest>".
fn rendered_tips() -> Vec<String> {
    let bindings = editor::key_bindings();
    let mut tips: Vec<String> = KEY_TIPS
        .iter()
        .filter_map(|tip| {
            let binding = bindings.iter().find(|b| (tip.matches)(b.action()))?;
            let label = format_keystroke(binding.keystrokes().first()?);
            Some(format!("Pro Tip: {label} {}", tip.rest))
        })
        .collect();
    tips.push(format!("Pro Tip: {SECONDARY}+click opens a link"));
    tips
}

/// `tips[index % tips.len()]`, `None` only if `rendered_tips` is somehow empty (never in
/// practice - the mouse-gesture tip always exists). Split out from `next_tip` so the rotation
/// itself is testable without sharing `NEXT`'s process-wide state with every other test.
fn tip_at(index: usize) -> Option<gpui::SharedString> {
    let tips = rendered_tips();
    if tips.is_empty() {
        return None;
    }
    Some(tips[index % tips.len()].clone().into())
}

/// Process-wide, so windows opened one after another rotate through the list instead of each
/// starting over at the first tip.
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// This window's tip: the next one in the rotation (see `tip_at`).
pub(crate) fn next_tip() -> Option<gpui::SharedString> {
    tip_at(NEXT.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tip_starts_with_the_label_and_a_real_bound_key() {
        for tip in rendered_tips() {
            assert!(tip.starts_with("Pro Tip: "), "{tip:?}");
        }
        let toggle = rendered_tips()
            .into_iter()
            .find(|t| t.contains("Markdown and plain text"))
            .expect("ToggleTextMode is bound in key_bindings()");
        assert_eq!(toggle, "Pro Tip: Ctrl+Shift+M switches between Markdown and plain text");
    }

    #[test]
    fn the_rotation_steps_through_every_tip_before_repeating() {
        let tips = rendered_tips();
        let seen: Vec<_> = (0..tips.len()).map(|i| tip_at(i).unwrap().to_string()).collect();
        assert_eq!(seen, tips, "index i picks tips[i] while i is in range");
        assert_eq!(tip_at(tips.len()).unwrap().to_string(), tips[0], "wraps back to the start");
    }
}
