//! Cheap line scan that splits large inserted text into provisional chunks
//! before the real parse has run. Boundaries fall after blank lines outside
//! fenced code, so a pasted code block with blank lines stays in one chunk.
//! Only display and parse scheduling use these boundaries; the parse decides
//! the real block structure.

/// Byte offsets (line starts) where a provisional chunk may begin, excluding
/// 0 and `src.len()`, in increasing order.
pub fn presegment(src: &str) -> Vec<usize> {
    let mut boundaries = Vec::new();
    scan(src, Some(&mut boundaries));
    boundaries
}

/// Whether `src` ends inside a fenced code block that it opened. Text
/// appended after such a `src` cannot use boundaries computed for it alone.
pub fn ends_in_fence(src: &str) -> bool {
    scan(src, None).is_some()
}

/// Same boundaries as [`presegment`], but fed from `pieces` (any sequence of fragments not
/// necessarily aligned to line ends - a rope's own chunk iterator, so a caller never has to copy
/// a large range into one contiguous `String` first just to hand this a `&str`: that copy-and-
/// scan, repeated on every keystroke into a many-megabyte block with no interior boundary to
/// reuse, is `Document::stale_block`'s dominant per-keystroke cost on such a block). `base` is
/// added to every returned offset, so they mean the same thing as `presegment`'s would on
/// `pieces` joined into one string, in the caller's own coordinates.
///
/// A line entirely inside one piece - the common case, since a rope chunk is almost always
/// longer than one line of real text - is scanned from a borrow of that piece; only a line
/// straddling two or more pieces is assembled into a small reused buffer first, so the total
/// extra copying is bounded by how many lines actually cross a chunk boundary, not by `pieces`'
/// combined length.
pub fn presegment_chunks<'a>(pieces: impl Iterator<Item = &'a str>, base: usize) -> Vec<usize> {
    let mut boundaries = Vec::new();
    scan_chunks(pieces, base, Some(&mut boundaries));
    boundaries
}

/// Chunked [`ends_in_fence`]: whether `pieces` joined together end inside a fence it opened.
pub fn ends_in_fence_chunks<'a>(pieces: impl Iterator<Item = &'a str>, base: usize) -> bool {
    scan_chunks(pieces, base, None).is_some()
}

/// The fence/blank-line state one line scan step needs to remember between lines, shared by
/// [`scan`] (fed whole lines from a contiguous `&str`) and [`scan_chunks`] (fed lines assembled
/// from a fragment stream), so the two can never disagree about what counts as a boundary.
#[derive(Default)]
struct LineScan {
    fence: Option<(u8, usize)>,
    previous_blank: bool,
}

impl LineScan {
    /// Advances the scan by one complete line (including its trailing `\n`, except possibly for
    /// a final line at the end of the text), starting at absolute offset `start`, pushing a
    /// boundary into `boundaries` if this line qualifies. `start == 0` can never qualify (there
    /// is no previous line yet to have been blank), so this needs no separate check for it.
    fn step(&mut self, line: &str, start: usize, boundaries: Option<&mut Vec<usize>>) {
        match self.fence {
            Some((ch, len)) => {
                if closes_fence(line, ch, len) {
                    self.fence = None;
                }
                self.previous_blank = false;
            }
            None => {
                // Byte checks: this runs over every line of a large paste.
                let first = line.bytes().find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
                let blank = first.is_none();
                if self.previous_blank
                    && !blank
                    && start > 0
                    && let Some(boundaries) = boundaries
                {
                    boundaries.push(start);
                }
                if matches!(first, Some(b'`' | b'~')) {
                    self.fence = opens_fence(line);
                }
                self.previous_blank = blank;
            }
        }
    }
}

/// Scans `src` line by line, pushing chunk boundaries into `boundaries` if
/// given; returns the fence still open at the end.
fn scan(src: &str, mut boundaries: Option<&mut Vec<usize>>) -> Option<(u8, usize)> {
    let mut state = LineScan::default();
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        state.step(line, offset, boundaries.as_deref_mut());
        offset += line.len();
    }
    state.fence
}

/// [`scan`] fed from a fragment stream instead of one contiguous `&str`: see
/// [`presegment_chunks`]. A line is scanned straight from whichever piece holds it whole; only a
/// line split across pieces is assembled into `buf` (cleared after each use) first.
fn scan_chunks<'a>(
    pieces: impl Iterator<Item = &'a str>,
    base: usize,
    mut boundaries: Option<&mut Vec<usize>>,
) -> Option<(u8, usize)> {
    let mut state = LineScan::default();
    let mut buf = String::new();
    let mut line_start = base;
    for piece in pieces {
        let mut rest = piece;
        while let Some(i) = rest.find('\n') {
            let line_len = if buf.is_empty() {
                let line = &rest[..=i];
                state.step(line, line_start, boundaries.as_deref_mut());
                line.len()
            } else {
                buf.push_str(&rest[..=i]);
                state.step(&buf, line_start, boundaries.as_deref_mut());
                let len = buf.len();
                buf.clear();
                len
            };
            line_start += line_len;
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            buf.push_str(rest);
        }
    }
    if !buf.is_empty() {
        state.step(&buf, line_start, boundaries);
    }
    state.fence
}

