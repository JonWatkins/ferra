//! Popups that plugins open (`ctx:popup{...}`): a bordered window with a
//! title, styled lines and a footer, centred, by the cursor or at the top
//! right. A popup with focus (`Mode::Popup`) takes keys: its own (each runs
//! a command, usually the plugin's), then `[keys.popup]` (moving, scrolling,
//! closing). A list popup (`select = true`) has a selected line the
//! plugin's commands read with `ctx:popup_selected()`. A popup without
//! focus is shown until the next key, like the hover.

use std::cell::Cell;

use super::{Editor, Mode};

/// Where the popup goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PopupPosition {
    /// Centred over the windows.
    #[default]
    Center,
    /// By the cursor (below it, or above if there's more room).
    Cursor,
    /// At the top right of the active window, like the find box.
    TopRight,
}

impl PopupPosition {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "center" | "centre" => Ok(Self::Center),
            "cursor" => Ok(Self::Cursor),
            "top-right" | "top_right" => Ok(Self::TopRight),
            other => Err(format!(
                "Unknown popup position: {other} (center, cursor or top-right)"
            )),
        }
    }
}

/// A run of text in a line, with the theme scope to draw it with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PopupSpan {
    pub text: String,
    /// A theme scope (`"ui.popup.title"`, `"keyword"`), or the popup's text
    /// style.
    pub style: Option<String>,
}

pub type PopupLine = Vec<PopupSpan>;

#[derive(Debug, Clone, Default)]
pub struct Popup {
    /// Which popup this is, for the plugin's commands (`ctx:popup_name()`).
    pub name: String,
    pub title: String,
    pub lines: Vec<PopupLine>,
    /// Shown in the bottom border (the popup's keys, say).
    pub footer: String,
    pub position: PopupPosition,
    /// Fixed size (border included); else it fits its lines.
    pub width: Option<u16>,
    pub height: Option<u16>,
    /// A list: a selected line, moved with `j`/`k`.
    pub select: bool,
    /// Whether it takes keys (`Mode::Popup`); else it closes on the next.
    pub focus: bool,
    /// Its own keys: notation and the command each runs.
    pub keys: Vec<(String, String)>,
    /// A command to run when it's closed with `q` or `<Esc>`.
    pub on_close: Option<String>,
    pub(crate) selected: usize,
    /// The first line shown (the view keeps the selection in sight).
    pub scroll: Cell<usize>,
}

impl Popup {
    pub fn selected(&self) -> Option<usize> {
        (self.select && !self.lines.is_empty()).then_some(self.selected)
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.lines.len().saturating_sub(1));
    }

    /// Moves the selection (or, in a popup that isn't a list, scrolls).
    pub fn move_by(&mut self, delta: isize) {
        if self.select {
            let last = self.lines.len().saturating_sub(1) as isize;
            self.selected = (self.selected as isize + delta).clamp(0, last.max(0)) as usize;
        } else {
            let last = self.lines.len().saturating_sub(1) as isize;
            let top = (self.scroll.get() as isize + delta).clamp(0, last.max(0));
            self.scroll.set(top as usize);
        }
    }

    pub fn to_end(&mut self, end: bool) {
        let to = if end {
            self.lines.len() as isize
        } else {
            -(self.lines.len() as isize)
        };
        self.move_by(to);
    }

    /// The first line to show in a view `height` lines tall, keeping the
    /// selection in it.
    pub fn first_shown(&self, height: usize) -> usize {
        let mut top = self.scroll.get();
        if let Some(selected) = self.selected() {
            if selected < top {
                top = selected;
            } else if height > 0 && selected >= top + height {
                top = selected + 1 - height;
            }
        }
        top = top.min(self.lines.len().saturating_sub(height.max(1)));
        self.scroll.set(top);
        top
    }
}

impl Editor {
    pub fn popup(&self) -> Option<&Popup> {
        self.popup.as_ref()
    }

    pub fn popup_mut(&mut self) -> Option<&mut Popup> {
        self.popup.as_mut()
    }

    /// Opens `popup` (replacing any other), with focus if it takes it.
    pub fn open_popup(&mut self, popup: Popup) {
        let focus = popup.focus;
        self.popup = Some(popup);
        self.popup_keys_changed = true;
        if focus {
            self.set_mode(Mode::Popup);
        } else if self.mode() == Mode::Popup {
            self.set_mode(Mode::Normal);
        }
    }

    /// Closes the popup. Returns its `on_close` command, for `q`/`<Esc>`.
    pub fn close_popup(&mut self) -> Option<String> {
        let popup = self.popup.take()?;
        if self.mode() == Mode::Popup {
            self.set_mode(Mode::Normal);
        }
        popup.on_close
    }

    /// Whether a new popup brought its own keys (taken by the app, which
    /// binds them).
    pub fn take_popup_keys_changed(&mut self) -> bool {
        std::mem::take(&mut self.popup_keys_changed)
    }

    /// After a key: a popup without focus goes, unless the key opened it.
    pub fn end_popup_unless_new(&mut self, opened_before: bool) {
        if opened_before
            && self.mode() != Mode::Popup
            && self.popup.as_ref().is_some_and(|p| !p.focus)
        {
            self.popup = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> Popup {
        Popup {
            name: "test".into(),
            lines: (0..n)
                .map(|i| {
                    vec![PopupSpan {
                        text: format!("item {i}"),
                        style: None,
                    }]
                })
                .collect(),
            select: true,
            focus: true,
            ..Popup::default()
        }
    }

    #[test]
    fn a_list_moves_its_selection_and_scrolls_to_it() {
        let mut popup = list(10);
        assert_eq!(popup.selected(), Some(0));
        popup.move_by(3);
        popup.move_by(-1);
        assert_eq!(popup.selected(), Some(2));
        popup.to_end(true);
        assert_eq!(popup.selected(), Some(9));
        assert_eq!(popup.first_shown(4), 6);
        popup.to_end(false);
        assert_eq!(popup.first_shown(4), 0);
    }

    #[test]
    fn text_scrolls_instead() {
        let mut popup = Popup {
            select: false,
            ..list(10)
        };
        popup.move_by(2);
        assert_eq!(popup.selected(), None);
        assert_eq!(popup.first_shown(4), 2);
        popup.move_by(100);
        assert_eq!(popup.first_shown(4), 6, "the last lines fill the view");
    }

    #[test]
    fn focus_and_closing() {
        let mut editor = Editor::default();
        editor.open_popup(Popup {
            on_close: Some("bye".into()),
            ..list(2)
        });
        assert_eq!(editor.mode(), Mode::Popup);
        assert!(editor.take_popup_keys_changed());
        assert_eq!(editor.close_popup().as_deref(), Some("bye"));
        assert_eq!(editor.mode(), Mode::Normal);

        editor.open_popup(Popup {
            focus: false,
            ..list(2)
        });
        assert_eq!(editor.mode(), Mode::Normal);
        editor.end_popup_unless_new(false);
        assert!(editor.popup().is_some(), "the key that opened it");
        editor.end_popup_unless_new(true);
        assert!(editor.popup().is_none(), "the next key");
    }
}
