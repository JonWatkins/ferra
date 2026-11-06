//! Marks: `m{a-z A-Z}` sets one, `'x` jumps to its line, `` `x `` to its
//! exact position. Both jumps are motions, so they work after operators
//! (`d'a`) and extend visual selections.

use super::motions::{self, Motion, MotionArgs};
use crate::commands::{CommandRegistry, CommandResult, Context};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "set_mark",
        "Mark the cursor position as {a-z}, or {A-Z} across buffers (m)",
        |cx| {
            let name = cx.char_arg.ok_or("missing mark name")?;
            cx.editor.set_mark(name)
        },
    )
    .takes_char();

    r.register(
        "goto_mark_line",
        "Go to the line of mark {char} (')",
        |cx| goto_mark(cx, &motions::MARK_LINE),
    )
    .takes_char();

    r.register(
        "goto_mark",
        "Go to the exact position of mark {char} (`)",
        |cx| goto_mark(cx, &motions::MARK_EXACT),
    )
    .takes_char();
}

fn goto_mark(cx: &mut Context, motion: &'static Motion) -> CommandResult {
    let name = cx.char_arg.ok_or("missing mark name")?;
    let (document, position) = cx.editor.mark_target(name)?;
    if document != cx.editor.document.id() {
        // A global mark in another buffer: switch to it. Operators can't
        // span buffers, so one waiting for this motion is cancelled.
        if cx.editor.take_pending_operator().is_some() {
            return Err("Mark is in another buffer".into());
        }
        cx.editor.switch_to(document);
    }
    let args = MotionArgs {
        count: None,
        target: Some(position),
        ..MotionArgs::default()
    };
    motions::run(cx, motion, args)
}

#[cfg(test)]
mod tests {
    use crate::commands::Invocation;
    use crate::commands::builtin::test_util::{editor, run_with};
    use crate::core::Position;
    use crate::editor::{Editor, Mode, Operator};

    fn with_char(e: &mut Editor, name: &str, c: char) {
        let invocation = Invocation {
            char_arg: Some(c),
            ..Invocation::default()
        };
        run_with(e, name, invocation);
    }

    #[test]
    fn set_and_jump() {
        let mut e = editor("one\n  two\nthree", Position::new(1, 4), Mode::Normal);
        with_char(&mut e, "set_mark", 'a');
        e.document.set_cursor(Position::new(0, 0));
        with_char(&mut e, "goto_mark_line", 'a');
        assert_eq!(e.document.cursor(), Position::new(1, 2), "first non-blank");
        e.document.set_cursor(Position::new(0, 0));
        with_char(&mut e, "goto_mark", 'a');
        assert_eq!(e.document.cursor(), Position::new(1, 4), "exact");
    }

    #[test]
    fn marks_are_motions_for_operators() {
        let mut e = editor("1\n2\n3\n4", Position::new(2, 0), Mode::Normal);
        with_char(&mut e, "set_mark", 'a');
        e.document.set_cursor(Position::new(0, 0));
        e.begin_operator(Operator::Delete, None, None);
        with_char(&mut e, "goto_mark_line", 'a');
        assert_eq!(e.document.buffer().text(), "4", "d'a deletes lines 1-3");
    }

    #[test]
    fn unset_mark_is_an_error() {
        let mut e = editor("x", Position::default(), Mode::Normal);
        with_char(&mut e, "goto_mark", 'q');
        assert_eq!(e.message().unwrap().text, "Mark not set: q");
    }
}
