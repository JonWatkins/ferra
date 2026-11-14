//! Auto-closing brackets and quotes (`auto-pairs`, on as shipped; the
//! tests' usual config has it off).

mod common;

use common::{app_with_config, cursor, keys, text};
use ferra::App;

fn paired() -> App {
    app_with_config("[editor]\nauto-pairs = true\nauto-indent = false\n")
}

#[test]
fn openers_get_their_closers_and_closers_step_over() {
    let mut app = paired();
    keys(&mut app, "if(");
    assert_eq!(text(&app), "f()");
    assert_eq!(cursor(&app), (0, 2), "between them");
    keys(&mut app, "x)");
    assert_eq!(text(&app), "f(x)", "typed over, not doubled");
    assert_eq!(cursor(&app), (0, 4));
    keys(&mut app, " [{\"");
    assert_eq!(text(&app), "f(x) [{\"\"}]");
    keys(&mut app, "s\"}]");
    assert_eq!(text(&app), "f(x) [{\"s\"}]");
    assert_eq!(cursor(&app), (0, 12), "at the end");
}

#[test]
fn not_before_a_word_or_after_one_for_quotes() {
    let mut app = paired();
    keys(&mut app, "ifoo<Esc>0i(");
    assert_eq!(text(&app), "(foo", "before a word: just the one");
    let mut app = paired();
    keys(&mut app, "idon't");
    assert_eq!(text(&app), "don't");
    let mut app = paired();
    keys(&mut app, "ifn f<lt>'a>(x: &'a str)");
    assert_eq!(text(&app), "fn f<'a>(x: &'a str)", "Rust's lifetimes");
}

#[test]
fn backspace_takes_an_empty_pair() {
    let mut app = paired();
    keys(&mut app, "ia(<BS>");
    assert_eq!(text(&app), "a");
    keys(&mut app, "\"x<BS><BS>");
    assert_eq!(text(&app), "a", "emptied, then the pair");
}

#[test]
fn enter_between_braces_puts_the_closer_on_its_own_line() {
    let mut app = app_with_config("[editor]\nauto-pairs = true\n");
    keys(&mut app, "if {<CR>x");
    assert_eq!(text(&app), "f {\n    x\n}");
}

#[test]
fn off_it_types_just_what_is_typed() {
    let mut app = app_with_config("[editor]\nauto-pairs = false\n");
    keys(&mut app, "if(\"");
    assert_eq!(text(&app), "f(\"");
}

#[test]
fn with_vs_codes_keys_a_selection_is_wrapped() {
    let mut app =
        app_with_config("[editor]\nkeymap = \"vscode\"\nauto-pairs = true\nauto-indent = false\n");
    keys(&mut app, "say hello<Home><C-S-Right>");
    keys(&mut app, "\"");
    assert_eq!(text(&app), "\"say\" hello");
    // Still selected (the words inside): another wraps them, inside.
    keys(&mut app, "(");
    assert_eq!(text(&app), "\"(say)\" hello");
    // Typing a letter replaces it as usual.
    keys(&mut app, "x");
    assert_eq!(text(&app), "\"(x)\" hello");
}
