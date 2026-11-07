//! Scrolling the view, end to end: the view (what's drawn) and the cursor.
//! A 40x12 terminal shows 10 rows of text; `scroll-off` is 5, which in
//! 10 rows is 4 (less than half).

mod common;

use common::{app, cursor, keys, render};
use ferra::App;

/// A 100-line buffer (`line 0` to `line 99`), drawn once, cursor on top.
fn hundred_lines() -> App {
    let mut app = app();
    let text: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
    keys(&mut app, &format!("i{}<Esc>gg", text.join("<CR>")));
    render(&mut app, 40, 12);
    app
}

/// The buffer line drawn on the first row of text.
fn first_shown(app: &mut App) -> usize {
    let rows = render(app, 40, 12).rows;
    let text = rows[0].split("line ").nth(1).unwrap();
    text.trim().parse().unwrap()
}

#[test]
fn half_pages_move_the_view_and_the_cursor() {
    let mut app = hundred_lines();
    keys(&mut app, "<C-d>");
    assert_eq!(first_shown(&mut app), 5);
    assert_eq!(cursor(&app).0, 9, "moved 5, then out of the margin");
    keys(&mut app, "<C-d>");
    assert_eq!(first_shown(&mut app), 10);
    assert_eq!(cursor(&app).0, 14);
    keys(&mut app, "<C-u>");
    assert_eq!(first_shown(&mut app), 5);
    assert_eq!(cursor(&app).0, 9);
    keys(&mut app, "3<C-d>");
    assert_eq!(first_shown(&mut app), 8, "a count is the number of lines");
}

#[test]
fn pages_keep_two_lines_of_overlap() {
    let mut app = hundred_lines();
    keys(&mut app, "<C-f>");
    assert_eq!(first_shown(&mut app), 8);
    keys(&mut app, "<PageDown>");
    assert_eq!(first_shown(&mut app), 16);
    keys(&mut app, "<C-b>");
    assert_eq!(first_shown(&mut app), 8);
    keys(&mut app, "<C-b><C-b>");
    assert_eq!(first_shown(&mut app), 0);
    assert_eq!(cursor(&app).0, 0);
}

#[test]
fn line_scrolling_moves_the_cursor_only_to_keep_it_in_sight() {
    let mut app = hundred_lines();
    keys(&mut app, "20G");
    let top = first_shown(&mut app);
    keys(&mut app, "<C-e>");
    assert_eq!(first_shown(&mut app), top + 1);
    assert_eq!(cursor(&app).0, 19, "still in sight");
    keys(&mut app, "5<C-e>");
    assert_eq!(first_shown(&mut app), top + 6);
    assert_eq!(cursor(&app).0, top + 6 + 4, "pushed down by the margin");
    keys(&mut app, "<C-y>");
    assert_eq!(first_shown(&mut app), top + 5);
}

#[test]
fn z_commands_put_the_cursor_line_at_the_top_middle_or_bottom() {
    let mut app = hundred_lines();
    keys(&mut app, "50Gzt");
    assert_eq!(first_shown(&mut app), 45, "the top, less the margin");
    keys(&mut app, "zz");
    assert_eq!(first_shown(&mut app), 45, "49 in the middle of 10 rows");
    keys(&mut app, "zb");
    assert_eq!(first_shown(&mut app), 44, "the bottom, less the margin");
    keys(&mut app, "30zz");
    assert_eq!(cursor(&app).0, 29, "a count picks the line");
    assert_eq!(first_shown(&mut app), 25);
    keys(&mut app, "$z<CR>");
    assert_eq!(cursor(&app), (29, 0), "to the first non-blank");
}

#[test]
fn h_m_and_l_go_to_lines_on_screen() {
    let mut app = hundred_lines();
    keys(&mut app, "50Gzz");
    let top = first_shown(&mut app);
    keys(&mut app, "H");
    assert_eq!(cursor(&app).0, top + 4, "kept out of the margin");
    keys(&mut app, "L");
    assert_eq!(cursor(&app).0, top + 9 - 4);
    keys(&mut app, "M");
    assert_eq!(cursor(&app).0, top + 4);
    keys(&mut app, "gg");
    render(&mut app, 40, 12);
    keys(&mut app, "3H");
    assert_eq!(cursor(&app).0, 2, "at the start the margin doesn't apply");
}

#[test]
fn h_m_and_l_are_motions_for_operators() {
    let mut app = hundred_lines();
    keys(&mut app, "dL");
    // From line 0 to the lowest line out of the margin (row 5 of 10).
    assert_eq!(common::text(&app).lines().next(), Some("line 6"));
    keys(&mut app, "u");
    assert_eq!(common::text(&app).lines().next(), Some("line 0"));
}

#[test]
fn before_anything_is_drawn_commands_still_work() {
    let mut app = app();
    let text: Vec<String> = (0..100).map(|i| format!("{i}")).collect();
    keys(&mut app, &format!("i{}<Esc>gg<C-d>", text.join("<CR>")));
    assert!(cursor(&app).0 > 0);
}
