//! Scrolling the view: `<C-d>`/`<C-u>`, `<C-f>`/`<C-b>`, `<C-e>`/`<C-y>`,
//! `zz`/`zt`/`zb`, and the screen-line motions `H`, `M`, `L`.
//!
//! `<C-d>`/`<C-u>` move the cursor with the view; `<C-f>`/`<C-b>` put it at
//! the new screen's top (or bottom), as in Vim. VS Code's PageDown/PageUp
//! move it with the view, a page at a time.
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
use crate::core::columns::{char_col_at, display_col};
use crate::core::wrap;
use crate::editor::Editor;
use crate::editor::windows::TextLayout;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "scroll_half_page_down",
        "Scroll down half a screen, and the cursor with it ([count] lines)",
        |cx| {
            let n = cx.count.unwrap_or_else(|| half(cx.editor));
            scroll_with_cursor(cx.editor, signed(n));
            Ok(())
        },
    );
    r.register(
        "scroll_half_page_up",
        "Scroll up half a screen, and the cursor with it ([count] lines)",
        |cx| {
            let n = cx.count.unwrap_or_else(|| half(cx.editor));
            scroll_with_cursor(cx.editor, -signed(n));
            Ok(())
        },
    );
    r.register(
        "scroll_page_down",
        "Scroll down a screen (less two lines), the cursor to its top (<C-f>)",
        |cx| {
            let n = page(cx.editor).saturating_mul(cx.count());
            scroll_page(cx.editor, signed(n));
            Ok(())
        },
    );
    r.register(
        "scroll_page_up",
        "Scroll up a screen (less two lines), the cursor to its bottom (<C-b>)",
        |cx| {
            let n = page(cx.editor).saturating_mul(cx.count());
            scroll_page(cx.editor, -signed(n));
            Ok(())
        },
    );
    r.register(
        "scroll_page_down_with_cursor",
        "Scroll down a screen (less two lines), and the cursor with it",
        |cx| {
            let n = page(cx.editor).saturating_mul(cx.count());
            scroll_with_cursor(cx.editor, signed(n));
            Ok(())
        },
    );
    r.register(
        "scroll_page_up_with_cursor",
        "Scroll up a screen (less two lines), and the cursor with it",
        |cx| {
            let n = page(cx.editor).saturating_mul(cx.count());
            scroll_with_cursor(cx.editor, -signed(n));
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

    r.register(
        "move_screen_down",
        "Move down [count] screen rows, through the rows of wrapped lines (gj)",
        |cx| {
            move_screen_rows(cx.editor, cx.count(), true);
            Ok(())
        },
    );
    r.register(
        "move_screen_up",
        "Move up [count] screen rows, through the rows of wrapped lines (gk)",
        |cx| {
            move_screen_rows(cx.editor, cx.count(), false);
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

/// `gj`, `gk`: moves the cursor `count` screen rows down (or up), keeping
/// its column on screen. A wrapped line's rows count separately; without
/// wrapping this is `j`/`k`.
fn move_screen_rows(editor: &mut Editor, count: usize, down: bool) {
    if !editor.options.wrap {
        // `j`/`k`, stepping over closed folds as over one line.
        let doc = &editor.document;
        let row = doc.cursor().row;
        let target = doc.step_visible_lines(row, count, down);
        editor.move_vertical(target as isize - row as isize);
        return;
    }
    let windows = editor.windows();
    let layout = TextLayout::new(
        windows.width(windows.active()),
        &editor.document,
        &editor.options,
    );
    let tab = editor.options.tab_width;
    let doc = &editor.document;
    let buffer = doc.buffer();
    let Position { row, col } = doc.cursor();
    // Lines as they're drawn, with any virtual text (inlay hints) in them,
    // and columns mapped to and from them.
    let drawn = |row: usize| -> (String, Option<crate::editor::DisplayLine>) {
        match doc.display_line(row) {
            Some(display) => (display.text.clone(), Some(display)),
            None => (buffer.line(row).into_owned(), None),
        }
    };
    let (line, display) = drawn(row);
    let col = display.as_ref().map_or(col, |d| d.to_display(col));
    let starts = layout.row_starts(&line);
    // A closed fold is one row.
    let part = if doc.closed_fold_at(row).is_some() {
        0
    } else {
        wrap::row_of(&starts, col)
    };
    // The column on screen, counting a continued row's indentation.
    let screen_col = editor.wanted_screen_column().unwrap_or_else(|| {
        layout.offset(&line, part) + display_col(&line, col, tab)
            - display_col(&line, starts[part], tab)
    });

    let (mut row, mut part) = (row, part);
    for _ in 0..count {
        let parts = layout.rows_in(doc, row);
        if down {
            if part + 1 < parts {
                part += 1;
            } else if let Some(next) = doc.next_visible_line(row) {
                row = next;
                part = 0;
            } else {
                break;
            }
        } else if part > 0 {
            part -= 1;
        } else if let Some(previous) = doc.previous_visible_line(row) {
            row = previous;
            part = layout.rows_in(doc, row).saturating_sub(1);
        } else {
            break;
        }
    }
    let (line, display) = drawn(row);
    let starts = layout.row_starts(&line);
    let part = part.min(starts.len() - 1);
    let left = display_col(&line, starts[part], tab);
    let offset = layout.offset(&line, part);
    let mut col = char_col_at(&line, left + screen_col.saturating_sub(offset), tab);
    // Stay on this row: not past its last char onto the next one.
    if let Some(&next) = starts.get(part + 1) {
        col = col.min(next.saturating_sub(1));
    }
    let col = display.as_ref().map_or(col, |d| d.to_real(col));
    editor.document.set_cursor(Position::new(row, col));
    // Normal mode can't sit past the last char; settle that now, so the
    // position recorded is where the cursor ends up.
    if editor.mode() == crate::editor::Mode::Normal {
        editor.document.clamp_to_last_char();
    }
    editor.set_wanted_screen_column(screen_col);
}

/// The active window's first line on screen and its height in rows.
fn view(editor: &Editor) -> (usize, usize) {
    let windows = editor.windows();
    let id = windows.active();
    let doc = &editor.document;
    let top = windows.view_top(id, doc, doc.cursor(), &editor.options);
    (top, windows.height(id))
}

/// The last line at least partly on screen when the view starts at `top`.
fn bottom(editor: &Editor, top: usize, height: usize) -> usize {
    let windows = editor.windows();
    let layout = TextLayout::new(
        windows.width(windows.active()),
        &editor.document,
        &editor.options,
    );
    layout.bottom(&editor.document, top, height)
}

fn half(editor: &Editor) -> usize {
    (view(editor).1 / 2).max(1)
}

fn page(editor: &Editor) -> usize {
    view(editor).1.saturating_sub(2).max(1)
}

/// A count of lines as a signed distance, a huge one (a count can
/// saturate at `usize::MAX`) capped rather than wrapped negative.
fn signed(lines: usize) -> isize {
    isize::try_from(lines).unwrap_or(isize::MAX)
}

fn last_row(editor: &Editor) -> usize {
    editor.document.buffer().line_count() - 1
}

/// `<C-d>`, `<C-u>` (and VS Code's PageDown, PageUp): moves the view and
/// the cursor `lines` (down if positive), as far as the buffer allows,
/// keeping the cursor's column.
fn scroll_with_cursor(editor: &mut Editor, lines: isize) {
    scroll(editor, lines, lines);
}

/// `<C-e>`, `<C-y>`: moves the view `lines`; the cursor moves only as far
/// as it must to stay in sight.
fn scroll_view(editor: &mut Editor, lines: isize) {
    scroll(editor, lines, 0);
}

/// `<C-f>`, `<C-b>`: moves the view `lines` (down if positive), and the
/// cursor to the new screen's top (or, up, its bottom), out of the
/// `scroll-off` margin, keeping its column. As in Vim, when the view
/// can't move the cursor stays.
fn scroll_page(editor: &mut Editor, lines: isize) {
    let top = view(editor).0;
    let (new_top, bounds) = move_view(editor, lines);
    if new_top == top {
        return;
    }
    let row = editor.document.cursor().row;
    let wanted = match bounds {
        Bounds::Fit { lowest, highest } => {
            if lines > 0 {
                lowest
            } else {
                highest
            }
        }
        Bounds::Cramped { top, bottom } => row.clamp(top, bottom),
    };
    if wanted != row {
        editor.move_vertical(wanted as isize - row as isize);
    }
}

/// Moves the view `view_lines` and the cursor `cursor_lines`, then the
/// cursor further if that left it in the `scroll-off` margin, as Vim does
/// (moving the view back instead would undo the scroll).
fn scroll(editor: &mut Editor, view_lines: isize, cursor_lines: isize) {
    let (_, bounds) = move_view(editor, view_lines);
    let last = last_row(editor);
    let row = editor.document.cursor().row;
    let moved = row.saturating_add_signed(cursor_lines).min(last);
    let wanted = match bounds {
        Bounds::Fit { lowest, highest } => moved.clamp(lowest, highest),
        Bounds::Cramped { top, bottom } => moved.clamp(top, bottom),
    };
    if wanted != row {
        editor.move_vertical(wanted as isize - row as isize);
    }
}

/// Asks for the view to move `lines` (down if positive), as far as the
/// buffer allows, and returns its new top with where the cursor can be
/// there (see [`cursor_bounds`]).
fn move_view(editor: &mut Editor, lines: isize) -> (usize, Bounds) {
    let top = view(editor).0;
    let new_top = top.saturating_add_signed(lines).min(last_row(editor));
    editor
        .windows()
        .request_top(editor.windows().active(), new_top);
    (new_top, cursor_bounds(editor, new_top))
}

/// Where the cursor can be in a view, from [`cursor_bounds`].
enum Bounds {
    /// Lines `lowest` to `highest` keep the view where it is.
    Fit { lowest: usize, highest: usize },
    /// No line does (near the end of the buffer, the margin wants rows
    /// past it; or a line is taller than the window): the next draw
    /// scrolls whatever happens, so the cursor just stays on screen,
    /// lines `top` to `bottom`.
    Cramped { top: usize, bottom: usize },
}

/// The lines the cursor can be on in a view starting at `top` without the
/// next draw scrolling it: out of the `scroll-off` margin, which is screen
/// rows (a wrapped line takes several, a closed fold one), except at the
/// start and end of the buffer. Each line is checked with the draw's own
/// [`follow_cursor`], so the two agree.
///
/// [`follow_cursor`]: crate::editor::windows::follow_cursor
fn cursor_bounds(editor: &Editor, top: usize) -> Bounds {
    let windows = editor.windows();
    let id = windows.active();
    let height = windows.height(id);
    let doc = &editor.document;
    let layout = TextLayout::new(windows.width(id), doc, &editor.options);
    let line_count = doc.buffer().line_count();
    // Whether the view stays at `top` with the cursor at the start of `row`.
    let stays = |row: usize| {
        crate::editor::windows::follow_cursor(
            top,
            (row, 0),
            height,
            line_count,
            editor.options.scroll_off,
            |line| layout.rows_in(doc, line),
        ) == top
    };
    let bottom = layout.bottom(doc, top, height);
    let mut lowest = top;
    while !stays(lowest) {
        match doc.next_visible_line(lowest) {
            Some(next) if next <= bottom => lowest = next,
            _ => return Bounds::Cramped { top, bottom },
        }
    }
    let mut highest = doc
        .closed_fold_at(bottom)
        .map_or(bottom, |(first, _)| first);
    while highest > lowest && !stays(highest) {
        match doc.previous_visible_line(highest) {
            Some(previous) if previous >= lowest => highest = previous,
            _ => highest = lowest,
        }
    }
    Bounds::Fit { lowest, highest }
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
        editor.move_vertical(row as isize - editor.document.cursor().row as isize);
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
    let bottom = bottom(editor, top, height);
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
