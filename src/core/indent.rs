//! Working out how a file is indented, so new indentation matches it.
//!
//! - **The indent style** (`Editor::indent_style`, an `IndentUnit`: tabs or N
//!   spaces) is the buffer's `:indent` choice, else what `core/indent.rs`
//!   detected when the file was read or reloaded (with `detect-indent`), else
//!   `expand-tab` and `shift-width`. Detection looks at the first 1000 lines:
//!   stepping by tabs then spaces means tabs with levels narrower than a tab
//!   (below); mostly tab-indented means tabs; otherwise the commonest step by
//!   which a line's indent grows over the previous non-blank line's, ignoring one-space
//!   steps, `*` comment continuations and lines mixing tabs in (alignment, not
//!   indentation), with ties going to the narrower step. Everything that
//!   indents (`>`/`<`, block shifts, Tab, `=`, new lines, closing brackets)
//!   goes through it; `tab-width` still sets how wide a tab is drawn. The
//!   `indent` status line element shows it.
//! - **A level with tabs** is `shift-width` wide (`EditorOptions::level_width`:
//!   `tab-width` when it's 0). Narrower than a tab, a level is made of tabs
//!   then spaces, so everything that indents works out the width it wants and
//!   rebuilds the line's indentation to it (`Editor::indent_string`) rather
//!   than adding or taking away a tab.
//! - **A level detected narrower than a tab**: a file whose levels go
//!   4 spaces, a tab, a tab and 4 spaces... is seen as tabs with levels of 4,
//!   which then win over `shift-width`. It's told without knowing `tab-width`
//!   (detection runs when the file is read, before any window's options):
//!   lines step from tabs to the same tabs and S spaces, and from tabs and
//!   S spaces to one more tab and none, more often than by a whole tab. Both
//!   are needed, as a tab file aligning with spaces (`\tint a,` then
//!   `\t    b;`) has only the first.

use super::Buffer;

/// How one level of indentation is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentUnit {
    /// Tabs, then spaces for what's left of a level narrower than a tab.
    /// A level is `level` columns wide, or with `None`, `shift-width`'s.
    Tabs { level: Option<usize> },
    /// This many spaces.
    Spaces(usize),
}

impl IndentUnit {
    /// Tabs, with levels as wide as `shift-width` says.
    pub const TABS: Self = Self::Tabs { level: None };

    /// The columns of one level, `configured` (`shift-width`'s) unless the
    /// unit says.
    pub fn level_width(self, configured: usize) -> usize {
        match self {
            Self::Spaces(n) | Self::Tabs { level: Some(n) } => n.max(1),
            Self::Tabs { level: None } => configured.max(1),
        }
    }

    pub fn is_tabs(self) -> bool {
        matches!(self, Self::Tabs { .. })
    }
}

impl std::fmt::Display for IndentUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tabs { level: None } => write!(f, "tabs"),
            Self::Tabs { level: Some(n) } => write!(f, "tabs, levels of {n}"),
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
/// Mostly-tab-indented files indent with tabs, with levels narrower than a
/// tab if their lines step that way (see the module docs). Otherwise the
/// unit is the commonest step by which a line's indent grows over the line
/// before it (blank lines skipped). Steps of one space, and lines starting
/// with `*` (the middle of a `/* */` comment), are usually alignment, so
/// they're ignored; so are lines whose indent mixes in tabs.
pub fn detect(buffer: &Buffer) -> Option<IndentUnit> {
    let mut tab_lines = 0;
    let mut space_lines = 0;
    let mut steps = [0usize; MAX_WIDTH + 1];
    let mut previous: Option<usize> = Some(0);
    let mut levels = Levels::default();
    for row in 0..buffer.line_count().min(MAX_LINES) {
        let line = buffer.line(row);
        let rest = line.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            continue;
        }
        let lead = &line[..line.len() - rest.len()];
        levels.see(lead, rest.starts_with('*'));
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

    if tab_lines > 0
        && let Some(level) = levels.narrower_than_a_tab()
    {
        return Some(IndentUnit::Tabs { level: Some(level) });
    }
    if tab_lines > space_lines {
        return Some(IndentUnit::TABS);
    }
    let (width, &count) = steps
        .iter()
        .enumerate()
        .rev()
        .max_by_key(|&(_, count)| count)?;
    if count > 0 {
        Some(IndentUnit::Spaces(width))
    } else if tab_lines > 0 {
        Some(IndentUnit::TABS)
    } else {
        None
    }
}

