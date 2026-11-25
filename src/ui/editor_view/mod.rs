//! The editor pane: a custom ratatui widget that renders a document with a
//! line-number gutter, cursorline and selection highlights, and a scrolling
//! viewport.
//!
//! Ratatui has no built-in widget suited to editing, so this is our own.
//! Each row is drawn as text, then highlights are layered on as styles;
//! syntax highlighting and diagnostics will layer on the same way.

mod gutter;
mod line;
mod selection;
mod viewport;

pub use viewport::{ScrollTarget, Viewport};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Position, Rect},
    style::Style,
    widgets::StatefulWidget,
};

use crate::config::{EditorOptions, LineNumbers, Theme};
use crate::core::search::{Match, SearchPattern};
use crate::core::{Position as TextPosition, Range};
use crate::editor::Document;
use crate::editor::windows::{Margins, TextLayout};
use crate::syntax::{HIGHLIGHT_NAMES, Highlights};

/// State kept between frames for an editor pane.
#[derive(Debug, Default, Clone)]
pub struct ViewState {
    pub viewport: Viewport,
    /// Where the terminal cursor should be drawn, set during rendering.
    pub cursor: Option<Position>,
    /// What each screen row showed when last drawn, for the mouse.
    pub rows: Vec<DrawnRow>,
    /// Where the text starts, after the gutter.
    pub text_x: u16,
    /// The fold column's x, when it's shown.
    pub fold_x: Option<u16>,
}

/// One screen row as drawn: which part of which buffer line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawnRow {
    pub y: u16,
    pub row: usize,
    /// Chars `start..end` of the line.
    pub start: usize,
    pub end: usize,
    /// The display column drawn first (a wrapped part's start, or the
    /// sideways scroll), and how far in it's drawn (after a wrap marker).
    pub left: usize,
    pub indent: usize,
    /// The line's last part (a click past its end goes to the end).
    pub last: bool,
    /// What the fold column showed beside it.
    pub fold: gutter::FoldMark,
}

impl ViewState {
    /// The fold markers a click can open or close, as last drawn: each
    /// `▸` and `▾` cell and its line.
    pub fn fold_markers(&self) -> impl Iterator<Item = (Position, usize)> + '_ {
        let x = self.fold_x;
        self.rows.iter().filter_map(move |r| {
            let clickable = matches!(r.fold, gutter::FoldMark::Closed | gutter::FoldMark::Open);
            Some((Position::new(x?, r.y), r.row)).filter(|_| clickable)
        })
    }

    /// The buffer position shown at screen cell `(x, y)` when last drawn:
    /// in the gutter, the start of that row; past the end of a line, its
    /// end; below the text, the last line. `None` before it's drawn.
    pub fn position_at(
        &self,
        document: &Document,
        x: u16,
        y: u16,
        tab_width: usize,
    ) -> Option<TextPosition> {
        let drawn = self
            .rows
            .iter()
            .find(|r| r.y == y)
            .or_else(|| self.rows.iter().rev().find(|r| r.y < y))
            .or(self.rows.first())?;
        let below = y > drawn.y;
        // Rows are drawn from the line with its virtual text, if any.
        let display = document.display_line(drawn.row);
        let real = |col: usize| display.as_ref().map_or(col, |d| d.to_real(col));
        if below {
            return Some(TextPosition::new(drawn.row, real(drawn.end)));
        }
        let line = match &display {
            Some(d) => std::borrow::Cow::Borrowed(d.text.as_str()),
            None => document.buffer().line(drawn.row),
        };
        let into = (x.saturating_sub(self.text_x) as usize).saturating_sub(drawn.indent);
        let col = crate::core::columns::char_col_at(&line, drawn.left + into, tab_width);
        let last_col = if drawn.last {
            drawn.end
        } else {
            drawn.end.saturating_sub(1).max(drawn.start)
        };
        Some(TextPosition::new(
            drawn.row,
            real(col.clamp(drawn.start, last_col)),
        ))
    }

    /// The char drawn at screen cell `(x, y)`, if there's one there (not
    /// the gutter, past a line's end or below the text): for the hover.
    pub fn char_at(
        &self,
        document: &Document,
        x: u16,
        y: u16,
        tab_width: usize,
    ) -> Option<TextPosition> {
        let drawn = self.rows.iter().find(|r| r.y == y)?;
        let start = self.text_x as usize + drawn.indent;
        if (x as usize) < start {
            return None;
        }
        let display = document.display_line(drawn.row);
        let line = match &display {
            Some(d) => std::borrow::Cow::Borrowed(d.text.as_str()),
            None => document.buffer().line(drawn.row),
        };
        let col =
            crate::core::columns::char_col_at(&line, drawn.left + (x as usize - start), tab_width);
        if col < drawn.start || col >= drawn.end || col >= line.chars().count() {
            return None;
        }
        let real = display.as_ref().map_or(col, |d| d.to_real(col));
        Some(TextPosition::new(drawn.row, real))
    }

    /// The rows the pane will show when it's next drawn `height` rows tall
    /// with the cursor at `cursor`: where it is, scrolled as drawing will
    /// scroll it.
    pub fn visible_rows(
        &self,
        document: &Document,
        cursor: TextPosition,
        area: Rect,
        options: &EditorOptions,
        covered: usize,
    ) -> std::ops::Range<usize> {
        let height = area.height as usize;
        let layout = TextLayout::new(area.width as usize, document, options);
        let margins = Margins {
            scroll_off: options.scroll_off,
            covered,
        };
        let top = layout.follow_cursor(self.viewport.top, document, cursor, height, margins);
        // Every line up to the last shown (closed folds and wrapped lines
        // change how many that is).
        top.line..layout.bottom(document, top, height) + 1
    }
}

/// How far above the top a search that spans lines looks for matches
/// running into view.
const SPANNING_LOOKBACK: usize = 200;

pub struct EditorView<'a> {
    document: &'a Document,
    options: &'a EditorOptions,
    theme: &'a Theme,
    selection: Option<Range>,
    /// More cursors (`multi_cursor`), with their selections.
    extra_cursors: &'a [crate::editor::multi_cursor::ExtraCursor],
    block: Option<crate::core::Block>,
    search: Option<&'a SearchPattern>,
    /// With a `search` that spans lines, its matches around the rows
    /// drawn (found once per draw, not per row).
    search_matches: Option<Vec<Match>>,
    /// The find box's "in selection": tinted, and the only matches shown.
    find_scope: Option<&'a crate::editor::find::FindScope>,
    current_match: Option<Match>,
    /// Git changes, for marks in the gutter.
    git: &'a [crate::git::Hunk],
    /// Merge conflicts, tinted.
    conflicts: &'a [crate::git::Conflict],
    /// The cursor's a caret between chars (insert mode): beside virtual
    /// text it goes before it, where typing goes.
    caret: bool,
    /// The other uses of the symbol under the cursor (a language server's).
    references: &'a [(crate::core::Position, crate::core::Position)],
    syntax: Option<&'a Highlights>,
    /// A language server's diagnostics for the document, in order.
    diagnostics: &'a [crate::editor::diagnostics::Diagnostic],
    /// The cursor to draw around; a window's own cursor when it isn't active.
    cursor: Option<TextPosition>,
    /// Only the active window shows the cursor line.
    active: bool,
    /// Rows at the top drawn over (by the find box), kept clear of the
    /// cursor where the buffer allows.
    covered: usize,
    /// Bracket pair colours: each line's starting state, and the
    /// language's comments.
    brackets: Option<(
        std::rc::Rc<Vec<crate::core::brackets::LineState>>,
        crate::core::brackets::Comments,
    )>,
    /// Indent guides: a level's width in columns.
    indent_guides: Option<usize>,
    /// The guide of the block the cursor's in: its column, and the rows.
    active_guide: Option<(usize, usize, usize)>,
    /// The bracket at the cursor and its pair, highlighted.
    bracket_pair: Option<(TextPosition, TextPosition)>,
}

impl<'a> EditorView<'a> {
    pub fn new(document: &'a Document, options: &'a EditorOptions, theme: &'a Theme) -> Self {
        Self {
            document,
            options,
            theme,
            selection: None,
            extra_cursors: &[],
            block: None,
            search: None,
            search_matches: None,
            find_scope: None,
            current_match: None,
            git: &[],
            conflicts: &[],
            caret: false,
            references: &[],
            syntax: None,
            diagnostics: &[],
            cursor: None,
            active: true,
            covered: 0,
            brackets: None,
            indent_guides: None,
            active_guide: None,
            bracket_pair: None,
        }
    }

