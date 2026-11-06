//! Recording the last change for `.` (repeat).
//!
//! The app records the keys of every command sequence that starts and ends
//! in normal mode with nothing pending: `dw`, `ct)foo<Esc>`, `vjd`. A
//! sequence that edited the buffer becomes the last change, and `.` feeds
//! its keys back through the normal input path. Recording keys rather than
//! commands means every way of changing text is repeatable, including
//! insert sessions and future plugin commands, with no extra work.

use crate::editor::DocumentId;
use crate::input::{KeyCode, KeyPress};

/// The active buffer and how many edits it has had. Comparing two stamps
/// tells whether a key sequence edited text: a sequence that ends in another
/// buffer (`]b`, `:e`) didn't, even though the edit counts differ.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EditStamp {
    pub document: DocumentId,
    pub edits: u64,
}

impl EditStamp {
    /// Whether the same buffer is active and has been edited since `earlier`.
    pub fn edited_since(self, earlier: EditStamp) -> bool {
        self.document == earlier.document && self.edits != earlier.edits
    }
}

/// The last change, split so that `.` with a count can replace its count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastChange {
    /// A `"x` register prefix, kept on replay.
    register: Vec<KeyPress>,
    /// The count typed before the change (`3` in `3x`), if any.
    count: Option<usize>,
    /// Everything after the leading register and count.
    body: Vec<KeyPress>,
}

impl LastChange {
    /// Splits recorded keys into leading register/count and the rest.
    pub fn parse(keys: &[KeyPress]) -> Self {
        let mut register = Vec::new();
        let mut count: Option<usize> = None;
        let mut i = 0;
        while i < keys.len() {
            let key = keys[i];
            match key.as_text() {
                Some('"') if i + 1 < keys.len() => {
                    register.extend_from_slice(&keys[i..i + 2]);
                    i += 2;
                }
                Some(c) if c.is_ascii_digit() && (c != '0' || count.is_some()) => {
                    let digit = c.to_digit(10).expect("ascii digit") as usize;
                    count = Some(count.unwrap_or(0).saturating_mul(10).saturating_add(digit));
                    i += 1;
                }
                _ => break,
            }
        }
        Self {
            register,
            count,
            body: keys[i..].to_vec(),
        }
    }

    /// The keys after the register and count (`p` in `"13p`).
    pub fn body(&self) -> &[KeyPress] {
        &self.body
    }

    /// Moves a `"1`-`"8` register prefix on to the next number, so `"1p`
    /// then `.` `.` pastes `"1`, `"2`, `"3` (Vim's way of going back
    /// through older deletes).
    pub fn advance_numbered_register(&mut self) {
        if let [_, key] = self.register.as_mut_slice()
            && let Some(digit @ '1'..='8') = key.as_text()
        {
            *key = KeyPress::plain(KeyCode::Char((digit as u8 + 1) as char));
        }
    }

    /// The keys to replay. A `count` replaces the original one.
    pub fn keys(&self, count: Option<usize>) -> Vec<KeyPress> {
        let count = count
            .or(self.count)
            .map(|n| n.to_string())
            .unwrap_or_default();
        let digits = count.chars().map(|c| KeyPress::plain(KeyCode::Char(c)));
        self.register
            .iter()
            .copied()
            .chain(digits)
            .chain(self.body.iter().copied())
            .collect()
    }
}

/// Watches key sequences and keeps the last one that changed the text.
#[derive(Debug, Default)]
pub struct ChangeRecorder {
    keys: Vec<KeyPress>,
    at_start: EditStamp,
    /// `:` commands aren't repeatable, as in Vim.
    used_command_line: bool,
    /// The sequence was `.` itself, which must not replace the last change.
    replayed: bool,
    last_change: Option<LastChange>,
}

impl ChangeRecorder {
    /// Starts a new sequence.
    pub fn begin(&mut self, stamp: EditStamp) {
        self.keys.clear();
        self.at_start = stamp;
        self.used_command_line = false;
        self.replayed = false;
    }

    pub fn push(&mut self, key: KeyPress) {
        self.keys.push(key);
    }

    pub fn note_command_line(&mut self) {
        self.used_command_line = true;
    }

    pub fn note_replay(&mut self) {
        self.replayed = true;
    }

    /// Ends the sequence. It becomes the last change if it edited the text.
    pub fn finish(&mut self, stamp: EditStamp) {
        let changed = stamp.edited_since(self.at_start);
        if changed && !self.used_command_line && !self.replayed && !self.keys.is_empty() {
            self.last_change = Some(LastChange::parse(&self.keys));
        }
        self.keys.clear();
    }

