//! The mouse end to end: a frame is drawn, then clicks, drags and the
//! wheel land where things were drawn.

mod common;

use common::{app, cursor, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::editor::Mode;

const W: u16 = 60;
const H: u16 = 40;

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.handle_mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}

fn click(app: &mut App, x: u16, y: u16) {
    mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
}

/// Where `text` was drawn: its first cell.
fn find(app: &mut App, text: &str) -> (u16, u16) {
    let screen = render(app, W, H);
    for (y, row) in screen.rows.iter().enumerate() {
        if let Some(byte) = row.find(text) {
            return (row[..byte].chars().count() as u16, y as u16);
        }
    }
    panic!("{text:?} isn't on screen:\n{}", screen.rows.join("\n"));
}

fn with_text(text: &str) -> App {
    let mut app = app();
    keys(&mut app, &format!("i{text}<Esc>gg0"));
    app
}

#[test]
fn a_click_puts_the_cursor_there() {
    let mut app = with_text("hello world\nsecond line");
    let (x, y) = find(&mut app, "line");
    click(&mut app, x + 1, y);
    assert_eq!(cursor(&app), (1, 8), "on the i of line");
    // Past the end of a line: its last char, in normal mode.
    let (x, y) = find(&mut app, "world");
    click(&mut app, x + 20, y);
    assert_eq!(cursor(&app), (0, 10));
}

#[test]
fn a_click_in_insert_mode_stays_in_insert_mode() {
    let mut app = with_text("hello world");
    keys(&mut app, "A");
    let (x, y) = find(&mut app, "world");
    click(&mut app, x, y);
    assert_eq!(app.editor().mode(), Mode::Insert);
    assert_eq!(cursor(&app), (0, 6));
    keys(&mut app, "big ");
    assert_eq!(common::text(&app), "hello big world");
}

#[test]
fn dragging_selects() {
    let mut app = with_text("hello world");
    let (x, y) = find(&mut app, "hello");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x + 1, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 3, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 7, y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 7, y);
    assert_eq!(app.editor().mode(), Mode::Visual);
    keys(&mut app, "d");
    // Through the char released on, as visual mode always includes it.
    assert_eq!(
        common::text(&app),
        "hrld",
        "ello wo, from the click to the release"
    );
}

#[test]
fn a_click_focuses_another_window() {
    let mut app = with_text("one\ntwo\nthree");
    keys(&mut app, "<C-w>v");
    render(&mut app, W, H);
    click(&mut app, 6, 2);
    let left = app.editor().windows().active();
    assert_eq!(cursor(&app).0, 2);
    click(&mut app, W - 25, 1);
    let right = app.editor().windows().active();
    assert_ne!(left, right, "the other window");
    assert_eq!(cursor(&app).0, 1);
    click(&mut app, 6, 0);
    assert_eq!(app.editor().windows().active(), left);
}

#[test]
fn the_wheel_scrolls_the_window_under_the_pointer() {
    let lines: Vec<String> = (1..=50).map(|n| format!("line {n}")).collect();
    let mut app = with_text(&lines.join("\n"));
    let (x, y) = find(&mut app, "line 1");
    mouse(&mut app, MouseEventKind::ScrollDown, x, y);
    mouse(&mut app, MouseEventKind::ScrollDown, x, y);
    let screen = render(&mut app, W, H);
    assert!(
        screen.rows[0].contains("line 7"),
        "{}",
        screen.rows.join("\n")
    );
    mouse(&mut app, MouseEventKind::ScrollUp, x, y);
    let screen = render(&mut app, W, H);
    assert!(
        screen.rows[0].contains("line 4"),
        "{}",
        screen.rows.join("\n")
    );
}

#[test]
fn clicks_in_the_sidebar_open_files_and_folders() {
    // The tree is the folder the tests run in: this repository.
    let mut app = app();
    keys(&mut app, "<Space>e<Esc>");
    let (x, y) = find(&mut app, "src");
    click(&mut app, x, y);
    assert_eq!(app.editor().mode(), Mode::Sidebar);
    let (x, y) = find(&mut app, "main.rs");
    click(&mut app, x, y);
    let path = app.editor().document.path().unwrap().to_path_buf();
    assert!(path.ends_with("src/main.rs"), "{path:?}");
    assert_eq!(
        app.editor().mode(),
        Mode::Normal,
        "opened, back in the editor"
    );
}

#[test]
fn a_click_chooses_in_the_picker_and_outside_closes_it() {
    let mut app = with_text("hello");
    keys(&mut app, "<Space>p");
    keys(&mut app, "delete_char_for");
    let (x, y) = find(&mut app, "delete_char_forward");
    click(&mut app, x, y);
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(common::text(&app), "ello", "ran it");

    keys(&mut app, "<Space>p");
    render(&mut app, W, H);
    click(&mut app, 0, H - 3);
    assert_eq!(
        app.editor().mode(),
        Mode::Normal,
        "a click outside closes it"
    );
    assert_eq!(common::text(&app), "ello");
}

