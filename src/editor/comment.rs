//! `gc`: commenting lines out and back in, like Neovim's built-in
//! commenting.
//!
//! If every non-blank line in the range is already commented, they're all
//! uncommented; otherwise they're all commented, with the comment marker at
//! the shallowest indent so the block stays aligned. Blank lines are left
//! alone. Languages without line comments (HTML, CSS) wrap each line in
//! their block comment.

use super::Editor;
use crate::config::CommentTokens;
use crate::core::{Position, motion};

impl Editor {
    /// Toggles comments on lines `first..=last` of the active buffer.
    pub fn toggle_comment(&mut self, first: usize, last: usize) -> Result<(), String> {
        let tokens = self.comment_tokens()?;
        let doc = &self.document;
        let lines: Vec<(usize, String)> = (first..=last)
            .map(|row| (row, doc.buffer().line(row).into_owned()))
            .filter(|(_, line)| !line.trim().is_empty())
            .collect();
        if lines.is_empty() {
            return Ok(());
        }

        if lines.iter().all(|(_, line)| is_commented(line, &tokens)) {
            for (row, line) in &lines {
                self.replace_line(*row, line, &uncomment(line, &tokens));
            }
        } else {
            let indent = lines
                .iter()
                .map(|(_, line)| indent_len(line))
                .min()
                .unwrap_or(0);
            for (row, line) in &lines {
                self.replace_line(*row, line, &comment(line, indent, &tokens));
            }
        }
        let doc = &mut self.document;
        let col = motion::first_non_blank(doc.buffer(), first);
        doc.set_cursor(Position::new(first, col));
        Ok(())
    }

    /// The active buffer's comment markers, from its language.
    fn comment_tokens(&self) -> Result<CommentTokens, String> {
        let language = self
            .language_of(&self.document)
            .ok_or("Can't comment: no language for this file")?;
        self.languages
            .comment_tokens(language)
            .ok_or_else(|| format!("Can't comment: {language} has no comments"))
    }

    /// Replaces line `row`'s text (`old`) with `new`, editing only the part
    /// that differs, so marks and the cursor elsewhere on the line stay put.
    fn replace_line(&mut self, row: usize, old: &str, new: &str) {
        if old == new {
            return;
        }
        let old_chars: Vec<char> = old.chars().collect();
        let new_chars: Vec<char> = new.chars().collect();
        let prefix = old_chars
            .iter()
            .zip(&new_chars)
            .take_while(|(a, b)| a == b)
            .count();
        let room = old_chars.len().min(new_chars.len()) - prefix;
        let suffix = old_chars
            .iter()
            .rev()
            .zip(new_chars.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let doc = &mut self.document;
        let start = Position::new(row, prefix);
        doc.delete(start, Position::new(row, old_chars.len() - suffix));
        let middle: String = new_chars[prefix..new_chars.len() - suffix].iter().collect();
        doc.insert(start, &middle);
    }
}

/// Leading whitespace, in chars.
fn indent_len(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

fn is_commented(line: &str, tokens: &CommentTokens) -> bool {
    let text = line.trim();
    text.starts_with(&tokens.start)
        && tokens.end.as_ref().is_none_or(|end| {
            text.ends_with(end.as_str()) && text.len() >= tokens.start.len() + end.len()
        })
}

/// `line` with the marker inserted at char column `indent` (and the block
/// end appended).
fn comment(line: &str, indent: usize, tokens: &CommentTokens) -> String {
    let split = line
        .char_indices()
        .nth(indent)
        .map_or(line.len(), |(i, _)| i);
    let (head, body) = line.split_at(split);
    match &tokens.end {
        None => format!("{head}{} {body}", tokens.start),
        Some(end) => format!("{head}{} {body} {end}", tokens.start),
    }
}

/// `line` without its marker, and one space after it (and before the block
/// end) if there is one.
fn uncomment(line: &str, tokens: &CommentTokens) -> String {
    let indent = line.len() - line.trim_start().len();
    let (head, rest) = line.split_at(indent);
    let rest = rest.trim_end();
    let mut body = rest.strip_prefix(tokens.start.as_str()).unwrap_or(rest);
    body = body.strip_prefix(' ').unwrap_or(body);
    if let Some(end) = &tokens.end {
        body = body.strip_suffix(end.as_str()).unwrap_or(body);
        body = body.strip_suffix(' ').unwrap_or(body);
    }
    format!("{head}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_tokens(start: &str) -> CommentTokens {
        CommentTokens {
            start: start.into(),
            end: None,
        }
    }

    fn block_tokens(start: &str, end: &str) -> CommentTokens {
        CommentTokens {
            start: start.into(),
            end: Some(end.into()),
        }
    }

    #[test]
    fn comment_and_uncomment_line_comments() {
        let slashes = line_tokens("//");
        assert_eq!(comment("    let x;", 4, &slashes), "    // let x;");
        assert_eq!(
            comment("        y;", 4, &slashes),
            "    //     y;",
            "at the shallowest indent"
        );
        assert_eq!(uncomment("    // let x;", &slashes), "    let x;");
        assert_eq!(
            uncomment("    //let x;", &slashes),
            "    let x;",
            "no space to remove"
        );
        assert!(is_commented("  // x", &slashes));
        assert!(!is_commented("x // y", &slashes));
    }

    #[test]
    fn comment_and_uncomment_block_comments() {
        let html = block_tokens("<!--", "-->");
        assert_eq!(comment("  <p>hi</p>", 2, &html), "  <!-- <p>hi</p> -->");
        assert_eq!(uncomment("  <!-- <p>hi</p> -->", &html), "  <p>hi</p>");
        assert!(is_commented("<!-- x -->", &html));
        assert!(!is_commented("<!-- x", &html), "needs the end too");
        assert!(!is_commented("<!-->", &html), "start and end can't overlap");
    }
}
