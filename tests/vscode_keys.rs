//! VS Code's keys (`keymap = "vscode"`, the default): no modes, typing
//! always; Shift and a move selects, typing replaces a selection; the
//! clipboard, undo by words, line editing; and switching keymaps live.

mod common;

use common::{app_as_shipped, app_with_config, cursor, keys, render, text};
use ferra::App;
use ferra::clipboard::MemoryClipboard;
use ferra::editor::Mode;

/// VS Code's keys, an in-memory clipboard, and `content` typed (no
/// auto-indent), the cursor at the start.
fn vscode(content: &str) -> (App, MemoryClipboard) {
    let clipboard = MemoryClipboard::default();
    let mut app = app_with_config("[editor]\nkeymap = \"vscode\"\nauto-indent = false\n");
    app.use_clipboard(Box::new(clipboard.clone()));
    for c in content.chars() {
        app.editor_mut().insert_text_char(c);
    }
    app.editor_mut().document.commit();
    keys(&mut app, "<C-Home>");
    (app, clipboard)
}

fn selected(app: &App) -> Option<String> {
    let editor = app.editor();
    let range = editor.selection_range()?;
    let ferra::core::Range::Chars { start, end } = range else {
        return None;
    };
    Some(editor.document.buffer().text_range(start, end))
}

#[test]
fn it_starts_typing_and_stays_typing() {
    let mut app = app_as_shipped();
    assert_eq!(app.editor().mode(), Mode::Insert, "the default keymap");
    keys(&mut app, "hello<Esc> world");
    assert_eq!(text(&app), "hello world");
    assert_eq!(app.editor().mode(), Mode::Insert);
    // Letters Vim would take as commands are text.
    keys(&mut app, "<CR>dd:q");
    assert_eq!(text(&app), "hello world\ndd:q");
    // No mode in the status line.
    let shown = render(&mut app, 80, 10).rows.join("\n");
    assert!(!shown.contains("INSERT"), "{shown}");
}

#[test]
fn shift_selects_and_typing_replaces_the_selection() {
    let (mut app, _) = vscode("hello world");
    keys(&mut app, "<S-Right><S-Right>");
    assert_eq!(app.editor().mode(), Mode::Visual);
    assert_eq!(selected(&app).as_deref(), Some("he"), "up to the cursor");
    keys(&mut app, "J");
    assert_eq!(text(&app), "Jllo world");
    assert_eq!(app.editor().mode(), Mode::Insert);

    // Ctrl+Shift+Right: by words; Backspace takes the selection.
    keys(&mut app, "<C-S-Right>");
    assert_eq!(selected(&app).as_deref(), Some("llo"));
    keys(&mut app, "<BS>");
    assert_eq!(text(&app), "J world");

    // An arrow without Shift drops the selection at its end.
    keys(&mut app, "<S-End><Left>");
    assert_eq!(app.editor().mode(), Mode::Insert);
    assert_eq!(cursor(&app), (0, 1));
    keys(&mut app, "<S-End><Right>");
    assert_eq!(cursor(&app), (0, 7), "the selection's end");

    // Shift+Home from the end: to the first non-blank.
    keys(&mut app, "<S-Home>");
    assert_eq!(selected(&app).as_deref(), Some("J world"));
}

#[test]
fn copy_cut_and_paste_use_the_clipboard() {
    let (mut app, clipboard) = vscode("one two\nthree");
    keys(&mut app, "<C-S-Right><C-c>");
    assert_eq!(clipboard.contents(), "one");
    assert_eq!(selected(&app).as_deref(), Some("one"), "still selected");
    // Pasted over a selection.
    keys(&mut app, "<C-Right><C-S-Left><C-v>");
    assert_eq!(text(&app), "one one\nthree");
    // Nothing selected: Ctrl+X cuts the whole line, and Ctrl+V puts it
    // back above the cursor's.
    keys(&mut app, "<C-x>");
    assert_eq!(text(&app), "three");
    assert_eq!(clipboard.contents(), "one one\n");
    keys(&mut app, "<C-v>");
    assert_eq!(text(&app), "one one\nthree");
    // Ctrl+A, then typing replaces everything.
    keys(&mut app, "<C-a>x");
    assert_eq!(text(&app), "x");
}

#[test]
fn undo_takes_a_word_at_a_time() {
    let (mut app, _) = vscode("");
    keys(&mut app, "one two three");
    keys(&mut app, "<C-z>");
    assert_eq!(text(&app), "one two ");
    keys(&mut app, "<C-z>");
    assert_eq!(text(&app), "one ");
    keys(&mut app, "<C-y>");
    assert_eq!(text(&app), "one two ");
    assert_eq!(app.editor().mode(), Mode::Insert);
    // A command between ends a step too.
    keys(&mut app, "<C-End><C-S-k>");
    assert_eq!(text(&app), "");
    keys(&mut app, "<C-z>");
    assert_eq!(text(&app), "one two ");
}

