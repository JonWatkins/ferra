//! Converting between a file's bytes on disk and the buffer's text.
//!
//! The buffer is always UTF-8, always uses `\n` and never stores the file's
//! final newline. [`TextFormat`] remembers what the file actually had (its
//! encoding and byte-order mark, line endings, final newline), so saving
//! writes it back exactly: a file round-trips byte for byte unless it's
//! edited.

pub use encoding_rs::Encoding;
use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// The encoding used for files that have no byte-order mark and aren't
/// valid UTF-8, unless configured otherwise. Every byte means something
/// in it, so it never fails (it's a superset of Latin-1).
pub const DEFAULT_FALLBACK: &Encoding = WINDOWS_1252;

/// A file's character encoding, and whether it starts with a byte-order
/// mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileEncoding {
    pub encoding: &'static Encoding,
    pub bom: bool,
}

impl Default for FileEncoding {
    fn default() -> Self {
        Self {
            encoding: UTF_8,
            bom: false,
        }
    }
}

impl FileEncoding {
    /// The encoding named `label`: any WHATWG label (`latin1`,
    /// `windows-1252`, `shift_jis`, `utf-16le`), or `utf-8-bom`. UTF-16 is
    /// always written with a byte-order mark.
    pub fn from_label(label: &str) -> Option<Self> {
        let label = label.trim().to_ascii_lowercase();
        if label == "utf-8-bom" || label == "utf8-bom" {
            return Some(Self {
                encoding: UTF_8,
                bom: true,
            });
        }
        let encoding = Encoding::for_label(label.as_bytes())?;
        Some(Self {
            encoding,
            bom: encoding == UTF_16LE || encoding == UTF_16BE,
        })
    }

    /// `UTF-8`, `UTF-8 BOM`, `windows-1252`, ...
    pub fn name(&self) -> String {
        if self.bom && self.encoding == UTF_8 {
            "UTF-8 BOM".to_string()
        } else {
            self.encoding.name().to_string()
        }
    }

    /// Plain UTF-8 without a byte-order mark: the usual case, not worth
    /// mentioning.
    pub fn is_plain_utf8(&self) -> bool {
        *self == Self::default()
    }

    /// Decodes a file's bytes: by its byte-order mark if it has one, else as
    /// UTF-8 if that's valid, else as `fallback`. Fails if the bytes aren't
    /// valid in the encoding chosen (a broken UTF-16 file, or a non-UTF-8
    /// file with `fallback` set to UTF-8) rather than lose any of them.
    pub fn decode(bytes: &[u8], fallback: &'static Encoding) -> Result<(String, Self), String> {
        let (encoding, bom_len) = match Encoding::for_bom(bytes) {
            Some((encoding, bom_len)) => (encoding, bom_len),
            None if std::str::from_utf8(bytes).is_ok() => (UTF_8, 0),
            None => (fallback, 0),
        };
        let (text, had_errors) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        if had_errors {
            return Err(format!(
                "not valid {}; refusing to open it",
                encoding.name()
            ));
        }
        Ok((
            text.into_owned(),
            Self {
                encoding,
                bom: bom_len > 0,
            },
        ))
    }

    /// Encodes `text` for saving, with the byte-order mark if it had one.
    /// Fails, naming the first character that can't be represented, rather
    /// than write a substitute.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(text.len() + 3);
        if self.encoding == UTF_16LE || self.encoding == UTF_16BE {
            let little = self.encoding == UTF_16LE;
            let unit = |u: u16| {
                if little {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                }
            };
            if self.bom {
                bytes.extend(unit(0xFEFF));
            }
            for u in text.encode_utf16() {
                bytes.extend(unit(u));
            }
            return Ok(bytes);
        }
        if self.encoding == UTF_8 {
            if self.bom {
                bytes.extend([0xEF, 0xBB, 0xBF]);
            }
            bytes.extend(text.as_bytes());
            return Ok(bytes);
        }
        let (encoded, _, had_errors) = self.encoding.encode(text);
        if had_errors {
            let mut buf = [0; 4];
            let bad = text
                .chars()
                .find(|c| self.encoding.encode(c.encode_utf8(&mut buf)).2)
                .unwrap_or('?');
            return Err(format!(
                "can't be saved as {}: it has no '{bad}' (convert with :fileencoding utf-8)",
                self.encoding.name()
            ));
        }
        bytes.extend(encoded.iter());
        Ok(bytes)
    }
}

