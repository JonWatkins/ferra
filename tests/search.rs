//! Search end-to-end: `/`, `?`, `n`, `N`, `*`, `#`, highlighting and `:noh`.

mod common;

use common::{app, app_with_config, cursor, keys, render, text};
use ferra::App;
use ferra::editor::Mode;
use ratatui::{Terminal, backend::TestBackend, style::Color};

/// An app containing `content`, with the cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// Cells on screen row `y` drawn with the default match highlight.
fn match_cells(app: &mut App, y: u16) -> Vec<u16> {
    let mut terminal = Terminal::new(TestBackend::new(30, 6)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let match_bg = Color::Rgb(0xe0, 0xaf, 0x68);
    (0..30).filter(|&x| buffer[(x, y)].bg == match_bg).collect()
}

#[test]
fn slash_searches_forward_and_n_repeats() {
    let mut app = with_text("one two<CR>two one<CR>two");
    keys(&mut app, "/two<CR>");
    assert_eq!(cursor(&app), (0, 4));
    assert_eq!(message(&app), "/two");
    keys(&mut app, "n");
    assert_eq!(cursor(&app), (1, 0));
    keys(&mut app, "n");
    assert_eq!(cursor(&app), (2, 0));
    keys(&mut app, "n");
    assert_eq!(cursor(&app), (0, 4));
    assert_eq!(message(&app), "Search hit BOTTOM, continuing at TOP");
    keys(&mut app, "N");
    assert_eq!(cursor(&app), (2, 0));
}

#[test]
fn question_mark_searches_backward() {
    let mut app = with_text("x a<CR>a<CR>a");
    keys(&mut app, "G?a<CR>");
    assert_eq!(cursor(&app), (1, 0));
    keys(&mut app, "n");
    assert_eq!(cursor(&app), (0, 2));
    keys(&mut app, "N");
    assert_eq!(cursor(&app), (1, 0), "N reverses a backward search");
}

#[test]
fn counts_and_operators_after_search() {
    let mut app = with_text("a1 a2 a3 a4");
    keys(&mut app, "/a<CR>2n");
    assert_eq!(cursor(&app), (0, 9));
    keys(&mut app, "dw");
    assert_eq!(text(&app), "a1 a2 a3 ");
}

#[test]
fn incremental_preview_and_cancel() {
    let mut app = with_text("alpha<CR>beta<CR>gamma");
    keys(&mut app, "/gam");
    assert_eq!(app.editor().mode(), Mode::Command);
    assert_eq!(
        cursor(&app),
        (2, 0),
        "cursor follows the match while typing"
    );
    let screen = render(&mut app, 30, 6);
    assert_eq!(screen.rows[5].trim_end(), "/gam");
    assert!(
        screen.rows[4].starts_with(" SEARCH "),
        "{:?}",
        screen.rows[4]
    );

    keys(&mut app, "<Esc>");
    assert_eq!(
        cursor(&app),
        (0, 0),
        "Esc returns to where the search began"
    );
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn not_found_reports_and_stays_put() {
    let mut app = with_text("abc");
    keys(&mut app, "l/zzz<CR>");
    assert_eq!(cursor(&app), (0, 1));
    assert_eq!(message(&app), "Pattern not found: zzz");
}

#[test]
fn invalid_regex_reports_error() {
    let mut app = with_text("abc");
    keys(&mut app, "/a(<CR>");
    assert!(
        message(&app).starts_with("Invalid pattern"),
        "{}",
        message(&app)
    );
}

#[test]
fn regex_and_smart_case() {
    let mut app = with_text("Foo foo f00");
    keys(&mut app, "/f\\d+<CR>");
    assert_eq!(cursor(&app), (0, 8));
    keys(&mut app, "0/foo<CR>");
    assert_eq!(
        cursor(&app),
        (0, 4),
        "lowercase pattern ignores case: next match"
    );
    keys(&mut app, "0/Foo<CR>");
    assert_eq!(
        cursor(&app),
        (0, 0),
        "uppercase pattern is exact (wraps to 0)"
    );
}

#[test]
fn case_option_is_configurable() {
    let mut app = app_with_config("[editor.search]\ncase = \"sensitive\"");
    keys(&mut app, "iFoo foo<Esc>0/foo<CR>");
    keys(&mut app, "0n");
    assert_eq!(cursor(&app), (0, 4));
}

#[test]
fn star_and_hash() {
    let mut app = with_text("let x = x + xs;<CR>x");
    keys(&mut app, "w*");
    assert_eq!(cursor(&app), (0, 8), "whole word only, not `xs`");
    keys(&mut app, "*");
    assert_eq!(cursor(&app), (1, 0));
    keys(&mut app, "#");
    assert_eq!(cursor(&app), (0, 8));
    keys(&mut app, "n");
    assert_eq!(cursor(&app), (0, 4), "n continues #'s direction");
}

#[test]
fn empty_pattern_repeats_last_search() {
    let mut app = with_text("a b a b");
    keys(&mut app, "/b<CR>/<CR>");
    assert_eq!(cursor(&app), (0, 6));
}

#[test]
fn matches_are_highlighted_until_noh() {
    let mut app = with_text("ab x ab");
    // Gutter is 5 cells wide, so text column 0 is screen column 5.
    keys(&mut app, "/ab<CR>");
    assert_eq!(match_cells(&mut app, 0), vec![5, 6, 10, 11]);

    keys(&mut app, ":noh<CR>");
    assert!(match_cells(&mut app, 0).is_empty());

    keys(&mut app, "n");
    assert_eq!(
        match_cells(&mut app, 0).len(),
        4,
        "searching again re-enables it"
    );
}

#[test]
fn highlight_can_be_disabled() {
    let mut app = app_with_config("[editor.search]\nhighlight = false");
    keys(&mut app, "iab ab<Esc>0/ab<CR>");
    assert!(match_cells(&mut app, 0).is_empty());
}

#[test]
fn visual_selection_extends_with_n() {
    let mut app = with_text("a, b, c");
    keys(&mut app, "/,<CR>0vnd");
    assert_eq!(text(&app), " b, c");
}

#[test]
fn search_is_not_a_repeatable_change() {
    let mut app = with_text("abc abc");
    // After `x` the text is "bc abc"; `/c` finds the c at column 1 and `.`
    // repeats `x` there (the search itself isn't repeated).
    keys(&mut app, "x/c<CR>.");
    assert_eq!(text(&app), "b abc");
}

#[test]
fn escape_in_normal_mode_clears_highlights() {
    let mut app = with_text("ab x ab");
    keys(&mut app, "/ab<CR>");
    assert_eq!(match_cells(&mut app, 0).len(), 4);
    keys(&mut app, "<Esc>");
    assert!(match_cells(&mut app, 0).is_empty());
    assert_eq!(cursor(&app), (0, 5), "the cursor stays on the match");
}
