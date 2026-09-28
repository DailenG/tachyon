use gpui::{Entity, EntityInputHandler, TestAppContext, VisualContext, VisualTestContext};
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
    // Not a simulated keystroke: that runs until parked, hiding whether the key itself did any
    // work. The key must be queued, not flush the paste or apply itself synchronously.
    editor.update_in(cx, |e, window, cx| e.replace_text_in_range(None, "x", window, cx));
    assert_eq!(text(&editor, cx), "start\n", "the key is queued, not applied yet");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), format!("start\n{paste}x"));

    // Undo removes the typed key, then the whole paste.
    cx.simulate_keystrokes("secondary-z");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "start\n");
}

#[gpui::test]
fn a_non_queueable_action_flushes_the_paste_and_queued_text_first(cx: &mut TestAppContext) {
    let (editor, cx) = open("start\n", cx);
    let paste = "Pasted paragraph.\n\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 10);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(paste.clone()));
    cx.simulate_keystrokes("ctrl-end");
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    editor.update_in(cx, |e, window, cx| e.replace_text_in_range(None, "x", window, cx));
    assert_eq!(text(&editor, cx), "start\n", "queued, not yet applied");
    // Backspace does not queue: it flushes the paste, then the queued "x" right after it, and
    // only then deletes - removing the "x" it just landed, keeping paste, queue, action in order.
    editor.update_in(cx, |e, window, cx| e.backspace(&crate::editor::Backspace, window, cx));
    assert_eq!(text(&editor, cx), format!("start\n{paste}"));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), format!("start\n{paste}"), "nothing was left pending");
}

#[gpui::test]
fn a_keystroke_through_the_real_input_path_is_queued_not_flushed(cx: &mut TestAppContext) {
    let (editor, cx) = open("start\n", cx);
    let paste = "Pasted paragraph.\n\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 10);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(paste.clone()));
    cx.simulate_keystrokes("ctrl-end");
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    assert_eq!(text(&editor, cx), "start\n", "still being prepared");
    // The real path this time: `lib::init`'s keystroke interceptor, then the keymap (no binding
    // claims a bare "z"), then the input handler - not a direct `replace_text_in_range` call,
    // and not `simulate_input`/`simulate_keystrokes`, which both run until parked and so would
    // hide whether the keystroke itself did any synchronous work.
    let window = cx.window_handle();
    cx.dispatch_keystroke(window, gpui::Keystroke::parse("z").unwrap());
    assert_eq!(text(&editor, cx), "start\n", "the keystroke is queued, not flushed on its frame");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), format!("start\n{paste}z"));
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
fn clicking_below_the_last_block_puts_the_caret_at_the_end(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    let last = editor.read_with(cx, |e, _| e.document().blocks().len() - 1);
    // `window_item_bounds` is one-frame-stale (see its own doc comment): paint a second frame
    // so it reflects the blocks this frame actually drew, not the empty range from the first.
    editor.update(cx, |e, cx| e.move_to(0, false, cx));
    cx.run_until_parked();
    let bounds = editor
        .read_with(cx, |e, _| e.window_item_bounds.get(&last).copied())
        .expect("last block rendered");
    let point = gpui::point(bounds.left() + gpui::px(10.), bounds.bottom() + gpui::px(20.));

    cx.simulate_click(point, gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(selection(&editor, cx), THREE_PARAGRAPHS.len()..THREE_PARAGRAPHS.len());
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(last));
}

#[gpui::test]
fn clicking_in_the_left_margin_activates_the_nearest_rendered_block(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    editor.update(cx, |e, cx| e.move_to(find("alpha"), false, cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(0));

    // Block 1 ("gamma delta") is rendered, not active: a click in its row's left margin, well
    // outside the centered content column, should land on it at its (only) line.
    let bounds = editor
        .read_with(cx, |e, _| e.window_item_bounds.get(&1).copied())
        .expect("second block rendered");
    let point = gpui::point(gpui::px(2.), bounds.top() + bounds.size.height / 2.);
    cx.simulate_click(point, gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1));
    assert_eq!(selection(&editor, cx), find("gamma")..find("gamma"));
}

#[gpui::test]
fn clicking_beside_a_wrapped_row_lands_near_that_row(cx: &mut TestAppContext) {
    // One long single-line paragraph: wide enough windows wrap it into many visual rows.
    let doc = "word ".repeat(400);
    let (editor, cx) = open(&doc, cx);
    editor.update(cx, |e, cx| e.move_to(0, false, cx));
    cx.run_until_parked();
    let bounds =
        editor.read_with(cx, |e, _| e.window_item_bounds.get(&0).copied()).expect("block rendered");
    let line_height = editor.read_with(cx, |e, _| e.theme.text_size * 1.6);

    // Both clicks are in the left margin (well left of the centered content column), so the
    // horizontal column is 0 for each; only the row differs.
    let x = gpui::px(2.);
    cx.simulate_click(gpui::point(x, bounds.top() + line_height * 1.5), gpui::Modifiers::none());
    cx.run_until_parked();
    let near = editor.read_with(cx, |e, _| e.head());

    cx.simulate_click(gpui::point(x, bounds.top() + line_height * 4.5), gpui::Modifiers::none());
    cx.run_until_parked();
    let far = editor.read_with(cx, |e, _| e.head());

    assert!(near > 0, "not the very first row: {near}");
    assert!(
        far > near + 20,
        "a lower row lands further into the wrapped line: near={near} far={far}"
    );
    assert!(far < doc.len(), "still inside the block: {far}");
}

