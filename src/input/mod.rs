//! Turns key presses into commands.
//!
//! [`InputHandler`] accumulates register selections (`"a`), counts (`3`) and
//! multi-key sequences (`g`, `gg`) and resolves them against the [`Keymap`].
//! It decides *what* should happen; the app then performs it.

mod key;
mod keymap;

pub use key::{KeyCode, KeyPress, Modifiers, parse_keys, paste_keys};
pub use keymap::{KeyBindings, Keymap, Lookup};

use crate::editor::Mode;

/// Selects a register for the next command, as in Vim (`"ayy`).
const REGISTER_PREFIX: char = '"';

/// What the app should do in response to a key press.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Run a named command.
    Execute {
        command: String,
        count: Option<usize>,
        register: Option<char>,
        /// The key typed after a command that takes one (`fx` → `x`).
        char_arg: Option<char>,
    },
    /// Insert literal text (unbound printable key in insert/command mode).
    InsertChar(char),
    /// Waiting for more keys to complete a sequence, count or register.
    Pending,
    /// Keys for the terminal panel's shell, as typed (unbound in terminal
    /// mode, or a bound prefix that went no further).
    PassThrough(Vec<KeyPress>),
    /// The key did nothing.
    Unbound,
}

#[derive(Debug, Default)]
pub struct InputHandler {
    pending: Vec<KeyPress>,
    count: Option<usize>,
    register: Option<char>,
    awaiting_register: bool,
    /// Display text of the last completed sequence (e.g. `"a2d`).
    last_sequence: String,
    /// Keys kept on display while an operator or `f`-style command waits.
    held: String,
    /// A command waiting for its character argument (after `f`, `t`, `r`).
    awaiting_char: Option<AwaitingChar>,
    /// The last key handled was taken as a digit of a count.
    took_count: bool,
    /// VS Code's keys: a selection (visual mode) is typed over, and
    /// there are no counts or registers.
    modeless: bool,
}

#[derive(Debug)]
struct AwaitingChar {
    command: String,
    count: Option<usize>,
    register: Option<char>,
}

impl InputHandler {
    /// With VS Code's keys (`keymap = "vscode"`) or Vim's.
    pub fn set_modeless(&mut self, modeless: bool) {
        self.modeless = modeless;
    }

    fn accepts_text(&self, mode: Mode) -> bool {
        mode.accepts_text() || (self.modeless && mode == Mode::Visual)
    }

    /// Processes one key press.
    ///
    /// Usually yields a single outcome, but an abandoned sequence in
    /// insert/command mode yields several: its leading keys are inserted as
    /// text and the final key is processed afresh. So with `jk` bound in
    /// insert mode, typing `jx` inserts "jx", and `jj` inserts "j" and
    /// waits on the second `j`.
    pub fn handle(&mut self, key: KeyPress, mode: Mode, keymap: &Keymap) -> Vec<Outcome> {
        self.took_count = false;
        if let Some(waiting) = self.awaiting_char.take() {
            // Any printable key is the argument, even one bound to a command
            // (`f:` finds a colon), and so is Ctrl plus a letter, as its
            // control character (`<C-r><C-w>`). Anything else (`<Esc>`)
            // cancels.
            return match key.as_text().or_else(|| key.as_control_char()) {
                Some(c) => vec![Outcome::Execute {
                    command: waiting.command,
                    count: waiting.count,
                    register: waiting.register,
                    char_arg: Some(c),
                }],
                None => vec![Outcome::Unbound],
            };
        }

        if self.awaiting_register {
            self.awaiting_register = false;
            return match key.as_text() {
                Some(name) => {
                    self.register = Some(name);
                    vec![Outcome::Pending]
                }
                None => {
                    self.reset();
                    vec![Outcome::Unbound]
                }
            };
        }

        if !self.modeless
            && mode.accepts_register()
            && self.pending.is_empty()
            && key.as_text() == Some(REGISTER_PREFIX)
        {
            self.awaiting_register = true;
            return vec![Outcome::Pending];
        }

        if !self.modeless && mode.accepts_count()
            && self.pending.is_empty()
            && let Some(digit) = key.as_digit()
            // A leading `0` is a command (go to line start), not a count.
            && (digit != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0).saturating_mul(10);
            self.count = Some(count.saturating_add(digit));
            self.took_count = true;
            return vec![Outcome::Pending];
        }

