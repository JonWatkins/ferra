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
//!
//! `editor::windows::TextLayout` (the text width beside the gutter, whether to
//! wrap, the tab width, the continued rows' indent) is the one place that knows
//! how a window lays out lines, shared by the UI and the commands:
//!
//! - Drawing (`EditorView::render`) walks lines from the view's top, drawing
//!   each of a line's rows (`draw_row`) with the line number on its first only,
//!   and clips syntax, search and selection highlights to each row's chars (a
//!   selected line break shows on the last row).
//! - `windows::follow_cursor` keeps the cursor in sight counting screen rows:
//!   `scroll-off` is in rows, the view always starts at a line's start, and a
//!   long jump starts looking a screen above the cursor rather than measuring
//!   every line on the way.
//! - `gj`/`gk` move through a line's rows, aiming for a screen column that
//!   `Editor::wanted_screen_column` remembers while the cursor stays where they
//!   put it, so a short row on the way doesn't lose it. Without wrap they're
//!   `j`/`k`.
//! - `TextLayout::bottom` (the last line at least partly shown) is what `L` and
//!   the scrolling commands use.
//!
//! `j`/`k` still move by lines, as in Vim, and `<C-d>` and friends scroll by
//! lines too.

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
    /// Its tabs, as VS Code's editor groups: the buffers opened in it, in
    /// order (`window_tabs` gives them, tidied).
    pub tabs: Vec<DocumentId>,
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
    /// Rows at the top of each window for its tabs and breadcrumbs (set by
    /// the UI).
    chrome: Cell<u16>,
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
                tabs: vec![DocumentId(0)],
            }],
            active: WindowId(0),
            last_id: 0,
            area: Cell::new(DEFAULT_AREA),
            tops: RefCell::default(),
            requested_tops: RefCell::default(),
            chrome: Cell::new(0),
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

    /// Every window, in the order they were made.
    pub fn list(&self) -> &[Window] {
        &self.windows
    }

    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Records where the windows are drawn (the UI calls this).
    pub fn note_area(&self, area: Area) {
        self.area.set(area);
    }

    /// Each window's current size, as last drawn (its tab and breadcrumb
    /// rows included).
    pub fn sizes(&self) -> Vec<(WindowId, Area)> {
        self.layout.arrange(self.area.get()).0
    }

    /// Records how many rows each window's tabs and breadcrumbs take (the
    /// UI calls this).
    pub fn note_chrome(&self, rows: u16) {
        self.chrome.set(rows);
    }

    /// The rows a window `height` rows tall gives its tabs and breadcrumbs:
    /// none if it's too short to spare them.
    pub fn chrome_for(&self, height: u16) -> u16 {
        let rows = self.chrome.get();
        if height > rows + 2 { rows } else { 0 }
    }

    /// How many rows of text window `id` shows.
    pub fn height(&self, id: WindowId) -> usize {
        self.sizes()
            .into_iter()
            .find(|(window, _)| *window == id)
            .map_or(0, |(_, area)| {
                (area.height - self.chrome_for(area.height)) as usize
            })
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
    /// Continued rows of a wrapped line start at its indentation.
    pub wrap_indent: bool,
    /// The width of the marker before continued rows (`wrap-marker`).
    pub marker_width: usize,
}

impl TextLayout {
    /// The layout of `document` in a window `window_width` cells wide.
    pub fn new(
        window_width: usize,
        document: &Document,
        options: &crate::config::EditorOptions,
    ) -> Self {
        let gutter = gutter_width(document.buffer().line_count(), options.line_numbers);
        Self::for_text(window_width.saturating_sub(gutter as usize), options)
    }

    /// The layout of text `width` cells wide (beside any gutter).
    pub fn for_text(width: usize, options: &crate::config::EditorOptions) -> Self {
        Self {
            width,
            wrap: options.wrap,
            tab_width: options.tab_width,
            wrap_indent: options.wrap_indent,
            marker_width: unicode_width::UnicodeWidthStr::width(options.wrap_marker.as_str()),
        }
    }

