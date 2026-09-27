use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext};
use tachyon_md::BlockKind;

use crate::Editor;

fn open<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut VisualTestContext) {
    cx.update(crate::init);
    let text = text.to_owned();
    let (editor, cx) = cx.add_window_view(move |window, cx| Editor::new(&text, window, cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, _| e.text())
}

fn kinds(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<BlockKind> {
    editor.read_with(cx, |e, _| {
        assert!(!e.document().is_dirty(), "reparse pending");
        e.document().blocks().iter().map(|b| b.parsed().kind.clone()).collect()
    })
}

#[gpui::test]
fn typing_edits_at_the_caret_and_reparses(cx: &mut TestAppContext) {
    let (editor, cx) = open("para\n", cx);
    cx.simulate_input("#");
    cx.simulate_keystrokes("space");
    assert_eq!(text(&editor, cx), "# para\n");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Heading(1)]);
}

#[gpui::test]
fn enter_splits_blocks_and_the_caret_owns_the_active_block(cx: &mut TestAppContext) {
    let (editor, cx) = open("one two\n", cx);
    cx.simulate_keystrokes("ctrl-right enter enter");
    assert_eq!(text(&editor, cx), "one\n\n two\n");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph, BlockKind::Paragraph]);
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1));

    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(0));
}

#[gpui::test]
fn arrows_cross_block_boundaries(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\nbody\n", cx);
    cx.simulate_keystrokes("down down");
    let (head, active) = editor.read_with(cx, |e, _| (e.head(), e.active_block()));
    assert_eq!((head, active), ("# Title\n\n".len(), Some(1)));
    cx.simulate_keystrokes("end left left");
    assert_eq!(editor.read_with(cx, |e, _| e.head()), "# Title\n\nbo".len());
    cx.simulate_keystrokes("up up");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(0));
}

#[gpui::test]
fn backspace_at_a_block_start_joins_blocks(cx: &mut TestAppContext) {
    let (editor, cx) = open("one\n\ntwo\n", cx);
    cx.simulate_keystrokes("down down backspace backspace");
    assert_eq!(text(&editor, cx), "onetwo\n");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph]);
}

#[gpui::test]
fn undo_and_redo_restore_text_and_blocks(cx: &mut TestAppContext) {
    let (editor, cx) = open("para\n", cx);
    cx.simulate_keystrokes("end enter enter");
    cx.simulate_input("x");
    assert_eq!(text(&editor, cx), "para\n\nx\n");
    assert_eq!(kinds(&editor, cx).len(), 2);
    // Each line break is its own undo step, separate from the typing after it.
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "para\n\n\n");
    cx.simulate_keystrokes("secondary-z secondary-z");
    assert_eq!(text(&editor, cx), "para\n");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph]);
    cx.simulate_keystrokes("secondary-shift-z secondary-shift-z secondary-shift-z");
    assert_eq!(text(&editor, cx), "para\n\nx\n");
    assert_eq!(kinds(&editor, cx).len(), 2);
}

#[gpui::test]
fn selection_copy_cut_and_paste(cx: &mut TestAppContext) {
    let (editor, cx) = open("hello world\n", cx);
    cx.simulate_keystrokes(
        "shift-right shift-right shift-right shift-right shift-right secondary-x",
    );
    assert_eq!(text(&editor, cx), " world\n");
    cx.simulate_keystrokes("end secondary-v");
    assert_eq!(text(&editor, cx), " worldhello\n");
    cx.simulate_keystrokes("secondary-a secondary-c");
    let clipboard = cx.update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text()));
    assert_eq!(clipboard.as_deref(), Some(" worldhello\n"));
}

#[gpui::test]
fn selection_across_blocks_deletes_them(cx: &mut TestAppContext) {
    let (editor, cx) = open("# A\n\ntext\n\n- item\n", cx);
    cx.simulate_keystrokes("right right shift-down shift-down shift-down shift-down");
    cx.simulate_keystrokes("shift-right shift-right backspace");
    assert_eq!(text(&editor, cx), "# item\n");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Heading(1)]);
}

