//! Cheap line scan that splits large inserted text into provisional chunks
//! before the real parse has run. Boundaries fall after blank lines outside
//! fenced code and outside HTML blocks, so a pasted code block with blank
//! lines - or one swallowed by an open HTML block - stays in one chunk.
//! Only display and parse scheduling use these boundaries; the parse decides
//! the real block structure.

/// Byte offsets (line starts) where a provisional chunk may begin, excluding
/// 0 and `src.len()`, in increasing order.
pub fn presegment(src: &str) -> Vec<usize> {
    let mut boundaries = Vec::new();
    scan(src, Some(&mut boundaries));
    boundaries
}

/// Whether `src` ends inside a fenced code block it opened, or inside an HTML block (`<div>`
/// and friends) still swallowing lines because no blank line has closed it yet. Either way, text
/// appended after such a `src` cannot use boundaries computed for it alone: the appended text's
/// own presegmenter run has no idea it is still inside a construct that started earlier.
pub fn ends_in_fence(src: &str) -> bool {
    let (fence, in_html) = scan(src, None);
    fence.is_some() || in_html
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

/// Chunked [`ends_in_fence`]: whether `pieces` joined together end inside a fence or an HTML
/// block it opened.
pub fn ends_in_fence_chunks<'a>(pieces: impl Iterator<Item = &'a str>, base: usize) -> bool {
    let (fence, in_html) = scan_chunks(pieces, base, None);
    fence.is_some() || in_html
}

/// The fence/HTML-block/blank-line state one line scan step needs to remember between lines,
/// shared by [`scan`] (fed whole lines from a contiguous `&str`) and [`scan_chunks`] (fed lines
/// assembled from a fragment stream), so the two can never disagree about what counts as a
/// boundary.
#[derive(Default)]
struct LineScan {
    fence: Option<(u8, usize)>,
    /// Set while swallowing an HTML block's lines (CommonMark type 6/7), which - like a fence,
    /// unlike everything else - does not end at the next line, only at the next blank one. See
    /// [`opens_html_block`] for why this needs its own tracking distinct from `fence`.
    in_html: bool,
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
                if self.in_html {
                    // A blank line closes the HTML block (CommonMark type 6/7); everything up
                    // to it - including anything that looks like a fence marker - is swallowed
                    // as literal HTML, never real Markdown, so none of the checks below apply.
                    if blank {
                        self.in_html = false;
                        self.previous_blank = true;
                    }
                    return;
                }
                if self.previous_blank
                    && !blank
                    && start > 0
                    && let Some(boundaries) = boundaries
                {
                    boundaries.push(start);
                }
                if !blank && opens_html_block(line) {
                    self.in_html = true;
                    self.previous_blank = false;
                    return;
                }
                if matches!(first, Some(b'`' | b'~')) {
                    self.fence = opens_fence(line);
                }
                self.previous_blank = blank;
            }
        }
    }
}

/// Scans `src` line by line, pushing chunk boundaries into `boundaries` if given; returns the
/// fence still open at the end, and whether the scan ends inside an HTML block still swallowing
/// lines.
fn scan(src: &str, mut boundaries: Option<&mut Vec<usize>>) -> (Option<(u8, usize)>, bool) {
    let mut state = LineScan::default();
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        state.step(line, offset, boundaries.as_deref_mut());
        offset += line.len();
    }
    (state.fence, state.in_html)
}

/// [`scan`] fed from a fragment stream instead of one contiguous `&str`: see
/// [`presegment_chunks`]. A line is scanned straight from whichever piece holds it whole; only a
/// line split across pieces is assembled into `buf` (cleared after each use) first.
fn scan_chunks<'a>(
    pieces: impl Iterator<Item = &'a str>,
    base: usize,
    mut boundaries: Option<&mut Vec<usize>>,
) -> (Option<(u8, usize)>, bool) {
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
    (state.fence, state.in_html)
}

