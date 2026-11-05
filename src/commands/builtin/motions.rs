//! Motions: commands that move the cursor or, when an operator is pending,
//! define the range it acts on (`w` moves; `dw` deletes a word).
//!
//! Each motion is described as data ([`Motion`]), so one definition serves
//! both uses and new motions only need a target function.

use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::{Buffer, MotionKind, Position, Range, motion};
use crate::editor::{Editor, Operator};

/// Computes a motion's target from the cursor, given an optional count.
type TargetFn = fn(&Buffer, Position, Option<usize>) -> Position;

pub struct Motion {
    pub name: &'static str,
    pub description: &'static str,
    pub kind: MotionKind,
    /// Vertical motions keep the remembered ("sticky") column.
    pub vertical: bool,
    pub target: TargetFn,
}

impl Motion {
    const fn new(
        name: &'static str,
        description: &'static str,
        kind: MotionKind,
        target: TargetFn,
    ) -> Self {
        Self {
            name,
            description,
            kind,
            vertical: false,
            target,
        }
    }

    const fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    /// The range this motion covers from `cursor`, for an operator.
    pub fn range(&self, buffer: &Buffer, cursor: Position, count: Option<usize>) -> Range {
        let target = (self.target)(buffer, cursor, count);
        Range::from_motion(buffer, cursor, target, self.kind)
    }
}

use MotionKind::{Exclusive, Inclusive, Linewise};

pub const LEFT: Motion = Motion::new("move_left", "Move left", Exclusive, |_, p, n| {
    p.with_col(p.col.saturating_sub(count(n)))
});
pub const RIGHT: Motion = Motion::new("move_right", "Move right", Exclusive, |b, p, n| {
    p.with_col((p.col + count(n)).min(b.line_len(p.row)))
});
pub const UP: Motion = Motion::new("move_up", "Move up", Linewise, |_, p, n| {
    Position::new(p.row.saturating_sub(count(n)), p.col)
})
.vertical();
pub const DOWN: Motion = Motion::new("move_down", "Move down", Linewise, |b, p, n| {
    Position::new((p.row + count(n)).min(last_row(b)), p.col)
})
.vertical();
pub const WORD_FORWARD: Motion = Motion::new(
    "move_word_forward",
    "Move to start of next word",
    Exclusive,
    |b, p, n| repeat(b, p, n, motion::word_forward),
);
pub const WORD_BACKWARD: Motion = Motion::new(
    "move_word_backward",
    "Move to start of previous word",
    Exclusive,
    |b, p, n| repeat(b, p, n, motion::word_backward),
);
pub const WORD_END: Motion = Motion::new(
    "move_word_end",
    "Move to end of word",
    Inclusive,
    |b, p, n| repeat(b, p, n, motion::word_end),
);
pub const LINE_START: Motion = Motion::new(
    "goto_line_start",
    "Move to start of line",
    Exclusive,
    |_, p, _| p.with_col(0),
);
pub const LINE_END: Motion = Motion::new(
    "goto_line_end",
    "Move to end of line ([count]-1 lines down)",
    Exclusive,
    |b, p, n| b.line_end((p.row + count(n) - 1).min(last_row(b))),
);
pub const FIRST_NON_BLANK: Motion = Motion::new(
    "goto_first_non_blank",
    "Move to first non-blank character of line",
    Exclusive,
    |b, p, _| p.with_col(motion::first_non_blank(b, p.row)),
);
pub const FILE_START: Motion = Motion::new(
    "goto_file_start",
    "Go to first line, or line [count]",
    Linewise,
    |b, _, n| line_start(b, n.map_or(0, |n| n.saturating_sub(1))),
);
pub const FILE_END: Motion = Motion::new(
    "goto_file_end",
    "Go to last line, or line [count]",
    Linewise,
    |b, _, n| line_start(b, n.map_or(last_row(b), |n| n.saturating_sub(1))),
);

/// `cw`: like `ce`, not `dw` (Vim special case). Not bound directly.
const CHANGE_WORD: Motion = Motion::new("change_word", "", Inclusive, |b, p, n| {
    let first = motion::current_word_end(b, p);
    repeat(b, first, Some(count(n) - 1), motion::word_end)
});