    /// Colours brackets by depth (`bracket-colors`), from `states`.
    pub fn brackets(
        mut self,
        states: std::rc::Rc<Vec<crate::core::brackets::LineState>>,
        comments: crate::core::brackets::Comments,
    ) -> Self {
        self.brackets = Some((states, comments));
        self
    }

    /// Draws indent guides every `width` columns (`indent-guides`).
    pub fn indent_guides(mut self, width: usize) -> Self {
        self.indent_guides = Some(width.max(1));
        self
    }

    /// The indentation of `row` in columns; for a blank line, the less of
    /// the lines above and below it that aren't (so guides carry through).
    fn indent_of(&self, row: usize) -> usize {
        let text = self.document.buffer();
        let tab = self.options.tab_width.max(1);
        let measure = |row: usize| -> Option<usize> {
            let line = text.line(row);
            if line.trim().is_empty() {
                return None;
            }
            let mut width = 0;
            for c in line.chars() {
                match c {
                    ' ' => width += 1,
                    '\t' => width += tab - width % tab,
                    _ => break,
                }
            }
            Some(width)
        };
        if let Some(width) = measure(row) {
            return width;
        }
        let up = (row.saturating_sub(100)..row)
            .rev()
            .find_map(measure)
            .unwrap_or(0);
        let down = (row + 1..(row + 100).min(text.line_count()))
            .find_map(measure)
            .unwrap_or(0);
        up.min(down)
    }

    /// The guide of the block the cursor's in: its column, and the rows
    /// from and to.
    fn find_active_guide(
        &self,
        cursor: TextPosition,
        unit: usize,
    ) -> Option<(usize, usize, usize)> {
        let indent = self.indent_of(cursor.row);
        if indent < unit {
            return None;
        }
        let level = indent - (indent - unit) % unit - unit;
        let rows = self.document.buffer().line_count();
        let inside = |row: usize| self.indent_of(row) > level;
        let mut first = cursor.row;
        while first > 0 && inside(first - 1) && cursor.row - first < 2000 {
            first -= 1;
        }
        let mut last = cursor.row;
        while last + 1 < rows && inside(last + 1) && last - cursor.row < 2000 {
            last += 1;
        }
        Some((level, first, last))
    }

    /// The bracket at (or, typing, just before) `cursor`, and its pair.
    fn find_bracket_pair(&self, cursor: TextPosition) -> Option<(TextPosition, TextPosition)> {
        let text = self.document.buffer();
        let line: Vec<char> = text.line(cursor.row).chars().collect();
        let is_bracket = |c: Option<&char>| c.is_some_and(|c| "()[]{}".contains(*c));
        let at = if is_bracket(line.get(cursor.col)) {
            cursor
        } else if cursor.col > 0 && is_bracket(line.get(cursor.col - 1)) {
            cursor.with_col(cursor.col - 1)
        } else {
            return None;
        };
        // With the brackets' depths (strings and comments skipped), the
        // pair is the next closer, or the last opener, at the same depth.
        if let Some((states, comments)) = &self.brackets {
            let brackets_on = |row: usize| {
                let mut state = states.get(row).copied().unwrap_or_default();
                crate::core::brackets::scan(&text.line(row), &mut state, comments)
            };
            let here = brackets_on(at.row).into_iter().find(|b| b.col == at.col)?;
            let found = if here.open {
                (at.row..text.line_count().min(at.row + 5000)).find_map(|row| {
                    brackets_on(row)
                        .into_iter()
                        .find(|b| {
                            (row > at.row || b.col > at.col) && !b.open && b.depth == here.depth
                        })
                        .map(|b| TextPosition::new(row, b.col))
                })
            } else {
                (at.row.saturating_sub(5000)..=at.row)
                    .rev()
                    .find_map(|row| {
                        brackets_on(row)
                            .into_iter()
                            .rev()
                            .find(|b| {
                                (row < at.row || b.col < at.col) && b.open && b.depth == here.depth
                            })
                            .map(|b| TextPosition::new(row, b.col))
                    })
            };
            return found.map(|other| (at, other));
        }
        let other = crate::core::motion::matching_bracket(text, at)?;
        Some((at, other))
    }

    /// Colours the text with syntax highlights, if any.
    /// Draws around `cursor` instead of the document's own cursor.
    pub fn cursor(mut self, cursor: TextPosition) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Whether this is the active window (inactive ones have no cursor line).
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Scrolls to keep the cursor below the top `rows`, which something is
    /// drawn over (the find box, so it doesn't hide the current match).
    pub fn covered(mut self, rows: usize) -> Self {
        self.covered = rows;
        self
    }

    fn margins(&self) -> Margins {
        Margins {
            scroll_off: self.options.scroll_off,
            covered: self.covered,
        }
    }

    /// Underlines `diagnostics` and shows their messages.
    pub fn diagnostics(
        mut self,
        diagnostics: &'a [crate::editor::diagnostics::Diagnostic],
    ) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn syntax(mut self, syntax: Option<&'a Highlights>) -> Self {
        self.syntax = syntax;
        self
    }

    /// Whether the cursor's a caret between chars (insert mode).
    pub fn caret(mut self, caret: bool) -> Self {
        self.caret = caret;
        self
    }

    /// Marks git's changes in the gutter.
    pub fn git_changes(mut self, hunks: &'a [crate::git::Hunk]) -> Self {
        self.git = hunks;
        self
    }

    /// Tints merge conflicts: each side's rows and the markers.
    pub fn conflicts(mut self, conflicts: &'a [crate::git::Conflict]) -> Self {
        self.conflicts = conflicts;
        self
    }

    /// The theme scope a row of a conflict is tinted with, if it's in one.
    fn conflict_scope(&self, row: usize) -> Option<&'static str> {
        let c = self.conflicts.iter().find(|c| c.contains(row))?;
        Some(
            if row == c.start || row == c.middle || row == c.end || c.base == Some(row) {
                "ui.conflict.marker"
            } else if c.ours().contains(&row) {
                "ui.conflict.ours"
            } else if c.theirs().contains(&row) {
                "ui.conflict.theirs"
            } else {
                "ui.conflict.base"
            },
        )
    }

    /// Highlights `found` apart from the other matches (the find box's).
    pub fn current_match(mut self, found: Option<Match>) -> Self {
        self.current_match = found;
        self
    }

    /// Tints the selection the find box finds in, and highlights only the
    /// matches inside it.
    pub fn find_scope(mut self, scope: Option<&'a crate::editor::find::FindScope>) -> Self {
        self.find_scope = scope;
        self
    }

    /// Highlights matches of `search` (the active or last search, if any).
    pub fn search(mut self, search: Option<&'a SearchPattern>) -> Self {
        self.search = search;
        self
    }

    /// Highlights the uses of the symbol under the cursor.
    pub fn references(
        mut self,
        ranges: &'a [(crate::core::Position, crate::core::Position)],
    ) -> Self {
        self.references = ranges;
        self
    }

    /// Highlights a visual block selection (drawn instead of `selection`).
    pub fn block(mut self, block: Option<crate::core::Block>) -> Self {
        self.block = block;
        self
    }

    /// Draws the extra cursors (and their selections).
    pub fn extra_cursors(
        mut self,
        cursors: &'a [crate::editor::multi_cursor::ExtraCursor],
    ) -> Self {
        self.extra_cursors = cursors;
        self
    }

    /// Highlights `selection` (the visual-mode selection, if any).
    pub fn selection(mut self, selection: Option<Range>) -> Self {
        self.selection = selection;
        self
    }
}