/// Whether `line` looks like the start of an HTML block (CommonMark type 6/7): after up to 3
/// spaces of indent, `<` optionally followed by `/`, then an ASCII letter. Loose on purpose: it
/// does not check the tag name, closing `>`, or that it is not interrupting a paragraph the way
/// CommonMark's real grammar does, only enough to keep [`LineScan`]'s fence tracking from being
/// fooled by a line a real parse would swallow as HTML - a `` ``` ``/`~~~` line inside an open
/// `<div>` block (`<div>\n[^1]: note\n~~~\n...`, none of it real Markdown) is not a real fence
/// marker, and without this, [`LineScan`]'s fence state can desync from the real parser's: a
/// later boundary placed while it wrongly believes no fence is open can land inside a fence that
/// is genuinely still open, which a downstream window then parses as if it were fresh top-level
/// text (`Document::follows_blank_line` trusts a presegment boundary's raw blank line alone, by
/// design - see its own doc comment). Over-recognizing a line as "maybe HTML" only suppresses
/// fence/boundary detection for a few extra lines (still always safe, never an unsafe cut), and
/// a real fence marker never starts with `<`. `pub`: `tachyon-doc`'s `block_shape` also needs to
/// tell an HTML-opening line apart from a fence-opening one (unlike a fence, an HTML block ends
/// at the next blank line, so a bounded window around an edit elsewhere in the same block cannot
/// assume no new boundary can appear there the way it can for a fence).
pub fn opens_html_block(line: &str) -> bool {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return false;
    }
    let mut bytes = line[indent..].bytes();
    if bytes.next() != Some(b'<') {
        return false;
    }
    let next = bytes.next();
    let tag_start = if next == Some(b'/') { bytes.next() } else { next };
    tag_start.is_some_and(|b| b.is_ascii_alphabetic())
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

    #[test]
    fn a_fence_marker_swallowed_by_an_html_block_does_not_toggle_fence_state() {
        // The first `~~~` is literal content of the `<div>` HTML block (which itself ends at
        // the blank line after `</div>`), not a real fence marker; the second `~~~`, after that
        // blank line, is a genuine fence that nothing here closes.
        let src = "<div>\n~~~\n</div>\n\n~~~\ncode\n\nmore code\n";
        let real_fence = src.rfind("~~~").unwrap();
        assert!(ends_in_fence(src), "the real, second ~~~ opens a fence nothing here closes");
        assert_eq!(
            presegment(src),
            vec![real_fence],
            "cuts right before the real fence, never inside it (the blank line before `code` and \
             `more code` is itself inside the still-open fence)"
        );
    }

    #[test]
    fn an_html_block_ends_at_the_next_blank_line() {
        let src = "<div>\nnot a fence marker: ~~~\n</div>\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);
        assert!(!ends_in_fence(src));
    }

    #[test]
    fn an_unclosed_html_block_is_reported_like_an_unclosed_fence() {
        assert!(ends_in_fence("text\n\n<div>\nstill open\n"));
        assert!(!ends_in_fence("text\n\n<div>\nclosed\n\nafter\n"));
    }

    #[test]
    fn only_a_leading_angle_bracket_suppresses_fence_tracking() {
        // "< 3" is not a tag: it must not swallow the fence that follows it.
        let src = "< 3\n\n~~~\ncode\n\nmore\n";
        assert!(ends_in_fence(src));
    }

    /// [`presegment_chunks`]/[`ends_in_fence_chunks`] agree with the contiguous scan regardless
    /// of where the input happens to be split into pieces - including splits that land inside a
    /// line, inside a fence delimiter run, inside an HTML block, and exactly on a line boundary.
    #[test]
    fn chunked_scan_matches_contiguous_scan_at_every_split() {
        let src = "# Title\n\npara one\nstill ```one\n\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\n\
                   <div>\n~~~\n</div>\n\n~~~\nreal fence\n\nmore\n\nafter\n";
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

        for unclosed in ["text\n\n```rust\nlet x = 1;\n\n", "text\n\n<div>\nstill open\n"] {
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
