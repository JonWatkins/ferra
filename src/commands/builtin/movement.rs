use crate::commands::{CommandRegistry, Context};
use crate::core::{Buffer, Position, motion};

pub fn register(r: &mut CommandRegistry) {
    r.register("move_left", "Move cursor left", |cx| {
        let cursor = cx.editor.document.cursor();
        let col = cursor.col.saturating_sub(cx.count());
        cx.editor.document.set_cursor(cursor.with_col(col));
        Ok(())
    });
    r.register("move_right", "Move cursor right", |cx| {
        let cursor = cx.editor.document.cursor();
        cx.editor
            .document
            .set_cursor(cursor.with_col(cursor.col + cx.count()));
        Ok(())
    });
    r.register("move_up", "Move cursor up", |cx| {
        let n = cx.count() as isize;
        cx.editor.document.move_vertical(-n);
        Ok(())
    });
    r.register("move_down", "Move cursor down", |cx| {
        let n = cx.count() as isize;
        cx.editor.document.move_vertical(n);
        Ok(())
    });

    r.register("move_word_forward", "Move to start of next word", |cx| {
        repeat_motion(cx, motion::word_forward);
        Ok(())
    });
    r.register(
        "move_word_backward",
        "Move to start of previous word",
        |cx| {
            repeat_motion(cx, motion::word_backward);
            Ok(())
        },
    );
    r.register("move_word_end", "Move to end of word", |cx| {
        repeat_motion(cx, motion::word_end);
        Ok(())
    });

    r.register("goto_line_start", "Move to start of line", |cx| {
        let cursor = cx.editor.document.cursor();
        cx.editor.document.set_cursor(cursor.with_col(0));
        Ok(())
    });
    r.register("goto_line_end", "Move to end of line", |cx| {
        let cursor = cx.editor.document.cursor();
        cx.editor.document.set_cursor(cursor.with_col(usize::MAX));
        Ok(())
    });
    r.register(
        "goto_first_non_blank",
        "Move to first non-blank character of line",
        |cx| {
            let row = cx.editor.document.cursor().row;
            goto_row(cx, row);
            Ok(())
        },
    );

    r.register(
        "goto_file_start",
        "Go to first line, or line [count]",
        |cx| {
            let row = cx.count.map_or(0, |n| n.saturating_sub(1));
            goto_row(cx, row);
            Ok(())
        },
    );
    r.register("goto_file_end", "Go to last line, or line [count]", |cx| {
        let last = cx.editor.document.buffer().line_count() - 1;
        let row = cx.count.map_or(last, |n| n.saturating_sub(1));
        goto_row(cx, row);
        Ok(())
    });
    r.register("goto_line", "Go to the line given as argument", |cx| {
        let line: usize = cx
            .args
            .first()
            .ok_or("goto_line: missing line number")?
            .parse()
            .map_err(|_| "goto_line: invalid line number")?;
        goto_row(cx, line.saturating_sub(1));
        Ok(())
    });
}

fn repeat_motion(cx: &mut Context, motion: fn(&Buffer, Position) -> Position) {
    let count = cx.count();
    let doc = &mut cx.editor.document;
    let mut pos = doc.cursor();
    for _ in 0..count {
        pos = motion(doc.buffer(), pos);
    }
    doc.set_cursor(pos);
}

/// Moves to `row` (clamped) at its first non-blank character.
fn goto_row(cx: &mut Context, row: usize) {
    let doc = &mut cx.editor.document;
    let row = row.min(doc.buffer().line_count() - 1);
    let col = motion::first_non_blank(doc.buffer(), row);
    doc.set_cursor(Position::new(row, col));
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run, run_with_args};
    use crate::core::Position;
    use crate::editor::Mode;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn move_right_stops_on_last_char_in_normal_mode() {
        let mut e = editor("abc", pos(0, 0), Mode::Normal);
        run(&mut e, "move_right", Some(10));
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn move_right_can_reach_line_end_in_insert_mode() {
        let mut e = editor("abc", pos(0, 0), Mode::Insert);
        run(&mut e, "move_right", Some(10));
        assert_eq!(e.document.cursor(), pos(0, 3));
    }

    #[test]
    fn move_left_respects_count() {
        let mut e = editor("abcdef", pos(0, 5), Mode::Normal);
        run(&mut e, "move_left", Some(3));
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn move_down_respects_count() {
        let mut e = editor("a\nb\nc\nd", pos(0, 0), Mode::Normal);
        run(&mut e, "move_down", Some(2));
        assert_eq!(e.document.cursor(), pos(2, 0));
        run(&mut e, "move_up", None);
        assert_eq!(e.document.cursor(), pos(1, 0));
    }

    #[test]
    fn word_motions_respect_count() {
        let mut e = editor("one two three", pos(0, 0), Mode::Normal);
        run(&mut e, "move_word_forward", Some(2));
        assert_eq!(e.document.cursor(), pos(0, 8));
        run(&mut e, "move_word_backward", None);
        assert_eq!(e.document.cursor(), pos(0, 4));
        run(&mut e, "move_word_end", None);
        assert_eq!(e.document.cursor(), pos(0, 6));
    }

    #[test]
    fn line_start_end_and_first_non_blank() {
        let mut e = editor("  hello", pos(0, 4), Mode::Normal);
        run(&mut e, "goto_line_end", None);
        assert_eq!(e.document.cursor(), pos(0, 6));
        run(&mut e, "goto_line_start", None);
        assert_eq!(e.document.cursor(), pos(0, 0));
        run(&mut e, "goto_first_non_blank", None);
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn goto_file_start_and_end() {
        let mut e = editor("a\nb\n  c", pos(1, 0), Mode::Normal);
        run(&mut e, "goto_file_end", None);
        assert_eq!(e.document.cursor(), pos(2, 2));
        run(&mut e, "goto_file_start", None);
        assert_eq!(e.document.cursor(), pos(0, 0));
    }

    #[test]
    fn goto_file_start_with_count_goes_to_line() {
        let mut e = editor("a\nb\nc", pos(0, 0), Mode::Normal);
        run(&mut e, "goto_file_start", Some(2));
        assert_eq!(e.document.cursor(), pos(1, 0));
    }

    #[test]
    fn goto_line_parses_argument() {
        let mut e = editor("a\nb\nc", pos(0, 0), Mode::Normal);
        run_with_args(&mut e, "goto_line", None, &["3"]);
        assert_eq!(e.document.cursor(), pos(2, 0));
    }

    #[test]
    fn goto_line_reports_bad_argument() {
        let mut e = editor("a", pos(0, 0), Mode::Normal);
        run_with_args(&mut e, "goto_line", None, &["x"]);
        assert!(e.message().unwrap().text.contains("invalid line number"));
    }
}
