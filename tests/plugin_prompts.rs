//! Plugins asking the user: `ctx:input` (a box for a line of text) and
//! `ctx:pick` (the picker with the plugin's items), each running a plugin
//! command with the answer.

mod common;

use common::{app, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

fn with_plugin(source: &str) -> App {
    let mut app = app();
    app.load_plugin("test.lua", source).expect("plugin loads");
    app
}

const ASKING: &str = r##"
editor.command("ask_name", function(ctx)
  ctx:input({ title = "Your name", value = "nobody", placeholder = "a name",
              on_submit = "got_name", on_cancel = "no_name" })
end)
editor.command("got_name", function(ctx) ctx:insert("hello " .. ctx.args[1]) end)
editor.command("no_name", function(ctx) ctx:message("cancelled") end)

editor.command("ask_colour", function(ctx)
  ctx:pick({ title = "Colour", on_choose = "got_colour",
             items = { "red", { label = "Green", detail = "#00ff00", value = "green" }, "blue" } })
end)
editor.command("got_colour", function(ctx) ctx:insert(ctx.args[1]) end)
"##;

#[test]
fn an_input_box_asks_for_text_and_runs_the_plugins_command() {
    let mut app = with_plugin(ASKING);
    keys(&mut app, ":ask_name<CR>");
    assert_eq!(app.editor().mode(), Mode::Input);
    let screen = render(&mut app, 70, 10);
    let shown = screen.rows.join("\n");
    assert!(shown.contains(" Your name "), "{shown}");
    assert!(shown.contains("nobody"), "{shown}");
    assert!(shown.contains("⏎ ok  Esc cancel"), "{shown}");
    assert_eq!(screen.cursor.1, 2, "the cursor's in the field");

    // Typing replaces what it started with.
    keys(&mut app, "Ada<BS>a<Left><Left>x<CR>");
    assert_eq!(text(&app), "hello Axda");
    keys(&mut app, ".");
    assert_eq!(text(&app), "hello Axda", "typing in the box isn't for `.`");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(app.editor().input_box().is_none());
}

#[test]
fn cancelling_runs_on_cancel() {
    let mut app = with_plugin(ASKING);
    keys(&mut app, ":ask_name<CR><Esc>");
    assert_eq!(text(&app), "");
    assert_eq!(
        app.editor().message().map(|m| m.text.as_str()),
        Some("cancelled")
    );
}

#[test]
fn a_pick_lists_the_plugins_items_to_filter_and_choose() {
    let mut app = with_plugin(ASKING);
    keys(&mut app, ":ask_colour<CR>");
    assert_eq!(app.editor().mode(), Mode::Picker);
    let shown = render(&mut app, 70, 16).rows.join("\n");
    assert!(shown.contains("Colour"), "{shown}");
    assert!(shown.contains("#00ff00"), "the detail: {shown}");
    keys(&mut app, "gre<CR>");
    assert_eq!(text(&app), "green", "the item's value");
}

#[test]
fn mistakes_are_the_plugins_errors() {
    let mut app = with_plugin(
        r#"editor.command("bad", function(ctx) ctx:pick({ items = { "a" }, on_choose = "missing" }) end)"#,
    );
    keys(&mut app, ":bad<CR>");
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(
        message.contains("pick.on_choose: no command named missing"),
        "{message}"
    );
    assert_eq!(app.editor().mode(), Mode::Normal);
}
