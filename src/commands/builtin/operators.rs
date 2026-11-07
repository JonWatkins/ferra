//! Operators (`d`, `c`, `y`), their single-key shortcuts (`x`, `D`, `C`,
//! `s`, `S`, `Y`, `X`) and pasting (`p`, `P`).

use super::motions::{self, Motion, MotionArgs};
use super::visual;
use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::{Position, Range, motion};
use crate::editor::{Mode, Operator};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "operator_delete",
        "Delete over a motion or text object",
        |cx| begin(cx, Operator::Delete),
    );
    r.register(
        "operator_change",
        "Change over a motion or text object",
        |cx| begin(cx, Operator::Change),
    );
    r.register("operator_yank", "Yank over a motion or text object", |cx| {
        begin(cx, Operator::Yank)
    });

    r.register(
        "operator_indent",
        "Indent over a motion or text object (>)",
        |cx| begin(cx, Operator::Indent),
    );
    r.register(
        "operator_outdent",
        "Outdent over a motion or text object (<)",
        |cx| begin(cx, Operator::Outdent),
    );
    r.register(
        "operator_lowercase",
        "Lowercase over a motion or text object (gu)",
        |cx| begin(cx, Operator::Lowercase),
    );
    r.register(
        "operator_uppercase",
        "Uppercase over a motion or text object (gU)",
        |cx| begin(cx, Operator::Uppercase),
    );
    r.register(
        "operator_toggle_case",
        "Toggle case over a motion or text object (g~)",
        |cx| begin(cx, Operator::ToggleCase),
    );
    r.register(
        "operator_comment",
        "Toggle comments on the lines of a motion or text object (gc, gcc)",
        |cx| begin(cx, Operator::Comment),
    );

    r.register(
        "delete_char",
        "Delete [count] characters under the cursor (dl)",
        |cx| operate(cx, Operator::Delete, &motions::RIGHT),
    );
    r.register(
        "delete_char_before",
        "Delete [count] characters before the cursor (dh)",
        |cx| operate(cx, Operator::Delete, &motions::LEFT),
    );
    r.register("delete_to_line_end", "Delete to end of line (d$)", |cx| {
        operate(cx, Operator::Delete, &motions::LINE_END)
    });
    r.register("change_to_line_end", "Change to end of line (c$)", |cx| {
        operate(cx, Operator::Change, &motions::LINE_END)
    });
    r.register("substitute_char", "Change [count] characters (cl)", |cx| {
        operate(cx, Operator::Change, &motions::RIGHT)
    });
    r.register("substitute_line", "Change [count] whole lines (cc)", |cx| {
        operate_on_lines(cx, Operator::Change)
    });
    r.register("yank_line", "Yank [count] whole lines (yy)", |cx| {
        operate_on_lines(cx, Operator::Yank)
    });

    r.register(
        "paste_after",
        "Paste after the cursor (below, for lines)",
        |cx| paste(cx, true),
    );
    r.register(
        "paste_before",
        "Paste before the cursor (above, for lines)",
        |cx| paste(cx, false),
    );
}

/// Starts an operator, or in visual mode applies it to the selection. A
/// count on a visual `>` or `<` shifts that many times (`3>`).
fn begin(cx: &mut Context, operator: Operator) -> CommandResult {
    let times = match operator {
        Operator::Indent | Operator::Outdent => cx.count(),
        _ => 1,
    };
    if let Some(block) = cx.editor.take_block() {
        for _ in 0..times {
            cx.editor
                .apply_block_operator(operator, block, cx.register)?;
        }
        return Ok(());
    }
    if let Some(range) = cx.editor.take_selection() {
        for _ in 0..times {
            cx.editor.apply_operator(operator, range, cx.register)?;
        }
        return Ok(());
    }
    cx.editor.begin_operator(operator, cx.count, cx.register);
    Ok(())
}

fn operate(cx: &mut Context, operator: Operator, motion: &Motion) -> CommandResult {
    let doc = &cx.editor.document;
    let args = MotionArgs::with_count(cx.count);
    let Some(range) = motion.range(doc.buffer(), doc.cursor(), args) else {
        return Ok(());
    };
    cx.editor.apply_operator(operator, range, cx.register)
}

fn operate_on_lines(cx: &mut Context, operator: Operator) -> CommandResult {
    let doc = &cx.editor.document;
    let first = doc.cursor().row;
    let last = (first + cx.count() - 1).min(doc.buffer().line_count() - 1);
    cx.editor
        .apply_operator(operator, Range::Lines { first, last }, cx.register)
}