#[test]
fn the_find_box_takes_clicks_on_its_toggles_and_fields() {
    // Below where the box is drawn, at the top of the window.
    let mut app = with_text("cat\n\n\n\n\n\n\n\nCat cat concat");
    keys(&mut app, "<Space>h");
    let (x, y) = find(&mut app, "Match case");
    click(&mut app, x, y);
    assert!(app.editor().find_state().match_case, "toggled");
    let (x, y) = find(&mut app, "Whole word");
    click(&mut app, x - 4, y);
    assert!(app.editor().find_state().whole_word, "on its chip");
    let (x, y) = find(&mut app, "Replace");
    click(&mut app, x + 12, y);
    assert_eq!(
        app.editor().find_state().focus,
        ferra::editor::find::FindField::Replace
    );

    // Out of the box, into the text: it closes, keeping the search for
    // `n` but not its highlights.
    let (x, y) = find(&mut app, "concat");
    click(&mut app, x + 1, y);
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(cursor(&app), (8, 9));
    assert!(app.editor().search_highlight().is_none());
    keys(&mut app, "n");
    assert!(app.editor().search_highlight().is_some());
}

#[test]
fn double_and_triple_clicks_select_the_word_and_the_line() {
    let mut app = with_text("one two three\nnext");
    let (x, y) = find(&mut app, "two");
    click(&mut app, x + 1, y);
    click(&mut app, x + 1, y);
    assert_eq!(app.editor().mode(), Mode::Visual);
    keys(&mut app, "y");
    assert_eq!(app.editor().registers.get(None).unwrap().text, "two");

    click(&mut app, x, y);
    click(&mut app, x, y);
    click(&mut app, x, y);
    keys(&mut app, "d");
    assert_eq!(common::text(&app), "next", "the whole line");
}

#[test]
fn dragging_edges_resizes_the_sidebar_and_windows() {
    let mut app = with_text("one\ntwo");
    keys(&mut app, "<Space>e<Esc>");
    let screen = render(&mut app, W, H);
    let edge = screen.rows[1].chars().position(|c| c == '│').unwrap() as u16;
    assert_eq!(
        app.pointer_for(ratatui::layout::Position::new(edge, 3)),
        "ew-resize"
    );
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), edge, 3);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 20, 3);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), 20, 3);
    assert_eq!(app.editor().options.sidebar_width, 20);
    keys(&mut app, ":explorer<CR>");

    keys(&mut app, "<C-w>v");
    let screen = render(&mut app, W, H);
    let line = screen.rows[1].chars().position(|c| c == '│').unwrap() as u16;
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), line, 2);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), 15, 2);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), 15, 2);
    let screen = render(&mut app, W, H);
    let moved = screen.rows[1].chars().position(|c| c == '│').unwrap() as u16;
    assert_eq!(moved, 15, "{}", screen.rows.join("\n"));
}

#[test]
fn a_plugins_list_popup_and_input_box_take_clicks() {
    let mut app = app();
    app.load_plugin(
        "t.lua",
        r#"
        editor.command("fruit", function(ctx)
          ctx:popup({ title = "Fruit", lines = { "apple", "banana" }, select = true,
                      keys = { ["<CR>"] = "fruit_pick" } })
        end)
        editor.command("fruit_pick", function(ctx)
          local n = ctx:popup_selected()
          ctx:popup_close()
          ctx:insert(({ "apple", "banana" })[n])
        end)
        editor.command("ask", function(ctx)
          ctx:input({ title = "Name", value = "abcdef", on_submit = "fruit_pick", on_cancel = "said_no" })
        end)
        editor.command("said_no", function(ctx) ctx:insert("no") end)
        "#,
    )
    .unwrap();
    keys(&mut app, ":fruit<CR>");
    let (x, y) = find(&mut app, "banana");
    assert_eq!(
        app.pointer_for(ratatui::layout::Position::new(x, y)),
        "pointer"
    );
    click(&mut app, x, y);
    assert_eq!(common::text(&app), "banana");

    keys(&mut app, ":ask<CR>");
    let (x, y) = find(&mut app, "abcdef");
    click(&mut app, x + 2, y);
    keys(&mut app, "X");
    assert_eq!(
        app.editor().input_box().unwrap().field.text(),
        "abXcdef",
        "the cursor went there"
    );
    click(&mut app, 2, H - 4);
    assert!(
        app.editor().input_box().is_none(),
        "a click outside cancels"
    );
    // Inserted before the cursor, which was on banana's last letter.
    assert_eq!(common::text(&app), "banannoa");
}

#[test]
fn the_pointer_shows_what_can_be_clicked() {
    let mut app = with_text("some text");
    keys(&mut app, "<Space>e<Esc>");
    let (x, y) = find(&mut app, "src");
    assert_eq!(
        app.pointer_for(ratatui::layout::Position::new(x, y)),
        "pointer"
    );
    let (x, y) = find(&mut app, "some text");
    assert_eq!(
        app.pointer_for(ratatui::layout::Position::new(x, y)),
        "text"
    );
    assert_eq!(
        app.pointer_for(ratatui::layout::Position::new(5, H - 1)),
        "default"
    );
}

#[test]
fn mouse_off_ignores_it() {
    let mut app = common::app_with_config("[editor]\nmouse = false");
    keys(&mut app, "ihello world<Esc>0");
    render(&mut app, W, H);
    click(&mut app, 12, 0);
    assert_eq!(cursor(&app), (0, 0));
}
