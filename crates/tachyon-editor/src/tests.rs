use gpui::{
    Entity, EntityInputHandler, TestAppContext, VisualContext, VisualTestContext, WindowHandle,
};
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
    assert_eq!(editor.read_with(cx, |e, _| e.title()), "*note.md - Tachyon");

    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Note\r\n\r\nbody\r\n!");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));
    assert_eq!(editor.read_with(cx, |e, _| e.title()), "note.md - Tachyon");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn undo_and_redo_back_to_the_saved_text_clears_is_modified(cx: &mut TestAppContext) {
    let (editor, cx) = open("para\n", cx);
    assert!(!editor.read_with(cx, |e, _| e.is_modified()));

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("!");
    assert!(editor.read_with(cx, |e, _| e.is_modified()));

    // Undo back to exactly the text last saved (here, loaded) clears the dirty marker: it is
    // not enough to check "no edit happened since", or this would still show dirty.
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "para\n");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()), "undo restored the saved text");

    // Redo back away from the saved text is dirty again, and redoing once more - landing back
    // on the saved position - clears it again.
    cx.simulate_keystrokes("secondary-shift-z");
    assert_eq!(text(&editor, cx), "para\n!");
    assert!(editor.read_with(cx, |e, _| e.is_modified()));

    cx.simulate_keystrokes("secondary-z");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()), "undo again restores the saved text");

    // Retyping the same characters by hand instead of redoing them is a new edit: it lands on
    // the same text but a different point in the undo history, so it stays dirty.
    cx.simulate_input("!");
    assert_eq!(text(&editor, cx), "para\n!");
    assert!(
        editor.read_with(cx, |e, _| e.is_modified()),
        "a hand-retyped edit still counts as modified"
    );
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
fn a_100_percent_content_width_still_leaves_a_frame_gap(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::System,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::Percent(100.),
            ..Default::default()
        })
    });
    let (editor, cx) = open("hello\n", cx);
    let (content_width, window_width) =
        editor.read_with(cx, |e, _| (e.theme.content_width, e.window_viewport.size.width));
    // The gap is `CONTENT_WIDTH_GAP_REMS` (3) rems at this window's default zoom of 1
    // (`BASE_REM_SIZE` is 16px).
    let gap = gpui::px(16.) * 3.;
    assert_eq!(
        content_width,
        window_width - gap * 2.,
        "100% is still capped to the window less a gap on each side"
    );
    assert!(
        content_width < window_width,
        "the column never reaches the window's own edges: {content_width:?} vs {window_width:?}"
    );
}

#[gpui::test]
fn saving_a_new_content_width_reflows_an_open_windows_column(cx: &mut TestAppContext) {
    let dir = backup_dir("content-width-setting");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| cx.set_global(crate::SettingsFile(file.clone())));
    let (editor, cx) = open_file(&file, cx);
    let before = editor.read_with(cx, |e, _| e.theme.content_width);
    assert_eq!(before, gpui::px(820.), "the default width, from the freshly written file");

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("content_width = \"680px\"\n");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();

    let after = editor.read_with(cx, |e, _| e.theme.content_width);
    assert_eq!(after, gpui::px(680.), "saved settings resize an open window's column, live");
    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert_eq!(settings.content_width, crate::ContentWidth::Px(680.));
    let _ = std::fs::remove_dir_all(&dir);
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
    assert_eq!(selected(cx).1, doc.rfind("fish").expect("fixture"), "wraps backward");
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

#[gpui::test]
fn session_end_backs_up_immediately_without_closing_the_window(cx: &mut TestAppContext) {
    let dir = backup_dir("session-end");
    cx.update(|cx| cx.set_global(crate::Backups::new(dir.clone())));
    let (editor, vcx) = open("", cx);

    // Nothing typed yet: no backup should appear, and nothing to do counts as safe.
    assert!(editor.update(vcx, |e, cx| e.backup_for_session_end(cx)));
    vcx.run_until_parked();
    assert!(backups_in(&dir).is_empty(), "nothing unsaved, nothing to back up");

    vcx.simulate_input("draft");
    assert!(editor.update(vcx, |e, cx| e.backup_for_session_end(cx)), "the write succeeded");
    vcx.run_until_parked();
    assert_eq!(backups_in(&dir), ["draft"], "written right away, not after the usual pause");
    assert!(!vcx.windows().is_empty(), "the window stays open: this only preserves the text");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn session_end_without_hot_exit_writes_nothing(cx: &mut TestAppContext) {
    let (editor, vcx) = open("", cx);
    vcx.simulate_input("draft");
    assert!(editor.update(vcx, |e, cx| e.backup_for_session_end(cx)), "no Backups global: safe");
    vcx.run_until_parked();
    assert!(editor.read_with(vcx, |e, _| e.backup_slot.is_none()), "no Backups global, no slot");
}

#[gpui::test]
fn session_end_reports_failure_when_the_backup_cannot_be_written(cx: &mut TestAppContext) {
    // A plain file where the backup directory needs to be: `create_dir_all` for any path under
    // it must fail (its parent is not a directory), so the write is deterministically refused
    // without relying on permissions or disk space.
    let blocker = backup_dir("session-end-failure");
    std::fs::write(&blocker, "not a directory").expect("create the blocking file");
    cx.update(|cx| cx.set_global(crate::Backups::new(blocker.join("backups"))));
    let (editor, vcx) = open("", cx);
    vcx.simulate_input("draft");
    assert!(
        !editor.update(vcx, |e, cx| e.backup_for_session_end(cx)),
        "the backup directory could not be created"
    );
    let _ = std::fs::remove_file(&blocker);
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
fn command_palette_filters_and_runs_a_command(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\npara\n", cx);
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("toggle plain");
    let labels = editor.read_with(cx, |e, _| {
        let picker = e.picker.as_ref().expect("open");
        picker.matches.iter().map(|&i| picker.items[i].label.clone()).collect::<Vec<_>>()
    });
    assert_eq!(labels, ["Toggle plain-text mode"], "fuzzy filter narrows to one command");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.picker.is_none()), "the palette closed");
    assert_eq!(
        editor.read_with(cx, |e, _| e.document().mode()),
        tachyon_doc::DocMode::Plain,
        "Enter ran the command"
    );
}

#[gpui::test]
fn command_palette_about_row_opens_the_about_window(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::AppInfo { version: "1.2.3".into(), resident: false });
    });
    let (_editor, cx) = open("text\n", cx);
    let before = cx.update(|_, cx| cx.windows().len());
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("about");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    // A row's action is looked up by name and an unknown name is skipped silently, so a
    // misspelled row would do nothing: the window must actually open.
    assert_eq!(cx.update(|_, cx| cx.windows().len()), before + 1, "About Tachyon opened a window");
}

#[gpui::test]
fn command_palette_shows_shortcuts_from_the_keymap(cx: &mut TestAppContext) {
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("secondary-shift-p");
    let shortcut = editor.read_with(cx, |e, _| {
        let picker = e.picker.as_ref().expect("open");
        picker
            .matches
            .iter()
            .map(|&i| &picker.items[i])
            .find(|item| item.label == "Go to heading")
            .and_then(|item| item.shortcut.clone())
    });
    // `secondary-shift-o` in `key_bindings()`, written like the README (`Ctrl+Shift+O`; `Cmd` on
    // macOS), not GPUI's `ctrl-shift-O`.
    let expected = if cfg!(target_os = "macos") { "Cmd+Shift+O" } else { "Ctrl+Shift+O" };
    assert_eq!(shortcut.as_deref(), Some(expected), "read from the actual key_bindings()");
}

#[gpui::test]
fn f1_opens_the_command_palette(cx: &mut TestAppContext) {
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("f1");
    let title = editor.read_with(cx, |e, _| e.picker.as_ref().map(|picker| picker.title));
    assert_eq!(title, Some("Command Palette"));
}

#[gpui::test]
fn right_click_opens_the_command_palette(cx: &mut TestAppContext) {
    let (editor, cx) = open("text\n", cx);
    cx.simulate_mouse_down(
        gpui::point(gpui::px(8.), gpui::px(8.)),
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    let title = editor.read_with(cx, |e, _| e.picker.as_ref().map(|picker| picker.title));
    assert_eq!(title, Some("Command Palette"));
}

#[gpui::test]
fn rest_after_load_clears_the_active_block_only_when_asked(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\npara\n", cx);
    editor.update(cx, |editor, cx| editor.rest_after_load(cx));
    assert_eq!(editor.read_with(cx, |editor, _| editor.active_block()), Some(0));
    cx.update(|_, cx| {
        cx.set_global(crate::Settings {
            start_without_active_block: true,
            ..crate::Settings::default()
        });
    });
    editor.update(cx, |editor, cx| editor.rest_after_load(cx));
    assert_eq!(editor.read_with(cx, |editor, _| editor.active_block()), None);
}

#[gpui::test]
fn click_outside_the_picker_closes_it_without_moving_the_caret(cx: &mut TestAppContext) {
    let (editor, cx) = open("one two\n", cx);
    editor.update(cx, |editor, cx| editor.move_to(4, false, cx));
    let before = selection(&editor, cx);
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_mouse_down(
        gpui::point(gpui::px(8.), gpui::px(8.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::none(),
    );
    assert!(editor.read_with(cx, |editor, _| editor.picker.is_none()));
    assert_eq!(selection(&editor, cx), before, "the dismiss click is not a caret click");
}

/// Filters the palette to the three "Theme: ..." rows and returns their labels in list order,
/// with `initial` set as the current theme (so a different one carries the checkmark each time).
/// The checkmark must never change this order - see `Item::marked`'s doc comment for the bug
/// this guards: folding the mark into the matched text made whichever row currently carried it
/// sort after the other two, so the same arrow-key count picked a different theme depending on
/// which one was already active.
fn theme_row_order(initial: crate::ThemeChoice, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: initial,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
        });
    });
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("theme");
    editor.read_with(cx, |e, _| {
        let picker = e.picker.as_ref().expect("open");
        picker.matches.iter().map(|&i| picker.items[i].label.clone()).collect()
    })
}

#[gpui::test]
fn command_palette_theme_rows_keep_the_same_order_whichever_theme_is_current(
    cx: &mut TestAppContext,
) {
    let expected = ["Theme: Dark", "Theme: Light", "Theme: System"];
    for choice in [crate::ThemeChoice::Dark, crate::ThemeChoice::Light, crate::ThemeChoice::System]
    {
        assert_eq!(theme_row_order(choice, cx), expected, "current theme: {choice:?}");
    }
}

#[gpui::test]
fn command_palette_escape_closes_without_running_anything(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\npara\n", cx);
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("toggle plain");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.picker.is_none()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.document().mode()),
        tachyon_doc::DocMode::Markdown,
        "nothing ran"
    );
}

