//! The hover popup: a language server's Markdown about the symbol under the
//! cursor, highlighted as Markdown (so code blocks are in their language),
//! with the code fences hidden and long lines wrapped between words.
//!
//! - Hover: the contents (Markdown, a marked string, or a list of them) become
//!   one Markdown text in a document of its own (`HOVER_DOCUMENT`, never a
//!   buffer). `ui/hover.rs` highlights it as Markdown with `highlight_as`, so
//!   code blocks get their language's highlighting through the usual
//!   injections, and draws it in a popup by the cursor: fences hidden, as are
//!   Markdown's emphasis and code-span markers, headings' `#`s and links' URLs
//!   (from the inline grammar's spans), blank runs squeezed, `---` drawn as a
//!   rule, long lines wrapped between words. A list's `-`, `*` or `+` is
//!   drawn as `•`, and an item's wrapped rows hang under its text. A pipe
//!   table (which the grammar leaves unhighlighted) is laid out from its
//!   text: columns padded to line up and aligned as its delimiter row says,
//!   `│` between cells, the header bold over a `─┼─` rule. Inside code blocks Markdown's own
//!   `text.*` styles are dropped so they don't show through. The popup closes
//!   on the next key unless that's `K` again, which scrolls a page (the draw
//!   records how many rows there are and fit).
//! - The chosen completion item's documentation is drawn the same way,
//!   beside the menu (`beside`), cut off rather than scrolled.

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

/// Where a shown char comes from, for its style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// The document line's char at this column, styled by its highlight.
    Doc(usize),
    /// Drawn by the popup (a table's padding and borders), in this theme
    /// style.
    Drawn(&'static str),
}

/// A line of the hover as shown: its visible chars (Markdown's `**` and
/// backticks hidden), each with where it comes from, for its style.
struct ShownLine {
    row: usize,
    chars: Vec<(char, Source)>,
    /// Inside a code block: styled by its language alone.
    code: bool,
    /// A `---` line, drawn across.
    rule: bool,
    /// How far its wrapped rows are indented, in cells: a list item's
    /// hang under its text.
    indent: usize,
    /// A table's header row, drawn bold.
    header: bool,
}

impl ShownLine {
    fn new(row: usize, chars: Vec<(char, Source)>) -> Self {
        Self {
            row,
            chars,
            code: false,
            rule: false,
            indent: 0,
            header: false,
        }
    }
}

/// One row of the popup: chars `start..end` of a shown line, `indent` cells
/// in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    line: usize,
    start: usize,
    end: usize,
    indent: usize,
}

pub struct HoverView<'a> {
    hover: &'a Hover,
    highlights: Option<&'a Highlights>,
    theme: &'a Theme,
    lines: Vec<ShownLine>,
    /// Whether `K` scrolls it (and the border says so when there's more).
    scrolls: bool,
}

/// The narrowest the completion's documentation is drawn beside the menu
/// (its text, in cells): with less room on either side it isn't shown.
const MIN_BESIDE: usize = 20;

impl<'a> HoverView<'a> {
    pub fn new(hover: &'a Hover, highlights: Option<&'a Highlights>, theme: &'a Theme) -> Self {
        let lines = shown_lines(hover, highlights);
        Self {
            hover,
            highlights,
            theme,
            lines,
            scrolls: true,
        }
    }