#[gpui::test]
fn clicking_just_left_of_the_text_in_a_wide_window_lands_at_the_line_start(
    cx: &mut TestAppContext,
) {
    let doc = "one two three\n";
    let (editor, cx) = open(doc, cx);
    editor.update(cx, |e, cx| e.move_to(0, false, cx));
    cx.run_until_parked();
    let bounds =
        editor.read_with(cx, |e, _| e.window_item_bounds.get(&0).copied()).expect("block rendered");
    let content_width = editor.read_with(cx, |e, _| e.theme.content_width);

    // The same insets `render_block`'s centered content column (`px_4`, at the test window's
    // default zoom `rem` of `BASE_REM_SIZE`) and the active block's raw card
    // (`crate::render::RAW_INSET`, less its 1 px border) add around the text.
    let rem = gpui::px(16.);
    let raw_inset = crate::render::RAW_INSET - gpui::px(1.);
    let column_left = bounds.left() + (bounds.size.width - content_width) / 2.;
    let text_start_x = column_left + rem + raw_inset;
    assert!(
        text_start_x - bounds.left() > gpui::px(50.),
        "the window is wide enough to actually center the column: {text_start_x:?}"
    );

    let point = gpui::point(text_start_x - gpui::px(2.), bounds.top() + gpui::px(5.));
    cx.simulate_click(point, gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(selection(&editor, cx), 0..0, "just left of the text still lands at its start");
}

#[gpui::test]
fn a_margin_click_with_nothing_rendered_yet_does_not_move_the_caret(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    editor.update(cx, |e, cx| e.move_to(find("gamma"), false, cx));
    cx.run_until_parked();
    let before = selection(&editor, cx);

    // Simulates a click before any frame has ever completed: nothing to click relative to, and
    // in particular no reason to assume the end of the document.
    editor.update(cx, |e, _| {
        e.window_item_bounds.clear();
        e.rendered = 0..0;
    });
    cx.simulate_click(gpui::point(gpui::px(2.), gpui::px(400.)), gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(selection(&editor, cx), before, "nothing known: the click is ignored");
}

#[gpui::test]
fn escape_leaves_edit_mode_and_a_typed_key_resumes_it_at_the_kept_position(
    cx: &mut TestAppContext,
) {
    let doc = "one\n\ntwo\n";
    let (editor, cx) = open(doc, cx);
    let caret = doc.find("two").expect("fixture");
    editor.update(cx, |e, cx| e.move_to(caret, false, cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.active_block()),
        None,
        "every block renders while not editing"
    );
    assert_eq!(selection(&editor, cx), caret..caret, "the caret keeps its offset");

    cx.simulate_input("X");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "one\n\nXtwo\n", "typed at the kept position");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1), "reactivated");
}

#[gpui::test]
fn escape_with_the_find_bar_open_closes_only_the_bar(cx: &mut TestAppContext) {
    let (editor, cx) = open("one\n\ntwo\n", cx);
    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.find.is_some()));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.find.is_none()), "the bar closed");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(0), "still editing");
}

#[gpui::test]
fn arrow_key_after_escape_moves_from_the_kept_position(cx: &mut TestAppContext) {
    let doc = "one two\n";
    let (editor, cx) = open(doc, cx);
    editor.update(cx, |e, cx| e.move_to(3, false, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), None);

    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), 4..4, "moved right from the kept offset");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(0), "resumed editing");
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

#[gpui::test]
fn the_document_caret_is_hidden_while_the_find_bar_is_open(cx: &mut TestAppContext) {
    let (editor, cx) = open("hello world\n", cx);
    assert!(editor.read_with(cx, |e, _| e.caret_painted), "caret painted while editing");

    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    assert!(
        !editor.read_with(cx, |e, _| e.caret_painted),
        "typing goes to the bar, not the document: only its own caret should show"
    );

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.caret_painted), "shown again once the bar closes");
}

#[gpui::test]
fn a_match_is_revealed_below_the_find_bar_in_a_small_window(cx: &mut TestAppContext) {
    // A match on the document's very first line: the list cannot scroll further up to reveal it
    // (item 0 is already at the top), so this exercises the extra top padding block 0 gets while
    // the bar is open (see `render_block`), not `reveal_caret_at`'s scroll-based inset.
    {
        let doc = format!("target\n\n{}", "filler line\n\n".repeat(60));
        let (editor, cx) = open(&doc, cx);
        cx.simulate_resize(gpui::size(gpui::px(480.), gpui::px(360.)));
        cx.run_until_parked();

        cx.simulate_keystrokes("secondary-f");
        cx.simulate_input("target");
        cx.run_until_parked();

        let (matched_y, bar_bottom) = editor.read_with(cx, |e, _| {
            let y = e.position_for_offset(0).expect("the match's block is laid out").y;
            (y, e.find_bar_bottom)
        });
        assert!(
            matched_y >= bar_bottom,
            "top-of-document match at y {matched_y:?} is under the find bar (bottom {bar_bottom:?})"
        );
    }

    // A match well after the first line: there is real content above it to scroll through, so
    // this exercises `reveal_caret_at`'s find-bar inset instead of the extra top padding.
    {
        let doc = format!("{}needle\n", "filler line\n\n".repeat(60));
        let needle = doc.find("needle").expect("fixture");
        let (editor, cx) = open(&doc, cx);
        cx.simulate_resize(gpui::size(gpui::px(480.), gpui::px(360.)));
        cx.run_until_parked();

        cx.simulate_keystrokes("secondary-f");
        cx.simulate_input("needle");
        cx.run_until_parked();

        let (matched_y, bar_bottom) = editor.read_with(cx, |e, _| {
            let y = e.position_for_offset(needle).expect("the match's block is laid out").y;
            (y, e.find_bar_bottom)
        });
        assert!(
            matched_y >= bar_bottom,
            "deep match at y {matched_y:?} is hidden under the find bar (bottom {bar_bottom:?})"
        );
    }
}

