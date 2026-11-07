//! `<C-a>` and `<C-x>`, end to end.

mod common;

use common::{app, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn the_number_under_or_after_the_cursor() {
    let mut app = with_text("x = 41;");
    keys(&mut app, "<C-a>");
    assert_eq!(text(&app), "x = 42;");
    assert_eq!(cursor(&app), (0, 5), "on its last digit");
    keys(&mut app, "5<C-x>");
    assert_eq!(text(&app), "x = 37;");
    keys(&mut app, ".");
    assert_eq!(text(&app), "x = 32;", ". repeats with the count");
    keys(&mut app, "uu");
    assert_eq!(text(&app), "x = 42;");
    keys(&mut app, "0100<C-x>");
    assert_eq!(text(&app), "x = -58;");
}

#[test]
fn no_number_says_so() {
    let mut app = with_text("nothing here");
    keys(&mut app, "<C-a>");
    assert_eq!(message(&app), "No number under or after the cursor");
    assert_eq!(text(&app), "nothing here");
}

#[test]
fn visual_lines_and_sequences() {
    let mut app = with_text("0.<CR>0.<CR>0.");
    keys(&mut app, "VG<C-a>");
    assert_eq!(text(&app), "1.\n1.\n1.");
    assert_eq!(cursor(&app), (0, 0), "back to the start");
    keys(&mut app, "VGg<C-a>");
    assert_eq!(text(&app), "2.\n3.\n4.", "+1, +2, +3");
    keys(&mut app, "VG2g<C-x>");
    assert_eq!(text(&app), "0.\n-1.\n-2.", "-2, -4, -6");
}

#[test]
fn only_numbers_inside_the_selection() {
    let mut app = with_text("10 20<CR>10 20");
    keys(&mut app, "wv$<C-a>");
    assert_eq!(text(&app), "10 21\n10 20", "the 20, not the 10");
    keys(&mut app, "gg0w<C-v>je<C-a>");
    assert_eq!(text(&app), "10 22\n10 21", "a block over the numbers");
    // As in Vim, only the digits inside the selection count.
    keys(&mut app, "gg0w<C-v>j<C-a>");
    assert_eq!(text(&app), "10 32\n10 31", "a block over the first digits");
}

#[test]
fn dot_repeats_a_visual_change_on_as_many_lines() {
    let mut app = with_text("1<CR>1<CR>1<CR>1");
    keys(&mut app, "Vj<C-a>jj.");
    assert_eq!(text(&app), "2\n2\n2\n2");
}
