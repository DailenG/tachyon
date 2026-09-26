//! Phase 1 view: shows Markdown source as plain wrapped text. Replaced by the
//! block-swap editor in `tachyon-editor`.

use std::path::PathBuf;

use gpui::{
    Context, FocusHandle, IntoElement, Render, SharedString, Window, div, prelude::*, px, relative,
    rgb,
};

use crate::app::CloseWindow;

const BACKGROUND: u32 = 0x1e1f22;
const FOREGROUND: u32 = 0xd8dade;

pub struct RawView {
    text: SharedString,
    focus_handle: FocusHandle,
}

impl RawView {
    pub fn new(text: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self { text: normalize(text), focus_handle }
    }

    /// Opens with empty content and fills it once the file has been read on
    /// the background executor, so window creation never waits on disk I/O.
    pub fn load(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = Self::new(SharedString::default(), window, cx);
        cx.spawn(async move |this, cx| {
            let read_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(read_path) })
                .await;
            let text = match result {
                Ok(text) => SharedString::from(text),
                Err(e) => format!("Could not read {}: {e}", path.display()).into(),
            };
            this.update(cx, |view, cx| {
                view.text = normalize(text);
                cx.notify();
            })
        })
        .detach();
        view
    }
}

fn normalize(text: SharedString) -> SharedString {
    if text.contains('\r') { text.replace("\r\n", "\n").into() } else { text }
}

impl Render for RawView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("raw-view")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .text_size(px(15.))
            .line_height(relative(1.6))
            .child(div().max_w(px(820.)).mx_auto().px_8().py_6().child(self.text.clone()))
    }
}
