use std::borrow::Cow;

/// Line ending used when writing the buffer back out. The buffer itself
/// always holds `\n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    /// The dominant line ending in `text`; ties and text without line breaks
    /// resolve to [`LineEnding::Lf`].
    pub fn detect(text: &str) -> Self {
        let bytes = text.as_bytes();
        let (mut crlf, mut lf) = (0usize, 0usize);
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                if i > 0 && bytes[i - 1] == b'\r' {
                    crlf += 1;
                } else {
                    lf += 1;
                }
            }
        }
        if crlf > lf { LineEnding::CrLf } else { LineEnding::Lf }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// Converts `\r\n` and lone `\r` (both line endings in CommonMark) to `\n`.
/// Borrows when there is nothing to convert.
pub fn normalize(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dominant_line_ending() {
        assert_eq!(LineEnding::detect("a\r\nb\r\nc\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\nb\r\nc\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("no breaks"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("\r\n"), LineEnding::CrLf);
    }

    #[test]
    fn normalizes_crlf_and_lone_cr() {
        assert_eq!(normalize("a\r\nb\rc\nd\r"), "a\nb\nc\nd\n");
        assert!(matches!(normalize("plain\ntext"), Cow::Borrowed(_)));
        assert_eq!(normalize("\r\r\n"), "\n\n");
    }
}