#[gpui::test]
fn replace_one_then_all_with_one_undo_step_each(cx: &mut TestAppContext) {
    let doc = "cat and cat\n\nthe cat sat\n";
    let (editor, cx) = open(doc, cx);
    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("cat");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("dog");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc, "typing fills the fields, not the document");

    // Enter in the replacement replaces the selected match and selects the next one.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "dog and cat\n\nthe cat sat\n");
    let selected = editor.read_with(cx, |e, _| e.selection.clone());
    assert_eq!(selected, 8..11);

    cx.simulate_keystrokes("secondary-enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "dog and dog\n\nthe dog sat\n");
    let status = editor.read_with(cx, |e, _| e.find.as_ref().map(|f| f.status()));
    assert_eq!(status.as_deref(), Some("Replaced 2"));

    cx.simulate_keystrokes("escape secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "dog and cat\n\nthe cat sat\n", "replace all undoes at once");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc);
}

#[gpui::test]
fn replace_all_replaces_every_match_past_the_display_cap_in_one_undo_step(cx: &mut TestAppContext) {
    // Comfortably over `MAX_FIND_MATCHES` (10,000) and over `FIND_BACKGROUND_THRESHOLD` (5 MiB),
    // so this exercises both the display cap and the background-scan path: the bug this is a
    // regression test for silently replaced only the first 10,000 matches of a much larger set,
    // with no notice at all.
    let line = "127.0.0.1 - - [10/Oct] \"GET /x\" status=200 1234\n";
    let count = 150_000;
    let doc = line.repeat(count);
    assert!(
        doc.len() as u64 >= tachyon_doc::FIND_BACKGROUND_THRESHOLD,
        "fixture must exceed the background threshold"
    );
    let (editor, cx) = open(&doc, cx);
    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("status=200");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("status=500");
    cx.run_until_parked();

    // Dispatched directly (not `simulate_keystrokes`, which runs until parked and so would hide
    // whether the scan itself is synchronous): the frame right after Ctrl+Enter must show
    // "replacing…", not have already blocked on the whole scan.
    let window = cx.window_handle();
    cx.dispatch_keystroke(window, gpui::Keystroke::parse("secondary-enter").unwrap());
    let status = editor.read_with(cx, |e, _| e.find.as_ref().map(|f| f.status()));
    assert_eq!(status.as_deref(), Some("replacing…"), "the scan runs off the UI thread first");
    cx.run_until_parked();

    let (replaced_text, status) =
        editor.read_with(cx, |e, _| (e.text(), e.find.as_ref().map(|f| f.status())));
    assert!(!replaced_text.contains("status=200"), "every match was replaced, not just 10,000");
    assert_eq!(replaced_text.matches("status=500").count(), count);
    assert_eq!(status.as_deref(), Some("Replaced 150,000"));

    cx.simulate_keystrokes("escape secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc, "replace all undoes at once, restoring every match");
}

#[gpui::test]
fn replace_all_past_the_display_cap_works_in_plain_mode(cx: &mut TestAppContext) {
    let line = "status=200 filler filler filler words to pad the line out a bit\n";
    let count = 150_000;
    let doc = line.repeat(count);
    assert!(
        doc.len() as u64 >= tachyon_doc::FIND_BACKGROUND_THRESHOLD,
        "fixture must exceed the background threshold"
    );
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.set_document(tachyon_doc::Document::new_plain(&doc), cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);

    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("status=200");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("status=500");
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-enter");
    cx.run_until_parked();

    let (replaced_text, status) =
        editor.read_with(cx, |e, _| (e.text(), e.find.as_ref().map(|f| f.status())));
    assert!(!replaced_text.contains("status=200"));
    assert_eq!(replaced_text.matches("status=500").count(), count);
    assert_eq!(status.as_deref(), Some("Replaced 150,000"));

    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc, "one undo restores every match, in plain mode too");
}

#[gpui::test]
fn a_typed_edit_during_a_background_replace_all_keeps_its_place(cx: &mut TestAppContext) {
    let line = "status=200 filler filler filler words to pad the line out a bit\n";
    let count = 150_000;
    let doc = line.repeat(count);
    assert!(
        doc.len() as u64 >= tachyon_doc::FIND_BACKGROUND_THRESHOLD,
        "fixture must exceed the background threshold"
    );
    let (editor, cx) = open(&doc, cx);
    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("status=200");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("status=500");
    cx.run_until_parked();

    // Starts the background scan, then - before it lands - closes the bar (typing while it is
    // open would edit the query/replacement fields, not the document) and types at the very
    // start of the document: a position the scan already committed to as its span's start,
    // so applying its stale result unchanged would either lose this edit or place it on the
    // wrong side of the (now shifted) first match.
    let window = cx.window_handle();
    cx.dispatch_keystroke(window, gpui::Keystroke::parse("secondary-enter").unwrap());
    assert_eq!(
        editor.read_with(cx, |e, _| e.find.as_ref().map(|f| f.status())).as_deref(),
        Some("replacing…")
    );

    // `dispatch_keystroke` (not `simulate_keystrokes`/`simulate_input`, which both run until
    // parked and so would let the background scan land before "X" is even typed): the typed
    // edit must land while the scan is genuinely still in flight.
    cx.dispatch_keystroke(window, gpui::Keystroke::parse("escape").unwrap());
    editor.update(cx, |e, cx| e.move_to(0, false, cx));
    cx.dispatch_keystroke(window, gpui::Keystroke::parse("X").unwrap());
    cx.run_until_parked();

    let final_text = editor.read_with(cx, |e, _| e.text());
    assert!(
        final_text.starts_with("Xstatus=500"),
        "the typed edit kept its place ahead of the first replacement"
    );
    assert!(!final_text.contains("status=200"), "Replace All reran against the post-edit text");
    assert_eq!(final_text.matches("status=500").count(), count);

    // One undo removes only Replace All's own edit, leaving the typed "X" - the two were never
    // merged into a single step, and neither was lost or reordered relative to the other.
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.text()), format!("X{doc}"));
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.text()), doc);
}

#[gpui::test]
fn appearance_hint_wins_until_the_window_catches_up(cx: &mut TestAppContext) {
    // The test window reports light, like GPUI's Linux windows before the portal answers.
    cx.update(|cx| cx.set_global(crate::AppearanceHint { dark: true }));
    let (editor, cx) = open("# Title\n", cx);
    let dark = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.theme.dark);
    assert!(dark(cx), "the first frame uses the hinted dark theme");

    // Decoration changes also fire appearance callbacks, still reporting light.
    editor.update_in(cx, |e, window, cx| e.follow_appearance(window, cx));
    assert!(dark(cx), "a stale light report does not flash the light theme");
    assert!(cx.update(|_, cx| cx.has_global::<crate::AppearanceHint>()));
}

#[gpui::test]
fn without_a_hint_the_window_appearance_decides(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n", cx);
    assert!(!editor.read_with(cx, |e, _| e.theme.dark), "the test window reports light");
}

#[gpui::test]
fn zoom_steps_scale_layout_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let (editor, cx) = open(THREE_PARAGRAPHS, cx);
    let state = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            let line = e.active_layout.as_ref().expect("raw block is laid out").0.line_height();
            (e.theme.zoom, f32::from(line))
        })
    };
    let (zoom, line) = state(cx);
    assert_eq!(zoom, 1.);

    cx.simulate_keystrokes("secondary-= secondary-=");
    let (zoom, zoomed_line) = state(cx);
    assert_eq!(zoom, 1.25);
    assert!(
        (zoomed_line / line - 1.25).abs() < 0.02,
        "lines are laid out larger: {line} -> {zoomed_line}"
    );

    cx.simulate_keystrokes("secondary-- secondary-- secondary--");
    assert_eq!(state(cx).0, 0.9);
    cx.simulate_keystrokes("secondary-0");
    assert_eq!(state(cx), (1., line), "reset restores the layout");

    for _ in 0..20 {
        cx.simulate_keystrokes("secondary--");
    }
    assert_eq!(state(cx).0, 0.5);
    for _ in 0..20 {
        cx.simulate_keystrokes("secondary-=");
    }
    assert_eq!(state(cx).0, 3.);
    assert_eq!(text(&editor, cx), THREE_PARAGRAPHS, "zoom keys do not type");
}

