//! The pointer's hover (`editor/mouse_hover.rs`): where it rests, and
//! when it's rested long enough (`mouse-hover-delay`) for the hover to show.

use std::time::{Duration, Instant};

use ratatui::layout::Position;

use super::App;
use crate::editor::Mode;
use crate::editor::mouse_hover::Word;

/// The pointer resting on a word, waiting to show its hover.
#[derive(Debug, Clone, Copy)]
pub struct HoverWait {
    at: crate::core::Position,
    word: Word,
    screen: Position,
    due: Instant,
    /// Ctrl (or ⌥) held: the definition's peek.
    peek: bool,
}

impl App {
    /// Whether `at` is over the hover popup.
    pub(super) fn over_hover(&self, at: Position) -> bool {
        self.editor.hover().is_some() && self.screen_map.hover.is_some_and(|area| area.contains(at))
    }

    /// The char of the active window's text under `at`, and its word, if
    /// the pointer can show a hover there.
    fn hover_target(&self, at: Position) -> Option<(crate::core::Position, Word)> {
        if !self.editor.options.mouse_hover
            || !matches!(
                self.editor.mode(),
                Mode::Normal | Mode::Insert | Mode::Visual | Mode::Sidebar | Mode::Panel
            )
        {
            return None;
        }
        let id = self.editor.windows().active();
        let (_, area) = self.screen_map.windows.iter().find(|(w, _)| *w == id)?;
        if !area.contains(at) {
            return None;
        }
        let document = self.editor.document.id();
        let tab_width = self.editor.options_for(id, document).tab_width;
        let state = self.views.get(&(id, document))?;
        let position = state.char_at(&self.editor.document, at.x, at.y, tab_width)?;
        let line = self.editor.document.buffer().line(position.row);
        let word = Word::at(&line, position)?;
        Some((position, word))
    }

    /// The pointer moved to `at`: its hover goes if it left the word (and
    /// isn't over the popup), and a new one's waited for.
    pub(super) fn pointer_moved(&mut self, at: Position, peek: bool) {
        if self.over_hover(at) {
            return;
        }
        let target = self.hover_target(at);
        let word = target.map(|(_, w)| (w, peek));
        if word.is_some() && word == self.editor.mouse_hover_word() {
            return;
        }
        if self.editor.end_mouse_hover() {
            self.hover_changed = true;
        }
        if word.is_some() && self.hover_wait.is_some_and(|w| Some((w.word, w.peek)) == word) {
            return;
        }
        let delay = Duration::from_millis(self.editor.options.mouse_hover_delay);
        self.hover_wait = target.map(|(position, word)| HoverWait {
            at: position,
            word,
            screen: at,
            due: Instant::now() + delay,
            peek,
        });
    }

    /// When the waited-for hover's due.
    pub(super) fn hover_due(&self) -> Option<Instant> {
        self.hover_wait.map(|w| w.due)
    }

    /// The pointer's rested long enough: its hover (public so tests can
    /// skip the wait).
    pub fn show_pending_hover(&mut self) {
        let Some(wait) = self.hover_wait.take() else {
            return;
        };
        // Still the same text under it.
        let line = self.editor.document.buffer().line(wait.at.row);
        if Word::at(&line, wait.at) != Some(wait.word) {
            return;
        }
        self.editor
            .mouse_hover_at(wait.at, wait.word, (wait.screen.x, wait.screen.y), wait.peek);
    }
}
