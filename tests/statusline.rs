//! Status lines: one per window (`statusline = "windows"`, the default) or
//! one for the screen (`"global"`), end to end.

mod common;

use std::fs;

use common::{app_with_config, keys, render};
use ratatui::style::Color;
use terminal_editor::App;

/// No line numbers, so screen columns are text columns.
fn plain_app(statusline: &str) -> App {
    app_with_config(&format!(
        "[editor]\nline-numbers = \"none\"\nauto-indent = false\nstatusline = \"{statusline}\""
    ))
}

fn background(app: &mut App, width: u16, height: u16, x: u16, y: u16) -> Color {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer()[(x, y)].bg
}

const ACTIVE_BG: Color = Color::Rgb(0x1f, 0x22, 0x29);
const INACTIVE_BG: Color = Color::Rgb(0x1a, 0x1c, 0x22);

#[test]
fn stacked_windows_each_have_a_status_line() {
    let mut app = plain_app("windows");
    keys(&mut app, "itop<Esc>:sp<CR>");
    // 7 rows: window, window, its status line, window, window, the active
    // window's status line, command line.
    let screen = render(&mut app, 30, 7);
    assert!(screen.rows[0].starts_with("top"));
    assert!(
        screen.rows[2].starts_with(" [scratch] [+]"),
        "{:?}",
        screen.rows[2]
    );
    assert!(
        screen.rows[2].trim_end().ends_with("1:3"),
        "{:?}",
        screen.rows[2]
    );
    assert!(
        !screen.rows[2].contains("NORMAL"),
        "only the active one shows the mode"
    );
    assert!(screen.rows[3].starts_with("top"));
    assert!(
        screen.rows[5].starts_with(" NORMAL "),
        "{:?}",
        screen.rows[5]
    );
    assert_eq!(screen.cursor, (2, 3), "the layout is unchanged");
}

#[test]
fn side_by_side_windows_share_the_bottom_row() {
    let mut app = plain_app("windows");
    keys(&mut app, "iab<Esc>:vs<CR>");
    let screen = render(&mut app, 41, 5);
    let status = &screen.rows[3];
    let (left, right) = status.split_at(20);
    assert!(left.starts_with(" [scratch] [+]"), "{status:?}");
    assert!(right.contains("NORMAL"), "{status:?}");
}

#[test]
fn the_active_status_line_stands_out() {
    let mut app = plain_app("windows");
    keys(&mut app, "itop<Esc>:sp<CR>");
    assert_eq!(
        background(&mut app, 30, 7, 25, 2),
        INACTIVE_BG,
        "upper window's line"
    );
    assert_eq!(
        background(&mut app, 30, 7, 25, 5),
        ACTIVE_BG,
        "active window's line"
    );
    keys(&mut app, "<C-w>k");
    assert_eq!(
        background(&mut app, 30, 7, 25, 2),
        ACTIVE_BG,
        "focus moved up"
    );
    assert_eq!(background(&mut app, 30, 7, 25, 5), INACTIVE_BG);
}

#[test]
fn each_line_shows_its_own_window() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("first.txt");
    let b = dir.path().join("second.txt");
    fs::write(&a, "1\n2\n3\n").unwrap();
    fs::write(&b, "x\n").unwrap();
    let mut app = plain_app("windows");
    app.open(&a);
    keys(&mut app, "G");
    keys(&mut app, &format!(":sp {}<CR>", b.display()));
    let screen = render(&mut app, 60, 9);
    let upper = screen
        .rows
        .iter()
        .find(|row| row.contains("first.txt"))
        .expect("the upper window's status line");
    assert!(
        upper.trim_end().ends_with("3:1"),
        "its own cursor: {upper:?}"
    );
    let lower = &screen.rows[7];
    assert!(
        lower.contains("second.txt") && lower.contains("NORMAL"),
        "{lower:?}"
    );
}

#[test]
fn global_mode_has_one_line_and_separators() {
    let mut app = plain_app("global");
    keys(&mut app, "itop<Esc>:sp<CR>");
    let screen = render(&mut app, 30, 7);
    assert_eq!(screen.rows[2], "─".repeat(30));
    assert!(screen.rows[5].starts_with(" NORMAL "));
    let lines = screen
        .rows
        .iter()
        .filter(|r| r.contains("[scratch]"))
        .count();
    assert_eq!(lines, 1);
}

#[test]
fn a_single_window_looks_the_same_either_way() {
    let mut windows = plain_app("windows");
    let mut global = plain_app("global");
    keys(&mut windows, "ihello<Esc>");
    keys(&mut global, "ihello<Esc>");
    assert_eq!(
        render(&mut windows, 30, 5).rows,
        render(&mut global, 30, 5).rows
    );
}
