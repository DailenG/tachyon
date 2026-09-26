use proptest::prelude::*;
use tachyon_text::Buffer;

#[derive(Clone, Debug)]
enum Op {
    Edit { start: usize, len: usize, text: String },
    Seal,
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (any::<usize>(), 0usize..8, "[a-c\n é😀\r]{0,6}")
            .prop_map(|(start, len, text)| Op::Edit { start, len, text }),
        1 => Just(Op::Seal),
        1 => Just(Op::Undo),
        1 => Just(Op::Redo),
    ]
}

/// Snaps a byte offset down to a char boundary of `text`.
fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

proptest! {
    /// Undoing everything restores the original text, redoing everything
    /// restores the newest text, and every step's logged edits account for
    /// the change in length.
    #[test]
    fn undo_redo_and_edit_log_are_consistent(
        initial in "[a-c\n é😀]{0,12}",
        ops in prop::collection::vec(op(), 0..40),
    ) {
        let mut buffer = Buffer::new(&initial);
        let original = buffer.text();

        for op in ops {
            let (version, len) = (buffer.version(), buffer.len());
            match op {
                Op::Edit { start, len, text } => {
                    let current = buffer.text();
                    let start = floor_boundary(&current, start % (current.len() + 1));
                    let end = floor_boundary(&current, start + len).max(start);
                    buffer.edit(start..end, &text).unwrap();
                }
                Op::Seal => buffer.seal_undo_group(),
                Op::Undo => { buffer.undo(); }
                Op::Redo => { buffer.redo(); }
            }
            let delta: isize = buffer
                .edits_since(version)
                .unwrap()
                .map(|e| e.new_len as isize - e.range.len() as isize)
                .sum();
            prop_assert_eq!(buffer.len() as isize, len as isize + delta);
        }
        // Redo whatever the ops left undone to reach the newest state.
        while buffer.redo().is_some() {}
        let newest = buffer.text();

        while buffer.undo().is_some() {}
        prop_assert_eq!(buffer.text(), original);
        while buffer.redo().is_some() {}
        prop_assert_eq!(buffer.text(), newest);
    }
}
