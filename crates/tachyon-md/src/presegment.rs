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

/// Whether `src` ends inside a fenced code block it opened, or inside an HTML block (`<div>`,
/// `<!--` comments and friends - see [`HtmlEnd`]) still swallowing lines because it has not met
/// its own end condition yet. Either way, text appended after such a `src` cannot use boundaries
/// computed for it alone: the appended text's own presegmenter run has no idea it is still
/// inside a construct that started earlier.
pub fn ends_in_fence(src: &str) -> bool {
    let (fence, html) = scan(src, None);
    fence.is_some() || html.is_some()
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
    let (fence, html) = scan_chunks(pieces, base, None);
    fence.is_some() || html.is_some()
}

/// Which end condition currently governs an open HTML block, mirroring CommonMark's seven HTML
/// block types (`opens_html_block_inner`'s doc comment has the exact start/end rules for each).
/// Types 6 and 7 end at the next blank line; types 1-5 end when their own specific pattern
/// appears on a line, which can be the very line that opened them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HtmlEnd {
    /// Types 6 and 7.
    BlankLine,
    /// Type 1 (`<script`, `<pre`, `<style`, `<textarea`): a line containing (case-insensitive)
    /// `</script>`, `</pre>`, `</style>` or `</textarea>` - not necessarily matching which one
    /// opened it.
    ScriptLike,
    /// Type 2 (`<!--`): a line containing `-->`.
    Comment,
    /// Type 3 (`<?`): a line containing `?>`.
    Processing,
    /// Type 4 (`<!` + an ASCII letter): a line containing `>`.
    Declaration,
    /// Type 5 (`<![CDATA[`): a line containing `]]>`.
    Cdata,
}

impl HtmlEnd {
    /// Whether `line` satisfies this type's own end condition. Never called for `BlankLine`
    /// (types 6/7): the caller already has `blank` computed and checks that directly instead.
    fn closes(self, line: &str) -> bool {
        match self {
            HtmlEnd::BlankLine => false,
            HtmlEnd::ScriptLike => {
                contains_ci(line, "</script>")
                    || contains_ci(line, "</pre>")
                    || contains_ci(line, "</style>")
                    || contains_ci(line, "</textarea>")
            }
            HtmlEnd::Comment => line.contains("-->"),
            HtmlEnd::Processing => line.contains("?>"),
            HtmlEnd::Declaration => line.contains('>'),
            HtmlEnd::Cdata => line.contains("]]>"),
        }
    }
}

/// Case-insensitive (ASCII) substring search. `needle` is always a short, fixed literal here, so
/// the naive `O(haystack * needle)` cost never matters in practice.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    needle.len() <= haystack.len()
        && haystack.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// The fence/HTML-block/blank-line state one line scan step needs to remember between lines,
/// shared by [`scan`] (fed whole lines from a contiguous `&str`) and [`scan_chunks`] (fed lines
/// assembled from a fragment stream), so the two can never disagree about what counts as a
/// boundary.
#[derive(Default)]
struct LineScan {
    fence: Option<(u8, usize)>,
    html: Option<HtmlEnd>,
    previous_blank: bool,
    /// Whether the previous line was ordinary paragraph-continuation text: non-blank, and not
    /// itself the start of a fence or HTML block (or, once one closes, the line that closed
    /// it). Gates HTML type 7's "cannot interrupt a paragraph" rule - see
    /// [`opens_html_block_inner`]'s doc comment. Never set within an open fence or HTML block
    /// (nothing there is a candidate `in_paragraph` line either way, since [`LineScan::step`]
    /// returns before reaching the code that would set it).
    in_paragraph: bool,
}

