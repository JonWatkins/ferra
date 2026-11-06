//! Motions: commands that move the cursor or, when an operator is pending,
//! define the range it acts on (`w` moves; `dw` deletes a word).
//!
//! Each motion is described as data ([`Motion`]), so one definition serves
//! both uses and new motions only need a target function. A target function
//! returns `None` when the motion fails (`fx` with no `x` on the line), which
//! leaves the cursor where it is and cancels a pending operator.

use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::motion::{self, CharSearch};
use crate::core::{Buffer, MotionKind, Position, Range};
use crate::editor::{Editor, Operator};

/// What a motion's target function gets besides the buffer and cursor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MotionArgs {
    pub count: Option<usize>,
    /// The character for `f`/`t`/`F`/`T`.
    pub char: Option<char>,
    /// Repeating a character search with `;` or `,`.
    pub repeating: bool,
    /// A position worked out by the command (a mark's), for motions that
    /// go to one.
    pub target: Option<Position>,
}

impl MotionArgs {
    pub fn with_count(count: Option<usize>) -> Self {
        Self {
            count,
            ..Self::default()
        }
    }

    /// The count, defaulting to 1.
    fn n(&self) -> usize {
        self.count.unwrap_or(1).max(1)
    }
}

type TargetFn = fn(&Buffer, Position, MotionArgs) -> Option<Position>;

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

    /// The range this motion covers from `cursor`, for an operator, or
    /// `None` if the motion fails.
    pub fn range(&self, buffer: &Buffer, cursor: Position, args: MotionArgs) -> Option<Range> {
        let target = (self.target)(buffer, cursor, args)?;
        Some(Range::from_motion(buffer, cursor, target, self.kind))
    }
}

use MotionKind::{Exclusive, Inclusive, Linewise};

pub const LEFT: Motion = Motion::new("move_left", "Move left", Exclusive, |_, p, a| {
    Some(p.with_col(p.col.saturating_sub(a.n())))
});
pub const RIGHT: Motion = Motion::new("move_right", "Move right", Exclusive, |b, p, a| {
    Some(p.with_col((p.col + a.n()).min(b.line_len(p.row))))
});
pub const UP: Motion = Motion::new("move_up", "Move up", Linewise, |_, p, a| {
    Some(Position::new(p.row.saturating_sub(a.n()), p.col))
})
.vertical();
pub const DOWN: Motion = Motion::new("move_down", "Move down", Linewise, |b, p, a| {
    Some(Position::new((p.row + a.n()).min(last_row(b)), p.col))
})
.vertical();
pub const WORD_FORWARD: Motion = Motion::new(
    "move_word_forward",
    "Move to start of next word",
    Exclusive,
    |b, p, a| Some(repeat(b, p, a.n(), motion::word_forward)),
);
pub const WORD_BACKWARD: Motion = Motion::new(
    "move_word_backward",
    "Move to start of previous word",
    Exclusive,
    |b, p, a| Some(repeat(b, p, a.n(), motion::word_backward)),
);
pub const WORD_END: Motion = Motion::new(
    "move_word_end",
    "Move to end of word",
    Inclusive,
    |b, p, a| Some(repeat(b, p, a.n(), motion::word_end)),
);
pub const LINE_START: Motion = Motion::new(
    "goto_line_start",
    "Move to start of line",
    Exclusive,
    |_, p, _| Some(p.with_col(0)),
);
pub const LINE_END: Motion = Motion::new(
    "goto_line_end",
    "Move to end of line ([count]-1 lines down)",
    Exclusive,
    |b, p, a| Some(b.line_end((p.row + a.n() - 1).min(last_row(b)))),
);
pub const FIRST_NON_BLANK: Motion = Motion::new(
    "goto_first_non_blank",
    "Move to first non-blank character of line",
    Exclusive,
    |b, p, _| Some(p.with_col(motion::first_non_blank(b, p.row))),
);
pub const FILE_START: Motion = Motion::new(
    "goto_file_start",
    "Go to first line, or line [count]",
    Linewise,
    |b, _, a| Some(line_start(b, a.count.map_or(0, |n| n.saturating_sub(1)))),
);
pub const FILE_END: Motion = Motion::new(
    "goto_file_end",
    "Go to last line, or line [count]",
    Linewise,
    |b, _, a| {
        let row = a.count.map_or(last_row(b), |n| n.saturating_sub(1));
        Some(line_start(b, row))
    },
);
pub const MATCHING_BRACKET: Motion = Motion::new(
    "goto_matching_bracket",
    "Jump to the matching bracket (%)",
    Inclusive,
    |b, p, _| motion::matching_bracket(b, p),
);
pub const NEXT_PARAGRAPH: Motion = Motion::new(
    "goto_next_paragraph",
    "Move to the empty line after the paragraph (})",
    Exclusive,
    |b, p, a| Some(motion::paragraph_forward(b, p, a.n())),
);
pub const PREV_PARAGRAPH: Motion = Motion::new(
    "goto_prev_paragraph",
    "Move to the empty line before the paragraph ({)",
    Exclusive,
    |b, p, a| Some(motion::paragraph_backward(b, p, a.n())),
);

