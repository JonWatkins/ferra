//! Split windows.
//!
//! A window is a view onto a buffer with its own cursor; the UI also keeps
//! a scroll position per window (and buffer), and tells the editor where
//! each window's view starts after drawing it, so commands like `<C-d>`,
//! `zz` and `H` can work from it ([`Windows::view_top`]). Several windows
//! can show the same buffer.
//!
//! The active window's buffer is always `Editor::document`, so commands work
//! exactly as with one window. Other windows remember which buffer they
//! show and where their cursor was; switching windows saves the active
//! window's cursor and swaps the target's buffer in.

mod layout;

pub use layout::{Area, Direction, Layout, Separator, SplitDirection};

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use super::{Document, DocumentId, Editor, Mode};
use crate::core::Position;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    /// The buffer shown. For the active window, `Editor::document` is the
    /// source of truth and this is updated when focus moves away.
    pub document: DocumentId,
    /// The cursor, for windows that aren't active.
    pub cursor: Position,
}

#[derive(Debug, Clone)]
pub struct Windows {
    layout: Layout,
    windows: Vec<Window>,
    active: WindowId,
    last_id: u64,
    /// Where the windows were last drawn, so resizing can work in cells.
    /// Set by the UI while drawing (hence the `Cell`).
    area: Cell<Area>,
    /// Each window's first row on screen, as last drawn (set by the UI).
    tops: RefCell<HashMap<WindowId, usize>>,
    /// First rows commands asked for, which the UI uses at the next draw.
    requested_tops: RefCell<HashMap<WindowId, usize>>,
}

/// The area assumed before anything is drawn (tests, a first command).
const DEFAULT_AREA: Area = Area::new(0, 0, 80, 22);

impl Default for Windows {
    fn default() -> Self {
        Self {
            layout: Layout::Window(WindowId(0)),
            windows: vec![Window {
                id: WindowId(0),
                document: DocumentId(0),
                cursor: Position::default(),
            }],
            active: WindowId(0),
            last_id: 0,
            area: Cell::new(DEFAULT_AREA),
            tops: RefCell::default(),
            requested_tops: RefCell::default(),
        }
    }
}

impl Windows {
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn active(&self) -> WindowId {
        self.active
    }

    pub fn count(&self) -> usize {
        self.windows.len()
    }

    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Records where the windows are drawn (the UI calls this).
    pub fn note_area(&self, area: Area) {
        self.area.set(area);
    }

    /// Each window's current size, as last drawn.
    pub fn sizes(&self) -> Vec<(WindowId, Area)> {
        self.layout.arrange(self.area.get()).0
    }

    /// How many rows of text window `id` shows.
    pub fn height(&self, id: WindowId) -> usize {
        self.sizes()
            .into_iter()
            .find(|(window, _)| *window == id)
            .map_or(0, |(_, area)| area.height as usize)
    }

    /// Records the first row window `id` showed when drawn (the UI calls
    /// this).
    pub fn note_top(&self, id: WindowId, top: usize) {
        self.tops.borrow_mut().insert(id, top);
    }

    /// Asks for window `id`'s view to start at `top` from the next draw.
    pub fn request_top(&self, id: WindowId, top: usize) {
        self.requested_tops.borrow_mut().insert(id, top);
    }

    /// The first row asked for by a command since the last draw (the UI
    /// calls this).
    pub fn take_requested_top(&self, id: WindowId) -> Option<usize> {
        self.requested_tops.borrow_mut().remove(&id)
    }

    /// How many cells wide window `id` is.
    pub fn width(&self, id: WindowId) -> usize {
        self.sizes()
            .into_iter()
            .find(|(window, _)| *window == id)
            .map_or(0, |(_, area)| area.width as usize)
    }

