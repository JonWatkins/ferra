//! `:sort`: parsing its argument and sorting lines, as in Vim.
//!
//! `:sort [!] [i] [n] [u] [r] [/pattern/]`:
//!
//! - `!` sorts in reverse; `i` ignores case; `n` sorts by the first
//!   (decimal, maybe negative) number in the line, lines without one first;
//!   `u` keeps only the first of lines that sort the same.
//! - With a pattern, lines sort by what follows its first match, or with `r`
//!   by the match itself. Lines it doesn't match keep their order, before
//!   the sorted ones (after them, reversed, with `!`). An empty pattern
//!   (`//`) is the last search.
//!
//! The sort is stable, so lines that sort the same keep their order.

use std::cmp::Ordering;

use crate::core::search::SearchPattern;

/// `:sort`'s options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortOptions {
    pub reverse: bool,
    pub ignore_case: bool,
    pub numeric: bool,
    pub unique: bool,
    /// Sort by the match itself rather than what follows it.
    pub by_match: bool,
    /// The pattern's text, if one was given (empty: the last search).
    pub pattern: Option<String>,
}

/// Parses `:sort`'s argument; `bang` is the `!` after the name.
pub fn parse_sort(arg: &str, bang: bool) -> Result<SortOptions, String> {
    let mut options = SortOptions {
        reverse: bang,
        ..SortOptions::default()
    };
    let mut chars = arg.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {}
            'i' => options.ignore_case = true,
            'n' => options.numeric = true,
            'u' => options.unique = true,
            'r' => options.by_match = true,
            c if !c.is_alphanumeric() && c != '"' && c != '\\' => {
                // A pattern between `c`s (usually `/`), with `\c` escaping it.
                let mut pattern = String::new();
                let mut closed = false;
                while let Some(p) = chars.next() {
                    if p == '\\' && chars.peek() == Some(&c) {
                        pattern.push(chars.next().expect("peeked"));
                    } else if p == c {
                        closed = true;
                        break;
                    } else {
                        pattern.push(p);
                    }
                }
                if !closed && chars.peek().is_some() {
                    return Err(format!("Unclosed pattern: {c}{pattern}"));
                }
                options.pattern = Some(pattern);
            }
            other => return Err(format!("Unknown :sort flag: {other}")),
        }
    }
    Ok(options)
}

/// What a line sorts by.
#[derive(Debug, Clone, PartialEq)]
enum Key {
    Text(String),
    /// `None` for a line without a number, which sorts first.
    Number(Option<i128>),
}

impl Key {
    fn compare(&self, other: &Key) -> Ordering {
        match (self, other) {
            (Key::Text(a), Key::Text(b)) => a.cmp(b),
            (Key::Number(a), Key::Number(b)) => a.cmp(b),
            _ => Ordering::Equal,
        }
    }
}

/// The first decimal number in `text` (with a `-` right before it).
fn first_number(text: &str) -> Option<i128> {
    let chars: Vec<char> = text.chars().collect();
    let start = chars.iter().position(char::is_ascii_digit)?;
    let digits: String = chars[start..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let value: i128 = digits.parse().unwrap_or(i128::MAX);
    Some(if start > 0 && chars[start - 1] == '-' {
        -value
    } else {
        value
    })
}

/// `lines` sorted as `options` say, with `pattern` compiled (for a
/// pattern). Returns the new lines.
pub fn sort_lines(
    lines: &[String],
    options: &SortOptions,
    pattern: Option<&SearchPattern>,
) -> Vec<String> {
    let key_of = |line: &str| -> Option<Key> {
        let text = match pattern {
            Some(pattern) => {
                let (start, end) = pattern.matches_in_line(line).into_iter().next()?;
                let chars: Vec<char> = line.chars().collect();
                if options.by_match {
                    chars[start..end].iter().collect()
                } else {
                    chars[end..].iter().collect()
                }
            }
            None => line.to_string(),
        };
        Some(if options.numeric {
            Key::Number(first_number(&text))
        } else if options.ignore_case {
            Key::Text(text.to_lowercase())
        } else {
            Key::Text(text)
        })
    };

    let mut unmatched = Vec::new();
    let mut keyed = Vec::new();
    for line in lines {
        match key_of(line) {
            Some(key) => keyed.push((key, line.clone())),
            None => unmatched.push(line.clone()),
        }
    }
    keyed.sort_by(|(a, _), (b, _)| {
        let order = a.compare(b);
        if options.reverse {
            order.reverse()
        } else {
            order
        }
    });
    if options.unique {
        keyed.dedup_by(|(b, _), (a, _)| a.compare(b) == Ordering::Equal);
    }
    let sorted = keyed.into_iter().map(|(_, line)| line);
    if options.reverse {
        unmatched.reverse();
        sorted.chain(unmatched).collect()
    } else {
        unmatched.into_iter().chain(sorted).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::search::CaseMode;

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(String::from).collect()
    }

    fn sort(text: &str, arg: &str, bang: bool) -> String {
        let options = parse_sort(arg, bang).unwrap();
        let pattern = options
            .pattern
            .as_deref()
            .map(|p| SearchPattern::new(p, CaseMode::Sensitive).unwrap());
        sort_lines(&lines(text), &options, pattern.as_ref()).join("\n")
    }

    #[test]
    fn plain_reverse_and_case() {
        assert_eq!(
            sort("b\nC\na", "", false),
            "C\na\nb",
            "byte order: capitals first"
        );
        assert_eq!(sort("b\nC\na", "i", false), "a\nb\nC");
        assert_eq!(sort("b\nC\na", "i", true), "C\nb\na");
    }

    #[test]
    fn numbers() {
        assert_eq!(
            sort("x10\nx9\nnone\nx-3\ny9", "n", false),
            "none\nx-3\nx9\ny9\nx10",
            "no number first; equal numbers keep their order"
        );
        assert_eq!(sort("2\n10\n1", "n", true), "10\n2\n1");
    }

    #[test]
    fn unique() {
        assert_eq!(sort("b\na\nb\na", "u", false), "a\nb");
        assert_eq!(sort("A\na\nb", "ui", false), "A\nb", "the first of equals");
        assert_eq!(sort("1 x\n1 y\n2", "nu", false), "1 x\n2", "same number");
    }

    #[test]
    fn by_what_follows_a_pattern_or_the_match() {
        let text = "b: 2\nplain\na: 3\nc: 1";
        assert_eq!(sort(text, "/: /", false), "plain\nc: 1\nb: 2\na: 3");
        assert_eq!(sort(text, "/: / n", true), "a: 3\nb: 2\nc: 1\nplain");
        assert_eq!(sort("x2 b\nx1 a", "r /x\\d/", false), "x1 a\nx2 b");
    }

    #[test]
    fn parsing() {
        assert_eq!(
            parse_sort(" iu /a\\/b/ ", false).unwrap(),
            SortOptions {
                ignore_case: true,
                unique: true,
                pattern: Some("a/b".into()),
                ..SortOptions::default()
            }
        );
        assert_eq!(
            parse_sort("//", false).unwrap().pattern.as_deref(),
            Some("")
        );
        assert_eq!(parse_sort("q", false), Err("Unknown :sort flag: q".into()));
    }
}
