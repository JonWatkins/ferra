//! Working out how a file is indented, so new indentation matches it.

use super::Buffer;

/// How one level of indentation is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentUnit {
    /// A tab character (as wide as `tab-width`).
    Tabs,
    /// This many spaces.
    Spaces(usize),
}

impl std::fmt::Display for IndentUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tabs => write!(f, "tabs"),
            Self::Spaces(1) => write!(f, "1 space"),
            Self::Spaces(n) => write!(f, "{n} spaces"),
        }
    }
}

/// Lines looked at; enough to see the style without reading huge files.
const MAX_LINES: usize = 1000;
/// The widest space indent recognised.
const MAX_WIDTH: usize = 8;

/// Guesses the file's indentation from its first lines, or `None` if
/// nothing is indented.
///
/// Mostly-tab-indented files indent with tabs. Otherwise the unit is the
/// commonest step by which a line's indent grows over the line before it
/// (blank lines skipped). Steps of one space, and lines starting with `*`
/// (the middle of a `/* */` comment), are usually alignment, so they're
/// ignored; so are lines whose indent mixes in tabs.
pub fn detect(buffer: &Buffer) -> Option<IndentUnit> {
    let mut tab_lines = 0;
    let mut space_lines = 0;
    let mut steps = [0usize; MAX_WIDTH + 1];
    let mut previous: Option<usize> = Some(0);
    for row in 0..buffer.line_count().min(MAX_LINES) {
        let line = buffer.line(row);
        let rest = line.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            continue;
        }
        let lead = &line[..line.len() - rest.len()];
        if lead.starts_with('\t') {
            tab_lines += 1;
            previous = None;
            continue;
        }
        if lead.contains('\t') {
            previous = None;
            continue;
        }
        if rest.starts_with('*') {
            continue;
        }
        let spaces = lead.len();
        if spaces > 0 {
            space_lines += 1;
        }
        if let Some(before) = previous
            && spaces > before + 1
            && spaces - before <= MAX_WIDTH
        {
            steps[spaces - before] += 1;
        }
        previous = Some(spaces);
    }

    if tab_lines > space_lines {
        return Some(IndentUnit::Tabs);
    }
    let (width, &count) = steps
        .iter()
        .enumerate()
        .rev()
        .max_by_key(|&(_, count)| count)?;
    if count > 0 {
        Some(IndentUnit::Spaces(width))
    } else if tab_lines > 0 {
        Some(IndentUnit::Tabs)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_in(text: &str) -> Option<IndentUnit> {
        detect(&Buffer::from_text(text))
    }

    #[test]
    fn two_spaces() {
        let text = "fn main() {\n  if x {\n    y();\n  }\n  z();\n}\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Spaces(2)));
    }

    #[test]
    fn four_spaces_despite_deeper_continuations() {
        let text = "def f(a,\n        b):\n    if a:\n        return b\n    return a\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Spaces(4)));
    }

    #[test]
    fn tabs() {
        let text = "func f() {\n\tif x {\n\t\ty()\n\t}\n}\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Tabs));
    }

    #[test]
    fn tabs_with_alignment_spaces() {
        let text = "int f() {\n\tint a = 1,\n\t    b = 2;\n\treturn a;\n}\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Tabs));
    }

    #[test]
    fn nothing_indented() {
        assert_eq!(detect_in("a\nb\n\nc\n"), None);
        assert_eq!(detect_in(""), None);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = "a:\n\n   b\n\n      c\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Spaces(3)));
    }

    #[test]
    fn comment_stars_and_one_space_steps_are_ignored() {
        let text = "/**\n * Doc.\n */\nfn f() {\n  a();\n  /*\n   * more\n   */\n}\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Spaces(2)));
    }

    #[test]
    fn ties_go_to_the_narrower_step() {
        let text = "a\n  b\nc\n    d\n";
        assert_eq!(detect_in(text), Some(IndentUnit::Spaces(2)));
    }

    #[test]
    fn display() {
        assert_eq!(IndentUnit::Tabs.to_string(), "tabs");
        assert_eq!(IndentUnit::Spaces(2).to_string(), "2 spaces");
    }
}
