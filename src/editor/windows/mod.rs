//! Split windows.
//!
//! A window is a view onto a buffer with its own cursor; the UI also keeps
//! a scroll position per window. Several windows can show the same buffer.
//!
//! The active window's buffer is always `Editor::document`, so commands work
//! exactly as with one window. Other windows remember which buffer they
//! show and where their cursor was; switching windows saves the active
//! window's cursor and swaps the target's buffer in.

mod layout;

pub use layout::{Area, Direction, Layout, Separator, SplitDirection};

use std::cell::Cell;

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

    fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
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
