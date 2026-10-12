//! List editing: Enter continues a list item with the next marker (Enter on an empty item ends the
//! list), Tab and Shift+Tab indent and outdent the list items a selection touches.

use gpui::Context;
use tachyon_md::BlockKind;

use crate::editor::Editor;
use crate::movement;

/// A list item's line, split into its prefix parts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ListLine<'a> {
    /// Block quote markers before the item (`> > `).
    pub(crate) quote: &'a str,
    /// Spaces between the quote markers and the list marker.
    pub(crate) indent: usize,
    pub(crate) marker: Marker,
    /// The task box after the marker, if any: whether it is checked.
    pub(crate) task: Option<bool>,
    /// Byte offset in the line where the item's text starts.
    pub(crate) content: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Marker {
    /// `-`, `*` or `+`.
    Bullet(char),
    /// A number and its delimiter, `.` or `)`.
    Ordered { number: u64, delimiter: char, digits: usize },
}

impl Marker {
    /// Width of the marker and the space after it: how far a nested item is indented.
    fn width(&self) -> usize {
        match self {
            Marker::Bullet(_) => 2,
            Marker::Ordered { digits, .. } => digits + 2,
        }
    }
}

/// Parses a list item line (without its line break). `None` if the line is not a list item.
pub(crate) fn parse(line: &str) -> Option<ListLine<'_>> {
    let bytes = line.as_bytes();
    let mut at = 0;
    // Quote markers, each optionally followed by one space.
    while bytes.get(at) == Some(&b'>') {
        at += 1;
        if bytes.get(at) == Some(&b' ') {
            at += 1;
        }
    }
    let quote = &line[..at];
    let indent = bytes[at..].iter().take_while(|&&b| b == b' ').count();
    at += indent;
    let marker = match bytes.get(at)? {
        &b @ (b'-' | b'*' | b'+') => {
            at += 1;
            Marker::Bullet(char::from(b))
        }
        b'0'..=b'9' => {
            let digits = bytes[at..].iter().take_while(|b| b.is_ascii_digit()).count();
            // CommonMark: at most nine digits.
            if digits > 9 {
                return None;
            }
            let number = line[at..at + digits].parse().ok()?;
            at += digits;
            let delimiter = match bytes.get(at)? {
                &d @ (b'.' | b')') => char::from(d),
                _ => return None,
            };
            at += 1;
            Marker::Ordered { number, delimiter, digits }
        }
        _ => return None,
    };
    // A marker is followed by a space, or ends the line (an empty item).
    match bytes.get(at) {
        None => {}
        Some(b' ') => at += 1,
        Some(_) => return None,
    }
    let task = match &bytes[at..] {
        [b'[', mark @ (b' ' | b'x' | b'X'), b']', rest @ ..]
            if rest.is_empty() || rest[0] == b' ' =>
        {
            at += 3 + usize::from(!rest.is_empty());
            Some(*mark != b' ')
        }
        _ => None,
    };
    Some(ListLine { quote, indent, marker, task, content: at })
}

/// The prefix of the item after `item`: same quote and indent, the next number, an unchecked box.
fn next_prefix(item: &ListLine<'_>) -> String {
    let marker = match item.marker {
        Marker::Bullet(c) => c.to_string(),
        Marker::Ordered { number, delimiter, .. } => format!("{}{delimiter}", number + 1),
    };
    let task = if item.task.is_some() { " [ ]" } else { "" };
    format!("{}{}{marker}{task} ", item.quote, " ".repeat(item.indent))
}

/// The byte offset of the mark (the space or `x` inside `[ ]`) of the task box on the list item
/// line `line` (without its line break), and whether it is checked. `None` if `line` is not a
/// task item. Scans the item's prefix itself rather than reusing [`parse`], because the Markdown
/// parser accepts any run of spaces between the marker and the box (`-  [ ] item`), which draws
/// a box that `parse` (one space, as Enter and Tab write them) would not find.
pub(crate) fn task_mark(line: &str) -> Option<(usize, bool)> {
    let item = parse_marker_only(line)?;
    let bytes = line.as_bytes();
    let mut at = item;
    while bytes.get(at) == Some(&b' ') {
        at += 1;
    }
    // At least one space after the marker, then `[`, the mark, `]`, and a space or the line end.
    if at == item {
        return None;
    }
    match &bytes[at..] {
        [b'[', mark @ (b' ' | b'x' | b'X'), b']'] => Some((at + 1, *mark != b' ')),
        [b'[', mark @ (b' ' | b'x' | b'X'), b']', b' ', ..] => Some((at + 1, *mark != b' ')),
        _ => None,
    }
}