#[gpui::test]
fn secondary_click_on_a_link_opens_it_without_moving_the_caret(cx: &mut TestAppContext) {
    let doc = "[site](https://example.com) and more\n\nplain\n";
    let (editor, cx) = open(doc, cx);
    // Offset 0 is the hidden `[`: its raw position is where the rendered link text starts.
    let link = rendered_glyph_point(&editor, 0, cx);
    let plain = doc.find("plain").expect("fixture");
    editor.update(cx, |e, cx| e.move_to(plain, false, cx));
    cx.run_until_parked();

    cx.simulate_click(link, gpui::Modifiers::secondary_key());
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    assert_eq!(selection(&editor, cx), plain..plain, "the caret stays");

    cx.simulate_click(link, gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(0), "a plain click edits");
}

#[gpui::test]
fn secondary_click_on_a_bare_url_opens_it(cx: &mut TestAppContext) {
    let doc = "https://example.com/docs, see above\n\nplain\n";
    let (editor, cx) = open(doc, cx);
    let url = rendered_glyph_point(&editor, 3, cx);
    let plain = doc.find("plain").expect("fixture");
    editor.update(cx, |e, cx| e.move_to(plain, false, cx));
    cx.run_until_parked();

    cx.simulate_click(url, gpui::Modifiers::secondary_key());
    cx.run_until_parked();
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
}

thread_local! {
    /// Clipboard text for the off-thread reader tests, standing in for the system clipboard.
    /// Per thread: tests run in parallel, and GPUI's test executor runs "background" work on the
    /// test's thread.
    static READER_TEXT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// How many times `read_reader_text` ran; reset by `open_with_reader`. Proves a paste reads
    /// the clipboard exactly once, even when both the UI thread's flush and the background task
    /// try to (the crash this fixes was two threads reading the clipboard for one paste).
    static READER_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn read_reader_text() -> Option<String> {
    READER_CALLS.set(READER_CALLS.get() + 1);
    READER_TEXT.with_borrow(Clone::clone)
}

fn open_with_reader<'a>(
    text: &str,
    clipboard: Option<String>,
    cx: &'a mut TestAppContext,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
    READER_TEXT.set(clipboard);
    READER_CALLS.set(0);
    cx.update(|cx| cx.set_global(crate::ClipboardReader(read_reader_text)));
    open(text, cx)
}

#[gpui::test]
fn a_paste_read_off_thread_lands_after_the_read(cx: &mut TestAppContext) {
    let (editor, cx) = open_with_reader("start\n", Some("pasted".into()), cx);
    cx.simulate_keystrokes("ctrl-end");
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    assert_eq!(text(&editor, cx), "start\n", "the clipboard is read in the background");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "start\npasted");
}

#[gpui::test]
fn keys_typed_while_the_clipboard_is_read_land_after_the_paste(cx: &mut TestAppContext) {
    let big = "Pasted paragraph.\n\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 10);
    for paste in ["small".to_owned(), big] {
        let (editor, cx) = open_with_reader("start\n", Some(paste.clone()), cx);
        cx.simulate_keystrokes("ctrl-end");
        editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
        cx.simulate_input("x");
        cx.run_until_parked();
        assert_eq!(text(&editor, cx), format!("start\n{paste}x"));
    }
}

#[gpui::test]
fn flushing_a_paste_before_the_background_read_starts_reads_the_clipboard_once(
    cx: &mut TestAppContext,
) {
    // Regression test: before the clipboard-read claim, a key typed right after Ctrl+V could
    // make `flush_pending_paste` read the clipboard on the UI thread while the background task
    // (here, not even polled yet) went on to read it too, racing two threads inside the
    // clipboard for the same paste (the Windows crash dumps that followed #47).
    let big = "Pasted paragraph.\n\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 10);
    for paste in ["small".to_owned(), big] {
        let (editor, cx) = open_with_reader("start\n", Some(paste.clone()), cx);
        cx.simulate_keystrokes("ctrl-end");
        editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
        // The background task has only been scheduled, not polled: flushing must claim the read
        // on this thread rather than wait on a background read that has not started.
        cx.simulate_input("x");
        cx.run_until_parked();
        assert_eq!(text(&editor, cx), format!("start\n{paste}x"), "the key lands after the paste");
        assert_eq!(READER_CALLS.get(), 1, "the clipboard is read exactly once");
    }
}

#[gpui::test]
fn a_large_paste_read_off_thread_is_prepared_in_the_background(cx: &mut TestAppContext) {
    let paste = "Pasted paragraph.\n\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 10);
    let (editor, cx) = open_with_reader("", Some(paste.clone()), cx);
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), paste);
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "", "one undo step");
}

#[gpui::test]
fn an_empty_clipboard_read_off_thread_pastes_nothing(cx: &mut TestAppContext) {
    let (editor, cx) = open_with_reader("start\n", None, cx);
    cx.simulate_keystrokes("ctrl-end secondary-v");
    cx.run_until_parked();
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "start\nx");
}

#[gpui::test]
fn queued_text_lands_even_when_the_background_read_is_empty(cx: &mut TestAppContext) {
    let (editor, cx) = open_with_reader("start\n", None, cx);
    cx.simulate_keystrokes("ctrl-end");
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    // Queued while the background read (of an empty clipboard) is still pending.
    editor.update_in(cx, |e, window, cx| e.replace_text_in_range(None, "x", window, cx));
    assert_eq!(text(&editor, cx), "start\n", "queued, not yet applied");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "start\nx", "the queued key lands although the paste was empty");
}

#[gpui::test]
fn enter_continues_lists_and_ends_them_on_an_empty_item(cx: &mut TestAppContext) {
    for (doc, typed, expected) in [
        ("- one", "enter t w o", "- one\n- two"),
        ("9. nine", "enter t", "9. nine\n10. t"),
        ("> - [x] done", "enter n", "> - [x] done\n> - [ ] n"),
        // Enter on the new, empty item ends the list.
        ("- one", "enter enter t", "- one\n\nt"),
        ("> - q", "enter enter t", "> - q\n>\n> t"),
    ] {
        let (editor, cx) = open(doc, cx);
        cx.simulate_keystrokes("ctrl-end");
        cx.simulate_keystrokes(typed);
        cx.run_until_parked();
        assert_eq!(text(&editor, cx), expected, "{doc:?} + {typed:?}");
    }
}

