//! The hover popup: a language server's Markdown about the symbol under the
//! cursor, highlighted as Markdown (so code blocks are in their language),
//! with the code fences hidden and long lines wrapped between words.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Style,
    text::Line,
    widgets::{Block, Clear, Widget},
};
use unicode_width::UnicodeWidthChar;

use crate::config::Theme;
use crate::editor::lsp::Hover;
use crate::syntax::Highlights;

/// The widest and tallest the text gets, in cells.
const MAX_WIDTH: usize = 80;
const MAX_HEIGHT: usize = 20;

/// A line of the hover as shown: its visible chars (Markdown's `**` and
/// backticks hidden), each with its column in the document's line, for its
/// highlight.
struct ShownLine {
    row: usize,
    chars: Vec<(char, usize)>,
    /// Inside a code block: styled by its language alone.
    code: bool,
    /// A `---` line, drawn across.
    rule: bool,
}

/// One row of the popup: chars `start..end` of a shown line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    line: usize,
    start: usize,
    end: usize,
}

pub struct HoverView<'a> {
    hover: &'a Hover,
    highlights: Option<&'a Highlights>,
    theme: &'a Theme,
    lines: Vec<ShownLine>,
}

impl<'a> HoverView<'a> {
    pub fn new(hover: &'a Hover, highlights: Option<&'a Highlights>, theme: &'a Theme) -> Self {
        let lines = shown_lines(hover, highlights);
        Self {
            hover,
            highlights,
            theme,
            lines,
        }
    }

    /// The rows for a popup `width` cells wide.
    fn rows(&self, width: usize) -> Vec<Row> {
        let mut rows = Vec::new();
        for (line, shown) in self.lines.iter().enumerate() {
            if shown.rule {
                rows.push(Row {
                    line,
                    start: 0,
                    end: 0,
                });
                continue;
            }
            let chars: Vec<char> = shown.chars.iter().map(|(c, _)| *c).collect();
            for (start, end) in wrap(&chars, width) {
                rows.push(Row { line, start, end });
            }
        }
        rows
    }

    /// How wide the text is: its widest line, up to the most there's room
    /// for.
    fn text_width(&self, screen: Rect) -> usize {
        let widest = self
            .lines
            .iter()
            .filter(|l| !l.rule)
            .map(|l| {
                l.chars
                    .iter()
                    .map(|(c, _)| c.width().unwrap_or(0))
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0);
        widest
            .clamp(1, MAX_WIDTH)
            .min((screen.width as usize).saturating_sub(4).max(1))
    }

    /// The style of the char at `col` of document line `row`.
    fn style_at(&self, row: usize, col: usize, code: bool) -> Style {
        let spans = self.highlights.map_or(&[][..], |h| h.line(row));
        spans
            .iter()
            .filter(|s| s.start <= col && col < s.end)
            // In a code block, Markdown's own style for it would show
            // through wherever its language has none.
            .filter(|s| !(code && s.name().starts_with("text")))
            .fold(Style::default(), |style, s| {
                style.patch(self.theme.get(s.name()))
            })
    }

    /// Where the popup goes for a cursor at `cursor` in `screen`: below the
    /// cursor's row if it fits there, else above, else wherever has more
    /// room.
    pub fn area(&self, cursor: Position, screen: Rect) -> Rect {
        let width = self.text_width(screen);
        let rows = self.rows(width).len().clamp(1, MAX_HEIGHT) as u16;
        let width = width as u16 + 4;
        let height = rows + 2;
        let below = (screen.y + screen.height).saturating_sub(cursor.y + 1);
        let above = cursor.y.saturating_sub(screen.y);
        let (y, height) = if height <= below || below >= above {
            (cursor.y + 1, height.min(below))
        } else {
            (cursor.y - height.min(above), height.min(above))
        };
        let x = cursor
            .x
            .saturating_sub(1)
            .max(screen.x)
            .min((screen.x + screen.width).saturating_sub(width));
        Rect::new(x, y, width, height).intersection(screen)
    }
}

/// The hover's lines as shown: without the code fences, runs of blank
/// lines, blank lines beside a rule, or Markdown's emphasis and code-span
/// markers.
fn shown_lines(hover: &Hover, highlights: Option<&Highlights>) -> Vec<ShownLine> {
    let buffer = hover.document.buffer();
    let is_rule = |text: &str| matches!(text.trim(), "---" | "***" | "___");
    let mut lines: Vec<ShownLine> = Vec::new();
    let mut in_code = false;
    for row in 0..buffer.line_count() {
        let text = buffer.line(row).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        let blank = text.trim().is_empty();
        if blank && lines.last().is_none_or(|l| l.chars.is_empty() && !l.rule) {
            continue;
        }
        let spans = highlights.map_or(&[][..], |h| h.line(row));
        let all: Vec<char> = text.chars().collect();
        // A heading's `### `.
        let heading = all.iter().take_while(|&&c| c == '#').count();
        let heading = if heading > 0 && all.get(heading) == Some(&' ') {
            heading + 1
        } else {
            0
        };
        let hidden = |col: usize| {
            if in_code {
                return false;
            }
            col < heading
                || spans.iter().any(|s| {
                    s.start <= col
                        && col < s.end
                        && (s.name() == "punctuation.delimiter"
                            // A link's `(url)` after its `[text]`.
                            || s.name() == "text.uri"
                                && s.start >= 2
                                && all[s.start - 1] == '('
                                && all[s.start - 2] == ']')
                })
        };
        let chars = if blank {
            Vec::new()
        } else {
            text.chars()
                .enumerate()
                .filter(|&(col, _)| !hidden(col))
                .map(|(col, c)| (c, col))
                .collect()
        };
        lines.push(ShownLine {
            row,
            chars,
            code: in_code,
            rule: !in_code && is_rule(text),
        });
    }
    // The rule between the signature and the docs is gap enough.
    let blank = |l: &ShownLine| l.chars.is_empty() && !l.rule;
    let keep: Vec<bool> = (0..lines.len())
        .map(|i| {
            !blank(&lines[i])
                || !(i > 0 && lines[i - 1].rule || lines.get(i + 1).is_some_and(|l| l.rule))
        })
        .collect();
    let mut lines: Vec<ShownLine> = lines
        .into_iter()
        .zip(keep)
        .filter_map(|(line, keep)| keep.then_some(line))
        .collect();
    while lines.last().is_some_and(blank) {
        lines.pop();
    }
    lines
}

/// `chars` split into rows at most `width` cells wide, between words where
/// there's a space to break at: each row's char range.
pub(super) fn wrap(chars: &[char], width: usize) -> Vec<(usize, usize)> {
    if chars.is_empty() {
        return vec![(0, 0)];
    }
    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut cells = 0;
        let mut end = start;
        while end < chars.len() {
            let w = chars[end].width().unwrap_or(0);
            if cells + w > width {
                break;
            }
            cells += w;
            end += 1;
        }
        if end < chars.len() {
            // Back to after the last space, unless that leaves nothing.
            if let Some(space) = (start + 1..end).rev().find(|&i| chars[i] == ' ') {
                end = space + 1;
            }
            end = end.max(start + 1);
        }
        rows.push((start, end));
        start = end;
    }
    rows
}