impl LineScan {
    /// Advances the scan by one complete line (including its trailing `\n`, except possibly for
    /// a final line at the end of the text), starting at absolute offset `start`, pushing a
    /// boundary into `boundaries` if this line qualifies. `start == 0` can never qualify (there
    /// is no previous line yet to have been blank), so this needs no separate check for it.
    fn step(&mut self, line: &str, start: usize, boundaries: Option<&mut Vec<usize>>) {
        if let Some((ch, len)) = self.fence {
            if closes_fence(line, ch, len) {
                self.fence = None;
                self.in_paragraph = false;
            }
            self.previous_blank = false;
            return;
        }
        if let Some(end) = self.html {
            let blank = line.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
            let closes = if end == HtmlEnd::BlankLine { blank } else { end.closes(line) };
            if closes {
                self.html = None;
                self.previous_blank = end == HtmlEnd::BlankLine && blank;
                self.in_paragraph = false;
            } else {
                self.previous_blank = false;
            }
            return;
        }
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
        if !blank && let Some(end) = opens_html_block_inner(line, self.in_paragraph) {
            // Types 1-5 can meet their own end condition on the very line that opens them
            // (`<script>ok</script>`); types 6/7 (`BlankLine`) never do.
            let closed_same_line = end != HtmlEnd::BlankLine && end.closes(line);
            self.html = if closed_same_line { None } else { Some(end) };
            self.previous_blank = false;
            self.in_paragraph = false;
            return;
        }
        if matches!(first, Some(b'`' | b'~')) {
            self.fence = opens_fence(line);
            if self.fence.is_some() {
                self.previous_blank = false;
                self.in_paragraph = false;
                return;
            }
        }
        self.previous_blank = blank;
        self.in_paragraph = !blank;
    }
}

/// Scans `src` line by line, pushing chunk boundaries into `boundaries` if given; returns the
/// fence still open at the end, and the HTML block end condition still pending, if any.
fn scan(
    src: &str,
    mut boundaries: Option<&mut Vec<usize>>,
) -> (Option<(u8, usize)>, Option<HtmlEnd>) {
    let mut state = LineScan::default();
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        state.step(line, offset, boundaries.as_deref_mut());
        offset += line.len();
    }
    (state.fence, state.html)
}

/// [`scan`] fed from a fragment stream instead of one contiguous `&str`: see
/// [`presegment_chunks`]. A line is scanned straight from whichever piece holds it whole; only a
/// line split across pieces is assembled into `buf` (cleared after each use) first.
fn scan_chunks<'a>(
    pieces: impl Iterator<Item = &'a str>,
    base: usize,
    mut boundaries: Option<&mut Vec<usize>>,
) -> (Option<(u8, usize)>, Option<HtmlEnd>) {
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
    (state.fence, state.html)
}

/// Whether `line` opens any CommonMark HTML block type (1-7) on its own, as if it were the very
/// first line of a fresh top-level context - never blocked by a preceding paragraph the way type
/// 7 can be (see [`opens_html_block_inner`], used internally by [`LineScan`] with the real,
/// tracked context). `tachyon-doc`'s `block_shape` uses this on a `single_source` block's own
/// first line, which is always such a fresh context (nothing precedes it within that block).
pub fn opens_html_block(line: &str) -> bool {
    opens_html_block_inner(line, false).is_some()
}

/// Block-level tag names for CommonMark HTML block type 6 (case-insensitive), checked after up
/// to 3 spaces of indent and a leading `<` or `</`.
const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "section",
    "source",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// Tag names for CommonMark HTML block type 1, which - unlike every other type - is excluded
/// from type 6 and type 7 entirely (its own start/end rules apply instead, or nothing does).
const SCRIPT_LIKE_TAGS: &[&str] = &["script", "pre", "style", "textarea"];