#[gpui::test]
fn command_palette_theme_light_applies_at_once_and_persists_keeping_a_comment(
    cx: &mut TestAppContext,
) {
    let dir = backup_dir("command-palette-theme");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::Dark,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
        });
        cx.set_global(crate::SettingsFile(file.clone()));
    });
    let (editor, cx) = open("text\n", cx);

    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("Theme: Light");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert!(editor.read_with(cx, |e, _| e.picker.is_none()));
    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert_eq!(settings.theme, crate::ThemeChoice::Light, "applied at once, not only on save");

    let written = std::fs::read_to_string(&file).expect("read");
    assert!(written.contains("theme = \"light\""), "persisted: {written:?}");
    assert!(
        written.contains("# Tachyon settings. Saving this file applies them to open windows."),
        "kept the file's comments: {written:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn command_palette_whats_new_toggle_turns_it_off_and_persists_keeping_other_lines(
    cx: &mut TestAppContext,
) {
    let dir = backup_dir("command-palette-whats-new");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::Dark,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
        });
        cx.set_global(crate::SettingsFile(file.clone()));
    });
    let (editor, cx) = open("text\n", cx);

    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("what's new after");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert!(editor.read_with(cx, |e, _| e.picker.is_none()));
    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert!(!settings.whats_new, "applied at once, not only on save");

    let written = std::fs::read_to_string(&file).expect("read");
    assert!(written.contains("whats_new = false"), "persisted: {written:?}");
    assert!(written.contains("hot_exit = true"), "kept the file's other lines: {written:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn command_palette_restore_session_toggle_turns_it_off_and_persists(cx: &mut TestAppContext) {
    let dir = backup_dir("command-palette-restore-session");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| cx.set_global(crate::SettingsFile(file.clone())));
    let (editor, cx) = open("text\n", cx);

    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("restore session");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert!(editor.read_with(cx, |e, _| e.picker.is_none()));
    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert!(!settings.restore_session, "applied at once, not only on save");

    let written = std::fs::read_to_string(&file).expect("read");
    assert!(written.contains("restore_session = false"), "persisted: {written:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn quick_open_lists_the_same_recent_files_as_open_recent(cx: &mut TestAppContext) {
    let dir = backup_dir("quick-open");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (a, b, current) = (dir.join("alpha.md"), dir.join("beta.md"), dir.join("gamma.md"));
    for file in [&a, &b, &current] {
        std::fs::write(file, "text\n").expect("write");
    }
    cx.update(|cx| cx.set_global(crate::RecentFiles::new(dir.join("recent.txt"))));
    let (editor, cx) = open("", cx);
    for file in [&a, &b, &current] {
        editor.update(cx, |e, cx| e.set_file(file.clone(), cx));
        cx.run_until_parked();
    }

    cx.simulate_keystrokes("secondary-p");
    let labels = editor.read_with(cx, |e, _| {
        let picker = e.picker.as_ref().expect("open");
        picker.matches.iter().map(|&i| picker.items[i].label.clone()).collect::<Vec<_>>()
    });
    assert_eq!(labels, ["beta.md", "alpha.md"], "same list Open recent shows, newest first");
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
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
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
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::Dark,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
        })
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
    let crate::LoadOutcome::Loaded(loaded) =
        crate::load_document(&path, tachyon_md::SoftBreaks::Space).expect("readable")
    else {
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
    let crate::LoadOutcome::Loaded(loaded) =
        crate::load_document(&path, tachyon_md::SoftBreaks::Space).expect("readable")
    else {
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

/// A settings/backups path pair for the About window's Environment table, isolated per test.
fn about_dirs(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = backup_dir(name);
    (dir.join("settings.toml"), dir.join("backups"))
}

fn open_about(cx: &mut TestAppContext) -> WindowHandle<crate::AboutView> {
    cx.update(crate::about::open_about).expect("the About window opens")
}

#[gpui::test]
fn about_window_opens_once_and_a_second_request_focuses_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::AppInfo { version: "1.2.3".into(), resident: true });
    });
    let first = open_about(cx);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);

    let second = open_about(cx);
    cx.run_until_parked();
    assert_eq!(first, second, "a second request reuses the same window");
    assert_eq!(cx.update(|cx| cx.windows().len()), 1, "no second window was opened");
}

#[gpui::test]
fn escape_and_the_close_button_close_the_about_window(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::AppInfo { version: "1.2.3".into(), resident: false });
    });

    let handle = open_about(cx);
    cx.run_until_parked();
    let mut about = VisualTestContext::from_window(handle.into(), cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    about.simulate_keystrokes("escape");
    assert_eq!(cx.update(|cx| cx.windows().len()), 0, "Escape closes the window");

    // Reopened, the same `CloseWindow` action the Close button's `on_click` and Quit's
    // close-every-window loop both use closes it too.
    let handle = open_about(cx);
    cx.run_until_parked();
    let mut about = VisualTestContext::from_window(handle.into(), cx);
    about.dispatch_action(crate::CloseWindow);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 0, "CloseWindow (the Close button) closes it");
}

#[gpui::test]
fn the_environment_details_include_the_version_and_settings_path(cx: &mut TestAppContext) {
    let (settings_path, backups_dir) = about_dirs("about-env");
    cx.update(|cx| {
        cx.set_global(crate::AppInfo { version: "9.9.9".into(), resident: true });
        cx.set_global(crate::SettingsFile(settings_path.clone()));
        cx.set_global(crate::Backups::new(backups_dir));
    });
    let handle = open_about(cx);
    cx.run_until_parked();
    let details = handle.read_with(cx, |view, _| view.environment_text()).expect("window is open");
    assert!(details.contains("9.9.9"), "{details}");
    assert!(details.contains(&settings_path.display().to_string()), "{details}");
}

#[gpui::test]
fn a_pro_tip_shows_by_default_after_the_first_frame(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    let tip = editor.read_with(cx, |e, _| e.tip.clone());
    assert!(tip.as_deref().is_some_and(|t| t.starts_with("Pro Tip: ")), "{tip:?}");
}

#[gpui::test]
fn tips_false_in_settings_keeps_the_tip_hidden(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::System,
            zoom: 1.,
            hot_exit: true,
            tips: false,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
        })
    });
    let (editor, cx) = open("", cx);
    assert_eq!(editor.read_with(cx, |e, _| e.tip.clone()), None);
}

#[gpui::test]
fn the_tip_does_not_change_while_editing(cx: &mut TestAppContext) {
    let (editor, cx) = open("hello\n", cx);
    let before = editor.read_with(cx, |e, _| e.tip.clone());
    cx.simulate_keystrokes("end");
    cx.simulate_input(" world");
    cx.run_until_parked();
    let after = editor.read_with(cx, |e, _| e.tip.clone());
    assert_eq!(before, after, "one tip per window, not re-picked on every edit");
}

#[gpui::test]
fn saving_tips_false_hides_an_already_shown_tip(cx: &mut TestAppContext) {
    let dir = backup_dir("tips-setting");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            theme: crate::ThemeChoice::System,
            zoom: 1.,
            hot_exit: true,
            tips: true,
            whats_new: true,
            restore_session: true,
            content_width: crate::ContentWidth::default(),
            ..Default::default()
        });
        cx.set_global(crate::SettingsFile(file.clone()));
    });
    let (editor, cx) = open_file(&file, cx);
    let tip = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.tip.clone());
    assert!(tip(cx).is_some_and(|t| t.starts_with("Pro Tip: ")), "tips on by default");

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("tips = false\n");
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(tip(cx), None, "saved tips = false hides an already-shown tip, live");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn restore_view_reapplies_caret_and_scroll_then_clamps_when_the_document_is_shorter(
    cx: &mut TestAppContext,
) {
    let long: String = (0..500).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let (editor, cx) = open(&long, cx);
    let caret = long.find("Paragraph 200.").expect("fixture");
    editor.update(cx, |e, cx| {
        e.restore_view(caret, caret, tachyon_doc::DocMode::Markdown, cx);
    });
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.head()), caret, "caret restored");
    let scroll_start = editor
        .read_with(cx, |e, _| e.document().block_range(e.list.logical_scroll_top().item_ix).start);
    assert!(scroll_start > 0, "scrolled to the recorded position, not left at the top");

    // The document is now much shorter, as if the session file predates an edit or a reload
    // (a file changed on disk, a different backup): restoring the same, now out-of-range caret
    // and scroll must clamp to the new length, not panic or leave the caret out of bounds.
    let short = "short\n";
    editor.update(cx, |e, cx| e.set_document(tachyon_doc::Document::new(short), cx));
    cx.run_until_parked();
    editor.update(cx, |e, cx| {
        e.restore_view(caret, caret, tachyon_doc::DocMode::Markdown, cx);
    });
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.head()), short.len(), "caret clamped to the new end");
}

#[gpui::test]
fn restore_view_switches_mode_but_refuses_markdown_above_the_size_limit(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\npara\n", cx);
    editor.update(cx, |e, cx| e.restore_view(0, 0, tachyon_doc::DocMode::Plain, cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);
    assert_eq!(text(&editor, cx), "# Title\n\npara\n", "retagging keeps the text as is");

    let big = "x".repeat(tachyon_doc::MARKDOWN_SIZE_LIMIT as usize + 1024);
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| e.set_document(tachyon_doc::Document::new_plain(&big), cx));
    cx.run_until_parked();
    editor.update(cx, |e, cx| e.restore_view(0, 0, tachyon_doc::DocMode::Markdown, cx));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.document().mode()),
        tachyon_doc::DocMode::Plain,
        "refused above the size limit, the same as a manual toggle"
    );
}

// -------------------------------------------------------------------------------------------
// Sticky notes (issue #69).
// -------------------------------------------------------------------------------------------

/// A fresh, empty notes folder for a test, removed first if an earlier run left it.
fn notes_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("tachyon-notes-editor-tests-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Opens a blank sticky note (unpinned, so the pin-toggle test has something to flip) with
/// `dir` as its `NotesDir`: the same injectable-global seam `backup_dir`'s callers use for
/// `Backups`, so autosave never touches a real per-user path in a test.
fn open_note(
    dir: std::path::PathBuf,
    cx: &mut TestAppContext,
) -> (Entity<Editor>, &mut VisualTestContext) {
    cx.update(crate::init);
    cx.update(|cx| cx.set_global(crate::NotesDir(dir)));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::new("", window, cx);
        editor.make_note(false);
        editor
    });
    cx.run_until_parked();
    (editor, cx)
}

#[gpui::test]
fn reopening_reserves_a_note_without_deleting_its_file_before_load(cx: &mut TestAppContext) {
    let dir = notes_dir("pending-reopen");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("saved.md");
    std::fs::write(&path, "important\n").unwrap();
    let (editor, cx) = open_note(dir.clone(), cx);
    editor.update(cx, |editor, cx| {
        editor.reserve_note_file(path.clone());
        assert!(editor.is_note_file(&path), "another reopen must focus this window");
        assert!(editor.flush_note(cx), "an empty loading note has nothing to save");
        assert!(path.exists(), "the pending path must not become a deletion target");
        editor.clear_pending_note_file();
        assert!(!editor.is_note_file(&path), "a failed read releases the reservation");
    });
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn a_superseded_note_folder_read_keeps_the_new_picker_and_its_query(cx: &mut TestAppContext) {
    let dir = notes_dir("picker-superseded");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("saved.md"), "saved\n").unwrap();
    let (editor, cx) = open_note(dir.clone(), cx);
    editor.update(cx, |editor, cx| editor.open_note_picker(cx));
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("scrollbar");
    cx.run_until_parked();
    let (title, query) = editor.read_with(cx, |editor, _| {
        let picker = editor.picker.as_ref().expect("palette stays open");
        (picker.title, picker.query.clone())
    });
    assert_eq!((title, query.as_str()), ("Command Palette", "scrollbar"));
    std::fs::remove_dir_all(&dir).unwrap();
}

const AUTOSAVE_PAUSE: std::time::Duration = std::time::Duration::from_millis(1600);