impl StatefulWidget for EditorView<'_> {
    type State = ViewState;

    fn render(mut self, area: Rect, buf: &mut Buffer, state: &mut ViewState) {
        state.cursor = None;
        state.rows.clear();
        state.fold_x = None;
        if area.is_empty() {
            return;
        }

        let cursor = self.cursor.unwrap_or_else(|| self.document.cursor());
        if self.active && self.options.match_brackets {
            self.bracket_pair = self.find_bracket_pair(cursor);
        }
        if let Some(unit) = self.indent_guides {
            self.active_guide = self.find_active_guide(cursor, unit);
        }
        let text = self.document.buffer();
        let tab_width = self.options.tab_width;
        let line_numbers = self.options.line_numbers;

        let fold_width = gutter::fold_width(self.options).min(area.width);
        let numbers_width =
            gutter::width(text.line_count(), line_numbers).min(area.width - fold_width);
        let [fold_area, gutter_area, text_area] = Layout::horizontal([
            Constraint::Length(fold_width),
            Constraint::Length(numbers_width),
            Constraint::Min(0),
        ])
        .areas(area);

        let height = text_area.height as usize;
        let width = text_area.width as usize;
        let layout = TextLayout::for_text(width, self.options);
        if layout.wrap {
            state.viewport.left = 0;
            state.viewport.top = layout.follow_cursor(
                state.viewport.top,
                self.document,
                cursor,
                height,
                self.margins(),
            );
        } else {
            let cursor_col = match self.document.display_line(cursor.row) {
                Some(display) => {
                    line::display_col(&display.text, display.to_display(cursor.col), tab_width)
                }
                None => line::display_col(&text.line(cursor.row), cursor.col, tab_width),
            };
            let target = ScrollTarget {
                row: cursor.row,
                col: cursor_col,
                line_count: text.line_count(),
            };
            state
                .viewport
                .scroll_to(target, width, height, self.margins());
        }
        let Viewport { top, left } = state.viewport;
        if let Some(pattern) = self.search.filter(|p| p.spans_lines()) {
            // From a little above the top, for matches that start above it
            // and run into view (longer ones aren't shown).
            let first = top.line.saturating_sub(SPANNING_LOOKBACK);
            let last = layout.bottom(self.document, top, height);
            let mut matches = pattern.matches_between(text, first..last + 1);
            matches.retain(|m| !m.is_empty() && m.end.row >= top.line);
            self.search_matches = Some(matches);
        }
        let mut fold_column =
            (fold_width > 0).then(|| crate::core::fold::Column::new(text, top.line, tab_width));
        state.fold_x = fold_column.as_ref().map(|_| fold_area.x);
        state.text_x = text_area.x;

        let text_style = self.theme.get("ui.text");
        let syntax_styles: Vec<Style> = match self.syntax {
            Some(_) => HIGHLIGHT_NAMES
                .iter()
                .map(|name| self.theme.get(name))
                .collect(),
            None => Vec::new(),
        };

        let mut y = 0;
        let mut row = top.line;
        // Rows of the first line scrolled off the top (it's taller than
        // the view).
        let mut skip = top.skip;
        while y < height && row < text.line_count() {
            let real_line = text.line(row);
            // With virtual text, the line as it's drawn (its columns mapped).
            let display = self.document.display_line(row);
            let line_text: &str = display.as_ref().map_or(&*real_line, |d| d.text.as_str());
            let line_len = line_text.chars().count();
            let cursor_col = match &display {
                Some(d) if self.caret => d.caret_at(cursor.col),
                Some(d) => d.to_display(cursor.col),
                None => cursor.col,
            };
            // A closed fold shows as its first line, on one row.
            let fold = self
                .document
                .closed_fold_at(row)
                .filter(|&(first, _)| first == row);
            let starts = if fold.is_some() {
                vec![0]
            } else {
                layout.row_starts(line_text)
            };
            let wrap = layout.wrap && fold.is_none();
            let fold_mark = fold_column.as_mut().map(|column| match fold {
                Some(_) => gutter::FoldMark::Closed,
                None => column.mark(row).into(),
            });
            let skipped = std::mem::take(&mut skip);
            for (part, &start) in starts.iter().enumerate().skip(skipped) {
                if y >= height {
                    break;
                }
                let end = starts.get(part + 1).copied().unwrap_or(line_len);
                let last_part = part + 1 == starts.len();
                let screen_y = area.y + y as u16;
                self.draw_row(
                    buf,
                    area,
                    gutter_area,
                    text_area,
                    screen_y,
                    RowPart {
                        row,
                        line: line_text,
                        real_line: &real_line,
                        display: display.as_ref(),
                        start,
                        end,
                        first: part == 0,
                        last: last_part,
                        indent: if wrap {
                            layout.offset(line_text, part)
                        } else {
                            0
                        },
                        left: if wrap {
                            line::display_col(line_text, start, tab_width)
                        } else if layout.wrap {
                            0
                        } else {
                            left
                        },
                    },
                    cursor,
                    text_style,
                    &syntax_styles,
                );
                // A wrapped line's later rows are inside what it heads.
                let drawn_mark =
                    fold_mark.map(|mark| if part == 0 { mark } else { mark.continued() });
                if let Some(mark) = drawn_mark {
                    let style = buf[(area.x, screen_y)]
                        .style()
                        .patch(self.theme.get("ui.fold.column"));
                    let cell = Rect::new(fold_area.x, screen_y, fold_area.width, 1);
                    gutter::render_fold(buf, cell, mark, style);
                }
                if let Some((first, last)) = fold {
                    self.draw_fold_note(buf, text_area, screen_y, line_text, last - first, left);
                }
                state.rows.push(DrawnRow {
                    y: screen_y,
                    row,
                    start,
                    end,
                    left: if wrap {
                        line::display_col(line_text, start, tab_width)
                    } else if layout.wrap {
                        0
                    } else {
                        left
                    },
                    indent: if wrap {
                        layout.offset(line_text, part)
                    } else {
                        0
                    },
                    last: last_part,
                    fold: drawn_mark.unwrap_or_default(),
                });
                // The extra cursors on this part of the line: like the
                // real one, a bar where there's no character (past the
                // line's end, or a space), else the character underlined.
                for extra in self.extra_cursors.iter().filter(|c| c.cursor.row == row) {
                    let col = match &display {
                        Some(d) => d.caret_at(extra.cursor.col),
                        None => extra.cursor.col,
                    };
                    if wrap && crate::core::wrap::row_of(&starts, col) != part {
                        continue;
                    }
                    let seg_left = if wrap {
                        line::display_col(line_text, start, tab_width)
                    } else if layout.wrap {
                        0
                    } else {
                        left
                    };
                    let indent = if wrap {
                        layout.offset(line_text, part)
                    } else {
                        0
                    };
                    let Some(x) = (line::display_col(line_text, col, tab_width) + indent)
                        .checked_sub(seg_left)
                        .filter(|&x| x < width)
                    else {
                        continue;
                    };
                    let color = self
                        .theme
                        .get("ui.cursor.secondary")
                        .fg
                        .or(self.theme.get("ui.text").fg)
                        .unwrap_or(ratatui::style::Color::Reset);
                    let cell = &mut buf[(text_area.x + x as u16, screen_y)];
                    // Past the end, or on a space: the bar hides nothing.
                    if col >= line_len || cell.symbol() == " " {
                        cell.set_symbol("▏").set_fg(color);
                    } else {
                        cell.set_style(
                            Style::new()
                                .underline_color(color)
                                .add_modifier(ratatui::style::Modifier::UNDERLINED),
                        );
                    }
                }
                let cursor_here = row == cursor.row
                    && (crate::core::wrap::row_of(&starts, cursor_col) == part || !wrap);
                if cursor_here && !text_area.is_empty() {
                    let seg_left = if wrap {
                        line::display_col(line_text, start, tab_width)
                    } else if layout.wrap {
                        0
                    } else {
                        left
                    };
                    let indent = if wrap {
                        layout.offset(line_text, part)
                    } else {
                        0
                    };
                    let x = line::display_col(line_text, cursor_col, tab_width) - seg_left + indent;
                    state.cursor = Some(Position::new(
                        text_area.x + x.min(width.saturating_sub(1)) as u16,
                        screen_y,
                    ));
                }
                y += 1;
            }
            row = fold.map_or(row, |(_, last)| last) + 1;
        }
        // Part of the first line is scrolled off: say so, as Vim does.
        if top.skip > 0 && height > 0 {
            let marker = "<<<";
            buf.set_stringn(
                text_area.x,
                text_area.y,
                marker,
                width.min(marker.len()),
                self.theme.get("ui.wrap-marker"),
            );
        }
    }
}

