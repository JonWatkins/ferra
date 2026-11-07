//! `:normal`: running normal-mode keys from the command line.
//!
//! Only the app can feed keys, so the command queues them (one run per
//! line in its range, or per line `:g` visits) and the app plays the queue
//! afterwards, like a macro. The lines are tracked through the edits the
//! keys make, so deleting a line doesn't shift the rest onto the wrong
//! ones. The whole run is one undo step.

use super::App;
use crate::core::Position;
use crate::editor::{Message, NormalRun};
use crate::input::{KeyCode, KeyPress, parse_keys};

impl App {
    pub(super) fn run_normal(&mut self, runs: Vec<NormalRun>) {
        if self.editor.document.is_tracking_rows() {
            self.editor
                .set_message(Message::error("Cannot use :normal inside :normal"));
            return;
        }
        let document = self.editor.document.id();
        self.editor
            .document
            .track_rows(runs.iter().map(|run| run.at.row).collect());
        self.editor.hold_undo(true);
        self.macros.begin_playback();

        for (index, run) in runs.iter().enumerate() {
            if self.editor.document.id() != document || self.macros.failed() {
                break;
            }
            let Some(row) = self.editor.document.tracked_row(index) else {
                continue;
            };
            self.editor
                .document
                .set_cursor(Position::new(row, run.at.col));
            for key in keys_of(&run.keys) {
                self.handle_key(key);
                if self.macros.failed() {
                    break;
                }
            }
            // Like Vim, an unfinished command (still inserting, an operator
            // waiting) ends as if `<Esc>` were pressed.
            self.finish_open_paste();
            if !self.is_idle() {
                self.handle_key(KeyPress::plain(KeyCode::Esc));
            }
        }

        self.macros.end_playback();
        self.editor.stop_tracking_rows_everywhere();
        self.editor.hold_undo(false);
    }
}

/// The keys to run: key notation (`A;<Esc>j`), or if that doesn't parse
/// (`i<div>`), the text as typed.
fn keys_of(text: &str) -> Vec<KeyPress> {
    parse_keys(text).unwrap_or_else(|_| {
        text.chars()
            .map(|c| KeyPress::plain(KeyCode::Char(c)))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notation_or_literal_text() {
        assert_eq!(keys_of("A;<Esc>"), parse_keys("A;<Esc>").unwrap());
        let literal = keys_of("i<div>");
        assert_eq!(literal.len(), 6, "an unknown <...> is typed as it is");
        assert_eq!(literal[1], KeyPress::plain(KeyCode::Char('<')));
    }
}