/// `f`, `t`, `F`, `T`: forward searches include the found character in an
/// operator's range, backward ones don't, as in Vim.
pub const FIND_CHAR_FORWARD: Motion = Motion::new(
    "find_char_forward",
    "Move to the next {char} on the line (f)",
    Inclusive,
    |b, p, a| char_search(a, true, false)?.target(b, p, a.n(), a.repeating),
);
pub const TILL_CHAR_FORWARD: Motion = Motion::new(
    "till_char_forward",
    "Move to just before the next {char} on the line (t)",
    Inclusive,
    |b, p, a| char_search(a, true, true)?.target(b, p, a.n(), a.repeating),
);
pub const FIND_CHAR_BACKWARD: Motion = Motion::new(
    "find_char_backward",
    "Move to the previous {char} on the line (F)",
    Exclusive,
    |b, p, a| char_search(a, false, false)?.target(b, p, a.n(), a.repeating),
);
pub const TILL_CHAR_BACKWARD: Motion = Motion::new(
    "till_char_backward",
    "Move to just after the previous {char} on the line (T)",
    Exclusive,
    |b, p, a| char_search(a, false, true)?.target(b, p, a.n(), a.repeating),
);

/// `'a`: the mark's line (linewise, at its first non-blank). The command
/// resolves the mark into `MotionArgs::target`. Not bound directly.
pub const MARK_LINE: Motion = Motion::new("mark_line", "", Linewise, |b, _, a| {
    let mark = a.target?;
    Some(line_start(b, mark.row))
});

/// `` `a ``: the mark's exact position (exclusive). Not bound directly.
pub const MARK_EXACT: Motion = Motion::new("mark_exact", "", Exclusive, |_, _, a| a.target);

/// `cw`: like `ce`, not `dw` (Vim special case). Not bound directly.
const CHANGE_WORD: Motion = Motion::new("change_word", "", Inclusive, |b, p, a| {
    let first = motion::current_word_end(b, p);
    Some(repeat(b, first, a.n() - 1, motion::word_end))
});

/// `dw`: if the last word moved over ends its line, stop at the end of that
/// line rather than at the next word (Vim special case). Not bound directly.
const OPERATOR_WORD_FORWARD: Motion =
    Motion::new("operator_word_forward", "", Exclusive, |b, p, a| {
        let target = repeat(b, p, a.n(), motion::word_forward);
        let only_blanks_before = b
            .line(target.row)
            .chars()
            .take(target.col)
            .all(char::is_whitespace);
        if target.row > p.row && only_blanks_before {
            Some(b.line_end(target.row - 1))
        } else {
            Some(target)
        }
    });

/// Motions bound directly, taking only a count.
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
    MATCHING_BRACKET,
    NEXT_PARAGRAPH,
    PREV_PARAGRAPH,
];

/// Motions that take a character argument.
const CHAR_SEARCHES: &[Motion] = &[
    FIND_CHAR_FORWARD,
    TILL_CHAR_FORWARD,
    FIND_CHAR_BACKWARD,
    TILL_CHAR_BACKWARD,
];

