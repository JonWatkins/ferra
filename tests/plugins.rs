//! Lua plugins end to end: registering commands, hooks and key bindings, the
//! `ctx` API, error handling and loading from the config directory.

mod common;

use std::fs;

use common::{app, cursor, keys, text};
use terminal_editor::App;

fn message(app: &App) -> String {
    app.editor()
        .message()
        .map(|m| m.text.clone())
        .unwrap_or_default()
}

fn with_plugin(source: &str) -> App {
    let mut app = app();
    app.load_plugin("test.lua", source).expect("plugin loads");
    app
}

#[test]
fn commands_run_from_the_command_line_and_palette() {
    let mut app = with_plugin(
        r#"
        editor.command("greet", "Insert a greeting", function(ctx)
          ctx:insert("hello from lua")
        end)
        "#,
    );
    keys(&mut app, ":greet<CR>");
    assert_eq!(text(&app), "hello from lua");

    keys(&mut app, "<Space>pgreet");
    let picker = app.editor().picker().unwrap();
    let item = picker.selected_item().unwrap();
    assert_eq!(
        (item.label.as_str(), item.detail.as_str()),
        ("greet", "Insert a greeting")
    );
}

#[test]
fn key_bindings_and_counts() {
    let mut app = with_plugin(
        r#"
        editor.command("stars", function(ctx)
          ctx:insert(string.rep("*", ctx.count or 1))
        end)
        editor.map("normal", "<Space>s", "stars")
        "#,
    );
    keys(&mut app, "3<Space>s");
    assert_eq!(text(&app), "***");
}

#[test]
fn line_api() {
    let mut app = with_plugin(
        r#"
        editor.command("rewrite", function(ctx)
          assert(ctx:line_count() == 3)
          assert(ctx:line(2) == "two")
          assert(ctx:line(9) == nil)
          ctx:set_line(1, ctx:line(1):upper())
          ctx:insert_line(2, "inserted")
          ctx:insert_line(ctx:line_count() + 1, "appended")
          ctx:delete_line(4)
        end)
        "#,
    );
    keys(&mut app, "ione<CR>two<CR>three<Esc>:rewrite<CR>");
    assert_eq!(text(&app), "ONE\ninserted\ntwo\nappended");
    assert_eq!(message(&app), "");
}

#[test]
fn cursor_path_mode_message_and_args() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, "abc\ndef\n").unwrap();

    let mut app = with_plugin(
        r#"
        editor.command("info", function(ctx)
          ctx:set_cursor(2, 3)
          local row, col = ctx:cursor()
          local name = ctx:path():match("[^/]+$")
          ctx:message(name .. " " .. row .. ":" .. col .. " " .. ctx:mode() .. " " .. table.concat(ctx.args, ","))
        end)
        "#,
    );
    app.open(&path);
    keys(&mut app, ":info a b<CR>");
    assert_eq!(message(&app), "notes.txt 2:3 normal a,b");
    assert_eq!(cursor(&app), (1, 2));
}

#[test]
fn run_calls_other_commands_including_plugin_ones() {
    let mut app = with_plugin(
        r#"
        editor.command("inner", function(ctx) ctx:insert("[inner]") end)
        editor.command("outer", function(ctx)
          ctx:run("inner")
          ctx:run("goto_line", "1")
          ctx:insert(">")
        end)
        "#,
    );
    // After <Esc> the cursor is on the `y`, so `inner` inserts before it.
    keys(&mut app, "ix<CR>y<Esc>:outer<CR>");
    assert_eq!(text(&app), ">x\n[inner]y");
}

#[test]
fn plugin_edits_undo_as_one_step_and_repeat_with_dot() {
    let mut app = with_plugin(
        r#"
        editor.command("wrap", function(ctx)
          local row = ctx:cursor()
          ctx:set_line(row, "<" .. ctx:line(row) .. ">")
        end)
        editor.map("normal", "<Space>w", "wrap")
        "#,
    );
    keys(&mut app, "ia<CR>b<Esc>gg");
    keys(&mut app, "<Space>w");
    assert_eq!(text(&app), "<a>\nb");
    keys(&mut app, "j.");
    assert_eq!(text(&app), "<a>\n<b>");
    keys(&mut app, "u");
    assert_eq!(text(&app), "<a>\nb");
}

