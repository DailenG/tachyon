//! Streaming decode helpers for [`crate::Buffer::load`]: normalizing line endings and replacing
//! invalid UTF-8 one read chunk at a time, so the caller never holds the whole input as one
//! contiguous `String`. Kept separate from `lib.rs` because the state a streamed decode carries
//! across chunk boundaries (a trailing `\r`, an incomplete UTF-8 sequence) is fiddly enough to
//! deserve its own tests.

/// Read buffer size: large enough that a multi-gigabyte file needs few syscalls, small enough
/// that it is not itself a large allocation next to the rope being built.
pub(crate) const READ_CHUNK: usize = 1024 * 1024;

/// Bytes checked for a NUL byte (binary-file signal), the same convention as most text-vs-binary
/// heuristics (`file`, Git's `core.autocrlf` detection).
pub(crate) const BINARY_SNIFF_LEN: usize = 8 * 1024;

/// Carries state across `read` calls: line-ending counts (for [`crate::LineEnding::detect`]'s
/// streamed equivalent), a `\r` seen at the very end of the last chunk whose pairing with a `\n`
/// is not yet known, and whether anything so far was invalid UTF-8.
#[derive(Default)]
pub(crate) struct Decoder {
    pub(crate) crlf: usize,
    pub(crate) lf: usize,
    pending_cr: bool,
    pub(crate) lossy: bool,
}

impl Decoder {
    /// Appends `s` (valid UTF-8) to `out`, normalizing `\r\n` and lone `\r` to `\n` and counting
    /// which ending each resolved as. A `\r` at the very end of `s` is held as `pending_cr`
    /// instead of resolved immediately: it might be the first half of a `\r\n` split across this
    /// read (or across an invalid-UTF-8 boundary within it), and resolving it now could double a
    /// line break that `finish` or the next call will otherwise correctly turn into one `\n`.
    pub(crate) fn push(&mut self, out: &mut String, s: &str) {
        let mut chars = s.chars().peekable();
        if self.pending_cr {
            self.pending_cr = false;
            if chars.peek() == Some(&'\n') {
                chars.next();
                self.crlf += 1;
            } else {
                self.lf += 1;
            }
            out.push('\n');
        }
        while let Some(c) = chars.next() {
            match c {
                '\r' if chars.peek().is_none() => self.pending_cr = true,
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                        self.crlf += 1;
                    } else {
                        self.lf += 1;
                    }
                    out.push('\n');
                }
                '\n' => {
                    self.lf += 1;
                    out.push('\n');
                }
                c => out.push(c),
            }
        }
    }

    /// Resolves a `\r` left pending by the last chunk (a lone `\r` at end of file) and reports
    /// the dominant line ending, same tie-break as [`crate::LineEnding::detect`].
    pub(crate) fn finish(&mut self, out: &mut String) -> crate::LineEnding {
        if self.pending_cr {
            self.lf += 1;
            out.push('\n');
        }
        if self.crlf > self.lf { crate::LineEnding::CrLf } else { crate::LineEnding::Lf }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&str]) -> (String, crate::LineEnding, bool) {
        let mut decoder = Decoder::default();
        let mut out = String::new();
        for chunk in chunks {
            decoder.push(&mut out, chunk);
        }
        let lossy = decoder.lossy;
        let ending = decoder.finish(&mut out);
        (out, ending, lossy)
    }

    #[test]
    fn normalizes_within_one_chunk() {
        let (text, ending, _) = run(&["a\r\nb\r\nc\r\nd\rz\n"]);
        assert_eq!(text, "a\nb\nc\nd\nz\n");
        assert_eq!(ending, crate::LineEnding::CrLf);
    }

    #[test]
    fn a_crlf_split_across_chunks_counts_once_as_crlf() {
        let (text, ending, _) = run(&["a\r", "\nb\r", "\nc"]);
        assert_eq!(text, "a\nb\nc");
        assert_eq!(ending, crate::LineEnding::CrLf);
    }

    #[test]
    fn a_lone_cr_at_end_of_input_is_one_line_ending() {
        let (text, ending, _) = run(&["a\r"]);
        assert_eq!(text, "a\n");
        assert_eq!(ending, crate::LineEnding::Lf);
    }

    #[test]
    fn a_cr_at_a_chunk_end_not_followed_by_lf_is_lone() {
        let (text, ending, _) = run(&["a\r", "b\n"]);
        assert_eq!(text, "a\nb\n");
        assert_eq!(ending, crate::LineEnding::Lf);
    }
}
