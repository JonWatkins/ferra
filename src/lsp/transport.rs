//! The language server protocol's framing: each message is JSON after a
//! `Content-Length: <bytes>` header and a blank line, over the server's
//! stdin and stdout.

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// Writes `message` with its header.
pub fn write_message(out: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    out.flush()
}

/// Reads the next message, or `None` at the end of the stream. Headers
/// other than `Content-Length` are skipped, as the spec allows.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.expect("checked before breaking");
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_round_trip() {
        let mut wire = Vec::new();
        write_message(
            &mut wire,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "x"}),
        )
        .unwrap();
        write_message(&mut wire, &json!({"jsonrpc": "2.0", "method": "é"})).unwrap();
        let text = String::from_utf8(wire.clone()).unwrap();
        assert!(text.starts_with("Content-Length: 37\r\n\r\n{"), "{text}");
        let mut input = io::BufReader::new(&wire[..]);
        assert_eq!(read_message(&mut input).unwrap().unwrap()["id"], 1);
        assert_eq!(read_message(&mut input).unwrap().unwrap()["method"], "é");
        assert!(read_message(&mut input).unwrap().is_none(), "end of stream");
    }

    #[test]
    fn other_headers_are_skipped() {
        let wire = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}";
        let mut input = io::BufReader::new(&wire[..]);
        assert_eq!(read_message(&mut input).unwrap().unwrap(), json!({}));
    }
}