#[gpui::test]
fn enter_splits_an_item_and_stays_plain_outside_lists(cx: &mut TestAppContext) {
    let (editor, cx) = open("- abcd", cx);
    editor.update(cx, |e, cx| e.move_to(4, false, cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- ab\n- cd");

    let code = "```\n- not a list\n```\n";
    let (editor, cx) = open(code, cx);
    let end = code.find("list").expect("fixture") + 4;
    editor.update(cx, |e, cx| e.move_to(end, false, cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "```\n- not a list\n\n```\n");
}

#[gpui::test]
fn tab_indents_list_items_and_shift_tab_outdents(cx: &mut TestAppContext) {
    let doc = "- a\n- b\n- c\n\ntext";
    let (editor, cx) = open(doc, cx);
    // Select from inside "b" to inside "c".
    editor.update(cx, |e, cx| {
        e.move_to(6, false, cx);
        e.move_to(10, true, cx);
    });
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n  - c\n\ntext");
    assert_eq!(selection(&editor, cx), 8..14, "the selection moves with the text");

    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc);
    assert_eq!(selection(&editor, cx), 6..10);

    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n  - c\n\ntext", "each is one undo step");

    // Outside a list, Tab still inserts spaces and Shift+Tab does nothing.
    let end = doc.len();
    editor.update(cx, |e, cx| e.move_to(end + 4, false, cx));
    cx.simulate_keystrokes("shift-tab tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n  - c\n\ntext    ");
}

#[gpui::test]
fn nesting_renumbers_ordered_items_and_empty_nested_items_move_up(cx: &mut TestAppContext) {
    let (editor, cx) = open("1. a\n2. b\n3. c", cx);
    editor.update(cx, |e, cx| e.move_to(8, false, cx));
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "1. a\n   1. b\n3. c", "a nested list starts at 1");
    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "1. a\n2. b\n3. c", "back in the parent list: 2");

    // The first item has nothing to nest under: Tab leaves it alone.
    editor.update(cx, |e, cx| e.move_to(3, false, cx));
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "1. a\n2. b\n3. c");

    let (editor, cx) = open("- a\n  - b", cx);
    cx.simulate_keystrokes("ctrl-end enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n  - ");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n- ", "Enter on an empty nested item moves it up");
}

#[gpui::test]
fn tab_leaves_lines_the_selection_only_touches_or_that_are_code(cx: &mut TestAppContext) {
    let doc = "- a\n- b\n- c\n";
    let (editor, cx) = open(doc, cx);
    // From inside "b" to the start of "c": "c" is not selected.
    editor.update(cx, |e, cx| {
        e.move_to(6, false, cx);
        e.move_to(8, true, cx);
    });
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n- c\n");

    let doc = "- a\n- b\n\n```\n- code\n```\n";
    let (editor, cx) = open(doc, cx);
    let end = doc.find("code").expect("fixture");
    editor.update(cx, |e, cx| {
        e.move_to(6, false, cx);
        e.move_to(end, true, cx);
    });
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "- a\n  - b\n\n```\n- code\n```\n", "the fenced line stays");
}

#[gpui::test]
fn go_to_heading_filters_chooses_and_jumps(cx: &mut TestAppContext) {
    let doc = "# Intro\n\ntext\n\n## Setup\n\nmore\n\n## Usage\n\n### Setup again\n\nend\n";
    let (editor, cx) = open(doc, cx);
    let titles = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            let picker = e.picker.as_ref().expect("the list is open");
            picker.matches.iter().map(|&i| picker.items[i].label.clone()).collect::<Vec<_>>()
        })
    };

    cx.simulate_keystrokes("secondary-shift-o");
    assert_eq!(titles(cx), ["Intro", "Setup", "Usage", "Setup again"]);
    cx.simulate_input("setup");
    assert_eq!(titles(cx), ["Setup", "Setup again"], "case-insensitive filter");
    assert_eq!(text(&editor, cx), doc, "typing goes to the filter");

    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    let target = doc.find("Setup again").expect("fixture");
    assert_eq!(selection(&editor, cx), target..target);
    assert!(editor.read_with(cx, |e, _| e.picker.is_none()), "jumping closes the list");

    // Escape closes without moving; Up wraps to the last heading.
    cx.simulate_keystrokes("secondary-shift-o up escape");
    assert_eq!(selection(&editor, cx), target..target);
    assert!(editor.read_with(cx, |e, _| e.picker.is_none()));
}

/// A fresh backup directory for a test, removed first if an earlier run left it.
fn backup_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tachyon-backups-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn backups_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut texts: Vec<String> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .map(|p| std::fs::read_to_string(p).expect("readable backup"))
        .collect();
    texts.sort();
    texts
}

#[gpui::test]
fn unsaved_text_is_backed_up_after_a_pause_and_dropped_once_saved(cx: &mut TestAppContext) {
    let dir = backup_dir("pause");
    let file = dir.with_extension("md");
    cx.update(|cx| cx.set_global(crate::Backups::new(dir.clone())));
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.set_file(file.clone(), cx));

    cx.simulate_input("hello");
    cx.run_until_parked();
    assert!(backups_in(&dir).is_empty(), "not while typing");
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(backups_in(&dir), ["hello"]);

    cx.simulate_input(" world");
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(backups_in(&dir), ["hello world"], "one backup per document, kept current");

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).expect("saved"), "hello world");
    assert!(backups_in(&dir).is_empty(), "nothing unsaved, nothing backed up");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&file);
}

