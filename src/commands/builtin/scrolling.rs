//! Scrolling the view: `<C-d>`/`<C-u>`, `<C-f>`/`<C-b>`, `<C-e>`/`<C-y>`,
//! `zz`/`zt`/`zb`, and the screen-line motions `H`, `M`, `L`.
//!
//! The view's first row lives in the UI; [`Windows::view_top`] reports it
//! (as the next draw will have it) and [`Windows::request_top`] asks the UI
//! to move it at the next draw, where it's still scrolled to keep the
//! cursor in sight, `scroll-off` rows from the edge.
//!
//! [`Windows::view_top`]: crate::editor::Windows::view_top
//! [`Windows::request_top`]: crate::editor::Windows::request_top

use super::motions::{self, MotionArgs, SCREEN_LINE};
use crate::commands::CommandRegistry;
use crate::core::Position;
use crate::editor::Editor;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "scroll_half_page_down",
        "Scroll down half a screen, and the cursor with it ([count] lines)",
        |cx| {
            let n = cx.count.unwrap_or_else(|| half(cx.editor));
            scroll_with_cursor(cx.editor, n as isize);
            Ok(())
        },
    );
    r.register(
        "scroll_half_page_up",
        "Scroll up half a screen, and the cursor with it ([count] lines)",
        |cx| {
            let n = cx.count.unwrap_or_else(|| half(cx.editor));
            scroll_with_cursor(cx.editor, -(n as isize));
            Ok(())
        },
    );
    r.register(
        "scroll_page_down",
        "Scroll down a screen (less two lines), and the cursor with it",
        |cx| {
            let n = page(cx.editor) * cx.count();
            scroll_with_cursor(cx.editor, n as isize);
            Ok(())
        },
    );
    r.register(
        "scroll_page_up",
        "Scroll up a screen (less two lines), and the cursor with it",
        |cx| {
            let n = page(cx.editor) * cx.count();
            scroll_with_cursor(cx.editor, -(n as isize));
            Ok(())
        },
    );
    r.register(
        "scroll_line_down",
        "Scroll down [count] lines, moving the cursor only to keep it in sight",
        |cx| {
            scroll_view(cx.editor, cx.count() as isize);
            Ok(())
        },
    );
    r.register(
        "scroll_line_up",
        "Scroll up [count] lines, moving the cursor only to keep it in sight",
        |cx| {
            scroll_view(cx.editor, -(cx.count() as isize));
            Ok(())
        },
    );

    for (name, description, place, first_non_blank) in [
        (
            "view_center",
            "Scroll the cursor line (or line [count]) to the middle (zz)",
            Place::Middle,
            false,
        ),
        (
            "view_top",
            "Scroll the cursor line (or line [count]) to the top (zt)",
            Place::Top,
            false,
        ),
        (
            "view_bottom",
            "Scroll the cursor line (or line [count]) to the bottom (zb)",
            Place::Bottom,
            false,
        ),
        (
            "view_center_first_non_blank",
            "Like zz, and go to the first non-blank (z.)",
            Place::Middle,
            true,
        ),
        (
            "view_top_first_non_blank",
            "Like zt, and go to the first non-blank (z<CR>)",
            Place::Top,
            true,
        ),
        (
            "view_bottom_first_non_blank",
            "Like zb, and go to the first non-blank (z-)",
            Place::Bottom,
            true,
        ),
    ] {
        r.register(name, description, move |cx| {
            place_cursor_line(cx.editor, cx.count, place, first_non_blank);
            Ok(())
        });
    }

    for (name, description, line) in [
        (
            "goto_screen_top",
            "Go to the top line on screen ([count]th from the top) (H)",
            ScreenLine::Top,
        ),
        (
            "goto_screen_middle",
            "Go to the middle line on screen (M)",
            ScreenLine::Middle,
        ),
        (
            "goto_screen_bottom",
            "Go to the bottom line on screen ([count]th from the bottom) (L)",
            ScreenLine::Bottom,
        ),
    ] {
        r.register(name, description, move |cx| {
            let count = cx
                .count
                .or_else(|| cx.editor.pending_operator().and_then(|p| p.count));
            let row = screen_line(cx.editor, line, count.unwrap_or(1).max(1));
            let args = MotionArgs {
                target: Some(Position::new(row, 0)),
                ..MotionArgs::default()
            };
            motions::run(cx, &SCREEN_LINE, args)
        });
    }
}