#[gpui::test]
fn ime_composition_replaces_marked_text(cx: &mut TestAppContext) {
    let (editor, cx) = open("x\n", cx);
    cx.simulate_keystrokes("end");
    editor.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "に", None, window, cx);
        e.replace_and_mark_text_in_range(None, "にほ", None, window, cx);
        assert!(e.marked_text_range(window, cx).is_some());
        e.replace_text_in_range(None, "日本", window, cx);
        assert!(e.marked_text_range(window, cx).is_none());
    });
    assert_eq!(text(&editor, cx), "x日本\n");
    // One undo group for the whole composition.
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "x\n");
}

#[gpui::test]
fn large_paste_parses_in_the_background(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    let section = "## Section\n\nSome *text* here.\n\n```\ncode\n```\n\n";
    let paste = section.repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / section.len() + 10);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(paste.clone()));
    cx.simulate_keystrokes("secondary-v");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), paste);
    let kinds = kinds(&editor, cx);
    assert!(kinds.iter().all(|k| *k != BlockKind::Unparsed));
    assert_eq!(kinds[0], BlockKind::Heading(2));
}

#[gpui::test]
fn a_large_paste_is_parsed_near_the_caret_first(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    let section = "## Section\n\nSome *text* here.\n\n```\ncode\n```\n\n";
    let paste = section.repeat(3 * tachyon_doc::PARSE_CHUNK / section.len());
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(paste));
    // Not a simulated keystroke: that would run until every chunk is back.
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    let unparsed = |cx: &mut VisualTestContext| -> Vec<bool> {
        editor.read_with(cx, |e, _| {
            e.document().blocks().iter().map(|b| b.parsed().kind == BlockKind::Unparsed).collect()
        })
    };
    // Step the executors until the first chunk comes back (the paste itself
    // lands first, as unparsed blocks).
    while !unparsed(cx).contains(&false) || unparsed(cx).len() < 2 {
        assert!(cx.executor().tick(), "parse finished without applying a chunk");
    }
    let blocks = unparsed(cx);
    assert!(!blocks[blocks.len() - 1], "the caret's end of the paste is parsed first");
    assert!(blocks[0], "the far end waits for later chunks");

    cx.run_until_parked();
    assert!(!unparsed(cx).contains(&true));
}

#[gpui::test]
fn keys_typed_right_after_a_large_paste_land_after_it(cx: &mut TestAppContext) {
    let (editor, cx) = open("start\n", cx);
    let paste = "Pasted paragraph.\n\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 10);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(paste.clone()));
    cx.simulate_keystrokes("ctrl-end");
    // The paste is prepared off the UI thread; type before it is applied.
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    assert_eq!(text(&editor, cx), "start\n", "still being prepared");
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), format!("start\n{paste}x"));

    // Undo removes the typed key, then the whole paste.
    cx.simulate_keystrokes("secondary-z");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "start\n");
}

#[gpui::test]
fn the_caret_stays_in_view_after_a_long_paste_and_ctrl_end(cx: &mut TestAppContext) {
    // Long enough to arrive as unparsed placeholder blocks first.
    let text: String = (0..8000).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let (editor, cx) = open("", cx);
    let caret_block_is_drawn = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.active_block().is_some_and(|i| e.rendered.contains(&i)))
    };
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    cx.simulate_keystrokes("secondary-v");
    cx.run_until_parked();
    assert!(caret_block_is_drawn(cx), "after pasting");

    cx.simulate_keystrokes("ctrl-home");
    cx.run_until_parked();
    assert!(caret_block_is_drawn(cx), "after jumping to the start");
    cx.simulate_keystrokes("ctrl-end");
    cx.run_until_parked();
    assert!(caret_block_is_drawn(cx), "after jumping to the end");
}

