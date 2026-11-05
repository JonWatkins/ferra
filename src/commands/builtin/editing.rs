//! Insert-mode editing and line opening. Operator-based edits (`x`, `dd`,
//! `cw`, ...) live in `operators.rs`.

use crate::commands::CommandRegistry;
use crate::editor::Mode;

pub fn register(r: &mut CommandRegistry) {
    r.register("insert_newline", "Insert a line break", |cx| {
        cx.editor.document.insert_str("\n");
        Ok(())
    });
    r.register("insert_tab", "Insert indentation", |cx| {
        let indent = " ".repeat(cx.editor.options.tab_width);
        cx.editor.document.insert_str(&indent);
        Ok(())
    });
    r.register(
        "delete_char_backward",
        "Delete character before cursor",
        |cx| {
            for _ in 0..cx.count() {
                cx.editor.document.delete_backward();
            }
            Ok(())
        },
    );
    r.register(
        "delete_char_forward",
        "Delete character under cursor, joining lines at line end",
        |cx| {
            for _ in 0..cx.count() {
                cx.editor.document.delete_forward();
            }
            Ok(())
        },
    );
    r.register(
        "open_line_below",
        "Open a new line below and enter insert mode",
        |cx| {
            cx.editor.document.open_line(true);
            cx.editor.set_mode(Mode::Insert);
            Ok(())
        },
    );
    r.register(
        "open_line_above",
        "Open a new line above and enter insert mode",
        |cx| {
            cx.editor.document.open_line(false);
            cx.editor.set_mode(Mode::Insert);
            Ok(())
        },
    );
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
    fn open_line_below_enters_insert_mode() {
        let mut e = editor("a\nb", pos(0, 0), Mode::Normal);
        run(&mut e, "open_line_below", None);
        assert_eq!(e.document.buffer().text(), "a\n\nb");
        assert_eq!(e.document.cursor(), pos(1, 0));
        assert_eq!(e.mode(), Mode::Insert);
    }

    #[test]
    fn open_line_above_enters_insert_mode() {
        let mut e = editor("a", pos(0, 0), Mode::Normal);
        run(&mut e, "open_line_above", None);
        assert_eq!(e.document.buffer().text(), "\na");
        assert_eq!(e.document.cursor(), pos(0, 0));
        assert_eq!(e.mode(), Mode::Insert);
    }

    #[test]
    fn insert_tab_uses_tab_width_option() {
        let mut e = editor("", pos(0, 0), Mode::Insert);
        e.options.tab_width = 2;
        run(&mut e, "insert_tab", None);
        assert_eq!(e.document.buffer().text(), "  ");
    }

    #[test]
    fn delete_char_backward_joins_lines() {
        let mut e = editor("a\nb", pos(1, 0), Mode::Insert);
        run(&mut e, "delete_char_backward", None);
        assert_eq!(e.document.buffer().text(), "ab");
    }

    #[test]
    fn delete_char_forward_respects_count() {
        let mut e = editor("abc", pos(0, 0), Mode::Insert);
        run(&mut e, "delete_char_forward", Some(2));
        assert_eq!(e.document.buffer().text(), "c");
    }

    #[test]
    fn insert_newline_splits_line() {
        let mut e = editor("ab", pos(0, 1), Mode::Insert);
        run(&mut e, "insert_newline", None);
        assert_eq!(e.document.buffer().text(), "a\nb");
        assert_eq!(e.document.cursor(), pos(1, 0));
    }
}
