//! Full-screen rendering through ratatui's test backend.

mod common;

use common::{app, app_with_config, keys, render};

#[test]
fn layout_has_editor_statusline_and_command_line() {
    let mut app = app();
    keys(&mut app, "ihello<Esc>");
    let screen = render(&mut app, 30, 5);

    assert!(
        screen.rows[0].starts_with("   1 hello"),
        "{:?}",
        screen.rows[0]
    );
    assert!(
        screen.rows[3].starts_with(" NORMAL "),
        "{:?}",
        screen.rows[3]
    );
    assert!(
        screen.rows[3].trim_end().ends_with("1:5"),
        "{:?}",
        screen.rows[3]
    );
    assert_eq!(screen.rows[4].trim(), "");
}

#[test]
fn cursor_is_placed_in_text_area() {
    let mut app = app();
    keys(&mut app, "iab<CR>cd");
    let screen = render(&mut app, 30, 6);
    // Gutter is 5 wide; insert mode cursor sits after "cd".
    assert_eq!(screen.cursor, (7, 1));
}

#[test]
fn relative_line_numbers_by_default() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<Esc>k");
    let screen = render(&mut app, 20, 6);
    assert!(screen.rows[0].starts_with("   1 a"), "{:?}", screen.rows[0]);
    assert!(screen.rows[1].starts_with("   2 b"), "{:?}", screen.rows[1]);
    assert!(screen.rows[2].starts_with("   1 c"), "{:?}", screen.rows[2]);
}

#[test]
fn absolute_line_numbers() {
    let mut app = app_with_config("[editor]\nline-numbers = \"absolute\"");
    keys(&mut app, "ia<CR>b<CR>c<Esc>gg");
    let screen = render(&mut app, 20, 6);
    assert!(screen.rows[2].starts_with("   3 c"), "{:?}", screen.rows[2]);
}

#[test]
fn command_mode_shows_prompt_and_moves_cursor() {
    let mut app = app();
    keys(&mut app, ":goto");
    let screen = render(&mut app, 30, 5);
    assert!(screen.rows[3].starts_with(" COMMAND "));
    assert_eq!(screen.rows[4].trim_end(), ":goto");
    assert_eq!(screen.cursor, (5, 4));
}

#[test]
fn error_messages_are_shown() {
    let mut app = app();
    keys(&mut app, ":bad<CR>");
    let screen = render(&mut app, 30, 5);
    assert_eq!(screen.rows[4].trim_end(), "Unknown command: bad");
}

#[test]
fn pending_keys_are_shown() {
    let mut app = app();
    keys(&mut app, "3g");
    let screen = render(&mut app, 30, 5);
    assert!(screen.rows[3].contains("3g"), "{:?}", screen.rows[3]);
}

#[test]
fn view_scrolls_to_follow_cursor() {
    let mut app = app_with_config("[editor]\nscroll-off = 0\nline-numbers = \"none\"");
    for n in 1..=20 {
        keys(&mut app, &format!("o{n}<Esc>"));
    }
    // Five rows: three for text, then statusline and command line.
    let screen = render(&mut app, 10, 5);
    assert_eq!(screen.rows[2].trim_end(), "20");
    assert_eq!(screen.cursor, (1, 2));
}

#[test]
fn tiny_terminal_does_not_panic() {
    let mut app = app();
    keys(&mut app, "ihello world<Esc>");
    for (w, h) in [(1, 1), (1, 3), (3, 3), (6, 2)] {
        render(&mut app, w, h);
    }
}