    /// Where window `id`'s view of `document` starts: the line asked for
    /// since the last draw, or the one drawn, scrolled as the next draw
    /// will scroll it to keep `cursor` in sight.
    pub fn view_top(
        &self,
        id: WindowId,
        document: &Document,
        cursor: Position,
        options: &crate::config::EditorOptions,
    ) -> usize {
        let top = self
            .requested_tops
            .borrow()
            .get(&id)
            .or(self.tops.borrow().get(&id))
            .copied()
            .unwrap_or(0);
        let layout = TextLayout::new(self.width(id), document, options);
        layout.follow_cursor(top, document, cursor, self.height(id), options.scroll_off)
    }

    fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }
}

/// The first line of a view `height` screen rows tall that started at line
/// `top`, once scrolled just enough to show the cursor (on line `row`, its
/// `row_in_line`th screen row) with `scroll_off` screen rows of context
/// above and below it where the buffer allows. `rows_of(line)` is how many
/// screen rows a line takes (1 each without wrapping). The view always
/// starts at the start of a line.
pub fn follow_cursor(
    top: usize,
    cursor: (usize, usize),
    height: usize,
    line_count: usize,
    scroll_off: usize,
    rows_of: impl Fn(usize) -> usize,
) -> usize {
    let (row, row_in_line) = cursor;
    if height == 0 {
        return top;
    }
    let margin = scroll_off.min((height - 1) / 2);
    let mut top = top;
    // Every line takes a row at least, so lines more than a screen above
    // the cursor can't be in the view; start no further back than that.
    if row > top + height {
        top = row - height;
    }

    // Above: `margin` rows (as many as there are) between the top and the
    // cursor's row.
    let mut above = if row < top {
        top = row;
        row_in_line
    } else {
        (top..row).map(&rows_of).sum::<usize>() + row_in_line
    };
    while above < margin && top > 0 {
        top -= 1;
        above += rows_of(top);
    }

    // Below: the cursor's row and `margin` more must fit.
    let mut below = rows_of(row).saturating_sub(row_in_line + 1);
    let mut next = row + 1;
    while below < margin && next < line_count {
        below += rows_of(next);
        next += 1;
    }
    let mut used = above + 1 + below.min(margin);
    while used > height && top < row {
        used -= rows_of(top);
        top += 1;
    }
    top
}

/// The line-number gutter's width in cells, for a buffer of `line_count`
/// lines.
pub fn gutter_width(line_count: usize, mode: crate::config::LineNumbers) -> u16 {
    if mode == crate::config::LineNumbers::None {
        return 0;
    }
    // At least three digits, and a cell of padding either side.
    let digits = line_count.to_string().len().max(3);
    (digits + 2) as u16
}

/// How a window lays out a buffer's lines: `width` cells of text (beside
/// the gutter), wrapped or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLayout {
    pub width: usize,
    pub wrap: bool,
    pub tab_width: usize,
}

impl TextLayout {
    /// The layout of `document` in a window `window_width` cells wide.
    pub fn new(
        window_width: usize,
        document: &Document,
        options: &crate::config::EditorOptions,
    ) -> Self {
        let gutter = gutter_width(document.buffer().line_count(), options.line_numbers);
        Self {
            width: window_width.saturating_sub(gutter as usize),
            wrap: options.wrap,
            tab_width: options.tab_width,
        }
    }

    /// The char column each screen row of `line` starts at.
    pub fn row_starts(&self, line: &str) -> Vec<usize> {
        if self.wrap {
            crate::core::wrap::row_starts(line, self.width, self.tab_width)
        } else {
            vec![0]
        }
    }

    /// How many screen rows `line` takes.
    pub fn rows(&self, line: &str) -> usize {
        if self.wrap {
            self.row_starts(line).len()
        } else {
            1
        }
    }

