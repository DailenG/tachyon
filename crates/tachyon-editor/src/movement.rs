//! Offset arithmetic over the document rope: grapheme and line boundaries.
//! Pure functions so they are testable without a window.

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

fn line_start(rope: &Rope, offset: usize) -> usize {
    rope.line_to_byte(rope.byte_to_line(offset))
}

/// End of the line containing `offset`, before its `\n`.
fn line_end(rope: &Rope, offset: usize) -> usize {
    let line = rope.byte_to_line(offset);
    if line + 1 < rope.len_lines() { rope.line_to_byte(line + 1) - 1 } else { rope.len_bytes() }
}

/// Start of the grapheme before `offset`; a line start moves onto the
/// previous line's end.
pub fn prev_grapheme(rope: &Rope, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let start = line_start(rope, offset);
    if offset == start {
        return offset - 1;
    }
    let text = rope.byte_slice(start..offset).to_string();
    text.grapheme_indices(true).next_back().map_or(start, |(i, _)| start + i)
}

/// End of the grapheme after `offset`; a line end moves onto the next line.
pub fn next_grapheme(rope: &Rope, offset: usize) -> usize {
    let len = rope.len_bytes();
    if offset >= len {
        return len;
    }
    let end = line_end(rope, offset);
    if offset == end {
        return offset + 1;
    }
    let text = rope.byte_slice(offset..end).to_string();
    text.graphemes(true).next().map_or(end, |g| offset + g.len())
}

/// Start of the word before `offset` (skipping whitespace first).
pub fn prev_word(rope: &Rope, offset: usize) -> usize {
    let start = line_start(rope, offset);
    if offset == start {
        return prev_grapheme(rope, offset);
    }
    let text = rope.byte_slice(start..offset).to_string();
    text.unicode_word_indices().next_back().map_or(start, |(i, _)| start + i)
}

/// End of the word after `offset`.
pub fn next_word(rope: &Rope, offset: usize) -> usize {
    let end = line_end(rope, offset);
    if offset == end {
        return next_grapheme(rope, offset);
    }
    let text = rope.byte_slice(offset..end).to_string();
    text.unicode_word_indices().next().map_or(end, |(i, w)| offset + i + w.len())
}

pub fn home(rope: &Rope, offset: usize) -> usize {
    line_start(rope, offset)
}

pub fn end(rope: &Rope, offset: usize) -> usize {
    line_end(rope, offset)
}

/// `offset` moved `delta` source lines up (negative) or down, keeping the
/// byte column where possible (snapped to a char boundary).
pub fn vertical(rope: &Rope, offset: usize, delta: isize) -> usize {
    let line = rope.byte_to_line(offset);
    let column = offset - rope.line_to_byte(line);
    let target = line as isize + delta;
    if target < 0 {
        return 0;
    }
    let target = target as usize;
    if target >= rope.len_lines() {
        return rope.len_bytes();
    }
    let start = rope.line_to_byte(target);
    let end = line_end(rope, start);
    let mut at = (start + column).min(end);
    while rope.char_to_byte(rope.byte_to_char(at)) != at {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes_cross_lines_and_keep_clusters_whole() {
        // "e" + combining acute is one grapheme; the flag is two code points.
        let rope = Rope::from_str("ae\u{301}\n🇨🇦x");
        let flag_start = "ae\u{301}\n".len();
        assert_eq!(next_grapheme(&rope, 1), 1 + "e\u{301}".len());
        assert_eq!(prev_grapheme(&rope, 1 + "e\u{301}".len()), 1);
        assert_eq!(next_grapheme(&rope, flag_start - 1), flag_start, "over the newline");
        assert_eq!(next_grapheme(&rope, flag_start), flag_start + "🇨🇦".len());
        assert_eq!(prev_grapheme(&rope, flag_start), flag_start - 1);
        assert_eq!(prev_grapheme(&rope, 0), 0);
        assert_eq!(next_grapheme(&rope, rope.len_bytes()), rope.len_bytes());
    }

    #[test]
    fn words_home_end() {
        let rope = Rope::from_str("one two  three\nnext");
        assert_eq!(next_word(&rope, 0), 3);
        assert_eq!(next_word(&rope, 3), 7);
        assert_eq!(prev_word(&rope, 14), 9);
        assert_eq!(prev_word(&rope, 9), 4);
        assert_eq!((home(&rope, 6), end(&rope, 6)), (0, 14));
        assert_eq!((home(&rope, 17), end(&rope, 17)), (15, 19));
    }

    #[test]
    fn vertical_keeps_column_and_clamps() {
        let rope = Rope::from_str("long line\nab\nçelse");
        assert_eq!(vertical(&rope, 7, 1), 12, "clamped to end of the short line");
        assert_eq!(vertical(&rope, 11, 1), 13, "column 1 lands inside 'ç' and snaps back");
        assert_eq!(vertical(&rope, 2, -1), 0);
        assert_eq!(vertical(&rope, 14, 5), rope.len_bytes());
    }
}