#[gpui::test]
fn a_note_autosaves_into_the_notes_folder_after_the_typing_pause_with_no_prompt(
    cx: &mut TestAppContext,
) {
    let dir = notes_dir("autosave");
    let (editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("# Grocery list");
    cx.run_until_parked();
    assert!(!dir.exists(), "the pause has not elapsed yet");

    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    assert!(cx.windows().len() == 1, "autosave never prompts or closes the window");

    let entries: Vec<_> = std::fs::read_dir(&dir).expect("folder created").flatten().collect();
    assert_eq!(entries.len(), 1, "exactly one file, named from the heading");
    let path = entries[0].path();
    assert_eq!(path.file_stem().and_then(|s| s.to_str()), Some("Grocery list"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Grocery list");
    assert_eq!(editor.read_with(cx, |e, _| e.file().map(|p| p.to_path_buf())), Some(path));

    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn an_empty_note_writes_nothing_and_emptying_a_saved_one_deletes_its_file(cx: &mut TestAppContext) {
    let dir = notes_dir("empty");

    // Never typed into: closing it writes nothing at all, not even the folder.
    let (_editor, note_cx) = open_note(dir.clone(), cx);
    note_cx.simulate_keystrokes("secondary-w");
    note_cx.run_until_parked();
    assert!(!dir.exists(), "an untouched note creates no folder");

    let (editor, note_cx) = open_note(dir.clone(), cx);
    note_cx.simulate_input("Draft");
    note_cx.executor().advance_clock(AUTOSAVE_PAUSE);
    note_cx.run_until_parked();
    let path = editor.read_with(note_cx, |e, _| e.file().expect("named by autosave").to_owned());
    assert!(path.exists(), "the first save wrote the file");

    note_cx.simulate_keystrokes("secondary-a backspace");
    note_cx.executor().advance_clock(AUTOSAVE_PAUSE);
    note_cx.run_until_parked();
    assert!(!path.exists(), "emptying it deletes the file");
    assert!(editor.read_with(note_cx, |e, _| e.file().is_none()));

    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn closing_a_note_never_prompts_even_with_unsaved_text(cx: &mut TestAppContext) {
    let dir = notes_dir("close-no-prompt");
    let (_editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("Typed just now, well inside the debounce window");
    // No prompt builder is installed (unlike `unsaved_close_prompt_works_from_the_keyboard`):
    // if `close_window` ever fell through to the ordinary `should_close` path for a note, this
    // would panic instead of closing.
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert!(cx.windows().is_empty(), "closed at once, no prompt");

    // The close flushed the text immediately, not waiting for the debounce.
    let entries: Vec<_> = std::fs::read_dir(&dir).expect("folder created").flatten().collect();
    assert_eq!(entries.len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn closing_a_note_from_the_window_frame_never_prompts(cx: &mut TestAppContext) {
    // Alt+F4, the taskbar or the Windows close area: the platform asks `should_close`, not the
    // editor's own Close action. Without a prompt builder a prompt would not answer, and the
    // window would stay open.
    let dir = notes_dir("frame-close-no-prompt");
    let (_editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("Typed just now, well inside the debounce window");
    assert!(cx.simulate_close(), "the frame close goes through without asking");
    let entries: Vec<_> = std::fs::read_dir(&dir).expect("folder created").flatten().collect();
    assert_eq!(entries.len(), 1, "the text was saved on the way out");
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn an_app_notice_survives_the_file_finishing_loading(cx: &mut TestAppContext) {
    // The sticky-note hotkey conflict is found at start-up, often while a restored window is
    // still loading its file; the load replacing the document must not take the notice with it.
    let (editor, cx) = open("Loading notes.md...\n", cx);
    editor.update(cx, |editor, cx| {
        editor.set_app_notice("The sticky-note hotkey is already in use".into(), cx);
        editor.set_document(tachyon_doc::Document::new("# Loaded\n"), cx);
    });
    let (notice, app_notice) =
        editor.read_with(cx, |editor, _| (editor.notice.clone(), editor.app_notice.clone()));
    assert_eq!(notice, None, "the document's own notice goes with the old document");
    assert_eq!(app_notice.as_deref(), Some("The sticky-note hotkey is already in use"));
}

#[gpui::test]
fn session_state_is_none_for_a_note(cx: &mut TestAppContext) {
    let dir = notes_dir("session-state");
    let (editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("Some text");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    let state = editor.update_in(cx, |e, window, cx| e.session_state(window, cx));
    assert!(state.is_none(), "a note is never part of the regular session file");
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn ctrl_shift_t_toggles_the_pinned_state_and_it_is_written_to_the_notes_state_file(
    cx: &mut TestAppContext,
) {
    let dir = notes_dir("pin-toggle");
    let (editor, cx) = open_note(dir.clone(), cx);
    // A real, positive-size placement for `note_window_state` to record (`Editor::last_placement`
    // only ever holds one after the window has actually been seen at some bounds).
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(360.)));
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.note_pinned()), "starts unpinned");

    cx.simulate_keystrokes("ctrl-shift-t");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.note_pinned()), "toggled on");

    // Name it - an unnamed note records nothing (`Editor::note_window_state`) - then write its
    // record through the real notes-state-file functions, the same ones the application uses.
    cx.simulate_input("Reminder");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    let record =
        editor.read_with(cx, |e, _| e.note_window_state()).expect("a named note has a record");
    assert!(record.pinned, "the toggle is reflected in the record");

    let state_path = dir.join("notes-state.txt");
    crate::write_notes_state(&state_path, std::slice::from_ref(&record)).unwrap();
    assert_eq!(crate::read_notes_state(&state_path), vec![record]);

    std::fs::remove_dir_all(&dir).ok();
}

/// A stand-in Mermaid renderer: an SVG of a fixed size for any source starting `graph`, an error
/// otherwise, and a panic for `boom`, like a renderer bug on input it does not handle.
fn fake_mermaid(source: &str, _dark: bool) -> Result<String, String> {
    if source.starts_with("boom") {
        panic!("renderer bug");
    }
    if source.starts_with("graph") {
        Ok(r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40" viewBox="0 0 120 40"><rect width="120" height="40"/></svg>"#.to_owned())
    } else {
        Err("unexpected token".to_owned())
    }
}

fn diagram_states(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    editor.read_with(cx, |e, _| e.diagram_states())
}

#[gpui::test]
fn mermaid_blocks_render_in_the_background_and_stay_code_when_they_fail(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(crate::MermaidRenderer(fake_mermaid)));
    let doc = "para\n\n```mermaid\ngraph TD\n```\n\n```mermaid\nnot a diagram\n```\n\n```mermaid\nboom\n```\n\n```rust\ngraph\n```\n";
    let (editor, cx) = open(doc, cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    // One rendered diagram, one refused source, and a renderer panic contained as a failure;
    // the Rust block is never sent to the renderer.
    let mut states = diagram_states(&editor, cx);
    states.sort();
    assert_eq!(states, vec!["failed", "failed", "ready"]);
    assert!(editor.read_with(cx, |e, _| e.text() == doc), "rendering never edits the text");
}

#[gpui::test]
fn mermaid_off_or_no_renderer_shows_code_and_renders_nothing(cx: &mut TestAppContext) {
    let doc = "para\n\n```mermaid\ngraph TD\n```\n";
    {
        let (editor, cx) = open(doc, cx);
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(diagram_states(&editor, cx).is_empty(), "no renderer installed");
    }
    cx.update(|cx| {
        cx.set_global(crate::MermaidRenderer(fake_mermaid));
        cx.set_global(crate::Settings { mermaid: false, ..Default::default() });
    });
    let (editor, cx) = open(doc, cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(diagram_states(&editor, cx).is_empty(), "the setting is off");
}

#[gpui::test]
fn preferences_flip_a_flag_and_write_it_keeping_comments(cx: &mut TestAppContext) {
    let dir = backup_dir("preferences-flag");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    // An early build's file: empty. The panel still lists every setting.
    std::fs::write(&file, "").expect("write");
    cx.update(|cx| cx.set_global(crate::SettingsFile(file.clone())));
    let (editor, cx) = open("text\n", cx);

    cx.simulate_keystrokes("secondary-,");
    let rows = editor.read_with(cx, |e, _| e.picker.as_ref().map(|p| p.items.len()));
    assert_eq!(rows, Some(crate::settings::KEYS.len() + 1), "every setting, then reset");

    cx.simulate_input("mermaid");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert!(!settings.mermaid, "flipped at once");
    let label = editor.read_with(cx, |e, _| {
        let p = e.picker.as_ref().expect("stays open");
        p.items[p.matches[p.selected]].label.clone()
    });
    assert_eq!(label, "Mermaid diagrams: Off", "on the same row, showing the new value");
    let query = editor.read_with(cx, |e, _| e.picker.as_ref().map(|p| p.query.clone()));
    assert_eq!(query.as_deref(), Some("mermaid"), "the filter survives the change");
    let written = std::fs::read_to_string(&file).expect("read");
    assert!(written.contains("# Draw ```mermaid code blocks as diagrams"), "{written}");
    assert!(written.ends_with("mermaid = false\n"), "{written}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn preferences_value_field_refuses_bad_input_and_applies_good(cx: &mut TestAppContext) {
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("secondary-,");
    cx.simulate_input("Zoom of new");
    cx.simulate_keystrokes("enter");
    let query = editor.read_with(cx, |e, _| e.picker.as_ref().map(|p| p.query.clone()));
    assert_eq!(query.as_deref(), Some("1"), "prefilled with the current value");

    cx.simulate_keystrokes("backspace");
    cx.simulate_input("9");
    cx.simulate_keystrokes("enter");
    let problem = editor.read_with(cx, |e, _| e.picker.as_ref().and_then(|p| p.problem.clone()));
    assert_eq!(problem.as_deref(), Some("zoom is a number from 0.5 to 3.0"));
    let zoom = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| cx.try_global::<crate::Settings>().cloned().unwrap_or_default().zoom)
    };
    assert_eq!(zoom(cx), 1., "nothing applied");

    cx.simulate_keystrokes("backspace");
    cx.simulate_input("1.5");
    cx.simulate_keystrokes("enter");
    assert_eq!(zoom(cx), 1.5);
    let back = editor.read_with(cx, |e, _| e.picker.as_ref().map(|p| p.editing.is_none()));
    assert_eq!(back, Some(true), "back to the list");

    // Escape in the field goes back to the list; Escape there closes it.
    cx.simulate_keystrokes("enter escape");
    assert!(editor.read_with(cx, |e, _| e.picker.as_ref().is_some_and(|p| p.editing.is_none())));
    cx.simulate_keystrokes("escape");
    assert!(editor.read_with(cx, |e, _| e.picker.is_none()));
}

#[gpui::test]
fn reset_restores_defaults_after_asking_and_keeps_a_backup(cx: &mut TestAppContext) {
    let dir = backup_dir("preferences-reset");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, "theme = \"dark\"\nzoom = 2\n").expect("write");
    cx.update(|cx| {
        cx.set_global(crate::Settings::parse("theme = \"dark\"\nzoom = 2\n").0);
        cx.set_global(crate::SettingsFile(file.clone()));
    });
    let (_editor, cx) = open("text\n", cx);

    cx.simulate_keystrokes("secondary-,");
    cx.simulate_input("Reset all");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx.global::<crate::Settings>().zoom),
        2.,
        "cancel changes nothing"
    );

    // Cancel returns to the panel on the reset row, so Enter asks again.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_prompt_answer("Reset");
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx.global::<crate::Settings>().clone()),
        crate::Settings::default()
    );
    assert_eq!(std::fs::read_to_string(&file).expect("read"), crate::DEFAULT_SETTINGS);
    let backup = std::fs::read_to_string(crate::settings::backup_path(&file)).expect("backup");
    assert_eq!(backup, "theme = \"dark\"\nzoom = 2\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn unsaved_scratch_and_undo_to_saved_title_follow_the_asterisk_rule(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    assert_eq!(editor.read_with(cx, |e, _| e.title()), "Tachyon");
    cx.simulate_input("x");
    assert_eq!(editor.read_with(cx, |e, _| e.title()), "*Tachyon", "scratch with unsaved text");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(editor.read_with(cx, |e, _| e.title()), "Tachyon", "undone back to unmodified");

    editor.update(cx, |e, _| e.set_title_override("What's new in Tachyon"));
    cx.simulate_input("y");
    assert_eq!(
        editor.read_with(cx, |e, _| e.title()),
        "What's new in Tachyon",
        "an override keeps its fixed title"
    );
}

#[gpui::test]
fn preferences_keep_the_changed_row_when_the_filter_matched_its_old_value(cx: &mut TestAppContext) {
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("secondary-,");
    // "mermaid diagrams: on" matches only while the value is On.
    cx.simulate_input("diagrams: on");
    cx.simulate_keystrokes("enter");
    let chosen = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            let p = e.picker.as_ref().expect("stays open");
            p.items[p.matches[p.selected]].label.clone()
        })
    };
    assert_eq!(chosen(cx), "Mermaid diagrams: Off");
    // A second Enter acts on the same setting, not on whatever else the filter matches.
    cx.simulate_keystrokes("enter");
    assert_eq!(chosen(cx), "Mermaid diagrams: On");
}

