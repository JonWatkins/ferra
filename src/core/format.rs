//! `gq`: reflowing lines of text to a width.
//!
//! `gq` is `Operator::Format` (`gw` with `keep_cursor`) over the lines its
//! range touches, a paragraph at a time: blank lines (or a comment marker
//! alone) separate paragraphs and are kept, as does a change between code and
//! a kind of comment. A paragraph's words are joined with single spaces (two
//! after a sentence's `.`, `!` or `?` where its line had two or more; joining
//! lines puts one, as Vim's `nojoinspaces`) and broken greedily at
//! `text-width` (0 means 79, Vim's fallback; it's buffer-local), every line
//! starting with the first line's *leader*: its indentation and its comment
//! marker with the space after it. A word wider than the width gets a line to
//! itself. The lines are replaced as one change; the cursor goes to the last
//! one's first non-blank, or with `gw` stays where it was. `gqq`, `gqgq` and
//! `gwgw` come from operator-pending `q`, `gq` and `gw` bound to
//! `current_line`. In a buffer whose language server formats ranges, `gq`
//! asks it instead (Neovim's `formatexpr` default: `Editor::apply_operator_over`
//! sends the lines to `:format`); otherwise, with `format-program` (Vim's
//! `formatprg`), `Editor::format_with_program` runs them through it (see
//! `filter`). `gw` is always this reflow.
//!
//! - **Comment markers** (`CommentMarkers`) are the language's line comment
//!   (`//`, `#`...) and, in languages with C's comments, the ` * ` of a
//!   `/* */` comment's middle lines, as Vim's `comments=s1:/*,mb:*,ex:*/`.
//!   Text after a `/*` or `/**` starts a paragraph whose later lines start
//!   ` * `, lined up under the first `*`; a line of just `/**`, ` *` or ` */`
//!   separates paragraphs. Other block comments (`<!-- -->`) aren't leaders,
//!   and in languages without C's comments a leading `*` is a list item.
//! - **A list item** (`- `, `* `, `+ `, `1. `, `1) `) starts a paragraph of its
//!   own, and its later lines line up under its text.
//! - **Auto-wrap** uses the same leaders: `continuation` gives what a line
//!   broken while typing continues with.

use super::columns::display_width;

/// The comment markers that are leaders for `gq` and auto-wrap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentMarkers {
    /// The language's line-comment marker (`//`, `#`), if it has one.
    pub line: Option<String>,
    /// Whether the language has C's `/* */` comments, whose middle lines
    /// start ` * ` (Vim's `s1:/*,mb:*,ex:*/`).
    pub c_block: bool,
}

