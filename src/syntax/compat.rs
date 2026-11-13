//! Reading queries written for other editors, so a language pack can use
//! the queries a grammar's repository ships (mostly written for Neovim)
//! as they are.
//!
//! - `#lua-match?` (and `#not-lua-match?`) filter with a Lua pattern,
//!   which tree-sitter doesn't know: left alone the filter would be
//!   ignored and the pattern would match too much. They become `#match?`
//!   with the pattern turned into a regex.
//! - Neovim's indent captures (`@indent.begin`, `@indent.end`,
//!   `@indent.branch`, `@indent.dedent`) become ferra's `@indent` and
//!   `@outdent` (see `indent.rs`); `@indent.ignore` and the rest are
//!   dropped, as ferra already leaves strings and comments alone.
//!
//! Capture names for highlighting (`@conditional`, `@markup.heading`) are
//! matched where they're used (`highlight.rs`).

use std::sync::LazyLock;

use regex::Regex;

/// A query from another editor, in the form ferra's tree-sitter reads.
pub fn adapt(query: &str) -> String {
    let query = adapt_lua_match(query);
    adapt_indent_captures(&query)
}

static LUA_MATCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"#(not-)?lua-match\?(\s+@[\w.]+\s+)"((?:[^"\\]|\\.)*)""#).expect("valid")
});

fn adapt_lua_match(query: &str) -> String {
    LUA_MATCH
        .replace_all(query, |caps: &regex::Captures| {
            let not = caps.get(1).map_or("", |m| m.as_str());
            let pattern = unescape(&caps[3]);
            match lua_pattern_to_regex(&pattern) {
                Some(regex) => format!("#{not}match?{}\"{}\"", &caps[2], escape(&regex)),
                // Can't be converted: left as it was (and ignored).
                None => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// A query string's text (`\\` is `\`, `\"` is `"`).
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The regex for a Lua pattern's class letter (`a` in `%a`), as the
/// inside of a `[...]`; `None` for one that isn't a class.
fn class(letter: char) -> Option<&'static str> {
    Some(match letter.to_ascii_lowercase() {
        'a' => "A-Za-z",
        'd' => "0-9",
        'l' => "a-z",
        'u' => "A-Z",
        's' => r" \t\n\r\x0B\x0C",
        'w' => "A-Za-z0-9",
        'x' => "0-9A-Fa-f",
        'p' => "[:punct:]",
        'c' => "[:cntrl:]",
        _ => return None,
    })
}

/// Turns a Lua pattern into a regex with the same meaning, or `None` for
/// what regexes can't say (`%b()`, `%f[...]`, back-references).
pub fn lua_pattern_to_regex(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '%' => {
                let next = *chars.get(i + 1)?;
                i += 2;
                if let Some(set) = class(next) {
                    let negated = next.is_ascii_uppercase();
                    out.push_str(&format!("[{}{set}]", if negated { "^" } else { "" }));
                } else if next.is_ascii_alphanumeric() {
                    // %b, %f, %1...
                    return None;
                } else {
                    out.push_str(&regex::escape(&next.to_string()));
                }
                continue;
            }
            '[' => {
                let (set, end) = lua_set(&chars, i)?;
                out.push_str(&set);
                i = end;
                continue;
            }
            // Lua's lazy `*`.
            '-' => out.push_str("*?"),
            '\\' | '{' | '}' | '|' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

/// The set starting at `chars[start]` (`[`), as a regex class, and the
/// index after its `]`.
fn lua_set(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut out = String::from("[");
    let mut i = start + 1;
    if chars.get(i) == Some(&'^') {
        out.push('^');
        i += 1;
    }
    let mut first = true;
    loop {
        let c = *chars.get(i)?;
        match c {
            ']' if !first => return Some((out + "]", i + 1)),
            '%' => {
                let next = *chars.get(i + 1)?;
                i += 2;
                match class(next) {
                    // A nested class keeps negated ones right (`[%S_]`).
                    Some(set) if next.is_ascii_uppercase() => {
                        out.push_str(&format!("[^{set}]"));
                    }
                    Some(set) => out.push_str(set),
                    None if next.is_ascii_alphanumeric() => return None,
                    None => {
                        out.push('\\');
                        out.push(next);
                    }
                }
                first = false;
                continue;
            }
            '[' | '\\' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        first = false;
        i += 1;
    }
}

/// Neovim's indent captures as ferra's.
fn adapt_indent_captures(query: &str) -> String {
    static CAPTURE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"@indent\.([a-z_.]+)").expect("valid"));
    CAPTURE
        .replace_all(query, |caps: &regex::Captures| match &caps[1] {
            "begin" => "@indent".to_string(),
            "end" | "branch" | "dedent" => "@outdent".to_string(),
            // Not used by ferra: a capture it ignores.
            other => format!("@_indent.{other}"),
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> bool {
        let regex = lua_pattern_to_regex(pattern).unwrap();
        Regex::new(&regex).unwrap().is_match(text)
    }

    #[test]
    fn lua_patterns_become_regexes() {
        assert!(matches("^[A-Z_][a-zA-Z0-9_]*", "Foo_1"));
        assert!(!matches("^[A-Z_][a-zA-Z0-9_]*", "foo"));
        assert!(matches("^[A-Z][A-Z_0-9]+$", "MAX_SIZE"));
        assert!(!matches("^[A-Z][A-Z_0-9]+$", "Max"));
        assert!(matches("^//!", "//! docs"));
        assert!(matches("^%u%l+$", "Hello"));
        assert!(matches("^[%a_][%w_]*$", "_x1"));
        assert!(!matches("^[%a_][%w_]*$", "1x"));
        assert!(matches("^%-%-%-", "--- doc"));
        assert!(matches("^a.-b$", "axxb"));
        assert!(matches("^[%S]+$", "abc"));
        assert!(!matches("^[%S]+$", "a c"));
        assert!(matches("%.lua$", "init.lua"));
        assert!(!matches("%.lua$", "initxlua"));
        assert_eq!(lua_pattern_to_regex("%b()"), None);
        assert_eq!(lua_pattern_to_regex("%f[%w]"), None);
    }

    #[test]
    fn queries_are_rewritten() {
        let query = r#"((identifier) @constant (#lua-match? @constant "^[A-Z][A-Z_0-9]+$"))"#;
        assert_eq!(
            adapt(query),
            r#"((identifier) @constant (#match? @constant "^[A-Z][A-Z_0-9]+$"))"#
        );
        let query = r#"((comment) @c (#not-lua-match? @c "^%-%-%-"))"#;
        assert_eq!(
            adapt(query),
            r#"((comment) @c (#not-match? @c "^\\-\\-\\-"))"#
        );
        let indents = "(block) @indent.begin\n(block \"}\" @indent.end)\n(comment) @indent.ignore";
        assert_eq!(
            adapt(indents),
            "(block) @indent\n(block \"}\" @outdent)\n(comment) @_indent.ignore"
        );
        assert_eq!(
            adapt("(block) @indent\n\"}\" @outdent"),
            "(block) @indent\n\"}\" @outdent"
        );
    }
}
