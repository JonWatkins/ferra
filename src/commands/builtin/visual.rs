//! Visual mode: entering and leaving it, and the commands that only make
//! sense on a selection. Operators (`d`, `c`, `y`), motions and text objects
//! handle visual mode themselves, since a selection is just another range
//! (block selections have their own path, in `editor/block.rs`).

use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::{Position, Range, motion};
use crate::editor::registers::BLACK_HOLE;
use crate::editor::{Operator, Register, SelectionKind};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "visual_mode",
        "Select characters (v); again to leave",
        |cx| {
            cx.editor.toggle_visual(SelectionKind::Char);
            Ok(())
        },
    );
    r.register(
        "visual_line_mode",
        "Select lines (V); again to leave",
        |cx| {
            cx.editor.toggle_visual(SelectionKind::Line);
            Ok(())
        },
    );
    r.register(
        "visual_block_mode",
        "Select a rectangle (<C-v>); again to leave",
        |cx| {
            cx.editor.toggle_visual(SelectionKind::Block);
            Ok(())
        },
    );
    r.register(
        "swap_selection_sides",
        "In a block, move the cursor to the other side of its row (O)",
        |cx| {
            if cx.editor.selection_block().is_some() {
                cx.editor.swap_block_sides();
            } else {
                cx.editor.swap_selection_ends();
            }
            Ok(())
        },
    );
    r.register(
        "visual_insert",
        "Insert before the selection; on every row of a block (I)",
        |cx| visual_insert(cx, false),
    );
    r.register(
        "visual_append",
        "Append after the selection; on every row of a block (A)",
        |cx| visual_insert(cx, true),
    );
    r.register(
        "visual_delete_to_end",
        "Delete the selected lines; in a block, to the end of each row (D)",
        |cx| to_end_or_lines(cx, Operator::Delete),
    );
    r.register(
        "visual_change_to_end",
        "Change the selected lines; in a block, to the end of each row (C)",
        |cx| to_end_or_lines(cx, Operator::Change),
    );
    r.register(
        "swap_selection_ends",
        "Move the cursor to the other end of the selection",
        |cx| {
            cx.editor.swap_selection_ends();
            Ok(())
        },
    );
    r.register(
        "reselect_visual",
        "Reselect the last selection (gv)",
        |cx| {
            if cx.editor.reselect() {
                Ok(())
            } else {
                Err("No previous selection".into())
            }
        },
    );

    r.register("visual_delete_lines", "Delete the selected lines", |cx| {
        operate_on_selected_lines(cx, Operator::Delete)
    });
    r.register("visual_change_lines", "Change the selected lines", |cx| {
        operate_on_selected_lines(cx, Operator::Change)
    });
    r.register("visual_yank_lines", "Yank the selected lines", |cx| {
        operate_on_selected_lines(cx, Operator::Yank)
    });
}

/// `I` / `A`: in a block, types on every row. Otherwise inserts before the
/// start of the selection, or appends after its end.
fn visual_insert(cx: &mut Context, append: bool) -> CommandResult {
    if let Some(block) = cx.editor.selection_block() {
        cx.editor.block_insert(block, append);
        return Ok(());
    }
    let Some(range) = cx.editor.take_selection() else {
        return Ok(());
    };
    let doc = &mut cx.editor.document;
    let at = match (range, append) {
        (Range::Chars { start, .. }, false) => start,
        (Range::Chars { end, .. }, true) => end,
        (Range::Lines { first, .. }, false) => {
            Position::new(first, motion::first_non_blank(doc.buffer(), first))
        }
        (Range::Lines { last, .. }, true) => doc.buffer().line_end(last),
    };
    doc.set_cursor(at);
    cx.editor.set_mode(crate::editor::Mode::Insert);
    Ok(())
}

/// `D` / `C`: whole lines, or in a block, from the block to each row's end.
fn to_end_or_lines(cx: &mut Context, operator: Operator) -> CommandResult {
    match cx.editor.take_block() {
        Some(block) => {
            let block = crate::core::Block {
                to_eol: true,
                ..block
            };
            cx.editor.apply_block_operator(operator, block, cx.register)
        }
        None => operate_on_selected_lines(cx, operator),
    }
}

