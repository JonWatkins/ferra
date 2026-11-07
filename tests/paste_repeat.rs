//! Text pasted into the terminal, repeated by `.` and kept in macros.

mod common;

use common::{app, app_with_config, cursor, keys, text};
use ferra::App;

fn with_text(content: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

fn register(app: &App, name: char) -> String {
    app.editor()
        .registers
        .get(Some(name))
        .map(|r| r.text.clone())
        .unwrap_or_default()
}

#[test]
fn dot_repeats_an_insert_with_a_paste_in_it() {
    let mut app = with_text("1\n2");
    keys(&mut app, "A ");
    app.handle_paste("pasted\ntext");
    keys(&mut app, "!<Esc>j.");
    assert_eq!(text(&app), "1 pasted\ntext!\n2 pasted\ntext!");
}

#[test]
fn dot_repeats_a_paste_in_normal_mode() {
    let mut app = with_text("ab");
    app.handle_paste("XY");
    assert_eq!(text(&app), "XYab");
    keys(&mut app, "$.");
    assert_eq!(text(&app), "XYaXYb", "put before the cursor again");
    keys(&mut app, "u");
    assert_eq!(text(&app), "XYab", "one undo step");
}

#[test]
fn replayed_pastes_skip_insert_bindings_and_auto_indent() {
    let mut app = app_with_config("[keys.insert]\njk = \"normal_mode\"");
    keys(&mut app, "o{<CR>");
    app.handle_paste("jk\nx");
    keys(&mut app, "<Esc>");
    let once = "\n{\n    jk\nx";
    assert_eq!(text(&app), once, "pasted as is");
    keys(&mut app, ".");
    assert_eq!(text(&app), format!("{once}{once}"));
}

#[test]
fn macros_record_pastes_as_key_notation() {
    let mut app = with_text("a\nb");
    keys(&mut app, "qqA ");
    app.handle_paste("<x>\ty");
    keys(&mut app, "<Esc>jq");
    assert_eq!(
        register(&app, 'q'),
        "A<Space><PasteStart><lt>x><Tab>y<PasteEnd><Esc>j"
    );
    assert_eq!(text(&app), "a <x>\ty\nb");
    keys(&mut app, "@q");
    assert_eq!(text(&app), "a <x>\ty\nb <x>\ty");
}

#[test]
fn a_macro_can_paste_in_normal_mode() {
    let mut app = with_text("ab\ncd");
    keys(&mut app, "qw");
    app.handle_paste("1\n2");
    keys(&mut app, "jq");
    assert_eq!(text(&app), "1\n2ab\ncd");
    keys(&mut app, "0@w");
    assert_eq!(text(&app), "1\n2ab\n1\n2cd");
}

#[test]
fn a_hand_written_macro_missing_paste_end_still_pastes() {
    let mut app = with_text("A<lt>PasteStart>!!");
    keys(&mut app, "\"ay$dd");
    keys(&mut app, "ix<Esc>@a");
    assert_eq!(text(&app), "x!!");
    keys(&mut app, "<Esc>0x");
    assert_eq!(text(&app), "!!", "keys after the macro work again");
    assert_eq!(cursor(&app), (0, 0));
}

#[test]
fn a_paste_typed_into_a_prompt_by_a_macro() {
    let mut app = with_text("foo foo");
    keys(&mut app, "qs:");
    app.handle_paste("s/foo/bar/g");
    keys(&mut app, "<CR>q");
    assert_eq!(text(&app), "bar bar");
    keys(&mut app, "ofoo<Esc>@s");
    assert_eq!(text(&app), "bar bar\nbar");
}
