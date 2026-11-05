use crate::commands::CommandRegistry;
use crate::core::motion;
use crate::editor::Mode;

pub fn register(r: &mut CommandRegistry) {
    r.register("normal_mode", "Enter normal mode", |cx| {
        cx.editor.set_mode(Mode::Normal);
        Ok(())
    });
    r.register("insert_mode", "Insert before cursor", |cx| {
        cx.editor.set_mode(Mode::Insert);
        Ok(())
    });
    r.register("append_mode", "Insert after cursor", |cx| {
        cx.editor.set_mode(Mode::Insert);
        let cursor = cx.editor.document.cursor();
        let line_len = cx.editor.document.buffer().line_len(cursor.row);
        let col = (cursor.col + 1).min(line_len);
        cx.editor.document.set_cursor(cursor.with_col(col));
        Ok(())
    });
    r.register(
        "insert_at_line_start",
        "Insert at first non-blank character of line",
        |cx| {
            cx.editor.set_mode(Mode::Insert);
            let cursor = cx.editor.document.cursor();
            let col = motion::first_non_blank(cx.editor.document.buffer(), cursor.row);
            cx.editor.document.set_cursor(cursor.with_col(col));
            Ok(())
        },
    );
    r.register("append_at_line_end", "Insert at end of line", |cx| {
        cx.editor.set_mode(Mode::Insert);
        let cursor = cx.editor.document.cursor();
        cx.editor.document.set_cursor(cursor.with_col(usize::MAX));
        Ok(())
    });
    r.register("command_mode", "Enter command-line mode", |cx| {
        cx.editor.set_mode(Mode::Command);
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run};
    use crate::core::Position;
    use crate::editor::Mode;

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    #[test]
    fn append_mode_moves_after_cursor() {
        let mut e = editor("abc", pos(0, 2), Mode::Normal);
        run(&mut e, "append_mode", None);
        assert_eq!(e.mode(), Mode::Insert);
        assert_eq!(e.document.cursor(), pos(0, 3));
    }

    #[test]
    fn append_mode_on_empty_line_stays_at_zero() {
        let mut e = editor("", pos(0, 0), Mode::Normal);
        run(&mut e, "append_mode", None);
        assert_eq!(e.document.cursor(), pos(0, 0));
    }

    #[test]
    fn insert_at_line_start_skips_indent() {
        let mut e = editor("  abc", pos(0, 4), Mode::Normal);
        run(&mut e, "insert_at_line_start", None);
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn append_at_line_end_moves_past_last_char() {
        let mut e = editor("abc", pos(0, 0), Mode::Normal);
        run(&mut e, "append_at_line_end", None);
        assert_eq!(e.document.cursor(), pos(0, 3));
    }

    #[test]
    fn normal_mode_steps_back_from_insert() {
        let mut e = editor("abc", pos(0, 3), Mode::Insert);
        run(&mut e, "normal_mode", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn command_mode_switches_mode() {
        let mut e = editor("", pos(0, 0), Mode::Normal);
        run(&mut e, "command_mode", None);
        assert_eq!(e.mode(), Mode::Command);
    }
}
