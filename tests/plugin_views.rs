//! Plugins' views in the sidebar end to end: registered with `editor.view`,
//! made by their Lua function before drawing, switched with `<Tab>`, their
//! own keys running the plugin's commands, and made again when asked.

mod common;

use common::{app, keys, render, text};
use ferra::App;
use ferra::editor::Mode;

fn with_plugin(source: &str) -> App {
    let mut app = app();
    app.load_plugin("test.lua", source).expect("plugin loads");
    app
}

const LETTERS: &str = r#"
made = 0
editor.view("letters", {
  title = "Letters",
  lines = function(ctx)
    made = made + 1
    return { "alpha", { { "beta", "keyword" } }, "made " .. made .. " for " .. ctx:line_count() .. " lines" }
  end,
  keys = { ["<CR>"] = "letters_pick" },
  refresh = { "after-save" },
})
editor.command("letters_pick", function(ctx)
  ctx:message(ctx:view_name() .. ":" .. tostring(ctx:view_selected()))
end)
"#;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn sidebar(app: &mut App) -> String {
    render(app, 70, 14)
        .rows
        .iter()
        .map(|row| row.chars().take(31).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_plugin_view_is_a_tab_in_the_sidebar_with_its_own_keys() {
    let mut app = with_plugin(LETTERS);
    keys(&mut app, ":sidebar_view letters<CR>");
    assert_eq!(app.editor().mode(), Mode::Sidebar);
    let shown = sidebar(&mut app);
    assert!(shown.contains("Files   Letters"), "{shown}");
    assert!(shown.contains(" LETTERS"), "{shown}");
    assert!(shown.contains(" alpha"), "{shown}");
    assert!(shown.contains("made 1 for 1 lines"), "{shown}");

    keys(&mut app, "j<CR>");
    assert_eq!(message(&app), "letters:2", "the plugin's <CR>");
    sidebar(&mut app);
    assert!(
        sidebar(&mut app).contains("made 1"),
        "not made again for nothing"
    );

    keys(&mut app, "R");
    assert!(sidebar(&mut app).contains("made 2"), "R makes it again");

    // Back to the files: <CR> is the tree's again.
    keys(&mut app, "<Tab>");
    let shown = sidebar(&mut app);
    assert!(!shown.contains("LETTERS"), "{shown}");
    keys(&mut app, "<S-Tab>");
    assert!(
        sidebar(&mut app).contains("made 3"),
        "shown again, made again"
    );
}

#[test]
fn views_are_made_again_after_their_events_and_for_another_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let mut app = with_plugin(LETTERS);
    keys(&mut app, &format!(":e {}<CR>", path.display()));
    keys(&mut app, ":sidebar_view letters<CR>");
    assert!(sidebar(&mut app).contains("made 1 for 1 lines"));
    keys(&mut app, "<Esc>ione<CR>two<Esc>");
    assert!(
        sidebar(&mut app).contains("made 1"),
        "an edit isn't one of its events"
    );
    keys(&mut app, ":w<CR>");
    assert!(
        sidebar(&mut app).contains("made 2 for 2 lines"),
        "after-save is"
    );
    keys(
        &mut app,
        &format!(":e {}<CR>", dir.path().join("b.txt").display()),
    );
    assert_eq!(text(&app), "");
    assert!(
        sidebar(&mut app).contains("made 3 for 1 lines"),
        "another buffer"
    );
}

#[test]
fn mistakes_are_reported() {
    let mut app = app();
    let error = app
        .load_plugin(
            "bad.lua",
            r#"editor.view("bad", { lines = function() return {} end, keys = { ["<CR>"] = "nope" } })"#,
        )
        .unwrap_err();
    assert!(error.contains("unknown command 'nope'"), "{error}");
    let error = app
        .load_plugin("bad2.lua", r#"editor.view("bad2", { title = "x" })"#)
        .unwrap_err();
    assert!(error.contains("needs a lines function"), "{error}");

    let mut app =
        with_plugin(r#"editor.view("broken", { lines = function() error("oops", 0) end })"#);
    keys(&mut app, ":sidebar_view broken<CR>");
    let shown = sidebar(&mut app);
    assert!(shown.contains("sidebar view broken"), "{shown}");
    assert!(message(&app).contains("oops"));
    keys(&mut app, ":sidebar_view nothing<CR>");
    assert!(message(&app).contains("No sidebar view nothing"));
}
