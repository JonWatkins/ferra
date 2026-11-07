//! Indentation by each language's rules: `=`, and new lines and closing
//! brackets while typing, end to end.

mod common;

use std::fs;

use common::{app_with_config, cursor, keys, text};
use ferra::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

/// A file called `name` holding `content`, open with `config`, cursor at
/// the start.
fn open_with(name: &str, content: &str, config: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    let mut app = app_with_config(config);
    app.open(&path);
    (dir, app)
}

fn open(name: &str, content: &str) -> (tempfile::TempDir, App) {
    open_with(name, content, "")
}

const MESSY: &str = "fn main() {
let x = foo(
      1,
  2,
);
        if x {
y();
    }
}";

const TIDY: &str = "fn main() {
    let x = foo(
        1,
        2,
    );
    if x {
        y();
    }
}";

#[test]
fn equals_reindents_a_whole_file() {
    let (_dir, mut app) = open("a.rs", MESSY);
    keys(&mut app, "jjgg=G");
    assert_eq!(text(&app), TIDY);
    assert_eq!(cursor(&app), (0, 0), "first non-blank of the first line");
    keys(&mut app, "u");
    assert_eq!(text(&app), MESSY, "one undo step");
}

#[test]
fn double_equals_counts_text_objects_visual_and_dot() {
    let (_dir, mut app) = open("a.rs", MESSY);
    keys(&mut app, "j==");
    assert!(
        text(&app).contains("\n    let x = foo(\n      1,"),
        "one line"
    );
    keys(&mut app, "j2==");
    assert!(
        text(&app).contains("foo(\n        1,\n        2,\n);"),
        "two"
    );
    keys(&mut app, "j.");
    assert!(
        text(&app).contains("2,\n    );\n        if x {"),
        "dot: the next two lines"
    );
    keys(&mut app, "jjVj=");
    assert_eq!(text(&app), TIDY, "visual");
}

#[test]
fn blank_lines_are_emptied_and_tabs_used_without_expand_tab() {
    let (_dir, mut app) = open_with(
        "a.rs",
        "fn f() {\n  \nif a {\nb();\n}\n}",
        "[editor]\nexpand-tab = false",
    );
    keys(&mut app, "=G");
    assert_eq!(text(&app), "fn f() {\n\n\tif a {\n\t\tb();\n\t}\n}");
}

#[test]
fn typing_code_indents_as_you_go() {
    let (_dir, mut app) = open("a.rs", "");
    keys(
        &mut app,
        "ifn main() {<CR>let x = foo(<CR>1,<CR>);<CR>if x {<CR>y();<CR>}<CR>}<Esc>",
    );
    assert_eq!(
        text(&app),
        "fn main() {\n    let x = foo(\n        1,\n    );\n    if x {\n        y();\n    }\n}"
    );
}

#[test]
fn enter_between_brackets_opens_an_indented_line() {
    let (_dir, mut app) = open("a.rs", "fn f() {}");
    keys(&mut app, "$i<CR>x<Esc>");
    assert_eq!(text(&app), "fn f() {\n    x\n}");
}

#[test]
fn o_and_capital_o_indent_by_the_rules() {
    let (_dir, mut app) = open("a.js", "if (a) {\n    b();\n}");
    keys(&mut app, "oc();<Esc>");
    assert_eq!(text(&app), "if (a) {\n    c();\n    b();\n}");
    keys(&mut app, "GOd();<Esc>");
    assert_eq!(text(&app), "if (a) {\n    c();\n    b();\n    d();\n}");
}

#[test]
fn other_languages_follow_their_own_rules() {
    let (_dir, mut app) = open("a.go", "func f() {\nswitch x {\ncase 1:\ng()\n}\n}");
    keys(&mut app, "=G");
    assert_eq!(
        text(&app),
        "func f() {\n    switch x {\n    case 1:\n        g()\n    }\n}"
    );

    let (_dir, mut app) = open("a.sh", "if a; then\nb\nelse\nc\nfi");
    keys(&mut app, "=G");
    assert_eq!(text(&app), "if a; then\n    b\nelse\n    c\nfi");

    let (_dir, mut app) = open("a.html", "<ul>\n<li>a</li>\n</ul>");
    keys(&mut app, "=G");
    assert_eq!(text(&app), "<ul>\n    <li>a</li>\n</ul>");
}