/// The gutter mark for `row` (`first`: its first screen row): a bar beside
/// added and changed lines, and a low bar on the line above removed ones
/// (a high one on the first line, for lines removed from the top).
fn git_mark(
    hunks: &[crate::git::Hunk],
    row: usize,
    first: bool,
) -> Option<(&'static str, &'static str)> {
    use crate::git::ChangeKind;
    // Hunks are in order: the last starting at or before the row after.
    let end = hunks.partition_point(|h| h.start <= row + 1);
    hunks[..end]
        .iter()
        .rev()
        .take(2)
        .find_map(|hunk| match hunk.kind {
            ChangeKind::Added if (hunk.start..hunk.end).contains(&row) => {
                Some(("▎", "ui.gutter.added"))
            }
            ChangeKind::Changed if (hunk.start..hunk.end).contains(&row) => {
                Some(("▎", "ui.gutter.changed"))
            }
            ChangeKind::Removed if first && hunk.start == 0 && row == 0 => {
                Some(("▔", "ui.gutter.removed"))
            }
            ChangeKind::Removed if first && hunk.start == row + 1 => {
                Some(("▁", "ui.gutter.removed"))
            }
            _ => None,
        })
}

/// One screen row's part of a buffer line: chars `start..end` of `line`,
/// drawn from display column `left`.
struct RowPart<'l> {
    row: usize,
    line: &'l str,
    /// The line as it is in the buffer, and how its columns map to
    /// `line` (which has its virtual text, if it has any).
    real_line: &'l str,
    display: Option<&'l crate::editor::DisplayLine>,
    start: usize,
    end: usize,
    /// The line's first row (where its number goes).
    first: bool,
    /// The line's last row (where a selected line break is drawn).
    last: bool,
    left: usize,
    /// Cells the text starts in by (a wrapped line's continued rows).
    indent: usize,
}

