//! Soft wrapping, end to end: drawing, the cursor, `gj`/`gk`, scrolling.
//! A 30-cell-wide terminal with a 5-cell gutter leaves 25 for text.

mod common;

use common::{app, cursor, keys, render};
use ferra::App;

const LONG: &str = "the quick brown fox jumps over the lazy dog and then some more words";

fn with_text(text: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{text}<Esc>gg0"));
    app
}

/// The text part of each screen row (after the 5-cell gutter).
fn text_rows(app: &mut App, height: u16) -> Vec<String> {
    render(app, 30, height)
        .rows
        .iter()
        .map(|row| {
            row.chars()
                .skip(5)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn long_lines_continue_on_the_next_rows_between_words() {
    let mut app = with_text(&format!("{LONG}<CR>short"));
    let screen = render(&mut app, 30, 8).rows;
    let rows = text_rows(&mut app, 8);
    assert_eq!(rows[0], "the quick brown fox jumps");
    assert_eq!(
        rows[1], "↪ over the lazy dog and",
        "continued rows are marked"
    );
    assert_eq!(rows[2], "↪ then some more words");
    assert_eq!(rows[3], "short");
    assert!(screen[0].starts_with("   1 "), "{:?}", screen[0]);
    assert!(
        screen[1].starts_with("     "),
        "no number on a continued row"
    );
    assert!(
        screen[3].trim_start().starts_with("1 short"),
        "relative number"
    );
}

#[test]
fn the_cursor_is_drawn_on_the_row_its_column_is_on() {
    let mut app = with_text(LONG);
    keys(&mut app, "$");
    let screen = render(&mut app, 30, 8);
    assert_eq!(
        screen.cursor,
        (5 + 2 + 19, 2),
        "the last char, on the third row, after the marker"
    );
    keys(&mut app, "0");
    assert_eq!(render(&mut app, 30, 8).cursor, (5, 0));
}

#[test]
fn nowrap_scrolls_sideways_instead() {
    let mut app = with_text(LONG);
    keys(&mut app, ":set nowrap<CR>$");
    let rows = text_rows(&mut app, 8);
    assert!(rows[0].ends_with("more words"), "{:?}", rows[0]);
    assert_eq!(rows[1], "", "one row for the line");
}

#[test]
fn gj_and_gk_move_by_screen_rows_and_j_by_lines() {
    let mut app = with_text(&format!("{LONG}<CR>short"));
    render(&mut app, 30, 8);
    keys(&mut app, "wwgj");
    // `brown` is at screen column 10; the row below starts at `over`, after
    // the 2-cell marker, so column 10 there is its 8th char.
    assert_eq!(cursor(&app), (0, 34), "column 10 of the second row");
    keys(&mut app, "gj");
    assert_eq!(cursor(&app), (0, 56), "screen column 10 again");
    keys(&mut app, "gj");
    assert_eq!(cursor(&app).0, 1, "on to the next line");
    keys(&mut app, "gkgkgk");
    assert_eq!(cursor(&app), (0, 10), "back where it started");
    keys(&mut app, "j");
    assert_eq!(cursor(&app).0, 1, "j skips the rest of the line");
    keys(&mut app, ":set nowrap<CR>gg0gj");
    assert_eq!(cursor(&app).0, 1, "without wrapping, gj is j");
}

#[test]
fn the_view_scrolls_by_screen_rows_to_keep_the_cursor_in_sight() {
    let lines: Vec<String> = (0..20).map(|i| format!("{i} {LONG}")).collect();
    let mut app = with_text(&lines.join("<CR>"));
    keys(&mut app, ":set scroll-off=0<CR>G$");
    let screen = render(&mut app, 30, 12);
    assert!(
        screen.cursor.1 < 10,
        "the cursor is on screen: {:?}",
        screen.cursor
    );
    let rows = text_rows(&mut app, 12);
    assert!(
        rows.iter().any(|r| r.starts_with("19 ")),
        "the last line is shown: {rows:?}"
    );
    keys(&mut app, "gg");
    assert!(text_rows(&mut app, 12)[0].starts_with("0 the"));
}

#[test]
fn continued_rows_start_at_the_lines_indentation() {
    let mut app = app();
    keys(
        &mut app,
        ":set ai<CR>i    one two three four five six seven eight nine<Esc>",
    );
    let rows = text_rows(&mut app, 8);
    // 25 cells, then 19 after the indentation and the marker.
    assert_eq!(rows[0], "    one two three four");
    assert_eq!(rows[1], "    ↪ five six seven");
    assert_eq!(rows[2], "    ↪ eight nine");
    keys(&mut app, ":setlocal nowrap-indent wrap-marker=<CR>");
    let rows = text_rows(&mut app, 8);
    assert_eq!(
        rows[1], "five six seven eight nine",
        "both off: all 25 cells"
    );
}

#[test]
fn virtual_text_takes_room_in_wrapped_lines_and_gj_steps_over_it() {
    // A line that fits, with hints that push it onto more rows.
    let mut app = with_text("let value = compute(1, 2);<CR>next");
    let hint = |col, text: &str| ferra::editor::VirtualText {
        at: ferra::core::Position::new(0, col),
        text: text.into(),
        style: "ui.virtual.inlay-hint".into(),
        source: "inlay".into(),
        end_of_line: false,
    };
    app.editor_mut().document.set_virtual_text(
        "inlay",
        vec![hint(9, ": Result<u32, Error>"), hint(20, "first: ")],
    );
    let rows = text_rows(&mut app, 6);
    assert!(rows[0].starts_with("let value: Result<u32,"), "{rows:?}");
    assert!(
        rows[1].contains("compute(first:"),
        "wrapped, hints and all: {rows:?}"
    );
    assert!(rows[2].contains("1, 2);"), "{rows:?}");
    assert!(rows[3].starts_with("next"), "{rows:?}");
    // gj steps through the line's rows, then on to the next line.
    keys(&mut app, "gg0gjgj");
    assert_eq!(cursor(&app).0, 0, "the third row of the same line");
    keys(&mut app, "gj");
    assert_eq!(cursor(&app).0, 1);
    keys(&mut app, "gkgkgk");
    assert_eq!(cursor(&app), (0, 0));
}