impl Widget for HoverView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width < 5 {
            return;
        }
        Clear.render(area, buf);
        let base = self.theme.get("ui.popup");
        let width = (area.width - 4) as usize;
        let rows = self.rows(width);
        let shown = (area.height - 2) as usize;
        let scroll = self.hover.scroll.min(rows.len().saturating_sub(1));
        self.hover.shown.set(shown);
        self.hover.total.set(rows.len());
        let mut block = Block::bordered()
            .border_style(base.patch(self.theme.get("ui.popup.border")))
            .style(base);
        if scroll + shown < rows.len() {
            block = block.title_bottom(Line::from(" K: more ").right_aligned());
        }
        let inner = block.inner(area);
        block.render(area, buf);

        for (i, row) in rows.iter().skip(scroll).take(shown).enumerate() {
            let y = inner.y + i as u16;
            let line = &self.lines[row.line];
            let mut x = inner.x + 1;
            if line.rule {
                let rule = "─".repeat(width);
                buf.set_string(x, y, rule, base.patch(self.theme.get("ui.popup.border")));
                continue;
            }
            for &(c, col) in &line.chars[row.start..row.end] {
                let w = c.width().unwrap_or(0) as u16;
                if w == 0 {
                    continue;
                }
                let style = self.style_at(line.row, col, line.code);
                buf[(x, y)].set_char(c).set_style(base.patch(style));
                x += w;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(markdown: &str) -> Vec<String> {
        let mut editor = crate::editor::Editor::default();
        editor.show_hover(markdown);
        let hover = editor.hover().unwrap();
        let mut engine = crate::syntax::SyntaxEngine::default();
        let rows = 0..hover.document.buffer().line_count();
        let languages = crate::config::Config::default().languages;
        let highlights = engine.highlight_as(
            &hover.document,
            Some("markdown"),
            &languages,
            &editor.syntax_trees,
            rows,
        );
        shown_lines(hover, highlights)
            .iter()
            .map(|l| {
                if l.rule {
                    "---".to_string()
                } else {
                    l.chars.iter().map(|(c, _)| c).collect()
                }
            })
            .collect()
    }

    #[test]
    fn markdown_markup_is_hidden() {
        let lines = shown(
            "### `area`\n\n```rust\nfn area(**r**: f64)\n```\n\n---\n\n\
             A **bold** and `code`, [docs](https://x.dev/a).\n\n\n\nEnd\n",
        );
        assert_eq!(
            lines,
            [
                "area",
                "",
                "fn area(**r**: f64)",
                "---",
                "A bold and code, docs.",
                "",
                "End",
            ],
            "code kept as it is; blank runs and blanks by the rule dropped"
        );
    }

    #[test]
    fn wrapping_breaks_between_words() {
        let chars: Vec<char> = "one two three".chars().collect();
        assert_eq!(
            wrap(&chars, 8),
            [(0, 8), (8, 13)],
            "`one two ` then `three`"
        );
        let long: Vec<char> = "abcdefghij".chars().collect();
        assert_eq!(wrap(&long, 4), [(0, 4), (4, 8), (8, 10)], "no space: cut");
        assert_eq!(wrap(&[], 4), [(0, 0)]);
    }
}