#[gpui::test]
fn a_reset_queued_after_a_change_leaves_the_file_at_the_defaults(cx: &mut TestAppContext) {
    let dir = backup_dir("preferences-order");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, crate::DEFAULT_SETTINGS).expect("write");
    cx.update(|cx| cx.set_global(crate::SettingsFile(file.clone())));
    let (_editor, cx) = open("text\n", cx);
    // A change and a reset in the same turn: their writes must land in that order.
    cx.simulate_keystrokes("secondary-,");
    cx.simulate_input("mermaid");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("escape secondary-,");
    cx.simulate_input("Reset all");
    cx.simulate_keystrokes("enter");
    cx.simulate_prompt_answer("Reset");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).expect("read"), crate::DEFAULT_SETTINGS);
    let backup = std::fs::read_to_string(crate::settings::backup_path(&file)).expect("backup");
    assert!(backup.ends_with("mermaid = false\n"), "the change was saved before the reset");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn a_reset_whose_backup_cannot_be_written_leaves_the_file_and_says_so(cx: &mut TestAppContext) {
    let dir = backup_dir("preferences-backup-fails");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, "zoom = 2\n").expect("write");
    // A directory where the backup file should go: writing it fails.
    std::fs::create_dir_all(crate::settings::backup_path(&file).join("blocker")).expect("dir");
    cx.update(|cx| cx.set_global(crate::SettingsFile(file.clone())));
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("secondary-,");
    cx.simulate_input("Reset all");
    cx.simulate_keystrokes("enter");
    cx.simulate_prompt_answer("Reset");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).expect("read"), "zoom = 2\n", "file untouched");
    let notice = editor.read_with(cx, |e, _| e.notice.clone()).expect("a notice");
    assert!(notice.starts_with("Settings were not saved: could not keep a backup"), "{notice}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn save_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tachyon-saveas-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn mode_of(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> tachyon_doc::DocMode {
    editor.read_with(cx, |e, _| e.document().mode())
}

#[gpui::test]
fn saving_under_a_text_extension_switches_a_markdown_document_to_plain_text(
    cx: &mut TestAppContext,
) {
    let dir = save_dir("to-plain");
    let (editor, cx) = open("# Title\n\nbody\n", cx);
    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Markdown);
    let text_before = text(&editor, cx);

    editor.update_in(cx, |e, window, cx| {
        e.write_to(dir.join("server.log"), false, window, cx, |_, _| {})
    });
    cx.run_until_parked();

    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Plain, "the extension decides");
    assert_eq!(text(&editor, cx), text_before, "the text is untouched");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Plain], "'#' is now literal");
    assert_eq!(std::fs::read_to_string(dir.join("server.log")).unwrap(), "# Title\n\nbody\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn saving_a_plain_document_as_markdown_switches_it_to_markdown(cx: &mut TestAppContext) {
    let dir = save_dir("to-markdown");
    let (editor, cx) = open("# Title\n", cx);
    editor.update(cx, |e, cx| e.retag_to(tachyon_doc::DocMode::Plain, cx));
    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Plain);

    editor.update_in(cx, |e, window, cx| {
        e.write_to(dir.join("notes.md"), false, window, cx, |_, _| {})
    });
    cx.run_until_parked();

    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Markdown);
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Heading(1)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn an_ordinary_save_keeps_a_manual_mode_choice(cx: &mut TestAppContext) {
    let dir = save_dir("keeps-choice");
    let path = dir.join("notes.md");
    std::fs::write(&path, "# Title\n").unwrap();
    let (editor, cx) = open("", cx);
    let loaded = tachyon_doc::Document::new("# Title\n");
    editor.update(cx, |e, cx| {
        e.set_document(loaded, cx);
        e.set_file(path.clone(), cx);
    });
    // A `.md` file the user switched to plain text with Ctrl+Shift+M stays plain when saved
    // under its own name; only a rename re-derives the mode.
    cx.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Plain);
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Plain);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_as_suggests_an_extension_that_matches_the_mode() {
    use tachyon_doc::DocMode;
    assert_eq!(super::editor::suggested_name(DocMode::Markdown), "untitled.md");
    assert_eq!(super::editor::suggested_name(DocMode::Plain), "untitled.txt");
}

#[test]
fn save_as_switches_to_markdown_only_when_that_is_cheap_and_sensible() {
    use std::path::Path;
    use tachyon_doc::{DocMode, PARSE_CHUNK};
    let f = super::editor::mode_after_save_as;
    let big = PARSE_CHUNK + 1;
    assert_eq!(f(DocMode::Plain, Path::new("small.md"), 10, false), Some(DocMode::Markdown));
    assert_eq!(
        f(DocMode::Plain, Path::new("edge.md"), PARSE_CHUNK, false),
        Some(DocMode::Markdown)
    );
    // Parsing this much on the UI thread would stall a frame; Ctrl+Shift+M still offers it.
    assert_eq!(f(DocMode::Plain, Path::new("big.md"), big, false), None);
    // Binary or garbled text is never parsed as Markdown, as when such a file is opened.
    assert_eq!(f(DocMode::Plain, Path::new("junk.md"), 10, true), None);
    // Going to plain text is always cheap, whatever the size or content.
    assert_eq!(f(DocMode::Markdown, Path::new("a.json"), big, true), Some(DocMode::Plain));
    assert_eq!(f(DocMode::Markdown, Path::new("a.MD"), 10, false), None, "case-insensitive");
    assert_eq!(f(DocMode::Plain, Path::new("README"), 10, false), None, "no extension is plain");
}

#[gpui::test]
fn saving_a_large_plain_document_as_markdown_leaves_it_plain(cx: &mut TestAppContext) {
    let dir = save_dir("large-stays-plain");
    let big = "word ".repeat(tachyon_doc::PARSE_CHUNK / 4);
    assert!(big.len() > tachyon_doc::PARSE_CHUNK);
    let (editor, cx) = open("", cx);
    editor.update(cx, |e, cx| {
        e.set_document(tachyon_doc::Document::new_plain(&big), cx);
    });
    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Plain);

    editor.update_in(cx, |e, window, cx| {
        e.write_to(dir.join("big.md"), false, window, cx, |_, _| {})
    });
    cx.run_until_parked();

    assert_eq!(mode_of(&editor, cx), tachyon_doc::DocMode::Plain, "no UI-thread parse");
    assert_eq!(std::fs::metadata(dir.join("big.md")).unwrap().len() as usize, big.len());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Where the task box of the list item on `line` (0-based) is drawn, for clicking it: found by
/// asking the editor for the same offset the render path passes to the toggle.
fn task_offset(editor: &Entity<Editor>, line_text: &str, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, _| e.text().find(line_text).expect("the item is in the text"))
}

#[gpui::test]
fn clicking_a_task_box_toggles_it_without_moving_the_caret_or_swapping_the_block(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open("intro\n\n- [ ] one\n- [x] two\n- plain\n", cx);
    // The caret rests in the intro paragraph; the list is rendered, not raw.
    let caret_before = editor.read_with(cx, |e, _| e.selection());
    let active_before = editor.read_with(cx, |e, _| e.active_block());
    let one = task_offset(&editor, "- [ ] one", cx);

    editor.update(cx, |e, cx| assert!(e.toggle_task_at(one, cx)));
    cx.run_until_parked();

    assert_eq!(text(&editor, cx), "intro\n\n- [x] one\n- [x] two\n- plain\n");
    assert_eq!(editor.read_with(cx, |e, _| e.selection()), caret_before, "the caret did not move");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), active_before, "no block went raw");

    // A checked box unchecks, from any offset on its line.
    let two = task_offset(&editor, "- [x] two", cx);
    editor.update(cx, |e, cx| assert!(e.toggle_task_at(two + 6, cx)));
    assert_eq!(text(&editor, cx), "intro\n\n- [x] one\n- [ ] two\n- plain\n");
}

#[gpui::test]
fn a_task_toggle_is_one_undo_step_of_its_own(cx: &mut TestAppContext) {
    let (editor, cx) = open("- [ ] a\n- [ ] b\n", cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("!");
    let a = task_offset(&editor, "- [ ] a", cx);
    editor.update(cx, |e, cx| e.toggle_task_at(a, cx));
    let b = task_offset(&editor, "- [ ] b", cx);
    editor.update(cx, |e, cx| e.toggle_task_at(b, cx));
    assert_eq!(text(&editor, cx), "- [x] a\n- [x] b\n!");

    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "- [x] a\n- [ ] b\n!", "undo reverts the last toggle only");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "- [ ] a\n- [ ] b\n!", "then the first, not the typing");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "- [ ] a\n- [ ] b\n", "then the typing");
}

#[gpui::test]
fn a_task_toggle_refuses_lines_that_are_not_tasks_and_code(cx: &mut TestAppContext) {
    let doc = "- plain [ ] inline\n\n```\n- [ ] in code\n```\n\ntext\n";
    let (editor, cx) = open(doc, cx);
    for needle in ["- plain", "- [ ] in code", "text"] {
        let at = task_offset(&editor, needle, cx);
        editor.update(cx, |e, cx| assert!(!e.toggle_task_at(at, cx), "{needle:?} is not a task"));
    }
    assert_eq!(text(&editor, cx), doc, "nothing changed");
}

#[gpui::test]
fn a_task_toggle_keeps_a_selection_in_place(cx: &mut TestAppContext) {
    let (editor, cx) = open("- [ ] one two\n", cx);
    editor.update(cx, |e, cx| {
        e.move_to(6, false, cx);
        e.move_to(9, true, cx);
    });
    let selection = editor.read_with(cx, |e, _| e.selection());
    assert_eq!(selection, 6..9);
    editor.update(cx, |e, cx| e.toggle_task_at(0, cx));
    assert_eq!(editor.read_with(cx, |e, _| e.selection()), selection);
    assert_eq!(text(&editor, cx), "- [x] one two\n");
}

