//! Folding by indentation, end to end.

mod common;

use common::{app, cursor, keys, render, text};
use ferra::App;

const CODE: &str = "fn a() {<CR>one();<CR>two();<CR>}<CR>fn b() {<CR>three();<CR>}";

/// Two little functions, typed with auto-indent; cursor on the first line.
fn code() -> App {
    let mut app = app();
    keys(&mut app, &format!("i{CODE}<Esc>gg0"));
    assert_eq!(
        text(&app),
        "fn a() {\n    one();\n    two();\n}\nfn b() {\n    three();\n}"
    );
    app
}

fn screen(app: &mut App) -> Vec<String> {
    render(app, 40, 10)
        .rows
        .iter()
        .map(|r| r.chars().skip(5).collect::<String>().trim_end().to_string())
        .collect()
}

#[test]
fn za_folds_a_block_into_one_row() {
    let mut app = code();
    keys(&mut app, "za");
    let rows = screen(&mut app);
    assert_eq!(rows[0], "fn a() { ⋯ 3 more lines");
    assert_eq!(rows[1], "fn b() {");
    keys(&mut app, "za");
    assert_eq!(screen(&mut app)[1], "    one();", "za again opens it");
}

#[test]
fn j_and_k_step_over_a_closed_fold() {
    let mut app = code();
    keys(&mut app, "zaj");
    assert_eq!(cursor(&app).0, 4);
    keys(&mut app, "k");
    assert_eq!(cursor(&app).0, 0);
    keys(&mut app, "2j");
    assert_eq!(cursor(&app).0, 5, "two visible lines down");
}

#[test]
fn dd_on_a_closed_fold_deletes_all_of_it() {
    let mut app = code();
    keys(&mut app, "zadd");
    assert_eq!(text(&app), "fn b() {\n    three();\n}");
    keys(&mut app, "u");
    assert!(text(&app).starts_with("fn a() {"));
}

#[test]
fn zc_from_inside_and_zo() {
    let mut app = code();
    keys(&mut app, "jjzc");
    assert_eq!(cursor(&app), (0, 0), "to the fold's first line");
    assert_eq!(screen(&mut app)[1], "fn b() {");
    keys(&mut app, "zo");
    assert_eq!(screen(&mut app)[1], "    one();");
}

#[test]
fn zm_closes_everything_and_zr_opens_it() {
    let mut app = code();
    keys(&mut app, "zM");
    let rows = screen(&mut app);
    assert_eq!(rows[0], "fn a() { ⋯ 3 more lines");
    assert_eq!(rows[1], "fn b() { ⋯ 2 more lines");
    keys(&mut app, "zR");
    assert_eq!(screen(&mut app)[1], "    one();");
}

#[test]
fn zf_folds_any_motion() {
    let mut app = code();
    keys(&mut app, "jzfj");
    let rows = screen(&mut app);
    assert_eq!(rows[1], "    one(); ⋯ 1 more line");
    assert_eq!(rows[2], "}");
    keys(&mut app, "zE");
    assert_eq!(screen(&mut app)[2], "    two();");
}

#[test]
fn a_search_opens_the_fold_it_lands_in() {
    let mut app = code();
    keys(&mut app, "zM/three<CR>");
    assert_eq!(cursor(&app), (5, 4));
    assert_eq!(screen(&mut app)[2], "    three();", "fn b's fold opened");
    assert_eq!(
        screen(&mut app)[0],
        "fn a() { ⋯ 3 more lines",
        "fn a's stayed closed"
    );
}

#[test]
fn an_edit_inside_a_fold_opens_it() {
    let mut app = code();
    keys(&mut app, "za:2s/one/uno/<CR>");
    assert_eq!(screen(&mut app)[1], "    uno();");
}

#[test]
fn gj_steps_over_folds_with_wrap_on_and_zj_finds_folds() {
    let mut app = code();
    keys(&mut app, "zagj");
    assert_eq!(cursor(&app).0, 4);
    keys(&mut app, "zR");
    keys(&mut app, "ggzj");
    assert_eq!(cursor(&app).0, 4, "the next fold's start");
    keys(&mut app, "zk");
    assert_eq!(cursor(&app).0, 0);
}

#[test]
fn a_big_fold_lets_the_lines_after_it_on_screen() {
    let mut app = common::app_with_config("[editor]\nauto-indent = false");
    let body: Vec<String> = (0..50).map(|i| format!("  line{i}")).collect();
    keys(
        &mut app,
        &format!("istart:<CR>{}<CR>after<Esc>gg", body.join("<CR>")),
    );
    keys(&mut app, "za");
    let rows = screen(&mut app);
    assert_eq!(rows[0], "start: ⋯ 50 more lines");
    assert_eq!(rows[1], "after");
    keys(&mut app, "G");
    assert_eq!(cursor(&app).0, 51);
    assert_eq!(screen(&mut app)[1], "after", "still all on screen");
}