pub fn register(r: &mut CommandRegistry) {
    for motion in ALL {
        r.register(motion.name, motion.description, move |cx| {
            run(cx, motion, MotionArgs::with_count(cx.count))
        });
    }

    for motion in CHAR_SEARCHES {
        r.register(motion.name, motion.description, move |cx| {
            let ch = cx.char_arg.ok_or("missing character")?;
            let args = MotionArgs {
                count: cx.count,
                char: Some(ch),
                ..MotionArgs::default()
            };
            cx.editor.last_char_search = char_search(args, is_forward(motion), is_till(motion));
            run(cx, motion, args)
        })
        .takes_char();
    }
    r.register("repeat_char_search", "Repeat the last f/t/F/T (;)", |cx| {
        repeat_char_search(cx, false)
    });
    r.register(
        "repeat_char_search_reverse",
        "Repeat the last f/t/F/T in the other direction (,)",
        |cx| repeat_char_search(cx, true),
    );

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

fn repeat_char_search(cx: &mut Context, reverse: bool) -> CommandResult {
    let Some(search) = cx.editor.last_char_search else {
        return Ok(());
    };
    let search = if reverse { search.reversed() } else { search };
    let motion = CHAR_SEARCHES
        .iter()
        .find(|m| is_forward(m) == search.forward && is_till(m) == search.till)
        .expect("one motion per direction and kind");
    let args = MotionArgs {
        count: cx.count,
        char: Some(search.ch),
        repeating: true,
        ..MotionArgs::default()
    };
    run(cx, motion, args)
}

fn char_search(args: MotionArgs, forward: bool, till: bool) -> Option<CharSearch> {
    Some(CharSearch {
        ch: args.char?,
        forward,
        till,
    })
}

fn is_forward(motion: &Motion) -> bool {
    motion.name.ends_with("_forward")
}

fn is_till(motion: &Motion) -> bool {
    motion.name.starts_with("till_")
}

/// Runs a motion: applies the pending operator over it, or moves the cursor.
pub(super) fn run(cx: &mut Context, motion: &'static Motion, args: MotionArgs) -> CommandResult {
    let Some(pending) = cx.editor.take_pending_operator() else {
        move_cursor(cx.editor, motion, args);
        return Ok(());
    };

    let doc = &cx.editor.document;
    let cursor = doc.cursor();
    let motion = if motion.name == WORD_FORWARD.name {
        let on_word = doc
            .buffer()
            .char_at(cursor)
            .is_some_and(|c| !c.is_whitespace());
        if pending.operator == Operator::Change && on_word {
            &CHANGE_WORD
        } else {
            &OPERATOR_WORD_FORWARD
        }
    } else {
        motion
    };

    let args = MotionArgs {
        count: multiply_counts(pending.count, args.count),
        ..args
    };
    // A failed motion (`dfx` with no `x`) cancels the operator.
    let Some(range) = motion.range(doc.buffer(), cursor, args) else {
        return Ok(());
    };
    let register = pending.register.or(cx.register);
    cx.editor.apply_operator(pending.operator, range, register)
}

fn move_cursor(editor: &mut Editor, motion: &Motion, args: MotionArgs) {
    let doc = &mut editor.document;
    let cursor = doc.cursor();
    let Some(target) = (motion.target)(doc.buffer(), cursor, args) else {
        return;
    };
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
    times: usize,
    step: fn(&Buffer, Position) -> Position,
) -> Position {
    (0..times).fold(start, |p, _| step(buffer, p))
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
            WORD_FORWARD.range(&b, p, MotionArgs::default()),
            Some(Range::Chars {
                start: p,
                end: pos(0, 4)
            })
        );
        assert_eq!(
            WORD_END.range(&b, p, MotionArgs::default()),
            Some(Range::Chars {
                start: p,
                end: pos(0, 3)
            })
        );
        assert_eq!(
            DOWN.range(&b, p, MotionArgs::default()),
            Some(Range::Lines { first: 0, last: 1 })
        );
    }

    #[test]
    fn change_word_stays_within_current_word() {
        let b = Buffer::from_text("ab cd ef");
        let range = |p, n| CHANGE_WORD.range(&b, p, MotionArgs::with_count(n)).unwrap();
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
