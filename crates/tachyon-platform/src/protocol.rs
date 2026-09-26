//! Wire format for forwarded launches: `MAGIC` followed by each argument as
//! UTF-8 terminated by a NUL byte. The sender closes its write half to mark
//! the end of the message.

use std::io::{self, Read};

const MAGIC: &[u8] = b"TACHYON1\n";

/// Upper bound on an accepted message; protects the primary from a peer that
/// never stops writing.
pub const MAX_MESSAGE_LEN: u64 = 1 << 20;

pub fn encode(args: &[String]) -> Vec<u8> {
    let len = MAGIC.len() + args.iter().map(|a| a.len() + 1).sum::<usize>();
    let mut buf = Vec::with_capacity(len);
    buf.extend_from_slice(MAGIC);
    for arg in args {
        buf.extend_from_slice(arg.as_bytes());
        buf.push(0);
    }
    buf
}

pub fn decode(buf: &[u8]) -> Option<Vec<String>> {
    let body = buf.strip_prefix(MAGIC)?;
    if body.is_empty() {
        return Some(Vec::new());
    }
    let body = body.strip_suffix(&[0])?;
    body.split(|&b| b == 0).map(|arg| String::from_utf8(arg.to_vec()).ok()).collect()
}

/// Reads one complete message from `stream`, bounded by [`MAX_MESSAGE_LEN`].
pub fn read_message(stream: impl Read) -> io::Result<Option<Vec<String>>> {
    let mut buf = Vec::new();
    stream.take(MAX_MESSAGE_LEN + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_MESSAGE_LEN {
        return Ok(None);
    }
    Ok(decode(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_arguments_including_empty_and_non_ascii() {
        let args: Vec<String> = ["--paste", "", "C:\\Users\\zoë\\notes.md", "a b\nc"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(decode(&encode(&args)), Some(args));
        assert_eq!(decode(&encode(&[])), Some(Vec::new()));
    }

    #[test]
    fn rejects_foreign_or_truncated_messages() {
        let mut truncated = encode(&["file.md".into()]);
        truncated.pop();
        assert_eq!(decode(&truncated), None);
        assert_eq!(decode(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(decode(&[MAGIC, &[0xff, 0x00]].concat()), None);
    }

    #[test]
    fn rejects_oversized_messages() {
        let huge = vec!["x".repeat(MAX_MESSAGE_LEN as usize)];
        assert_eq!(read_message(encode(&huge).as_slice()).unwrap(), None);
    }
}
