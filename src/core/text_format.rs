//! Converting between a file's bytes-on-disk text and the buffer's text.
//!
//! The buffer always uses `\n` and never stores the file's final newline.
//! [`TextFormat`] remembers what the file actually had, so saving writes it
//! back exactly: a file round-trips byte for byte unless it's edited.

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
}

impl Default for TextFormat {
    /// The format for a brand new file: LF with a final newline.
    fn default() -> Self {
        Self {
            line_ending: LineEnding::Lf,
            final_newline: true,
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
        };
        (text, format)
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
        };
        assert_eq!(format.encode("x\ny"), "x\r\ny");
    }
}
