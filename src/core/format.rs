//! `gq`: reflowing lines of text to a width.
//!
//! Lines are taken a paragraph at a time: blank lines (or a comment marker
//! alone) separate paragraphs and are kept, and so does a change in whether
//! lines are comments. A paragraph's words are joined with single spaces
//! and broken into lines no wider than the width, each starting with the
//! first line's *leader*: its indentation, and its line-comment marker
//! (`//`, `#`...) with the space after it. A list item (`- `, `* `, `+ `,
//! `1. `, `1) `) starts a paragraph of its own, and its later lines line up
//! under its text. A word wider than the width gets a line to itself.

use super::columns::display_width;

/// Reflows `lines` to `width` display columns. `comment` is the language's
/// line-comment marker, if it has one.
pub fn format_lines(
    lines: &[String],
    width: usize,
    comment: Option<&str>,
    tab_width: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut paragraph: Option<Paragraph> = None;
    for line in lines {
        let parsed = Line::parse(line, comment);
        if parsed.text.is_empty() {
            if let Some(p) = paragraph.take() {
                p.fill(width, tab_width, &mut out);
            }
            out.push(line.trim_end().to_string());
            continue;
        }
        let continues = paragraph
            .as_ref()
            .is_some_and(|p| p.commented == parsed.commented && parsed.bullet.is_none());
        if continues {
            let p = paragraph.as_mut().expect("checked");
            p.words
                .extend(parsed.text.split_whitespace().map(String::from));
        } else {
            if let Some(p) = paragraph.take() {
                p.fill(width, tab_width, &mut out);
            }
            let rest_leader = match parsed.bullet {
                Some(bullet) => format!("{}{}", parsed.leader, " ".repeat(bullet)),
                None => parsed.leader.clone(),
            };
            paragraph = Some(Paragraph {
                first_leader: parsed.leader,
                rest_leader,
                commented: parsed.commented,
                words: parsed.text.split_whitespace().map(String::from).collect(),
            });
        }
    }
    if let Some(p) = paragraph {
        p.fill(width, tab_width, &mut out);
    }
    out
}

/// A line split into its leader and its text.
struct Line {
    leader: String,
    text: String,
    commented: bool,
    /// The width of a list marker the text starts with (`- ` is 2).
    bullet: Option<usize>,
}

impl Line {
    fn parse(line: &str, comment: Option<&str>) -> Line {
        let rest = line.trim_start_matches([' ', '\t']);
        let indent = &line[..line.len() - rest.len()];
        let (leader, text, commented) = match comment.and_then(|c| rest.strip_prefix(c)) {
            Some(after) => {
                let text = after.trim_start_matches([' ', '\t']);
                let marker = &rest[..rest.len() - text.len()];
                (format!("{indent}{marker}"), text, true)
            }
            None => (indent.to_string(), rest, false),
        };
        // A comment marker alone gets a space after it when text follows.
        let leader = if commented && !leader.ends_with([' ', '\t']) && !text.is_empty() {
            format!("{leader} ")
        } else {
            leader
        };
        Line {
            leader,
            text: text.trim_end().to_string(),
            commented,
            bullet: bullet_width(text),
        }
    }
}

/// The width of the list marker `text` starts with, with its space.
fn bullet_width(text: &str) -> Option<usize> {
    for marker in ["- ", "* ", "+ "] {
        if text.starts_with(marker) {
            return Some(marker.len());
        }
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let rest = &text[digits..];
    (digits > 0 && (rest.starts_with(". ") || rest.starts_with(") "))).then_some(digits + 2)
}

struct Paragraph {
    first_leader: String,
    rest_leader: String,
    commented: bool,
    words: Vec<String>,
}

impl Paragraph {
    fn fill(self, width: usize, tab_width: usize, out: &mut Vec<String>) {
        let mut line = self.first_leader.clone();
        let mut has_words = false;
        for word in self.words {
            let candidate = if has_words {
                format!("{line} {word}")
            } else {
                format!("{line}{word}")
            };
            if has_words && display_width(&candidate, tab_width) > width {
                out.push(std::mem::replace(&mut line, self.rest_leader.clone()));
                line.push_str(&word);
            } else {
                line = candidate;
            }
            has_words = true;
        }
        out.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(text: &str, width: usize, comment: Option<&str>) -> String {
        let lines: Vec<String> = text.split('\n').map(String::from).collect();
        format_lines(&lines, width, comment, 4).join("\n")
    }

    #[test]
    fn joins_and_breaks_to_the_width() {
        assert_eq!(
            format("the quick brown fox\njumps over the lazy dog", 15, None),
            "the quick brown\nfox jumps over\nthe lazy dog"
        );
        assert_eq!(format("a\nb\nc", 80, None), "a b c", "short lines join");
    }

    #[test]
    fn blank_lines_separate_paragraphs() {
        assert_eq!(
            format("one two\nthree\n\nfour\nfive", 80, None),
            "one two three\n\nfour five"
        );
    }

    #[test]
    fn indentation_is_kept() {
        assert_eq!(
            format("    aaa bbb ccc ddd", 12, None),
            "    aaa bbb\n    ccc ddd"
        );
    }

    #[test]
    fn comments_keep_their_marker() {
        assert_eq!(
            format("  // one two three\n  // four five", 15, Some("//")),
            "  // one two\n  // three four\n  // five"
        );
        assert_eq!(
            format("# a\n#\n# b", 80, Some("#")),
            "# a\n#\n# b",
            "a marker alone is a blank line"
        );
        assert_eq!(
            format("// a\ncode b", 80, Some("//")),
            "// a\ncode b",
            "comments and code don't join"
        );
    }

    #[test]
    fn list_items_are_paragraphs_lined_up_under_their_text() {
        assert_eq!(
            format("- one two three\n- four\n1. five six seven", 9, None),
            "- one two\n  three\n- four\n1. five\n   six\n   seven"
        );
    }

    #[test]
    fn long_words_get_a_line_of_their_own() {
        assert_eq!(
            format("a supercalifragilistic b", 10, None),
            "a\nsupercalifragilistic\nb"
        );
    }
}