/// Whether `line` opens a CommonMark HTML block, and which end condition would then apply.
/// `in_paragraph`: whether the previous line was ordinary paragraph-continuation text, which
/// blocks type 7 (see below) - always `false` for a fresh top-level context ([`opens_html_block`]).
///
/// After up to 3 spaces of indent (the same indentation [`fence_run`] allows), in source order:
/// - **Type 2** (`<!--`), **type 5** (`<![CDATA[`, checked first since it also starts `<!`),
///   **type 4** (`<!` + an ASCII letter) and **type 3** (`<?`) are recognized from their fixed
///   opening string alone, regardless of what (if anything) follows on the line.
/// - **Type 1** (`<script`, `<pre`, `<style`, `<textarea`, case-insensitive; opening tag only,
///   not `</script>` etc.) needs whitespace, `>`, or the end of the line right after the tag
///   name. These four tag names are otherwise excluded from types 6 and 7 entirely (their own
///   end condition governs instead, checked separately by [`HtmlEnd::closes`]; a malformed
///   `<script`-like line that fails type 1's own terminator check opens nothing here).
/// - **Type 6**: one of [`HTML_BLOCK_TAGS`] (open or closing tag, case-insensitive), followed
///   immediately by whitespace, the end of the line, `>`, or `/>` (nothing about the *rest* of
///   the line matters - unlike type 7, type 6 does not need to be alone on it). Can interrupt a
///   paragraph.
/// - **Type 7**: any other complete open or closing tag - [`parses_as_one_tag`] implements
///   enough of CommonMark's tag grammar (name, `name` or `name=value` attributes, optional `/`,
///   `>`) to tell whether one fills the *entire* line but for trailing whitespace, the CommonMark
///   requirement type 6 does not share. Blocked when `in_paragraph`: unlike every other type,
///   type 7 cannot interrupt a paragraph, and a line that only qualifies as type 7 is otherwise
///   ordinary text (this scanner does not otherwise distinguish a real paragraph from an
///   in-progress list item or block quote it cannot see the container markers of - one non-blank
///   line that opened nothing else is treated as "paragraph enough" to block type 7, which is
///   conservative in exactly the direction that matters: understating what type 7 may interrupt
///   only risks treating a line that truly does open type 7 as ordinary text instead - never an
///   unsafe boundary, only a missed one, since ordinary text never toggles fence state on its
///   own - while overstating it risks the opposite: swallowing a line that is really a *different*
///   construct able to interrupt a paragraph for real (a fence), never seeing that it opens one).
fn opens_html_block_inner(line: &str, in_paragraph: bool) -> Option<HtmlEnd> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    if !body.starts_with('<') {
        return None;
    }
    if body.starts_with("<!--") {
        return Some(HtmlEnd::Comment);
    }
    if body.starts_with("<![CDATA[") {
        return Some(HtmlEnd::Cdata);
    }
    if let Some(rest) = body.strip_prefix("<!")
        && rest.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
    {
        return Some(HtmlEnd::Declaration);
    }
    if body.starts_with("<?") {
        return Some(HtmlEnd::Processing);
    }
    let (closing, name, rest) = parse_tag_start(body)?;
    if SCRIPT_LIKE_TAGS.iter().any(|t| name.eq_ignore_ascii_case(t)) {
        let starts_terminator =
            rest.is_empty() || rest.starts_with([' ', '\t', '\r', '\n']) || rest.starts_with('>');
        return (!closing && starts_terminator).then_some(HtmlEnd::ScriptLike);
    }
    if HTML_BLOCK_TAGS.iter().any(|t| name.eq_ignore_ascii_case(t))
        && (rest.is_empty()
            || rest.starts_with([' ', '\t', '\r', '\n'])
            || rest.starts_with('>')
            || rest.starts_with("/>"))
    {
        return Some(HtmlEnd::BlankLine);
    }
    (!in_paragraph && parses_as_one_tag(body)).then_some(HtmlEnd::BlankLine)
}

/// If `body` starts with `<` or `</` followed by an ASCII-letter-led tag name (letters, digits
/// and hyphens after the first letter), returns `(is_closing, name, rest)`: `name` is the tag
/// name itself, `rest` is everything in `body` after it (including, e.g., attributes or a
/// trailing `\n`).
fn parse_tag_start(body: &str) -> Option<(bool, &str, &str)> {
    let after_lt = body.strip_prefix('<')?;
    let (closing, after_slash) = match after_lt.strip_prefix('/') {
        Some(r) => (true, r),
        None => (false, after_lt),
    };
    let bytes = after_slash.as_bytes();
    if !bytes.first().is_some_and(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut end = 1;
    while bytes.get(end).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-') {
        end += 1;
    }
    Some((closing, &after_slash[..end], &after_slash[end..]))
}