    /// For text `K` doesn't scroll (the completion's documentation): what
    /// doesn't fit is cut off, without the hint.
    pub fn without_scrolling(mut self) -> Self {
        self.scrolls = false;
        self
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
                    indent: 0,
                });
                continue;
            }
            let chars: Vec<char> = shown.chars.iter().map(|(c, _)| *c).collect();
            for (start, end, indent) in wrap_hanging(&chars, width, shown.indent) {
                rows.push(Row {
                    line,
                    start,
                    end,
                    indent,
                });
            }
        }
        rows
    }

    /// How wide the text is: its widest line, up to the most there's room
    /// for.
    fn text_width(&self, screen: Rect) -> usize {
        self.text_width_in((screen.width as usize).saturating_sub(4).max(1))
    }

    /// Its widest line, at most `room` (and `MAX_WIDTH`) cells.
    fn text_width_in(&self, room: usize) -> usize {
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
        widest.clamp(1, MAX_WIDTH).min(room.max(1))
    }

    /// Where it goes beside the completion menu at `menu`: to its right if
    /// there's room, else to its left, its top level with the menu's (moved
    /// up to fit). `None` if there's no room either side.
    pub fn beside(&self, menu: Rect, screen: Rect) -> Option<Rect> {
        // The room for the text each side, less the border and padding.
        let right = ((screen.x + screen.width).saturating_sub(menu.x + menu.width) as usize)
            .saturating_sub(4);
        let left = (menu.x.saturating_sub(screen.x) as usize).saturating_sub(4);
        // Short text needs only its own width.
        let needed = MIN_BESIDE.min(self.text_width_in(MAX_WIDTH));
        let (on_right, text) = if right >= needed {
            (true, right)
        } else if left >= needed {
            (false, left)
        } else {
            return None;
        };
        let width = self.text_width_in(text);
        let rows = self.rows(width).len().clamp(1, MAX_HEIGHT) as u16;
        let (width, height) = (width as u16 + 4, (rows + 2).min(screen.height));
        let x = if on_right {
            menu.x + menu.width
        } else {
            menu.x - width
        };
        let y = menu
            .y
            .min((screen.y + screen.height).saturating_sub(height));
        Some(Rect::new(x, y.max(screen.y), width, height).intersection(screen))
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
    /// Where it goes: below `cursor`, or above if there's more room there;
    /// with `above` (the pointer's, as VS Code's, so moving down a line
    /// leaves it), above if it fits.
    pub fn area(&self, cursor: Position, screen: Rect, above_first: bool) -> Rect {
        let width = self.text_width(screen);
        let rows = self.rows(width).len().clamp(1, MAX_HEIGHT) as u16;
        let width = width as u16 + 4;
        let height = rows + 2;
        let below = (screen.y + screen.height).saturating_sub(cursor.y + 1);
        let above = cursor.y.saturating_sub(screen.y);
        let go_below = if above_first {
            height > above && below > above
        } else {
            height <= below || below >= above
        };
        let (y, height) = if go_below {
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
/// markers; lists' bullets drawn as `•` and tables laid out.
fn shown_lines(hover: &Hover, highlights: Option<&Highlights>) -> Vec<ShownLine> {
    let buffer = hover.document.buffer();
    let text_of = |row: usize| {
        let text = buffer.line(row).to_string();
        text.trim_end_matches(['\n', '\r']).to_string()
    };
    let is_rule = |text: &str| matches!(text.trim(), "---" | "***" | "___");
    let mut lines: Vec<ShownLine> = Vec::new();
    let mut in_code = false;
    let mut row = 0;
    while row < buffer.line_count() {
        let text = text_of(row);
        if text.trim_start().starts_with("```") {
            in_code = !in_code;
            row += 1;
            continue;
        }
        let blank = text.trim().is_empty();
        if blank && lines.last().is_none_or(|l| l.chars.is_empty() && !l.rule) {
            row += 1;
            continue;
        }
        if !in_code {
            let rows: Vec<String> = (row..buffer.line_count()).map(text_of).collect();
            if let Some((table, taken)) = table(row, &rows) {
                lines.extend(table);
                row += taken;
                continue;
            }
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
        let rule = !in_code && is_rule(&text);
        let marker = (!in_code && !rule).then(|| list_marker(&all)).flatten();
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
            all.iter()
                .enumerate()
                .filter(|&(col, _)| !hidden(col))
                .map(|(col, &c)| match marker {
                    Some(m) if m.bullet && col == m.len - 2 => ('•', Source::Doc(col)),
                    _ => (c, Source::Doc(col)),
                })
                .collect()
        };
        lines.push(ShownLine {
            code: in_code,
            rule,
            indent: marker.map_or(0, |m| m.len),
            ..ShownLine::new(row, chars)
        });
        row += 1;
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

/// The start of a list item: `len` chars of indent, marker and the space
/// after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListMarker {
    len: usize,
    /// `-`, `*` or `+`, rather than a number.
    bullet: bool,
}

/// The list marker `line` starts with, if it's a list item.
fn list_marker(line: &[char]) -> Option<ListMarker> {
    let indent = line.iter().take_while(|&&c| c == ' ').count();
    let rest = &line[indent..];
    if rest.len() > 2 && matches!(rest[0], '-' | '*' | '+') && rest[1] == ' ' {
        return Some(ListMarker {
            len: indent + 2,
            bullet: true,
        });
    }
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    let numbered = (1..=9).contains(&digits)
        && matches!(rest.get(digits), Some('.' | ')'))
        && rest.get(digits + 1) == Some(&' ');
    numbered.then_some(ListMarker {
        len: indent + digits + 2,
        bullet: false,
    })
}

/// How a table's column lines its cells up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// The cells of a table row: each one's column range in `line`, without
/// the spaces round it. `|`s escaped or in a code span don't split.
fn table_cells(line: &[char]) -> Vec<(usize, usize)> {
    let mut bars = Vec::new();
    let mut in_code = false;
    for (col, &c) in line.iter().enumerate() {
        match c {
            '`' => in_code = !in_code,
            '|' if !in_code && (col == 0 || line[col - 1] != '\\') => bars.push(col),
            _ => {}
        }
    }
    let first = line.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let last = line
        .iter()
        .rposition(|c| !c.is_whitespace())
        .map_or(0, |i| i + 1);
    let mut bounds = vec![first];
    for &bar in &bars {
        bounds.push(bar);
        bounds.push(bar + 1);
    }
    bounds.push(last);
    let mut cells: Vec<(usize, usize)> = bounds
        .chunks(2)
        .map(|pair| {
            let (mut start, mut end) = (pair[0], pair[1].max(pair[0]));
            while start < end && line[start].is_whitespace() {
                start += 1;
            }
            while end > start && line[end - 1].is_whitespace() {
                end -= 1;
            }
            (start, end)
        })
        .collect();
    // The empty cells outside a leading and a trailing `|`.
    if bars.first() == Some(&first) {
        cells.remove(0);
    }
    if !bars.is_empty() && bars.last() == Some(&(last.saturating_sub(1))) {
        cells.pop();
    }
    cells
}

/// A table's delimiter row (`|:--|--:|`): its columns' alignments.
fn delimiter_row(line: &[char]) -> Option<Vec<Align>> {
    table_cells(line)
        .into_iter()
        .map(|(start, end)| {
            let cell = &line[start..end];
            let left = cell.first() == Some(&':');
            let right = cell.last() == Some(&':') && cell.len() > 1;
            let dashes = &cell[left as usize..cell.len() - right as usize];
            (!dashes.is_empty() && dashes.iter().all(|&c| c == '-')).then_some(
                match (left, right) {
                    (true, true) => Align::Center,
                    (false, true) => Align::Right,
                    _ => Align::Left,
                },
            )
        })
        .collect::<Option<Vec<_>>>()
        .filter(|aligns| !aligns.is_empty())
}

/// The table starting at document row `row`, whose text and the lines after
/// it are `lines`: its shown lines, and how many lines it took.
fn table(row: usize, lines: &[String]) -> Option<(Vec<ShownLine>, usize)> {
    let chars: Vec<Vec<char>> = lines
        .iter()
        .take_while(|l| !l.trim().is_empty() && l.contains('|'))
        .map(|l| l.chars().collect())
        .collect();
    let aligns = delimiter_row(chars.get(1)?)?;
    if !chars[0].contains(&'|') || table_cells(&chars[0]).len() != aligns.len() {
        return None;
    }
    // Each row's cells as shown: without the backticks of code spans or the
    // `\` of an escaped `|`.
    let cells: Vec<Vec<Vec<(char, Source)>>> = chars
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != 1)
        .map(|(_, line)| {
            let mut cells: Vec<Vec<(char, Source)>> = table_cells(line)
                .into_iter()
                .take(aligns.len())
                .map(|(start, end)| {
                    (start..end)
                        .filter(|&col| {
                            line[col] != '`'
                                && !(line[col] == '\\' && line.get(col + 1) == Some(&'|'))
                        })
                        .map(|col| (line[col], Source::Doc(col)))
                        .collect()
                })
                .collect();
            cells.resize(aligns.len(), Vec::new());
            cells
        })
        .collect();
    let cell_width = |cell: &[(char, Source)]| {
        cell.iter()
            .map(|(c, _)| c.width().unwrap_or(0))
            .sum::<usize>()
    };
    let widths: Vec<usize> = (0..aligns.len())
        .map(|j| cells.iter().map(|r| cell_width(&r[j])).max().unwrap_or(0))
        .collect();
    let border = |s: &str| {
        s.chars()
            .map(|c| (c, Source::Drawn("ui.popup.border")))
            .collect::<Vec<_>>()
    };
    let pad = |n: usize| std::iter::repeat_n((' ', Source::Drawn("ui.popup")), n);
    let mut shown = Vec::new();
    for (i, row_cells) in cells.iter().enumerate() {
        let mut line = Vec::new();
        for (j, cell) in row_cells.iter().enumerate() {
            if j > 0 {
                line.extend(pad(1));
                line.extend(border("│"));
                line.extend(pad(1));
            }
            let room = widths[j] - cell_width(cell);
            let before = match aligns[j] {
                Align::Left => 0,
                Align::Center => room / 2,
                Align::Right => room,
            };
            line.extend(pad(before));
            line.extend(cell.iter().copied());
            line.extend(pad(room - before));
        }
        // No padding at the end, to wrap nothing.
        while line.last() == Some(&(' ', Source::Drawn("ui.popup"))) {
            line.pop();
        }
        // The source row: the delimiter row is skipped.
        let source = row + if i == 0 { 0 } else { i + 1 };
        shown.push(ShownLine {
            header: i == 0,
            ..ShownLine::new(source, line)
        });
        if i == 0 {
            let rule: Vec<String> = widths.iter().map(|&w| "─".repeat(w)).collect();
            shown.push(ShownLine::new(row + 1, border(&rule.join("─┼─"))));
        }
    }
    Some((shown, chars.len()))
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

/// `chars` wrapped as `wrap` does, its rows after the first `indent` cells
/// narrower (to hang that far in): each row's char range and indent.
fn wrap_hanging(chars: &[char], width: usize, indent: usize) -> Vec<(usize, usize, usize)> {
    let indent = if indent < width { indent } else { 0 };
    let mut rows = Vec::new();
    let (start, end) = wrap(chars, width)[0];
    rows.push((start, end, 0));
    if end < chars.len() {
        for (s, e) in wrap(&chars[end..], width - indent) {
            rows.push((end + s, end + e, indent));
        }
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
        if self.scrolls && scroll + shown < rows.len() {
            block = block.title_bottom(Line::from(" K: more ").right_aligned());
        }
        let inner = block.inner(area);
        block.render(area, buf);

        for (i, row) in rows.iter().skip(scroll).take(shown).enumerate() {
            let y = inner.y + i as u16;
            let line = &self.lines[row.line];
            let mut x = inner.x + 1 + row.indent as u16;
            if line.rule {
                let rule = "─".repeat(width);
                buf.set_string(x, y, rule, base.patch(self.theme.get("ui.popup.border")));
                continue;
            }
            let header = if line.header {
                self.theme.get("text.strong")
            } else {
                Style::default()
            };
            for &(c, source) in &line.chars[row.start..row.end] {
                let w = c.width().unwrap_or(0) as u16;
                if w == 0 {
                    continue;
                }
                let style = match source {
                    Source::Doc(col) => header.patch(self.style_at(line.row, col, line.code)),
                    Source::Drawn(name) => self.theme.get(name),
                };
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
    fn list_bullets_are_drawn_and_numbers_kept() {
        let lines = shown("- one **b**\n  * nested\n+ plus\n1. first\n10) tenth\n-not a list\n");
        assert_eq!(
            lines,
            [
                "• one b",
                "  • nested",
                "• plus",
                "1. first",
                "10) tenth",
                "-not a list"
            ]
        );
    }

    #[test]
    fn a_list_item_wraps_under_its_text() {
        let chars: Vec<char> = "- one two three".chars().collect();
        assert_eq!(
            wrap_hanging(&chars, 8, 2),
            [(0, 6, 0), (6, 10, 2), (10, 15, 2)],
            "`- one ` then `two `, `three`, both 2 in"
        );
        assert_eq!(wrap_hanging(&chars, 2, 2)[1].2, 0, "no room to hang");
    }

    #[test]
    fn tables_are_laid_out_in_columns() {
        let lines = shown(
            "Before\n\n| Name | `Type` | n |\n|:-----|:---:|--:|\n| x | a\\|b | 10 |\n| longer |\n\nAfter\n",
        );
        assert_eq!(
            lines,
            [
                "Before",
                "",
                "Name   │ Type │  n",
                "───────┼──────┼───",
                "x      │ a|b  │ 10",
                "longer │      │",
                "",
                "After",
            ],
            "padded, aligned, code and escapes shown plain, short rows filled"
        );
    }

    #[test]
    fn pipes_without_a_delimiter_row_are_text() {
        assert_eq!(shown("a | b\nc | d\n"), ["a | b", "c | d"]);
        assert_eq!(
            table_cells(&"| a | `x|y` |".chars().collect::<Vec<_>>()),
            [(2, 3), (6, 11)],
            "a `|` in a code span doesn't split"
        );
    }

    #[test]
    fn documentation_goes_right_of_the_menu_else_left() {
        let mut editor = crate::editor::Editor::default();
        editor.show_hover("Some documentation that is fairly long, to wrap.");
        let hover = editor.hover().unwrap();
        let theme = Theme::default();
        let view = HoverView::new(hover, None, &theme);
        let screen = Rect::new(0, 0, 80, 20);
        let right = view.beside(Rect::new(10, 15, 20, 5), screen).unwrap();
        assert_eq!(
            (right.x, right.width),
            (30, 50),
            "beside, as wide as there's room"
        );
        assert_eq!(
            (right.y, right.height),
            (15, 4),
            "level with the menu, wrapped"
        );
        let low = view.beside(Rect::new(10, 17, 20, 3), screen).unwrap();
        assert_eq!(low.y + low.height, 20, "moved up to fit");
        let left = view.beside(Rect::new(55, 2, 20, 5), screen).unwrap();
        assert_eq!(left.x + left.width, 55, "no room on the right");
        assert!(left.width < 55);
        assert_eq!(
            view.beside(Rect::new(20, 2, 40, 5), screen),
            None,
            "nor either side"
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
