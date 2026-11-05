//! Insert-mode editing and line opening. Operator-based edits (`x`, `dd`,
//! `cw`, ...) live in `operators.rs`.

use crate::commands::CommandRegistry;
use crate::core::Position;
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
        "trim_trailing_whitespace",
        "Remove trailing whitespace from every line",
        |cx| {
            let doc = &mut cx.editor.document;
            let cursor = doc.cursor();
            for row in 0..doc.buffer().line_count() {
                let line = doc.buffer().line(row);
                let len = line.chars().count();
                let kept = line.trim_end().chars().count();
                if kept < len {
                    doc.delete(Position::new(row, kept), Position::new(row, len));
                }
            }
            doc.set_cursor(cursor);
            Ok(())
        },
    );
    r.register(
        "replace_char",
        "Replace [count] characters under the cursor with {char} (r)",
        |cx| {
            let ch = cx.char_arg.ok_or("missing character")?;
            let n = cx.count();
            let doc = &mut cx.editor.document;
            let start = doc.cursor();
            // Like Vim, do nothing unless there are enough characters.
            if start.col + n > doc.buffer().line_len(start.row) {
                return Ok(());
            }
            doc.delete(start, start.with_col(start.col + n));
            doc.insert(start, &ch.to_string().repeat(n));
            doc.set_cursor(start.with_col(start.col + n - 1));
            Ok(())
        },
    )
    .takes_char();
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

    fn replace(e: &mut crate::editor::Editor, count: Option<usize>, ch: char) {
        let invocation = crate::commands::Invocation {
            count,
            char_arg: Some(ch),
            ..Default::default()
        };
        crate::commands::builtin::test_util::run_with(e, "replace_char", invocation);
    }

    #[test]
    fn replace_char_with_count() {
        let mut e = editor("abcd", pos(0, 1), Mode::Normal);
        replace(&mut e, Some(2), 'x');
        assert_eq!(e.document.buffer().text(), "axxd");
        assert_eq!(e.document.cursor(), pos(0, 2));
    }

    #[test]
    fn replace_char_needs_enough_characters() {
        let mut e = editor("ab", pos(0, 1), Mode::Normal);
        replace(&mut e, Some(2), 'x');
        assert_eq!(e.document.buffer().text(), "ab");
        let mut e = editor("", pos(0, 0), Mode::Normal);
        replace(&mut e, None, 'x');
        assert_eq!(e.document.buffer().text(), "");
    }

    #[test]
    fn replace_char_is_one_undo_step() {
        let mut e = editor("abc", pos(0, 0), Mode::Normal);
        replace(&mut e, Some(3), 'z');
        e.document.undo();
        assert_eq!(e.document.buffer().text(), "abc");
    }

    #[test]
    fn trim_trailing_whitespace_is_one_undo_step() {
        let mut e = editor("a  \nb\t\n  \nc", pos(0, 3), Mode::Normal);
        run(&mut e, "trim_trailing_whitespace", None);
        assert_eq!(e.document.buffer().text(), "a\nb\n\nc");
        assert_eq!(
            e.document.cursor(),
            pos(0, 0),
            "cursor clamped onto the line"
        );
        e.document.undo();
        assert_eq!(e.document.buffer().text(), "a  \nb\t\n  \nc");
    }

    #[test]
    fn insert_newline_splits_line() {
        let mut e = editor("ab", pos(0, 1), Mode::Insert);
        run(&mut e, "insert_newline", None);
        assert_eq!(e.document.buffer().text(), "a\nb");
        assert_eq!(e.document.cursor(), pos(1, 0));
    }
}
