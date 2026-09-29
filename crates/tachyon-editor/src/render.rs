//! Block rendering: the block holding the caret shows its raw Markdown; every
//! other block shows its rendered IR with syntax hidden.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, ElementInputHandler, Entity, FontWeight, HighlightStyle, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels, Render, SharedString, Size,
    StyledImage as _, StyledText, TextLayout, UnderlineStyle, Window, WindowControlArea, canvas,
    div, fill, img, list, prelude::*, px, relative, size,
};
use tachyon_doc::raw_segment_lens;
use tachyon_md::{BlockKind, LineInfo, LineKind, Marker, ParsedBlock};

use crate::editor::{Editor, KEY_CONTEXT, TextTarget};
use crate::theme::Theme;

const INDENT: f32 = 22.;

/// A raw (active) block's source, or a rendered block's line list, is shown as one piece up to
/// this many bytes/lines. Above it, [`Editor::render_raw`] splits the source with
/// [`raw_segment_lens`] (smaller than [`DocMode::Plain`]'s own chunking: shaping the one segment
/// a keystroke touches must stay cheap regardless of how large the rest of the block is - see
/// [`raw_segment_lens`]'s own doc comment) and [`Editor::render_rendered`] windows `ir.lines`, so
/// a Markdown block that collapses into one enormous unit - a huge fenced code block, or a whole
/// paragraph with no blank lines - never hands GPUI megabytes of text or tens of thousands of
/// child elements to shape and lay out at once, active or not. Below this, both behave exactly
/// as before (one `StyledText`/one child per line): the split only changes anything for a block
/// this large.
pub(crate) const RAW_SPLIT_THRESHOLD: usize = tachyon_doc::PLAIN_CHUNK_BYTES * 2;

/// Same idea as [`RAW_SPLIT_THRESHOLD`] for [`Editor::render_rendered`]'s per-line loop: below
/// this many lines, every line still gets a real element, as before.
const LINE_SPLIT_THRESHOLD: usize = 2_000;

/// How many viewport-heights of a large block's own segments/lines get a real element built and
/// laid out, on each side of the one estimated nearest the viewport (see
/// [`Editor::render_window`]) - generous enough that ordinary scrolling never has to wait a
/// frame for a segment to appear, while still bounding a frame's work to a small, fixed multiple
/// of the viewport instead of the whole block, regardless of how tall one segment/line is (a
/// forced-cut segment of one huge unwrapped line can itself be many rows tall).
const RENDER_WINDOW_OVERDRAW_VIEWPORTS: f32 = 2.;

/// How many blocks on each side of the previous frame's drawn range ([`Editor::rendered`])
/// [`Document::evict`] leaves resident, in addition to the drawn range itself and the caret's
/// own block: generous enough that ordinary scrolling never evicts a block only to need it back
/// a frame or two later, while still bounding steady-state memory to a small, fixed multiple of
/// what is on screen instead of the whole document (report fix 6).
const EVICT_MARGIN_BLOCKS: usize = 200;

