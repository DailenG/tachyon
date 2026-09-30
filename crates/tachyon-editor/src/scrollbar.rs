//! An overlay thumb on the right edge. Absent from the tree unless the document is taller than
//! the window. The thumb itself is painted only while scrolling, while the pointer is in the
//! edge strip, or while it is being dragged, and disappears a second later. No layout reservation
//! and no animation: a keystroke with the thumb hidden does not notify for it.

use std::time::Duration;

use gpui::{
    App, Context, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Window, canvas,
    div, point, prelude::*, px,
};

use crate::editor::Editor;
use crate::settings::{ScrollbarMode, Settings};

const HIDE_AFTER: Duration = Duration::from_secs(1);
const THUMB_MIN: f32 = 24.;
const EDGE: f32 = 12.;

/// Thumb top and height, in pixels from the top of a viewport `viewport` tall, for content
/// `content` tall scrolled by `scroll` (both >= 0). `None` when the content fits or there is
/// not enough room to show a draggable thumb.
pub fn thumb_geom(viewport: f32, content: f32, scroll: f32) -> Option<(f32, f32)> {
    if viewport <= THUMB_MIN || content <= viewport + 0.5 {
        return None;
    }
    let height = (viewport * viewport / content).clamp(THUMB_MIN, viewport);
    let track = viewport - height;
    let max_scroll = content - viewport;
    let y = track * (scroll / max_scroll).clamp(0., 1.);
    Some((y, height))
}

/// Pointer offset from the thumb's top for a mouse-down at `y`, both absolute in the same
/// coordinate space as `thumb_top`: the press point's offset inside the thumb when it lands on
/// the thumb (`thumb_top ..= thumb_top + thumb_h`), so the rest of the drag keeps that same grab
/// point under the pointer instead of re-centering the thumb on it. A press on the bare track
/// instead centers the thumb on the pointer, since a track click already means "go here".
fn grab_offset(y: f32, thumb_top: f32, thumb_h: f32) -> f32 {
    if y >= thumb_top && y <= thumb_top + thumb_h { y - thumb_top } else { thumb_h / 2. }
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

    /// A mouse-down inside the edge strip. Presses on the thumb keep the pointer at the same
    /// offset from its top for the rest of the drag; presses on the bare track center the thumb
    /// on the pointer instead, as a track click already implies "go here".
    pub(crate) fn scrollbar_drag_started(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let viewport = self.list.viewport_bounds();
        let Some((thumb_y, thumb_h)) = scrollbar_metrics(&self.list) else { return };
        let thumb_top = (viewport.origin.y + px(thumb_y)).as_f32();
        self.scrollbar_drag_offset = px(grab_offset(y.as_f32(), thumb_top, thumb_h));
        if !self.list.is_scrollbar_dragging() {
            self.list.scrollbar_drag_started();
        }
        self.scrollbar_drag_to(y, cx);
    }

    /// Moves the thumb so the pointer stays at `scrollbar_drag_offset` from its top, the offset
    /// `scrollbar_drag_started` captured when the drag began.
    pub(crate) fn scrollbar_drag_to(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let viewport = self.list.viewport_bounds();
        let max = self.list.max_offset_for_scrollbar().y;
        let Some((_, thumb_h)) = scrollbar_metrics(&self.list) else { return };
        let track = (viewport.size.height - px(thumb_h)).max(px(1.));
        let rel = (y - viewport.origin.y - self.scrollbar_drag_offset).clamp(px(0.), track);
        let offset = -max * (rel / track);
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
                        editor.scrollbar_drag_started(event.position.y, cx);
                    }),
                )
                // A window-level handler, not `.on_mouse_move`: GPUI calls that only while this
                // 12px-wide strip is hovered, so a drag would stop following the pointer the
                // moment it drifts sideways off the strip. This keeps tracking it anywhere in the
                // window; the `is_scrollbar_dragging` check below is what stops it once the drag
                // ends.
                .child({
                    let editor = cx.entity();
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _cx| {
                            let editor = editor.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if !phase.bubble() {
                                    return;
                                }
                                editor.update(cx, |editor, cx| {
                                    if editor.list.is_scrollbar_dragging()
                                        && event.pressed_button == Some(MouseButton::Left)
                                    {
                                        cx.stop_propagation();
                                        editor.scrollbar_drag_to(event.position.y, cx);
                                    }
                                });
                            });
                        },
                    )
                    .absolute()
                    .size_full()
                })
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
    use super::{grab_offset, thumb_geom};

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

    #[test]
    fn a_viewport_shorter_than_the_minimum_thumb_has_no_scrollbar() {
        assert_eq!(thumb_geom(10., 100., 0.), None);
        assert_eq!(thumb_geom(24., 100., 90.), None);
    }

    #[test]
    fn a_press_on_the_thumb_keeps_its_grab_point_a_press_off_it_centers() {
        // Thumb spans 40..90 (top 40, height 50).
        assert_eq!(grab_offset(45., 40., 50.), 5., "grabbed 5px below the thumb's top");
        assert_eq!(grab_offset(40., 40., 50.), 0., "grabbed exactly at the top");
        assert_eq!(grab_offset(90., 40., 50.), 50., "grabbed exactly at the bottom");
        assert_eq!(grab_offset(120., 40., 50.), 25., "track click centers the thumb");
        assert_eq!(grab_offset(10., 40., 50.), 25., "track click above the thumb also centers");
    }
}
