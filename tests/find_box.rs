//! The find box doesn't hide the match it found.

mod common;

use common::{app_with_config, keys, render};

/// 100 lines, `l<n>` at their start; line `needle_at` has `needle` far
/// enough right to be under the box.
fn lines(needle_at: usize) -> String {
    (0..100)
        .map(|n| {
            let pad = " ".repeat(40);
            if n == needle_at {
                format!("l{n}{pad}needle\n")
            } else {
                format!("l{n}\n")
            }
        })
        .collect()
}

fn app_with(text: &str) -> ferra::App {
    let mut app = app_with_config("[editor]\nscroll-off = 0\nline-numbers = \"none\"");
    app.editor_mut()
        .document
        .insert(ferra::core::Position::new(0, 0), text);
    app.editor_mut()
        .document
        .set_cursor(ferra::core::Position::new(0, 0));
    app
}

/// The screen row showing line `n` (its `l<n>` at the start).
fn row_of(rows: &[String], n: usize) -> Option<usize> {
    let label = format!("l{n} ");
    rows.iter().position(|row| row.starts_with(&label))
}

#[test]
fn the_window_scrolls_the_match_out_from_under_the_box() {
    let mut app = app_with(&lines(49));
    // Line 50 at the window's top, where the box goes.
    keys(&mut app, "50Gzt");
    let screen = render(&mut app, 80, 24);
    assert_eq!(row_of(&screen.rows, 49), Some(0), "{:#?}", screen.rows);

    keys(&mut app, "gg<Space>/needle");
    let screen = render(&mut app, 80, 24);
    let row = row_of(&screen.rows, 49).expect("the match's line is shown");
    assert_eq!(row, 5, "just below the box: {:#?}", screen.rows);
    assert!(
        screen.rows[1].contains("Find"),
        "the box is at the top: {:#?}",
        screen.rows
    );
}

#[test]
fn at_the_buffers_start_the_box_goes_to_the_bottom() {
    let mut app = app_with(&lines(1));
    keys(&mut app, "<Space>/needle");
    let screen = render(&mut app, 80, 24);
    assert_eq!(row_of(&screen.rows, 1), Some(1), "{:#?}", screen.rows);
    assert!(
        screen.rows[1].contains("needle"),
        "the match shows: {:#?}",
        screen.rows
    );
    let find_row = screen
        .rows
        .iter()
        .position(|row| row.contains("│ Find"))
        .expect("the box is drawn");
    assert!(find_row > 10, "at the bottom: {:#?}", screen.rows);
}

#[test]
fn a_match_left_of_the_box_leaves_it_at_the_top() {
    let mut app = app_with("needle\nhay\n");
    keys(&mut app, "<Space>/needle");
    let screen = render(&mut app, 120, 24);
    assert!(
        screen.rows[1].contains("│ Find"),
        "the box stays at the top: {:#?}",
        screen.rows
    );
}

#[test]
fn a_paste_with_line_breaks_finds_across_lines() {
    let mut app = app_with("fn a()\n{\n}\nfn b() {}\n");
    keys(&mut app, "<Space>/");
    app.handle_paste(")\r\n{");
    assert_eq!(app.editor().find_state().find.text(), ")\n{");
    assert_eq!(common::cursor(&app), (0, 5), "on the match");
    let screen = render(&mut app, 60, 10);
    let find_row = screen
        .rows
        .iter()
        .find(|row| row.contains("Find"))
        .expect("the box");
    assert!(find_row.contains(")↵{"), "the line break shown: {find_row}");
    assert!(find_row.contains("1 of 1"), "{find_row}");
}