/// Minimum gap kept between the resolved text column and the window frame, in rems (so it
/// scales with zoom): real padding the owner wants felt at every width, wider than the one-rem
/// text inset (`px_4`) `render_block`'s own centred column always applies inside whatever this
/// leaves. Only ever narrows the column below its requested width - `680px`/`820px` in an
/// ordinary window are far under this and unaffected (`ContentWidth::resolve`).
const CONTENT_WIDTH_GAP_REMS: f32 = 3.;

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_title(window);
        if let Some(stats) = &mut self.frame_stats {
            stats.begin();
        }
        if let Some(log) = &mut self.frame_log {
            log.begin();
        }
        // Set again below if the document caret is actually painted this frame.
        self.caret_painted = false;
        let editor = cx.entity();
        let focus = self.focus.clone();
        // Spacing given in rems (padding, gaps) follows the zoom.
        window.set_rem_size(self.theme.scaled(BASE_REM_SIZE));
        let viewport = window.viewport_size();
        // The column's width for this frame: the setting's own request, capped so a real gap
        // (`CONTENT_WIDTH_GAP_REMS` per side, scaled by zoom) always remains between the column
        // and the window frame, at every width and zoom, including 100 % (`ContentWidth::
        // resolve`'s own doc comment) - wider than `render_block`'s own one-rem text inset
        // (`px_4`), which stays unchanged and still applies inside whatever column this leaves.
        // Pure arithmetic against the viewport GPUI already reports, so a resize costs nothing
        // beyond the re-wrap its own changed available width already causes.
        self.theme.content_width = self.content_width.resolve(
            self.theme.zoom,
            viewport.width,
            // A sticky note is small: a one-rem frame gap, not the full one.
            window.rem_size() * if self.is_note() { 1. } else { CONTENT_WIDTH_GAP_REMS },
        );
        // `ListState` caches each item's own measured size; nothing else tells it that a
        // block's wrapped height changed when only its *available width* did - a plain window
        // resize changing a percentage-based width, not just a settings save or zoom step (both
        // already remeasure explicitly). Comparing against last frame's resolved value catches
        // every cause in one place, including the very first frame (`None` always differs).
        if self.resolved_content_width != Some(self.theme.content_width) {
            self.resolved_content_width = Some(self.theme.content_width);
            self.list.remeasure();
        }
        // Snapshotted here, before `list(...)` below borrows `self.list`'s `ListState` for its
        // own layout pass: `render_window` (called from inside that pass, through `render_block`)
        // must not call back into `ListState` itself (it already holds the same `RefCell`
        // mutably for the whole pass), so it reads this cache instead. One frame stale, exactly
        // like `active_layout`'s own reliance on the previous frame's paint - self-correcting,
        // since a wrong window this frame only redraws slightly more or less than ideal, and the
        // caret's own segment/line is always included regardless (`render_window`'s `keep`).
        // Drop `ir` for blocks well outside the previous frame's drawn range
        // (`self.rendered`, still holding last frame's value here): keeps memory proportional
        // to what is shown, not the whole document (`Document::evict`, report fix 6). One frame
        // stale like `window_item_bounds` below - self-correcting, since `render_block`'s
        // `ensure_ir` restores a block synchronously the moment it is actually drawn, even if
        // this window turns out to have been wrong (a jump, not a smooth scroll).
        let keep_start = self.rendered.start.saturating_sub(EVICT_MARGIN_BLOCKS);
        let keep_end =
            (self.rendered.end + EVICT_MARGIN_BLOCKS).min(self.doc.blocks().len()).max(keep_start);
        let mut keep = keep_start..keep_end;
        if let Some(active) = self.active_block() {
            keep = keep.start.min(active)..keep.end.max(active + 1);
        }
        self.doc.evict(keep);
        self.window_viewport = self.list.viewport_bounds();
        self.window_item_bounds = self
            .rendered
            .clone()
            .filter_map(|i| self.list.bounds_for_item(i).map(|b| (i, b)))
            .collect();
        // Translucency (issue #69): a sticky note renders at `sticky_unfocused_opacity` while
        // its window is not active, header included - focused, or an ordinary window, it is
        // always fully opaque.
        let note_opacity = if self.is_note() && !window.is_window_active() {
            cx.try_global::<crate::settings::Settings>()
                .map_or(1.0, |settings| settings.sticky_unfocused_opacity)
        } else {
            1.0
        };
        // A note's compact layout keeps a smaller frame gap (1 rem, matching `px_4`'s own
        // one-rem text inset) than an ordinary window's `py_6` (1.5 rem).
        let list_padding_y = window.rem_size() * if self.is_note() { 1. } else { 1.5 };
        div()
            .id("editor")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(self.theme.surface.canvas)
            .text_color(self.theme.text.primary)
            .font_family(self.theme.text_font.clone())
            .text_size(self.theme.text_size)
            .line_height(relative(1.6))
            .opacity(note_opacity)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_drop(|paths: &gpui::ExternalPaths, _, cx| {
                if let Some(open) = cx.try_global::<crate::editor::OpenPaths>().map(|o| o.0.clone())
                {
                    open(paths.paths().to_vec(), cx);
                }
            })
            .on_action(cx.listener(Self::shift_newline))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::go_to_heading))
            .on_action(cx.listener(Self::open_recent))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::replace_bar))
            .on_action(cx.listener(Self::replace_all))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::copy_as_html))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::toggle_note_pin))
            .on_action(cx.listener(Self::toggle_frame_stats))
            .on_action(cx.listener(Self::toggle_text_mode))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|editor, event: &MouseDownEvent, window, cx| {
                    window.focus(&editor.focus, cx);
                    editor.mouse_down_in_margin(
                        event.position,
                        event.modifiers,
                        event.click_count,
                        window,
                        cx,
                    );
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|editor, _, _, _| editor.selecting = false))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.selecting = false),
            )
            .children(self.note_header(cx))
            .children(self.mode_notice())
            .children(self.tip_overlay())
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|editor, index, window, cx| {
                        editor.render_block(index, window, cx)
                    }),
                )
                .w_full()
                .flex_1()
                .py(list_padding_y),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, editor.clone()),
                            cx,
                        );
                        // Painted after the list: the editor's frame ends here.
                        editor.update(cx, |editor, _| {
                            editor.rendered = editor.rendering.take().unwrap_or(0..0);
                            if let Some(stats) = &mut editor.frame_stats {
                                stats.end();
                            }
                            let (drawn, blocks) =
                                (editor.rendered.len(), editor.doc.blocks().len());
                            if let Some(log) = &mut editor.frame_log {
                                log.end(drawn, blocks);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(self.frame_stats_overlay())
            .children(self.find_bar(viewport, cx))
            .children(self.picker_bar(viewport, cx))
    }
}

impl Editor {
    /// The find bar: the query with a caret and the match count. At most the window's width
    /// less the margins; with Replace open in a narrow window the fields wrap.
    fn find_bar(&self, viewport: Size<Pixels>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let theme = &self.theme;
        let max_width = viewport.width - OVERLAY_MARGIN * 2.;
        let field_width = |width: f32| theme.scaled(px(width)).min(max_width - px(64.));
        let editor = cx.entity();
        Some(
            div()
                .absolute()
                .top_2()
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_wrap()
                        .gap_x_3()
                        .px_3()
                        .py_1()
                        .max_w(max_width)
                        .rounded(theme.radius_medium)
                        .bg(theme.surface.raised)
                        .border_1()
                        .border_color(theme.border.control)
                        .text_color(theme.text.primary)
                        .child(div().text_color(theme.text.muted).child("Find"))
                        .child(div().w(field_width(200.)).child(field(
                            theme,
                            &find.query,
                            !find.editing_replacement,
                        )))
                        // Reserves the width of "no matches" so the bar (centred) does not shift
                        // when the status appears on the first query character.
                        .child(
                            div()
                                .min_w(theme.scaled(px(96.)))
                                .text_color(theme.text.muted)
                                .child(find.status()),
                        )
                        .children(find.replacement.as_ref().map(|replacement| {
                            div()
                                .flex()
                                .gap_3()
                                .child(div().text_color(theme.text.muted).child("Replace"))
                                .child(div().w(field_width(160.)).child(field(
                                    theme,
                                    replacement,
                                    find.editing_replacement,
                                )))
                        }))
                        .child(
                            // Measures the bar's rendered height, in window coordinates, without
                            // an extra layout pass: `reveal_caret_at` keeps revealed content below
                            // it while it is open. Prepaints before any block paints (see
                            // `find_bar_bottom`'s doc comment), so the value is current this frame.
                            canvas(
                                move |bounds, window, cx| {
                                    let bottom = bounds.bottom();
                                    let changed = editor.update(cx, |editor, _| {
                                        let changed = editor.find_bar_bottom != bottom;
                                        editor.find_bar_bottom = bottom;
                                        changed
                                    });
                                    if changed {
                                        // Block 0's reserved top space (see `render_block`) and
                                        // `reveal_caret_at`'s inset were sized from the previous
                                        // measurement; redraw once more so both reflect this
                                        // frame's, the way `render_raw` does when it scrolls.
                                        window.request_animation_frame();
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The open picker: its title and filter with a caret, then the matching rows, the chosen one
    /// highlighted. Clicking a row picks it.
    fn picker_bar(&self, viewport: Size<Pixels>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.picker.as_ref()?;
        let theme = &self.theme;
        // The top edge sits a fifth of the way down (the optical centre Spotlight-style launchers
        // use), not at the window's top: the eye finds it without travelling to the edge, and
        // anchoring the top rather than centring the whole box keeps it from jumping as typing
        // shrinks the list. Rows that fit under it: line height plus the 4 px row gap, with the
        // bottom margin, padding and header taken off. A short window falls back to the margin.
        let row_height = theme.text_size * 1.6 + theme.scaled(px(4.));
        let full_height =
            theme.scaled(px(16.)) + row_height * (crate::picker::VISIBLE_ROWS as f32 + 1.);
        let top = (viewport.height * 0.2)
            .min(viewport.height - OVERLAY_MARGIN - full_height)
            .max(OVERLAY_MARGIN);
        let room = viewport.height - top - OVERLAY_MARGIN - theme.scaled(px(16.)) - row_height;
        let fit = (room / row_height).floor().max(1.) as usize;
        let rows = picker.window(fit.min(crate::picker::VISIBLE_ROWS)).map(|row| {
            let item = &picker.items[picker.matches[row]];
            let selected = row == picker.selected;
            let detail_color = if selected { theme.text.on_accent } else { theme.text.muted };
            div()
                .id(row)
                .flex()
                .gap_2()
                .px_2()
                .rounded(theme.radius_small)
                .pl(self.theme.scaled(px(8. + 14. * f32::from(item.indent))))
                .overflow_hidden()
                .when(selected, |d| d.bg(theme.accent).text_color(theme.text.on_accent))
                .child(div().flex_none().child(if item.marked {
                    format!("\u{2713} {}", item.label)
                } else {
                    item.label.clone()
                }))
                .children(item.detail.clone().map(|detail| {
                    div()
                        .text_color(detail_color)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(detail)
                }))
                .child(div().flex_1())
                .children(
                    item.shortcut
                        .clone()
                        .map(|shortcut| div().flex_none().text_color(detail_color).child(shortcut)),
                )
                .on_click(cx.listener(move |editor, _, window, cx| {
                    editor.picker_pick(Some(row), window, cx)
                }))
        });
        let note = if picker.items.is_empty() {
            Some(picker.empty)
        } else if picker.matches.is_empty() {
            Some("no matches")
        } else {
            None
        };
        Some(
            div()
                .absolute()
                .top(top)
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .w(self.theme.scaled(px(520.)).min(viewport.width - OVERLAY_MARGIN * 2.))
                        .rounded(theme.radius_medium)
                        .bg(theme.surface.raised)
                        .border_1()
                        .border_color(theme.border.control)
                        .text_color(theme.text.primary)
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(div().text_color(theme.text.muted).child(picker.title))
                                .child(div().flex_1().min_w_0().child(field(
                                    theme,
                                    &picker.query,
                                    true,
                                ))),
                        )
                        .children(rows)
                        .children(note.map(|note| div().text_color(theme.text.muted).child(note))),
                )
                .into_any_element(),
        )
    }

    /// Highlighted code tokens of the block containing `range`, as absolute source ranges, while
    /// its parse is current (the source of fenced code is its visible text, so the IR's token runs
    /// map back exactly).
    fn code_tokens(&self, range: &Range<usize>) -> Vec<(Range<usize>, tachyon_md::Style)> {
        let Some(index) = self.doc.block_at(range.start) else { return Vec::new() };
        let block = self.doc.block_range(index);
        if self.doc.dirty_ranges().iter().any(|d| d.start < block.end && block.start < d.end) {
            return Vec::new();
        }
        let ir = &self.doc.blocks()[index].parsed().ir;
        ir.runs
            .iter()
            .filter(|run| run.style.is_code_token())
            .map(|run| {
                let start = block.start + ir.visible_to_source(run.range.start);
                let end = block.start + ir.visible_to_source(run.range.end);
                (start..end, run.style)
            })
            .collect()
    }

    /// Marks for `block` (absolute source range): find matches, then the selection. Matches are
    /// underlined, 1 px, and the current one 2 px with a stronger fill, so they differ by shape
    /// as well as color. The current match is also the selection; it keeps its find fill.
    pub(crate) fn marks(&self, block: &Range<usize>) -> Vec<Mark> {
        let editing = &self.theme.editing;
        let mut marks = Vec::new();
        let mut selection_is_match = false;
        if let Some(find) = &self.find {
            selection_is_match =
                find.current.and_then(|i| find.matches.get(i)) == Some(&self.selection);
            for (m, current) in find.matches_in(block) {
                let (fill, thickness) =
                    if current { (editing.find_current, 2.) } else { (editing.find_match, 1.) };
                let style = HighlightStyle {
                    background_color: Some(fill),
                    underline: Some(UnderlineStyle {
                        thickness: px(thickness),
                        color: Some(editing.find_underline),
                        wavy: false,
                    }),
                    ..Default::default()
                };
                marks.push((m, style));
            }
        }
        if !self.selection.is_empty() && !selection_is_match {
            let style =
                HighlightStyle { background_color: Some(editing.selection), ..Default::default() };
            marks.push((self.selection.clone(), style));
        }
        marks
    }

    fn frame_stats_overlay(&self) -> Option<AnyElement> {
        let stats = self.frame_stats.as_ref()?;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.;
        let text = match stats.summary() {
            Some((p50, max, over, n)) => format!(
                "frame p50 {:.1} ms · max {:.1} ms · {over}/{n} over {:.1} ms",
                ms(p50),
                ms(max),
                ms(crate::frame_stats::FRAME_BUDGET)
            ),
            None => "frame stats: waiting for frames".to_owned(),
        };
        let over = stats.summary().is_some_and(|(_, _, over, _)| over > 0);
        Some(
            div()
                .absolute()
                .top_2()
                .right_2()
                .px_2()
                .rounded_md()
                .bg(self.theme.surface.raised)
                .border_1()
                .border_color(if over { gpui::red() } else { self.theme.border.subtle })
                .font_family(self.theme.code_font.clone())
                .text_size(px(12.))
                .text_color(self.theme.text.muted)
                .child(SharedString::from(text))
                .into_any_element(),
        )
    }

    /// A one-line notice at the top of the view (currently only the oversized-Markdown
    /// fallback: see `disk::oversized_markdown_notice`), a normal (not floating) row so it pushes
    /// the document down rather than covering its first line.
    fn mode_notice(&self) -> Option<AnyElement> {
        let notice = self.notice.clone()?;
        let theme = &self.theme;
        Some(
            div()
                .w_full()
                .flex_none()
                .px_4()
                .py_1()
                .text_size(px(12.))
                .bg(theme.surface.raised)
                .text_color(theme.text.muted)
                .border_b_1()
                .border_color(theme.border.subtle)
                .child(notice)
                .into_any_element(),
        )
    }

    /// The rotating tip (`Settings::tips`; picked once per window, see `Editor::refresh_tip`),
    /// in `theme.text.tip`, centred in the content column near the bottom of the window. Added
    /// to `render`'s tree before the list below, so the list's own blocks paint over it wherever
    /// the document has content there, and it only shows through the empty space around and
    /// below them - it stays visible whether the document is empty or not, since it is never the
    /// only thing in that space. Decorative: no id and no mouse handler, so it is neither
    /// hit-tested nor selectable.
    fn tip_overlay(&self) -> Option<AnyElement> {
        // No Pro Tip in a sticky note's compact layout.
        if self.is_note() {
            return None;
        }
        let tip = self.tip.clone()?;
        let theme = &self.theme;
        Some(
            div()
                .absolute()
                .bottom_6()
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .max_w(theme.content_width)
                        .px_4()
                        .text_center()
                        .text_size(theme.text_size)
                        .text_color(theme.text.tip)
                        .child(tip),
                )
                .into_any_element(),
        )
    }

    /// The sticky note's compact header, replacing the native title bar
    /// (`crates/tachyon/src/app.rs`'s `note_window_options` sets `TitlebarOptions::
    /// appears_transparent`): a drag area showing the note's name (`Editor::title`), a pin
    /// toggle - only when `tachyon_platform::supports_always_on_top()` says the platform does
    /// anything with it - and a close button. `None` for an ordinary editor window. The drag
    /// area sets `WindowControlArea::Drag` (Windows/macOS's own hit-test convention for a
    /// custom title bar) and calls `Window::start_window_move` on a plain mouse-down, covering
    /// the platforms where the hit-test area alone does not initiate the move.
    fn note_header(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.is_note() {
            return None;
        }
        let theme = &self.theme;
        let editor = cx.entity();
        let pinned = self.note_pinned();
        let close_editor = editor.clone();
        let mut header = div()
            .id("note-header")
            .flex()
            .flex_none()
            .items_center()
            .h(theme.scaled(px(28.)))
            .px_2()
            .gap_2()
            .bg(theme.surface.raised)
            .border_b_1()
            .border_color(theme.border.subtle)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, move |_event: &MouseDownEvent, window, _cx| {
                window.start_window_move();
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(theme.scaled(px(12.)))
                    .text_color(theme.text.muted)
                    .child(SharedString::from(self.title())),
            );
        if tachyon_platform::supports_always_on_top() {
            let pin_editor = editor.clone();
            header = header.child(
                div()
                    .id("note-pin")
                    .flex_none()
                    .size(theme.scaled(px(20.)))
                    .rounded(theme.radius_small)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(pinned, |d| d.bg(theme.accent).text_color(theme.text.on_accent))
                    .when(!pinned, |d| d.text_color(theme.text.muted))
                    .child(if pinned { "\u{25cf}" } else { "\u{25cb}" })
                    .on_mouse_down(MouseButton::Left, move |_event: &MouseDownEvent, window, cx| {
                        pin_editor.update(cx, |editor, cx| {
                            editor.toggle_note_pin(&crate::editor::ToggleNotePin, window, cx);
                        });
                    }),
            );
        }
        header = header.child(
            div()
                .id("note-close")
                .flex_none()
                .size(theme.scaled(px(20.)))
                .rounded(theme.radius_small)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(theme.text.muted)
                .window_control_area(WindowControlArea::Close)
                .child("\u{2715}")
                .on_mouse_down(MouseButton::Left, move |_event: &MouseDownEvent, window, cx| {
                    close_editor.update(cx, |editor, cx| {
                        editor.close_window(&crate::editor::CloseWindow, window, cx);
                    });
                }),
        );
        Some(header.into_any_element())
    }
}

impl Editor {
    fn render_block(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Restore `ir` first if `Document::evict` had dropped it (report fix 6): every block
        // this function goes on to read `parsed()`/`parsed_shared()` for must have real ir.
        self.doc.ensure_ir(index);
        let Some(block) = self.doc.blocks().get(index) else {
            return div().into_any_element();
        };
        self.rendering = Some(match self.rendering.take() {
            Some(r) => r.start.min(index)..r.end.max(index + 1),
            None => index..index + 1,
        });
        let range = self.doc.block_range(index);
        let content = if self.doc.mode() == tachyon_doc::DocMode::Plain {
            // No block-swap distinction in plain text: every chunk is always shown this way, the
            // same "raw" path Markdown uses for the block under the caret, just without its
            // editing-card styling (see `render_raw`'s own `DocMode::Plain` checks).
            self.render_raw(index, range, true, window, cx)
        } else {
            let parsed = block.parsed_shared();
            let active = self.active_block() == Some(index);
            let leaf = if active { self.active_leaf() } else { None };
            if block.is_stale() || (active && leaf.is_none()) {
                let code = matches!(parsed.kind, BlockKind::CodeBlock { .. } | BlockKind::Html);
                self.render_raw(index, range, code, window, cx)
            } else {
                self.render_rendered(index, range.start, parsed, leaf, window, cx)
            }
        };
        // The list cannot scroll above its first item, so `reveal_caret_at`'s inset (which
        // relies on scrolling) cannot keep a match in this block below the find bar on its own.
        // Growing this block's own top padding while the bar is open reserves that room instead:
        // it is part of the block's measured height, so scrolling past it (bar open or not)
        // behaves like any other content and leaves nothing to jump when viewed elsewhere.
        let mut wrapper = div().w_full().flex().justify_center();
        if index == 0 && self.find.is_some() {
            wrapper = wrapper.pt(self.find_bar_bottom + OVERLAY_MARGIN);
        }
        wrapper
            .child(div().w_full().max_w(self.theme.content_width).px_4().child(content))
            .into_any_element()
    }

    /// The active block's (or, in `DocMode::Plain`, any block's) source, caret and selection.
    /// Above [`RAW_SPLIT_THRESHOLD`], splits it into several stacked [`render_raw_segment`]
    /// pieces with [`raw_segment_lens`] instead of one - small enough that re-shaping the one
    /// segment a keystroke touches stays well under a frame regardless of the block's own size -
    /// the same reason `DocMode::Plain` itself never shows more than that much of one
    /// pathological line at once, just reached through the active-block view of a huge Markdown
    /// block. Hit testing, the caret and highlights all
    /// stay correct across the split: `TextTarget::Raw { base }` already carries whichever
    /// range's own start, unchanged by how many pieces one block is drawn in, and `marks`/
    /// `code_tokens` already clip to whatever range they are asked about.
    fn render_raw(
        &mut self,
        index: usize,
        range: Range<usize>,
        code: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if range.len() <= RAW_SPLIT_THRESHOLD {
            return self.render_raw_segment(range, code, window, cx);
        }
        let rope = self.doc.buffer().rope().clone();
        let version = self.doc.buffer().version();
        let lens = match &self.raw_chunk_cache {
            Some((cached_range, cached_version, cached_lens))
                if *cached_range == range && *cached_version == version =>
            {
                cached_lens.clone()
            }
            _ => {
                let lens = raw_segment_lens(&rope, range.clone());
                self.raw_chunk_cache = Some((range.clone(), version, lens.clone()));
                lens
            }
        };
        if lens.len() <= 1 {
            return self.render_raw_segment(range, code, window, cx);
        }
        let mut starts = Vec::with_capacity(lens.len() + 1);
        let mut at = range.start;
        starts.push(at);
        for &len in &lens {
            at += len;
            starts.push(at);
        }
        let row_height =
            if code { self.theme.code_size * 1.45 } else { self.theme.text_size * 1.6 };
        // A coarse estimate for windowing only, not real layout: bytes/60, a rough characters-
        // per-wrapped-row guess. Deliberately not `rope.byte_to_line` (a real per-segment line
        // count would be more accurate for a segment cut at real newlines): querying it for
        // every segment, every frame, costs more on a rope with many real lines - like this
        // file's own content - than the whole rest of building the window.
        let heights: Vec<Pixels> = starts
            .windows(2)
            .map(|w| row_height * ((w[1] - w[0]) as f32 / 60.).ceil().max(1.))
            .collect();
        let head = self.head();
        let keep = (head >= range.start && head <= range.end)
            .then(|| starts.partition_point(|&s| s <= head).saturating_sub(1).min(lens.len() - 1));
        let (win, keep_outside, before, after) = self.render_window(index, &heights, keep);
        let mut column = div().flex().flex_col().w_full();
        if before > px(0.) {
            column = column.child(div().h(before).w_full());
        }
        for i in win {
            column =
                column.child(self.render_raw_segment(starts[i]..starts[i + 1], code, window, cx));
        }
        if let Some(k) = keep_outside {
            column =
                column.child(self.render_raw_segment(starts[k]..starts[k + 1], code, window, cx));
        }
        if after > px(0.) {
            column = column.child(div().h(after).w_full());
        }
        column.into_any_element()
    }

    /// Which of `heights` (by index) are worth a real element this frame, plus the pixel height
    /// to reserve before/after that window: `list` virtualizes whole top-level blocks, so
    /// without this a single enormous block - one huge fenced code block's lines, or one
    /// pathologically long line's forced-cut segments - would otherwise be shaped and laid out
    /// in full every frame regardless of scroll position, the dominant cost the diagnosis
    /// measured. Estimates which one is nearest the viewport from this block's own bounds *last
    /// frame* (`ListState::bounds_for_item`/`viewport_bounds`) - the same one-frame-stale
    /// reliance `active_layout` already has, for the same reason: nothing else exposes a scroll
    /// position within one list item. `keep` (typically the caret's own segment/line), when
    /// given, always gets a real element too even if the estimate places it outside the window,
    /// so typing, clicking or a far jump always has something live to paint a caret into or
    /// scroll from; it may paint one frame out of place until the next frame's estimate catches
    /// up, exactly like `reveal_caret_at`'s own "scrolled: draw again" correction.
    fn render_window(
        &self,
        index: usize,
        heights: &[Pixels],
        keep: Option<usize>,
    ) -> (Range<usize>, Option<usize>, Pixels, Pixels) {
        let n = heights.len();
        if n == 0 {
            return (0..0, None, px(0.), px(0.));
        }
        let mut first_visible = 0;
        if let Some(bounds) = self.window_item_bounds.get(&index) {
            let visible_top = (self.window_viewport.top() - bounds.top()).max(px(0.));
            let mut consumed = px(0.);
            first_visible = n - 1;
            for (i, &h) in heights.iter().enumerate() {
                if consumed + h > visible_top {
                    first_visible = i;
                    break;
                }
                consumed += h;
            }
        }
        // Overdraw is a pixel budget, not a segment/line count: one segment/line can be a
        // fraction of a row (an ordinary heading) or, for a forced-cut segment of one huge
        // unwrapped line, tens of rows on its own, so a flat count either wastes a frame's worth
        // of shaping on tiny lines or (worse) stops short of a real viewport's worth of tall
        // ones. `RENDER_WINDOW_OVERDRAW_VIEWPORTS` viewport-heights each side, falling back to a
        // fixed minimum before the first real layout (`window_viewport` still zeroed).
        let overdraw =
            (self.window_viewport.size.height * RENDER_WINDOW_OVERDRAW_VIEWPORTS).max(px(600.));
        let mut start = first_visible;
        let mut back = px(0.);
        while start > 0 && back < overdraw {
            start -= 1;
            back += heights[start];
        }
        let mut end = (first_visible + 1).min(n);
        let mut forward = px(0.);
        while end < n && forward < overdraw {
            forward += heights[end];
            end += 1;
        }
        let keep_outside = keep.filter(|k| *k >= n || !(start..end).contains(k));
        let mut before = heights[..start].iter().copied().fold(px(0.), |a, b| a + b);
        let mut after = heights[end..].iter().copied().fold(px(0.), |a, b| a + b);
        if let Some(k) = keep_outside.filter(|&k| k < n) {
            if k < start {
                before -= heights[k];
            } else if k >= end {
                after -= heights[k];
            }
        }
        (start..end, keep_outside, before, after)
    }

    /// One piece of the active block's (or, in `DocMode::Plain`, any block's) source: its own
    /// caret, highlights and mouse handling, exactly as [`Editor::render_raw`] drew the whole
    /// block before it could be split into several of these.
    fn render_raw_segment(
        &mut self,
        range: Range<usize>,
        code: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &self.theme;
        let source = self.doc.buffer().rope().byte_slice(range.clone()).to_string();
        // A block's final newline ends its last line; drawing it would add an empty line. The
        // document's last block keeps it: the empty line after it is where the caret sits at
        // the end of a file, and without it the caret has no position and is not drawn.
        let last_block = range.end == self.doc.len();
        let text = match source.strip_suffix('\n') {
            Some(stripped) if !last_block => stripped.to_owned(),
            _ => source,
        };
        let base = range.start;
        let len = text.len();

        let plain = self.doc.mode() == tachyon_doc::DocMode::Plain;
        let mut highlights = Vec::new();
        if code {
            for (token, style) in self.code_tokens(&range) {
                if let Some(local) = intersect(&token, &range, base, len) {
                    highlights = overlay(highlights, local, theme.highlight(style));
                }
            }
        }
        if plain {
            // Cheap: a single scan, skipped entirely unless the text contains "http" (see
            // `bare_urls`), and only ever run over one visible chunk's text at paint time.
            for url in tachyon_md::bare_urls(&text) {
                highlights = overlay(highlights, url, theme.highlight(tachyon_md::Style::LINK));
            }
        }
        for (mark, style) in self.marks(&range) {
            if let Some(local) = intersect(&mark, &range, base, len) {
                highlights = overlay(highlights, local, style);
            }
        }
        if let Some(marked) = &self.marked
            && let Some(local) = intersect(marked, &range, base, len)
        {
            highlights = overlay(
                highlights,
                local,
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(theme.text.primary),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            );
        }
        let styled = StyledText::new(text).with_highlights(highlights);
        let layout = styled.layout().clone();
        let head = self.head();
        let caret = (head >= base && head <= base + len).then(|| head - base);
        let editor = cx.entity();
        // Only one caret is ever visible: while the bar holds typing (see `bar_open`) the field
        // it belongs to draws its own, and the document's stays hidden even if it is focused.
        let show_caret = self.focus.is_focused(window) && !self.bar_open() && self.editing;
        let cursor_color = theme.editing.caret;
        let paint_layout = layout.clone();

        let mut element = div().relative().cursor_text().child(styled).child(
            canvas(
                |_, _, _| {},
                move |_, _, window, cx| {
                    let caret_position =
                        caret.and_then(|caret| paint_layout.position_for_index(caret));
                    let line_height = paint_layout.line_height();
                    let scrolled = editor.update(cx, |editor, _| {
                        // In `DocMode::Plain` every block renders raw (see `render_block`),
                        // so several of these canvases paint each frame; only the one that
                        // actually holds the caret may claim `active_layout` (`vertical`'s
                        // notion of "the laid-out block"), or a later-painted neighbor with
                        // no caret in it would silently steal it and misdirect Up/Down.
                        if caret_position.is_some() {
                            editor.active_layout = Some((paint_layout.clone(), base));
                        }
                        if show_caret && caret_position.is_some() {
                            editor.caret_painted = true;
                        }
                        caret_position.is_some_and(|p| editor.reveal_caret_at(p.y, line_height))
                    });
                    if scrolled {
                        // The caret was out of view: draw again.
                        window.request_animation_frame();
                    }
                    if show_caret && let Some(position) = caret_position {
                        let caret = fill(
                            gpui::Bounds::new(position, size(px(2.), line_height)),
                            cursor_color,
                        );
                        window.paint_quad(caret);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        if !plain {
            // The raw editing card: only Markdown's block-swap shows this (the active block
            // looks editable, distinct from rendered ones). A plain chunk is always "raw", so it
            // never gets this treatment - it would otherwise draw a border around every ~16 KiB
            // chunk boundary of an ordinary text file.
            element = element
                .my_1()
                .px(theme.scaled(RAW_INSET) - px(1.))
                .rounded(theme.radius_small)
                .border_1()
                .border_color(theme.border.control)
                .bg(theme.surface.raised);
        }
        if code || plain {
            element = element.font_family(theme.code_font.clone()).text_size(theme.code_size);
        }
        with_mouse(element, layout, TextTarget::Raw { base }, cx).into_any_element()
    }

    /// Rendered lines with syntax hidden; the lines of `raw_leaf` (the leaf
    /// holding the caret, with its absolute source range) are shown raw.
    fn render_rendered(
        &mut self,
        index: usize,
        block_start: usize,
        parsed: Arc<ParsedBlock>,
        raw_leaf: Option<(usize, Range<usize>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ir = &parsed.ir;
        let block_len = parsed.len;
        // Marks (find matches, selection) mapped to visible offsets of this block.
        let block = block_start..block_start + block_len;
        let marks: Vec<Mark> = self
            .marks(&block)
            .into_iter()
            .filter_map(|(mark, style)| {
                let local = intersect(&mark, &block, block_start, block_len)?;
                let visible = ir.source_to_visible(local.start)..ir.source_to_visible(local.end);
                (!visible.is_empty()).then_some((visible, style))
            })
            .collect();

        if ir.lines.is_empty() {
            // Blank or empty block: still clickable, to put the caret in it.
            let target = TextTarget::Raw { base: block_start };
            let styled = StyledText::new(" ");
            let layout = styled.layout().clone();
            return with_mouse(
                div().h(self.theme.scaled(px(8.))).child(styled),
                layout,
                target,
                cx,
            )
            .into_any_element();
        }

        // Above `LINE_SPLIT_THRESHOLD`, only a window of lines around the one estimated nearest
        // the viewport gets a real element (see `render_window`) - a non-active block (this
        // function only ever draws one) never holds the caret, so nothing here needs a `keep`
        // line the way `render_raw`'s active-block segments do. Skipped when a leaf is being
        // shown raw (`raw_leaf`, inside a list/quote/footnote): that combination - editing one
        // leaf of a container with many thousands of lines - is rare enough not to be worth the
        // extra bookkeeping of keeping the swapped leaf's lines live across a window.
        let windowed = raw_leaf.is_none() && ir.lines.len() > LINE_SPLIT_THRESHOLD;
        let (win, before, after) = if windowed {
            let heights: Vec<Pixels> = match &self.rendered_heights_cache {
                Some((cached_parsed, cached_heights)) if Arc::ptr_eq(cached_parsed, &parsed) => {
                    cached_heights.clone()
                }
                _ => {
                    let heights: Vec<Pixels> =
                        ir.lines.iter().map(|line| self.line_height_estimate(line.kind)).collect();
                    self.rendered_heights_cache = Some((Arc::clone(&parsed), heights.clone()));
                    heights
                }
            };
            let (win, _, before, after) = self.render_window(index, &heights, None);
            (win, before, after)
        } else {
            (0..ir.lines.len(), px(0.), px(0.))
        };

        let mut column = div().flex().flex_col().my_2();
        if before > px(0.) {
            column = column.child(div().h(before).w_full());
        }
        let mut raw_shown = false;
        for i in win {
            let line = &ir.lines[i];
            if let Some((leaf, range)) = &raw_leaf
                && line.leaf == *leaf
            {
                if !std::mem::replace(&mut raw_shown, true) {
                    let code = ir.lines.iter().any(|l| {
                        l.leaf == *leaf && matches!(l.kind, LineKind::Code | LineKind::Html)
                    });
                    column = column.child(self.render_raw(index, range.clone(), code, window, cx));
                }
                continue;
            }
            let end = ir.lines.get(i + 1).map_or(ir.text.len(), |next| next.start - 1);
            let images = self.line_images(ir, &(line.start..end), block_start);
            // A line that is just an image shows the image instead of its alt text.
            let image_only = images.len() == 1 && images[0].2 == (line.start..end);
            if !image_only {
                column = column.child(self.render_line(
                    line,
                    line.start..end,
                    &parsed,
                    block_start,
                    &marks,
                    cx,
                ));
            }
            for (path, offset, _) in images {
                column = column.child(self.render_image(path, offset, cx));
            }
        }
        if after > px(0.) {
            column = column.child(div().h(after).w_full());
        }
        if let BlockKind::Heading(1 | 2) = parsed.kind {
            column = column.pb_1().border_b_1().border_color(self.theme.border.subtle);
        }
        column.into_any_element()
    }

    /// A coarse per-`LineKind` row-height estimate, for [`Editor::render_window`]'s windowing
    /// only (never real layout): matches the `.line_height(relative(N))` each `render_line` arm
    /// actually uses closely enough that the window rarely needs its overdraw margin to cover
    /// the difference.
    fn line_height_estimate(&self, kind: LineKind) -> Pixels {
        let theme = &self.theme;
        match kind {
            LineKind::Heading(level) => theme.heading_sizes[(level.clamp(1, 6) - 1) as usize] * 1.3,
            LineKind::Code | LineKind::Html => theme.code_size * 1.45,
            _ => theme.text_size * 1.6,
        }
    }

    /// Local images in `line` (visible range) of a rendered block: their file, the source offset
    /// a click puts the caret at, and their visible range.
    pub(crate) fn line_images(
        &self,
        ir: &tachyon_md::BlockIr,
        line: &Range<usize>,
        block_start: usize,
    ) -> Vec<(PathBuf, usize, Range<usize>)> {
        ir.links
            .iter()
            .filter(|link| line.start <= link.visible.start && link.visible.end <= line.end)
            .filter(|link| {
                ir.runs.iter().any(|run| {
                    run.style.contains(tachyon_md::Style::IMAGE)
                        && run.range.start <= link.visible.start
                        && link.visible.start < run.range.end
                })
            })
            .filter_map(|link| {
                let path = crate::links::image_path(&link.dest, self.file.as_deref())?;
                let offset = block_start + ir.visible_to_source(link.visible.start);
                Some((path, offset, link.visible.clone()))
            })
            .collect()
    }

    /// An image, scaled down to fit the column; loaded and decoded off the UI thread by GPUI. A
    /// click edits its Markdown.
    fn render_image(&self, path: PathBuf, offset: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = &self.theme;
        let editor = cx.entity();
        let missing = path.display().to_string();
        let muted = theme.text.muted;
        div()
            .py_1()
            .child(
                img(path)
                    .max_w_full()
                    .max_h(theme.scaled(px(480.)))
                    .object_fit(ObjectFit::ScaleDown)
                    .with_fallback(move || {
                        div()
                            .text_color(muted)
                            .child(format!("image not found: {missing}"))
                            .into_any_element()
                    }),
            )
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.mouse_down(offset, event.modifiers, event.click_count, window, cx)
                });
                cx.stop_propagation();
            })
            .into_any_element()
    }

    fn render_line(
        &self,
        line: &LineInfo,
        visible: Range<usize>,
        parsed: &Arc<ParsedBlock>,
        block_start: usize,
        marks: &[Mark],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &self.theme;
        let content: AnyElement = match line.kind {
            LineKind::Rule => {
                div().w_full().h(px(1.)).my_3().bg(theme.border.subtle).into_any_element()
            }
            LineKind::TableRow { header } => {
                // Each cell draws only its right and bottom edges, and the row its left edge (the
                // header row, always first, also its top): neighbouring cells used to draw two
                // lines side by side, a double-width grid next to 1 px card and field borders.
                let mut row = div().flex().w_full().border_l_1().border_color(theme.border.subtle);
                if header {
                    row = row.border_t_1();
                }
                let mut cell_start = visible.start;
                let text = &parsed.ir.text[visible.clone()];
                for cell in text.split('\t') {
                    let range = cell_start..cell_start + cell.len();
                    let mut cell_el = div()
                        .flex_1()
                        .min_w_0()
                        .px_2()
                        .border_r_1()
                        .border_b_1()
                        .border_color(theme.border.subtle)
                        .child(self.rendered_text(range.clone(), parsed, block_start, marks, cx));
                    if header {
                        cell_el = cell_el.font_weight(FontWeight::BOLD).bg(theme.surface.code);
                    }
                    row = row.child(cell_el);
                    cell_start = range.end + 1;
                }
                row.into_any_element()
            }
            kind => {
                let mut el = div().flex_1().min_w_0().child(self.rendered_text(
                    visible,
                    parsed,
                    block_start,
                    marks,
                    cx,
                ));
                match kind {
                    LineKind::Heading(level) => {
                        let size = theme.heading_sizes[(level.clamp(1, 6) - 1) as usize];
                        el = el
                            .text_size(size)
                            .font_weight(FontWeight::BOLD)
                            .line_height(relative(1.3));
                    }
                    LineKind::Code => {
                        el = el
                            .font_family(theme.code_font.clone())
                            .text_size(theme.code_size)
                            .line_height(relative(1.45))
                            .bg(theme.surface.code)
                            .px_3();
                    }
                    LineKind::Html => {
                        el = el
                            .font_family(theme.code_font.clone())
                            .text_size(theme.code_size)
                            .text_color(theme.text.muted);
                    }
                    _ => {}
                }
                el.into_any_element()
            }
        };

        let mut row = div().flex().w_full();
        if line.quote > 0 {
            row = row
                .border_l_2()
                .border_color(theme.border.subtle)
                .pl(theme.scaled(px(12. * f32::from(line.quote))))
                .text_color(theme.text.muted);
        }
        let depth = line.indent.saturating_sub(u8::from(line.marker.is_some()));
        if depth > 0 {
            row = row.pl(theme.scaled(px(INDENT * f32::from(depth))));
        }
        if let Some(marker) = line.marker {
            row = row.child(self.marker(marker));
        } else if line.indent > 0 {
            row = row.pl(theme.scaled(px(INDENT * f32::from(line.indent))));
        }
        row.child(content).into_any_element()
    }

    fn marker(&self, marker: Marker) -> AnyElement {
        let theme = &self.theme;
        let el = div().w(theme.scaled(px(INDENT))).flex_none().text_color(theme.text.muted);
        match marker {
            Marker::Bullet => el.child("•").into_any_element(),
            Marker::Ordered(n) => el.child(SharedString::from(format!("{n}."))).into_any_element(),
            Marker::Task { checked } => el
                .flex()
                .items_center()
                .child(
                    div()
                        .size(theme.scaled(px(13.)))
                        .rounded(theme.radius_small)
                        .border_1()
                        .border_color(theme.border.control)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(theme.scaled(px(11.)))
                        .line_height(theme.scaled(px(11.)))
                        .when(checked, |d| {
                            d.bg(theme.accent).text_color(theme.text.on_accent).child("✓")
                        }),
                )
                .into_any_element(),
        }
    }

    /// Rendered text for `visible` (a range of the block's IR text) with
    /// inline styles, selection and click handling.
    fn rendered_text(
        &self,
        visible: Range<usize>,
        parsed: &Arc<ParsedBlock>,
        block_start: usize,
        marks: &[Mark],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ir = &parsed.ir;
        let text = &ir.text[visible.clone()];
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = ir
            .runs
            .iter()
            .filter_map(|run| {
                let start = run.range.start.max(visible.start);
                let end = run.range.end.min(visible.end);
                (start < end).then(|| {
                    (start - visible.start..end - visible.start, self.theme.highlight(run.style))
                })
            })
            .collect();
        for (mark, style) in marks {
            let start = mark.start.max(visible.start);
            let end = mark.end.min(visible.end);
            if start < end {
                highlights =
                    overlay(highlights, start - visible.start..end - visible.start, *style);
            }
        }
        // An empty line still needs a line box to be visible and clickable.
        let shown = if text.is_empty() { " ".to_owned() } else { text.to_owned() };
        let styled = StyledText::new(shown).with_highlights(highlights);
        let layout = styled.layout().clone();
        let target = TextTarget::Rendered {
            block_start,
            visible_base: visible.start,
            parsed: Arc::clone(parsed),
        };
        with_mouse(div().cursor_text().child(styled), layout, target, cx).into_any_element()
    }
}

/// A find-bar or picker field: a bordered box (`border.focus` while it receives typing,
/// `border.control` otherwise) that fills its reserved width (the caller sizes that container;
/// `find_bar`'s wrappers use a capped width, the picker's uses `flex_1`), so it neither grows
/// with the query nor changes width when Tab moves focus between differently-sized fields. It
/// has a steady caret painted as a 2 px quad, full line height, after the text, never a glyph, so
/// ClearType's subpixel anti-aliasing on a caret character cannot tint it (a fringe seen on
/// Windows). The height is fixed to one text line so the border never changes it, and an empty
/// active field still reserves a line box for the caret.
fn field(theme: &Theme, text: &str, active: bool) -> AnyElement {
    let border = if active { theme.border.focus } else { theme.border.control };
    let el = div()
        .w_full()
        .h(theme.text_size * 1.6)
        .flex()
        .items_center()
        .px_1()
        .overflow_hidden()
        .rounded(theme.radius_small)
        .border_1()
        .border_color(border);
    if !active {
        return el.child(text.to_owned()).into_any_element();
    }
    let shown = if text.is_empty() { " ".to_owned() } else { text.to_owned() };
    let styled = StyledText::new(shown);
    let layout = styled.layout().clone();
    let caret_index = text.len();
    let cursor_color = theme.editing.caret;
    el.relative()
        .child(styled)
        .child(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    if let Some(position) = layout.position_for_index(caret_index) {
                        let caret = fill(
                            gpui::Bounds::new(position, size(px(2.), layout.line_height())),
                            cursor_color,
                        );
                        window.paint_quad(caret);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

/// Horizontal padding of the active block's card: raw text sits this much to
/// the right of the same text rendered.
pub(crate) const RAW_INSET: Pixels = px(8.);

/// A highlight over an absolute source range (find match, selection).
pub(crate) type Mark = (Range<usize>, HighlightStyle);

/// Space kept between an overlay (find bar, picker, prompt) and each window edge.
pub(crate) const OVERLAY_MARGIN: Pixels = px(8.);

/// GPUI's default rem size, the 100 % zoom reference.
const BASE_REM_SIZE: Pixels = px(16.);

/// Attaches click and drag-select handlers mapping pointer positions in
/// `layout` to document offsets through `target`.
fn with_mouse(
    el: gpui::Div,
    layout: TextLayout,
    target: TextTarget,
    cx: &mut Context<Editor>,
) -> gpui::Div {
    let editor: Entity<Editor> = cx.entity();
    let (down_layout, down_target, down_editor) = (layout.clone(), target.clone(), editor.clone());
    el.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
        let (Ok(index) | Err(index)) = down_layout.index_for_position(event.position);
        let index = index.min(down_layout.len());
        let offset = down_target.offset(index);
        down_editor.update(cx, |editor, cx| {
            editor.mouse_down(offset, event.modifiers, event.click_count, window, cx)
        });
        cx.stop_propagation();
    })
    .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (Ok(index) | Err(index)) = layout.index_for_position(event.position);
        let offset = target.offset(index.min(layout.len()));
        editor.update(cx, |editor, cx| editor.mouse_drag(offset, cx));
    })
}

/// `range ∩ block`, relative to `base`, clamped to `len`.
fn intersect(
    range: &Range<usize>,
    block: &Range<usize>,
    base: usize,
    len: usize,
) -> Option<Range<usize>> {
    let start = range.start.max(block.start);
    let end = range.end.min(block.end);
    (start < end).then(|| (start - base).min(len)..(end - base).min(len)).filter(|r| !r.is_empty())
}

/// Adds `style` over `range` on top of existing non-overlapping highlights.
fn overlay(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    range: Range<usize>,
    style: HighlightStyle,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut out = Vec::with_capacity(highlights.len() + 2);
    let mut cursor = range.start;
    for (r, h) in highlights {
        if r.end <= range.start || r.start >= range.end {
            out.push((r, h));
            continue;
        }
        if r.start < range.start {
            out.push((r.start..range.start, h));
        }
        if cursor < r.start.max(range.start) {
            out.push((cursor..r.start.max(range.start), style));
        }
        let inner = r.start.max(range.start)..r.end.min(range.end);
        out.push((inner.clone(), h.highlight(style)));
        cursor = inner.end;
        if r.end > range.end {
            out.push((range.end..r.end, h));
        }
    }
    if cursor < range.end {
        out.push((cursor..range.end, style));
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}