/// Where a list item's marker ends on `line` (past its quote markers, indent and bullet or
/// number), the offset the task box scan starts from. `None` if the line is not a list item.
fn parse_marker_only(line: &str) -> Option<usize> {
    let item = parse(line)?;
    Some(item.quote.len() + item.indent + item.marker.len())
}

impl Editor {
    /// Whether list editing applies at `offset`: not inside code, HTML or text awaiting a parse,
    /// where a line starting with `-` is not a list item.
    fn in_list_context(&self, offset: usize) -> bool {
        let Some(index) = self.doc.block_at(offset) else { return false };
        !matches!(
            self.doc.blocks()[index].parsed().kind,
            BlockKind::CodeBlock { .. } | BlockKind::Html | BlockKind::Unparsed
        )
    }

    /// Enter in a list item: continues the list. On an empty item it moves the item up a level,
    /// or ends the list at the top level. Returns whether it handled the key.
    pub(crate) fn list_newline(&mut self, cx: &mut Context<Self>) -> bool {
        let caret = self.selection.start;
        if !self.selection.is_empty() || !self.in_list_context(caret) {
            return false;
        }
        let rope = self.doc.buffer().rope();
        let (start, end) = (movement::home(rope, caret), movement::end(rope, caret));
        let line = rope.byte_slice(start..end).to_string();
        let Some(item) = parse(&line) else { return false };
        if caret - start < item.content {
            return false;
        }
        if line[item.content..].trim().is_empty() {
            if item.indent > 0 && self.list_indent(true, cx) {
                return true;
            }
            // An empty top-level item ends the list: its marker goes, and a blank line keeps the
            // text typed next from continuing the last item.
            let quote = item.quote;
            let text = if quote.is_empty() {
                "\n".to_owned()
            } else {
                format!("{}\n{quote}", quote.trim_end())
            };
            self.replace(start..end, &text, cx);
            return true;
        }
        let text = format!("\n{}", next_prefix(&item));
        self.replace(caret..caret, &text, cx);
        true
    }

    /// Tab (or Shift+Tab, `outdent`) with the caret or selection on list items: nests each item
    /// under the previous one (or moves it up to its parent's level), renumbering ordered items
    /// for their new list. Returns whether it handled the key: it does when a touched line is a
    /// list item, even one that cannot move (a list's first item has nothing to nest under).
    pub(crate) fn list_indent(&mut self, outdent: bool, cx: &mut Context<Self>) -> bool {
        if !self.in_list_context(self.selection.start) {
            return false;
        }
        let rope = self.doc.buffer().rope();
        let start = movement::home(rope, self.selection.start);
        // A selection ending at a line start does not include that line.
        let last = if self.selection.end > self.selection.start
            && movement::home(rope, self.selection.end) == self.selection.end
        {
            self.selection.end - 1
        } else {
            self.selection.end
        };
        let end = movement::end(rope, last);
        let text = rope.byte_slice(start..end).to_string();
        let first_line = rope.byte_to_line(start);
        // Lines before the selection, nearest first, for finding siblings and parents.
        let before: Vec<String> = (first_line.saturating_sub(LOOKBACK)..first_line)
            .rev()
            .map(|i| rope.line(i).to_string())
            .collect();
        // The selection's lines as edited so far, and for each changed line where its prefix
        // changed (an original offset) and by how much, to move the selection with the text.
        let mut done: Vec<String> = Vec::new();
        let mut shifts: Vec<(usize, isize)> = Vec::new();
        let mut line_start = start;
        let mut any_item = false;
        for line in text.split('\n') {
            // A selection can reach into a code block, where `- x` is text, not an item.
            let item = parse(line).filter(|_| self.in_list_context(line_start));
            any_item |= item.is_some();
            let edited = item.and_then(|item| {
                let previous = done.iter().rev().chain(&before).map(String::as_str);
                let indent = if outdent {
                    parent_indent(previous, &item)?
                } else {
                    child_indent(previous, &item)?
                };
                let previous = done.iter().rev().chain(&before).map(String::as_str);
                let marker = match item.marker {
                    Marker::Bullet(c) => c.to_string(),
                    Marker::Ordered { delimiter, .. } => {
                        format!("{}{delimiter}", number_at(previous, item.quote, indent))
                    }
                };
                let rest = &line[item.quote.len() + item.indent + item.marker.len()..];
                Some(format!("{}{}{marker}{rest}", item.quote, " ".repeat(indent)))
            });
            if let Some(edited) = &edited {
                let at = line_start + parse(line).map_or(0, |item| item.quote.len());
                shifts.push((at, edited.len() as isize - line.len() as isize));
            }
            line_start += line.len() + 1;
            done.push(edited.unwrap_or_else(|| line.to_owned()));
        }
        if shifts.is_empty() {
            return any_item;
        }
        let map = |offset: usize| -> usize {
            let mut new = offset as isize;
            for &(at, delta) in &shifts {
                if offset < at {
                    break;
                }
                // Longer prefixes push the offset along; shorter ones pull it back, and an offset
                // inside the removed part lands where it started.
                new += if delta > 0 { delta } else { -((offset - at) as isize).min(-delta) };
            }
            new as usize
        };
        let (anchor, head) = (map(self.tail()), map(self.head()));
        self.replace_selection_with(start..end, &done.join("\n"), cx);
        self.move_to(anchor, false, cx);
        self.move_to(head, anchor != head, cx);
        true
    }
}

