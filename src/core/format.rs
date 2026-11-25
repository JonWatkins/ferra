//! `gq`: reflowing lines of text to a width.
//!
//! Lines are taken a paragraph at a time: blank lines (or a comment marker
//! alone) separate paragraphs and are kept, and so does a change in whether
//! lines are comments. A paragraph's words are joined with single spaces
//! (two after a sentence's `.`, `!` or `?` where its line had two or more;
//! joining lines puts one, as Vim's `nojoinspaces`) and broken into lines
//! no wider than the width, each starting with the first line's *leader*:
//! its indentation, and its line-comment marker (`//`, `#`...) with the
//! space after it. A list item (`- `, `* `, `+ `,
//! `1. `, `1) `) starts a paragraph of its own, and its later lines line up
//! under its text. A word wider than the width gets a line to itself.
//!
//! `gq` is `Operator::Format` (`gw` with `keep_cursor`) over the lines its
//! range touches. `core/format.rs` does the work, a paragraph at a time: blank
//! lines (or a comment marker alone) separate paragraphs and are kept, as does
//! a change between comment and code. Each paragraph's words are joined with
//! single spaces (two kept after a sentence, as above) and broken greedily
//! at `text-width` (0 means 79, Vim's fallback; it's buffer-local), every
//! line starting with the first line's leader: its indentation and the
//! language's line-comment marker with its space (block comments aren't
//! leaders, so Markdown's `#` headings stay put). A list item (`- `, `* `,
//! `+ `, `1. `, `1) `) starts its own paragraph, its later lines indented to
//! its text. The lines are replaced as one change; the
//! cursor goes to the last one's first non-blank, or with `gw` stays where it
//! was. `gqq`, `gqgq` and `gwgw` come from operator-pending `q`, `gq` and `gw`
//! bound to `current_line`.

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
            p.words.extend(words(&parsed.text));
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
                words: words(&parsed.text).collect(),
            });
        }
    }
    if let Some(p) = paragraph {
        p.fill(width, tab_width, &mut out);
    }
    out
}

/// For wrapping `line` as it's typed: the char column its text starts at
/// (after indentation and any comment marker), what a line continuing it
/// starts with (the same leader, lined up under a list item's text), and
/// whether it's a comment.
pub fn continuation(line: &str, comment: Option<&str>) -> (usize, String, bool) {
    let parsed = Line::parse(line, comment);
    let rest = line.trim_start_matches([' ', '\t']);
    let text_start = match comment.and_then(|c| rest.strip_prefix(c)) {
        Some(after) => line.chars().count() - after.trim_start_matches([' ', '\t']).chars().count(),
        None => line.chars().count() - rest.chars().count(),
    };
    let leader = match parsed.bullet {
        Some(bullet) => format!("{}{}", parsed.leader, " ".repeat(bullet)),
        None => parsed.leader,
    };
    (text_start, leader, parsed.commented)
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

/// A word, and whether two spaces go before it: it follows a sentence's end
/// (`.`, `!`, `?`) and two or more spaces were there on its line.
struct Word {
    text: String,
    wide: bool,
}

/// The words of one line's text.
fn words(text: &str) -> impl Iterator<Item = Word> + '_ {
    let mut rest = text;
    let mut sentence_end = false;
    std::iter::from_fn(move || {
        let trimmed = rest.trim_start();
        let gap = rest.len() - trimmed.len();
        let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        if end == 0 {
            return None;
        }
        let (text, after) = trimmed.split_at(end);
        let word = Word {
            text: text.to_string(),
            wide: sentence_end && gap >= 2,
        };
        sentence_end = text.ends_with(['.', '!', '?']);
        rest = after;
        Some(word)
    })
}

struct Paragraph {
    first_leader: String,
    rest_leader: String,
    commented: bool,
    words: Vec<Word>,
}

impl Paragraph {
    fn fill(self, width: usize, tab_width: usize, out: &mut Vec<String>) {
        let mut line = self.first_leader.clone();
        let mut has_words = false;
        for word in self.words {
            let gap = match (has_words, word.wide) {
                (false, _) => "",
                (true, false) => " ",
                (true, true) => "  ",
            };
            let candidate = format!("{line}{gap}{}", word.text);
            if has_words && display_width(&candidate, tab_width) > width {
                out.push(std::mem::replace(&mut line, self.rest_leader.clone()));
                line.push_str(&word.text);
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
    fn continuing_a_line_as_its_typed() {
        assert_eq!(
            continuation("    // some text", Some("//")),
            (7, "    // ".into(), true)
        );
        assert_eq!(
            continuation("  - item text", None),
            (2, "    ".into(), false)
        );
        assert_eq!(continuation("plain", None), (0, String::new(), false));
    }

    #[test]
    fn two_spaces_after_a_sentence_are_kept() {
        assert_eq!(
            format("One.  Two!  Three?  Four", 80, None),
            "One.  Two!  Three?  Four"
        );
        assert_eq!(
            format("a.   b", 80, None),
            "a.  b",
            "more than two become two"
        );
        assert_eq!(
            format("a  b.c  d", 80, None),
            "a b.c d",
            "only after a sentence's end"
        );
        assert_eq!(
            format("One.\nTwo.  Three", 80, None),
            "One. Two.  Three",
            "joined lines get one space, as Vim's nojoinspaces"
        );
        assert_eq!(
            format("aaa.  bbb", 5, None),
            "aaa.\nbbb",
            "a break takes both spaces"
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
