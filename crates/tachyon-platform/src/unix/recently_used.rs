//! The freedesktop "recently used" list (`~/.local/share/recently-used.xbel`), a
//! `bookmark:`/`mime:`-namespaced XBEL document GTK and Qt file choosers read as their own
//! "Recent" list: <https://www.freedesktop.org/wiki/Specifications/desktop-bookmark-spec/>.
//!
//! This only ever touches Tachyon's own entry (found by its `href`, an escaped `file://` URI of
//! the opened path): any other application's bookmarks in the file are copied through byte for
//! byte, untouched. XBEL bookmarks never nest, so locating one - and the two document markers
//! (`<xbel ...>`'s implied start, `</xbel>`'s close) this needs to safely insert or replace it -
//! is a handful of substring searches, not a general XML parser. If the existing file does not
//! contain a `</xbel>` close (missing, or some other format entirely), [`add`] leaves it alone
//! rather than guess at content an unrelated writer left there; a brand new file is created only
//! when none exists yet.
//!
//! No timestamp crate: `added`/`modified`/`visited` need only whole-second UTC ISO 8601, so
//! [`civil_from_days`] (Howard Hinnant's public-domain `civil_from_days`, the algorithm nearly
//! every date library's UNIX-epoch conversion is built on) is the entire calendar this needs.

use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::unix::data_home;

/// The `bookmark:application` entry Tachyon writes for itself. Matches the App Execution
/// Alias/terminal command name (`crates/tachyon/src/main.rs`'s default `instance_id`), not the
/// display name: this `exec` string is informational metadata other tools may read, not something
/// Tachyon itself parses back.
const APP_NAME: &str = "tachyon";

/// Adds or refreshes `file`'s entry. Best-effort: any I/O error (missing `$HOME`, a read-only
/// `$XDG_DATA_HOME`) is swallowed by the caller (`super::note_recently_used`) the same way a
/// failed recent-file write already is - this is a convenience surface, not a data path Tachyon's
/// own correctness depends on.
pub(super) fn add(file: &Path) -> io::Result<()> {
    let path = xbel_path()?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let Some(text) = upsert(existing.as_deref(), file) else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomically(&path, &text)
}

/// `$XDG_DATA_HOME/recently-used.xbel` (default `~/.local/share`).
fn xbel_path() -> io::Result<PathBuf> {
    Ok(data_home()?.join("recently-used.xbel"))
}

fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("xbel.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// `existing`'s text with `file`'s bookmark added or refreshed, or `None` if `existing` is
/// present but does not look like a bookmark file this can safely rewrite (see the module doc
/// comment) - `add` then leaves the file untouched. `None` for `existing` (the file does not
/// exist yet) always produces a fresh minimal document.
fn upsert(existing: Option<&str>, file: &Path) -> Option<String> {
    let href = file_uri(file);
    let now = now_iso8601();
    let Some(text) = existing else { return Some(fresh_document(&href, file, &now, &now)) };
    let close = text.find("</xbel>")?;

    let needle = format!("href=\"{}\"", xml_escape(&href));
    let removed = text[..close].find(&needle).and_then(|at| {
        let start = text[..at].rfind("<bookmark")?;
        // The match is inside `text[..close]`, so `</bookmark>` (if present at all) is found
        // within that same bound - never past `close` - by construction of `str::find`.
        let end = at + text[at..close].find("</bookmark>")? + "</bookmark>".len();
        Some((start, end))
    });
    let (head, added) = match removed {
        Some((start, end)) => {
            let added = extract_attr(&text[start..end], "added").unwrap_or_else(|| now.clone());
            (format!("{}{}", &text[..start], &text[end..close]), added)
        }
        None => (text[..close].to_owned(), now.clone()),
    };
    Some(format!("{head}{}{}", bookmark_xml(&href, file, &added, &now), &text[close..]))
}

/// A whole, valid `recently-used.xbel` document containing only `href`'s bookmark: written when
/// none exists yet.
fn fresh_document(href: &str, file: &Path, added: &str, modified: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\" \
         xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" \
         xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n{}</xbel>\n",
        bookmark_xml(href, file, added, modified)
    )
}

/// One `<bookmark>` element: `href` is already a `file://` URI ([`file_uri`]); `added`/`modified`
/// are already-formatted ISO 8601 timestamps ([`now_iso8601`]), `visited` reuses `modified` since
/// Tachyon has no separate "opened without changing" event to distinguish them.
fn bookmark_xml(href: &str, file: &Path, added: &str, modified: &str) -> String {
    let href = xml_escape(href);
    let mime = mime_type_for(file);
    format!(
        "<bookmark href=\"{href}\" added=\"{added}\" modified=\"{modified}\" visited=\"{modified}\">\n\
<info>\n<metadata owner=\"http://www.freedesktop.org\">\n<mime:mime-type type=\"{mime}\"/>\n\
<bookmark:applications>\n<bookmark:application name=\"{APP_NAME}\" exec=\"&apos;tachyon&apos; %u\" \
modified=\"{modified}\" count=\"1\"/>\n</bookmark:applications>\n</metadata>\n</info>\n</bookmark>\n"
    )
}

/// `block`'s `name="..."` attribute value, unescaped: every value this module ever wrote there is
/// a plain ISO 8601 timestamp with no XML-special characters, so no unescaping is needed to read
/// one back.
fn extract_attr(block: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let at = block.find(&needle)? + needle.len();
    let end = block[at..].find('"')?;
    Some(block[at..at + end].to_owned())
}

