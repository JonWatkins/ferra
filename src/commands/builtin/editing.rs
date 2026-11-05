//! Insert-mode editing and line opening. Operator-based edits (`x`, `dd`,
//! `cw`, ...) live in `operators.rs`.

use crate::commands::CommandRegistry;
use crate::core::{Position, Range};
use crate::editor::CaseChange;
use crate::editor::Mode;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "insert_newline",
        "Insert a line break (keeping indentation with auto-indent)",
        |cx| {
            let cursor = cx.editor.document.cursor();
            let indent = cx.editor.indent_for_new_line(cursor.row, cursor.col);
            cx.editor.document.insert_str(&format!("\n{indent}"));
            Ok(())
        },
    );
    r.register("insert_tab", "Insert one level of indentation", |cx| {
        let indent = cx.editor.indent_unit();
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
        "join_lines",
        "Join [count] lines (at least two), or the selected lines (J)",
        |cx| {
            let (row, joins) = match cx.editor.take_selection() {
                Some(range) => {
                    let (first, last) = range.rows();
                    (first, (last - first).max(1))
                }
                None => (cx.editor.document.cursor().row, cx.count().max(2) - 1),
            };
            for _ in 0..joins {
                if !cx.editor.join_line(row) {
                    break;
                }
            }
            Ok(())
        },
    );
    r.register(
        "toggle_case_char",
        "Toggle the case of [count] characters and move past them (~)",
        |cx| {
            let n = cx.count();
            let doc = &cx.editor.document;
            let start = doc.cursor();
            let end = start.with_col((start.col + n).min(doc.buffer().line_len(start.row)));
            cx.editor
                .change_case(Range::Chars { start, end }, CaseChange::Toggle);
            cx.editor.document.set_cursor(end);
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
            let row = cx.editor.document.cursor().row;
            let line_len = cx.editor.document.buffer().line_len(row);
            let indent = cx.editor.indent_for_new_line(row, line_len);
            cx.editor.document.open_line(true);
            cx.editor.document.insert_str(&indent);
            cx.editor.set_mode(Mode::Insert);
            Ok(())
        },
    );
    r.register(
        "open_line_above",
        "Open a new line above and enter insert mode",
        |cx| {
            // The new line above takes this line's indentation.
            let row = cx.editor.document.cursor().row;
            let indent = cx.editor.indent_for_new_line(row, 0);
            cx.editor.document.open_line(false);
            cx.editor.document.insert_str(&indent);
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
    fn auto_indent_on_enter_and_open_line() {
        let mut e = editor("    foo {", pos(0, 9), Mode::Insert);
        run(&mut e, "insert_newline", None);
        assert_eq!(e.document.buffer().text(), "    foo {\n        ");
        assert_eq!(e.document.cursor(), pos(1, 8));

        let mut e = editor("  a", pos(0, 0), Mode::Normal);
        run(&mut e, "open_line_below", None);
        run(&mut e, "normal_mode", None);
        run(&mut e, "open_line_above", None);
        assert_eq!(e.document.buffer().text(), "  a\n  \n  ");
    }

    #[test]
    fn join_lines_with_count_and_selection() {
        let mut e = editor("a\n  b\nc\nd", pos(0, 0), Mode::Normal);
        run(&mut e, "join_lines", Some(3));
        assert_eq!(e.document.buffer().text(), "a b c\nd");

        let mut e = editor("a\nb\nc", pos(0, 0), Mode::Normal);
        e.toggle_visual(crate::editor::SelectionKind::Line);
        e.document.set_cursor(pos(1, 0));
        run(&mut e, "join_lines", None);
        assert_eq!(e.document.buffer().text(), "a b\nc");
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn tilde_toggles_and_moves() {
        let mut e = editor("abC", pos(0, 0), Mode::Normal);
        run(&mut e, "toggle_case_char", Some(2));
        assert_eq!(e.document.buffer().text(), "ABC");
        assert_eq!(e.document.cursor(), pos(0, 2));
        run(&mut e, "toggle_case_char", Some(5));
        assert_eq!(e.document.buffer().text(), "ABc");
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
