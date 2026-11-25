//! Recording the last change for `.` (repeat).
//!
//! The app records the keys of every command sequence that starts and ends
//! in normal mode with nothing pending: `dw`, `ct)foo<Esc>`, `vjd`. A
//! sequence that edited the buffer becomes the last change, and `.` feeds
//! its keys back through the normal input path. Recording keys rather than
//! commands means every way of changing text is repeatable, including
//! insert sessions and future plugin commands, with no extra work.
//!
//! A change made by an operator on a visual selection is the exception: as
//! in Vim, `.` acts on the same *amount* of text from the cursor rather
//! than replaying the keys that made the selection. The recorder notes
//! where the operator's key binding began and the selection's shape, keeps
//! only the keys from there on, and `.` selects that shape first.
//!
//! `app/repeat.rs` records the keys of each command sequence that starts and
//! ends idle (normal mode, nothing partially typed): `dw`, `ct)foo<Esc>`,
//! `vjd`. If `Document::edit_count` moved during it, the sequence becomes the
//! last change. Undo/redo don't count as edits, and sequences that went through
//! the `:` line are skipped. `.` is an ordinary command that sets
//! `Editor::request_repeat`; the app then feeds the recorded keys back through
//! the normal input path. That's why recording happens at the app level: only
//! the app sees keys.
//!
//! Recording keys rather than commands makes every change repeatable, including
//! insert sessions and future plugin commands, without each command having to
//! support it. Each recorded key notes whether the input layer took it as a
//! count (`InputHandler::took_count`, in normal and operator-pending mode
//! only), since only it can tell the `2` of `d2w` from the `2` of `i2` or `f2`.
//! So `3.` drops every count digit and puts `3` first (`d2w` becomes `3dw`, as
//! in Vim), and keeps the new count for later `.`. A `"x` register prefix is
//! kept. A sequence counts as a change when it edited the buffer it started in
//! (`EditStamp`: buffer and edit count), so switching buffers never does. If
//! the change was a paste from `"1`-`"8`, `.` moves the prefix on one number
//! first, so `"1pu.u.` steps back through older deletes.
//!
//! A change made by an operator on a visual selection is repeated on the same
//! *amount* of text from the cursor, as in Vim (`visual-repeat`), not by
//! replaying the motion: after `ved` on a two-letter word, `.` deletes two
//! characters wherever it's used. Taking a selection (`take_selection`,
//! `take_block`, block `I`/`A`) notes its `VisualShape`: a number of characters
//! on one line, a number of lines and the end column on the last, a number of
//! lines, or a block's rows and display width (and `$`). The app collects it
//! after each key (`Editor::take_visual_shape`), and the recorder pairs it with
//! where that key binding began (the start of `"ad` or `3>`), which it tracks
//! by noting each key that arrives with nothing half typed. The last change
//! then keeps only the keys from there on, and `.` first calls
//! `Editor::select_shape`, which selects that much from the cursor, cut short
//! at a line's or the buffer's end. A count given to `.` is ignored for these,
//! since the size comes from the selection. A count on a visual `>` or `<`
//! shifts that many times.
//!
//! A completion is repeated by its text, not its keys, as in Vim: `.` after
//! `ifo<C-n><Esc>` puts in the word chosen, not whatever `fo` completes to
//! now (and `<Tab>` taking a server's item doesn't become a tab). After each
//! key the app takes the editor's last completion step
//! (`Editor::take_completed`), and the recorder swaps the keys of that key
//! binding (from where it began, so `<C-x><C-f>` goes too) for a
//! `<Completed>` block (`completion_keys`): `<BS>` for each character taken
//! out, the text as in a paste, `<Left>` for each character the cursor ends
//! before its end, then `<PasteEnd>`. A step straight after another
//! (`<C-n><C-n>`, `<C-y>`) replaces its block, keeping what the first took
//! out. On replay the app collects the block like a paste and hands it to
//! `Editor::put_completed` (`completed_from_keys`), so key bindings,
//! auto-pairs and auto-indent don't touch it.

use crate::editor::{Completed, DocumentId, VisualShape};
use crate::input::{KeyCode, KeyPress, paste_keys};

/// A completion step as keys, for `.` (see the module docs).
pub fn completion_keys(completed: &Completed) -> Vec<KeyPress> {
    let mut keys = vec![KeyPress::plain(KeyCode::Completed)];
    keys.extend(std::iter::repeat_n(
        KeyPress::plain(KeyCode::Backspace),
        completed.removed,
    ));
    let text = paste_keys(&completed.text);
    // Without the paste's own markers.
    keys.extend_from_slice(&text[1..text.len() - 1]);
    let back = completed
        .text
        .chars()
        .count()
        .saturating_sub(completed.cursor);
    keys.extend(std::iter::repeat_n(KeyPress::plain(KeyCode::Left), back));
    keys.push(KeyPress::plain(KeyCode::PasteEnd));
    keys
}