pub const ALL: &[Motion] = &[
    LEFT,
    RIGHT,
    UP,
    DOWN,
    WORD_FORWARD,
    WORD_BACKWARD,
    WORD_END,
    LINE_START,
    LINE_END,
    FIRST_NON_BLANK,
    FILE_START,
    FILE_END,
];

pub fn register(r: &mut CommandRegistry) {
    for motion in ALL {
        r.register(motion.name, motion.description, move |cx| run(cx, motion));
    }
    r.register("goto_line", "Go to the line given as argument", |cx| {
        let line: usize = cx
            .args
            .first()
            .ok_or("goto_line: missing line number")?
            .parse()
            .map_err(|_| "goto_line: invalid line number")?;
        let target = line_start(cx.editor.document.buffer(), line.saturating_sub(1));
        cx.editor.document.set_cursor(target);
        Ok(())
    });
}

/// Runs a motion: applies the pending operator over it, or moves the cursor.
fn run(cx: &mut Context, motion: &'static Motion) -> CommandResult {
    let Some(pending) = cx.editor.take_pending_operator() else {
        move_cursor(cx.editor, motion, cx.count);
        return Ok(());
    };

    let doc = &cx.editor.document;
    let cursor = doc.cursor();
    let on_word = doc
        .buffer()
        .char_at(cursor)
        .is_some_and(|c| !c.is_whitespace());
    let motion =
        if pending.operator == Operator::Change && motion.name == WORD_FORWARD.name && on_word {
            &CHANGE_WORD
        } else {
            motion
        };

    let count = multiply_counts(pending.count, cx.count);
    let range = motion.range(doc.buffer(), cursor, count);
    let register = pending.register.or(cx.register);
    cx.editor.apply_operator(pending.operator, range, register)
}

fn move_cursor(editor: &mut Editor, motion: &Motion, count: Option<usize>) {
    let doc = &mut editor.document;
    let cursor = doc.cursor();
    let target = (motion.target)(doc.buffer(), cursor, count);
    if motion.vertical {
        doc.move_vertical(target.row as isize - cursor.row as isize);
    } else {
        doc.set_cursor(target);
    }
}

/// `2d3w` deletes six words: counts before and after the operator multiply.
pub fn multiply_counts(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(1).saturating_mul(b.unwrap_or(1))),
    }
}

fn count(n: Option<usize>) -> usize {
    n.unwrap_or(1).max(1)
}

fn last_row(buffer: &Buffer) -> usize {
    buffer.line_count() - 1
}

/// `row` (clamped) at its first non-blank character.
fn line_start(buffer: &Buffer, row: usize) -> Position {
    let row = row.min(last_row(buffer));
    Position::new(row, motion::first_non_blank(buffer, row))
}

