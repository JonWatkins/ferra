//! Plugins' popups end to end: opened from Lua, drawn, moved about with
//! `[keys.popup]`, the plugin's own keys running its commands, closing.

mod common;

use common::{app, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

fn with_plugin(source: &str) -> App {
    let mut app = app();
    app.load_plugin("test.lua", source).expect("plugin loads");
    app
}

const PICK_FRUIT: &str = r#"
editor.command("fruit", "Pick a fruit", function(ctx)
  ctx:popup({
    name = "fruit",
    title = "Fruit",
    lines = { "apple", "banana", { { "cherry", "keyword" }, { " (red)" } } },
    footer = "⏎ insert  q close",
    select = true,
    keys = { ["<CR>"] = "fruit_insert", ["x"] = "fruit_rename" },
    on_close = "fruit_closed",
  })
end)
editor.command("fruit_insert", function(ctx)
  local names = { "apple", "banana", "cherry" }
  local chosen = names[ctx:popup_selected()]
  ctx:popup_close()
  ctx:insert(chosen)
end)
editor.command("fruit_rename", function(ctx)
  ctx:popup_update({ title = "Renamed", lines = { "only" } })
end)
editor.command("fruit_closed", function(ctx)
  ctx:insert("closed")
end)
editor.map("normal", "<Space>F", "fruit")
"#;

#[test]
fn a_list_popup_takes_its_own_keys_and_runs_the_plugins_commands() {
    let mut app = with_plugin(PICK_FRUIT);
    keys(&mut app, "<Space>F");
    assert_eq!(app.editor().mode(), Mode::Popup);

    let screen = render(&mut app, 60, 16);
    let all = screen.rows.join("\n");
    assert!(all.contains(" Fruit "), "{all}");
    assert!(all.contains("│ cherry (red)"), "{all}");
    assert!(all.contains("⏎ insert  q close"), "{all}");
    assert!(all.contains(" 1/3 "), "{all}");

    keys(&mut app, "jj");
    assert_eq!(app.editor().popup().unwrap().selected(), Some(2));
    keys(&mut app, "k<CR>");
    assert_eq!(text(&app), "banana", "the plugin's <CR>");
    assert!(app.editor().popup().is_none());
    assert_eq!(app.editor().mode(), Mode::Normal);
    // j moves the buffer's cursor again, not the popup's selection.
    keys(&mut app, "u");
    assert_eq!(text(&app), "");
}

#[test]
fn q_closes_it_running_on_close_and_updates_show() {
    let mut app = with_plugin(PICK_FRUIT);
    keys(&mut app, "<Space>Fx");
    let popup = app.editor().popup().unwrap();
    assert_eq!((popup.title.as_str(), popup.lines.len()), ("Renamed", 1));
    keys(&mut app, "q");
    assert!(app.editor().popup().is_none());
    assert_eq!(text(&app), "closed");
}

#[test]
fn the_popups_keys_are_its_own() {
    let mut app = with_plugin(PICK_FRUIT);
    // `x` deletes a character in normal mode, as ever.
    keys(&mut app, "ihello<Esc>0x");
    assert_eq!(text(&app), "ello");
}

#[test]
fn a_popup_without_focus_goes_with_the_next_key() {
    let mut app = with_plugin(
        r#"
        editor.command("note", function(ctx)
          ctx:popup({ title = "Note", lines = { "just so you know" }, focus = false, position = "cursor" })
        end)
        editor.map("normal", "<Space>N", "note")
        "#,
    );
    keys(&mut app, "<Space>N");
    assert_eq!(app.editor().mode(), Mode::Normal);
    let screen = render(&mut app, 60, 12);
    assert!(screen.rows.join("\n").contains("just so you know"));
    keys(&mut app, "l");
    assert!(app.editor().popup().is_none());
}

#[test]
fn a_bad_popup_is_the_plugins_error() {
    let mut app = with_plugin(
        r#"
        editor.command("broken", function(ctx)
          ctx:popup({ keys = { ["<CR>"] = "no_such_command" } })
        end)
        "#,
    );
    keys(&mut app, ":broken<CR>");
    let message = app
        .editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(
        message.contains("no command named no_such_command"),
        "{message}"
    );
    assert!(app.editor().popup().is_none());
}