/// `(fence char, fence length)` if `line` opens a fenced code block.
fn opens_fence(line: &str) -> Option<(u8, usize)> {
    let (ch, len, rest) = fence_run(line)?;
    // A backtick fence's info string may not contain backticks.
    if ch == b'`' && rest.contains('`') {
        return None;
    }
    Some((ch, len))
}

fn closes_fence(line: &str, ch: u8, open_len: usize) -> bool {
    matches!(fence_run(line), Some((c, len, rest)) if c == ch && len >= open_len && rest.trim().is_empty())
}

/// A run of at least three backticks or tildes after at most three spaces.
fn fence_run(line: &str) -> Option<(u8, usize, &str)> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let ch = *body.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = body.bytes().take_while(|&b| b == ch).count();
    (len >= 3).then(|| (ch, len, &body[len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_fence_left_open() {
        assert!(ends_in_fence("text\n\n```rust\nlet x = 1;\n\n"));
        assert!(!ends_in_fence("```\ncode\n```\n\ntext\n"));
        assert!(!ends_in_fence("inline ``` is not a fence\n"));
        assert!(ends_in_fence("~~~~\n```\n"), "a shorter or different fence does not close it");
    }

    #[test]
    fn splits_after_blank_lines() {
        let src = "# Title\n\npara one\nstill one\n\n\n- item\n";
        assert_eq!(presegment(src), vec![9, 30]);
        assert_eq!(&src[9..], "para one\nstill one\n\n\n- item\n");
    }

    #[test]
    fn never_splits_inside_fenced_code() {
        let src = "```rust\nfn a() {}\n\nfn b() {}\n```\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);
    }

    #[test]
    fn fence_closes_only_with_same_char_and_sufficient_length() {
        let src = "````\n```\n\n~~~~\n\n````\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);
    }

    #[test]
    fn unclosed_fence_swallows_the_rest() {
        assert_eq!(presegment("text\n\n~~~\ncode\n\nmore code\n"), vec![6]);
    }

    #[test]
    fn indented_or_inline_backticks_do_not_open_fences() {
        let src = "    ```\n\nnot code\n\n``a`` inline\n\nend\n";
        assert_eq!(presegment(src).len(), 3);
    }

    /// [`presegment_chunks`]/[`ends_in_fence_chunks`] agree with the contiguous scan regardless
    /// of where the input happens to be split into pieces - including splits that land inside a
    /// line, inside a fence delimiter run, and exactly on a line boundary.
    #[test]
    fn chunked_scan_matches_contiguous_scan_at_every_split() {
        let src = "# Title\n\npara one\nstill ```one\n\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\nafter\n";
        let want = presegment(src);
        for split in 0..=src.len() {
            if !src.is_char_boundary(split) {
                continue;
            }
            let (a, b) = src.split_at(split);
            let got = presegment_chunks([a, b].into_iter(), 0);
            assert_eq!(got, want, "split at {split}");
        }
        // Split into many small (even mid-line) pieces at once.
        let tiny: Vec<&str> = {
            let mut pieces = Vec::new();
            let mut at = 0;
            while at < src.len() {
                let end = (at + 3).min(src.len());
                let end = (0..=end).rev().find(|&e| src.is_char_boundary(e)).unwrap();
                pieces.push(&src[at..end]);
                at = end;
            }
            pieces
        };
        assert_eq!(presegment_chunks(tiny.into_iter(), 0), want);

        let unclosed = "text\n\n```rust\nlet x = 1;\n\n";
        for split in 0..=unclosed.len() {
            if !unclosed.is_char_boundary(split) {
                continue;
            }
            let (a, b) = unclosed.split_at(split);
            assert_eq!(
                ends_in_fence_chunks([a, b].into_iter(), 0),
                ends_in_fence(unclosed),
                "split at {split}"
            );
        }
    }

    /// `base` shifts every returned boundary by exactly that much, and does not itself change
    /// which lines qualify (the redundant `start == 0` exclusion in [`LineScan::step`] talks
    /// about the scan's own first line, not the caller's absolute coordinate space).
    #[test]
    fn chunked_scan_base_offset_shifts_boundaries() {
        let src = "para one\nstill one\n\n\n- item\n";
        let base = 1_000;
        let want: Vec<usize> = presegment(src).into_iter().map(|b| b + base).collect();
        assert_eq!(presegment_chunks([src].into_iter(), base), want);
    }
}