/// The active window's first row on screen and its height.
fn view(editor: &Editor) -> (usize, usize) {
    let windows = editor.windows();
    let id = windows.active();
    let doc = &editor.document;
    let top = windows.view_top(
        id,
        doc.cursor().row,
        doc.buffer().line_count(),
        editor.options.scroll_off,
    );
    (top, windows.height(id))
}

fn half(editor: &Editor) -> usize {
    (view(editor).1 / 2).max(1)
}

fn page(editor: &Editor) -> usize {
    view(editor).1.saturating_sub(2).max(1)
}

fn last_row(editor: &Editor) -> usize {
    editor.document.buffer().line_count() - 1
}

/// `<C-d>`, `<C-f>`...: moves the view and the cursor `lines` (down if
/// positive), as far as the buffer allows, keeping the cursor's column.
fn scroll_with_cursor(editor: &mut Editor, lines: isize) {
    scroll(editor, lines, lines);
}

/// `<C-e>`, `<C-y>`: moves the view `lines`; the cursor moves only as far
/// as it must to stay in sight.
fn scroll_view(editor: &mut Editor, lines: isize) {
    scroll(editor, lines, 0);
}

/// Moves the view `view_lines` and the cursor `cursor_lines`, then the
/// cursor further if that left it in the `scroll-off` margin, as Vim does
/// (moving the view back instead would undo the scroll).
fn scroll(editor: &mut Editor, view_lines: isize, cursor_lines: isize) {
    let (top, height) = view(editor);
    let last = last_row(editor);
    let new_top = top.saturating_add_signed(view_lines).min(last);
    editor
        .windows()
        .request_top(editor.windows().active(), new_top);
    let margin = editor.options.scroll_off.min(height.saturating_sub(1) / 2);
    let bottom = (new_top + height).saturating_sub(1).min(last);
    let lowest = if new_top == 0 { 0 } else { new_top + margin };
    let highest = if bottom == last {
        last
    } else {
        bottom.saturating_sub(margin)
    };
    let row = editor.document.cursor().row;
    let moved = row.saturating_add_signed(cursor_lines).min(last);
    let wanted = moved.clamp(lowest.min(highest), highest);
    if wanted != row {
        editor
            .document
            .move_vertical(wanted as isize - row as isize);
    }
}

#[derive(Clone, Copy)]
enum Place {
    Top,
    Middle,
    Bottom,
}

/// `zt`, `zz`, `zb` (and `z<CR>`, `z.`, `z-`): scrolls so the cursor's line
/// (or line `count`, which the cursor goes to) is at the top, middle or
/// bottom. Drawing keeps `scroll-off` rows around it, as in Vim.
fn place_cursor_line(
    editor: &mut Editor,
    count: Option<usize>,
    place: Place,
    first_non_blank: bool,
) {
    if let Some(line) = count {
        let row = line.saturating_sub(1).min(last_row(editor));
        let doc = &mut editor.document;
        doc.move_vertical(row as isize - doc.cursor().row as isize);
    }
    if first_non_blank {
        let doc = &mut editor.document;
        let row = doc.cursor().row;
        let col = crate::core::motion::first_non_blank(doc.buffer(), row);
        doc.set_cursor(Position::new(row, col));
    }
    let (_, height) = view(editor);
    let row = editor.document.cursor().row;
    let top = match place {
        Place::Top => row,
        Place::Middle => row.saturating_sub(height.saturating_sub(1) / 2),
        Place::Bottom => (row + 1).saturating_sub(height),
    };
    editor.windows().request_top(editor.windows().active(), top);
}

#[derive(Clone, Copy)]
enum ScreenLine {
    Top,
    Middle,
    Bottom,
}

/// The row `H` (`count`th from the top), `M` or `L` (`count`th from the
/// bottom) goes to: never into the `scroll-off` margin, unless that's the
/// start or end of the buffer, since going there would scroll.
fn screen_line(editor: &Editor, line: ScreenLine, count: usize) -> usize {
    let (top, height) = view(editor);
    let last = last_row(editor);
    let bottom = (top + height).saturating_sub(1).min(last);
    let margin = editor.options.scroll_off.min(height.saturating_sub(1) / 2);
    let lowest = if top == 0 { 0 } else { top + margin };
    let highest = if bottom == last {
        last
    } else {
        bottom.saturating_sub(margin)
    };
    let row = match line {
        ScreenLine::Top => (top + count - 1).max(lowest),
        ScreenLine::Middle => top + (bottom - top) / 2,
        ScreenLine::Bottom => bottom.saturating_sub(count - 1).min(highest),
    };
    row.clamp(top, bottom)
}