    /// The first line of a view of `document` `height` rows tall that
    /// started at `top`, scrolled to show `cursor` (see [`follow_cursor`]).
    pub fn follow_cursor(
        &self,
        top: usize,
        document: &Document,
        cursor: Position,
        height: usize,
        scroll_off: usize,
    ) -> usize {
        let buffer = document.buffer();
        let row_in_line = if self.wrap {
            crate::core::wrap::row_of(&self.row_starts(&buffer.line(cursor.row)), cursor.col)
        } else {
            0
        };
        follow_cursor(
            top,
            (cursor.row, row_in_line),
            height,
            buffer.line_count(),
            scroll_off,
            |row| self.rows(&buffer.line(row)),
        )
    }

    /// The last line at least partly shown in a view `height` rows tall
    /// starting at `top`.
    pub fn bottom(&self, document: &Document, top: usize, height: usize) -> usize {
        let buffer = document.buffer();
        let last = buffer.line_count() - 1;
        if !self.wrap {
            return (top + height)
                .saturating_sub(1)
                .min(last)
                .max(top.min(last));
        }
        let mut used = 0;
        let mut row = top.min(last);
        loop {
            used += self.rows(&buffer.line(row));
            if used >= height || row == last {
                return row;
            }
            row += 1;
        }
    }
}

impl Editor {
    pub fn windows(&self) -> &Windows {
        &self.windows
    }

    /// The buffer shown in window `id`, with the cursor to draw for it.
    pub fn window_view(&self, id: WindowId) -> Option<(&Document, Position)> {
        if id == self.windows.active {
            return Some((&self.document, self.document.cursor()));
        }
        let window = self.windows.get(id)?;
        let document = self.document_by_id(window.document)?;
        Some((document, window.cursor))
    }

    /// Resizes the active window's height (`Horizontal`) or width
    /// (`Vertical`); `want` maps the current size and the most it can be to
    /// the size wanted. Like Vim, does nothing (returning `false`) when no
    /// split in that direction holds the window.
    pub fn resize_window(&mut self, axis: SplitDirection, want: impl Fn(u16, u16) -> u16) -> bool {
        let active = self.windows.active;
        let area = self.windows.area.get();
        self.windows.layout.resize(active, axis, area, &want)
    }

    /// `<C-w>x`: swaps the active window with the next one in its row or
    /// column. The cursor stays in place on screen, so it ends up in the
    /// other window, as in Vim.
    pub fn exchange_window(&mut self) -> Result<(), String> {
        let active = self.windows.active;
        let other = self
            .windows
            .layout
            .exchange(active)
            .ok_or("No window to exchange with in this row or column")?;
        self.focus_window(other);
        Ok(())
    }

    /// `<C-w>r` / `<C-w>R`: rotates the windows in the active one's row or
    /// column.
    pub fn rotate_windows(&mut self, forward: bool) -> Result<(), String> {
        let active = self.windows.active;
        if self.windows.layout.rotate(active, forward) {
            Ok(())
        } else {
            Err("Can't rotate: this row or column has windows split the other way".into())
        }
    }

    /// `<C-w>H`/`J`/`K`/`L`: moves the active window to an edge of the
    /// screen, full height or full width.
    pub fn move_window_to_edge(&mut self, edge: Direction) {
        let active = self.windows.active;
        self.windows.layout.move_to_edge(active, edge);
    }

    /// `<C-w>=`: makes every window the same size again.
    pub fn equalize_windows(&mut self) {
        self.windows.layout.equalize();
    }

    /// `:split` / `:vsplit`: opens a new window on the current buffer, at
    /// the same place, and makes it active.
    pub fn split_window(&mut self, direction: SplitDirection) -> WindowId {
        self.windows.last_id += 1;
        let id = WindowId(self.windows.last_id);
        let active = self.windows.active;
        self.windows.layout.split(active, id, direction);
        self.windows.windows.push(Window {
            id,
            document: self.document.id(),
            cursor: self.document.cursor(),
        });
        self.focus_window(id);
        id
    }