/// How lines indented with tabs then spaces step from one to the next, to
/// tell levels narrower than a tab (see the module docs).
#[derive(Default)]
struct Levels {
    /// The previous line's tabs and spaces, if made that way.
    previous: Option<(usize, usize)>,
    /// Steps from tabs to the same tabs and this many spaces.
    onto_spaces: [usize; MAX_WIDTH + 1],
    /// Steps from tabs and this many spaces to one more tab and none.
    onto_tab: [usize; MAX_WIDTH + 1],
    /// Steps by a whole tab, no spaces either side.
    whole_tabs: usize,
}

impl Levels {
    fn see(&mut self, lead: &str, comment_star: bool) {
        let tabs = lead.len() - lead.trim_start_matches('\t').len();
        let spaces = lead.len() - tabs;
        if lead[tabs..].contains('\t') {
            self.previous = None;
            return;
        }
        if comment_star {
            return;
        }
        if let Some((tabs_before, spaces_before)) = self.previous {
            if tabs == tabs_before && tabs > 0 && spaces_before == 0 {
                if (2..=MAX_WIDTH).contains(&spaces) {
                    self.onto_spaces[spaces] += 1;
                }
            } else if tabs == tabs_before + 1 && spaces == 0 {
                if spaces_before == 0 {
                    self.whole_tabs += 1;
                } else if (2..=MAX_WIDTH).contains(&spaces_before) {
                    self.onto_tab[spaces_before] += 1;
                }
            }
        }
        self.previous = Some((tabs, spaces));
    }

    /// The commonest width that's stepped onto after tabs and stepped on
    /// from to a tab, if those steps outnumber whole tabs; ties go to the
    /// narrower.
    fn narrower_than_a_tab(&self) -> Option<usize> {
        let (width, count) = (2..=MAX_WIDTH)
            .rev()
            .filter(|&w| self.onto_spaces[w] > 0 && self.onto_tab[w] > 0)
            .map(|w| (w, self.onto_spaces[w] + self.onto_tab[w]))
            .max_by_key(|&(_, count)| count)?;
        (count > self.whole_tabs).then_some(width)
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
        assert_eq!(detect_in(text), Some(IndentUnit::TABS));
    }

    #[test]
    fn tabs_with_alignment_spaces() {
        let text = "int f() {\n\tint a = 1,\n\t    b = 2;\n\treturn a;\n}\n";
        assert_eq!(detect_in(text), Some(IndentUnit::TABS));
    }

    /// Levels of 4 with tabs of 8, as in many C projects.
    const NARROW_LEVELS: &str = "int f(void)\n{\n    if (a) {\n\tif (b) {\n\t    c();\n\t    if (d) {\n\t\te();\n\t    }\n\t}\n    }\n    return 0;\n}\n";

    #[test]
    fn tabs_then_spaces_make_levels_narrower_than_a_tab() {
        assert_eq!(
            detect_in(NARROW_LEVELS),
            Some(IndentUnit::Tabs { level: Some(4) })
        );
    }

    #[test]
    fn whole_tab_steps_outweigh_a_few_narrow_ones() {
        let text = format!("{NARROW_LEVELS}a\n\tb\n\t\tc\n\t\t\td\n\t\t\t\te\n");
        assert_eq!(detect_in(&text), Some(IndentUnit::TABS));
    }

    #[test]
    fn spaces_aligning_after_tabs_are_not_levels() {
        let text = "int f() {\n\tint a = 1,\n\t    b = 2;\n\tif (a)\n\t\treturn b;\n}\n";
        assert_eq!(detect_in(text), Some(IndentUnit::TABS));
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
        assert_eq!(IndentUnit::TABS.to_string(), "tabs");
        assert_eq!(
            IndentUnit::Tabs { level: Some(4) }.to_string(),
            "tabs, levels of 4"
        );
        assert_eq!(IndentUnit::Spaces(2).to_string(), "2 spaces");
    }
}