/// The completion step in the keys between `<Completed>` and `<PasteEnd>`.
pub fn completed_from_keys(keys: &[KeyPress]) -> Completed {
    let removed = keys
        .iter()
        .take_while(|k| k.code == KeyCode::Backspace)
        .count();
    let back = keys[removed..]
        .iter()
        .rev()
        .take_while(|k| k.code == KeyCode::Left)
        .count();
    let text: String = keys[removed..keys.len() - back]
        .iter()
        .filter_map(KeyPress::as_pasted)
        .collect();
    let cursor = text.chars().count() - back.min(text.chars().count());
    Completed {
        removed,
        text,
        cursor,
    }
}

/// Where a recorded completion block is in the keys, and how many
/// characters the completion took out first.
#[derive(Debug, Clone, Copy)]
struct CompletionBlock {
    start: usize,
    end: usize,
    removed: usize,
}

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

/// A recorded key, and whether the input layer took it as (part of) a
/// count, which `.` with a count replaces. Only the input layer can tell:
/// the `2` in `d2w` is a count, the one in `i2<Esc>` or `f2` isn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedKey {
    pub key: KeyPress,
    pub count: bool,
}

/// The last change, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastChange {
    keys: Vec<RecordedKey>,
    /// The change acted on a selection of this shape, which `.` selects
    /// before replaying the keys (the operator and what followed).
    visual: Option<VisualShape>,
}

impl LastChange {
    #[cfg(test)]
    pub fn new(keys: Vec<RecordedKey>) -> Self {
        Self { keys, visual: None }
    }

    pub fn visual(&self) -> Option<VisualShape> {
        self.visual
    }

    /// Where a leading `"x` register prefix starts (counts may come before
    /// it: `2"add`).
    fn register_index(&self) -> Option<usize> {
        let i = self.keys.iter().position(|k| !k.count)?;
        (self.keys[i].key.as_text() == Some('"') && i + 1 < self.keys.len()).then_some(i)
    }

    /// The keys without counts or a leading register (`p` in `"13p`).
    pub fn body(&self) -> Vec<KeyPress> {
        let skip = self.register_index().map_or(0, |i| i + 2);
        self.keys
            .iter()
            .enumerate()
            .filter(|&(i, k)| !k.count && i >= skip)
            .map(|(_, k)| k.key)
            .collect()
    }

    /// Moves a `"1`-`"8` register prefix on to the next number, so `"1p`
    /// then `.` `.` pastes `"1`, `"2`, `"3` (Vim's way of going back
    /// through older deletes).
    pub fn advance_numbered_register(&mut self) {
        let Some(i) = self.register_index() else {
            return;
        };
        let name = &mut self.keys[i + 1].key;
        if let Some(digit @ '1'..='8') = name.as_text() {
            *name = KeyPress::plain(KeyCode::Char((digit as u8 + 1) as char));
        }
    }

    /// Replaces every count in the change with `count`, for this and later
    /// replays.
    pub fn set_count(&mut self, count: usize) {
        let digits = count.to_string();
        let digits = digits.chars().map(|c| RecordedKey {
            key: KeyPress::plain(KeyCode::Char(c)),
            count: true,
        });
        let rest = self.keys.iter().copied().filter(|k| !k.count);
        self.keys = digits.chain(rest).collect();
    }

    /// The keys to replay. A `count` replaces every count in the change,
    /// before the operator or after it, as in Vim: after `d2w`, `3.` is
    /// `3dw`, deleting three words.
    pub fn keys(&self, count: Option<usize>) -> Vec<KeyPress> {
        let Some(count) = count else {
            return self.keys.iter().map(|k| k.key).collect();
        };
        let digits = count
            .to_string()
            .chars()
            .map(|c| KeyPress::plain(KeyCode::Char(c)))
            .collect::<Vec<_>>();
        digits
            .into_iter()
            .chain(self.keys.iter().filter(|k| !k.count).map(|k| k.key))
            .collect()
    }
}

/// Watches key sequences and keeps the last one that changed the text.
#[derive(Debug, Default)]
pub struct ChangeRecorder {
    keys: Vec<RecordedKey>,
    at_start: EditStamp,
    /// `:` commands aren't repeatable, as in Vim.
    used_command_line: bool,
    /// The sequence was `.` itself, which must not replace the last change.
    replayed: bool,
    /// Where the key binding being typed began, in `keys`.
    binding_start: usize,
    /// An operator took a selection of this shape, in the binding that
    /// began at this index.
    visual: Option<(VisualShape, usize)>,
    /// The last completion block put in the keys.
    completion: Option<CompletionBlock>,
    last_change: Option<LastChange>,
}