/// The line terminator a file uses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }
}

/// How a file's text is laid out on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFormat {
    pub line_ending: LineEnding,
    /// Whether the last line ends with a line terminator (POSIX text files do).
    pub final_newline: bool,
    pub encoding: FileEncoding,
}

impl Default for TextFormat {
    /// The format for a brand new file: LF with a final newline.
    fn default() -> Self {
        Self {
            line_ending: LineEnding::Lf,
            final_newline: true,
            encoding: FileEncoding::default(),
        }
    }
}

impl TextFormat {
    /// Splits file contents into buffer text (`\n`-separated, no final
    /// newline) and the format needed to write it back.
    ///
    /// The line ending is taken from the first line break. In a mixed file,
    /// stray `\r` characters on LF lines are kept as text, so nothing is lost.
    pub fn decode(contents: &str) -> (String, TextFormat) {
        let line_ending = match contents.find('\n') {
            Some(i) if i > 0 && contents.as_bytes()[i - 1] == b'\r' => LineEnding::Crlf,
            _ => LineEnding::Lf,
        };

        let mut text = match line_ending {
            LineEnding::Lf => contents.to_string(),
            LineEnding::Crlf => contents.replace("\r\n", "\n"),
        };
        let final_newline = text.ends_with('\n');
        if final_newline {
            text.pop();
        }

        let format = TextFormat {
            line_ending,
            final_newline,
            encoding: FileEncoding::default(),
        };
        (text, format)
    }

    /// [`TextFormat::decode`] for a file's bytes: decodes them first (see
    /// [`FileEncoding::decode`]) and records the encoding.
    pub fn decode_bytes(
        bytes: &[u8],
        fallback: &'static Encoding,
    ) -> Result<(String, TextFormat), String> {
        let (contents, encoding) = FileEncoding::decode(bytes, fallback)?;
        let (text, format) = Self::decode(&contents);
        Ok((text, TextFormat { encoding, ..format }))
    }

    /// [`TextFormat::encode`], then encoded into the file's bytes.
    pub fn encode_bytes(&self, text: &str) -> Result<Vec<u8>, String> {
        self.encoding.encode(&self.encode(text))
    }