#[gpui::test]
fn a_real_click_on_a_task_box_toggles_it_and_leaves_the_caret_alone(cx: &mut TestAppContext) {
    let (editor, cx) = open("intro\n\n- [ ] one\n- [x] two\n", cx);
    let caret_before = editor.read_with(cx, |e, _| e.selection());
    // Each box's selector carries its item's source offset: "one" starts at byte 7.
    let boxed = cx.debug_bounds("task-box-7").expect("the first task box is drawn");

    cx.simulate_click(boxed.center(), gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(text(&editor, cx), "intro\n\n- [x] one\n- [x] two\n");
    assert_eq!(editor.read_with(cx, |e, _| e.selection()), caret_before, "the caret stayed");
    assert!(!editor.read_with(cx, |e, _| e.selecting), "a toggle does not start a drag");
}

#[gpui::test]
fn a_real_click_on_a_task_items_text_edits_it_instead_of_toggling(cx: &mut TestAppContext) {
    let (editor, cx) = open("intro\n\n- [ ] one two three\n", cx);
    let boxed = cx.debug_bounds("task-box-7").expect("the task box is drawn");
    // On the item's text, a box-width and a half to the right of the box.
    let on_text = gpui::point(boxed.right() + boxed.size.width * 2., boxed.center().y);

    cx.simulate_click(on_text, gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(text(&editor, cx), "intro\n\n- [ ] one two three\n", "nothing toggled");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), Some(1), "the item is being edited");
}

#[gpui::test]
fn a_task_toggle_waits_for_a_pending_paste_so_edits_keep_input_order(cx: &mut TestAppContext) {
    let paste = "pasted\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 4);
    let (editor, cx) = open_with_reader("- [ ] one\n", Some(paste.clone()), cx);
    cx.simulate_keystrokes("ctrl-end");
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    // The paste is read and prepared in the background: still pending right now.
    assert_eq!(text(&editor, cx), "- [ ] one\n", "the paste has not landed yet");

    // The box is clicked before it lands; the paste was the earlier input, so it goes first.
    editor.update(cx, |e, cx| e.toggle_task_at(0, cx));
    cx.run_until_parked();

    let after = text(&editor, cx);
    assert!(after.starts_with("- [x] one\n"), "the box toggled");
    assert!(after.ends_with(&paste), "the paste landed at the caret, after the list");
    assert_eq!(after.len(), "- [x] one\n".len() + paste.len(), "nothing else changed");
    // Two separate undo steps: the toggle, then the paste.
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert!(text(&editor, cx).starts_with("- [ ] one\n"), "undo reverts the toggle");
    assert!(text(&editor, cx).ends_with(&paste), "the paste is still there");
}

#[gpui::test]
fn a_task_toggle_does_not_ask_for_the_caret_to_be_scrolled_into_view(cx: &mut TestAppContext) {
    let (editor, cx) = open("- [ ] one\n", cx);
    // An edit sets `reveal` (keep the caret on screen), which clicking a box elsewhere in a
    // long document must not do: the view would jump away from the box that was clicked.
    // Read inside the same update as the call: a repaint between would consume the flag.
    let (toggle, typing) = editor.update(cx, |e, cx| {
        e.reveal = false;
        e.toggle_task_at(0, cx);
        let toggle = e.reveal;
        // Control: an ordinary edit does ask for one, so the assertion below can fail.
        e.reveal = false;
        e.replace(0..0, "x", cx);
        (toggle, e.reveal)
    });
    assert!(typing, "typing reveals the caret");
    assert!(!toggle, "a toggle asks for no caret reveal");
}

#[gpui::test]
fn a_task_toggle_follows_its_box_when_a_pending_paste_lands_before_it(cx: &mut TestAppContext) {
    let paste = "pasted\n".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 4);
    let (editor, cx) = open_with_reader("- [ ] one\n- [ ] two\n", Some(paste.clone()), cx);
    // The caret is at the very start, so the paste lands *before* both boxes and shifts them.
    cx.simulate_keystrokes("ctrl-home");
    editor.update_in(cx, |e, window, cx| e.paste(&crate::editor::Paste, window, cx));
    assert_eq!(text(&editor, cx), "- [ ] one\n- [ ] two\n", "the paste has not landed yet");

    // "two" is clicked at the offset it had when drawn; the paste then moves it.
    let two_when_drawn = "- [ ] one\n".len();
    editor.update(cx, |e, cx| e.toggle_task_at(two_when_drawn, cx));
    cx.run_until_parked();

    let after = text(&editor, cx);
    assert_eq!(after, format!("{paste}- [ ] one\n- [x] two\n"), "the box that was clicked toggled");
}

#[gpui::test]
fn the_note_header_toggle_switches_mode_without_moving_the_caret_and_records_it(
    cx: &mut TestAppContext,
) {
    let dir = notes_dir("mode-toggle");
    let (editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(360.)));
    cx.simulate_input("# Title");
    cx.run_until_parked();
    let caret = editor.read_with(cx, |e, _| e.selection());
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Markdown);

    let toggle = cx.debug_bounds("note-mode").expect("the header has a mode toggle");
    cx.simulate_click(toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();

    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain, "the click switched it");
    assert_eq!(text(&editor, cx), "# Title", "the text is untouched");
    assert_eq!(editor.read_with(cx, |e, _| e.selection()), caret, "the caret did not move");
    assert!(!editor.read_with(cx, |e, _| e.selecting), "no selection drag started");

    // A named note's record carries the mode; an unnamed one has no record.
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    let record = editor.read_with(cx, |e, _| e.note_window_state()).expect("a named note");
    assert!(record.plain, "the record says plain text");
    assert_eq!(
        crate::read_notes_state(&{
            let state = dir.join("notes-state.txt");
            crate::write_notes_state(&state, std::slice::from_ref(&record)).unwrap();
            state
        }),
        vec![record],
        "and survives a trip through the state file"
    );

    let toggle = cx.debug_bounds("note-mode").expect("still there");
    cx.simulate_click(toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Markdown, "and back");
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn the_note_header_toggle_label_names_the_mode(cx: &mut TestAppContext) {
    // MD / TXT, so the state is not shown by color alone.
    let dir = notes_dir("mode-label");
    let (editor, cx) = open_note(dir.clone(), cx);
    let label = |cx: &mut VisualTestContext| {
        cx.debug_bounds("note-mode").map(|b| b.size.width).expect("the toggle is drawn")
    };
    let md_width = label(cx);
    editor.update(cx, |e, cx| e.set_note_plain(true, cx));
    cx.run_until_parked();
    let txt_width = label(cx);
    assert_ne!(md_width, txt_width, "the label text changed between MD and TXT");
    std::fs::remove_dir_all(&dir).ok();
}

#[gpui::test]
fn set_note_plain_retags_a_note_and_leaves_big_markdown_alone(cx: &mut TestAppContext) {
    let dir = notes_dir("set-plain");
    let (editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("# kept");
    editor.update(cx, |e, cx| e.set_note_plain(true, cx));
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain);
    assert_eq!(text(&editor, cx), "# kept");
    editor.update(cx, |e, cx| e.set_note_plain(true, cx));
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain, "already plain: a no-op");
    editor.update(cx, |e, cx| e.set_note_plain(false, cx));
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Markdown);
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Heading(1)]);
    std::fs::remove_dir_all(&dir).ok();
}

fn note_mode(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> tachyon_doc::DocMode {
    editor.read_with(cx, |e, _| e.document().mode())
}

#[gpui::test]
fn preferences_step_the_new_note_mode_and_write_it(cx: &mut TestAppContext) {
    let dir = backup_dir("preferences-note-mode");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("settings.toml");
    std::fs::write(&file, "").expect("write");
    cx.update(|cx| cx.set_global(crate::SettingsFile(file.clone())));
    let (_editor, cx) = open("text\n", cx);

    cx.simulate_keystrokes("secondary-,");
    cx.simulate_input("New sticky note mode");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    let settings = cx.update(|_, cx| cx.global::<crate::Settings>().clone());
    assert!(settings.sticky_plain, "markdown -> plain");
    let written = std::fs::read_to_string(&file).expect("read");
    assert!(written.ends_with("sticky_default_mode = \"plain\"\n"), "{written}");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!cx.update(|_, cx| cx.global::<crate::Settings>().clone()).sticky_plain, "and back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn an_external_edit_keeps_a_plain_text_note_plain(cx: &mut TestAppContext) {
    let dir = notes_dir("reload-plain");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Log.md");
    std::fs::write(&path, "# not a heading\n").unwrap();
    let (editor, cx) = open_note(dir.clone(), cx);
    editor.update(cx, |e, cx| {
        e.set_document(tachyon_doc::Document::new("# not a heading\n"), cx);
        e.adopt_note_file(path.clone(), cx);
    });
    // The user switches the note to plain text with the keyboard, as they would.
    cx.simulate_keystrokes("ctrl-shift-m");
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain);

    // Another program rewrites the file; activating the window reloads it. The note's mode is
    // the user's choice and the file is still `.md`, so a reload must not turn it back.
    std::fs::write(&path, "# still not a heading, and longer\n").unwrap();
    refocus(cx);

    assert_eq!(text(&editor, cx), "# still not a heading, and longer\n", "it did reload");
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain, "and stayed plain text");
    let record = editor.read_with(cx, |e, _| e.note_window_state()).expect("a named note");
    assert!(record.plain, "so its record still says plain");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn an_external_edit_keeps_a_markdown_notes_mode_too(cx: &mut TestAppContext) {
    // The control: nothing about a plain-text note's handling changes a Markdown note's reload.
    let dir = notes_dir("reload-markdown");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Doc.md");
    std::fs::write(&path, "# Title\n").unwrap();
    let (editor, cx) = open_note(dir.clone(), cx);
    editor.update(cx, |e, cx| {
        e.set_document(tachyon_doc::Document::new("# Title\n"), cx);
        e.adopt_note_file(path.clone(), cx);
    });
    std::fs::write(&path, "# Longer title\n").unwrap();
    refocus(cx);
    assert_eq!(text(&editor, cx), "# Longer title\n");
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Markdown);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn a_note_plain_only_by_fallback_becomes_markdown_when_its_file_is_repaired(
    cx: &mut TestAppContext,
) {
    let dir = notes_dir("reload-fallback");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Garbled.md");
    // Invalid UTF-8: `load_document` opens it as plain text on its own, not by the user's choice.
    std::fs::write(&path, b"# broken \xff\xfe bytes\n").unwrap();
    let (editor, cx) = open_note(dir.clone(), cx);
    let Ok(crate::LoadOutcome::Loaded(loaded)) =
        crate::load_document(&path, tachyon_md::SoftBreaks::Space)
    else {
        panic!("the file loads");
    };
    editor.update(cx, |e, cx| {
        e.set_loaded(*loaded, cx);
        e.adopt_note_file(path.clone(), cx);
    });
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain, "fell back to plain");
    let record = editor.read_with(cx, |e, _| e.note_window_state()).expect("a named note");
    assert!(!record.plain, "the fallback is not the user's choice, so it is not recorded");

    // The file is repaired elsewhere: it loads as Markdown now, and the note follows it.
    std::fs::write(&path, "# repaired, and longer\n").unwrap();
    refocus(cx);
    assert_eq!(text(&editor, cx), "# repaired, and longer\n", "it did reload");
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Markdown, "and renders as Markdown");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn a_note_the_user_switched_to_plain_stays_plain_and_is_recorded(cx: &mut TestAppContext) {
    let dir = notes_dir("chosen-plain");
    let (editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("# Title");
    cx.simulate_keystrokes("ctrl-shift-m");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Plain);
    assert!(editor.read_with(cx, |e, _| e.note_window_state()).unwrap().plain, "recorded");

    // Switching back is a choice too: it is no longer recorded as plain.
    cx.simulate_keystrokes("ctrl-shift-m");
    cx.run_until_parked();
    assert_eq!(note_mode(&editor, cx), tachyon_doc::DocMode::Markdown);
    assert!(!editor.read_with(cx, |e, _| e.note_window_state()).unwrap().plain);
    let _ = std::fs::remove_dir_all(&dir);
}