/// `D`, `X`, `Y`, `C`, `S` in visual mode act on whole lines even in a
/// characterwise selection.
fn operate_on_selected_lines(cx: &mut Context, operator: Operator) -> CommandResult {
    let Some(range) = cx.editor.take_selection() else {
        return Ok(());
    };
    let (first, last) = range.rows();
    let lines = Range::Lines { first, last };
    cx.editor.apply_operator(operator, lines, cx.register)
}

/// `p` / `P` in visual mode: replaces the selection with `register`.
///
/// With `p` the replaced text goes to the unnamed register, so selecting
/// and pasting again swaps two pieces of text. `P` discards it instead,
/// keeping the register for repeated pastes (as in Neovim).
pub(super) fn replace_selection(
    cx: &mut Context,
    register: Register,
    keep_replaced: bool,
) -> CommandResult {
    if let Some(block) = cx.editor.take_block() {
        return replace_block(cx, block, register, keep_replaced);
    }
    let Some(range) = cx.editor.take_selection() else {
        return Ok(());
    };
    let target = if keep_replaced {
        None
    } else {
        Some(BLACK_HOLE)
    };
    let count = cx.count();

    match range {
        Range::Lines { first, last } => {
            cx.editor.apply_operator(Operator::Delete, range, target)?;
            let block = vec![register.text.as_str(); count].join("\n");
            let doc = &mut cx.editor.document;
            let buffer = doc.buffer();
            if buffer.line_count() == 1 && buffer.line_len(0) == 0 {
                // Everything was selected: the block becomes the whole buffer.
                doc.insert(Position::new(0, 0), &block);
            } else if first < buffer.line_count() {
                doc.insert(Position::new(first, 0), &format!("{block}\n"));
            } else {
                doc.insert(
                    buffer.line_end(last.min(buffer.line_count() - 1)),
                    &format!("\n{block}"),
                );
            }
            let row = first.min(doc.buffer().line_count() - 1);
            let col = motion::first_non_blank(doc.buffer(), row);
            doc.set_cursor(Position::new(row, col));
        }
        Range::Chars { start, .. } => {
            cx.editor.apply_operator(Operator::Delete, range, target)?;
            let doc = &mut cx.editor.document;
            if register.linewise {
                let block = vec![register.text.as_str(); count].join("\n");
                doc.insert(start, &format!("\n{block}\n"));
                let row = start.row + 1;
                let col = motion::first_non_blank(doc.buffer(), row);
                doc.set_cursor(Position::new(row, col));
            } else {
                let end = doc.insert(start, &register.text.repeat(count));
                doc.set_cursor(end.with_col(end.col.saturating_sub(1)));
            }
        }
    }
    Ok(())
}