impl ChangeRecorder {
    /// Starts a new sequence.
    pub fn begin(&mut self, stamp: EditStamp) {
        self.keys.clear();
        self.at_start = stamp;
        self.used_command_line = false;
        self.replayed = false;
        self.binding_start = 0;
        self.visual = None;
        self.completion = None;
    }

    /// The next key starts a new key binding (nothing is half typed).
    pub fn mark_binding_start(&mut self) {
        self.binding_start = self.keys.len();
    }

    /// The binding being typed took a visual selection of `shape`.
    pub fn note_visual(&mut self, shape: VisualShape) {
        self.visual = Some((shape, self.binding_start));
    }

    /// The binding being typed made a completion step: its keys become
    /// the step's block, or replace the block just before them.
    pub fn note_completed(&mut self, completed: Completed) {
        let start = self.binding_start.min(self.keys.len());
        let (start, removed) = match self.completion {
            Some(block) if block.end == start => (block.start, block.removed),
            _ => (start, completed.removed),
        };
        self.keys.truncate(start);
        let completed = Completed {
            removed,
            ..completed
        };
        for key in completion_keys(&completed) {
            self.push(key);
        }
        self.completion = Some(CompletionBlock {
            start,
            end: self.keys.len(),
            removed,
        });
    }

    pub fn push(&mut self, key: KeyPress) {
        self.keys.push(RecordedKey { key, count: false });
    }

    /// The key just pushed was taken as a count.
    pub fn mark_count(&mut self) {
        if let Some(last) = self.keys.last_mut() {
            last.count = true;
        }
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
            let mut keys = std::mem::take(&mut self.keys);
            let visual = self.visual.map(|(shape, from)| {
                keys.drain(..from.min(keys.len()));
                shape
            });
            self.last_change = Some(LastChange { keys, visual });
        }
        self.keys.clear();
        self.visual = None;
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

    /// A change from notation in which `#` marks the next key as a count
    /// (`#2d#3w` is `2d3w` with both digits counts).
    fn change(notation: &str) -> LastChange {
        let mut recorded = Vec::new();
        let mut count = false;
        for part in notation.split_inclusive('#') {
            let (text, marks_next) = match part.strip_suffix('#') {
                Some(text) => (text, true),
                None => (part, false),
            };
            if text.is_empty() {
                count = marks_next;
                continue;
            }
            for (i, key) in keys(text).into_iter().enumerate() {
                recorded.push(RecordedKey {
                    key,
                    count: count && i == 0,
                });
            }
            count = marks_next;
        }
        LastChange::new(recorded)
    }

    #[test]
    fn replay_without_a_count_is_as_recorded() {
        assert_eq!(change("#2d#3w").keys(None), keys("2d3w"));
        assert_eq!(change("i42<Esc>").keys(None), keys("i42<Esc>"));
    }

    #[test]
    fn a_count_replaces_every_count() {
        assert_eq!(
            change("d#2w").keys(Some(3)),
            keys("3dw"),
            "after the operator"
        );
        assert_eq!(change("#2d#3w").keys(Some(5)), keys("5dw"), "both sides");
        assert_eq!(change("#1#2x").keys(Some(4)), keys("4x"), "several digits");
        assert_eq!(change("x").keys(Some(10)), keys("10x"));
    }

    #[test]
    fn digits_that_arent_counts_stay() {
        assert_eq!(change("ci2<Esc>").keys(Some(3)), keys("3ci2<Esc>"));
        assert_eq!(change("df2").keys(Some(3)), keys("3df2"));
    }