#[test]
fn lua_errors_become_command_errors() {
    let mut app = with_plugin(
        r#"
        editor.command("boom", function(ctx) error("it broke") end)
        editor.command("badrow", function(ctx) ctx:set_line(99, "x") end)
        "#,
    );
    keys(&mut app, ":boom<CR>");
    assert!(message(&app).contains("it broke"), "{}", message(&app));
    keys(&mut app, ":badrow<CR>");
    assert!(
        message(&app).contains("line 99 is out of range"),
        "{}",
        message(&app)
    );
}

#[test]
fn ctx_cannot_be_used_after_the_call() {
    let mut app = with_plugin(
        r#"
        editor.command("keep", function(ctx) saved = ctx end)
        editor.command("misuse", function(ctx) saved:insert("x") end)
        "#,
    );
    keys(&mut app, ":keep<CR>:misuse<CR>");
    assert_eq!(text(&app), "", "the stale ctx did nothing");
    assert!(!message(&app).is_empty(), "and reported an error");
}

#[test]
fn hooks_edit_before_save_and_can_cancel_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "x  \n").unwrap();

    let mut app = with_plugin(
        r#"
        editor.on("before-save", function(ctx, event)
          assert(event.kind == "before-save")
          for row = 1, ctx:line_count() do
            ctx:set_line(row, (ctx:line(row):gsub("%s+$", "")))
          end
          if ctx:text():find("SECRET") then error("refusing to save a secret") end
        end)
        "#,
    );
    app.open(&path);
    keys(&mut app, ":w<CR>");
    assert_eq!(fs::read_to_string(&path).unwrap(), "x\n");

    keys(&mut app, "OSECRET<Esc>:w<CR>");
    assert!(
        message(&app).contains("refusing to save a secret"),
        "{}",
        message(&app)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "x\n");
}

#[test]
fn hooks_receive_event_details() {
    let mut app = with_plugin(
        r#"
        editor.on("mode-changed", function(ctx, event)
          if event.to == "insert" then ctx:message("from " .. event.from) end
        end)
        "#,
    );
    keys(&mut app, "i");
    assert_eq!(message(&app), "from normal");
}

#[test]
fn plugins_can_override_built_in_commands() {
    let mut app =
        with_plugin(r#"editor.command("delete_char", function(ctx) ctx:insert("!") end)"#);
    keys(&mut app, "x");
    assert_eq!(text(&app), "!");
}

#[test]
fn load_errors_are_reported() {
    let mut app = app();
    let err = app
        .load_plugin("broken.lua", "this is not lua")
        .unwrap_err();
    assert!(err.starts_with("broken.lua:"), "{err}");

    let err = app
        .load_plugin("events.lua", r#"editor.on("nope", function() end)"#)
        .unwrap_err();
    assert!(err.contains("unknown event 'nope'"), "{err}");

    let err = app
        .load_plugin("maps.lua", r#"editor.map("sideways", "x", "missing")"#)
        .unwrap_err();
    assert!(err.contains("unknown mode 'sideways'"), "{err}");
}

#[test]
fn loads_init_and_plugin_directory_in_order() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("plugins")).unwrap();
    fs::write(
        dir.path().join("init.lua"),
        r#"editor.command("who", function(ctx) ctx:insert("init") end)"#,
    )
    .unwrap();
    fs::write(dir.path().join("plugins/a.lua"), "this does not parse").unwrap();
    fs::write(
        dir.path().join("plugins/b.lua"),
        r#"editor.command("who", function(ctx) ctx:insert("b") end)
           editor.map("normal", "<Space>w", "who")"#,
    )
    .unwrap();
    fs::write(dir.path().join("plugins/notes.txt"), "ignored").unwrap();

    let mut app = app();
    let error = app.load_plugins(dir.path()).expect("a.lua fails");
    assert!(error.contains("a.lua"), "{error}");
    assert!(!error.contains("b.lua"), "{error}");

    keys(&mut app, "<Space>w");
    assert_eq!(
        text(&app),
        "b",
        "later files override earlier ones, despite a.lua"
    );
}

#[test]
fn no_config_directory_means_no_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    assert_eq!(app.load_plugins(dir.path()), None);
}
