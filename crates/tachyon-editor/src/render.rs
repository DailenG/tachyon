//! Block rendering: the block holding the caret shows its raw Markdown; every
//! other block shows its rendered IR with syntax hidden.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, ElementInputHandler, Entity, FontWeight, HighlightStyle, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels, Render, SharedString, Size,
    StyledImage as _, StyledText, TextLayout, UnderlineStyle, Window, canvas, div, fill, img, list,
    prelude::*, px, relative, size,
};
use tachyon_md::{BlockKind, LineInfo, LineKind, Marker, ParsedBlock};

use crate::editor::{Editor, KEY_CONTEXT, TextTarget};
use crate::theme::Theme;

const INDENT: f32 = 22.;

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
        div()
            .id("editor")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(self.theme.surface.canvas)
            .text_color(self.theme.text.primary)
            .font_family(self.theme.text_font.clone())
            .text_size(self.theme.text_size)
            .line_height(relative(1.6))
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
            .on_action(cx.listener(Self::toggle_frame_stats))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|editor, _: &MouseDownEvent, window, cx| {
                    window.focus(&editor.focus, cx);
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|editor, _, _, _| editor.selecting = false))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|editor, _, _, _| editor.selecting = false),
            )
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|editor, index, window, cx| {
                        editor.render_block(index, window, cx)
                    }),
                )
                .size_full()
                .py_6(),
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
        // Rows that fit under the header in the window: line height plus the 4 px row gap, with
        // the margins, padding and header taken off.
        let row_height = theme.text_size * 1.6 + theme.scaled(px(4.));
        let room = viewport.height - OVERLAY_MARGIN * 2. - theme.scaled(px(16.)) - row_height;
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
                .child(div().flex_none().child(item.label.clone()))
                .children(item.detail.clone().map(|detail| {
                    div()
                        .text_color(detail_color)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(detail)
                }))
                .on_click(cx.listener(move |editor, _, _, cx| editor.picker_pick(Some(row), cx)))
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
                .top_2()
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
}

impl Editor {
    fn render_block(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(block) = self.doc.blocks().get(index) else {
            return div().into_any_element();
        };
        self.rendering = Some(match self.rendering.take() {
            Some(r) => r.start.min(index)..r.end.max(index + 1),
            None => index..index + 1,
        });
        let range = self.doc.block_range(index);
        let parsed = block.parsed_shared();
        let active = self.active_block() == Some(index);
        let leaf = if active { self.active_leaf() } else { None };
        let content = if block.is_stale() || (active && leaf.is_none()) {
            let code = matches!(parsed.kind, BlockKind::CodeBlock { .. } | BlockKind::Html);
            self.render_raw(range, code, window, cx)
        } else {
            self.render_rendered(range.start, parsed, leaf, window, cx)
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

    /// The active block: its source, caret and selection.
    fn render_raw(
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

        let mut highlights = Vec::new();
        if code {
            for (token, style) in self.code_tokens(&range) {
                if let Some(local) = intersect(&token, &range, base, len) {
                    highlights = overlay(highlights, local, theme.highlight(style));
                }
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
        let show_caret = self.focus.is_focused(window) && !self.bar_open();
        let cursor_color = theme.editing.caret;
        let paint_layout = layout.clone();

        let mut element = div()
            .relative()
            .my_1()
            .px(theme.scaled(RAW_INSET) - px(1.))
            .rounded(theme.radius_small)
            .border_1()
            .border_color(theme.border.control)
            .bg(theme.surface.raised)
            .cursor_text()
            .child(styled)
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        let caret_position =
                            caret.and_then(|caret| paint_layout.position_for_index(caret));
                        let line_height = paint_layout.line_height();
                        let scrolled = editor.update(cx, |editor, _| {
                            editor.active_layout = Some((paint_layout.clone(), base));
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
        if code {
            element = element.font_family(theme.code_font.clone()).text_size(theme.code_size);
        }
        with_mouse(element, layout, TextTarget::Raw { base }, cx).into_any_element()
    }

    /// Rendered lines with syntax hidden; the lines of `raw_leaf` (the leaf
    /// holding the caret, with its absolute source range) are shown raw.
    fn render_rendered(
        &mut self,
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

        let mut column = div().flex().flex_col().my_2();
        let mut raw_shown = false;
        for (i, line) in ir.lines.iter().enumerate() {
            if let Some((leaf, range)) = &raw_leaf
                && line.leaf == *leaf
            {
                if !std::mem::replace(&mut raw_shown, true) {
                    let code = ir.lines.iter().any(|l| {
                        l.leaf == *leaf && matches!(l.kind, LineKind::Code | LineKind::Html)
                    });
                    column = column.child(self.render_raw(range.clone(), code, window, cx));
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
        if let BlockKind::Heading(1 | 2) = parsed.kind {
            column = column.pb_1().border_b_1().border_color(self.theme.border.subtle);
        }
        column.into_any_element()
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
