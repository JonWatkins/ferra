//! Parsing `:g/pattern/command`.

/// A parsed `:g` (or `:v`) command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Global {
    /// Empty means "the last search pattern".
    pub pattern: String,
    /// The `:` command to run on each line; empty means none.
    pub command: String,
}

/// Parses the text after `:g`: a delimiter (any punctuation), the pattern
/// (`\` escapes the delimiter), the delimiter again, then the command as
/// it is. `g/pattern` with no command is allowed.
pub fn parse_global(text: &str) -> Result<Global, String> {
    const USAGE: &str = "Usage: g/pattern/command";
    let mut chars = text.chars();
    let delimiter = chars.next().ok_or(USAGE)?;
    if delimiter.is_alphanumeric() || delimiter.is_whitespace() || delimiter == '\\' {
        return Err(USAGE.into());
    }
    let mut pattern = String::new();
    let rest = chars.as_str();
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if escaped {
            if c != delimiter {
                pattern.push('\\');
            }
            pattern.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter {
            return Ok(Global {
                pattern,
                command: rest[i + c.len_utf8()..].trim().to_string(),
            });
        } else {
            pattern.push(c);
        }
    }
    if escaped {
        pattern.push('\\');
    }
    // No closing delimiter: no command.
    Ok(Global {
        pattern,
        command: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(pattern: &str, command: &str) -> Global {
        Global {
            pattern: pattern.into(),
            command: command.into(),
        }
    }

    #[test]
    fn pattern_and_command() {
        assert_eq!(parse_global("/TODO/d").unwrap(), global("TODO", "d"));
        assert_eq!(
            parse_global("/a/s/a/b/g").unwrap(),
            global("a", "s/a/b/g"),
            "the command keeps its own slashes"
        );
        assert_eq!(parse_global("#x/y#d").unwrap(), global("x/y", "d"));
    }

    #[test]
    fn escapes_and_missing_parts() {
        assert_eq!(parse_global(r"/a\/b/d").unwrap(), global("a/b", "d"));
        assert_eq!(parse_global(r"/\d+/d").unwrap(), global(r"\d+", "d"));
        assert_eq!(parse_global("/only").unwrap(), global("only", ""));
        assert_eq!(parse_global("//d").unwrap(), global("", "d"));
        assert!(parse_global("").is_err());
        assert!(parse_global("abc").is_err());
    }
}