    /// Makes window `id` active. Returns `false` if there's no such window.
    pub fn focus_window(&mut self, id: WindowId) -> bool {
        let Some(target) = self.windows.get(id).cloned() else {
            return false;
        };
        if id != self.windows.active {
            self.set_mode(Mode::Normal);
            self.save_active_window();
            self.windows.active = id;
            self.switch_to(target.document);
            self.document.set_cursor(target.cursor);
            self.enforce_cursor_rules();
        }
        true
    }

    /// `<C-w>w`: the next window in reading order, wrapping.
    pub fn focus_next_window(&mut self) {
        let order = self.windows.layout.windows();
        let i = order
            .iter()
            .position(|&w| w == self.windows.active)
            .unwrap_or(0);
        self.focus_window(order[(i + 1) % order.len()]);
    }

    /// `<C-w>h/j/k/l`: the window in `direction`. Returns `false` if there
    /// is none.
    pub fn focus_window_in(&mut self, direction: Direction) -> bool {
        match self.windows.layout.neighbor(self.windows.active, direction) {
            Some(id) => self.focus_window(id),
            None => false,
        }
    }

    /// `:close`: closes the active window (its buffer stays open). The last
    /// window can't be closed.
    pub fn close_window(&mut self) -> Result<(), String> {
        if self.windows.count() == 1 {
            return Err("Cannot close the last window".into());
        }
        let closing = self.windows.active;
        let order = self.windows.layout.windows();
        let i = order.iter().position(|&w| w == closing).unwrap_or(0);
        // Focus the window before this one (or after, if it was first).
        let next = if i > 0 { order[i - 1] } else { order[1] };
        self.focus_window(next);
        self.windows.layout.remove(closing);
        self.windows.windows.retain(|w| w.id != closing);
        Ok(())
    }

    /// `:only`: closes every other window.
    pub fn only_window(&mut self) {
        let active = self.windows.active;
        self.windows.layout = Layout::Window(active);
        self.windows.windows.retain(|w| w.id == active);
    }

    /// Whether a window other than the active one shows `document`.
    pub(super) fn shown_in_other_window(&self, document: DocumentId) -> bool {
        self.windows
            .windows
            .iter()
            .any(|w| w.id != self.windows.active && w.document == document)
    }

    /// Points windows showing `old` (a closed buffer) at `new` instead.
    pub(super) fn replace_window_document(&mut self, old: DocumentId, new: DocumentId) {
        for window in &mut self.windows.windows {
            if window.document == old {
                window.document = new;
                window.cursor = Position::default();
            }
        }
    }

