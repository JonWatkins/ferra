//! Replace mode (`R`), end to end.

mod common;

use common::{app, cursor, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn typing_takes_the_place_of_the_text() {
    let mut app = with_text("abcdef");
    keys(&mut app, "Rxy<Esc>");
    assert_eq!(text(&app), "xycdef");
    assert_eq!(cursor(&app), (0, 1));
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "u");
    assert_eq!(text(&app), "abcdef", "one undo step");
}

#[test]
fn past_the_end_of_the_line_it_adds() {
    let mut app = with_text("ab");
    keys(&mut app, "Rwxyz<Esc>");
    assert_eq!(text(&app), "wxyz");
}

#[test]
fn backspace_puts_back_what_was_typed_over() {
    let mut app = with_text("abcdef");
    keys(&mut app, "Rxyz<BS><BS><Esc>");
    assert_eq!(text(&app), "xbcdef");
    // Over text not typed this time, it only moves left.
    keys(&mut app, "0lllRx<BS><BS>q<Esc>");
    assert_eq!(text(&app), "xbqdef");
}

#[test]
fn a_count_types_it_again() {
    let mut app = with_text("abcdefgh");
    keys(&mut app, "3Rab<Esc>");
    assert_eq!(text(&app), "abababgh");
}

#[test]
fn enter_breaks_the_line_without_deleting() {
    let mut app = with_text("abcd");
    keys(&mut app, "lRx<CR>y<Esc>");
    assert_eq!(text(&app), "ax\nyd");
}

#[test]
fn dot_repeats_it() {
    let mut app = with_text("aaaa<CR>aaaa");
    keys(&mut app, "ggRxy<Esc>j0.");
    assert_eq!(text(&app), "xyaa\nxyaa");
}

#[test]
fn a_paste_types_over_too() {
    let mut app = with_text("abcdef");
    keys(&mut app, "R");
    app.handle_paste("XY");
    keys(&mut app, "<Esc>");
    assert_eq!(text(&app), "XYcdef");
}

#[test]
fn the_status_line_says_replace() {
    let mut app = with_text("abc");
    keys(&mut app, "R");
    let screen = render(&mut app, 40, 5);
    assert!(
        screen.rows.iter().any(|r| r.contains("REPLACE")),
        "{:?}",
        screen.rows
    );
    keys(&mut app, "<Esc>i");
    let screen = render(&mut app, 40, 5);
    assert!(
        screen.rows.iter().any(|r| r.contains("INSERT")),
        "plain insert after"
    );
}
