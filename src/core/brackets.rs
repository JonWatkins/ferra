//! Finding brackets in text, and how deeply each is nested, for bracket
//! pair colours: a light scan that skips strings (`"…"`, `` `…` ``, and
//! `'x'` char literals, not Rust's lifetimes or an apostrophe) and
//! comments (the language's line comment, or its block comment), so the
//! depth at each line's start can be worked out once per edit for the
//! whole file.

/// What a line starts inside of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineState {
    /// How many brackets are open.
    pub depth: u32,
    /// Inside a block comment.
    pub in_comment: bool,
}

/// The language's comments, as the scan needs them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Comments {
    /// `//`, `#`.
    pub line: Option<String>,
    /// `/*` and `*/`, `<!--` and `-->`.
    pub block: Option<(String, String)>,
}

/// A bracket found: its column (in chars), the depth it's at (an opener's
/// before it opens, a closer's once closed: a pair has the same), and
/// whether it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bracket {
    pub col: usize,
    pub depth: u32,
    pub open: bool,
}

fn starts_with_at(chars: &[char], at: usize, token: &str) -> bool {
    let token: Vec<char> = token.chars().collect();
    !token.is_empty() && chars.len() >= at + token.len() && chars[at..at + token.len()] == token[..]
}

/// Scans `line`, starting in `state` (left as the next line starts),
/// giving each bracket outside strings and comments.
pub fn scan(line: &str, state: &mut LineState, comments: &Comments) -> Vec<Bracket> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if state.in_comment {
            match &comments.block {
                Some((_, end)) if starts_with_at(&chars, i, end) => {
                    state.in_comment = false;
                    i += end.chars().count();
                }
                _ => i += 1,
            }
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if let Some(token) = &comments.line
            && starts_with_at(&chars, i, token)
        {
            break;
        }
        if let Some((start, _)) = &comments.block
            && starts_with_at(&chars, i, start)
        {
            state.in_comment = true;
            i += start.chars().count();
            continue;
        }
        match c {
            '"' | '`' => quote = Some(c),
            // A char literal: `'x'` or `'\n'` (not `'a` or `don't`).
            '\'' => {
                let close = if chars.get(i + 1) == Some(&'\\') {
                    i + 3
                } else {
                    i + 2
                };
                if chars.get(close) == Some(&'\'') {
                    i = close + 1;
                    continue;
                }
            }
            '(' | '[' | '{' => {
                found.push(Bracket {
                    col: i,
                    depth: state.depth,
                    open: true,
                });
                state.depth += 1;
            }
            ')' | ']' | '}' => {
                state.depth = state.depth.saturating_sub(1);
                found.push(Bracket {
                    col: i,
                    depth: state.depth,
                    open: false,
                });
            }
            _ => {}
        }
        i += 1;
    }
    found
}

/// Each line's starting state, for `lines` (one more than the lines: the
/// state after the last).
pub fn line_states<'a>(
    lines: impl Iterator<Item = std::borrow::Cow<'a, str>>,
    comments: &Comments,
) -> Vec<LineState> {
    let mut state = LineState::default();
    let mut out = vec![state];
    for line in lines {
        scan(&line, &mut state, comments);
        out.push(state);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust() -> Comments {
        Comments {
            line: Some("//".into()),
            block: None,
        }
    }

    fn depths(line: &str) -> Vec<(usize, u32, bool)> {
        let mut state = LineState::default();
        scan(line, &mut state, &rust())
            .into_iter()
            .map(|b| (b.col, b.depth, b.open))
            .collect()
    }

    #[test]
    fn pairs_share_a_depth() {
        assert_eq!(
            depths("f(a[0], {b})"),
            [
                (1, 0, true),
                (3, 1, true),
                (5, 1, false),
                (8, 1, true),
                (10, 1, false),
                (11, 0, false)
            ]
        );
    }

    #[test]
    fn strings_comments_and_char_literals_are_skipped() {
        assert_eq!(
            depths(r#"f("(", ')', '\'') // ("#),
            [(1, 0, true), (16, 0, false)]
        );
        // A lifetime isn't a char literal.
        assert_eq!(depths("fn f<'a>(x: &'a T) {}").len(), 4);
    }

    #[test]
    fn block_comments_and_depth_carry_over_lines() {
        let html = Comments {
            line: None,
            block: Some(("<!--".into(), "-->".into())),
        };
        let lines = ["<p>(<!-- ( ", "still ) -->", "x)"];
        let states = line_states(lines.iter().map(|l| std::borrow::Cow::Borrowed(*l)), &html);
        assert_eq!(
            states[1],
            LineState {
                depth: 1,
                in_comment: true
            }
        );
        assert_eq!(
            states[2],
            LineState {
                depth: 1,
                in_comment: false
            }
        );
        assert_eq!(states[3].depth, 0);
    }
}
