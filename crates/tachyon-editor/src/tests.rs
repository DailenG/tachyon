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