        self.pending.push(key);
        match keymap.lookup(mode, &self.pending) {
            Lookup::Command(command) => {
                let outcome = Outcome::Execute {
                    command: command.to_string(),
                    count: self.count,
                    register: self.register,
                    char_arg: None,
                };
                self.last_sequence = self.typed_display();
                self.reset();
                vec![outcome]
            }
            Lookup::Pending => vec![Outcome::Pending],
            Lookup::NoMatch => {
                let keys = std::mem::take(&mut self.pending);
                self.reset();
                if mode == Mode::Terminal {
                    return vec![Outcome::PassThrough(keys)];
                }
                if !self.accepts_text(mode) {
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

    /// Keys of a sequence are waiting for more (`g` of `gg`), as opposed to
    /// a count, register or character argument, which wait forever.
    pub fn has_pending_keys(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The key timeout passed with a sequence half typed. If the keys so far
    /// are bound on their own (`g` with `gg` also bound), that runs. In
    /// insert and command mode the first key is typed as text and the rest
    /// are looked up again, so with `jk` bound, `j` then a pause types "j".
    /// Otherwise (`g` in normal mode) it keeps waiting, like Vim.
    pub fn timeout(&mut self, mode: Mode, keymap: &Keymap) -> Vec<Outcome> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        if let Some(command) = keymap.command_at(mode, &self.pending) {
            let outcome = Outcome::Execute {
                command: command.to_string(),
                count: self.count,
                register: self.register,
                char_arg: None,
            };
            self.last_sequence = self.typed_display();
            self.reset();
            return vec![outcome];
        }
        if mode == Mode::Terminal {
            let keys = std::mem::take(&mut self.pending);
            self.reset();
            return vec![Outcome::PassThrough(keys)];
        }
        if !self.accepts_text(mode) {
            return Vec::new();
        }
        let keys = std::mem::take(&mut self.pending);
        self.reset();
        let (first, rest) = keys.split_first().expect("pending is non-empty");
        let mut outcomes: Vec<Outcome> = first
            .as_text()
            .map(Outcome::InsertChar)
            .into_iter()
            .collect();
        for key in rest {
            let more = self.handle(*key, mode, keymap);
            outcomes.extend(more.into_iter().filter(|o| *o != Outcome::Pending));
        }
        // The time is up for the rest too.
        outcomes.extend(self.timeout(mode, keymap));
        outcomes
    }

    /// Whether the last key handled was taken as a digit of a count (the
    /// `2` of `d2w`), as opposed to text or a character argument.
    pub fn took_count(&self) -> bool {
        self.took_count
    }

    /// Clears any partially typed register, count or key sequence.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.count = None;
        self.register = None;
        self.awaiting_register = false;
    }

    /// Keeps the last completed sequence on display, after anything already
    /// held. The app calls this when an operator waits for a motion (`d`) or
    /// a command waits for its character (`df`).
    pub fn hold_last_sequence(&mut self) {
        let last = std::mem::take(&mut self.last_sequence);
        self.held.push_str(&last);
    }

    /// Makes the next key the character argument of `command` instead of
    /// looking it up in the keymap.
    pub fn await_char(&mut self, command: String, count: Option<usize>, register: Option<char>) {
        self.awaiting_char = Some(AwaitingChar {
            command,
            count,
            register,
        });
    }

    pub fn is_awaiting_char(&self) -> bool {
        self.awaiting_char.is_some()
    }

    /// Nothing is partially typed: no keys, count, register or argument.
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
            && self.count.is_none()
            && self.register.is_none()
            && !self.awaiting_register
            && self.awaiting_char.is_none()
            && self.held.is_empty()
    }

    pub fn release_held(&mut self) {
        self.held.clear();
    }

    /// Partially typed input for display, e.g. `"a3g` or `d2`.
    pub fn pending_display(&self) -> String {
        self.held.clone() + &self.typed_display()
    }

    /// The register, count and keys typed for the current sequence only.
    fn typed_display(&self) -> String {
        let mut out = String::new();
        if let Some(name) = self.register {
            out.push(REGISTER_PREFIX);
            out.push(name);
        } else if self.awaiting_register {
            out.push(REGISTER_PREFIX);
        }
        if let Some(count) = self.count {
            out.push_str(&count.to_string());
        }
        out.extend(self.pending.iter().map(ToString::to_string));
        out
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
        map.bind(Mode::Normal, &parse_keys("p").unwrap(), "paste_after");
        map.bind(
            Mode::OperatorPending,
            &parse_keys("w").unwrap(),
            "move_word_forward",
        );
        map.bind(Mode::Sidebar, &parse_keys("j").unwrap(), "sidebar_down");
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
            register: None,
            char_arg: None,
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
    fn register_prefix_selects_register() {
        let mut h = InputHandler::default();
        let outcome = last(&mut h, Mode::Normal, "\"a2p");
        assert_eq!(
            outcome,
            Outcome::Execute {
                command: "paste_after".into(),
                count: Some(2),
                register: Some('a'),
                char_arg: None,
            }
        );
        assert_eq!(
            last(&mut h, Mode::Normal, "p"),
            execute("paste_after", None)
        );
    }

    #[test]
    fn register_prefix_after_count() {
        let mut h = InputHandler::default();
        feed(&mut h, Mode::Normal, "3\"b");
        assert_eq!(h.pending_display(), "\"b3");
    }

    #[test]
    fn register_prefix_with_non_text_key_cancels() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::Normal, "\"<Esc>"), Outcome::Unbound);
        assert_eq!(h.pending_display(), "");
    }

    #[test]
    fn quote_is_typed_in_insert_mode() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::Insert, "\""), Outcome::InsertChar('"'));
    }

    #[test]
    fn counts_work_in_operator_pending_mode() {
        let mut h = InputHandler::default();
        assert_eq!(
            last(&mut h, Mode::OperatorPending, "3w"),
            execute("move_word_forward", Some(3))
        );
    }

    #[test]
    fn unbound_keys_pass_through_to_the_terminal() {
        let mut h = InputHandler::default();
        let mut map = keymap();
        map.bind(Mode::Terminal, &parse_keys("<C-\\><C-n>").unwrap(), "leave");
        let feed_map = |h: &mut InputHandler, keys: &str| -> Vec<Outcome> {
            parse_keys(keys)
                .unwrap()
                .into_iter()
                .flat_map(|k| h.handle(k, Mode::Terminal, &map))
                .collect()
        };
        let keys = |n: &str| parse_keys(n).unwrap();
        assert_eq!(
            feed_map(&mut h, "3j"),
            [
                Outcome::PassThrough(keys("3")),
                Outcome::PassThrough(keys("j"))
            ],
            "no counts"
        );
        assert_eq!(
            feed_map(&mut h, "<C-\\><C-n>"),
            [Outcome::Pending, execute("leave", None)]
        );
        assert_eq!(
            feed_map(&mut h, "<C-\\>x"),
            [Outcome::Pending, Outcome::PassThrough(keys("<C-\\>x"))],
            "a prefix that goes no further"
        );
        feed_map(&mut h, "<C-\\>");
        assert_eq!(
            h.timeout(Mode::Terminal, &map),
            [Outcome::PassThrough(keys("<C-\\>"))]
        );
    }

    #[test]
    fn counts_work_in_the_sidebar() {
        let mut h = InputHandler::default();
        assert_eq!(
            last(&mut h, Mode::Sidebar, "3j"),
            execute("sidebar_down", Some(3))
        );
    }

    #[test]
    fn unbound_keys_in_operator_pending_mode_are_not_text() {
        let mut h = InputHandler::default();
        assert_eq!(last(&mut h, Mode::OperatorPending, "z"), Outcome::Unbound);
    }

    #[test]
    fn held_sequence_is_displayed_until_released() {
        let mut h = InputHandler::default();
        feed(&mut h, Mode::Normal, "2j");
        h.hold_last_sequence();
        feed(&mut h, Mode::OperatorPending, "3");
        assert_eq!(h.pending_display(), "2j3");
        h.release_held();
        assert_eq!(h.pending_display(), "3");
    }

    #[test]
    fn awaited_char_completes_the_command() {
        let mut h = InputHandler::default();
        h.await_char("find_char".into(), Some(2), Some('a'));
        assert!(h.is_awaiting_char());
        // `j` is bound, but here it's the argument.
        assert_eq!(
            last(&mut h, Mode::Normal, "j"),
            Outcome::Execute {
                command: "find_char".into(),
                count: Some(2),
                register: Some('a'),
                char_arg: Some('j'),
            }
        );
        assert!(!h.is_awaiting_char());
        assert_eq!(last(&mut h, Mode::Normal, "j"), execute("move_down", None));
    }

    #[test]
    fn non_text_key_cancels_awaited_char() {
        let mut h = InputHandler::default();
        h.await_char("find_char".into(), None, None);
        assert_eq!(last(&mut h, Mode::Normal, "<Esc>"), Outcome::Unbound);
        assert!(!h.is_awaiting_char());
    }

    #[test]
    fn idle_only_when_nothing_is_partially_typed() {
        let mut h = InputHandler::default();
        assert!(h.is_idle());
        feed(&mut h, Mode::Normal, "2");
        assert!(!h.is_idle());
        feed(&mut h, Mode::Normal, "j");
        assert!(h.is_idle());
        feed(&mut h, Mode::Normal, "\"a");
        assert!(!h.is_idle());
        h.reset();
        h.await_char("f".into(), None, None);
        assert!(!h.is_idle());
    }

    #[test]
    fn held_sequences_accumulate() {
        let mut h = InputHandler::default();
        feed(&mut h, Mode::Normal, "j");
        h.hold_last_sequence();
        feed(&mut h, Mode::Normal, "p");
        h.hold_last_sequence();
        assert_eq!(h.pending_display(), "jp");
    }

    #[test]
    fn pending_display_shows_count_and_keys() {
        let mut h = InputHandler::default();
        feed(&mut h, Mode::Normal, "3g");
        assert_eq!(h.pending_display(), "3g");
    }

    #[test]
    fn timeout_types_a_half_finished_insert_binding_as_text() {
        let mut handler = InputHandler::default();
        assert_eq!(last(&mut handler, Mode::Insert, "j"), Outcome::Pending);
        assert!(handler.has_pending_keys());
        assert_eq!(
            handler.timeout(Mode::Insert, &keymap()),
            vec![Outcome::InsertChar('j')]
        );
        assert!(handler.is_idle());
    }

    #[test]
    fn timeout_runs_a_binding_that_is_also_a_prefix() {
        let mut map = keymap();
        map.bind(Mode::Normal, &parse_keys("g").unwrap(), "goto_file_end");
        let mut handler = InputHandler::default();
        handler.handle(KeyPress::plain(KeyCode::Char('3')), Mode::Normal, &map);
        handler.handle(KeyPress::plain(KeyCode::Char('g')), Mode::Normal, &map);
        assert_eq!(
            handler.timeout(Mode::Normal, &map),
            vec![execute("goto_file_end", Some(3))],
            "the count carries over"
        );
        assert_eq!(handler.last_sequence, "3g");
    }

    #[test]
    fn timeout_keeps_waiting_on_a_normal_mode_prefix() {
        let mut handler = InputHandler::default();
        feed(&mut handler, Mode::Normal, "g");
        assert_eq!(handler.timeout(Mode::Normal, &keymap()), vec![]);
        assert_eq!(
            last(&mut handler, Mode::Normal, "g"),
            execute("goto_file_start", None),
            "gg still completes"
        );
    }

    #[test]
    fn timeout_with_nothing_pending_does_nothing() {
        let mut handler = InputHandler::default();
        feed(&mut handler, Mode::Normal, "3");
        assert!(!handler.has_pending_keys(), "a count isn't a key sequence");
        assert_eq!(handler.timeout(Mode::Normal, &keymap()), vec![]);
    }

    #[test]
    fn timeout_retries_the_rest_of_a_longer_sequence() {
        let mut map = keymap();
        map.bind(Mode::Insert, &parse_keys("abc").unwrap(), "normal_mode");
        map.bind(Mode::Insert, &parse_keys("bcd").unwrap(), "normal_mode");
        let mut handler = InputHandler::default();
        for c in "ab".chars() {
            handler.handle(KeyPress::plain(KeyCode::Char(c)), Mode::Insert, &map);
        }
        assert_eq!(
            handler.timeout(Mode::Insert, &map),
            vec![Outcome::InsertChar('a'), Outcome::InsertChar('b')],
            "`b` starts `bcd`, but its time is up too"
        );
    }

    #[test]
    fn took_count_reports_count_digits_only() {
        let mut handler = InputHandler::default();
        feed(&mut handler, Mode::Normal, "2");
        assert!(handler.took_count());
        feed(&mut handler, Mode::Normal, "j");
        assert!(!handler.took_count());
        feed(&mut handler, Mode::Insert, "2");
        assert!(!handler.took_count(), "typed text");
        feed(&mut handler, Mode::Normal, "0");
        assert!(!handler.took_count(), "a leading 0 is a motion");
    }
}