#[test]
fn python_has_no_rules_but_new_lines_follow_colons() {
    let (_dir, mut app) = open("a.py", "def f():\n  return 1");
    keys(&mut app, "=G");
    assert_eq!(
        message(&app),
        "Can't re-indent: no indentation rules for python"
    );
    assert_eq!(text(&app), "def f():\n  return 1", "unchanged");
    keys(&mut app, "ggox<Esc>");
    // The file indents by two, so the new line does too.
    assert_eq!(text(&app), "def f():\n  x\n  return 1");
}

#[test]
fn files_without_a_language_say_so() {
    let (_dir, mut app) = open("notes.txt", "  a\nb");
    keys(&mut app, "=G");
    assert_eq!(message(&app), "Can't re-indent: no language for this file");
}

#[test]
fn auto_indent_off_still_allows_equals() {
    let (_dir, mut app) = open_with("a.rs", "fn f() {\nx();\n}", "[editor]\nauto-indent = false");
    keys(&mut app, "oy();<Esc>");
    assert_eq!(text(&app), "fn f() {\ny();\nx();\n}", "no indent typing");
    keys(&mut app, "gg=G");
    assert_eq!(text(&app), "fn f() {\n    y();\n    x();\n}");
}

#[test]
fn bash_keywords_go_back_to_their_block_as_typed() {
    let (_dir, mut app) = open("a.sh", "");
    keys(&mut app, "iif a; then<CR>b<CR>else<CR>c<CR>fi<Esc>");
    assert_eq!(text(&app), "if a; then\n    b\nelse\n    c\nfi");

    let (_dir, mut app) = open("b.sh", "");
    keys(&mut app, "ifor i in x<CR>do<CR>a<CR>done<Esc>");
    assert_eq!(text(&app), "for i in x\ndo\n    a\ndone");
}

#[test]
fn a_word_that_only_starts_like_a_keyword_goes_back() {
    let (_dir, mut app) = open("a.sh", "");
    keys(&mut app, "iif a; then<CR>file=1<CR>done_x=2<Esc>");
    assert_eq!(text(&app), "if a; then\n    file=1\n    done_x=2");
    assert_eq!(cursor(&app), (2, 11), "the cursor stays on its character");
}

#[test]
fn a_closing_tag_typed_first_goes_back_to_its_element() {
    let (_dir, mut app) = open("a.html", "");
    keys(&mut app, "i<lt>div><CR><lt>p>x<lt>/p><CR><lt>/div><Esc>");
    assert_eq!(text(&app), "<div>\n    <p>x</p>\n</div>");
}

#[test]
fn python_else_elif_except_and_finally_line_up_on_their_colon() {
    let (_dir, mut app) = open("a.py", "");
    keys(&mut app, "iif a:<CR>b<CR>elif c:<CR>d<CR>else:<CR>e<Esc>");
    assert_eq!(text(&app), "if a:\n    b\nelif c:\n    d\nelse:\n    e");

    let (_dir, mut app) = open("b.py", "");
    keys(
        &mut app,
        "itry:<CR>f<CR>except E as x:<CR>g<CR>finally:<CR>h<Esc>",
    );
    assert_eq!(
        text(&app),
        "try:\n    f\nexcept E as x:\n    g\nfinally:\n    h"
    );
}

#[test]
fn python_else_finds_the_if_it_belongs_to() {
    let (_dir, mut app) = open("a.py", "if a:\n    if b:\n        x\n");
    keys(&mut app, "Goelse:<Esc>");
    assert_eq!(text(&app).lines().last(), Some("    else:"), "the inner if");
    keys(&mut app, "oy<CR>else:<Esc>");
    assert_eq!(
        text(&app).lines().last(),
        Some("    else:"),
        "already in place"
    );
}

#[test]
fn python_words_that_only_start_like_keywords_stay() {
    let (_dir, mut app) = open("a.py", "if a:\n    b\n");
    keys(&mut app, "Goelsewhere = {1: 2}<Esc>");
    assert_eq!(text(&app).lines().last(), Some("    elsewhere = {1: 2}"));
}

#[test]
fn python_lines_after_return_and_friends_go_back_a_level() {
    let (_dir, mut app) = open("a.py", "");
    keys(
        &mut app,
        "idef f(x):<CR>if x:<CR>return 1<CR>pass<CR>x<Esc>",
    );
    assert_eq!(
        text(&app),
        "def f(x):\n    if x:\n        return 1\n    pass\nx"
    );
}

#[test]
fn yaml_keys_with_nothing_after_them_open_a_level() {
    let (_dir, mut app) = open_with("a.yml", "", "[editor]\ntab-width = 2");
    keys(&mut app, "ia:<CR>b: 1<CR>c: |<CR>text<Esc>");
    assert_eq!(text(&app), "a:\n  b: 1\n  c: |\n    text");
}