/// The Markdown/plain-text distinction Tachyon itself draws (`tachyon_doc::mode_for_extension`):
/// this only ever opens one of those two kinds of document, so there is no broader MIME sniffing
/// to do.
fn mime_type_for(file: &Path) -> &'static str {
    const MARKDOWN_EXTENSIONS: [&str; 6] = ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];
    let markdown = file
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| MARKDOWN_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
    if markdown { "text/markdown" } else { "text/plain" }
}

/// `file` as a `file://` URI: percent-encodes every byte outside the unreserved set (`A-Za-z0-9`,
/// `-_.~`) and the path separator `/`, per RFC 3986. Reads the path's raw bytes
/// (`OsStrExt::as_bytes`, not `char`s), since a Linux path is not required to be valid UTF-8 and
/// this must still round-trip one. `std::path::absolute` resolves a relative path against the
/// current directory (lexically; no filesystem access, so it cannot fail on a path that does not
/// exist) so the URI is always well-formed - a bare `file:///a/b`, never `file://a/b`.
fn file_uri(file: &Path) -> String {
    let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let mut uri = String::from("file://");
    for &byte in absolute.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                uri.push(byte as char);
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// Escapes `&`, `<`, `>` and `"` for use inside an XML attribute value.
fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn now_iso8601() -> String {
    format_iso8601(SystemTime::now())
}

fn format_iso8601(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (time_of_day / 3600, (time_of_day % 3600) / 60, time_of_day % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's `civil_from_days`: the proleptic-Gregorian civil date for `z`, the number of
/// days since the Unix epoch (1970-01-01). Public-domain algorithm from
/// <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>, ported as-is (only the
/// integer types changed to Rust's) rather than pulling in a date/time crate for one timestamp
/// format.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29), "2024 is a leap year");
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31), "the day before the epoch");
        assert_eq!(civil_from_days(24_855), (2038, 1, 19));
    }

    #[test]
    fn format_iso8601_matches_a_known_instant() {
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_709_251_199);
        assert_eq!(format_iso8601(time), "2024-02-29T23:59:59Z");
    }

    #[test]
    fn xml_escape_covers_the_five_predefined_entities_this_module_needs() {
        assert_eq!(xml_escape("a & b <c> \"d\""), "a &amp; b &lt;c&gt; &quot;d&quot;");
    }

    #[test]
    fn file_uri_percent_encodes_spaces_and_keeps_path_separators() {
        assert_eq!(
            file_uri(Path::new("/home/user/My Notes.md")),
            "file:///home/user/My%20Notes.md"
        );
    }

    #[test]
    fn mime_type_for_recognizes_markdown_extensions_case_insensitively() {
        assert_eq!(mime_type_for(Path::new("a.md")), "text/markdown");
        assert_eq!(mime_type_for(Path::new("a.MARKDOWN")), "text/markdown");
        assert_eq!(mime_type_for(Path::new("a.txt")), "text/plain");
        assert_eq!(mime_type_for(Path::new("a.log")), "text/plain");
        assert_eq!(mime_type_for(Path::new("a")), "text/plain");
    }

    #[test]
    fn upsert_creates_a_fresh_document_when_none_exists() {
        let text = upsert(None, Path::new("/tmp/a.md")).expect("a fresh document");
        assert!(text.starts_with("<?xml"));
        assert!(text.contains("href=\"file:///tmp/a.md\""));
        assert!(text.trim_end().ends_with("</xbel>"));
    }

    #[test]
    fn upsert_inserts_before_the_closing_tag_and_keeps_unrelated_bookmarks() {
        let existing = "<?xml version=\"1.0\"?>\n<xbel version=\"1.0\">\n\
            <bookmark href=\"file:///tmp/other.txt\" added=\"2020-01-01T00:00:00Z\" \
            modified=\"2020-01-01T00:00:00Z\" visited=\"2020-01-01T00:00:00Z\"><info/></bookmark>\n\
            </xbel>\n";
        let text = upsert(Some(existing), Path::new("/tmp/a.md")).expect("an upsert");
        assert!(
            text.contains("file:///tmp/other.txt"),
            "an unrelated bookmark must survive untouched"
        );
        assert!(text.contains("file:///tmp/a.md"));
        assert_eq!(text.matches("</bookmark>").count(), 2);
    }

    #[test]
    fn upsert_replaces_its_own_entry_in_place_and_keeps_the_original_added_time() {
        let existing = "<xbel version=\"1.0\">\n\
            <bookmark href=\"file:///tmp/a.md\" added=\"2020-01-01T00:00:00Z\" \
            modified=\"2020-01-01T00:00:00Z\" visited=\"2020-01-01T00:00:00Z\"><info/></bookmark>\n\
            </xbel>\n";
        let text = upsert(Some(existing), Path::new("/tmp/a.md")).expect("an upsert");
        assert_eq!(
            text.matches("file:///tmp/a.md").count(),
            1,
            "the stale entry must be replaced, not duplicated"
        );
        assert!(
            text.contains("added=\"2020-01-01T00:00:00Z\""),
            "the original added time must be kept"
        );
        assert!(
            !text.contains("modified=\"2020-01-01T00:00:00Z\""),
            "modified/visited must be refreshed"
        );
    }

    #[test]
    fn upsert_skips_content_with_no_closing_xbel_tag() {
        assert_eq!(upsert(Some("not an xbel file"), Path::new("/tmp/a.md")), None);
    }
}