#[gpui::test]
fn quit_closes_without_asking_and_the_backup_restores(cx: &mut TestAppContext) {
    let dir = backup_dir("quit");
    cx.update(|cx| {
        cx.set_global(crate::Backups::new(dir.clone()));
        cx.set_global(crate::HotExit);
    });
    let (_editor, vcx) = open("", cx);
    vcx.simulate_input("draft");
    vcx.dispatch_action(crate::CloseWindow);
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt(), "no Save prompt during Quit");
    assert!(vcx.windows().is_empty(), "the window closed");
    assert_eq!(backups_in(&dir), ["draft"], "written right away, not after a pause");

    // The next start: the backup comes back as an unsaved document in the same slot.
    let restored = cx.update(|cx| cx.global::<crate::Backups>().restore());
    assert_eq!(restored.len(), 1);
    cx.update(|cx| cx.remove_global::<crate::HotExit>());
    let (editor, vcx) = open("", cx);
    let restored = restored.into_iter().next().expect("one backup");
    editor.update(vcx, |e, cx| e.adopt_backup(restored, cx));
    assert_eq!(text(&editor, vcx), "draft");
    assert!(editor.read_with(vcx, |e, _| e.is_modified()));

    // Closing it without saving (not Quit) asks, and "Don't Save" drops the backup.
    vcx.dispatch_action(crate::CloseWindow);
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Don't Save");
    vcx.run_until_parked();
    assert!(backups_in(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

thread_local! {
    /// What the fake rich-text clipboard received: (html, text).
    static RICH: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

#[gpui::test]
fn copy_as_html_puts_rich_text_or_falls_back_to_the_source(cx: &mut TestAppContext) {
    let (editor, vcx) = open("# Title\n\nSome **bold** text\n", cx);
    // Without rich-text support: the HTML source, for the whole document when nothing is selected.
    vcx.simulate_keystrokes("secondary-shift-c");
    let copied = vcx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("<h1>Title</h1>\n<p>Some <strong>bold</strong> text</p>\n"));

    // With it: HTML plus the selected Markdown as plain text.
    vcx.update(|_, cx| {
        cx.set_global(crate::HtmlClipboard(|_, html, text| {
            RICH.set(Some((html.to_owned(), text.to_owned())));
            true
        }));
    });
    let start = "# Title\n\n".len();
    editor.update(vcx, |e, cx| {
        e.move_to(start, false, cx);
        e.move_to(start + "Some **bold**".len(), true, cx);
    });
    vcx.simulate_keystrokes("secondary-shift-c");
    let rich = RICH.with_borrow(Clone::clone);
    assert_eq!(
        rich,
        Some(("<p>Some <strong>bold</strong></p>\n".to_owned(), "Some **bold**".to_owned()))
    );
}

/// An editor showing `path` as loaded from disk.
fn open_file<'a>(
    path: &std::path::Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
    let (editor, cx) = open("", cx);
    let loaded = tachyon_doc::Document::new(&std::fs::read_to_string(path).expect("readable"));
    editor.update(cx, |e, cx| {
        e.set_document(loaded, cx);
        e.set_file(path.to_owned(), cx);
    });
    (editor, cx)
}

fn refocus(cx: &mut VisualTestContext) {
    cx.deactivate_window();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
}

#[gpui::test]
fn a_file_changed_on_disk_reloads_when_the_window_is_activated(cx: &mut TestAppContext) {
    let dir = backup_dir("reload");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("note.md");
    std::fs::write(&path, "first version\n").expect("write");
    let (editor, cx) = open_file(&path, cx);

    std::fs::write(&path, "second, longer version\n").expect("write");
    refocus(cx);
    assert_eq!(text(&editor, cx), "second, longer version\n");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn saving_over_a_file_changed_on_disk_asks_first(cx: &mut TestAppContext) {
    let dir = backup_dir("conflict");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("note.md");
    std::fs::write(&path, "original\n").expect("write");
    let (editor, cx) = open_file(&path, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("mine");

    std::fs::write(&path, "theirs, from another program\n").expect("write");
    refocus(cx);
    assert_eq!(text(&editor, cx), "original\nmine", "unsaved changes are kept");

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "asks before overwriting");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "theirs, from another program\n");

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    cx.simulate_prompt_answer("Overwrite");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "original\nmine");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));

    // Saved now: the next save needs no confirmation.
    cx.simulate_input("!");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "original\nmine!");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_checked_write_keeps_the_old_file_when_the_check_fails() {
    let dir = backup_dir("checked-write");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("f.md");
    std::fs::write(&path, "theirs").expect("write");
    let written =
        crate::editor::write_atomically_if(&path, b"mine", || false).expect("no io error");
    assert!(!written);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "theirs");
    assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1, "no temporary file left");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn a_restored_file_without_a_recorded_version_asks_before_saving(cx: &mut TestAppContext) {
    let dir = backup_dir("unknown-version");
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).expect("temp dir");
    let file = dir.join("note.md");
    std::fs::write(&file, "changed while Tachyon was closed\n").expect("write");
    // A backup in the format that predates recorded versions: just the path.
    std::fs::write(backups.join("1.md"), "my unsaved text\n").expect("write");
    std::fs::write(backups.join("1.path"), file.to_str().expect("utf-8")).expect("write");

    let restored = crate::Backups::new(backups).restore().pop().expect("one backup");
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.adopt_backup(restored, cx));
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "the file may have changed: ask first");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).expect("read"), "changed while Tachyon was closed\n");
    let _ = std::fs::remove_dir_all(&dir);
}