    /// Converts buffer text back into file contents.
    pub fn encode(&self, text: &str) -> String {
        let ending = self.line_ending.as_str();
        let mut contents = match self.line_ending {
            LineEnding::Lf => text.to_string(),
            LineEnding::Crlf => text.replace('\n', ending),
        };
        if self.final_newline {
            contents.push_str(ending);
        }
        contents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(contents: &str) {
        let (text, format) = TextFormat::decode(contents);
        assert_eq!(format.encode(&text), contents, "{contents:?}");
    }

    #[test]
    fn decodes_lf_with_final_newline() {
        let (text, format) = TextFormat::decode("a\nb\n");
        assert_eq!(text, "a\nb");
        assert_eq!(format, TextFormat::default());
    }

    #[test]
    fn decodes_crlf() {
        let (text, format) = TextFormat::decode("a\r\nb\r\n");
        assert_eq!(text, "a\nb");
        assert_eq!(format.line_ending, LineEnding::Crlf);
        assert!(format.final_newline);
    }

    #[test]
    fn detects_missing_final_newline() {
        let (text, format) = TextFormat::decode("a\nb");
        assert_eq!(text, "a\nb");
        assert!(!format.final_newline);
    }

    #[test]
    fn empty_file_has_no_final_newline() {
        let (text, format) = TextFormat::decode("");
        assert_eq!(text, "");
        assert!(!format.final_newline);
    }

    #[test]
    fn mixed_endings_keep_stray_carriage_returns() {
        let (text, format) = TextFormat::decode("a\nb\r\nc");
        assert_eq!(format.line_ending, LineEnding::Lf);
        assert_eq!(text, "a\nb\r\nc");
    }

    #[test]
    fn files_round_trip_exactly() {
        for contents in [
            "",
            "\n",
            "\n\n",
            "one line",
            "one line\n",
            "a\nb\nc\n",
            "a\r\nb\r\n",
            "a\r\nb",
            "\r\n",
            "mixed\nends\r\nhere\n",
            "unicode é 日本\n",
        ] {
            round_trip(contents);
        }
    }

    #[test]
    fn encodes_new_text_in_the_files_format() {
        let format = TextFormat {
            line_ending: LineEnding::Crlf,
            final_newline: false,
            ..TextFormat::default()
        };
        assert_eq!(format.encode("x\ny"), "x\r\ny");
    }

    fn bytes_round_trip(bytes: &[u8]) -> (String, TextFormat) {
        let (text, format) = TextFormat::decode_bytes(bytes, DEFAULT_FALLBACK).unwrap();
        assert_eq!(format.encode_bytes(&text).unwrap(), bytes, "byte for byte");
        (text, format)
    }

    #[test]
    fn utf8_with_and_without_a_bom() {
        let (text, format) = bytes_round_trip(b"caf\xC3\xA9\n");
        assert_eq!(text, "café");
        assert!(format.encoding.is_plain_utf8());
        let (text, format) = bytes_round_trip(b"\xEF\xBB\xBFhi\n");
        assert_eq!(text, "hi", "the mark isn't text");
        assert_eq!(format.encoding.name(), "UTF-8 BOM");
    }

    #[test]
    fn utf16_by_its_byte_order_mark() {
        let (text, format) = bytes_round_trip(b"\xFF\xFEh\x00\xE9\x00\n\x00");
        assert_eq!(text, "hé");
        assert_eq!(format.encoding.name(), "UTF-16LE");
        let (text, format) = bytes_round_trip(b"\xFE\xFF\x00h\x00\n");
        assert_eq!(text, "h");
        assert_eq!(format.encoding.name(), "UTF-16BE");
    }

    #[test]
    fn other_bytes_use_the_fallback() {
        let (text, format) = bytes_round_trip(b"caf\xE9 \x80\r\n");
        assert_eq!(text, "café €");
        assert_eq!(format.encoding.name(), "windows-1252");
        assert_eq!(format.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn a_utf8_fallback_refuses_other_bytes() {
        let err = TextFormat::decode_bytes(b"\xE9", encoding_rs::UTF_8).unwrap_err();
        assert!(err.contains("not valid UTF-8"), "{err}");
    }

    #[test]
    fn characters_an_encoding_lacks_refuse_to_save() {
        let latin = TextFormat {
            encoding: FileEncoding::from_label("latin1").unwrap(),
            ..TextFormat::default()
        };
        let err = latin.encode_bytes("ok 日本").unwrap_err();
        assert!(err.contains("has no '日'"), "{err}");
    }

    #[test]
    fn labels() {
        assert_eq!(
            FileEncoding::from_label("UTF-8").unwrap(),
            FileEncoding::default()
        );
        assert_eq!(
            FileEncoding::from_label("utf-8-bom").unwrap().name(),
            "UTF-8 BOM"
        );
        assert_eq!(
            FileEncoding::from_label("latin1").unwrap().name(),
            "windows-1252"
        );
        assert!(FileEncoding::from_label("utf-16le").unwrap().bom);
        assert!(FileEncoding::from_label("klingon").is_none());
    }
}
