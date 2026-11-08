//! Converting between the editor's positions and paths and the language
//! server protocol's.
//!
//! LSP positions are a line and a column counted in UTF-16 code units, or
//! in bytes (UTF-8) when the server agrees to it at start-up; the editor
//! counts columns in chars. Files are `file://` URIs.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::core::{Buffer, Position};

/// How a server counts columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
}

impl Encoding {
    /// The encoding a server chose (`capabilities.positionEncoding`).
    pub fn from_name(name: Option<&str>) -> Encoding {
        match name {
            Some("utf-8") => Encoding::Utf8,
            _ => Encoding::Utf16,
        }
    }
}

/// `pos` as an LSP position in `buffer`.
pub fn to_lsp(buffer: &Buffer, pos: Position, encoding: Encoding) -> Value {
    let line = buffer.line(pos.row.min(buffer.line_count() - 1));
    let character: usize = line
        .chars()
        .take(pos.col)
        .map(|c| match encoding {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        })
        .sum();
    json!({ "line": pos.row, "character": character })
}

/// An LSP position (`{line, character}`) in `buffer`, clamped into it.
pub fn from_lsp(buffer: &Buffer, position: &Value, encoding: Encoding) -> Position {
    let row = position["line"].as_u64().unwrap_or(0) as usize;
    let character = position["character"].as_u64().unwrap_or(0) as usize;
    if row >= buffer.line_count() {
        let last = buffer.line_count() - 1;
        return buffer.line_end(last);
    }
    let line = buffer.line(row);
    let mut units = 0;
    let mut col = 0;
    for c in line.chars() {
        if units >= character {
            break;
        }
        units += match encoding {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        };
        col += 1;
    }
    Position::new(row, col)
}

/// The `file://` URI for `path` (made absolute).
pub fn path_to_uri(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut uri = String::from("file://");
    for byte in absolute.to_string_lossy().bytes() {
        let keep = byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte);
        if keep {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The path a `file://` URI names, if it is one.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(rest.get(i + 1..i + 3)?, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_in_utf16_and_utf8() {
        let buffer = Buffer::from_text("a😀é b");
        // 😀 is 2 UTF-16 units and 4 bytes; é 1 unit and 2 bytes.
        let pos = Position::new(0, 3);
        assert_eq!(
            to_lsp(&buffer, pos, Encoding::Utf16),
            json!({"line": 0, "character": 4})
        );
        assert_eq!(
            to_lsp(&buffer, pos, Encoding::Utf8),
            json!({"line": 0, "character": 7})
        );
        let back = |v: Value, e| from_lsp(&buffer, &v, e);
        assert_eq!(
            back(json!({"line": 0, "character": 4}), Encoding::Utf16),
            pos
        );
        assert_eq!(
            back(json!({"line": 0, "character": 7}), Encoding::Utf8),
            pos
        );
        assert_eq!(
            back(json!({"line": 9, "character": 0}), Encoding::Utf16),
            Position::new(0, 5),
            "past the end: clamped"
        );
    }

    #[test]
    fn uris_round_trip() {
        let path = Path::new("/tmp/a dir/é.rs");
        let uri = path_to_uri(path);
        assert_eq!(uri, "file:///tmp/a%20dir/%C3%A9.rs");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
        assert_eq!(uri_to_path("https://x"), None);
    }
}
