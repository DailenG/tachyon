//! Bare `http://` and `https://` URLs in text become links, as GitHub renders them (GFM's extended
//! autolinks; pulldown-cmark leaves them as text). Runs on a finished block's IR, so the source map
//! is untouched: the URL's visible text is its source text.

use std::ops::Range;

use crate::ir::{BlockIr, LineKind, LinkSpan, Style, StyleRun};

/// Styles whose text is never scanned for URLs.
const LITERAL: [Style; 4] = [Style::CODE, Style::MATH, Style::HTML, Style::IMAGE];

/// Characters trimmed from the end of a URL: sentence punctuation around it, not part of it.
const TRAILING: &[char] = &['?', '!', '.', ',', ':', '*', '_', '~', '\'', '"', ';'];

pub(crate) fn autolink(ir: &mut BlockIr) {
    if !ir.text.contains("http") {
        return;
    }
    let found: Vec<Range<usize>> = urls(&ir.text)
        .filter(|url| {
            let overlaps = |r: &Range<usize>| r.start < url.end && url.start < r.end;
            !ir.links.iter().any(|link| overlaps(&link.visible))
                && !ir.runs.iter().any(|run| {
                    overlaps(&run.range) && LITERAL.iter().any(|&s| run.style.contains(s))
                })
                && !matches!(line_kind(ir, url.start), Some(LineKind::Code | LineKind::Html))
        })
        .collect();
    if found.is_empty() {
        return;
    }
    for url in found {
        add_style(&mut ir.runs, &url, Style::LINK);
        let dest = ir.text[url.clone()].to_owned();
        ir.links.push(LinkSpan { visible: url, dest });
    }
    ir.links.sort_by_key(|link| link.visible.start);
}

fn line_kind(ir: &BlockIr, offset: usize) -> Option<LineKind> {
    let index = ir.lines.partition_point(|line| line.start <= offset).checked_sub(1)?;
    Some(ir.lines[index].kind)
}

/// Byte ranges of bare URLs in `text`: `http://` or `https://` at a word start, running to
/// whitespace or `<`, without trailing punctuation or an unbalanced closing parenthesis.
fn urls(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut from = 0;
    std::iter::from_fn(move || {
        while let Some(found) = text.get(from..)?.find("http") {
            let start = from + found;
            let rest = &text[start..];
            let scheme = if rest.starts_with("https://") {
                8
            } else if rest.starts_with("http://") {
                7
            } else {
                from = start + 4;
                continue;
            };
            from = start + scheme;
            if text[..start].chars().next_back().is_some_and(char::is_alphanumeric) {
                continue;
            }
            let body = &rest[scheme..];
            let mut end = start
                + scheme
                + body.find(|c: char| c.is_whitespace() || c == '<').unwrap_or(body.len());
            while let Some(last) = text[start..end].chars().next_back() {
                let url = &text[start..end];
                let unbalanced = last == ')' && url.matches('(').count() < url.matches(')').count();
                if !TRAILING.contains(&last) && !unbalanced {
                    break;
                }
                end -= last.len_utf8();
            }
            from = end.max(from);
            if text[start + scheme..end].starts_with(char::is_alphanumeric) {
                return Some(start..end);
            }
        }
        None
    })
}

/// Adds `style` to every run over `range`, splitting runs at its ends and filling gaps with runs
/// of `style` alone. `runs` stays sorted and non-overlapping.
fn add_style(runs: &mut Vec<StyleRun>, range: &Range<usize>, style: Style) {
    let mut out = Vec::with_capacity(runs.len() + 3);
    let mut cursor = range.start;
    for run in runs.drain(..) {
        if run.range.end <= range.start {
            out.push(run);
            continue;
        }
        if run.range.start >= range.end {
            if cursor < range.end {
                out.push(StyleRun { range: cursor..range.end, style });
                cursor = range.end;
            }
            out.push(run);
            continue;
        }
        if run.range.start < range.start {
            out.push(StyleRun { range: run.range.start..range.start, style: run.style });
        }
        let inner = run.range.start.max(range.start)..run.range.end.min(range.end);
        if cursor < inner.start {
            out.push(StyleRun { range: cursor..inner.start, style });
        }
        cursor = inner.end;
        out.push(StyleRun { range: inner, style: run.style | style });
        if run.range.end > range.end {
            out.push(StyleRun { range: range.end..run.range.end, style: run.style });
        }
    }
    if cursor < range.end {
        out.push(StyleRun { range: cursor..range.end, style });
    }
    *runs = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<&str> {
        urls(text).map(|r| &text[r]).collect()
    }

    #[test]
    fn urls_end_before_trailing_punctuation_and_unbalanced_parens() {
        assert_eq!(found("see https://x.dev/a?b=1#c."), ["https://x.dev/a?b=1#c"]);
        assert_eq!(
            found("(https://en.wikipedia.org/wiki/Rust_(language))"),
            ["https://en.wikipedia.org/wiki/Rust_(language)"]
        );
        assert_eq!(found("\"http://x.dev\", then"), ["http://x.dev"]);
        assert_eq!(
            found("a https://x.dev<b> and https://y.dev"),
            ["https://x.dev", "https://y.dev"]
        );
    }

    #[test]
    fn not_urls() {
        assert!(found("xhttps://x.dev http:// https://.x http://- https").is_empty());
    }

    #[test]
    fn styles_merge_over_existing_runs() {
        let mut runs = vec![
            StyleRun { range: 0..4, style: Style::STRONG },
            StyleRun { range: 10..12, style: Style::EMPHASIS },
        ];
        add_style(&mut runs, &(2..11), Style::LINK);
        let got: Vec<_> = runs.iter().map(|r| (r.range.clone(), r.style)).collect();
        assert_eq!(
            got,
            [
                (0..2, Style::STRONG),
                (2..4, Style::STRONG | Style::LINK),
                (4..10, Style::LINK),
                (10..11, Style::EMPHASIS | Style::LINK),
                (11..12, Style::EMPHASIS),
            ]
        );
    }
}