fn with_sticky_folder(folder: &std::path::Path, cx: &mut VisualTestContext) {
    let value = folder.display().to_string();
    cx.update(|_, cx| {
        cx.set_global(crate::Settings::parse(&format!("sticky_folder = \"{value}\"\n")).0)
    });
}

fn md_files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[gpui::test]
fn a_new_note_saves_in_the_sticky_folder_and_the_picker_lists_it(cx: &mut TestAppContext) {
    let standard = notes_dir("folder-standard");
    let custom = notes_dir("folder-custom");
    let (editor, cx) = open_note(standard.clone(), cx);
    with_sticky_folder(&custom, cx);
    cx.simulate_input("Shopping list");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();

    assert_eq!(md_files(&custom), vec!["Shopping list.md".to_owned()], "saved in the new folder");
    assert!(md_files(&standard).is_empty(), "and not in the standard one");

    editor.update(cx, |e, cx| e.open_note_picker(cx));
    cx.run_until_parked();
    let listed: Vec<String> = editor.read_with(cx, |e, _| {
        e.picker
            .as_ref()
            .expect("the picker is open")
            .items
            .iter()
            .map(|i| i.label.clone())
            .collect()
    });
    assert_eq!(listed, vec!["Shopping list".to_owned()], "the picker lists the folder that is set");
    let _ = std::fs::remove_dir_all(&standard);
    let _ = std::fs::remove_dir_all(&custom);
}

#[gpui::test]
fn an_existing_note_keeps_its_file_when_the_folder_changes(cx: &mut TestAppContext) {
    let standard = notes_dir("keeps-standard");
    let custom = notes_dir("keeps-custom");
    let (editor, cx) = open_note(standard.clone(), cx);
    cx.simulate_input("First");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    assert_eq!(md_files(&standard), vec!["First.md".to_owned()]);

    // The setting changes while the note is open; its next edit still rewrites its own file.
    with_sticky_folder(&custom, cx);
    cx.simulate_input(" and more");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();

    assert_eq!(std::fs::read_to_string(standard.join("First.md")).unwrap(), "First and more");
    assert!(md_files(&custom).is_empty(), "nothing moved, nothing new in the new folder");
    assert_eq!(
        editor.read_with(cx, |e, _| e.file().map(|p| p.to_path_buf())),
        Some(standard.join("First.md"))
    );
    let _ = std::fs::remove_dir_all(&standard);
    let _ = std::fs::remove_dir_all(&custom);
}

#[gpui::test]
fn an_unwritable_sticky_folder_keeps_the_text_and_says_where_it_failed(cx: &mut TestAppContext) {
    let standard = notes_dir("bad-standard");
    let blocker = notes_dir("bad-blocker");
    std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    // A file where the folder should be: it cannot be created.
    std::fs::write(&blocker, "not a folder").unwrap();
    let (editor, cx) = open_note(standard.clone(), cx);
    with_sticky_folder(&blocker.join("notes"), cx);
    cx.simulate_input("keep me");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();

    assert_eq!(text(&editor, cx), "keep me", "the text is still in the window");
    let notice = editor.read_with(cx, |e, _| e.app_notice.clone()).expect("the failure is shown");
    assert!(notice.contains("could not be saved"), "{notice}");
    assert!(notice.contains("Preferences"), "and says where to fix it: {notice}");
    assert!(editor.read_with(cx, |e, _| e.file().is_none()), "no file was named");
    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_dir_all(&standard);
}

#[gpui::test]
fn the_notes_folder_is_the_setting_when_set_and_the_standard_one_otherwise(
    cx: &mut TestAppContext,
) {
    let standard = notes_dir("which-standard");
    cx.update(|cx| cx.set_global(crate::NotesDir(standard.clone())));
    assert_eq!(cx.update(|cx| crate::notes::notes_folder(cx)), Some(standard.clone()));
    for blank in ["", "   "] {
        cx.update(|cx| {
            cx.set_global(crate::Settings { sticky_folder: blank.to_owned(), ..Default::default() })
        });
        assert_eq!(
            cx.update(|cx| crate::notes::notes_folder(cx)),
            Some(standard.clone()),
            "{blank:?}"
        );
    }
    // Absolute on every platform (a bare `/x` gains a drive letter on Windows).
    let custom = std::env::temp_dir().join("somewhere").join("else");
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            sticky_folder: format!(" {} ", custom.display()),
            ..Default::default()
        })
    });
    assert_eq!(
        cx.update(|cx| crate::notes::notes_folder(cx)),
        Some(custom),
        "trimmed, not expanded"
    );
}

static OPENED_FOLDERS: std::sync::Mutex<Vec<std::path::PathBuf>> =
    std::sync::Mutex::new(Vec::new());

/// How often `dir` was opened. The log is shared by tests running in parallel, so each looks
/// only at its own (unique) folder.
fn opened_count(dir: &std::path::Path) -> usize {
    OPENED_FOLDERS.lock().unwrap().iter().filter(|d| d.as_path() == dir).count()
}

fn record_opened_folder(_: &mut gpui::App, dir: &std::path::Path) {
    OPENED_FOLDERS.lock().unwrap().push(dir.to_path_buf());
}

#[gpui::test]
fn open_sticky_notes_folder_opens_the_folder_that_is_set_and_reports_failure(
    cx: &mut TestAppContext,
) {
    let standard = notes_dir("open-standard");
    let custom = notes_dir("open-custom");
    cx.update(|cx| cx.set_global(crate::OpenFolder(record_opened_folder)));
    let (editor, cx) = open_note(standard.clone(), cx);
    assert!(!standard.exists(), "nothing saved yet, so no folder");

    editor.update(cx, |e, cx| e.open_notes_folder(cx));
    cx.run_until_parked();
    assert!(standard.is_dir(), "the folder exists once there is something to open");
    assert_eq!(opened_count(&standard), 1, "the standard folder");

    // With `sticky_folder` set, it opens that one, not the standard one.
    with_sticky_folder(&custom, cx);
    editor.update(cx, |e, cx| e.open_notes_folder(cx));
    cx.run_until_parked();
    assert!(custom.is_dir());
    assert_eq!(opened_count(&custom), 1);

    // A folder that cannot be created is reported, and nothing is opened.
    let blocker = notes_dir("open-blocker");
    std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    std::fs::write(&blocker, "x").unwrap();
    with_sticky_folder(&blocker.join("sub"), cx);
    editor.update(cx, |e, cx| e.open_notes_folder(cx));
    cx.run_until_parked();
    let notice = editor.read_with(cx, |e, _| e.app_notice.clone()).expect("a notice");
    assert!(notice.starts_with("Could not open"), "{notice}");
    assert_eq!(opened_count(&blocker.join("sub")), 0, "nothing was opened");
    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_dir_all(&standard);
    let _ = std::fs::remove_dir_all(&custom);
}

#[gpui::test]
fn an_existing_note_still_saves_when_the_new_folder_setting_is_unwritable(cx: &mut TestAppContext) {
    let standard = notes_dir("existing-bad-standard");
    let blocker = notes_dir("existing-bad-blocker");
    let (editor, cx) = open_note(standard.clone(), cx);
    cx.simulate_input("Draft");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    assert_eq!(md_files(&standard), vec!["Draft.md".to_owned()]);

    // The folder setting now points somewhere that cannot be created. The note already has a
    // file, so it must keep saving there instead of failing on a folder it does not need.
    std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    std::fs::write(&blocker, "not a folder").unwrap();
    with_sticky_folder(&blocker.join("notes"), cx);
    cx.simulate_input(" v2");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();

    assert_eq!(std::fs::read_to_string(standard.join("Draft.md")).unwrap(), "Draft v2");
    assert!(editor.read_with(cx, |e, _| e.app_notice.is_none()), "no failure was reported");
    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_dir_all(&standard);
}

#[gpui::test]
fn a_note_that_cannot_be_saved_stays_open_instead_of_losing_its_text(cx: &mut TestAppContext) {
    let standard = notes_dir("close-bad-standard");
    let blocker = notes_dir("close-bad-blocker");
    std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    std::fs::write(&blocker, "not a folder").unwrap();
    let (editor, cx) = open_note(standard.clone(), cx);
    with_sticky_folder(&blocker.join("notes"), cx);
    cx.simulate_input("only copy of this text");

    // Closing flushes the note; the flush fails, so the window must stay with its text.
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert!(!cx.windows().is_empty(), "the note did not close");
    assert_eq!(text(&editor, cx), "only copy of this text");
    let notice = editor.read_with(cx, |e, _| e.app_notice.clone()).expect("it says why");
    assert!(notice.contains("could not be saved"), "{notice}");

    // The window-frame close (Alt+F4, the taskbar) is refused the same way.
    assert!(!cx.simulate_close(), "the frame close is refused too");
    assert_eq!(text(&editor, cx), "only copy of this text");

    // Once the folder works, the same note saves and closes normally.
    with_sticky_folder(&standard, cx);
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert!(cx.windows().is_empty(), "closed once it could be saved");
    assert_eq!(md_files(&standard), vec!["only copy of this text.md".to_owned()]);
    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_dir_all(&standard);
}

#[gpui::test]
fn a_save_warning_goes_away_once_the_note_saves(cx: &mut TestAppContext) {
    let standard = notes_dir("stale-standard");
    let blocker = notes_dir("stale-blocker");
    std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    std::fs::write(&blocker, "not a folder").unwrap();
    let (editor, cx) = open_note(standard.clone(), cx);
    with_sticky_folder(&blocker.join("notes"), cx);
    cx.simulate_input("draft");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.app_notice.is_some()), "the failure is shown");

    with_sticky_folder(&standard, cx);
    cx.simulate_input(" two");
    cx.executor().advance_clock(AUTOSAVE_PAUSE);
    cx.run_until_parked();
    assert_eq!(md_files(&standard), vec!["draft two.md".to_owned()], "it saved");
    assert!(editor.read_with(cx, |e, _| e.app_notice.is_none()), "so the warning is gone");
    let _ = std::fs::remove_file(&blocker);
    let _ = std::fs::remove_dir_all(&standard);
}

#[gpui::test]
fn a_relative_sticky_folder_is_made_absolute_so_the_record_survives_a_restart(
    cx: &mut TestAppContext,
) {
    let standard = notes_dir("relative-standard");
    cx.update(|cx| {
        cx.set_global(crate::Settings {
            sticky_folder: "some/relative/notes".to_owned(),
            ..Default::default()
        })
    });
    cx.update(|cx| cx.set_global(crate::NotesDir(standard)));
    let folder = cx.update(|cx| crate::notes::notes_folder(cx)).expect("a folder");
    assert!(folder.is_absolute(), "{folder:?} must not depend on the working directory");
    assert!(folder.ends_with("some/relative/notes"), "{folder:?}");
}

#[gpui::test]
fn open_sticky_notes_folder_does_not_create_the_folder_on_the_ui_thread(cx: &mut TestAppContext) {
    let standard = notes_dir("async-open");
    cx.update(|cx| cx.set_global(crate::OpenFolder(record_opened_folder)));
    let (editor, cx) = open_note(standard.clone(), cx);

    editor.update(cx, |e, cx| e.open_notes_folder(cx));
    // Nothing happens synchronously: creating the folder is left to the background executor.
    assert!(!standard.exists(), "no filesystem work on the UI thread");
    assert_eq!(opened_count(&standard), 0);

    cx.run_until_parked();
    assert!(standard.is_dir(), "created in the background");
    assert_eq!(opened_count(&standard), 1, "then opened");
    let _ = std::fs::remove_dir_all(&standard);
}