impl Editor {
    /// Flips the task box on the list item at source offset `at` (any offset on its line): the
    /// space or `x` inside `[ ]` becomes the other, as one undoable edit that leaves the caret,
    /// the selection and the block's rendered state alone. Returns whether `at` was a task item.
    pub(crate) fn toggle_task_at(&mut self, at: usize, cx: &mut Context<Self>) -> bool {
        // Decided from the text as the user saw it when they clicked: which line, where its mark
        // is, whether it is checked.
        if !self.in_list_context(at) {
            return false;
        }
        let rope = self.doc.buffer().rope();
        let (start, end) = (movement::home(rope, at), movement::end(rope, at));
        let line = rope.byte_slice(start..end).to_string();
        let Some((mark, checked)) = task_mark(&line) else { return false };
        let mut mark_at = start + mark;
        // A paste still being read or prepared lands first, so the edits keep the order of the
        // user's input (every other non-typing input does the same). It may move the box, so the
        // mark's offset follows it; and the byte found there must still be the mark we decided
        // on, so a paste that rewrote the box itself toggles nothing.
        let before = self.doc.buffer().version();
        self.flush_pending_paste(cx);
        if let Some(edits) = self.doc.buffer().edits_since(before) {
            mark_at = edits.fold(mark_at, |at, edit| edit.map_offset(at));
        }
        let rope = self.doc.buffer().rope();
        let current = (mark_at < rope.len_bytes()).then(|| rope.byte_slice(mark_at..mark_at + 1));
        let intact = current.is_some_and(|byte| {
            let byte = byte.to_string();
            if checked { byte == "x" || byte == "X" } else { byte == " " }
        });
        if !intact {
            return false;
        }
        let (caret, reversed, reveal) = (self.selection.clone(), self.reversed, self.reveal);
        self.doc.seal_undo_group();
        self.replace(mark_at..mark_at + 1, if checked { " " } else { "x" }, cx);
        self.doc.seal_undo_group();
        // `replace` puts the caret after the edit and asks for it to be scrolled into view; a
        // checkbox click is neither a caret move nor a reason to leave the box that was clicked.
        self.selection = caret;
        self.reversed = reversed;
        self.reveal = reveal;
        self.update_active();
        cx.notify();
        true
    }
}

/// How many lines before a selection are searched for its items' siblings and parents.
const LOOKBACK: usize = 500;

impl Marker {
    /// Length of the marker itself (`-`, `12.`).
    fn len(&self) -> usize {
        match self {
            Marker::Bullet(_) => 1,
            Marker::Ordered { digits, .. } => digits + 1,
        }
    }
}

/// The nearest list item before `item` (walking `previous`, nearest first) that is at its level or
/// above, in the same quote. Stops at text outside the list.
fn previous_item<'a>(
    previous: impl Iterator<Item = &'a str>,
    item: &ListLine<'_>,
    level: usize,
) -> Option<ListLine<'a>> {
    for line in previous {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            // Blank lines separate the items of a loose list.
            continue;
        }
        match parse(line) {
            Some(found) if found.quote != item.quote => return None,
            Some(found) if found.indent <= level => return Some(found),
            // A deeper item, or an item's continuation text.
            Some(_) => {}
            None if line.starts_with(' ') => {}
            None => return None,
        }
    }
    None
}

