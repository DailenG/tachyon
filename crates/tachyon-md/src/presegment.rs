//! Cheap line scan that splits large inserted text into provisional chunks
//! before the real parse has run. Boundaries fall after blank lines outside
//! fenced code, so a pasted code block with blank lines stays in one chunk.
//! Only display and parse scheduling use these boundaries; the parse decides
//! the real block structure.

/// Byte offsets (line starts) where a provisional chunk may begin, excluding
/// 0 and `src.len()`, in increasing order.
pub fn presegment(src: &str) -> Vec<usize> {
    let mut boundaries = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut previous_blank = false;
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        match fence {
            Some((ch, len)) => {
                if closes_fence(line, ch, len) {
                    fence = None;
                }
                previous_blank = false;
            }
            None => {
                let blank = line.trim().is_empty();
                if previous_blank && !blank && start > 0 {
                    boundaries.push(start);
                }
                fence = opens_fence(line);
                previous_blank = blank;
            }
        }
    }
    boundaries
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
}