fn paste(cx: &mut Context, after: bool) -> CommandResult {
    let register = cx
        .editor
        .read_register(cx.register)?
        .ok_or("Nothing to paste: register is empty")?;
    if cx.editor.mode() == Mode::Visual {
        return visual::replace_selection(cx, register, after);
    }
    let count = cx.count();
    if register.blockwise {
        cx.editor.paste_block(&register, after, count);
        return Ok(());
    }
    let doc = &mut cx.editor.document;
    let cursor = doc.cursor();

    if register.linewise {
        let block = vec![register.text.as_str(); count].join("\n");
        let row = if after {
            doc.insert(doc.buffer().line_end(cursor.row), &format!("\n{block}"));
            cursor.row + 1
        } else {
            doc.insert(Position::new(cursor.row, 0), &format!("{block}\n"));
            cursor.row
        };
        let col = motion::first_non_blank(doc.buffer(), row);
        doc.set_cursor(Position::new(row, col));
    } else {
        let line_len = doc.buffer().line_len(cursor.row);
        let at = if after && line_len > 0 {
            cursor.with_col(cursor.col + 1)
        } else {
            cursor
        };
        let end = doc.insert(at, &register.text.repeat(count));
        // Leave the cursor on the last pasted character.
        doc.set_cursor(end.with_col(end.col.saturating_sub(1)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commands::Invocation;
    use crate::commands::builtin::test_util::{editor, run, run_with};
    use crate::core::Position;
    use crate::editor::registers::StoreKind;
    use crate::editor::{Editor, Mode, Register};

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn text(e: &Editor) -> String {
        e.document.buffer().text()
    }

    #[test]
    fn delete_char_fills_register_and_respects_count() {
        let mut e = editor("abcdef", pos(0, 1), Mode::Normal);
        run(&mut e, "delete_char", Some(3));
        assert_eq!(text(&e), "aef");
        assert_eq!(e.registers.get(None), Some(&Register::charwise("bcd")));
    }

    #[test]
    fn delete_char_at_line_end_keeps_cursor_on_char() {
        let mut e = editor("abc", pos(0, 2), Mode::Normal);
        run(&mut e, "delete_char", None);
        assert_eq!(text(&e), "ab");
        assert_eq!(e.document.cursor(), pos(0, 1));
    }

    #[test]
    fn delete_char_before() {
        let mut e = editor("abc", pos(0, 2), Mode::Normal);
        run(&mut e, "delete_char_before", None);
        assert_eq!(text(&e), "ac");
    }

    #[test]
    fn delete_and_change_to_line_end() {
        let mut e = editor("hello world", pos(0, 5), Mode::Normal);
        run(&mut e, "delete_to_line_end", None);
        assert_eq!(text(&e), "hello");

        let mut e = editor("hello world", pos(0, 5), Mode::Normal);
        run(&mut e, "change_to_line_end", None);
        assert_eq!(text(&e), "hello");
        assert_eq!(e.mode(), Mode::Insert);
    }

    #[test]
    fn substitute_line_clears_line_and_inserts() {
        let mut e = editor("a\nbbb\nc", pos(1, 1), Mode::Normal);
        run(&mut e, "substitute_line", None);
        assert_eq!(text(&e), "a\n\nc");
        assert_eq!(e.mode(), Mode::Insert);
    }

    #[test]
    fn operators_apply_to_selection_in_visual_mode() {
        let mut e = editor("hello world", pos(0, 0), Mode::Normal);
        e.toggle_visual(crate::editor::SelectionKind::Char);
        e.document.set_cursor(pos(0, 4));
        run(&mut e, "operator_yank", None);
        assert_eq!(e.registers.get(None), Some(&Register::charwise("hello")));
        assert_eq!(e.mode(), Mode::Normal);

        e.toggle_visual(crate::editor::SelectionKind::Char);
        e.document.set_cursor(pos(0, 5));
        run(&mut e, "operator_delete", None);
        assert_eq!(text(&e), "world");
    }

    #[test]
    fn begin_operator_records_count_and_register() {
        let mut e = editor("", pos(0, 0), Mode::Normal);
        let invocation = Invocation {
            count: Some(2),
            register: Some('a'),
            ..Invocation::default()
        };
        run_with(&mut e, "operator_delete", invocation);
        assert_eq!(e.mode(), Mode::OperatorPending);
        let pending = e.pending_operator().unwrap();
        assert_eq!((pending.count, pending.register), (Some(2), Some('a')));
    }

    #[test]
    fn yank_line_then_paste_below_and_above() {
        let mut e = editor("one\ntwo", pos(0, 1), Mode::Normal);
        run(&mut e, "yank_line", None);
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "one\none\ntwo");
        assert_eq!(e.document.cursor(), pos(1, 0));
        run(&mut e, "paste_before", Some(2));
        assert_eq!(text(&e), "one\none\none\none\ntwo");
    }

    #[test]
    fn charwise_paste_after_and_before() {
        let mut e = editor("ac", pos(0, 0), Mode::Normal);
        e.registers
            .store(None, Register::charwise("b"), StoreKind::Yank)
            .unwrap();
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "abc");
        assert_eq!(e.document.cursor(), pos(0, 1));
        run(&mut e, "paste_before", Some(2));
        assert_eq!(text(&e), "abbbc");
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn paste_on_empty_line_inserts_at_start() {
        let mut e = editor("", pos(0, 0), Mode::Normal);
        e.registers
            .store(None, Register::charwise("xy"), StoreKind::Yank)
            .unwrap();
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "xy");
    }

    #[test]
    fn paste_from_named_register() {
        let mut e = editor("", pos(0, 0), Mode::Normal);
        e.registers
            .store(Some('q'), Register::charwise("Q"), StoreKind::Yank)
            .unwrap();
        e.registers
            .store(None, Register::charwise("other"), StoreKind::Yank)
            .unwrap();
        let invocation = Invocation {
            register: Some('q'),
            ..Invocation::default()
        };
        run_with(&mut e, "paste_after", invocation);
        assert_eq!(text(&e), "Q");
    }

    #[test]
    fn paste_with_empty_register_is_an_error() {
        let mut e = editor("", pos(0, 0), Mode::Normal);
        run(&mut e, "paste_after", None);
        assert!(e.message().unwrap().text.contains("register is empty"));
    }
}