#[gpui::test]
fn the_notes_folder_still_opens_when_the_window_that_asked_has_closed(cx: &mut TestAppContext) {
    let standard = notes_dir("open-after-close");
    cx.update(|cx| cx.set_global(crate::OpenFolder(record_opened_folder)));
    let (editor, cx) = open_note(standard.clone(), cx);
    editor.update(cx, |e, cx| e.open_notes_folder(cx));
    // The note is empty, so closing it is accepted at once, before the folder is created.
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    assert_eq!(opened_count(&standard), 1, "opened although its window is gone");
    let _ = std::fs::remove_dir_all(&standard);
}

#[gpui::test]
fn view_mode_refuses_every_route_into_the_text_and_says_why(cx: &mut TestAppContext) {
    let (editor, cx) = open("alpha beta\n\nsecond block\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    assert!(editor.read_with(cx, |e, _| e.locked));
    let original = text(&editor, cx);

    cx.simulate_input("typed");
    cx.simulate_keystrokes("backspace delete enter tab");
    cx.simulate_keystrokes("secondary-a secondary-x");
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".to_owned()));
    cx.simulate_keystrokes("secondary-v");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), original, "no keystroke changed the text");
    assert!(!editor.read_with(cx, |e, _| e.is_modified()), "nothing is unsaved");
    let notice = editor.read_with(cx, |e, _| e.notice.clone()).expect("it explains itself");
    assert!(notice.contains("Ctrl+E"), "{notice}");

    // Find and replace goes through the same gate.
    editor.update(cx, |e, cx| e.replace_selection_with(0..5, "ALPHA", cx));
    assert_eq!(text(&editor, cx), original, "replace is refused too");
}

#[gpui::test]
fn view_mode_leaves_no_block_raw_and_no_caret_even_after_clicks_and_movement(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open("# Title\n\nsome *emphasis* here\n", cx);
    assert!(editor.read_with(cx, |e, _| e.active_block()).is_some(), "editing before");
    cx.simulate_keystrokes("ctrl-e");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), None, "locking un-raws the block");

    // Moving the caret and selecting (for copy) must not bring a raw block back.
    editor.update(cx, |e, cx| {
        e.move_to(12, false, cx);
        e.move_to(20, true, cx);
    });
    cx.simulate_keystrokes("ctrl-end left");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), None);

    // Copy still works on a selection.
    editor.update(cx, |e, cx| {
        e.move_to(0, false, cx);
        e.move_to(7, true, cx);
    });
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some("# Title".to_owned()));

    // Leaving view mode brings editing back at the same caret.
    cx.simulate_keystrokes("ctrl-e");
    assert!(editor.read_with(cx, |e, _| e.active_block()).is_some());
    cx.simulate_input("!");
    assert!(text(&editor, cx).contains('!'), "typing works again");
}

