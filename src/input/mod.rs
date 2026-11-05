//! Turns key presses into commands.
//!
//! [`InputHandler`] accumulates counts (`3`) and multi-key sequences (`g`,
//! `gg`) and resolves them against the [`Keymap`]. It decides *what* should
//! happen; the app then performs it.

mod key;
mod keymap;

pub use key::{KeyCode, KeyPress, Modifiers, parse_keys};
pub use keymap::{KeyBindings, Keymap, Lookup};

use crate::editor::Mode;

/// What the app should do in response to a key press.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Run a named command.
    Execute {
        command: String,
        count: Option<usize>,
    },
    /// Insert literal text (unbound printable key in insert/command mode).
    InsertChar(char),
    /// Waiting for more keys to complete a sequence or count.
    Pending,
    /// The key did nothing.
    Unbound,
}

#[derive(Debug, Default)]
pub struct InputHandler {
    pending: Vec<KeyPress>,
    count: Option<usize>,
}

impl InputHandler {
    /// Processes one key press.
    ///
    /// Usually yields a single outcome, but an abandoned sequence in
    /// insert/command mode yields several: its leading keys are inserted as
    /// text and the final key is processed afresh. So with `jk` bound in
    /// insert mode, typing `jx` inserts "jx", and `jj` inserts "j" and
    /// waits on the second `j`.
    pub fn handle(&mut self, key: KeyPress, mode: Mode, keymap: &Keymap) -> Vec<Outcome> {
        if mode == Mode::Normal
            && self.pending.is_empty()
            && let Some(digit) = key.as_digit()
            // A leading `0` is a command (go to line start), not a count.
            && (digit != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0).saturating_mul(10);
            self.count = Some(count.saturating_add(digit));
            return vec![Outcome::Pending];
        }

        self.pending.push(key);
        match keymap.lookup(mode, &self.pending) {
            Lookup::Command(command) => {
                let outcome = Outcome::Execute {
                    command: command.to_string(),
                    count: self.count,
                };
                self.reset();
                vec![outcome]
            }
            Lookup::Pending => vec![Outcome::Pending],
            Lookup::NoMatch => {
                let keys = std::mem::take(&mut self.pending);
                self.reset();
                if mode == Mode::Normal {
                    // Like Vim, an invalid normal-mode sequence is discarded.
                    return vec![Outcome::Unbound];
                }

                let (last, leading) = keys.split_last().expect("pending is non-empty");
                if leading.is_empty() {
                    return vec![last.as_text().map_or(Outcome::Unbound, Outcome::InsertChar)];
                }
                let mut outcomes: Vec<Outcome> = leading
                    .iter()
                    .filter_map(|k| k.as_text().map(Outcome::InsertChar))
                    .collect();
                outcomes.extend(self.handle(*last, mode, keymap));
                outcomes
            }
        }
    }

    /// Clears any partially typed count or key sequence.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.count = None;
    }

    /// The partially typed count and keys, for display (e.g. `"3g"`).
    pub fn pending_display(&self) -> String {
        let count = self.count.map(|c| c.to_string()).unwrap_or_default();
        let keys: String = self.pending.iter().map(ToString::to_string).collect();
        count + &keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap() -> Keymap {
        let mut map = Keymap::default();
        map.bind(Mode::Normal, &parse_keys("j").unwrap(), "move_down");
        map.bind(Mode::Normal, &parse_keys("0").unwrap(), "goto_line_start");
        map.bind(Mode::Normal, &parse_keys("gg").unwrap(), "goto_file_start");
        map.bind(Mode::Insert, &parse_keys("<Esc>").unwrap(), "normal_mode");
        map.bind(Mode::Insert, &parse_keys("jk").unwrap(), "normal_mode");
        map
    }

    fn feed(handler: &mut InputHandler, mode: Mode, notation: &str) -> Vec<Outcome> {
        let map = keymap();
        parse_keys(notation)
            .unwrap()
            .into_iter()
            .flat_map(|k| handler.handle(k, mode, &map))
            .collect()
    }

    fn last(handler: &mut InputHandler, mode: Mode, notation: &str) -> Outcome {
        feed(handler, mode, notation).pop().unwrap()
    }

    fn execute(command: &str, count: Option<usize>) -> Outcome {
        Outcome::Execute {
            command: command.into(),
            count,
        }
    }

    #[test]
    fn single_key_executes() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::Normal, "j"), execute("move_down", None));
    }

    #[test]
    fn counts_are_accumulated() {
        let mut h = InputHandler::default();
        assert_eq!(
            last(&mut h, Mode::Normal, "12j"),
            execute("move_down", Some(12))
        );
    }

    #[test]
    fn zero_alone_is_a_command_but_extends_counts() {
        let mut h = InputHandler::default();
        assert_eq!(
            last(&mut h, Mode::Normal, "0"),
            execute("goto_line_start", None)
        );
        assert_eq!(
            last(&mut h, Mode::Normal, "10j"),
            execute("move_down", Some(10))
        );
    }

    #[test]
    fn sequences_wait_for_completion() {
        let mut h = InputHandler::default();
        let outcomes = feed(&mut h, Mode::Normal, "2gg");
        assert_eq!(
            outcomes,
            vec![
                Outcome::Pending,
                Outcome::Pending,
                execute("goto_file_start", Some(2))
            ]
        );
    }

    #[test]
    fn broken_sequence_resets() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::Normal, "3gx"), Outcome::Unbound);
        assert_eq!(h.pending_display(), "");
        assert_eq!(last(&mut h, Mode::Normal, "j"), execute("move_down", None));
    }

    #[test]
    fn unbound_text_is_inserted_outside_normal_mode() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::Insert, "x"), Outcome::InsertChar('x'));
        assert_eq!(last(&mut h, Mode::Insert, "5"), Outcome::InsertChar('5'));
        assert_eq!(last(&mut h, Mode::Command, "q"), Outcome::InsertChar('q'));
        assert_eq!(last(&mut h, Mode::Normal, "z"), Outcome::Unbound);
    }

    #[test]
    fn insert_mode_sequences_execute() {
        let mut h = InputHandler::default();
        let outcomes = feed(&mut h, Mode::Insert, "jk");
        assert_eq!(
            outcomes,
            vec![Outcome::Pending, execute("normal_mode", None)]
        );
    }

    #[test]
    fn abandoned_insert_sequence_is_typed() {
        let mut h = InputHandler::default();
        let outcomes = feed(&mut h, Mode::Insert, "jx");
        assert_eq!(
            outcomes,
            vec![
                Outcome::Pending,
                Outcome::InsertChar('j'),
                Outcome::InsertChar('x')
            ]
        );
    }

    #[test]
    fn abandoned_insert_sequence_replays_last_key() {
        let mut h = InputHandler::default();
        let outcomes = feed(&mut h, Mode::Insert, "jjk");
        assert_eq!(
            outcomes,
            vec![
                Outcome::Pending,
                Outcome::InsertChar('j'),
                Outcome::Pending,
                execute("normal_mode", None)
            ]
        );
        let outcomes = feed(&mut h, Mode::Insert, "j<Esc>");
        assert_eq!(outcomes.last(), Some(&execute("normal_mode", None)));
    }

    #[test]
    fn unbound_ctrl_keys_are_not_inserted() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::Insert, "<C-z>"), Outcome::Unbound);
    }

    #[test]
    fn pending_display_shows_count_and_keys() {
        let mut h = InputHandler::default();
        feed(&mut h, Mode::Normal, "3g");
        assert_eq!(h.pending_display(), "3g");
    }
}
