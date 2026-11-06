//! The `:` and `/` prompts end to end: editing, history and completion.

mod common;

use common::{app, cursor, keys, render, text};
use terminal_editor::App;
use terminal_editor::editor::Mode;

fn input(app: &App) -> String {
    app.editor().command_line.input().to_string()
}

#[test]
fn edit_in_the_middle_of_a_command() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<Esc>");
    // Typo: "goto_lin 2" — go back and fix it before running.
    keys(&mut app, ":goto_lin 2<Left><Left>e<CR>");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn home_end_delete_and_word_delete() {
    let mut app = app();
    keys(&mut app, ":xgoto_line 3<Home><Del>");
    assert_eq!(input(&app), "goto_line 3");
    keys(&mut app, "<End><C-w>");
    assert_eq!(input(&app), "goto_line ");
    keys(&mut app, "<C-u>");
    assert_eq!(input(&app), "");
    assert_eq!(app.editor().mode(), Mode::Command, "still at the prompt");
}

#[test]
fn command_history_with_prefix_matching() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<Esc>");
    keys(&mut app, ":goto_line 1<CR>:noh<CR>:goto_line 3<CR>");

    keys(&mut app, ":<Up>");
    assert_eq!(input(&app), "goto_line 3", "most recent first");
    keys(&mut app, "<Up>");
    assert_eq!(input(&app), "noh");
    keys(&mut app, "<Esc>");

    keys(&mut app, ":go<Up><Up>");
    assert_eq!(
        input(&app),
        "goto_line 1",
        "only entries starting with `go`"
    );
    keys(&mut app, "<Down><Down>");
    assert_eq!(input(&app), "go", "back to what was typed");
    keys(&mut app, "<Esc>:<C-p><CR>");
    assert_eq!(cursor(&app), (2, 0), "<C-p> recalls and runs it again");
}

#[test]
fn search_history_is_separate() {
    let mut app = app();
    keys(&mut app, "ione two<Esc>/one<CR>:noh<CR>");
    keys(&mut app, "/<Up>");
    assert_eq!(input(&app), "one");
    keys(&mut app, "<Esc>:<Up>");
    assert_eq!(input(&app), "noh");
}

#[test]
fn editing_a_search_updates_the_preview() {
    let mut app = app();
    keys(&mut app, "iab ac ad<Esc>0/ad<Left><Del>c");
    assert_eq!(input(&app), "ac");
    assert_eq!(cursor(&app), (0, 3), "preview follows the edited pattern");
    keys(&mut app, "<Esc>");
    assert_eq!(cursor(&app), (0, 0));
}

#[test]
fn tab_completes_commands_and_cycles() {
    let mut app = app();
    keys(&mut app, ":goto_l<Tab>");
    assert_eq!(input(&app), "goto_line");
    keys(&mut app, "<Esc>:write_a<Tab>");
    assert_eq!(input(&app), "write_all");
    let screen = render(&mut app, 40, 4);
    assert!(screen.rows[3].contains("(1/2)"), "{:?}", screen.rows[3]);
    keys(&mut app, "<Tab>");
    assert_eq!(input(&app), "write_all_quit");
    keys(&mut app, "<S-Tab>");
    assert_eq!(input(&app), "write_all");
}

#[test]
fn tab_completes_file_paths_for_file_commands() {
    // Tests run from the crate root.
    let mut app = app();
    keys(&mut app, ":e sr<Tab>");
    assert_eq!(input(&app), "e src/", "a directory, ready to go further");
    keys(&mut app, "ma<Tab>");
    assert_eq!(input(&app), "e src/main.rs");
    keys(&mut app, "<CR>");
    assert!(text(&app).contains("fn main()"), "and it opens");
}

#[test]
fn tab_does_nothing_without_candidates() {
    let mut app = app();
    keys(&mut app, ":goto_line 1<Tab>");
    assert_eq!(input(&app), "goto_line 1");
    keys(&mut app, "<Esc>/abc<Tab>");
    assert_eq!(input(&app), "abc", "no completion in searches");
}

#[test]
fn backspace_on_an_empty_prompt_leaves_it() {
    let mut app = app();
    keys(&mut app, ":ab<BS><BS>");
    assert_eq!(app.editor().mode(), Mode::Command);
    keys(&mut app, "<BS>");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn the_terminal_cursor_follows_the_prompt_cursor() {
    let mut app = app();
    keys(&mut app, ":write<Left><Left>");
    let screen = render(&mut app, 30, 4);
    assert_eq!(screen.cursor, (4, 3), "after `:wri`");
}