/// Whether `body` (from its own `<` onward) is one complete HTML open or closing tag - enough of
/// CommonMark's tag grammar for this scanner's purposes (a closing tag: name then only
/// whitespace then `>`; an open tag: name, then zero or more `name` or `name=value` attributes,
/// then optional `/`, then `>`) - with nothing but whitespace following it to the end of the
/// line, as HTML block type 7 requires. Conservative on purpose (see
/// [`opens_html_block_inner`]'s doc comment): a line this cannot fully parse this way is treated
/// as opening nothing, which only risks a missed HTML boundary, never a wrong one.
fn parses_as_one_tag(body: &str) -> bool {
    let Some((closing, _name, mut rest)) = parse_tag_start(body) else { return false };
    if closing {
        let Some(after) = rest.trim_start_matches([' ', '\t']).strip_prefix('>') else {
            return false;
        };
        return after.chars().all(char::is_whitespace);
    }
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t', '\n']);
        if let Some(after) = trimmed.strip_prefix("/>").or_else(|| trimmed.strip_prefix('>')) {
            return after.chars().all(char::is_whitespace);
        }
        let bytes = trimmed.as_bytes();
        let Some(&first) = bytes.first() else { return false };
        if !(first.is_ascii_alphabetic() || first == b'_' || first == b':') {
            return false;
        }
        let mut i = 1;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
        {
            i += 1;
        }
        let after_ws = trimmed[i..].trim_start_matches([' ', '\t', '\n']);
        rest = match after_ws.strip_prefix('=') {
            None => after_ws,
            Some(after_eq) => {
                let after_eq = after_eq.trim_start_matches([' ', '\t', '\n']);
                match after_eq.as_bytes().first() {
                    Some(b'"') => match after_eq[1..].find('"') {
                        Some(end) => &after_eq[1 + end + 1..],
                        None => return false,
                    },
                    Some(b'\'') => match after_eq[1..].find('\'') {
                        Some(end) => &after_eq[1 + end + 1..],
                        None => return false,
                    },
                    Some(_) => {
                        let end = after_eq
                            .find([' ', '\t', '\n', '"', '\'', '=', '<', '>', '`'])
                            .unwrap_or(after_eq.len());
                        if end == 0 {
                            return false;
                        }
                        &after_eq[end..]
                    }
                    None => return false,
                }
            }
        };
    }
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

    /// CodeRabbit finding on the original HTML-awareness fix: types 1-5 end at their own
    /// pattern, not a blank line, and the end can be on the very line that opens them.
    #[test]
    fn html_types_1_to_5_end_at_their_own_pattern_not_a_blank_line() {
        // Type 1: ends on the same line it opens (`</script>` right there), so the fence after
        // it is real and immediately closed by nothing - no boundary may appear before `text`.
        let src = "<script>ok</script>\n~~~\n\ntext\n";
        assert!(
            ends_in_fence(src),
            "the script tag closes on line 1, so line 2's ~~~ genuinely opens a fence, never closed"
        );
        assert_eq!(
            presegment(src),
            Vec::<usize>::new(),
            "no blank line precedes ~~~, so no boundary is ever proposed for it or after it \
             (nothing may cut inside the fence it opens, which is never closed)"
        );

        // Type 1: spans multiple lines, blank line included, until its own end tag.
        let src = "<script>\nvar x = 1;\n\nstill in script\n</script>\n\nafter\n";
        assert!(!ends_in_fence(src));
        let after = src.find("after").unwrap();
        assert_eq!(
            presegment(src),
            vec![after],
            "a blank line inside <script>...</script> does not end it"
        );

        // Type 2: comment, ends at `-->`, spans a blank line.
        let src = "<!--\ncomment\n\nstill comment\n-->\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);
        assert!(ends_in_fence("<!--\nunterminated\n"));

        // Type 3: processing instruction, ends at `?>`.
        let src = "<?php\necho 1;\n\n?>\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);

        // Type 4: declaration, ends at `>`.
        let src = "<!DOCTYPE html\nmore\n>\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);

        // Type 5: CDATA, ends at `]]>`.
        let src = "<![CDATA[\ndata\n\nmore data\n]]>\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);
    }

    /// CodeRabbit finding: types 1-5 (comments and friends) were not recognized as opening
    /// anything at all, so a fence marker swallowed inside one could desync fence tracking the
    /// same way the original bug's `<div>` case did.
    #[test]
    fn a_fence_marker_swallowed_by_a_comment_does_not_toggle_fence_state() {
        let src = "<!--\n~~~\n-->\n\n~~~\ntext\n";
        let real_fence = src.rfind("~~~").unwrap();
        assert!(ends_in_fence(src), "the real, second ~~~ opens a fence nothing here closes");
        assert_eq!(presegment(src), vec![real_fence]);
    }

    /// CodeRabbit finding: HTML block type 7 (a bare tag alone on its own line) cannot interrupt
    /// a paragraph, so a tag that is really just paragraph text must not swallow what follows -
    /// including a fence that genuinely does interrupt the paragraph.
    #[test]
    fn a_type_7_tag_does_not_interrupt_a_paragraph() {
        let src = "para\n<a>\n~~~\n\ntext\n";
        // `<a>` is blocked from opening type 7 (it directly continues the paragraph "para"), so
        // `~~~` genuinely opens a fence - unclosed, so it swallows the rest of the text.
        assert!(ends_in_fence(src), "the fence ~~~ opens is never closed");
        // No blank line precedes ~~~ either, so presegment proposes nothing at all here -
        // nothing may cut inside the fence it opens, which is never closed.
        assert_eq!(presegment(src), Vec::<usize>::new());
    }

    /// Type 7 *can* open freshly (not interrupting anything): at the very start of the text, or
    /// right after a blank line.
    #[test]
    fn a_type_7_tag_opens_when_not_interrupting_a_paragraph() {
        assert_eq!(presegment("<a>\nstill in it\n\nafter\n"), vec!["<a>\nstill in it\n\n".len()]);
        let src = "para\n\n<a>\nstill in it\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec!["para\n\n".len(), after]);
    }

    /// Type 7 needs the *entire* line (but for trailing whitespace); type 6 does not - extra
    /// content after a known tag name still opens type 6, but the same extra content after an
    /// unknown tag name does not open type 7.
    #[test]
    fn type_6_does_not_need_to_be_alone_on_its_line_type_7_does() {
        let src = "<div class=\"x\">\n~~~\nstill open\n";
        assert!(ends_in_fence(src), "a known tag name with attributes still opens type 6");
        let src = "para\n<b>extra</b>\n~~~\n\ntext\n";
        // "b" is not a type-6 name; "<b>extra</b>" is not a bare type-7 tag either (real
        // content follows the first tag's own close), and it directly continues the paragraph
        // "para" regardless - so it is just ordinary text, and the fence on the next line is
        // real (unclosed, and with no blank line before it, presegment proposes nothing here).
        assert_eq!(presegment(src), Vec::<usize>::new());
        assert!(ends_in_fence(src));
        // Same, but with a real closing fence and a trailing blank line, so the boundary it
        // does yield (once closed for real) is actually observable.
        let src = "para\n<b>extra</b>\n~~~\nfence content\n~~~\n\nafter\n";
        let after = src.find("after").unwrap();
        assert_eq!(presegment(src), vec![after]);
        assert!(!ends_in_fence(src));
    }

    /// The four type-1 tag names never open type 6 (not in [`HTML_BLOCK_TAGS`]) or type 7
    /// (explicitly excluded), regardless of surrounding context: only type 1's own start/end
    /// rules ever apply to them, and a line matching one of these names but failing type 1's
    /// own terminator (here, a colon right after the name instead of whitespace/`>`/end of
    /// line) opens nothing at all - it must not fall through and match as a type 7 tag instead
    /// (`:bad` would otherwise parse as one attribute, closing `>` and all).
    #[test]
    fn script_like_tag_names_never_open_type_6_or_7() {
        let src = "<script:bad>\n~~~\nstill\n\nafter\n";
        assert_eq!(
            presegment(src),
            Vec::<usize>::new(),
            "the line matches no HTML block type, so ~~~ genuinely opens a fence that swallows \
             the rest of the text including the blank line before `after`"
        );
    }

    /// [`presegment_chunks`]/[`ends_in_fence_chunks`] agree with the contiguous scan regardless
    /// of where the input happens to be split into pieces - including splits that land inside a
    /// line, inside a fence delimiter run, inside an HTML block of every type, and exactly on a
    /// line boundary.
    #[test]
    fn chunked_scan_matches_contiguous_scan_at_every_split() {
        let src = "# Title\n\npara one\nstill ```one\n\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\n\
                   <div>\n~~~\n</div>\n\n~~~\nreal fence\n\nmore\n\nafter1\n\n\
                   <script>ok</script>\n~~~\n\nafter2\n\n\
                   <!--\n~~~\n-->\n\n~~~\nreal fence 2\n\nafter3\n\n\
                   <?php\necho 1;\n?>\n\nafter4\n\n\
                   <![CDATA[\ndata\n]]>\n\nafter5\n\n\
                   <!DOCTYPE html>\n\nafter6\n\n\
                   para\n<a>\n~~~\n\nafter7\n";
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

        for unclosed in [
            "text\n\n```rust\nlet x = 1;\n\n",
            "text\n\n<div>\nstill open\n",
            "text\n\n<script>\nstill open\n",
            "text\n\n<!--\nstill open\n",
            "text\n\n<?\nstill open\n",
            "text\n\n<!X\nstill open\n",
            "text\n\n<![CDATA[\nstill open\n",
        ] {
            for split in 0..=unclosed.len() {
                if !unclosed.is_char_boundary(split) {
                    continue;
                }
                let (a, b) = unclosed.split_at(split);
                assert_eq!(
                    ends_in_fence_chunks([a, b].into_iter(), 0),
                    ends_in_fence(unclosed),
                    "split at {split} in {unclosed:?}"
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

    /// Every boundary [`presegment`] proposes for `src` must be a real top-level block start of
    /// a full parse of the same text - never a cut inside a block `tachyon_md::parse_document`
    /// keeps whole. This is the invariant every other test in this module is really protecting;
    /// checking it directly, for a wide range of inputs (including every fixture the other
    /// tests use), is the strongest guard against a scanner change quietly reintroducing an
    /// unsafe cut of a kind no single hand-picked example happens to cover.
    fn assert_boundaries_are_real_block_starts(src: &str) {
        let (blocks, _) = crate::parse_document(src);
        let mut real_starts = std::collections::HashSet::new();
        let mut at = 0;
        for block in &blocks {
            real_starts.insert(at);
            at += block.len;
        }
        for boundary in presegment(src) {
            assert!(
                real_starts.contains(&boundary),
                "presegment proposed {boundary} in {src:?}, not a real block start \
                 (real starts: {real_starts:?}, blocks: {blocks:?})"
            );
        }
    }

    #[test]
    fn every_presegment_boundary_is_a_real_block_start() {
        for src in [
            "# Title\n\npara one\nstill one\n\n\n- item\n",
            "```rust\nfn a() {}\n\nfn b() {}\n```\n\nafter\n",
            "````\n```\n\n~~~~\n\n````\n\nafter\n",
            "text\n\n~~~\ncode\n\nmore code\n",
            "    ```\n\nnot code\n\n``a`` inline\n\nend\n",
            "<div>\n~~~\n</div>\n\n~~~\ncode\n\nmore code\n",
            "<div>\nnot a fence marker: ~~~\n</div>\n\nafter\n",
            "< 3\n\n~~~\ncode\n\nmore\n",
            "<script>ok</script>\n~~~\n\ntext\n",
            "<script>\nvar x = 1;\n\nstill in script\n</script>\n\nafter\n",
            "<!--\ncomment\n\nstill comment\n-->\n\nafter\n",
            "<?php\necho 1;\n\n?>\n\nafter\n",
            "<!DOCTYPE html\nmore\n>\n\nafter\n",
            "<![CDATA[\ndata\n\nmore data\n]]>\n\nafter\n",
            "<!--\n~~~\n-->\n\n~~~\ntext\n",
            "para\n<a>\n~~~\n\ntext\n",
            "<a>\nstill in it\n\nafter\n",
            "para\n\n<a>\nstill in it\n\nafter\n",
            "<div class=\"x\">\n~~~\nstill open\n\nafter\n",
            "para\n<b class=\"x\">\n~~~\n\ntext\n",
            "<pre>\ncode\n\nmore\n</pre>\n\nafter\n",
        ] {
            assert_boundaries_are_real_block_starts(src);
        }
    }
}