    #[test]
    fn registers_are_kept_and_found_either_side_of_a_count() {
        assert_eq!(change("\"a#2dd").keys(Some(4)), keys("4\"add"));
        assert_eq!(change("#2\"add").body(), keys("dd"));
        assert_eq!(change("\"a#2dd").body(), keys("dd"));
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
    fn recorded_counts_are_marked() {
        let mut recorder = ChangeRecorder::default();
        recorder.begin(stamp(0));
        for key in keys("d2w") {
            recorder.push(key);
            if key.as_text() == Some('2') {
                recorder.mark_count();
            }
        }
        recorder.finish(stamp(1));
        assert_eq!(recorder.last_change(), Some(&change("d#2w")));
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
        assert_eq!(recorder.last_change(), Some(&change("x")));
    }

    #[test]
    fn a_visual_change_keeps_only_the_operators_keys() {
        let mut recorder = ChangeRecorder::default();
        recorder.begin(stamp(0));
        for key in keys("vj\"ad") {
            // `"`, `a` and `d` are one binding: `a` and `d` arrive with the
            // register half typed.
            let half_typed = matches!(key.as_text(), Some('a' | 'd'));
            if !half_typed {
                recorder.mark_binding_start();
            }
            recorder.push(key);
        }
        recorder.note_visual(VisualShape::Lines(2));
        recorder.finish(stamp(1));
        let change = recorder.last_change().unwrap();
        assert_eq!(change.keys(None), keys("\"ad"));
        assert_eq!(change.visual(), Some(VisualShape::Lines(2)));

        // The next change starts without one.
        record(&mut recorder, "x", 1, 2);
        assert_eq!(recorder.last_change().unwrap().visual(), None);
    }

    #[test]
    fn only_sequences_that_edit_are_kept() {
        let mut recorder = ChangeRecorder::default();
        record(&mut recorder, "dw", 0, 1);
        record(&mut recorder, "j", 1, 1);
        record(&mut recorder, "u", 1, 1);
        assert_eq!(recorder.last_change(), Some(&change("dw")));
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

        assert_eq!(recorder.last_change(), Some(&change("x")));
    }

    #[test]
    fn numbered_registers_advance() {
        let mut c = change("\"1p");
        c.advance_numbered_register();
        assert_eq!(c.keys(None), keys("\"2p"));
        let mut c = change("#3\"1p");
        c.advance_numbered_register();
        assert_eq!(c.keys(None), keys("3\"2p"), "after a count");
        let mut c = change("\"9p");
        c.advance_numbered_register();
        assert_eq!(c.keys(None), keys("\"9p"), "9 is the last");
        let mut c = change("\"ap");
        c.advance_numbered_register();
        assert_eq!(c.keys(None), keys("\"ap"));
    }

    #[test]
    fn set_count_sticks_for_later_replays() {
        let mut c = change("#2d#3w");
        c.set_count(4);
        assert_eq!(c.keys(None), keys("4dw"));
        c.set_count(7);
        assert_eq!(c.keys(None), keys("7dw"), "the new digits are counts too");
        assert_eq!(c.body(), keys("dw"));
    }

    fn completed(removed: usize, text: &str, cursor: usize) -> Completed {
        Completed {
            removed,
            text: text.into(),
            cursor,
        }
    }

    #[test]
    fn completion_blocks_round_trip() {
        for c in [
            completed(2, "foobar", 6),
            completed(0, "call()\n\tx", 5),
            completed(3, "", 0),
        ] {
            let keys = completion_keys(&c);
            assert_eq!(keys[0].code, KeyCode::Completed);
            assert_eq!(keys.last().unwrap().code, KeyCode::PasteEnd);
            assert_eq!(completed_from_keys(&keys[1..keys.len() - 1]), c);
        }
        assert_eq!(
            completion_keys(&completed(2, "f(x)", 2)),
            keys("<Completed><BS><BS>f(x)<Left><Left><PasteEnd>")
        );
    }

    /// Records `notation` as one binding per key, except those after a
    /// `<C-x>`, which continue it.
    fn type_keys(recorder: &mut ChangeRecorder, notation: &str) {
        let mut half_typed = false;
        for key in keys(notation) {
            if !half_typed {
                recorder.mark_binding_start();
            }
            half_typed = key == keys("<C-x>")[0];
            recorder.push(key);
        }
    }

    #[test]
    fn a_completion_replaces_its_binding_and_the_one_just_before() {
        let mut recorder = ChangeRecorder::default();
        recorder.begin(stamp(0));
        type_keys(&mut recorder, "ifo<C-n>");
        recorder.note_completed(completed(2, "foobar", 6));
        type_keys(&mut recorder, "<C-n>");
        recorder.note_completed(completed(6, "foobaz", 6));
        type_keys(&mut recorder, "!<C-x><C-l>");
        recorder.note_completed(completed(7, "foobaz! x", 9));
        type_keys(&mut recorder, "<Esc>");
        recorder.finish(stamp(1));
        assert_eq!(
            recorder.last_change().unwrap().keys(None),
            keys(
                "ifo<Completed><BS><BS>foobaz<PasteEnd>!\
                 <Completed><BS><BS><BS><BS><BS><BS><BS>foobaz!<Space>x<PasteEnd><Esc>"
            ),
            "the first keeps what it took out; the third is apart from the others"
        );
    }
}
