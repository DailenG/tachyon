//! `Ctrl+click` (`Cmd+click` on macOS) on a link opens it: web and mail links in the system's
//! handler, links to Markdown and text files in Tachyon. Anything else (other schemes, other local
//! files, which could be programs) is ignored.

use std::path::{Path, PathBuf};

use gpui::{App, Context};

use crate::editor::{Editor, OpenPaths};

/// What following a link does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    /// Opened by the system (browser, mail client).
    Url(String),
    /// A document opened in Tachyon.
    File(PathBuf),
}

/// Schemes handed to the system.
const URL_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Extensions of files Tachyon opens itself.
const DOCUMENT_EXTENSIONS: [&str; 5] = ["md", "markdown", "mdown", "mkd", "txt"];

/// Resolves a link destination. Relative paths are relative to `document` (the file being
/// edited); a scratch buffer has no base, so they resolve to nothing.
pub(crate) fn resolve(dest: &str, document: Option<&Path>) -> Option<LinkTarget> {
    let dest = dest.trim();
    if dest.is_empty() || dest.starts_with('#') {
        return None;
    }
    if let Some((scheme, rest)) = split_scheme(dest) {
        return if URL_SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)) {
            Some(LinkTarget::Url(dest.to_owned()))
        } else if scheme.eq_ignore_ascii_case("file") {
            let path = rest.strip_prefix("//").unwrap_or(rest);
            // `file:///C:/x` has a slash before the drive letter.
            let path = match path.as_bytes() {
                [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
                _ => path,
            };
            document_file(Path::new(&percent_decode(strip_suffix(path))))
        } else {
            None
        };
    }
    let path = PathBuf::from(percent_decode(strip_suffix(dest)));
    let path = if path.is_absolute() { path } else { document?.parent()?.join(path) };
    document_file(&path)
}

/// `scheme` and the rest of `dest`, if it starts with a URL scheme. A single letter is a Windows
/// drive (`C:\notes.md`), not a scheme.
fn split_scheme(dest: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = dest.split_once(':')?;
    let valid = scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some((scheme, rest))
}

/// `path` without a `#fragment` or `?query`.
fn strip_suffix(path: &str) -> &str {
    path.split(['#', '?']).next().unwrap_or(path)
}

/// Decodes `%XX` escapes (`my%20notes.md`); invalid escapes stay as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(&hi), Some(&lo)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(hi), Some(lo)) = (hex(hi), hex(lo))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extensions of images shown in rendered blocks.
const IMAGE_EXTENSIONS: [&str; 9] =
    ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tif", "tiff"];

/// The local file an image destination names, resolved like a link to a document (relative to
/// `document`, absolute, or `file:`). Remote images (`http:` …) are not loaded: GPUI would need an
/// HTTP client.
pub(crate) fn image_path(dest: &str, document: Option<&Path>) -> Option<PathBuf> {
    let dest = dest.trim();
    let path = match split_scheme(dest) {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("file") => {
            let path = rest.strip_prefix("//").unwrap_or(rest);
            let path = match path.as_bytes() {
                [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
                _ => path,
            };
            PathBuf::from(percent_decode(strip_suffix(path)))
        }
        Some(_) => return None,
        None => {
            let path = PathBuf::from(percent_decode(strip_suffix(dest)));
            if path.is_absolute() { path } else { document?.parent()?.join(path) }
        }
    };
    let extension = path.extension()?.to_str()?;
    IMAGE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(extension)).then_some(path)
}

fn document_file(path: &Path) -> Option<LinkTarget> {
    let extension = path.extension()?.to_str()?;
    DOCUMENT_EXTENSIONS
        .iter()
        .any(|e| e.eq_ignore_ascii_case(extension))
        .then(|| LinkTarget::File(path.to_owned()))
}

impl Editor {
    /// Destination of the link at source `offset`: on its text, rendered or raw.
    pub(crate) fn link_at(&self, offset: usize) -> Option<&str> {
        let index = self.doc.block_at(offset)?;
        let start = self.doc.block_range(index).start;
        let ir = &self.doc.blocks()[index].parsed().ir;
        let visible = ir.source_to_visible(offset.checked_sub(start)?);
        ir.links.iter().find(|link| link.visible.contains(&visible)).map(|link| link.dest.as_str())
    }

    /// Follows the link at `offset`, if there is one. Returns whether it did.
    pub(crate) fn follow_link(&mut self, offset: usize, cx: &mut Context<Self>) -> bool {
        let Some(dest) = self.link_at(offset) else { return false };
        match resolve(dest, self.file()) {
            Some(LinkTarget::Url(url)) => cx.open_url(&url),
            Some(LinkTarget::File(path)) => {
                cx.defer(move |cx: &mut App| {
                    if let Some(open) = cx.try_global::<OpenPaths>().map(|o| o.0.clone()) {
                        open(vec![path], cx);
                    }
                });
            }
            None => {}
        }
        // A link that resolves to nothing still is not a place to put the caret by Ctrl+click.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Option<LinkTarget> {
        Some(LinkTarget::Url(s.to_owned()))
    }

    fn file(s: &str) -> Option<LinkTarget> {
        Some(LinkTarget::File(PathBuf::from(s)))
    }

    #[test]
    fn web_and_mail_links_go_to_the_system() {
        assert_eq!(resolve("https://example.com/a?b#c", None), url("https://example.com/a?b#c"));
        assert_eq!(resolve(" HTTP://x.dev ", None), url("HTTP://x.dev"));
        assert_eq!(resolve("mailto:me@x.dev", None), url("mailto:me@x.dev"));
    }

    #[test]
    fn other_schemes_and_anchors_do_nothing() {
        for dest in ["javascript:alert(1)", "ftp://x", "ms-settings:", "#heading", "", "  "] {
            assert_eq!(resolve(dest, None), None, "{dest:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn relative_documents_resolve_against_the_file() {
        let doc = Some(Path::new("/notes/today.md"));
        assert_eq!(resolve("ideas.md", doc), file("/notes/ideas.md"));
        assert_eq!(
            resolve("../old/My%20List.markdown#top", doc),
            file("/notes/../old/My List.markdown")
        );
        assert_eq!(resolve("/abs/readme.TXT", None), file("/abs/readme.TXT"));
        assert_eq!(resolve("file:///abs/x.md", None), file("/abs/x.md"));
        assert_eq!(resolve("ideas.md", None), None, "a scratch buffer has no base");
    }

    #[cfg(unix)]
    #[test]
    fn images_resolve_to_local_files_only() {
        let doc = Some(Path::new("/notes/today.md"));
        assert_eq!(image_path("img/a%20b.png", doc), Some(PathBuf::from("/notes/img/a b.png")));
        assert_eq!(image_path("file:///pics/x.JPG", None), Some(PathBuf::from("/pics/x.JPG")));
        for dest in ["https://x.dev/a.png", "data:image/png;base64,AAAA", "notes.md", "a.png"] {
            let document = if dest == "a.png" { None } else { doc };
            assert_eq!(image_path(dest, document), None, "{dest:?}");
        }
    }

    #[test]
    fn local_files_that_are_not_documents_are_ignored() {
        let doc = Some(Path::new("/notes/today.md"));
        for dest in
            ["setup.exe", "run.sh", "image.png", "folder/", "file:///C:/Windows/notepad.exe"]
        {
            assert_eq!(resolve(dest, doc), None, "{dest:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_not_schemes() {
        assert_eq!(resolve(r"C:\notes\a.md", None), file(r"C:\notes\a.md"));
        assert_eq!(resolve("file:///C:/notes/a.md", None), file("C:/notes/a.md"));
    }
}