/// Indent that nests `item` under the previous item at its level: that item's text column. `None`
/// for the first item of a list, which has nothing to nest under.
fn child_indent<'a>(previous: impl Iterator<Item = &'a str>, item: &ListLine<'_>) -> Option<usize> {
    let sibling = previous_item(previous, item, item.indent)?;
    (sibling.indent == item.indent).then(|| sibling.indent + sibling.marker.width())
}

/// Indent that moves `item` up to its parent's level. `None` at the top level.
fn parent_indent<'a>(
    previous: impl Iterator<Item = &'a str>,
    item: &ListLine<'_>,
) -> Option<usize> {
    if item.indent == 0 {
        return None;
    }
    let parent = previous_item(previous, item, item.indent - 1);
    Some(parent.map_or(0, |parent| parent.indent))
}

/// Number for an ordered item placed at `indent`: one more than the previous item of that list,
/// or 1 if it starts the list (a nested list must start at 1 to interrupt its parent's text).
fn number_at<'a>(previous: impl Iterator<Item = &'a str>, quote: &str, indent: usize) -> u64 {
    let probe = ListLine { quote, indent, marker: Marker::Bullet('-'), task: None, content: 0 };
    match previous_item(previous, &probe, indent) {
        Some(ListLine { indent: found, marker: Marker::Ordered { number, .. }, .. })
            if found == indent =>
        {
            number + 1
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bullets_numbers_tasks_and_quotes() {
        let item = parse("  * [x] done").expect("a list item");
        assert_eq!(
            (item.quote, item.indent, &item.marker, item.task, item.content),
            ("", 2, &Marker::Bullet('*'), Some(true), 8)
        );
        let item = parse("> 12) twelve").expect("a list item");
        assert_eq!(item.quote, "> ");
        assert_eq!(item.marker, Marker::Ordered { number: 12, delimiter: ')', digits: 2 });
        assert_eq!(item.content, 6);
        assert_eq!(parse("-").map(|i| i.content), Some(1), "an empty item");
        assert_eq!(parse("- [ ]").map(|i| (i.task, i.content)), Some((Some(false), 5)));
        assert_eq!(parse("- [x]not a box").map(|i| (i.task, i.content)), Some((None, 2)));
    }

    #[test]
    fn not_list_items() {
        for line in ["-a", "1.x", "text - no", "1234567890. big", "#- h", ""] {
            assert_eq!(parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn the_next_item_increments_and_unchecks() {
        let next = |line: &str| next_prefix(&parse(line).expect("a list item"));
        assert_eq!(next("- one"), "- ");
        assert_eq!(next("  9. nine"), "  10. ");
        assert_eq!(next("> - [x] done"), "> - [ ] ");
    }

    #[test]
    fn task_mark_finds_the_box_in_every_item_shape() {
        // (line, expected mark offset, checked)
        for (line, mark, checked) in [
            ("- [ ] one", 3, false),
            ("- [x] two", 3, true),
            ("- [X] two", 3, true),
            ("  - [ ] nested", 5, false),
            ("1. [ ] numbered", 4, false),
            ("12) [x] twelve", 5, true),
            ("> - [ ] quoted", 5, false),
            ("> > * [x] deep", 7, true),
            ("- [ ]", 3, false), // an empty task item: the box ends the line
            ("-  [ ] padded", 4, false), // the parser accepts any run of spaces after the marker
            ("-   [x] more", 5, true),
            ("1.  [ ] numbered", 5, false),
        ] {
            let found = task_mark(line);
            assert_eq!(found, Some((mark, checked)), "{line:?}");
            assert!(
                matches!(line.as_bytes()[mark], b' ' | b'x' | b'X'),
                "{line:?}: points at the mark"
            );
        }
    }

    #[test]
    fn task_mark_ignores_lines_that_are_not_task_items() {
        for line in [
            "- plain [ ] not a task",
            "- [] no mark",
            "- [y] bad mark",
            "text [ ]",
            "[ ] bare",
            "- [ ]x",
            "",
        ] {
            assert_eq!(task_mark(line), None, "{line:?}");
        }
    }
}