#[gpui::test]
fn streamed_parse_results_keep_the_view_on_the_same_text(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    let text: String = (0..20_000).map(|i| format!("Paragraph {i}.\n\n")).collect();
    editor.update(cx, |e, cx| e.replace(0..0, &text, cx));
    // Look at the middle of the paste while its parse streams in, as if the
    // user scrolled there (or the caret has not been drawn yet).
    let wanted = text.find("Paragraph 10000.").expect("fixture");
    editor.update(cx, |e, _| {
        let item = e.document().block_at(wanted).expect("placeholder at the offset");
        e.list.scroll_to(gpui::ListOffset { item_ix: item, offset_in_item: gpui::px(0.) });
    });
    let shown = editor
        .read_with(cx, |e, _| e.document().block_range(e.list.logical_scroll_top().item_ix).start);
    cx.run_until_parked();

    let top = editor.read_with(cx, |e, _| {
        assert!(!e.document().is_dirty());
        e.document().block_range(e.list.logical_scroll_top().item_ix)
    });
    assert!(top.contains(&shown), "view moved from byte {shown} to block {top:?}");
}

#[gpui::test]
fn the_caret_after_a_final_newline_has_a_position(cx: &mut TestAppContext) {
    for text in ["para\n", "para\n\n", "\n", "para"] {
        let (editor, cx) = open(text, cx);
        editor.update(cx, |e, cx| e.move_to(text.len(), false, cx));
        cx.run_until_parked();
        let position = editor.read_with(cx, |e, _| e.position_for_offset(text.len()));
        assert!(position.is_some(), "no caret position at the end of {text:?}");
    }
}

#[gpui::test]
fn ctrl_end_reveals_the_end_of_a_long_document(cx: &mut TestAppContext) {
    // Opened, not pasted: nothing below the first screen has been measured.
    let text: String = (0..8000).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let (editor, cx) = open(&text, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.run_until_parked();
    let drawn =
        editor.read_with(cx, |e, _| e.active_block().is_some_and(|i| e.rendered.contains(&i)));
    assert!(drawn);
}

#[test]
fn drawn_blocks_map_through_splices() {
    use crate::editor::map_drawn;
    // Before, after, and across the drawn range.
    assert_eq!(map_drawn(&(5..10), &(0..2), 5), 8..13);
    assert_eq!(map_drawn(&(5..10), &(12..14), 1), 5..10);
    assert_eq!(map_drawn(&(5..10), &(7..8), 2), 5..11, "a drawn block split in two");
    // A drawn block replaced by many: only as many count as drawn.
    assert_eq!(map_drawn(&(0..1), &(0..1), 640), 0..1);
    assert_eq!(map_drawn(&(3..6), &(4..9), 100), 3..6);
    assert_eq!(map_drawn(&(3..6), &(0..9), 1), 0..1, "merged into one");
}

#[gpui::test]
fn save_writes_the_file_with_its_line_endings(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("tachyon-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("note.md");
    std::fs::write(&path, "# Note\r\n\r\nbody\r\n").unwrap();

    let (editor, cx) = open("", cx);
    let loaded = tachyon_doc::Document::new(&std::fs::read_to_string(&path).unwrap());
    editor.update(cx, |e, cx| {
        e.set_document(loaded, cx);
        e.set_file(path.clone(), cx);
    });
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("!");
    assert!(editor.read_with(cx, |e, _| e.is_modified()));
    assert!(editor.read_with(cx, |e, _| e.title()).starts_with("• note.md"));

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Note\r\n\r\nbody\r\n!");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));
    assert_eq!(editor.read_with(cx, |e, _| e.title()), "note.md - Tachyon");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn atomic_write_replaces_content_and_leaves_no_temp_file() {
    let dir = std::env::temp_dir().join(format!("tachyon-atomic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.md");
    std::fs::write(&path, "old").unwrap();
    let permissions = std::fs::metadata(&path).unwrap().permissions();

    crate::editor::write_atomically(&path, b"new contents").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new contents");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions(), permissions);
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary file left behind");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn in_a_list_only_the_item_under_the_caret_is_raw(cx: &mut TestAppContext) {
    let (editor, cx) = open("- one\n- two\n  - nested\n\nafter\n", cx);
    let leaf = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.active_leaf());
    assert_eq!(leaf(cx), Some((0, 0..6)));
    cx.simulate_keystrokes("down");
    assert_eq!(leaf(cx), Some((1, 6..12)));
    cx.simulate_keystrokes("down end");
    cx.simulate_input("!");
    assert_eq!(text(&editor, cx), "- one\n- two\n  - nested!\n\nafter\n");
    assert_eq!(leaf(cx), Some((2, 12..24)));
    // Outside containers the whole block is raw, as before.
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(leaf(cx), None);
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1));
}

#[gpui::test]
fn unsaved_close_prompt_works_from_the_keyboard(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_prompt_builder(crate::keyboard_prompt));
    let (editor, cx) = open("text\n", cx);
    cx.simulate_input("x");
    cx.simulate_keystrokes("secondary-w");
    assert!(cx.update(|window, _| window.has_active_prompt()));

    // Escape picks "Cancel": the prompt closes, the window and edit stay.
    cx.simulate_keystrokes("escape");
    assert!(!cx.update(|window, _| window.has_active_prompt()));
    assert_eq!(text(&editor, cx), "xtext\n");

    // Tab to "Don't Save", Enter: the window closes without saving.
    cx.simulate_keystrokes("secondary-w");
    cx.simulate_keystrokes("tab enter");
    cx.run_until_parked();
    assert!(cx.windows().is_empty(), "window closed");
}