thread_local! {
    /// Paths the fake `OpenPaths` was asked to open.
    static OPENED: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[gpui::test]
fn open_recent_lists_opened_files_newest_first_and_opens_the_pick(cx: &mut TestAppContext) {
    let dir = backup_dir("recent");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (a, b, c) = (dir.join("alpha.md"), dir.join("beta.md"), dir.join("gamma.md"));
    for file in [&a, &b, &c] {
        std::fs::write(file, "text\n").expect("write");
    }
    cx.update(|cx| {
        cx.set_global(crate::RecentFiles::new(dir.join("recent.txt")));
        cx.set_global(crate::OpenPaths(std::rc::Rc::new(|paths, _| OPENED.set(paths))));
    });
    let (editor, cx) = open("", cx);
    for file in [&a, &b, &a, &c] {
        editor.update(cx, |e, cx| e.set_file(file.clone(), cx));
        cx.run_until_parked();
    }

    cx.simulate_keystrokes("secondary-r");
    let labels = editor.read_with(cx, |e, _| {
        let picker = e.picker.as_ref().expect("open");
        picker.matches.iter().map(|&i| picker.items[i].label.clone()).collect::<Vec<_>>()
    });
    assert_eq!(labels, ["alpha.md", "beta.md"], "newest first, without duplicates or this file");

    cx.simulate_input("bet");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(OPENED.with_borrow(Clone::clone), [b]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn settings_choose_theme_and_zoom_and_saving_them_applies_at_once(cx: &mut TestAppContext) {
    let dir = backup_dir("settings");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::Dark,
            zoom: 1.25,
            hot_exit: true,
        });
        cx.set_global(crate::SettingsFile(file.clone()));
    });
    // The test window reports a light system appearance.
    let (editor, cx) = open_file(&file, cx);
    let theme =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| (e.theme.dark, e.theme.zoom));
    assert_eq!(theme(cx), (true, 1.25), "the settings win over the system appearance");

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("theme = \"light\"\n");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(
        theme(cx),
        (false, 1.25),
        "saved settings apply to open windows; zoom is for new ones"
    );
    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert_eq!(settings.theme, crate::ThemeChoice::Light);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn local_images_are_found_per_line_and_remote_ones_are_not_loaded(cx: &mut TestAppContext) {
    let doc = "![diagram](img/d.png)\n\nSee ![remote](https://x.dev/r.png) here.\n";
    let (editor, cx) = open(doc, cx);
    let dir = std::env::temp_dir().join("tachyon-images");
    editor.update(cx, |e, cx| e.set_file(dir.join("doc.md"), cx));
    let images = editor.read_with(cx, |e, _| {
        e.document()
            .blocks()
            .iter()
            .enumerate()
            .map(|(i, block)| {
                let ir = &block.parsed().ir;
                let start = e.document().block_range(i).start;
                e.line_images(ir, &(0..ir.text.len()), start)
                    .into_iter()
                    .map(|(path, offset, visible)| (path, offset, visible, ir.text.len()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    });
    // The first paragraph is just the image: its whole visible text is the alt text, and a click
    // puts the caret at the alt text's source (after `![`).
    assert_eq!(images[0], [(dir.join("img/d.png"), 2, 0..7, 7)]);
    assert!(images[1].is_empty(), "remote images are not loaded");
}

#[gpui::test]
fn the_current_find_match_keeps_its_fill_and_a_thicker_underline(cx: &mut TestAppContext) {
    let doc = "one fish, two fish\n";
    let (editor, cx) = open(doc, cx);
    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("fish");
    let (marks, editing) = editor.read_with(cx, |e, _| (e.marks(&(0..doc.len())), e.theme.editing));
    let style = |range: std::ops::Range<usize>| {
        let (_, style) = marks.iter().find(|(r, _)| *r == range).expect("a mark over the range");
        (style.background_color, style.underline.map(|u| f32::from(u.thickness)))
    };
    // The current match is also the selection: no selection mark hides its fill.
    assert_eq!(marks.len(), 2);
    assert_eq!(style(4..8), (Some(editing.find_current), Some(2.)));
    assert_eq!(style(14..18), (Some(editing.find_match), Some(1.)));
}

#[gpui::test]
fn prompts_use_the_theme_the_settings_choose(cx: &mut TestAppContext) {
    // The test window reports a light system appearance.
    cx.update(|cx| {
        cx.set_global(crate::Settings { theme: crate::ThemeChoice::Dark, zoom: 1., hot_exit: true })
    });
    let (_editor, cx) = open("", cx);
    let dark = cx.update(|window, cx| crate::Theme::for_window(window, cx).dark);
    assert!(dark);
}

#[gpui::test]
fn a_log_file_opens_as_plain_text_and_hashes_stay_literal(cx: &mut TestAppContext) {
    let dir = backup_dir("log-ext");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("app.log");
    std::fs::write(&path, "# not a heading\nplain text\n").expect("write");
    let (editor, cx) = open("", cx);
    let crate::LoadOutcome::Loaded(loaded) = crate::load_document(&path).expect("readable") else {
        panic!("not refused")
    };
    editor.update(cx, |e, cx| {
        e.set_loaded(*loaded, cx);
        e.set_file(path.clone(), cx);
    });
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);
    assert_eq!(text(&editor, cx), "# not a heading\nplain text\n");
    assert_eq!(
        kinds(&editor, cx),
        vec![BlockKind::Plain],
        "'#' is literal, not parsed as a heading"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn ctrl_shift_m_toggles_text_mode_and_keeps_text_and_undo(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\npara\n", cx);
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Markdown);
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Heading(1), BlockKind::Paragraph]);

    cx.simulate_keystrokes("ctrl-shift-m");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);
    assert_eq!(text(&editor, cx), "# Title\n\npara\n", "the toggle keeps the text as is");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Plain], "'#' is literal in plain mode");

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("!");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "# Title\n\npara\n!");

    cx.simulate_keystrokes("ctrl-shift-m");
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Markdown);
    assert_eq!(text(&editor, cx), "# Title\n\npara\n!", "the typed text survived both toggles");

    // The same buffer and undo log carried across both toggles (retagging, not reloading):
    // undoes the typed "!", not either toggle (a toggle is not itself an undo step).
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "# Title\n\npara\n");
    assert_eq!(
        kinds(&editor, cx),
        vec![BlockKind::Heading(1), BlockKind::Paragraph],
        "back to Markdown parsing"
    );
}

#[gpui::test]
fn toggle_to_markdown_is_refused_above_the_size_limit_with_a_notice(cx: &mut TestAppContext) {
    let big = "x".repeat(tachyon_doc::MARKDOWN_SIZE_LIMIT as usize + 1024);
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.set_document(tachyon_doc::Document::new_plain(&big), cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);

    cx.simulate_keystrokes("ctrl-shift-m");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.document().mode()),
        tachyon_doc::DocMode::Plain,
        "refused: too large to parse as Markdown"
    );
    let notice = editor.read_with(cx, |e, _| e.notice.clone());
    assert_eq!(notice, Some(crate::oversized_markdown_notice()));
}