    fn save_active_window(&mut self) {
        let document = self.document.id();
        let cursor = self.document.cursor();
        if let Some(window) = self.windows.get_mut(self.windows.active) {
            window.document = document;
            window.cursor = cursor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn editor(text: &str) -> Editor {
        Editor::with_document(Document::with_text(text))
    }

    #[test]
    fn split_shows_the_same_buffer_at_the_same_place() {
        let mut e = editor("a\nb\nc");
        e.document.set_cursor(pos(1, 0));
        let new = e.split_window(SplitDirection::Vertical);
        assert_eq!(e.windows().count(), 2);
        assert_eq!(e.windows().active(), new);
        assert_eq!(e.document.cursor(), pos(1, 0));
        let (doc, cursor) = e.window_view(WindowId(0)).unwrap();
        assert_eq!((doc.id(), cursor), (e.document.id(), pos(1, 0)));
    }

    #[test]
    fn windows_on_one_buffer_keep_their_own_cursors() {
        let mut e = editor("a\nb\nc");
        e.split_window(SplitDirection::Horizontal);
        e.document.set_cursor(pos(2, 0));
        e.focus_window(WindowId(0));
        assert_eq!(e.document.cursor(), pos(0, 0));
        e.focus_next_window();
        assert_eq!(e.document.cursor(), pos(2, 0));
    }

    #[test]
    fn edits_show_in_every_window_on_the_buffer() {
        let mut e = editor("a");
        e.split_window(SplitDirection::Vertical);
        e.document.insert_str("new ");
        let (doc, _) = e.window_view(WindowId(0)).unwrap();
        assert_eq!(doc.buffer().text(), "new a");
    }

    #[test]
    fn windows_can_show_different_buffers() {
        let mut e = editor("first");
        e.split_window(SplitDirection::Vertical);
        let mut other = Document::with_text("second");
        other.set_id(e.new_document_id());
        e.activate(other);

        let (doc, _) = e.window_view(WindowId(0)).unwrap();
        assert_eq!(
            doc.buffer().text(),
            "first",
            "the other window keeps its buffer"
        );
        e.focus_window(WindowId(0));
        assert_eq!(e.document.buffer().text(), "first");
        e.focus_next_window();
        assert_eq!(e.document.buffer().text(), "second");
    }

    #[test]
    fn a_buffer_shown_elsewhere_isnt_dropped() {
        // The starting scratch buffer is normally replaced by the first file
        // opened, but not while another window still shows it.
        let mut e = Editor::default();
        e.split_window(SplitDirection::Vertical);
        let mut file = Document::with_text("file");
        file.set_id(e.new_document_id());
        e.activate(file);
        assert!(e.window_view(WindowId(0)).is_some());
        assert_eq!(e.buffer_count(), 2);
    }

    #[test]
    fn directional_focus() {
        let mut e = editor("x");
        e.split_window(SplitDirection::Vertical);
        assert!(e.focus_window_in(Direction::Left));
        assert_eq!(e.windows().active(), WindowId(0));
        assert!(!e.focus_window_in(Direction::Left));
        assert!(!e.focus_window_in(Direction::Up));
        assert!(e.focus_window_in(Direction::Right));
        assert_eq!(e.windows().active(), WindowId(1));
    }

    #[test]
    fn close_and_only() {
        let mut e = editor("x");
        assert!(e.close_window().is_err(), "can't close the last window");
        e.split_window(SplitDirection::Vertical);
        e.split_window(SplitDirection::Horizontal);
        assert_eq!(e.windows().count(), 3);
        e.close_window().unwrap();
        assert_eq!(e.windows().count(), 2);
        assert_eq!(e.windows().active(), WindowId(1));
        e.only_window();
        assert_eq!(e.windows().count(), 1);
        assert_eq!(e.windows().layout(), &Layout::Window(WindowId(1)));
    }

    #[test]
    fn switching_windows_leaves_visual_mode() {
        let mut e = editor("x");
        e.split_window(SplitDirection::Vertical);
        e.toggle_visual(crate::editor::SelectionKind::Char);
        e.focus_next_window();
        assert_eq!(e.mode(), Mode::Normal);
    }
}

#[cfg(test)]
mod follow_tests {
    use super::follow_cursor;

    #[test]
    fn without_wrapping_it_keeps_the_margin_in_lines() {
        let one = |_| 1;
        assert_eq!(follow_cursor(0, (3, 0), 10, 100, 2, one), 0);
        assert_eq!(follow_cursor(0, (9, 0), 10, 100, 2, one), 2);
        assert_eq!(follow_cursor(50, (40, 0), 10, 100, 2, one), 38);
        assert_eq!(follow_cursor(0, (99, 0), 10, 100, 2, one), 90, "at the end");
    }

    #[test]
    fn wrapped_lines_count_all_their_rows() {
        // Every line is 3 rows; 10 rows show 3 lines and a row of a fourth.
        let three = |_| 3;
        assert_eq!(
            follow_cursor(0, (3, 0), 10, 100, 0, three),
            0,
            "the 10th row"
        );
        assert_eq!(follow_cursor(0, (3, 2), 10, 100, 0, three), 1, "the 12th");
        assert_eq!(follow_cursor(5, (5, 2), 10, 100, 0, three), 5);
        assert_eq!(
            follow_cursor(0, (500, 0), 10, 1000, 0, three),
            497,
            "a long jump"
        );
    }
}