    #[cfg(test)]
    pub fn last_change(&self) -> Option<&LastChange> {
        self.last_change.as_ref()
    }

    pub fn last_change_mut(&mut self) -> Option<&mut LastChange> {
        self.last_change.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::parse_keys;

    fn keys(notation: &str) -> Vec<KeyPress> {
        parse_keys(notation).unwrap()
    }

    fn replayed(change: &LastChange, count: Option<usize>) -> Vec<KeyPress> {
        change.keys(count)
    }

    #[test]
    fn parse_splits_count_and_body() {
        let change = LastChange::parse(&keys("12dw"));
        assert_eq!(change.count, Some(12));
        assert_eq!(change.body, keys("dw"));
    }

    #[test]
    fn leading_zero_is_a_motion_not_a_count() {
        let change = LastChange::parse(&keys("0D"));
        assert_eq!(change.count, None);
        assert_eq!(change.body, keys("0D"));
    }

    #[test]
    fn register_is_kept_either_side_of_the_count() {
        for notation in ["\"a2dd", "2\"add"] {
            let change = LastChange::parse(&keys(notation));
            assert_eq!(change.register, keys("\"a"), "{notation}");
            assert_eq!(change.count, Some(2), "{notation}");
            assert_eq!(change.body, keys("dd"), "{notation}");
        }
    }

    #[test]
    fn replay_uses_original_or_replacement_count() {
        let change = LastChange::parse(&keys("\"a3x"));
        assert_eq!(replayed(&change, None), keys("\"a3x"));
        assert_eq!(replayed(&change, Some(5)), keys("\"a5x"));

        let change = LastChange::parse(&keys("x"));
        assert_eq!(replayed(&change, None), keys("x"));
        assert_eq!(replayed(&change, Some(10)), keys("10x"));
    }

    #[test]
    fn digits_after_the_command_are_body() {
        let change = LastChange::parse(&keys("i42<Esc>"));
        assert_eq!(change.count, None);
        assert_eq!(change.body, keys("i42<Esc>"));
    }

    /// `edits` edits into buffer 0.
    fn stamp(edits: u64) -> EditStamp {
        EditStamp {
            document: DocumentId(0),
            edits,
        }
    }

    fn record(recorder: &mut ChangeRecorder, notation: &str, edits_before: u64, edits_after: u64) {
        recorder.begin(stamp(edits_before));
        for key in keys(notation) {
            recorder.push(key);
        }
        recorder.finish(stamp(edits_after));
    }

    #[test]
    fn switching_buffers_is_not_a_change() {
        let mut recorder = ChangeRecorder::default();
        record(&mut recorder, "x", 0, 1);
        recorder.begin(stamp(1));
        recorder.push(keys("]")[0]);
        recorder.push(keys("b")[0]);
        recorder.finish(EditStamp {
            document: DocumentId(1),
            edits: 5,
        });
        assert_eq!(recorder.last_change(), Some(&LastChange::parse(&keys("x"))));
    }

    #[test]
    fn only_sequences_that_edit_are_kept() {
        let mut recorder = ChangeRecorder::default();
        record(&mut recorder, "dw", 0, 1);
        record(&mut recorder, "j", 1, 1);
        record(&mut recorder, "u", 1, 1);
        assert_eq!(
            recorder.last_change(),
            Some(&LastChange::parse(&keys("dw")))
        );
    }

    #[test]
    fn command_line_and_replays_are_not_kept() {
        let mut recorder = ChangeRecorder::default();
        record(&mut recorder, "x", 0, 1);

        recorder.begin(stamp(1));
        recorder.push(keys(":")[0]);
        recorder.note_command_line();
        recorder.finish(stamp(2));

        recorder.begin(stamp(2));
        recorder.push(keys(".")[0]);
        recorder.note_replay();
        recorder.finish(stamp(3));

        assert_eq!(recorder.last_change(), Some(&LastChange::parse(&keys("x"))));
    }

    #[test]
    fn numbered_registers_advance() {
        let mut change = LastChange::parse(&keys("\"1p"));
        change.advance_numbered_register();
        assert_eq!(change.keys(None), keys("\"2p"));
        let mut change = LastChange::parse(&keys("\"9p"));
        change.advance_numbered_register();
        assert_eq!(change.keys(None), keys("\"9p"), "9 is the last");
        let mut change = LastChange::parse(&keys("\"ap"));
        change.advance_numbered_register();
        assert_eq!(change.keys(None), keys("\"ap"));
    }
}