fn repeat(
    buffer: &Buffer,
    start: Position,
    n: Option<usize>,
    step: fn(&Buffer, Position) -> Position,
) -> Position {
    (0..n.unwrap_or(1)).fold(start, |p, _| step(buffer, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::builtin::test_util::{editor, run as run_cmd, run_with_args};
    use crate::editor::Mode;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn move_right_stops_on_last_char_in_normal_mode() {
        let mut e = editor("abc", pos(0, 0), Mode::Normal);
        run_cmd(&mut e, "move_right", Some(10));
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn move_right_can_reach_line_end_in_insert_mode() {
        let mut e = editor("abc", pos(0, 0), Mode::Insert);
        run_cmd(&mut e, "move_right", Some(10));
        assert_eq!(e.document.cursor(), pos(0, 3));
    }

    #[test]
    fn move_left_respects_count() {
        let mut e = editor("abcdef", pos(0, 5), Mode::Normal);
        run_cmd(&mut e, "move_left", Some(3));
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn vertical_moves_keep_sticky_column() {
        let mut e = editor("abcdef\nab\nabcdef", pos(0, 4), Mode::Normal);
        run_cmd(&mut e, "move_down", None);
        assert_eq!(e.document.cursor(), pos(1, 1));
        run_cmd(&mut e, "move_down", None);
        assert_eq!(e.document.cursor(), pos(2, 4));
        run_cmd(&mut e, "move_up", Some(2));
        assert_eq!(e.document.cursor(), pos(0, 4));
    }

    #[test]
    fn word_motions_respect_count() {
        let mut e = editor("one two three", pos(0, 0), Mode::Normal);
        run_cmd(&mut e, "move_word_forward", Some(2));
        assert_eq!(e.document.cursor(), pos(0, 8));
        run_cmd(&mut e, "move_word_backward", None);
        assert_eq!(e.document.cursor(), pos(0, 4));
        run_cmd(&mut e, "move_word_end", None);
        assert_eq!(e.document.cursor(), pos(0, 6));
    }

    #[test]
    fn line_start_end_and_first_non_blank() {
        let mut e = editor("  hello", pos(0, 4), Mode::Normal);
        run_cmd(&mut e, "goto_line_end", None);
        assert_eq!(e.document.cursor(), pos(0, 6));
        run_cmd(&mut e, "goto_line_start", None);
        assert_eq!(e.document.cursor(), pos(0, 0));
        run_cmd(&mut e, "goto_first_non_blank", None);
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn line_end_with_count_goes_down() {
        let mut e = editor("ab\ncde", pos(0, 0), Mode::Normal);
        run_cmd(&mut e, "goto_line_end", Some(2));
        assert_eq!(e.document.cursor(), pos(1, 2));
    }

    #[test]
    fn goto_file_start_and_end_with_counts() {
        let mut e = editor("a\nb\n  c", pos(1, 0), Mode::Normal);
        run_cmd(&mut e, "goto_file_end", None);
        assert_eq!(e.document.cursor(), pos(2, 2));
        run_cmd(&mut e, "goto_file_start", None);
        assert_eq!(e.document.cursor(), pos(0, 0));
        run_cmd(&mut e, "goto_file_start", Some(2));
        assert_eq!(e.document.cursor(), pos(1, 0));
        run_cmd(&mut e, "goto_file_end", Some(1));
        assert_eq!(e.document.cursor(), pos(0, 0));
    }

    #[test]
    fn goto_line_parses_argument() {
        let mut e = editor("a\nb\nc", pos(0, 0), Mode::Normal);
        run_with_args(&mut e, "goto_line", &["3"]);
        assert_eq!(e.document.cursor(), pos(2, 0));
        run_with_args(&mut e, "goto_line", &["x"]);
        assert!(e.message().unwrap().text.contains("invalid line number"));
    }

    #[test]
    fn counts_multiply() {
        assert_eq!(multiply_counts(None, None), None);
        assert_eq!(multiply_counts(Some(2), None), Some(2));
        assert_eq!(multiply_counts(None, Some(3)), Some(3));
        assert_eq!(multiply_counts(Some(2), Some(3)), Some(6));
    }

    #[test]
    fn motion_ranges_use_their_kind() {
        let b = Buffer::from_text("one two\nthree");
        let p = pos(0, 0);
        assert_eq!(
            WORD_FORWARD.range(&b, p, None),
            Range::Chars {
                start: p,
                end: pos(0, 4)
            }
        );
        assert_eq!(
            WORD_END.range(&b, p, None),
            Range::Chars {
                start: p,
                end: pos(0, 3)
            }
        );
        assert_eq!(DOWN.range(&b, p, None), Range::Lines { first: 0, last: 1 });
    }

    #[test]
    fn change_word_stays_within_current_word() {
        let b = Buffer::from_text("ab cd ef");
        let range = |p, n| CHANGE_WORD.range(&b, p, n);
        assert_eq!(
            range(pos(0, 0), None),
            Range::Chars {
                start: pos(0, 0),
                end: pos(0, 2)
            }
        );
        assert_eq!(
            range(pos(0, 1), None),
            Range::Chars {
                start: pos(0, 1),
                end: pos(0, 2)
            }
        );
        assert_eq!(
            range(pos(0, 0), Some(2)),
            Range::Chars {
                start: pos(0, 0),
                end: pos(0, 5)
            }
        );
    }

    #[test]
    fn all_motions_are_registered_with_unique_names() {
        let mut names: Vec<_> = ALL.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL.len());
        let registry = CommandRegistry::with_builtins();
        assert!(names.iter().all(|n| registry.contains(n)));
    }
}