#[gpui::test]
fn lossy_load_then_save_shows_the_prompt_and_cancel_leaves_the_file_untouched(
    cx: &mut TestAppContext,
) {
    let dir = backup_dir("lossy");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("note.log");
    // A lone 0xFF is invalid UTF-8 on its own (never a valid lead byte): `Buffer::load`
    // replaces it with one U+FFFD and reports `lossy`.
    std::fs::write(&path, b"before\xff after\n").expect("write");
    let (editor, cx) = open("", cx);
    let crate::LoadOutcome::Loaded(loaded) = crate::load_document(&path).expect("readable") else {
        panic!("not refused")
    };
    editor.update(cx, |e, cx| {
        e.set_loaded(*loaded, cx);
        e.set_file(path.clone(), cx);
    });
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.lossy), "decoded lossily");

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "asks before overwriting a lossily-decoded file");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(&path).expect("read"),
        b"before\xff after\n",
        "cancel leaves the file untouched"
    );
    assert!(editor.read_with(cx, |e, _| e.lossy), "nothing was saved: still flagged lossy");
    assert!(
        !editor.read_with(cx, |e, _| e.is_modified()),
        "no edit was made and the file was not written: canceling a lossy-save prompt must not \
         itself mark the document modified"
    );

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Save Anyway");
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&path).expect("valid utf-8 now");
    assert_eq!(saved, "before\u{FFFD} after\n");
    assert!(!editor.read_with(cx, |e, _| e.lossy), "the round trip is lossless from here on");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn caret_crosses_a_forced_long_line_cut(cx: &mut TestAppContext) {
    let cut = tachyon_doc::PLAIN_FORCED_CUT_BYTES;
    // One line with no `\n` spanning more than three forced display-only cuts, then a real
    // second line: the shape of the reported 20 MB single-line pathology, scaled down.
    let giant = "x".repeat(cut * 3 + 500);
    let doc = format!("{giant}\nEND\n");
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.set_document(tachyon_doc::Document::new_plain(&doc), cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);

    // Right/Left step by grapheme directly on the rope: chunk-agnostic, so the forced cut never
    // affects them. Checked as the baseline the Up/Down fix below must not regress.
    editor.update(cx, |e, cx| e.move_to(cut - 1, false, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), cut..cut);
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), (cut - 1)..(cut - 1));

    // Down from the very last byte of the first forced-cut chunk. Without crossing the cut
    // correctly, `rope.byte_to_line` sees the whole 25 KB+ line as line 0 throughout and jumps
    // straight to "END", skipping the rest of the line entirely; crossing it correctly lands in
    // the next chunk at the same within-chunk byte offset (here, its last byte again).
    editor.update(cx, |e, cx| e.move_to(cut - 1, false, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(
        selection(&editor, cx),
        (2 * cut - 1)..(2 * cut - 1),
        "crossed into the next chunk of the same line, not past it to \"END\""
    );

    // Up from the first byte of the second chunk symmetrically crosses back into the first.
    editor.update(cx, |e, cx| e.move_to(cut, false, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), 0..0);
}

#[gpui::test]
fn background_find_on_a_huge_document_matches_a_synchronous_scan(cx: &mut TestAppContext) {
    // Well over `FIND_BACKGROUND_THRESHOLD` (5 MiB).
    let filler = "filler word ".repeat(500_000);
    let doc = format!("{filler}needle here\nand needle again\n");
    assert!(
        doc.len() as u64 >= tachyon_doc::FIND_BACKGROUND_THRESHOLD,
        "fixture must exceed the background threshold"
    );
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.set_document(tachyon_doc::Document::new_plain(&doc), cx));
    cx.run_until_parked();

    let expected = editor.read_with(cx, |e, _| e.document().find_all("needle"));
    assert_eq!(expected.len(), 2, "fixture sanity check");

    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("needle");
    cx.run_until_parked();
    let (matches, searching, status) = editor.read_with(cx, |e, _| {
        let find = e.find.as_ref().expect("bar is open");
        (find.matches.clone(), find.searching, find.status())
    });
    assert!(!searching, "the background scan finished");
    assert_eq!(matches, expected);
    // The bar opened with the caret at the document's start (`origin = 0`), so the first match
    // at or after it - "needle here", not "and needle again" - is current, same as a synchronous
    // scan would select (`research`'s `select_first`, independent of which path found it).
    assert_eq!(status, "1/2");
}

#[gpui::test]
fn typing_across_an_oversized_raw_block_stays_correct(cx: &mut TestAppContext) {
    // Well over `RAW_SPLIT_THRESHOLD` so `render_raw` splits this one CodeBlock (no blank lines
    // inside a fence, the shape the memory-fix diagnosis measured) into several stacked
    // segments instead of one `StyledText` for the whole thing.
    let mut lines = String::new();
    while lines.len() <= crate::render::RAW_SPLIT_THRESHOLD * 3 {
        lines.push_str(&format!("line {:04}\n", lines.matches('\n').count()));
    }
    let doc_text = format!("```\n{lines}```\n");
    assert_eq!(
        doc_text.len().saturating_sub(8),
        lines.len(),
        "fixture sanity: only the fence markers surround the counted lines"
    );
    let (editor, cx) = open(&doc_text, cx);
    assert_eq!(kinds(&editor, cx).len(), 1, "one oversized block, not split into several");

    // A caret placed well inside a later segment reads and edits at the right source offset:
    // each segment's own `TextTarget::Raw { base }` must be its own start, not the whole
    // block's, and hit testing/typing must not leak into a neighboring segment.
    let deep = doc_text.find("line 0200").unwrap();
    editor.update(cx, |e, cx| e.move_to(deep, false, cx));
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), deep..deep);
    cx.simulate_input("X");
    cx.run_until_parked();
    assert_eq!(&text(&editor, cx)[deep..deep + 1], "X");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), doc_text, "undo restores the exact source");

    // The very last line of the block, in its own (last) segment.
    let last_line = format!("line {:04}", lines.matches('\n').count() - 1);
    let near_end = doc_text.rfind(&last_line).unwrap();
    editor.update(cx, |e, cx| e.move_to(near_end, false, cx));
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), near_end..near_end);
    cx.simulate_input("Y");
    cx.run_until_parked();
    assert_eq!(&text(&editor, cx)[near_end..near_end + 1], "Y");

    // Home/End still land on the real source line's boundaries, not a segment boundary.
    cx.simulate_keystrokes("secondary-z home");
    cx.run_until_parked();
    let start_of_line = doc_text[..near_end].rfind('\n').map_or(0, |nl| nl + 1);
    assert_eq!(selection(&editor, cx), start_of_line..start_of_line);
}

#[gpui::test]
fn editing_a_large_non_active_list_does_not_use_a_stale_rendered_height_cache(
    cx: &mut TestAppContext,
) {
    // A list well over `render::LINE_SPLIT_THRESHOLD` (2,000) lines, as a non-active block (the
    // caret stays in the paragraph before it), exercises `render_rendered`'s windowed path and
    // its cached per-line height estimate. The cache is keyed by the block's `Arc<ParsedBlock>`
    // pointer, so a reparse (a new `Arc`) must miss it, not hand `render_window` a heights list
    // sized for the old (longer) line count against the new, shorter `ir.lines`.
    let items = 2_500;
    let list: String = (0..items).map(|i| format!("- item {i}\n")).collect();
    let doc = format!("para\n\n{list}\nafter\n");
    let (editor, cx) = open(&doc, cx);
    assert_eq!(
        kinds(&editor, cx)[1],
        BlockKind::List { ordered: false },
        "sanity: the big block is a list"
    );

    // Delete most of the list items in one edit: a reparse producing far fewer `ir.lines`,
    // while the caret (and so "active") stays in block 0 throughout.
    let at = doc.find("- item 2000\n").unwrap();
    let end = doc.rfind("- item 2499\n").unwrap() + "- item 2499\n".len();
    editor.update(cx, |e, cx| e.replace(at..end, "", cx));
    cx.run_until_parked();

    assert_eq!(
        kinds(&editor, cx)[1],
        BlockKind::List { ordered: false },
        "still one list block, just shorter"
    );
    assert!(!text(&editor, cx).contains("item 2000"), "the removed items are really gone");
    assert!(text(&editor, cx).contains("item 1999"), "items before the cut remain");
    assert!(text(&editor, cx).contains("after\n"), "the trailing paragraph still parses");
}
