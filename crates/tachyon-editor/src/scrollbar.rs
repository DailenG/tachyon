//! An overlay thumb on the right edge. Absent from the tree unless the document is taller than
//! the window. The thumb itself is painted only while scrolling, while the pointer is in the
//! edge strip, or while it is being dragged, and disappears a second later. No layout reservation
//! and no animation: a keystroke with the thumb hidden does not notify for it.

use std::time::Duration;

use gpui::{
    App, Context, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Window, div,
    point, prelude::*, px,
};

use crate::editor::Editor;
use crate::settings::{ScrollbarMode, Settings};

const HIDE_AFTER: Duration = Duration::from_secs(1);
const THUMB_MIN: f32 = 24.;
const EDGE: f32 = 12.;

/// Thumb top and height, in pixels from the top of a viewport `viewport` tall, for content
/// `content` tall scrolled by `scroll` (both >= 0). `None` when the content fits.
pub fn thumb_geom(viewport: f32, content: f32, scroll: f32) -> Option<(f32, f32)> {
    if content <= viewport + 0.5 {
        return None;
    }
    let height = (viewport * viewport / content).clamp(THUMB_MIN, viewport);
    let track = (viewport - height).max(1.);
    let max_scroll = content - viewport;
    let y = track * (scroll / max_scroll).clamp(0., 1.);
    Some((y, height))
}

impl Editor {
    /// A scroll happened. Shows the thumb and restarts the hide timer. No notify when the thumb
    /// is already showing: the list's own scroll already scheduled a frame.
    pub(crate) fn scrollbar_scrolled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !scrollbar_on(cx) {
            return;
        }
        let was = self.scrollbar_visible;
        self.scrollbar_visible = true;
        self.arm_scrollbar_hide(window, cx);
        if !was {
            cx.notify();
        }
    }

    pub(crate) fn scrollbar_edge(
        &mut self,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scrollbar_edge = hovered;
        if hovered {
            self.scrollbar_visible = true;
            cx.notify();
        } else {
            self.arm_scrollbar_hide(window, cx);
        }
    }

    fn arm_scrollbar_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.scrollbar_generation = self.scrollbar_generation.wrapping_add(1);
        let generation = self.scrollbar_generation;
        self.scrollbar_hide = Some(cx.spawn_in(window, async move |editor, cx| {
            cx.background_executor().timer(HIDE_AFTER).await;
            let _ = editor.update(cx, |editor, cx| {
                if editor.scrollbar_generation != generation {
                    return;
                }
                editor.scrollbar_hide = None;
                if editor.scrollbar_edge || editor.list.is_scrollbar_dragging() {
                    return;
                }
                if editor.scrollbar_visible {
                    editor.scrollbar_visible = false;
                    cx.notify();
                }
            });
        }));
    }

    pub(crate) fn scrollbar_drag_to(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let viewport = self.list.viewport_bounds();
        let max = self.list.max_offset_for_scrollbar().y;
        let Some((_, thumb_h)) = scrollbar_metrics(&self.list) else { return };
        let track = (viewport.size.height - px(thumb_h)).max(px(1.));
        let rel = (y - viewport.origin.y - px(thumb_h) / 2.).clamp(px(0.), track);
        let offset = -max * (rel / track);
        if !self.list.is_scrollbar_dragging() {
            self.list.scrollbar_drag_started();
        }
        self.list.set_offset_from_scrollbar(point(px(0.), offset));
        self.scrollbar_visible = true;
        cx.notify();
    }

    pub(crate) fn scrollbar_drag_ended(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.is_scrollbar_dragging() {
            self.list.scrollbar_drag_ended();
        }
        self.arm_scrollbar_hide(window, cx);
    }

    /// The edge strip, and the thumb when it should be seen. `None` when the setting is off or
    /// the document fits: nothing in the tree, so a keystroke does not pay for it.
    pub(crate) fn scrollbar_layer(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !scrollbar_on(cx) {
            return None;
        }
        let (thumb_y, thumb_h) = scrollbar_metrics(&self.list)?;
        let viewport = self.list.viewport_bounds();
        let show =
            self.scrollbar_visible || self.scrollbar_edge || self.list.is_scrollbar_dragging();
        let theme = &self.theme;
        Some(
            div()
                .id("scrollbar-edge")
                .absolute()
                .top(viewport.origin.y)
                .left(viewport.origin.x + viewport.size.width - px(EDGE))
                .w(px(EDGE))
                .h(viewport.size.height)
                .on_hover(cx.listener(|editor, hovered: &bool, window, cx| {
                    editor.scrollbar_edge(*hovered, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|editor, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        editor.scrollbar_drag_to(event.position.y, cx);
                    }),
                )
                .on_mouse_move(cx.listener(|editor, event: &MouseMoveEvent, _, cx| {
                    if editor.list.is_scrollbar_dragging()
                        && event.pressed_button == Some(MouseButton::Left)
                    {
                        cx.stop_propagation();
                        editor.scrollbar_drag_to(event.position.y, cx);
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|editor, _, window, cx| editor.scrollbar_drag_ended(window, cx)),
                )
                .when(show, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(thumb_y))
                            .right(px(2.))
                            .w(px(6.))
                            .h(px(thumb_h))
                            .rounded(theme.radius_small)
                            .bg(theme.text.muted),
                    )
                })
                .into_any_element(),
        )
    }
}

fn scrollbar_on(cx: &App) -> bool {
    cx.try_global::<Settings>().is_none_or(|settings| settings.scrollbar == ScrollbarMode::Auto)
}

fn scrollbar_metrics(list: &ListState) -> Option<(f32, f32)> {
    let viewport = list.viewport_bounds().size.height.as_f32();
    let max = list.max_offset_for_scrollbar().y.as_f32();
    if max <= 0.5 {
        return None;
    }
    let scroll = (-list.scroll_px_offset_for_scrollbar().y.as_f32()).max(0.);
    thumb_geom(viewport, max + viewport, scroll)
}

#[cfg(test)]
mod tests {
    use super::thumb_geom;

    #[test]
    fn a_document_that_fits_has_no_thumb() {
        assert_eq!(thumb_geom(400., 400., 0.), None);
        assert_eq!(thumb_geom(400., 200., 0.), None);
    }

    #[test]
    fn the_thumb_tracks_the_scroll_and_stays_at_least_24px() {
        let (y, height) = thumb_geom(200., 800., 0.).unwrap();
        assert_eq!(height, 50.);
        assert_eq!(y, 0.);
        let (y, _) = thumb_geom(200., 800., 600.).unwrap();
        assert!((y - 150.).abs() < 0.1, "scrolled to the end, thumb at the bottom");
        let (_, short) = thumb_geom(100., 10_000., 0.).unwrap();
        assert_eq!(short, 24.);
    }
}
