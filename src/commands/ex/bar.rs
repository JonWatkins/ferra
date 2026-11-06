//! Splitting a `:` line at `|`, which separates commands
//! (`:s/a/b/ | normal A!`).

/// How a command treats `|` in its arguments.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    /// `|` ends the command; `\|` is a literal `|`.
    #[default]
    Separates,
    /// Everything after the command is its argument, `|` included (`:g`,
    /// `:normal`), so `:g/x/cmd1 | cmd2` runs both on each line.
    Argument,
    /// `|` inside a `/pattern/replacement/` belongs to it (regex
    /// alternation); after it, `|` separates (`:s`).
    AfterPattern,
}

/// Splits `args` at the `|` that ends the command, if any: the command's
/// own arguments (with `\|` turned into `|` where `|` separates), and the
/// rest of the line.
pub fn split_bar(args: &str, bar: Bar) -> (String, Option<&str>) {
    match bar {
        Bar::Argument => (args.to_string(), None),
        Bar::Separates => split_at_bar(args),
        Bar::AfterPattern => {
            let pattern_len = delimited_len(args);
            let (pattern, flags) = args.split_at(pattern_len);
            let (flags, rest) = split_at_bar(flags);
            (format!("{pattern}{flags}"), rest)
        }
    }
}

/// Up to the first unescaped `|`.
fn split_at_bar(text: &str) -> (String, Option<&str>) {
    let mut own = String::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if chars.peek().map(|&(_, n)| n) == Some('|') => {
                own.push('|');
                chars.next();
            }
            '|' => return (own.trim_end().to_string(), Some(&text[i + 1..])),
            c => own.push(c),
        }
    }
    (own, None)
}

/// The length of a `/pattern/replacement/` at the start of `text`: through
/// its third delimiter, or all of it if it isn't closed. Nothing if `text`
/// doesn't start with a delimiter (`:s` alone, or flags only).
fn delimited_len(text: &str) -> usize {
    let mut chars = text.char_indices();
    let Some((_, delimiter)) = chars.next() else {
        return 0;
    };
    if delimiter.is_alphanumeric()
        || delimiter.is_whitespace()
        || delimiter == '\\'
        || delimiter == '|'
    {
        return 0;
    }
    let mut seen = 1;
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if c == delimiter {
            seen += 1;
            if seen == 3 {
                return i + c.len_utf8();
            }
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_at_the_first_bar() {
        assert_eq!(
            split_bar("a.txt | q", Bar::Separates),
            ("a.txt".into(), Some(" q"))
        );
        assert_eq!(split_bar("no bar", Bar::Separates), ("no bar".into(), None));
        assert_eq!(
            split_bar(r"a\|b|c", Bar::Separates),
            ("a|b".into(), Some("c"))
        );
    }

    #[test]
    fn arguments_keep_their_bars() {
        assert_eq!(
            split_bar("/x/d | s/a/b/", Bar::Argument),
            ("/x/d | s/a/b/".into(), None)
        );
    }

    #[test]
    fn substitute_patterns_keep_their_bars() {
        assert_eq!(
            split_bar("/a|b/c/g", Bar::AfterPattern),
            ("/a|b/c/g".into(), None)
        );
        assert_eq!(
            split_bar("/a|b/c|d/g | normal x", Bar::AfterPattern),
            ("/a|b/c|d/g".into(), Some(" normal x"))
        );
        assert_eq!(
            split_bar(r"/a\/|/b/|q", Bar::AfterPattern),
            (r"/a\/|/b/".into(), Some("q")),
            "escaped delimiters don't count"
        );
        assert_eq!(
            split_bar("/open|ended", Bar::AfterPattern),
            ("/open|ended".into(), None)
        );
        assert_eq!(
            split_bar("|q", Bar::AfterPattern),
            ("".into(), Some("q")),
            "bare :s"
        );
    }
}