impl EditorView<'_> {
    /// After a closed fold's first line: `⋯ N more lines`, dimmed, if it
    /// fits.
    fn draw_fold_note(
        &self,
        buf: &mut Buffer,
        text_area: Rect,
        y: u16,
        line_text: &str,
        hidden: usize,
        left: usize,
    ) {
        let width = text_area.width as usize;
        let end = crate::core::columns::display_width(line_text, self.options.tab_width)
            .saturating_sub(left);
        let s = if hidden == 1 { "" } else { "s" };
        let note = format!("⋯ {hidden} more line{s}");
        let x = end + 1;
        if x < width {
            buf.set_stringn(
                text_area.x + x as u16,
                y,
                &note,
                width - x,
                self.theme.get("ui.fold"),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        buf: &mut Buffer,
        area: Rect,
        gutter_area: Rect,
        text_area: Rect,
        y: u16,
        part: RowPart,
        cursor: TextPosition,
        text_style: Style,
        syntax_styles: &[Style],
    ) {
        let tab_width = self.options.tab_width;
        let RowPart {
            row,
            line: line_text,
            real_line,
            display,
            start,
            end,
            first,
            last,
            left,
            indent,
        } = part;
        // Continued rows start further in, after the wrap marker.
        let text_area = Rect::new(
            text_area.x + indent.min(text_area.width as usize) as u16,
            text_area.y,
            text_area.width.saturating_sub(indent as u16),
            text_area.height,
        );
        let width = text_area.width as usize;
        let is_cursor_row = row == cursor.row;

        let mut row_style = if is_cursor_row && self.active && self.options.cursorline {
            self.theme.get("ui.cursorline")
        } else {
            Style::default()
        };
        if let Some(scope) = self.conflict_scope(row) {
            row_style = row_style.patch(self.theme.get(scope));
        }
        buf.set_style(Rect::new(area.x, y, area.width, 1), row_style);

        let gutter_scope = if is_cursor_row {
            "ui.linenr.selected"
        } else {
            "ui.linenr"
        };
        let label = if first {
            let distance = match self.options.line_numbers {
                LineNumbers::Relative => self.document.visible_lines_between(row, cursor.row),
                _ => 0,
            };
            gutter::label(row, distance, self.options.line_numbers)
        } else {
            String::new()
        };
        // The most serious diagnostic starting on the line colours its
        // number and shows its message (once, where it starts, as one
        // that covers the whole file would otherwise be on every line).
        let on_line = self
            .diagnostics
            .iter()
            .filter(|d| d.start.row == row)
            .min_by_key(|d| d.severity);
        let gutter_style = match on_line {
            Some(d) => self.theme.get(gutter_scope).patch(
                self.theme
                    .get(&format!("ui.diagnostic.{}", d.severity.id())),
            ),
            None => self.theme.get(gutter_scope),
        };
        gutter::render(
            buf,
            Rect::new(gutter_area.x, y, gutter_area.width, 1),
            &label,
            gutter_style,
        );
        // A git change's mark, in the padding after the number.
        if gutter_area.width >= 2
            && let Some((mark, scope)) = git_mark(self.git, row, first)
        {
            let x = gutter_area.right() - 1;
            buf[(x, y)]
                .set_symbol(mark)
                .set_style(row_style.patch(self.theme.get(scope)));
        }

        if indent > 0 && !self.options.wrap_marker.is_empty() {
            let marker = &self.options.wrap_marker;
            let marker_width = unicode_width::UnicodeWidthStr::width(marker.as_str()) as u16;
            let x = text_area.x.saturating_sub(marker_width);
            buf.set_stringn(
                x,
                y,
                marker,
                marker_width as usize,
                self.theme.get("ui.wrap-marker"),
            );
        }
        // Only this part's chars: the rest of a wrapped line is on later rows.
        let upto = line_text
            .char_indices()
            .nth(end)
            .map_or(line_text.len(), |(i, _)| i);
        let visible = line::visible_slice(&line_text[..upto], left, width, tab_width);
        buf.set_stringn(text_area.x, y, visible, width, text_style);

        // Indent guides, in the indentation's blank cells (on a line's
        // first row; through blank lines inside a block).
        if let Some(unit) = self.indent_guides.filter(|_| first) {
            let indent = self.indent_of(row);
            let mut col = 0;
            while col < indent {
                if col >= left && col - left < width {
                    let x = text_area.x + (col - left) as u16;
                    let cell = &mut buf[(x, y)];
                    if cell.symbol() == " " {
                        let active = self
                            .active_guide
                            .is_some_and(|(at, from, to)| at == col && (from..=to).contains(&row));
                        let scope = if active {
                            "ui.indent-guide.active"
                        } else {
                            "ui.indent-guide"
                        };
                        cell.set_symbol("│").set_style(self.theme.get(scope));
                    }
                }
                col += unit;
            }
        }

        // Highlights come in the buffer's columns.
        let line_len = real_line.chars().count();
        let mut paint = |from: usize, to: usize, to_end: bool, style: Style| {
            // Columns of the drawn line to display cells, within this part
            // of the line; a column past the last char (a selected line
            // break) is drawn as one extra cell on the line's last row.
            let from = from.max(start);
            let to = to.min(end);
            if from >= to && !to_end {
                return;
            }
            let from = line::display_col(line_text, from, tab_width);
            let to = line::display_col(line_text, to, tab_width) + usize::from(to_end);
            let begin = from.max(left) - left;
            let finish = to.saturating_sub(left).min(width);
            if begin < finish {
                let cells = Rect::new(text_area.x + begin as u16, y, (finish - begin) as u16, 1);
                buf.set_style(cells, style);
            }
        };
        let mut highlight = |from: usize, to: usize, style: Style| {
            let to_end = to > line_len && last;
            match display {
                // A range starts at its first char, after any virtual text
                // before it, and ends before any virtual text at its end.
                Some(d) => paint(
                    d.to_display(from),
                    d.caret_at(to.min(line_len)),
                    to_end,
                    style,
                ),
                None => paint(from, to, to_end, style),
            }
        };

        if let Some(syntax) = self.syntax {
            for span in syntax.line(row) {
                highlight(span.start, span.end, syntax_styles[span.highlight]);
            }
        }
        // Brackets by depth, over the syntax's colours (not in strings or
        // comments, as the highlighting says where they are).
        if let Some((states, comments)) = &self.brackets {
            let mut state = states.get(row).copied().unwrap_or_default();
            let in_text = |col: usize| {
                self.syntax.is_some_and(|syntax| {
                    syntax.line(row).iter().any(|span| {
                        span.start <= col
                            && col < span.end
                            && (span.name().starts_with("string")
                                || span.name().starts_with("comment"))
                    })
                })
            };
            for bracket in crate::core::brackets::scan(real_line, &mut state, comments) {
                if in_text(bracket.col) {
                    continue;
                }
                let scope =
                    ["ui.bracket.1", "ui.bracket.2", "ui.bracket.3"][bracket.depth as usize % 3];
                highlight(bracket.col, bracket.col + 1, self.theme.get(scope));
            }
        }
        if let Some((a, b)) = self.bracket_pair {
            let style = self.theme.get("ui.bracket.match");
            for at in [a, b] {
                if at.row == row {
                    highlight(at.col, at.col + 1, style);
                }
            }
        }
        // Plugins' highlighted ranges (decorations).
        for h in self.document.highlights_on(row) {
            let from = if h.start.row == row { h.start.col } else { 0 };
            let to = if h.end.row == row {
                h.end.col
            } else {
                line_len
            };
            if from < to {
                highlight(from, to, self.theme.get(&h.style));
            }
        }
        let reference = self.theme.get("ui.highlight.reference");
        for (start, end) in self.references {
            if start.row <= row && row <= end.row {
                let from = if start.row == row { start.col } else { 0 };
                let to = if end.row == row { end.col } else { line_len };
                if from < to {
                    highlight(from, to, reference);
                }
            }
        }
        let span = self.find_scope.and_then(|scope| scope.span(row));
        if let Some((from, to)) = span {
            // Running to the line's end, the line break too (as a
            // selection's is), so empty lines in it show.
            let to = to.min(line_len + 1);
            if from < to {
                highlight(from, to, self.theme.get("ui.find.scope"));
            }
        }
        let style = self.theme.get("ui.search.match");
        let real_len = real_line.chars().count();
        if let Some(matches) = &self.search_matches {
            for found in matches {
                let inside = self.find_scope.is_none_or(|scope| scope.contains(found));
                if let Some((from, to)) = found.cols_on(row, real_len).filter(|_| inside) {
                    highlight(from, to, style);
                }
            }
        } else if let Some(pattern) = self.search {
            for (from, to) in pattern.matches_in_line(real_line) {
                let inside = span.is_some_and(|(s, e)| s <= from && to <= e);
                if self.find_scope.is_none() || inside {
                    highlight(from, to, style);
                }
            }
        }
        if let Some((from, to)) = self.current_match.and_then(|m| m.cols_on(row, real_len)) {
            highlight(from, to, self.theme.get("ui.search.current"));
        }
        for d in self
            .diagnostics
            .iter()
            .filter(|d| d.start.row <= row && row <= d.end.row)
        {
            let from = if d.start.row == row { d.start.col } else { 0 };
            let to = if d.end.row == row {
                d.end.col
            } else {
                line_len
            };
            // An empty range still marks the char it's at; the empty lines
            // inside a longer one aren't marked.
            let to = if d.start == d.end { from + 1 } else { to };
            if to <= from {
                continue;
            }
            highlight(
                from,
                to,
                self.theme.get(&format!("diagnostic.{}", d.severity.id())),
            );
        }
        // The selection is drawn last, so it wins over search matches.
        if let Some(block) = self.block {
            if block.rows().contains(&row) {
                // In columns as drawn, so the block lines up on screen
                // over virtual text (inlay hints).
                let cells = match display {
                    Some(d) => d.cells(tab_width),
                    None => crate::core::columns::LineCells::new(real_line, tab_width),
                };
                let (from, to) = block.chars_in(&cells);
                if from < to {
                    highlight(from, to, self.theme.get("ui.selection"));
                }
            }
        } else if let Some(range) = self.selection
            && let Some((from, to)) = selection::selected_cols(range, row, line_len)
        {
            highlight(from, to, self.theme.get("ui.selection"));
        }
        // The extra cursors' selections (the cursors are drawn once the
        // row's done: see `draw_extra_cursors`).
        for extra in self.extra_cursors {
            if let Some((start, end)) = extra.range()
                && let Some((from, to)) =
                    selection::selected_cols(Range::Chars { start, end }, row, line_len)
            {
                highlight(from, to, self.theme.get("ui.selection"));
            }
        }

        // Virtual text, in its own colours, over whatever was under it.
        if let Some(display) = display {
            for (from, to, scope) in &display.virtual_spans {
                paint(*from, *to, false, self.theme.get(scope));
            }
        }

        if !last {
            return;
        }
        // After the line's text on its last row: a diagnostic's message,
        // then any end-of-line virtual text (git blame), as far as fits.
        let mut x = crate::core::columns::display_width(&line_text[..upto], tab_width)
            .saturating_sub(left)
            + 2;
        if let Some(d) = on_line
            && x < width
        {
            let message = d.message.lines().next().unwrap_or_default();
            let text = format!("● {message}");
            buf.set_stringn(
                text_area.x + x as u16,
                y,
                &text,
                width - x,
                self.theme
                    .get(&format!("ui.diagnostic.{}", d.severity.id())),
            );
            x += unicode_width::UnicodeWidthStr::width(text.as_str()) + 3;
        }
        for piece in self.document.end_of_line_text(row) {
            if x >= width {
                break;
            }
            buf.set_stringn(
                text_area.x + x as u16,
                y,
                &piece.text,
                width - x,
                row_style.patch(self.theme.get(&piece.style)),
            );
            x += unicode_width::UnicodeWidthStr::width(piece.text.as_str()) + 3;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LineNumbers;
    use crate::core::Position as TextPos;
    use crate::editor::windows::ViewStart;

    fn render(doc: &Document, options: &EditorOptions, w: u16, h: u16) -> (Buffer, ViewState) {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(doc, options, &Theme::default()).render(area, &mut buf, &mut state);
        (buf, state)
    }

    #[test]
    fn the_fold_column_marks_folds_left_of_the_numbers() {
        let mut doc = Document::with_text("fn a() {\n    x\n}\n{\n    y\n}\nz");
        doc.close_fold(3, 5);
        let folds = EditorOptions {
            fold_column: crate::config::FoldColumn::On,
            ..options(LineNumbers::Absolute)
        };
        let column = |buf: &Buffer| -> Vec<String> {
            (0..5).map(|y| buf[(0, y)].symbol().to_string()).collect()
        };
        let (buf, state) = render(&doc, &folds, 24, 6);
        assert_eq!(state.text_x, 6, "one cell for folds, five for numbers");
        assert_eq!(column(&buf), ["▾", "│", "│", "▸", " "]);
        assert_eq!(row_text(&buf, 4).trim_end(), "    7 z");

        let (buf, _) = render(&doc, &folds, 12, 6);
        assert_eq!(row_text(&buf, 1).trim_end(), "│     ↪ {", "wrapped: inside");

        let (_, state) = render(&doc, &options(LineNumbers::Absolute), 24, 6);
        assert_eq!(state.text_x, 5, "off by default");
    }

    #[test]
    fn maps_screen_cells_back_to_the_text() {
        let doc = Document::with_text("one two three four\n\tx\nlast");
        let options = EditorOptions {
            wrap: true,
            ..options(LineNumbers::Absolute)
        };
        let (_, state) = render(&doc, &options, 16, 8);
        let t = state.text_x;
        let at = |x, y| state.position_at(&doc, x, y, 4).map(|p| (p.row, p.col));
        assert_eq!(at(t, 0), Some((0, 0)));
        assert_eq!(at(t + 4, 0), Some((0, 4)), "the t of two");
        assert_eq!(at(0, 0), Some((0, 0)), "the gutter is the row's start");
        let first = state.rows[0];
        let second = state.rows[1];
        assert_eq!(
            (second.row, second.start),
            (0, first.end),
            "wrapped: {:?}",
            state.rows
        );
        assert_eq!(
            at(15, 0),
            Some((0, first.end - 1)),
            "past a wrapped part's end, its last char"
        );
        assert_eq!(
            at(t + second.indent as u16, 1),
            Some((0, second.start)),
            "after the wrap marker"
        );
        let tab_row = state.rows.iter().find(|r| r.row == 1).unwrap().y;
        assert_eq!(at(t + 2, tab_row), Some((1, 0)), "inside the tab");
        assert_eq!(at(t + 4, tab_row), Some((1, 1)), "after it");
        assert_eq!(at(15, tab_row), Some((1, 2)), "past the end");
        assert_eq!(
            at(t + 2, 7),
            Some((2, 4)),
            "below the text: the end of the last line"
        );
    }

    #[test]
    fn virtual_text_is_drawn_in_the_line_and_the_cursor_and_clicks_map_around_it() {
        use crate::editor::VirtualText;
        let mut doc = Document::with_text("let x = f(1);");
        let hint = |col, text: &str| VirtualText {
            at: TextPos::new(0, col),
            text: text.into(),
            style: "ui.virtual.inlay-hint".into(),
            source: "inlay".into(),
            end_of_line: false,
        };
        doc.set_virtual_text("inlay", vec![hint(5, ": i32"), hint(10, "n: ")]);
        doc.set_cursor(TextPos::new(0, 10));
        let options = options(LineNumbers::None);
        let theme = theme();
        let area = Rect::new(0, 0, 30, 2);
        let search =
            crate::core::search::SearchPattern::new("1", crate::core::search::CaseMode::Smart)
                .unwrap();
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(&doc, &options, &theme)
            .search(Some(&search))
            .render(area, &mut buf, &mut state);
        assert_eq!(row_text(&buf, 0).trim_end(), "let x: i32 = f(n: 1);");
        let hint_fg = theme.get("ui.virtual.inlay-hint").fg.unwrap();
        assert_eq!(buf[(5, 0)].fg, hint_fg, "the hint's colour");
        assert_eq!(buf[(15, 0)].fg, hint_fg);
        assert_eq!(
            buf[(18, 0)].bg,
            theme.get("ui.search.match").bg.unwrap(),
            "the 1, after the hint"
        );
        assert_eq!(state.cursor, Some(Position::new(18, 0)), "on the 1");
        assert_eq!(
            state.position_at(&doc, 7, 0, 4).map(|p| p.col),
            Some(5),
            "in the hint: the char after it"
        );
        assert_eq!(state.position_at(&doc, 18, 0, 4).map(|p| p.col), Some(10));

        // In insert mode the caret's before the hint, where typing goes.
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(&doc, &options, &theme)
            .caret(true)
            .render(area, &mut buf, &mut state);
        assert_eq!(state.cursor, Some(Position::new(15, 0)));

        // Wrapping counts the hints' width.
        let wrapping = EditorOptions {
            wrap: true,
            ..options.clone()
        };
        let narrow = Rect::new(0, 0, 14, 3);
        let mut buf = Buffer::empty(narrow);
        let mut state = ViewState::default();
        EditorView::new(&doc, &wrapping, &theme).render(narrow, &mut buf, &mut state);
        assert_eq!(state.rows.len(), 2, "{:?}", state.rows);
    }

    #[test]
    fn end_of_line_text_goes_after_the_line_and_is_cut_off_at_the_edge() {
        use crate::editor::VirtualText;
        let mut doc = Document::with_text("short\nother");
        doc.set_virtual_text(
            "blame",
            vec![VirtualText {
                at: TextPos::new(0, 0),
                text: "Jon, 3 days ago • A long commit summary".into(),
                style: "ui.virtual.blame".into(),
                source: "blame".into(),
                end_of_line: true,
            }],
        );
        let options = options(LineNumbers::None);
        let theme = theme();
        let area = Rect::new(0, 0, 30, 2);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(&doc, &options, &theme).render(area, &mut buf, &mut state);
        assert_eq!(row_text(&buf, 0), "short  Jon, 3 days ago • A lon");
        assert_eq!(buf[(7, 0)].fg, theme.get("ui.virtual.blame").fg.unwrap());
        assert_eq!(row_text(&buf, 1).trim_end(), "other");
        assert_eq!(
            state.position_at(&doc, 20, 0, 4).map(|p| p.col),
            Some(5),
            "clicks past it: the end"
        );
    }

    #[test]
    fn plugins_highlighted_ranges_are_drawn() {
        use crate::editor::RangeHighlight;
        let mut doc = Document::with_text("trailing   \nfine");
        doc.add_highlights(vec![RangeHighlight {
            start: TextPos::new(0, 8),
            end: TextPos::new(0, 11),
            style: "ui.search.match".into(),
            source: "plugin:ws".into(),
        }]);
        let options = options(LineNumbers::None);
        let theme = theme();
        let area = Rect::new(0, 0, 20, 2);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options, &theme).render(area, &mut buf, &mut ViewState::default());
        let bg = theme.get("ui.search.match").bg.unwrap();
        assert_eq!(
            [
                buf[(7, 0)].bg == bg,
                buf[(8, 0)].bg == bg,
                buf[(10, 0)].bg == bg,
                buf[(11, 0)].bg == bg
            ],
            [false, true, true, false]
        );
    }

    #[test]
    fn git_changes_are_marked_in_the_gutter() {
        let doc = Document::with_text("a\nadded\nchanged\nb\nc");
        let hunks = crate::git::line_changes("a\nCHANGED\nb\nremoved\nc\n", &doc.buffer().text());
        let options = options(LineNumbers::Absolute);
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let theme = theme();
        EditorView::new(&doc, &options, &theme)
            .git_changes(&hunks)
            .render(area, &mut buf, &mut ViewState::default());
        let mark = |y: u16| buf[(4, y)].symbol().to_string();
        assert_eq!(row_text(&buf, 1).trim_end(), "   2▎added");
        assert_eq!(
            [mark(0), mark(1), mark(2), mark(3), mark(4)],
            [" ", "▎", "▎", "▁", " "],
            "{hunks:?}"
        );
        assert_eq!(buf[(4, 1)].fg, theme.get("ui.gutter.changed").fg.unwrap());
        assert_eq!(buf[(4, 3)].fg, theme.get("ui.gutter.removed").fg.unwrap());
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn options(line_numbers: LineNumbers) -> EditorOptions {
        EditorOptions {
            line_numbers,
            scroll_off: 0,
            ..EditorOptions::default()
        }
    }

    fn diagnostic(
        start: (usize, usize),
        end: (usize, usize),
        severity: crate::editor::diagnostics::Severity,
        message: &str,
    ) -> crate::editor::diagnostics::Diagnostic {
        crate::editor::diagnostics::Diagnostic {
            start: TextPos::new(start.0, start.1),
            end: TextPos::new(end.0, end.1),
            severity,
            message: message.into(),
            source: None,
        }
    }

    fn render_with(
        doc: &Document,
        diagnostics: &[crate::editor::diagnostics::Diagnostic],
        w: u16,
        h: u16,
    ) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let options = options(LineNumbers::Absolute);
        EditorView::new(doc, &options, &theme())
            .diagnostics(diagnostics)
            .render(area, &mut buf, &mut ViewState::default());
        buf
    }

    /// The default config's theme, which has the diagnostic styles.
    fn theme() -> Theme {
        crate::config::Config::default().theme
    }

    /// Draws `doc` with indent guides (4 wide), bracket colours and the
    /// pair at the cursor, no line numbers.
    fn render_decorated(doc: &Document, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let options = EditorOptions {
            match_brackets: true,
            ..options(LineNumbers::None)
        };
        let comments = crate::core::brackets::Comments {
            line: Some("//".into()),
            block: None,
        };
        let states = crate::core::brackets::line_states(
            (0..doc.buffer().line_count()).map(|r| doc.buffer().line(r)),
            &comments,
        );
        EditorView::new(doc, &options, &theme())
            .indent_guides(4)
            .brackets(std::rc::Rc::new(states), comments)
            .render(area, &mut buf, &mut ViewState::default());
        buf
    }

    #[test]
    fn indent_guides_go_in_the_indentation_and_through_blank_lines() {
        let doc = Document::with_text("fn f() {\n    if x {\n        y\n\n        z\n    }\n}");
        let buf = render_decorated(&doc, 20, 8);
        assert_eq!(row_text(&buf, 0).trim_end(), "fn f() {");
        assert_eq!(row_text(&buf, 1).trim_end(), "│   if x {");
        assert_eq!(row_text(&buf, 2).trim_end(), "│   │   y");
        assert_eq!(row_text(&buf, 3).trim_end(), "│   │", "a blank line inside");
        assert_eq!(row_text(&buf, 5).trim_end(), "│   }");
        // The cursor's block (row 0 here is outside every block): no
        // guide is the active one.
        let guide = theme().get("ui.indent-guide");
        assert_eq!(buf[(0, 1)].fg, guide.fg.unwrap());
    }

    #[test]
    fn the_cursors_block_has_the_brighter_guide() {
        let mut doc = Document::with_text("{\n    a\n    b\n}\n{\n    c\n}");
        doc.set_cursor(TextPos::new(2, 4));
        let buf = render_decorated(&doc, 20, 8);
        let active = theme().get("ui.indent-guide.active").fg.unwrap();
        let plain = theme().get("ui.indent-guide").fg.unwrap();
        assert_eq!(buf[(0, 1)].fg, active);
        assert_eq!(buf[(0, 2)].fg, active);
        assert_eq!(buf[(0, 5)].fg, plain, "another block's");
    }

    #[test]
    fn brackets_are_coloured_by_depth_and_the_pair_at_the_cursor_marked() {
        let mut doc = Document::with_text("f(a[0], \"(\") // (");
        doc.set_cursor(TextPos::new(0, 1));
        let buf = render_decorated(&doc, 30, 2);
        let colour = |n: usize| theme().get(&format!("ui.bracket.{n}")).fg.unwrap();
        assert_eq!(buf[(1, 0)].fg, colour(1), "(");
        assert_eq!(buf[(3, 0)].fg, colour(2), "[");
        assert_eq!(buf[(5, 0)].fg, colour(2), "]");
        assert_eq!(buf[(11, 0)].fg, colour(1), ")");
        // In a string or a comment: not a bracket.
        assert_ne!(buf[(9, 0)].fg, colour(2));
        assert_ne!(buf[(16, 0)].fg, colour(1));
        // The cursor's on `(`: it and its `)` are marked.
        let matched = theme().get("ui.bracket.match").bg.unwrap();
        assert_eq!(buf[(1, 0)].bg, matched);
        assert_eq!(buf[(11, 0)].bg, matched);
        assert_ne!(buf[(3, 0)].bg, matched);
    }

    fn underlined(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| {
                let cell = &buf[(x, y)];
                if cell.modifier.contains(ratatui::style::Modifier::UNDERLINED) {
                    '^'
                } else {
                    ' '
                }
            })
            .collect()
    }

    #[test]
    fn a_diagnostics_message_shows_once_where_it_starts() {
        use crate::editor::diagnostics::Severity;
        let doc = Document::with_text("struct A {\n\n    x: u8,\n}");
        // Like rust-analyzer's "not included in the module tree", over
        // the whole file.
        let whole = diagnostic((0, 0), (3, 1), Severity::Hint, "not in the tree");
        let buf = render_with(&doc, &[whole], 40, 4);
        assert_eq!(
            row_text(&buf, 0).trim_end(),
            "   1 struct A {  ● not in the tree"
        );
        for y in 1..4 {
            assert!(!row_text(&buf, y).contains('●'), "row {y}");
        }
        // Underlined throughout, but the empty line isn't marked.
        assert_eq!(underlined(&buf, 0).trim_end(), "     ^^^^^^^^^^");
        assert_eq!(underlined(&buf, 1).trim_end(), "", "empty line");
        assert_eq!(underlined(&buf, 2).trim_end(), "     ^^^^^^^^^^");
        assert_eq!(underlined(&buf, 3).trim_end(), "     ^");
        // Only the first line's number takes its colour.
        let hint = theme().get("ui.diagnostic.hint").fg;
        assert_eq!(buf[(3, 0)].fg, hint.unwrap());
        assert_ne!(Some(buf[(3, 2)].fg), hint);
    }

    #[test]
    fn references_are_drawn_with_their_background() {
        let doc = Document::with_text("let value = value;");
        let ranges = [
            (TextPos::new(0, 4), TextPos::new(0, 9)),
            (TextPos::new(0, 12), TextPos::new(0, 17)),
        ];
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        let options = options(LineNumbers::Absolute);
        EditorView::new(&doc, &options, &theme())
            .references(&ranges)
            .render(area, &mut buf, &mut ViewState::default());
        let bg = theme().get("ui.highlight.reference").bg.unwrap();
        let marked: String = (0..30)
            .map(|x| if buf[(x, 0)].bg == bg { '^' } else { ' ' })
            .collect();
        assert_eq!(marked.trim_end(), "         ^^^^^   ^^^^^");
    }

    #[test]
    fn the_most_serious_diagnostic_on_a_line_is_the_one_shown() {
        use crate::editor::diagnostics::Severity;
        let doc = Document::with_text("let x = y;");
        let warning = diagnostic((0, 4), (0, 5), Severity::Warning, "unused x");
        let error = diagnostic((0, 8), (0, 9), Severity::Error, "no y");
        // An empty range at the end still marks a cell.
        let at_end = diagnostic((0, 10), (0, 10), Severity::Info, "end");
        let buf = render_with(&doc, &[warning, error, at_end], 40, 1);
        assert_eq!(row_text(&buf, 0).trim_end(), "   1 let x = y;  ● no y");
        assert_eq!(underlined(&buf, 0).trim_end(), "         ^   ^ ^");
        let colour = |scope: &str| theme().get(scope).fg.unwrap();
        assert_eq!(
            buf[(19, 0)].fg,
            colour("ui.diagnostic.error"),
            "the message"
        );
        let underline = |x: u16| buf[(x, 0)].underline_color;
        let underline_of = |scope: &str| theme().get(scope).underline_color.unwrap();
        assert_eq!(underline(9), underline_of("diagnostic.warning"));
        assert_eq!(underline(13), underline_of("diagnostic.error"));
    }

    #[test]
    fn renders_gutter_and_text() {
        let doc = Document::with_text("one\ntwo");
        let (buf, _) = render(&doc, &options(LineNumbers::Absolute), 12, 3);
        assert_eq!(row_text(&buf, 0), "   1 one    ");
        assert_eq!(row_text(&buf, 1), "   2 two    ");
        assert_eq!(row_text(&buf, 2), "            ");
    }

    #[test]
    fn renders_without_gutter() {
        let doc = Document::with_text("one");
        let (buf, state) = render(&doc, &options(LineNumbers::None), 5, 1);
        assert_eq!(row_text(&buf, 0), "one  ");
        assert_eq!(state.cursor, Some(Position::new(0, 0)));
    }

    #[test]
    fn cursor_position_accounts_for_gutter() {
        let mut doc = Document::with_text("abc\ndef");
        doc.set_cursor(TextPos::new(1, 2));
        let (_, state) = render(&doc, &options(LineNumbers::Absolute), 12, 3);
        assert_eq!(state.cursor, Some(Position::new(7, 1)));
    }

    #[test]
    fn scrolls_to_keep_cursor_visible() {
        let text = (1..=10)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut doc = Document::with_text(&text);
        doc.set_cursor(TextPos::new(9, 0));
        let (buf, state) = render(&doc, &options(LineNumbers::None), 4, 3);
        assert_eq!(row_text(&buf, 2), "10  ");
        assert_eq!(state.viewport.top.line, 7);
        assert_eq!(state.cursor, Some(Position::new(0, 2)));
    }

    #[test]
    fn a_line_taller_than_the_view_is_drawn_from_partway_with_a_marker() {
        let mut doc = Document::with_text(&format!(
            "x\n{}{}{}{}",
            "a".repeat(10),
            "b".repeat(10),
            "c".repeat(10),
            "d".repeat(10)
        ));
        doc.set_cursor(TextPos::new(1, 35));
        let options = EditorOptions {
            wrap: true,
            wrap_marker: String::new(),
            ..options(LineNumbers::None)
        };
        let (buf, state) = render(&doc, &options, 10, 3);
        assert_eq!(state.viewport.top, ViewStart { line: 1, skip: 1 });
        assert_eq!(row_text(&buf, 0), "<<<bbbbbbb");
        assert_eq!(row_text(&buf, 1), "cccccccccc");
        assert_eq!(row_text(&buf, 2), "dddddddddd");
        assert_eq!(state.cursor, Some(Position::new(5, 2)));
        // Clicks map to what's drawn, not the line's first rows.
        assert_eq!(state.rows[0].start, 10);
        let clicked = state.position_at(&doc, 4, 0, 4);
        assert_eq!(clicked, Some(TextPos::new(1, 14)));
    }

    #[test]
    fn cursor_accounts_for_wide_chars() {
        let mut doc = Document::with_text("日本");
        doc.set_cursor(TextPos::new(0, 1));
        let (_, state) = render(&doc, &options(LineNumbers::None), 10, 1);
        assert_eq!(state.cursor, Some(Position::new(2, 0)));
    }

    #[test]
    fn highlights_selection_including_line_break() {
        let doc = Document::with_text("abc\nde");
        let mut theme = Theme::default();
        let selected = Style::new().bg(ratatui::style::Color::Blue);
        theme.set("ui.selection", selected);
        let range = Range::Chars {
            start: TextPos::new(0, 1),
            end: TextPos::new(1, 1),
        };

        let area = Rect::new(0, 0, 6, 2);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .selection(Some(range))
            .render(area, &mut buf, &mut ViewState::default());

        let highlighted = |y: u16| -> Vec<u16> {
            (0..6)
                .filter(|&x| buf[(x, y)].bg == ratatui::style::Color::Blue)
                .collect()
        };
        assert_eq!(highlighted(0), vec![1, 2, 3], "b, c and the line break");
        assert_eq!(highlighted(1), vec![0], "d");
    }

    #[test]
    fn highlights_a_block_column_by_column() {
        let doc = Document::with_text("abcd\na\n\tx");
        let mut theme = Theme::default();
        theme.set("ui.selection", Style::new().bg(ratatui::style::Color::Blue));
        let block = crate::core::Block {
            first: 0,
            last: 2,
            left: 1,
            right: 2,
            to_eol: false,
        };
        let area = Rect::new(0, 0, 8, 3);
        let mut buf = Buffer::empty(area);
        // A selection range is ignored when there's a block.
        let lines = Range::Lines { first: 0, last: 2 };
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .selection(Some(lines))
            .block(Some(block))
            .render(area, &mut buf, &mut ViewState::default());

        let highlighted = |y: u16| -> Vec<u16> {
            (0..8)
                .filter(|&x| buf[(x, y)].bg == ratatui::style::Color::Blue)
                .collect()
        };
        assert_eq!(highlighted(0), vec![1, 2], "b and c");
        assert!(highlighted(1).is_empty(), "too short to reach the block");
        assert_eq!(
            highlighted(2),
            vec![0, 1, 2, 3],
            "the whole tab it cuts into"
        );
    }

    #[test]
    fn a_block_over_inlay_hints_lines_up_on_screen() {
        use crate::editor::VirtualText;
        // `f(1, 2)` above `f(x: 1, 2)`, the `x: ` a hint.
        let mut doc = Document::with_text("f(1, 2)\nf(1, 2)");
        doc.set_virtual_text(
            "inlay",
            vec![VirtualText {
                at: TextPos::new(1, 2),
                text: "x: ".into(),
                style: "ui.virtual.inlay-hint".into(),
                source: "inlay".into(),
                end_of_line: false,
            }],
        );
        let mut theme = Theme::default();
        theme.set("ui.selection", Style::new().bg(ratatui::style::Color::Blue));
        let block = crate::core::Block {
            first: 0,
            last: 1,
            left: 2,
            right: 5,
            to_eol: false,
        };
        let area = Rect::new(0, 0, 12, 2);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .block(Some(block))
            .render(area, &mut buf, &mut ViewState::default());
        let highlighted = |y: u16| -> Vec<u16> {
            (0..12)
                .filter(|&x| buf[(x, y)].bg == ratatui::style::Color::Blue)
                .collect()
        };
        assert_eq!(highlighted(0), vec![2, 3, 4, 5], "`1, 2`");
        assert_eq!(highlighted(1), vec![5], "the `1`: the hint holds no text");
    }

    #[test]
    fn selections_highlight_every_row_of_a_wrapped_line() {
        let doc = Document::with_text("aaaa bbbb");
        let mut theme = Theme::default();
        theme.set("ui.selection", Style::new().bg(ratatui::style::Color::Blue));
        let range = Range::Chars {
            start: TextPos::new(0, 2),
            end: TextPos::new(0, 7),
        };
        let area = Rect::new(0, 0, 5, 2);
        let mut buf = Buffer::empty(area);
        let options = EditorOptions {
            wrap_marker: String::new(),
            ..options(LineNumbers::None)
        };
        EditorView::new(&doc, &options, &theme)
            .selection(Some(range))
            .render(area, &mut buf, &mut ViewState::default());
        let blue = |y: u16| -> Vec<u16> {
            (0..5)
                .filter(|&x| buf[(x, y)].bg == ratatui::style::Color::Blue)
                .collect()
        };
        assert_eq!(blue(0), vec![2, 3, 4], "\"aa \" of \"aaaa \"");
        assert_eq!(blue(1), vec![0, 1], "\"bb\" of \"bbbb\"");
    }

    #[test]
    fn selection_highlight_respects_horizontal_scroll() {
        let mut doc = Document::with_text("0123456789");
        doc.set_cursor(TextPos::new(0, 9));
        let mut theme = Theme::default();
        theme.set("ui.selection", Style::new().bg(ratatui::style::Color::Blue));
        let range = Range::Chars {
            start: TextPos::new(0, 2),
            end: TextPos::new(0, 8),
        };

        let area = Rect::new(0, 0, 4, 1);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        let options = EditorOptions {
            wrap: false,
            ..options(LineNumbers::None)
        };
        EditorView::new(&doc, &options, &theme)
            .selection(Some(range))
            .render(area, &mut buf, &mut state);

        // Scrolled so columns 6..=9 are visible; 6 and 7 are selected.
        assert_eq!(state.viewport.left, 6);
        let highlighted: Vec<u16> = (0..4)
            .filter(|&x| buf[(x, 0)].bg == ratatui::style::Color::Blue)
            .collect();
        assert_eq!(highlighted, vec![0, 1]);
    }

    #[test]
    fn highlights_search_matches() {
        let doc = Document::with_text("ab ab");
        let mut theme = Theme::default();
        theme.set(
            "ui.search.match",
            Style::new().bg(ratatui::style::Color::Yellow),
        );
        let pattern = SearchPattern::new("ab", crate::core::search::CaseMode::Smart).unwrap();

        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .search(Some(&pattern))
            .render(area, &mut buf, &mut ViewState::default());

        let highlighted: Vec<u16> = (0..6)
            .filter(|&x| buf[(x, 0)].bg == ratatui::style::Color::Yellow)
            .collect();
        assert_eq!(highlighted, vec![0, 1, 3, 4]);
    }

    #[test]
    fn highlights_matches_running_across_rows() {
        let doc = Document::with_text("ab\ncd\nef");
        let mut theme = Theme::default();
        theme.set(
            "ui.search.match",
            Style::new().bg(ratatui::style::Color::Yellow),
        );
        let pattern =
            SearchPattern::new(r"b\ncd\ne", crate::core::search::CaseMode::Smart).unwrap();
        let area = Rect::new(0, 0, 4, 3);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .search(Some(&pattern))
            .render(area, &mut buf, &mut ViewState::default());
        let highlighted = |y: u16| -> Vec<u16> {
            (0..4)
                .filter(|&x| buf[(x, y)].bg == ratatui::style::Color::Yellow)
                .collect()
        };
        assert_eq!(highlighted(0), vec![1, 2], "the b and its line break");
        assert_eq!(highlighted(1), vec![0, 1, 2]);
        assert_eq!(highlighted(2), vec![0]);
    }

    #[test]
    fn colours_text_with_syntax_highlights() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "fn main() {}").unwrap();
        let (doc, _) = Document::open(&path).unwrap();
        let mut engine = crate::syntax::SyntaxEngine::default();
        let highlights = engine.highlights_default(&doc);

        let mut theme = Theme::default();
        theme.set("keyword", Style::new().fg(ratatui::style::Color::Magenta));
        let area = Rect::new(0, 0, 12, 1);
        let mut buf = Buffer::empty(area);
        EditorView::new(&doc, &options(LineNumbers::None), &theme)
            .syntax(highlights)
            .render(area, &mut buf, &mut ViewState::default());

        let magenta: Vec<u16> = (0..12)
            .filter(|&x| buf[(x, 0)].fg == ratatui::style::Color::Magenta)
            .collect();
        assert_eq!(magenta, vec![0, 1], "`fn` is a keyword");
    }

    #[test]
    fn tabs_render_as_spaces_and_the_cursor_follows() {
        let mut doc = Document::with_text("\tx");
        doc.set_cursor(TextPos::new(0, 1));
        let options = EditorOptions {
            tab_width: 4,
            ..options(LineNumbers::None)
        };
        let area = Rect::new(0, 0, 8, 1);
        let mut buf = Buffer::empty(area);
        let mut state = ViewState::default();
        EditorView::new(&doc, &options, &Theme::default()).render(area, &mut buf, &mut state);
        assert_eq!(row_text(&buf, 0), "    x   ");
        assert_eq!(state.cursor, Some(Position::new(4, 0)));
    }

    #[test]
    fn zero_sized_area_does_not_panic() {
        let doc = Document::with_text("x");
        let (_, state) = render(&doc, &options(LineNumbers::Absolute), 0, 0);
        assert_eq!(state.cursor, None);
    }
}