#[test]
fn lines_move_copy_and_go() {
    let (mut app, _) = vscode("a\nb\nc");
    keys(&mut app, "<A-Down>");
    assert_eq!(text(&app), "b\na\nc");
    assert_eq!(cursor(&app), (1, 0), "with the line");
    keys(&mut app, "<A-S-Down>");
    assert_eq!(text(&app), "b\na\na\nc");
    assert_eq!(cursor(&app).0, 2);
    keys(&mut app, "<C-S-k>");
    assert_eq!(text(&app), "b\na\nc");
    // A selection over lines moves them all.
    keys(&mut app, "<C-Home><S-Down><S-Right><A-Down>");
    assert_eq!(text(&app), "c\nb\na");
    assert_eq!(selected(&app).as_deref(), Some("b\na"));
}

#[test]
fn comments_and_indents_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.sh");
    std::fs::write(&path, "echo a\necho b\n").unwrap();
    let (mut app, _) = vscode("");
    app.open(&path);
    keys(&mut app, "<C-/>");
    assert_eq!(text(&app), "# echo a\necho b");
    keys(&mut app, "<C-/><C-a><Tab>");
    assert_eq!(text(&app), "    echo a\n    echo b");
    assert_eq!(app.editor().mode(), Mode::Visual, "still selected");
    keys(&mut app, "<S-Tab>");
    assert_eq!(text(&app), "echo a\necho b");
}

#[test]
fn ctrl_d_selects_the_word_then_its_next_match() {
    let (mut app, _) = vscode("foo bar foo");
    keys(&mut app, "<C-d>");
    assert_eq!(selected(&app).as_deref(), Some("foo"));
    keys(&mut app, "<C-d>");
    assert_eq!(cursor(&app), (0, 11), "the next one");
    keys(&mut app, "baz");
    assert_eq!(text(&app), "foo bar baz");
}

#[test]
fn escape_drops_the_selection_and_nothing_else() {
    let (mut app, _) = vscode("abc");
    keys(&mut app, "<S-End><Esc>");
    assert_eq!(app.editor().mode(), Mode::Insert);
    assert!(app.editor().selection().is_none());
    assert_eq!(cursor(&app), (0, 3));
    keys(&mut app, "<Esc><Esc>d");
    assert_eq!(text(&app), "abcd");
}

#[test]
fn the_usual_keys_open_things() {
    let (mut app, _) = vscode("x");
    keys(&mut app, "<C-p>");
    assert_eq!(app.editor().mode(), Mode::Picker);
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Insert, "back to typing");
    keys(&mut app, "<F1>");
    assert_eq!(app.editor().mode(), Mode::Picker, "the command palette");
    keys(&mut app, "<Esc><C-f>");
    assert_eq!(app.editor().mode(), Mode::Find);
    keys(&mut app, "<Esc><C-b>");
    assert!(app.editor().sidebar().is_some());
    keys(&mut app, "<C-b>");
    assert!(app.editor().sidebar().is_none());
    assert_eq!(app.editor().mode(), Mode::Insert);
}

#[test]
fn switching_keymaps_takes_effect_at_once() {
    let (mut app, _) = vscode("abc");
    keys(&mut app, ":");
    assert_eq!(text(&app), ":abc", "typed");
    keys(&mut app, "<BS><C-g>set keymap=vim<CR>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "x");
    assert_eq!(text(&app), "bc", "Vim's x");
    keys(&mut app, ":set keymap=vscode<CR>");
    assert_eq!(app.editor().mode(), Mode::Insert);
    keys(&mut app, "x");
    assert_eq!(text(&app), "xbc");
}

#[test]
fn the_settings_editor_switches_them() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let (mut app, _) = vscode("abc");
    app.editor_mut().set_config_file(Some(config.clone()));
    keys(&mut app, "<C-g>settings keymap<CR>");
    assert_eq!(app.editor().mode(), Mode::Settings);
    keys(&mut app, "<CR>");
    assert_eq!(
        app.editor().global_options().keymap,
        ferra::config::KeymapPreset::Vim
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "[editor]\nkeymap = \"vim\"\n"
    );
    // Still in the settings editor, now with Vim's keys around it.
    assert_eq!(app.editor().mode(), Mode::Settings);
    keys(&mut app, "q");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn menus_show_vscode_keys() {
    let mut app = app_as_shipped();
    keys(&mut app, "<F10>");
    let shown = render(&mut app, 80, 24).rows.join("\n");
    let save = if cfg!(target_os = "macos") {
        "⌃S"
    } else {
        "Ctrl+S"
    };
    assert!(shown.contains(save), "{shown}");
}
