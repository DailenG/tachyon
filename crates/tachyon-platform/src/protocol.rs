//! Wire format for forwarded launches.
//!
//! Request: `MAGIC`, body length as `u32` little-endian, then the body: each
//! argument as UTF-8 terminated by a NUL byte. Response: `ACK` once the
//! primary has accepted the launch.
//!
//! The request is length-framed rather than terminated by end-of-stream
//! because Windows pipes have no half-close; the acknowledgement lets the
//! sender know the launch was delivered, so it can start standalone instead of
//! losing it when the primary is gone or unresponsive.

use std::io::{self, Read, Write};

const MAGIC: &[u8] = b"TACHYON2\n";

/// Sent by the primary after it has accepted a request.
pub const ACK: &[u8; 2] = b"OK";

/// Upper bound on an accepted body; protects the primary from a hostile or
/// broken peer.
pub const MAX_BODY_LEN: usize = 1 << 20;

pub fn encode(args: &[String]) -> Vec<u8> {
    let body_len: usize = args.iter().map(|a| a.len() + 1).sum();
    let mut buf = Vec::with_capacity(MAGIC.len() + 4 + body_len);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&(body_len as u32).to_le_bytes());
    for arg in args {
        buf.extend_from_slice(arg.as_bytes());
        buf.push(0);
    }
    buf
}

fn decode_body(body: &[u8]) -> Option<Vec<String>> {
    if body.is_empty() {
        return Some(Vec::new());
    }
    let body = body.strip_suffix(&[0])?;
    body.split(|&b| b == 0).map(|arg| String::from_utf8(arg.to_vec()).ok()).collect()
}

/// Reads one request. `Ok(None)` means the peer sent something that is not a
/// valid request; I/O failures (including a truncated frame) are errors.
pub fn read_request(mut stream: impl Read) -> io::Result<Option<Vec<String>>> {
    let mut magic = [0u8; MAGIC.len()];
    stream.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Ok(None);
    }
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_BODY_LEN {
        return Ok(None);
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(decode_body(&body))
}

/// Sends a request and waits for the acknowledgement.
pub fn send_request(mut stream: impl Read + Write, request: &[u8]) -> io::Result<()> {
    stream.write_all(request)?;
    stream.flush()?;
    let mut ack = [0u8; ACK.len()];
    stream.read_exact(&mut ack)?;
    if &ack == ACK {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "primary instance sent an invalid reply"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8]) -> io::Result<Option<Vec<String>>> {
        read_request(bytes)
    }

    #[test]
    fn round_trips_arguments_including_empty_and_non_ascii() {
        let args: Vec<String> = ["--paste", "", "C:\\Users\\zoë\\notes.md", "a b\nc"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(read(&encode(&args)).unwrap(), Some(args));
        assert_eq!(read(&encode(&[])).unwrap(), Some(Vec::new()));
    }

    #[test]
    fn reads_exactly_one_frame_from_a_stream() {
        let mut stream = encode(&["a.md".into()]);
        stream.extend_from_slice(b"trailing bytes from a confused peer");
        assert_eq!(read(&stream).unwrap(), Some(vec!["a.md".to_owned()]));
    }

    #[test]
    fn rejects_foreign_or_malformed_requests() {
        assert_eq!(read(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap(), None);
        let mut invalid_utf8 = MAGIC.to_vec();
        invalid_utf8.extend_from_slice(&2u32.to_le_bytes());
        invalid_utf8.extend_from_slice(&[0xff, 0x00]);
        assert_eq!(read(&invalid_utf8).unwrap(), None);
        let mut unterminated = MAGIC.to_vec();
        unterminated.extend_from_slice(&1u32.to_le_bytes());
        unterminated.push(b'a');
        assert_eq!(read(&unterminated).unwrap(), None);
    }

    #[test]
    fn truncated_frame_is_an_error() {
        let mut truncated = encode(&["file.md".into()]);
        truncated.pop();
        assert_eq!(read(&truncated).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn rejects_oversized_body_without_reading_it() {
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(&(MAX_BODY_LEN as u32 + 1).to_le_bytes());
        assert_eq!(read(&header).unwrap(), None);
    }
}
