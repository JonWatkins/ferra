//! Folding: `za`, `zc`, `zo`, `zR`, `zM`, `zf`, `zd`, `zE`, `zj`, `zk`.
//!
//! Folds come from indentation (see [`core::fold`](crate::core::fold)):
//! the fold at the cursor is the one its line heads, or else the one it's
//! inside. `zf` makes one over any motion.

use super::operators::begin;
use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::fold::{region_around, region_from};
use crate::core::{Position, motion};
use crate::editor::{Editor, Operator};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "fold_toggle",
        "Open the closed fold at the cursor, or close the fold it's in (za)",
        |cx| {
            let row = cx.editor.document.cursor().row;
            if !cx.editor.document.open_fold_at(row) {
                close_here(cx.editor)?;
            }
            Ok(())
        },
    );
    r.register("fold_close", "Close the fold the cursor is in (zc)", |cx| {
        close_here(cx.editor)
    });
    r.register(
        "fold_open",
        "Open the closed fold at the cursor (zo, zd)",
        |cx| {
            let row = cx.editor.document.cursor().row;
            cx.editor.document.open_fold_at(row);
            Ok(())
        },
    );
    r.register("fold_open_all", "Open every fold (zR, zE)", |cx| {
        cx.editor.document.open_all_folds();
        Ok(())
    });
    r.register("fold_close_all", "Close every outermost fold (zM)", |cx| {
        close_all(cx.editor);
        Ok(())
    });
    r.register(
        "operator_fold",
        "Fold the lines of a motion or text object (zf, zfip, visual zf)",
        |cx| begin(cx, Operator::Fold),
    );
    r.register(
        "fold_next",
        "Go to the start of the next fold below (zj)",
        |cx| goto_fold(cx, true),
    );
    r.register(
        "fold_previous",
        "Go to the start of the previous fold above (zk)",
        |cx| goto_fold(cx, false),
    );
}

fn close_here(editor: &mut Editor) -> CommandResult {
    let tab = editor.options.tab_width.max(1);
    let doc = &mut editor.document;
    let row = doc.cursor().row;
    let (first, last) = region_around(doc.buffer(), row, tab).ok_or("No fold here")?;
    doc.close_fold(first, last);
    let col = motion::first_non_blank(doc.buffer(), first);
    doc.set_cursor(Position::new(first, col));
    Ok(())
}

/// `zM`: closes each outermost fold in the buffer.
fn close_all(editor: &mut Editor) {
    let tab = editor.options.tab_width.max(1);
    let doc = &mut editor.document;
    let count = doc.buffer().line_count();
    let mut row = 0;
    while row < count {
        match region_from(doc.buffer(), row, tab) {
            Some((first, last)) => {
                doc.close_fold(first, last);
                row = last + 1;
            }
            None => row += 1,
        }
    }
    editor.enforce_cursor_rules();
}

/// `zj`/`zk`: the next line below (or above) that starts a fold.
fn goto_fold(cx: &mut Context, down: bool) -> CommandResult {
    let tab = cx.editor.options.tab_width.max(1);
    let count = cx.count();
    let doc = &mut cx.editor.document;
    let mut row = doc.cursor().row;
    for _ in 0..count {
        let next = if down {
            (row + 1..doc.buffer().line_count())
                .find(|&r| region_from(doc.buffer(), r, tab).is_some())
        } else {
            (0..row)
                .rev()
                .find(|&r| region_from(doc.buffer(), r, tab).is_some())
        };
        match next {
            Some(next) => row = next,
            None => break,
        }
    }
    let col = motion::first_non_blank(doc.buffer(), row);
    doc.set_cursor(Position::new(row, col));
    Ok(())
}
