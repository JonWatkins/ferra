//! Parsing `:s/pattern/replacement/flags`.

/// A parsed `:s` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitute {
    /// Empty means "the last search pattern".
    pub pattern: String,
    /// In `regex` crate syntax (`$0`, `${1}`), translated from Vim's.
    pub replacement: String,
    pub flags: Flags,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    /// `g`: every match on a line, not just the first.
    pub global: bool,
    /// `i` / `I`: ignore or respect case (overriding the `case` option).
    pub ignore_case: Option<bool>,
    /// `n`: count matches without changing anything.
    pub count_only: bool,
    /// `e`: no error when nothing matches.
    pub no_error: bool,
    /// `c`: ask before each replacement.
    pub confirm: bool,
}

/// Parses the text after `:s`: a delimiter (any punctuation), the pattern,
/// the replacement and flags, with the closing delimiter optional
/// (`s/a/b`). `\` escapes the delimiter. A `~` in the replacement stands
/// for `previous`, the last substitution's replacement (already in
/// `regex` syntax).
pub fn parse_substitute(text: &str, previous: Option<&str>) -> Result<Substitute, String> {
    const USAGE: &str = "Usage: s/pattern/replacement/[flags]";
    let mut chars = text.chars();
    let delimiter = chars.next().ok_or(USAGE)?;
    if delimiter.is_alphanumeric() || delimiter.is_whitespace() || delimiter == '\\' {
        return Err(USAGE.into());
    }

    let parts = split_unescaped(chars.as_str(), delimiter);
    let pattern = parts.first().cloned().unwrap_or_default();
    let replacement = parts.get(1).cloned().unwrap_or_default();
    let flags = parse_flags(parts.get(2).map_or("", String::as_str))?;
    if parts.len() > 3 {
        return Err(format!(
            "Trailing characters after the flags: {}",
            parts[3..].join("")
        ));
    }
    Ok(Substitute {
        pattern,
        replacement: vim_replacement(&replacement, previous.unwrap_or_default()),
        flags,
    })
}

/// Splits on `delimiter`, turning `\delimiter` into a literal delimiter and
/// keeping every other escape as it is (for the regex).
fn split_unescaped(text: &str, delimiter: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let current = parts.last_mut().expect("at least one part");
        if c == '\\' && chars.peek() == Some(&delimiter) {
            current.push(delimiter);
            chars.next();
        } else if c == '\\' {
            current.push(c);
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else if c == delimiter {
            parts.push(String::new());
        } else {
            current.push(c);
        }
    }
    parts
}

fn parse_flags(text: &str) -> Result<Flags, String> {
    let mut flags = Flags::default();
    for c in text.trim().chars() {
        match c {
            'g' => flags.global = true,
            'i' => flags.ignore_case = Some(true),
            'I' => flags.ignore_case = Some(false),
            'n' => flags.count_only = true,
            'e' => flags.no_error = true,
            'c' => flags.confirm = true,
            _ => return Err(format!("Unknown flag '{c}' (use g, c, i, I, n or e)")),
        }
    }
    Ok(flags)
}

/// Translates a Vim replacement into `regex` syntax: `&` (whole match) and
/// `\0`..`\9` become `${0}`..`${9}`, `~` is the previous replacement,
/// `\&`, `\~` and `\\` are literal, `\n` is a line break, `\t` a tab, and a
/// literal `$` is escaped.
fn vim_replacement(text: &str, previous: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.push_str("${0}"),
            '~' => out.push_str(previous),
            '$' => out.push_str("$$"),
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => out.push_str(&format!("${{{d}}}")),
                Some('n' | 'r') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('$') => out.push_str("$$"),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Substitute {
        parse_substitute(text, None).unwrap()
    }

    #[test]
    fn basic_forms() {
        let s = parse("/old/new/");
        assert_eq!((s.pattern.as_str(), s.replacement.as_str()), ("old", "new"));
        assert_eq!(s.flags, Flags::default());
        assert_eq!(
            parse("/old/new").replacement,
            "new",
            "closing delimiter optional"
        );
        assert_eq!(
            parse("/old").replacement,
            "",
            "replacement optional: delete"
        );
        assert_eq!(
            parse("//x/").pattern,
            "",
            "empty pattern: reuse the last search"
        );
    }

    #[test]
    fn flags() {
        let f = parse("/a/b/gIn").flags;
        assert!(f.global && f.count_only);
        assert_eq!(f.ignore_case, Some(false));
        assert!(parse("/a/b/e").flags.no_error);
        assert!(
            parse_substitute("/a/b/z", None)
                .unwrap_err()
                .contains("Unknown flag 'z'")
        );
    }

    #[test]
    fn other_delimiters_and_escapes() {
        let s = parse("#/usr/bin#/opt/bin#");
        assert_eq!(
            (s.pattern.as_str(), s.replacement.as_str()),
            ("/usr/bin", "/opt/bin")
        );
        let s = parse(r"/a\/b/c\/d/");
        assert_eq!((s.pattern.as_str(), s.replacement.as_str()), ("a/b", "c/d"));
        assert_eq!(parse(r"/\d+/N/").pattern, r"\d+", "regex escapes kept");
    }

    #[test]
    fn vim_replacement_syntax() {
        assert_eq!(vim_replacement(r"[&]", ""), "[${0}]");
        assert_eq!(vim_replacement(r"\2-\1", ""), "${2}-${1}");
        assert_eq!(vim_replacement(r"a\&b\\c", ""), r"a&b\c");
        assert_eq!(vim_replacement(r"line\nnext\tx", ""), "line\nnext\tx");
        assert_eq!(vim_replacement("$5", ""), "$$5", "a literal dollar");
    }

    #[test]
    fn invalid_forms() {
        assert!(parse_substitute("", None).is_err());
        assert!(
            parse_substitute("abc", None).is_err(),
            "letters can't be delimiters"
        );
        assert!(parse_substitute("/a/b/g/extra", None).is_err());
    }

    #[test]
    fn tilde_is_the_previous_replacement() {
        let s = parse_substitute("/a/<~>/", Some("${0}x")).unwrap();
        assert_eq!(s.replacement, "<${0}x>");
        let s = parse_substitute(r"/a/\~/", Some("x")).unwrap();
        assert_eq!(s.replacement, "~", "escaped: a literal tilde");
        let s = parse_substitute("/a/~/", None).unwrap();
        assert_eq!(s.replacement, "", "nothing before");
    }
}