    /// How far in a wrapped `line`'s continued rows start: its indentation
    /// (with `wrap-indent`) and the marker. Nothing if that would leave
    /// less than half the width for text.
    pub fn continued(&self, line: &str) -> usize {
        let indent = if self.wrap_indent {
            let rest = line.trim_start_matches([' ', '\t']);
            crate::core::columns::display_width(&line[..line.len() - rest.len()], self.tab_width)
        } else {
            0
        };
        let offset = indent + self.marker_width;
        if offset * 2 > self.width {
            self.marker_width.min(self.width / 2)
        } else {
            offset
        }
    }

    /// How far in row `part` of `line` starts on screen.
    pub fn offset(&self, line: &str, part: usize) -> usize {
        if self.wrap && part > 0 {
            self.continued(line)
        } else {
            0
        }
    }

    /// The char column each screen row of `line` starts at.
    pub fn row_starts(&self, line: &str) -> Vec<usize> {
        if self.wrap {
            crate::core::wrap::row_starts_indented(
                line,
                self.width,
                self.tab_width,
                self.continued(line),
            )
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
        let folded = document.closed_fold_at(cursor.row).is_some();
        let row_in_line = if self.wrap && !folded {
            // With virtual text, the line as it's drawn.
            match document.display_line(cursor.row) {
                Some(display) => crate::core::wrap::row_of(
                    &self.row_starts(&display.text),
                    display.to_display(cursor.col),
                ),
                None => crate::core::wrap::row_of(
                    &self.row_starts(&buffer.line(cursor.row)),
                    cursor.col,
                ),
            }
        } else {
            0
        };
        let top = follow_cursor(
            top,
            (cursor.row, row_in_line),
            height,
            buffer.line_count(),
            scroll_off,
            |row| self.rows_in(document, row),
        );
        // The view starts at a fold's first line, not inside it.
        document.closed_fold_at(top).map_or(top, |(first, _)| first)
    }

    /// How many screen rows line `row` of `document` takes: a closed fold
    /// takes one, for its first line, and its other lines none.
    pub fn rows_in(&self, document: &Document, row: usize) -> usize {
        match document.closed_fold_at(row) {
            Some((first, _)) => usize::from(first == row),
            None if !self.wrap => 1,
            None => match document.display_line(row) {
                Some(display) => self.rows(&display.text),
                None => self.rows(&document.buffer().line(row)),
            },
        }
    }

    /// The last line at least partly shown in a view `height` rows tall
    /// starting at `top` (a closed fold's last line, if it ends there).
    pub fn bottom(&self, document: &Document, top: usize, height: usize) -> usize {
        let buffer = document.buffer();
        let last = buffer.line_count() - 1;
        if !self.wrap && document.closed_folds().is_empty() {
            return (top + height)
                .saturating_sub(1)
                .min(last)
                .max(top.min(last));
        }
        let mut used = 0;
        let mut row = top.min(last);
        loop {
            used += self.rows_in(document, row);
            match document.next_visible_line(row) {
                Some(next) if used < height => row = next,
                _ => return document.closed_fold_at(row).map_or(row, |(_, end)| end),
            }
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
        // Kept within the buffer, which may have shrunk since (another
        // window's edits, a reload).
        let last = document.buffer().line_count().saturating_sub(1);
        let row = window.cursor.row.min(last);
        let col = window.cursor.col.min(document.buffer().line_len(row));
        Some((document, Position::new(row, col)))
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
        // The new window's one tab is what it shows (as VS Code's new
        // group).
        self.windows.windows.push(Window {
            id,
            document: self.document.id(),
            cursor: self.document.cursor(),
            tabs: vec![self.document.id()],
        });
        // A split starts with its window's own options, as in Vim.
        self.copy_window_options(active, id);
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
            self.refresh_options();
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

    /// Window `id`'s tabs: the open buffers in its list, with the one it
    /// shows.
    pub fn window_tabs(&self, id: WindowId) -> Vec<DocumentId> {
        let Some(window) = self.windows.get(id) else {
            return Vec::new();
        };
        let shown = if id == self.windows.active {
            self.document.id()
        } else {
            window.document
        };
        let mut tabs: Vec<DocumentId> = window
            .tabs
            .iter()
            .copied()
            .filter(|&t| self.document_by_id(t).is_some())
            .collect();
        if !tabs.contains(&shown) {
            tabs.push(shown);
        }
        tabs
    }

    /// `new` is shown in the active window, opened from `previous`: its
    /// tab goes after `previous`'s (VS Code's order), if it hasn't one.
    pub(super) fn note_tab(&mut self, previous: DocumentId, new: DocumentId) {
        let active = self.windows.active;
        let Some(window) = self.windows.get_mut(active) else {
            return;
        };
        if window.tabs.is_empty() {
            window.tabs.push(previous);
        }
        if !window.tabs.contains(&new) {
            let at = window
                .tabs
                .iter()
                .position(|&t| t == previous)
                .map_or(window.tabs.len(), |i| i + 1);
            window.tabs.insert(at, new);
        }
    }

    /// Whether a window other than the active one shows buffer `id` or
    /// has a tab for it.
    pub(super) fn in_other_window(&self, id: DocumentId) -> bool {
        self.windows
            .windows
            .iter()
            .filter(|w| w.id != self.windows.active)
            .any(|w| w.document == id || w.tabs.contains(&id))
    }

    /// Buffer `id`'s tab in the active window goes.
    pub(super) fn forget_tab_here(&mut self, id: DocumentId) {
        let active = self.windows.active;
        if let Some(window) = self.windows.get_mut(active) {
            window.tabs.retain(|&t| t != id);
        }
    }

    /// Whether buffer `id` is shown or has a tab, in any window.
    pub fn buffer_in_a_window(&self, id: DocumentId) -> bool {
        id == self.document.id()
            || self
                .windows
                .windows
                .iter()
                .any(|w| w.document == id || w.tabs.contains(&id))
    }

    /// Whether buffer `id` is what a window shows.
    pub fn shown_in_a_window(&self, id: DocumentId) -> bool {
        id == self.document.id() || self.windows.windows.iter().any(|w| w.document == id)
    }

    /// Closes buffer `id` if it isn't shown or in a tab anywhere (one
    /// opened only to be read or asked about). Returns whether it was.
    pub fn close_hidden_buffer(&mut self, id: DocumentId) -> bool {
        if self.buffer_in_a_window(id) {
            return false;
        }
        let before = self.background.len();
        self.background.retain(|d| d.id() != id);
        before != self.background.len()
    }

    /// Buffer `id` closed: its tabs go.
    pub(super) fn forget_tab(&mut self, id: DocumentId) {
        for window in &mut self.windows.windows {
            window.tabs.retain(|&t| t != id);
        }
    }

    /// The tab beside `document`'s in window `id` (before it, else after).
    fn tab_beside(&self, id: WindowId, document: DocumentId) -> Option<DocumentId> {
        let tabs = self.window_tabs(id);
        let at = tabs.iter().position(|&t| t == document)?;
        if at > 0 {
            tabs.get(at - 1).copied()
        } else {
            tabs.get(at + 1).copied()
        }
    }

    /// Whether the active buffer has a tab in another window too (so
    /// closing its tab here leaves it open).
    pub fn tab_open_elsewhere(&self) -> bool {
        let here = self.document.id();
        self.windows
            .windows
            .iter()
            .filter(|w| w.id != self.windows.active)
            .any(|w| self.window_tabs(w.id).contains(&here))
    }

    /// Closes the active buffer's tab in this window only (it's open in
    /// another): the tab beside it shows, or with none, the window closes.
    pub fn close_tab_here(&mut self) {
        let active = self.windows.active;
        let closing = self.document.id();
        let beside = self.tab_beside(active, closing);
        if let Some(window) = self.windows.get_mut(active) {
            window.tabs.retain(|&t| t != closing);
        }
        match beside {
            Some(next) => {
                self.switch_to(next);
            }
            None => {
                let _ = self.close_window();
            }
        }
    }

    /// A tab dragged along its window's tab row: `document`'s tab to
    /// position `index`.
    pub fn move_tab(&mut self, window: WindowId, document: DocumentId, index: usize) {
        let mut tabs = self.window_tabs(window);
        let Some(at) = tabs.iter().position(|&t| t == document) else {
            return;
        };
        tabs.remove(at);
        tabs.insert(index.min(tabs.len()), document);
        if let Some(w) = self.windows.get_mut(window) {
            w.tabs = tabs;
        }
    }

    /// A tab dragged to another window (its tab row at `index`, or its
    /// text: the end): it moves there and shows, the window it left
    /// showing the tab beside it, or closing if that was its only one.
    pub fn move_tab_to_window(
        &mut self,
        document: DocumentId,
        from: WindowId,
        to: WindowId,
        index: Option<usize>,
    ) {
        if from == to || self.windows.get(to).is_none() {
            return;
        }
        self.focus_window(from);
        let beside = self.tab_beside(from, document);
        if let Some(w) = self.windows.get_mut(from) {
            w.tabs.retain(|&t| t != document);
        }
        if let Some(next) = beside {
            self.switch_to(next);
        }
        self.focus_window(to);
        let mut tabs = self.window_tabs(to);
        tabs.retain(|&t| t != document);
        let index = index.unwrap_or(tabs.len()).min(tabs.len());
        tabs.insert(index, document);
        if let Some(w) = self.windows.get_mut(to) {
            w.tabs = tabs;
        }
        self.switch_to(document);
        if beside.is_none() {
            self.windows.layout.remove(from);
            self.windows.windows.retain(|w| w.id != from);
        }
    }

    /// The next or previous tab in this window (`Ctrl+Tab`), wrapping.
    pub fn switch_tab(&mut self, delta: isize) {
        let tabs = self.window_tabs(self.windows.active);
        let Some(at) = tabs.iter().position(|&t| t == self.document.id()) else {
            return;
        };
        let next = tabs[(at as isize + delta).rem_euclid(tabs.len() as isize) as usize];
        self.switch_to(next);
    }

    /// The tab to show in place of the active buffer, closing: the one
    /// beside it in this window.
    pub(super) fn tab_to_show_after_closing(&self) -> Option<DocumentId> {
        self.tab_beside(self.windows.active, self.document.id())
    }

    /// Buffer `old` closed (`new` shows in the active window): other
    /// windows that showed it show the tab beside it, or close if it was
    /// their only one (while another window's left).
    pub(super) fn replace_window_document(&mut self, old: DocumentId, new: DocumentId) {
        let active = self.windows.active;
        let others: Vec<WindowId> = self
            .windows
            .windows
            .iter()
            .filter(|w| w.id != active && w.document == old)
            .map(|w| w.id)
            .collect();
        for id in others {
            let beside = self
                .windows
                .get(id)
                .and_then(|w| {
                    let at = w.tabs.iter().position(|&t| t == old)?;
                    let rest: Vec<DocumentId> =
                        w.tabs.iter().copied().filter(|&t| t != old).collect();
                    rest.get(at.saturating_sub(1)).copied()
                })
                .filter(|&t| self.document_by_id(t).is_some() && t != old);
            match beside {
                Some(tab) => {
                    if let Some(window) = self.windows.get_mut(id) {
                        window.document = tab;
                        window.cursor = Position::default();
                    }
                }
                None if self.windows.count() > 1 => {
                    self.windows.layout.remove(id);
                    self.windows.windows.retain(|w| w.id != id);
                }
                None => {
                    if let Some(window) = self.windows.get_mut(id) {
                        window.document = new;
                        window.cursor = Position::default();
                    }
                }
            }
        }
        self.forget_tab(old);
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
