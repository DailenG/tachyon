//! Block rendering: the block holding the caret shows its raw Markdown; every
//! other block shows its rendered IR with syntax hidden.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, ElementInputHandler, Entity, FontWeight, HighlightStyle, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, Render, SharedString, StyledText, TextLayout,
    UnderlineStyle, Window, canvas, div, fill, list, prelude::*, px, relative, size,
};
use tachyon_md::{BlockKind, LineInfo, LineKind, Marker, ParsedBlock};

use crate::editor::{Editor, KEY_CONTEXT, TextTarget};

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
        let editor = cx.entity();
        let focus = self.focus.clone();
        // Spacing given in rems (padding, gaps) follows the zoom.
        window.set_rem_size(self.theme.scaled(BASE_REM_SIZE));
        div()
            .id("editor")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
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
            .children(self.find_bar())
            .children(self.outline_bar(cx))
    }
}

impl Editor {
    /// The find bar: the query with a caret and the match count.
    fn find_bar(&self) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let theme = &self.theme;
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
                        .gap_3()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme.raw_background)
                        .border_1()
                        .border_color(theme.rule)
                        .text_color(theme.foreground)
                        .child(div().text_color(theme.muted).child("Find"))
                        .child(
                            div()
                                .min_w(px(200.))
                                .child(field(&find.query, !find.editing_replacement)),
                        )
                        .child(div().text_color(theme.muted).child(find.status()))
                        .children(find.replacement.as_ref().map(|replacement| {
                            div()
                                .flex()
                                .gap_3()
                                .child(div().text_color(theme.muted).child("Replace"))
                                .child(
                                    div()
                                        .min_w(px(160.))
                                        .child(field(replacement, find.editing_replacement)),
                                )
                        })),
                )
                .into_any_element(),
        )
    }

    /// The heading list: the filter with a caret, then the matching headings, the chosen one
    /// highlighted. Clicking a row jumps to it.
    fn outline_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let outline = self.outline.as_ref()?;
        let theme = &self.theme;
        let rows = outline.window().map(|row| {
            let heading = &outline.headings[outline.matches[row]];
            div()
                .id(row)
                .px_2()
                .rounded_sm()
                .pl(self.theme.scaled(px(8. + 14. * f32::from(heading.level.saturating_sub(1)))))
                .when(row == outline.selected, |d| d.bg(theme.selection))
                .child(heading.title.clone())
                .on_click(cx.listener(move |editor, _, _, cx| editor.outline_jump(Some(row), cx)))
        });
        let note = if outline.headings.is_empty() {
            Some("no headings")
        } else if outline.matches.is_empty() {
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
                        .w(self.theme.scaled(px(420.)))
                        .rounded_md()
                        .bg(theme.raw_background)
                        .border_1()
                        .border_color(theme.rule)
                        .text_color(theme.foreground)
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(div().text_color(theme.muted).child("Go to heading"))
                                .child(field(&outline.query, true)),
                        )
                        .children(rows)
                        .children(note.map(|note| div().text_color(theme.muted).child(note))),
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

    /// Background marks for `block` (absolute source range): find matches, then the selection.
    fn marks(&self, block: &Range<usize>) -> Vec<(Range<usize>, gpui::Hsla)> {
        let mut marks = Vec::new();
        if let Some(find) = &self.find {
            for (m, current) in find.matches_in(block) {
                let color = if current { self.theme.find_current } else { self.theme.find_match };
                marks.push((m, color));
            }
        }
        if !self.selection.is_empty() {
            marks.push((self.selection.clone(), self.theme.selection));
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
                .bg(self.theme.raw_background)
                .border_1()
                .border_color(if over { gpui::red() } else { self.theme.rule })
                .font_family(self.theme.code_font.clone())
                .text_size(px(12.))
                .text_color(self.theme.muted)
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
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(div().w_full().max_w(self.theme.content_width).px_8().child(content))
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
        for (mark, color) in self.marks(&range) {
            if let Some(local) = intersect(&mark, &range, base, len) {
                let style = HighlightStyle { background_color: Some(color), ..Default::default() };
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
                        color: Some(theme.foreground),
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
        let focused = self.focus.is_focused(window);
        let cursor_color = theme.cursor;
        let paint_layout = layout.clone();

        let mut element = div()
            .relative()
            .my_1()
            .px(theme.scaled(RAW_INSET))
            .rounded_md()
            .bg(theme.raw_background)
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
                            caret_position.is_some_and(|p| editor.reveal_caret_at(p.y, line_height))
                        });
                        if scrolled {
                            // The caret was out of view: draw again.
                            window.request_animation_frame();
                        }
                        if focused
                            && let Some(caret) = caret
                            && let Some(position) = paint_layout.position_for_index(caret)
                        {
                            let caret = fill(
                                gpui::Bounds::new(
                                    position,
                                    size(px(2.), paint_layout.line_height()),
                                ),
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
        let marks: Vec<(Range<usize>, gpui::Hsla)> = self
            .marks(&block)
            .into_iter()
            .filter_map(|(mark, color)| {
                let local = intersect(&mark, &block, block_start, block_len)?;
                let visible = ir.source_to_visible(local.start)..ir.source_to_visible(local.end);
                (!visible.is_empty()).then_some((visible, color))
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
            column = column.child(self.render_line(
                line,
                line.start..end,
                &parsed,
                block_start,
                &marks,
                cx,
            ));
        }
        if let BlockKind::Heading(1 | 2) = parsed.kind {
            column = column.pb_1().border_b_1().border_color(self.theme.rule);
        }
        column.into_any_element()
    }

    fn render_line(
        &self,
        line: &LineInfo,
        visible: Range<usize>,
        parsed: &Arc<ParsedBlock>,
        block_start: usize,
        marks: &[(Range<usize>, gpui::Hsla)],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = &self.theme;
        let content: AnyElement = match line.kind {
            LineKind::Rule => div().w_full().h(px(1.)).my_3().bg(theme.rule).into_any_element(),
            LineKind::TableRow { header } => {
                let mut row = div().flex().w_full();
                let mut cell_start = visible.start;
                let text = &parsed.ir.text[visible.clone()];
                for cell in text.split('\t') {
                    let range = cell_start..cell_start + cell.len();
                    let mut cell_el =
                        div().flex_1().min_w_0().px_2().border_1().border_color(theme.rule).child(
                            self.rendered_text(range.clone(), parsed, block_start, marks, cx),
                        );
                    if header {
                        cell_el = cell_el.font_weight(FontWeight::BOLD).bg(theme.code_background);
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
                            .bg(theme.code_background)
                            .px_3();
                    }
                    LineKind::Html => {
                        el = el
                            .font_family(theme.code_font.clone())
                            .text_size(theme.code_size)
                            .text_color(theme.muted);
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
                .border_color(theme.quote_bar)
                .pl(theme.scaled(px(12. * f32::from(line.quote))))
                .text_color(theme.muted);
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
        let el = div().w(theme.scaled(px(INDENT))).flex_none().text_color(theme.muted);
        match marker {
            Marker::Bullet => el.child("•").into_any_element(),
            Marker::Ordered(n) => el.child(SharedString::from(format!("{n}."))).into_any_element(),
            Marker::Task { checked } => el
                .flex()
                .items_center()
                .child(
                    div()
                        .size(theme.scaled(px(13.)))
                        .rounded_sm()
                        .border_1()
                        .border_color(theme.muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(theme.scaled(px(11.)))
                        .line_height(theme.scaled(px(11.)))
                        .when(checked, |d| {
                            d.bg(theme.accent).text_color(theme.background).child("✓")
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
        marks: &[(Range<usize>, gpui::Hsla)],
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
        for (mark, color) in marks {
            let start = mark.start.max(visible.start);
            let end = mark.end.min(visible.end);
            if start < end {
                highlights = overlay(
                    highlights,
                    start - visible.start..end - visible.start,
                    HighlightStyle { background_color: Some(*color), ..Default::default() },
                );
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

/// A find-bar field's text, with a caret when it receives typing.
fn field(text: &str, active: bool) -> String {
    if active { format!("{text}\u{258f}") } else { text.to_owned() }
}

/// Horizontal padding of the active block's card: raw text sits this much to
/// the right of the same text rendered.
pub(crate) const RAW_INSET: gpui::Pixels = px(8.);

/// GPUI's default rem size, the 100 % zoom reference.
const BASE_REM_SIZE: gpui::Pixels = px(16.);

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