#[gpui::test]
fn view_mode_still_toggles_task_boxes_and_keeps_the_view_locked(cx: &mut TestAppContext) {
    let (editor, cx) = open("intro\n\n- [ ] one\n- [x] two\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    let one = task_offset(&editor, "- [ ] one", cx);
    assert!(editor.update(cx, |e, cx| e.toggle_task_at(one, cx)));
    assert_eq!(text(&editor, cx), "intro\n\n- [x] one\n- [x] two\n");
    assert!(editor.read_with(cx, |e, _| e.locked), "still in view mode");
    assert_eq!(editor.read_with(cx, |e, _| e.active_block()), None, "and still no raw block");
    cx.simulate_input("x");
    assert_eq!(text(&editor, cx), "intro\n\n- [x] one\n- [x] two\n", "typing is still refused");
}

#[gpui::test]
fn a_plain_click_follows_a_link_only_in_view_mode(cx: &mut TestAppContext) {
    let (editor, cx) = open("see [site](https://example.com/page) now\n", cx);
    let at = editor.read_with(cx, |e, _| e.text().find("site").unwrap());
    let click = |editor: &Entity<Editor>, cx: &mut VisualTestContext| {
        editor.update_in(cx, |e, window, cx| {
            e.press_position = Some(gpui::point(gpui::px(10.), gpui::px(10.)));
            e.mouse_down(at, gpui::Modifiers::default(), 1, window, cx);
            e.mouse_released(gpui::point(gpui::px(11.), gpui::px(10.)), cx);
        });
        cx.run_until_parked();
    };
    click(&editor, cx);
    assert_eq!(cx.opened_url(), None, "a plain click in edit mode only places the caret");
    cx.simulate_keystrokes("ctrl-e");
    click(&editor, cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/page"));
}

#[gpui::test]
fn a_note_in_view_mode_is_recorded_restored_and_toggled_from_the_header(cx: &mut TestAppContext) {
    let dir = notes_dir("view-mode");
    let (editor, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("checklist");
    editor.update(cx, |e, cx| {
        e.flush_note(cx);
    });
    let label = |cx: &mut VisualTestContext| cx.debug_bounds("note-view").is_some();
    assert!(label(cx), "the header has the view button");

    let button = cx.debug_bounds("note-view").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    assert!(editor.read_with(cx, |e, _| e.locked), "the header button locks");
    let state = editor.read_with(cx, |e, _| e.note_window_state()).expect("the note has a file");
    assert!(state.locked, "and the record says so");

    cx.simulate_click(button.center(), gpui::Modifiers::default());
    assert!(!editor.read_with(cx, |e, _| e.locked), "a second click unlocks");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn dragging_across_a_link_in_view_mode_selects_it_instead_of_opening_it(cx: &mut TestAppContext) {
    let (editor, cx) = open("see [site](https://example.com/page) now\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    let start = editor.read_with(cx, |e, _| e.text().find("site").unwrap());
    editor.update_in(cx, |e, window, cx| {
        e.press_position = Some(gpui::point(gpui::px(10.), gpui::px(10.)));
        e.mouse_down(start, gpui::Modifiers::default(), 1, window, cx);
        e.mouse_drag(start + 4, cx);
        e.mouse_released(gpui::point(gpui::px(60.), gpui::px(10.)), cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), None, "a drag is not a click");
    assert_eq!(editor.read_with(cx, |e, _| e.selection()), start..start + 4, "it selected");
}

#[gpui::test]
fn view_mode_lets_a_query_be_pasted_into_the_find_bar_and_never_replaces(cx: &mut TestAppContext) {
    let (editor, cx) = open("one two one\n", cx);
    cx.simulate_keystrokes("ctrl-e ctrl-h");
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("one".to_owned()));
    cx.simulate_keystrokes("secondary-v");
    cx.run_until_parked();
    assert!(
        editor.read_with(cx, |e, _| e.find.as_ref().is_some_and(|f| f.query == "one")),
        "the query was pasted into the find bar"
    );
    cx.simulate_keystrokes("tab");
    cx.simulate_input("1");
    cx.simulate_keystrokes("secondary-enter enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "one two one\n", "nothing was replaced");
    assert!(
        editor.read_with(cx, |e, _| e.find.as_ref().is_some_and(|f| f.replaced.is_none())),
        "and the bar does not claim it replaced anything"
    );
}

#[gpui::test]
fn changing_a_locked_notes_mode_keeps_it_free_of_a_caret_and_raw_blocks(cx: &mut TestAppContext) {
    let (editor, cx) = open("# Title\n\ntext\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    for _ in 0..2 {
        cx.simulate_keystrokes("ctrl-shift-m");
        assert!(editor.read_with(cx, |e, _| e.locked));
        assert!(!editor.read_with(cx, |e, _| e.editing), "no caret after the mode change");
        assert_eq!(editor.read_with(cx, |e, _| e.active_block()), None, "and no raw block");
    }
}

#[gpui::test]
fn dragging_off_a_link_into_the_margin_does_not_open_it(cx: &mut TestAppContext) {
    let (editor, cx) = open("see [site](https://example.com/page) now\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    let at = editor.read_with(cx, |e, _| e.text().find("site").unwrap());
    editor.update_in(cx, |e, window, cx| {
        e.press_position = Some(gpui::point(gpui::px(10.), gpui::px(10.)));
        e.mouse_down(at, gpui::Modifiers::default(), 1, window, cx);
        // No text under the pointer on the way out: the selection never grows.
        e.mouse_released(gpui::point(gpui::px(400.), gpui::px(300.)), cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), None, "the pointer left the link before the release");
}

#[gpui::test]
fn a_margin_click_near_a_link_in_view_mode_does_not_open_it(cx: &mut TestAppContext) {
    let (editor, cx) = open("see [site](https://example.com/page) now\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    let at = editor.read_with(cx, |e, _| e.text().find("site").unwrap());
    // `mouse_down_in_margin` maps a click beside the text to a nearby offset and never sets
    // `press_position`: only a press on the text itself can follow a link.
    editor.update_in(cx, |e, window, cx| {
        e.mouse_down(at, gpui::Modifiers::default(), 1, window, cx);
        e.mouse_released(gpui::point(gpui::px(10.), gpui::px(10.)), cx);
    });
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), None);
}

#[gpui::test]
fn a_replace_all_that_finishes_after_locking_is_dropped_and_not_reported(cx: &mut TestAppContext) {
    let line = "127.0.0.1 - - [10/Oct] \"GET /x\" status=200 1234\n";
    let doc = line.repeat(150_000);
    assert!(doc.len() as u64 >= tachyon_doc::FIND_BACKGROUND_THRESHOLD, "scans in the background");
    let (editor, cx) = open(&doc, cx);
    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("status=200");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("status=500");
    // Dispatched directly: `simulate_keystrokes` would run until parked, past the scan.
    let window = cx.window_handle();
    cx.dispatch_keystroke(window, gpui::Keystroke::parse("secondary-enter").unwrap());
    // The scan is in flight; the user locks before it lands.
    editor.update(cx, |e, _| e.locked = true);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.text() == doc), "the text is untouched");
    assert!(
        editor.read_with(cx, |e, _| e.find.as_ref().is_some_and(|f| f.replaced.is_none())),
        "and nothing is reported as replaced"
    );
}

#[gpui::test]
fn a_modifier_click_on_a_link_leaves_no_press_behind_for_a_margin_click(cx: &mut TestAppContext) {
    let (editor, cx) = open("see [site](https://example.com/page) now\n", cx);
    cx.simulate_keystrokes("ctrl-e");
    let at = editor.read_with(cx, |e, _| e.text().find("site").unwrap());
    // A Ctrl+click on the text follows the link at once, from the press itself.
    editor.update_in(cx, |e, window, cx| {
        e.press_position = Some(gpui::point(gpui::px(10.), gpui::px(10.)));
        e.mouse_down(at, gpui::Modifiers::secondary_key(), 1, window, cx);
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/page"), "Ctrl+click opens");
    assert_eq!(
        editor.read_with(cx, |e, _| e.press_position),
        None,
        "and the press position is not left behind for the next click to find"
    );
}

fn note_text_lines(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    editor
        .read_with(cx, |e, _| e.document().blocks().iter().map(|b| b.parsed().ir.lines.len()).sum())
}

#[gpui::test]
fn a_sticky_note_shows_each_typed_line_but_a_document_joins_them(cx: &mut TestAppContext) {
    // The same text, two windows.
    let (document, cx) = open("ABC\nBCC\nEAI\n", cx);
    assert_eq!(note_text_lines(&document, cx), 1, "a document reflows a single newline");

    let dir = notes_dir("soft-breaks");
    let (note, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("ABC");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("BCC");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("EAI");
    assert_eq!(text(&note, cx), "ABC\nBCC\nEAI");
    assert_eq!(note_text_lines(&note, cx), 3, "a note shows the lines as typed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn a_notes_line_breaks_survive_a_reload_a_mode_toggle_and_a_restart(cx: &mut TestAppContext) {
    let dir = notes_dir("soft-breaks-reload");
    let (note, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("one");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("two");
    // Plain and back (retag_to rebuilds the document).
    cx.simulate_keystrokes("ctrl-shift-m ctrl-shift-m");
    assert_eq!(note_text_lines(&note, cx), 2, "still two lines after a mode round trip");

    // A document loaded into the note (a restore reading its file) gets the same treatment.
    let loaded = tachyon_doc::Document::new("alpha\nbeta\ngamma\n");
    note.update(cx, |e, cx| e.set_document(loaded, cx));
    cx.run_until_parked();
    assert_eq!(note_text_lines(&note, cx), 3, "set_document applies it to a loaded file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn shift_enter_inserts_a_backslash_hard_break_that_renders_as_a_new_line(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    cx.simulate_input("first");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("second");
    assert_eq!(text(&editor, cx), "first\\\nsecond", "the source shows the break");
    assert_eq!(note_text_lines(&editor, cx), 2, "a document breaks the line, unlike Enter");

    cx.simulate_keystrokes("enter");
    cx.simulate_input("third");
    assert_eq!(text(&editor, cx), "first\\\nsecond\nthird");
    assert_eq!(note_text_lines(&editor, cx), 2, "Enter in a document still joins");
}

#[gpui::test]
fn shift_enter_is_a_plain_newline_where_a_backslash_would_be_text(cx: &mut TestAppContext) {
    // Plain-text mode.
    let (editor, cx) = open("", cx);
    cx.simulate_keystrokes("ctrl-shift-m");
    cx.simulate_input("a");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("b");
    assert_eq!(text(&editor, cx), "a\nb", "no backslash in plain text");

    // A code block, and a line with nothing before the caret.
    let (editor, cx) = open("```\ncode", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    assert_eq!(text(&editor, cx), "```\ncode\n", "no backslash inside a fence");
    let (editor, cx) = open("text\n", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    assert_eq!(text(&editor, cx), "text\n\n", "nothing before the caret to break");
}

#[gpui::test]
fn shift_enter_in_a_list_item_continues_the_item_instead_of_starting_one(cx: &mut TestAppContext) {
    let (editor, cx) = open("- one", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    cx.simulate_input("more");
    assert_eq!(text(&editor, cx), "- one\\\n  more", "indented to the item's text, no new bullet");
    let kinds = editor.read_with(cx, |e, _| e.document().blocks().len());
    assert_eq!(kinds, 1, "still one list block");

    let (editor, cx) = open("1. one", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    cx.simulate_input("more");
    assert_eq!(text(&editor, cx), "1. one\\\n   more", "aligned past `1. `");

    let (editor, cx) = open("- [ ] task", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    cx.simulate_input("more");
    assert_eq!(text(&editor, cx), "- [ ] task\\\n      more", "aligned past the box");
}

#[gpui::test]
fn copy_as_html_from_a_note_follows_the_spec_even_though_the_note_shows_each_line(
    cx: &mut TestAppContext,
) {
    let dir = notes_dir("soft-breaks-html");
    let (note, cx) = open_note(dir.clone(), cx);
    cx.simulate_input("ABC");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("BCC");
    assert_eq!(note_text_lines(&note, cx), 2, "shown as two lines");
    cx.simulate_keystrokes("secondary-shift-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("<p>ABC\nBCC</p>\n"), "exported as CommonMark");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_loaded_under_the_notes_policy_needs_no_conversion_on_the_ui_thread() {
    // `with_soft_breaks` rebuilds the document when the policy differs and is a no-op when it
    // already matches, so a note loaded off the UI thread under `Line` installs without a
    // second parse. Compared by identity of the block list, which a rebuild replaces.
    let doc =
        tachyon_doc::Document::new("one\ntwo\n").with_soft_breaks(tachyon_md::SoftBreaks::Line);
    let before = doc.blocks().as_ptr();
    let doc = doc.with_soft_breaks(tachyon_md::SoftBreaks::Line);
    assert_eq!(doc.blocks().as_ptr(), before, "same policy: no rebuild");
    let doc = doc.with_soft_breaks(tachyon_md::SoftBreaks::Space);
    assert_ne!(doc.blocks().as_ptr(), before, "a different policy does rebuild");
}

#[gpui::test]
fn load_document_builds_a_note_file_under_the_policy_it_is_given(cx: &mut TestAppContext) {
    let dir = notes_dir("load-policy");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("n.md");
    std::fs::write(&path, "ABC\nBCC\nEAI\n").unwrap();
    for (breaks, lines) in [(tachyon_md::SoftBreaks::Line, 3), (tachyon_md::SoftBreaks::Space, 1)] {
        let crate::LoadOutcome::Loaded(loaded) = crate::load_document(&path, breaks).unwrap()
        else {
            panic!("loads");
        };
        assert_eq!(loaded.doc.soft_breaks(), breaks);
        let rendered: usize = loaded.doc.blocks().iter().map(|b| b.parsed().ir.lines.len()).sum();
        assert_eq!(rendered, lines, "{breaks:?}");
    }
    // The note window then keeps that document as is.
    let (note, cx) = open_note(dir.clone(), cx);
    let crate::LoadOutcome::Loaded(loaded) =
        crate::load_document(&path, tachyon_md::SoftBreaks::Line).unwrap()
    else {
        panic!("loads");
    };
    note.update(cx, |e, cx| e.set_loaded(*loaded, cx));
    assert_eq!(note_text_lines(&note, cx), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn shift_enter_after_a_backslash_uses_trailing_spaces_so_the_break_still_counts(
    cx: &mut TestAppContext,
) {
    // One backslash already there: another would make an escaped pair, not a break.
    let (editor, cx) = open("path\\", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    cx.simulate_input("next");
    assert_eq!(text(&editor, cx), "path\\  \nnext");
    assert_eq!(note_text_lines(&editor, cx), 2, "rendered as a hard break");

    // Two backslashes (an escaped backslash): a third is a real break marker.
    let (editor, cx) = open("a\\\\", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    cx.simulate_input("b");
    assert_eq!(text(&editor, cx), "a\\\\\\\nb");
    assert_eq!(note_text_lines(&editor, cx), 2);

    // The same inside a list item.
    let (editor, cx) = open("- item\\", cx);
    cx.simulate_keystrokes("ctrl-end shift-enter");
    cx.simulate_input("more");
    assert_eq!(text(&editor, cx), "- item\\  \n  more");
    assert_eq!(note_text_lines(&editor, cx), 2);
}

#[gpui::test]
fn shift_enter_in_a_block_awaiting_its_parse_still_inserts_a_hard_break(cx: &mut TestAppContext) {
    let (editor, cx) = open("", cx);
    // A paste big enough to leave an `Unparsed` placeholder until its background parse lands.
    let big = format!("hello {}\n", "word ".repeat(tachyon_doc::UNPARSED_SPLIT_THRESHOLD / 5 + 10));
    editor.update(cx, |e, cx| {
        e.replace_selection_with(0..0, &big, cx);
    });
    let at = editor.read_with(cx, |e, _| e.text().find("hello").unwrap() + "hello".len());
    let pending = editor.read_with(cx, |e, _| {
        matches!(
            e.document().blocks()[e.document().block_at(at).unwrap()].parsed().kind,
            BlockKind::Unparsed
        )
    });
    editor.update(cx, |e, cx| {
        e.move_to(at, false, cx);
    });
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    let source = text(&editor, cx);
    assert!(
        source.starts_with("hello\\\n"),
        "a hard break, not a plain newline (pending={pending})"
    );
}

fn plain_json_editor<'a>(
    name: &str,
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
    let (editor, cx) = open("", cx);
    let path = std::env::temp_dir().join(format!("tachyon-colour-{}-{name}", std::process::id()));
    editor.update(cx, |e, cx| {
        e.set_document(tachyon_doc::Document::new_plain(text), cx);
        e.set_file(path, cx);
    });
    cx.run_until_parked();
    (editor, cx)
}

#[gpui::test]
fn only_json_files_in_plain_mode_are_coloured_and_only_up_to_the_size_limit(
    cx: &mut TestAppContext,
) {
    for (name, expected) in [
        ("a.json", Some("json")),
        ("a.JSON", Some("json")),
        ("a.jsonc", Some("jsonc")),
        ("a.json5", Some("json5")),
        ("a.txt", None),
        ("a.log", None),
        ("a.toml", None),
        ("json", None),
    ] {
        let (editor, cx) = plain_json_editor(name, "{\"a\": 1}\n", cx);
        let got = editor.read_with(cx, |e, _| e.plain_language());
        assert_eq!(got, expected, "{name}");
    }
    let big = format!("[{}]\n", "1,".repeat(crate::render::COLOUR_LIMIT / 2));
    let (editor, cx) = plain_json_editor("big.json", &big, cx);
    assert!(big.len() > crate::render::COLOUR_LIMIT);
    assert_eq!(editor.read_with(cx, |e, _| e.plain_language()), None, "over the limit: unstyled");
}

#[gpui::test]
fn a_json_file_in_plain_mode_still_draws_and_edits_as_plain_text(cx: &mut TestAppContext) {
    let (editor, cx) =
        plain_json_editor("draw.json", "{\n  \"name\": \"tachyon\",\n  \"n\": 42\n}\n", cx);
    assert_eq!(editor.read_with(cx, |e, _| e.document().mode()), tachyon_doc::DocMode::Plain);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("// end");
    cx.run_until_parked();
    assert!(text(&editor, cx).ends_with("}\n// end"), "typing into coloured text works");
    assert_eq!(note_text_lines(&editor, cx), 0, "plain blocks carry no rendered IR");
}

#[gpui::test]
fn only_a_chunk_that_starts_a_line_is_coloured_so_a_forced_cut_inside_a_string_stays_plain(
    cx: &mut TestAppContext,
) {
    // One line far longer than the forced-cut size: plain chunking cuts it inside itself.
    let long = format!("{{\"k\": \"{}\", \"n\": 1}}\n", "a".repeat(30_000));
    let (editor, cx) = plain_json_editor("long.json", &long, cx);
    let starts: Vec<usize> = editor.read_with(cx, |e, _| {
        (0..e.document().blocks().len()).map(|i| e.document().block_range(i).start).collect()
    });
    assert!(starts.len() > 2, "the line was cut into several chunks: {starts:?}");
    let langs: Vec<_> =
        starts.iter().map(|&s| editor.read_with(cx, |e, _| e.chunk_language(s))).collect();
    assert_eq!(langs[0], Some("json"), "the chunk holding the line start is coloured");
    assert!(langs[1..].iter().all(Option::is_none), "chunks beginning mid-line are not: {langs:?}");

    // Ordinary multi-line JSON: every chunk starts a line, so every chunk is coloured.
    let lines = "{\"a\": 1}\n".repeat(5_000);
    let (editor, cx) = plain_json_editor("lines.json", &lines, cx);
    let all = editor.read_with(cx, |e, _| {
        (0..e.document().blocks().len())
            .map(|i| e.chunk_language(e.document().block_range(i).start))
            .collect::<Vec<_>>()
    });
    assert!(all.len() > 1 && all.iter().all(|l| *l == Some("json")), "{all:?}");
}
