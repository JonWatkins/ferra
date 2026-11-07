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
    assert_eq!(rows[1], "over the lazy dog and");
    assert_eq!(rows[2], "then some more words");
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
        (5 + 19, 2),
        "the last char, on the third row"
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
    // `brown` is at screen column 10; the row below starts at `over`.
    assert_eq!(cursor(&app), (0, 36), "column 10 of the second row");
    keys(&mut app, "gj");
    assert_eq!(cursor(&app), (0, 58));
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