#[gpui::test]
fn typing_in_a_tall_block_keeps_the_scroll_position(cx: &mut TestAppContext) {
    let lines: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let text = format!("```\n{lines}```\n\nafter\n");
    let (editor, cx) = open(&text, cx);
    // Caret on line 150 of the code block, scrolled so the block's top is
    // far above the viewport.
    let caret = text.find("line 150").unwrap();
    editor.update(cx, |e, cx| e.move_to(caret, false, cx));
    cx.run_until_parked();
    let before = editor.read_with(cx, |e, _| e.list.logical_scroll_top());
    assert_eq!(before.item_ix, 0);
    assert!(before.offset_in_item > gpui::px(100.), "caret revealed inside the block: {before:?}");

    cx.simulate_input("x");
    cx.run_until_parked();
    let after = editor.read_with(cx, |e, _| e.list.logical_scroll_top());
    assert_eq!(after.item_ix, 0);
    let drift = (after.offset_in_item - before.offset_in_item).abs();
    assert!(drift < gpui::px(40.), "scroll jumped from {before:?} to {after:?}");
}

/// Window position inside the glyph starting at `offset` once its block is
/// rendered. Measured on the raw layout (the caret is moved into the block),
/// then shifted by the raw card's inset. Valid while every block above keeps
/// its layout: a raw block is taller than rendered, so after measuring only
/// blocks below may become raw.
fn rendered_glyph_point(
    editor: &Entity<Editor>,
    offset: usize,
    cx: &mut VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    editor.update(cx, |e, cx| e.move_to(offset, false, cx));
    cx.run_until_parked();
    editor.read_with(cx, |e, _| {
        let at = e.position_for_offset(offset).expect("offset is in the raw block");
        let line_height = e.active_layout.as_ref().expect("raw block is laid out").0.line_height();
        at + gpui::point(gpui::px(1.) - crate::render::RAW_INSET, line_height / 2.)
    })
}

fn selection(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> std::ops::Range<usize> {
    editor.read_with(cx, |e, _| e.selection.clone())
}

const THREE_PARAGRAPHS: &str = "alpha beta\n\ngamma delta\n\nepsilon zeta\n";

fn find(needle: &str) -> usize {
    THREE_PARAGRAPHS.find(needle).expect("fixture contains the needle")
}

#[gpui::test]
fn clicking_a_rendered_block_moves_the_caret_into_it(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    let target = rendered_glyph_point(&editor, find("delta"), cx);
    editor.update(cx, |e, cx| e.move_to(find("epsilon"), false, cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(2));

    cx.simulate_click(target, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), find("delta")..find("delta"));
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1));
}

#[gpui::test]
fn dragging_selects_across_blocks_until_the_button_is_released(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    let gamma = rendered_glyph_point(&editor, find("gamma"), cx);
    let delta = rendered_glyph_point(&editor, find("delta"), cx);
    // Leaves the caret in the last block, which stays raw for the press.
    let zeta = rendered_glyph_point(&editor, find("zeta"), cx)
        + gpui::point(crate::render::RAW_INSET, gpui::px(0.));

    let (left, none) = (gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_down(zeta, left, none);
    cx.simulate_mouse_move(delta, left, none);
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), find("delta")..find("zeta"));

    cx.simulate_mouse_up(delta, left, none);
    cx.simulate_mouse_move(gamma, None, none);
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), find("delta")..find("zeta"), "moved after release");
}