/// `p` / `P` on a block: a one-line register fills every row; a block
/// replaces it column for column; anything else goes in at the top left.
fn replace_block(
    cx: &mut Context,
    block: crate::core::Block,
    register: Register,
    keep_replaced: bool,
) -> CommandResult {
    let target = if keep_replaced {
        None
    } else {
        Some(BLACK_HOLE)
    };
    if register.blockwise || register.linewise || register.text.contains('\n') {
        cx.editor
            .apply_block_operator(Operator::Delete, block, target)?;
        let count = cx.count();
        if register.blockwise {
            cx.editor.paste_block(&register, false, count);
        } else {
            let doc = &mut cx.editor.document;
            let at = doc.cursor();
            let text = if register.linewise {
                format!("\n{}\n", register.text)
            } else {
                register.text.clone()
            };
            doc.insert(at, &text);
        }
        return Ok(());
    }
    // With `p` the replaced block goes to the unnamed register.
    let rows = cx.editor.block_text(block);
    if keep_replaced && rows.iter().any(|r| !r.is_empty()) {
        let replaced = Register::blockwise(&rows);
        cx.editor
            .registers
            .store(None, replaced, crate::editor::registers::StoreKind::Delete)?;
    }
    cx.editor
        .fill_block(block, &register.text.repeat(cx.count()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run};
    use crate::core::Position;
    use crate::editor::registers::StoreKind;
    use crate::editor::{Editor, Mode, Register, SelectionKind};

    fn pos(row: usize, col: usize) -> Position {
        Position::new(row, col)
    }

    fn text(e: &Editor) -> String {
        e.document.buffer().text()
    }

    /// An editor with a selection from `anchor` to `cursor`.
    fn selecting(content: &str, anchor: Position, cursor: Position, kind: SelectionKind) -> Editor {
        let mut e = editor(content, anchor, Mode::Normal);
        e.toggle_visual(kind);
        e.document.set_cursor(cursor);
        e
    }

    fn yank(e: &mut Editor, register: Register) {
        e.registers.store(None, register, StoreKind::Yank).unwrap();
    }

    #[test]
    fn v_and_capital_v_enter_and_toggle() {
        let mut e = editor("abc", pos(0, 0), Mode::Normal);
        run(&mut e, "visual_mode", None);
        assert_eq!(e.mode(), Mode::Visual);
        run(&mut e, "visual_line_mode", None);
        assert_eq!(e.selection().unwrap().kind, SelectionKind::Line);
        run(&mut e, "visual_line_mode", None);
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn reselect_without_history_is_an_error() {
        let mut e = editor("abc", pos(0, 0), Mode::Normal);
        run(&mut e, "reselect_visual", None);
        assert_eq!(e.message().unwrap().text, "No previous selection");
    }

    #[test]
    fn line_operators_widen_char_selection() {
        let mut e = selecting("a1\nb2\nc3", pos(0, 1), pos(1, 0), SelectionKind::Char);
        run(&mut e, "visual_delete_lines", None);
        assert_eq!(text(&e), "c3");
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn line_operators_ignore_line_whose_break_alone_is_selected() {
        // Selecting the empty line 1 includes its line break, which ends at (2, 0).
        let mut e = selecting("a\n\nc", pos(1, 0), pos(1, 0), SelectionKind::Char);
        run(&mut e, "visual_yank_lines", None);
        assert_eq!(e.registers.get(None), Some(&Register::linewise("")));
    }

    #[test]
    fn change_lines_enters_insert() {
        let mut e = selecting("a\nb", pos(0, 0), pos(0, 0), SelectionKind::Char);
        run(&mut e, "visual_change_lines", None);
        assert_eq!(text(&e), "\nb");
        assert_eq!(e.mode(), Mode::Insert);
    }

    #[test]
    fn paste_over_chars_swaps_into_unnamed_register() {
        let mut e = selecting("one two", pos(0, 4), pos(0, 6), SelectionKind::Char);
        yank(&mut e, Register::charwise("2"));
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "one 2");
        assert_eq!(e.registers.get(None), Some(&Register::charwise("two")));
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn capital_p_keeps_register() {
        let mut e = selecting("one two", pos(0, 0), pos(0, 2), SelectionKind::Char);
        yank(&mut e, Register::charwise("1"));
        run(&mut e, "paste_before", None);
        assert_eq!(text(&e), "1 two");
        assert_eq!(e.registers.get(None), Some(&Register::charwise("1")));
    }

    #[test]
    fn paste_lines_over_lines() {
        let mut e = selecting("a\nb\nc\nd", pos(1, 0), pos(2, 0), SelectionKind::Line);
        yank(&mut e, Register::linewise("X"));
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "a\nX\nd");
        assert_eq!(e.document.cursor(), pos(1, 0));
    }

    #[test]
    fn paste_lines_over_last_lines() {
        let mut e = selecting("a\nb\nc", pos(1, 0), pos(2, 0), SelectionKind::Line);
        yank(&mut e, Register::linewise("X"));
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "a\nX");
    }

    #[test]
    fn paste_lines_over_whole_buffer() {
        let mut e = selecting("a\nb", pos(0, 0), pos(1, 0), SelectionKind::Line);
        yank(&mut e, Register::linewise("X"));
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "X");
    }

    #[test]
    fn paste_chars_over_lines_makes_a_line() {
        let mut e = selecting("a\nb\nc", pos(1, 0), pos(1, 0), SelectionKind::Line);
        yank(&mut e, Register::charwise("X"));
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "a\nX\nc");
    }

    #[test]
    fn paste_lines_over_chars_splits_the_line() {
        let mut e = selecting("abcd", pos(0, 1), pos(0, 2), SelectionKind::Char);
        yank(&mut e, Register::linewise("X"));
        run(&mut e, "paste_after", None);
        assert_eq!(text(&e), "a\nX\nd");
        assert_eq!(e.document.cursor(), pos(1, 0));
    }

    #[test]
    fn paste_with_count_repeats() {
        let mut e = selecting("ab", pos(0, 0), pos(0, 0), SelectionKind::Char);
        yank(&mut e, Register::charwise("x"));
        run(&mut e, "paste_after", Some(3));
        assert_eq!(text(&e), "xxxb");
    }
}