/// Reflows `lines` to `width` display columns.
pub fn format_lines(
    lines: &[String],
    width: usize,
    markers: &CommentMarkers,
    tab_width: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut paragraph: Option<Paragraph> = None;
    for line in lines {
        let parsed = Line::parse(line, markers);
        if parsed.text.is_empty() {
            if let Some(p) = paragraph.take() {
                p.fill(width, tab_width, &mut out);
            }
            out.push(line.trim_end().to_string());
            continue;
        }
        let continues = paragraph.as_ref().is_some_and(|p| {
            p.kind == parsed.kind && parsed.bullet.is_none() && !parsed.opens_block
        });
        if continues {
            let p = paragraph.as_mut().expect("checked");
            p.words.extend(words(&parsed.text));
        } else {
            if let Some(p) = paragraph.take() {
                p.fill(width, tab_width, &mut out);
            }
            let rest_leader = parsed.rest_leader();
            paragraph = Some(Paragraph {
                first_leader: parsed.leader,
                rest_leader,
                kind: parsed.kind,
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
/// starts with (the same leader, ` * ` after a `/*`, lined up under a list
/// item's text), and whether it's a comment.
pub fn continuation(line: &str, markers: &CommentMarkers) -> (usize, String, bool) {
    let parsed = Line::parse(line, markers);
    let leader = parsed.rest_leader();
    (parsed.text_start, leader, parsed.kind != Kind::Code)
}

/// What a line is, for which lines join: code (or prose) and the two kinds
/// of comment don't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Code,
    LineComment,
    BlockComment,
}

/// A line split into its leader and its text.
struct Line {
    leader: String,
    /// What lines after it start with, before any list item's alignment:
    /// the leader, except after a `/*`, where it's ` * `.
    next_leader: String,
    text: String,
    /// The char column the text starts at.
    text_start: usize,
    kind: Kind,
    /// Whether it starts a block comment (`/* text`): a paragraph of its own.
    opens_block: bool,
    /// The width of a list marker the text starts with (`- ` is 2).
    bullet: Option<usize>,
}

impl Line {
    fn parse(line: &str, markers: &CommentMarkers) -> Line {
        let rest = line.trim_start_matches([' ', '\t']);
        let indent = &line[..line.len() - rest.len()];
        // `after` is what follows the comment marker.
        let mut opens_block = false;
        let (kind, after) =
            if let Some(after) = markers.line.as_deref().and_then(|c| rest.strip_prefix(c)) {
                (Kind::LineComment, after)
            } else if markers.c_block && rest.trim_end() == "*/" {
                (Kind::BlockComment, "")
            } else if let Some(after) = rest
                .strip_prefix("/*")
                .map(|a| a.trim_start_matches('*'))
                .filter(|a| markers.c_block && !a.starts_with('/'))
            {
                opens_block = true;
                (Kind::BlockComment, after)
            } else if let Some(after) = rest
                .strip_prefix('*')
                .filter(|a| markers.c_block && (a.is_empty() || a.starts_with([' ', '\t'])))
            {
                (Kind::BlockComment, after)
            } else {
                (Kind::Code, rest)
            };
        let text = after.trim_start_matches([' ', '\t']);
        let marker = &rest[..rest.len() - text.len()];
        // A comment marker alone gets a space after it when text follows.
        let space = if kind != Kind::Code && !marker.ends_with([' ', '\t']) && !text.is_empty() {
            " "
        } else {
            ""
        };
        let leader = format!("{indent}{marker}{space}");
        let next_leader = if opens_block {
            format!("{indent} * ")
        } else {
            leader.clone()
        };
        Line {
            leader,
            next_leader,
            text: text.trim_end().to_string(),
            text_start: line.chars().count() - text.chars().count(),
            kind,
            opens_block,
            bullet: bullet_width(text),
        }
    }

    /// What the lines a paragraph from this line wraps onto start with.
    fn rest_leader(&self) -> String {
        match self.bullet {
            Some(bullet) => format!("{}{}", self.next_leader, " ".repeat(bullet)),
            None => self.next_leader.clone(),
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
    kind: Kind,
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

    fn markers(comment: Option<&str>) -> CommentMarkers {
        CommentMarkers {
            line: comment.map(String::from),
            c_block: comment == Some("//"),
        }
    }

    fn format(text: &str, width: usize, comment: Option<&str>) -> String {
        let lines: Vec<String> = text.split('\n').map(String::from).collect();
        format_lines(&lines, width, &markers(comment), 4).join("\n")
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
            continuation("    // some text", &markers(Some("//"))),
            (7, "    // ".into(), true)
        );
        assert_eq!(
            continuation("  - item text", &markers(None)),
            (2, "    ".into(), false)
        );
        assert_eq!(
            continuation("plain", &markers(None)),
            (0, String::new(), false)
        );
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

    #[test]
    fn block_comments_middle_lines_are_leaders() {
        let c = Some("//");
        assert_eq!(
            format("/**\n * one two three\n * four five\n */", 12, c),
            "/**\n * one two\n * three\n * four five\n */",
            "the opening and closing lines stay"
        );
        assert_eq!(
            format("  /* one two three four", 12, c),
            "  /* one two\n   * three\n   * four",
            "text after /* carries on as ` * ` lines"
        );
        assert_eq!(
            format("/* one\n * two\n *\n * three", 80, c),
            "/* one two\n *\n * three",
            "a `*` alone separates paragraphs"
        );
        assert_eq!(
            format(" * - one two three", 9, c),
            " * - one\n *   two\n *   three"
        );
        assert_eq!(
            format("// a\n * b\n*ptr = c;\nd", 80, c),
            "// a\n * b\n*ptr = c; d",
            "line comments, block comments and code don't join"
        );
        assert_eq!(
            format("* one\n* two", 80, None),
            "* one\n* two",
            "without C comments, `*` is a list item"
        );
    }

    #[test]
    fn continuing_a_block_comment_as_its_typed() {
        let c = markers(Some("//"));
        assert_eq!(
            continuation("   * some text", &c),
            (5, "   * ".into(), true)
        );
        assert_eq!(continuation("/** some text", &c), (4, " * ".into(), true));
        assert_eq!(continuation("*ptr = 1", &c), (0, String::new(), false));
    }
}