#[gpui::test]
fn double_click_selects_a_word(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    let inside = rendered_glyph_point(&editor, find("gamma") + 2, cx);
    editor.update(cx, |e, cx| e.move_to(find("epsilon"), false, cx));
    cx.run_until_parked();

    cx.simulate_event(gpui::MouseDownEvent {
        position: inside,
        modifiers: gpui::Modifiers::none(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), find("gamma")..find("gamma") + "gamma".len());
}

#[gpui::test]
fn page_down_and_up_move_by_about_a_screen(cx: &mut TestAppContext) {
    let text: String = (0..400).map(|i| format!("Line {i}\n")).collect();
    let (editor, cx) = open(&text, cx);
    let line_of = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.document().buffer().rope().byte_to_line(e.head()))
    };
    cx.simulate_keystrokes("pagedown");
    cx.run_until_parked();
    let after_down = line_of(cx);
    assert!(after_down > 10, "moved {after_down} lines");
    cx.simulate_keystrokes("pagedown");
    cx.run_until_parked();
    assert!(line_of(cx) > after_down);

    cx.simulate_keystrokes("shift-pageup");
    cx.run_until_parked();
    let (line, selected) = editor.read_with(cx, |e, _| {
        (e.document().buffer().rope().byte_to_line(e.head()), !e.selection.is_empty())
    });
    assert_eq!(line, after_down);
    assert!(selected, "shift extends the selection");
    cx.simulate_keystrokes("pageup pageup pageup");
    cx.run_until_parked();
    assert_eq!(line_of(cx), 0);
}

#[gpui::test]
fn find_types_into_the_bar_and_steps_through_matches(cx: &mut TestAppContext) {
    let doc = "one fish\n\ntwo fish\n\nred Fish\n\nblue fish\n";
    let (editor, cx) = open(doc, cx);
    let selected = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            (e.document().buffer().text()[e.selection.clone()].to_owned(), e.selection.start)
        })
    };
    let status = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.find.as_ref().map(|f| f.status()))
    };

    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("fish");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc, "typing goes to the query, not the document");
    assert_eq!(selected(cx), ("fish".to_owned(), 4));
    assert_eq!(status(cx).as_deref(), Some("1/4"), "lowercase query matches Fish too");

    cx.simulate_keystrokes("enter");
    assert_eq!(selected(cx).1, 14);
    cx.simulate_keystrokes("f3 f3 f3");
    assert_eq!(selected(cx).1, 4, "wraps to the first match");
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(selected(cx).1, doc.rfind("fish").expect("fixture"), "wraps backwards");
    assert_eq!(status(cx).as_deref(), Some("4/4"));

    // Backspace edits the query; an uppercase letter makes it case-sensitive.
    cx.simulate_keystrokes("backspace backspace backspace backspace");
    cx.simulate_input("Fish");
    cx.run_until_parked();
    assert_eq!(status(cx).as_deref(), Some("1/1"));
    assert_eq!(selected(cx), ("Fish".to_owned(), doc.find("Fish").expect("fixture")));
    cx.simulate_input("x");
    assert_eq!(status(cx).as_deref(), Some("no matches"));

    // Escape closes the bar; typing edits the document again.
    cx.simulate_keystrokes("escape");
    assert_eq!(status(cx), None);
    cx.simulate_keystrokes("secondary-z");
    cx.simulate_input("!");
    cx.run_until_parked();
    assert_ne!(text(&editor, cx), doc);
}

#[gpui::test]
fn find_starts_with_the_selected_text(cx: &mut TestAppContext) {
    let text = "alpha beta alpha beta\n";
    let (editor, cx) = open(text, cx);
    editor.update(cx, |e, cx| {
        e.move_to(6, false, cx);
        e.move_to(10, true, cx);
    });
    cx.simulate_keystrokes("secondary-f");
    let (query, status) = editor.read_with(cx, |e, _| {
        let find = e.find.as_ref().expect("bar is open");
        (find.query.clone(), find.status())
    });
    assert_eq!((query.as_str(), status.as_str()), ("beta", "1/2"));
}
