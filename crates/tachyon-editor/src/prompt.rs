//! In-window prompt for platforms without native dialogs (GPUI's fallback
//! prompt is mouse-only). Keyboard: Enter/Space choose the selected button,
//! Tab/arrows move the selection, Escape chooses the cancel button.

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, KeyDownEvent, PromptButton,
    PromptHandle, PromptLevel, PromptResponse, Render, RenderablePromptHandle, SharedString,
    Window, div, prelude::*, px,
};

use crate::theme::Theme;

/// Renders prompts with [`KeyboardPrompt`]. Install with
/// `cx.set_prompt_builder(tachyon_editor::keyboard_prompt)`; this replaces
/// native dialogs, so only do it where the platform has none.
pub fn keyboard_prompt(
    _level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let prompt = cx.new(|cx| KeyboardPrompt {
        message: message.to_owned().into(),
        detail: detail.map(|d| d.to_owned().into()),
        actions: actions.iter().map(|a| a.label().clone()).collect(),
        cancel: cancel_index(actions),
        selected: 0,
        focus: cx.focus_handle(),
        theme: Theme::dark(),
    });
    handle.with_view(prompt, window, cx)
}

/// The button Escape chooses: an explicit cancel button, else one labelled
/// "Cancel", else the last.
fn cancel_index(actions: &[PromptButton]) -> usize {
    actions
        .iter()
        .position(|a| matches!(a, PromptButton::Cancel(_)))
        .or_else(|| actions.iter().position(|a| a.label().eq_ignore_ascii_case("cancel")))
        .unwrap_or(actions.len().saturating_sub(1))
}

pub struct KeyboardPrompt {
    message: SharedString,
    detail: Option<SharedString>,
    actions: Vec<SharedString>,
    cancel: usize,
    selected: usize,
    focus: FocusHandle,
    theme: Theme,
}

impl KeyboardPrompt {
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.actions.len().max(1);
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "enter" | "space" => cx.emit(PromptResponse(self.selected)),
            "escape" => cx.emit(PromptResponse(self.cancel)),
            "tab" if keystroke.modifiers.shift => {
                self.selected = (self.selected + count - 1) % count
            }
            "up" | "left" => self.selected = (self.selected + count - 1) % count,
            "tab" | "down" | "right" => self.selected = (self.selected + 1) % count,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl EventEmitter<PromptResponse> for KeyboardPrompt {}

impl Focusable for KeyboardPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for KeyboardPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let buttons = self.actions.iter().enumerate().map(|(ix, label)| {
            let selected = ix == self.selected;
            div()
                .id(ix)
                .px_3()
                .py_1()
                .rounded_md()
                .border_1()
                .cursor_pointer()
                .border_color(if selected { theme.accent } else { theme.rule })
                .when(selected, |d| d.bg(theme.accent).text_color(theme.background))
                .child(label.clone())
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(PromptResponse(ix));
                    cx.stop_propagation();
                }))
        });
        let panel = div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .w(px(380.))
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.rule)
            .bg(theme.raw_background)
            .text_color(theme.foreground)
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(FontWeight::BOLD).child(self.message.clone()))
            .children(self.detail.clone().map(|d| div().text_color(theme.muted).child(d)))
            .child(div().flex().justify_end().gap_2().mt_2().children(buttons));
        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .bg(gpui::hsla(0., 0., 0., 0.45))
            .flex()
            .items_center()
            .justify_center()
            .child(panel)
    }
}
