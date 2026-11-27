//! Terminal-normal mode (`editor/terminal_normal.rs`), end to end: the
//! terminal panel's text as a read-only buffer, with Vim's keys and VS
//! Code's, and the mouse.

mod common;

use common::{keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::clipboard::MemoryClipboard;
use ferra::editor::Mode;
use ferra::editor::windows::PANEL_WINDOW;

const W: u16 = 90;
const H: u16 = 24;

/// An app with a shell in the panel that's printed `alpha`, `beta`,
/// `gamma`, with focus there.
fn with_shell(config: &str) -> (App, MemoryClipboard) {
    let mut app = common::app_with_config(&format!("[editor]\nshell = \"/bin/sh\"\n{config}"));
    let clipboard = MemoryClipboard::default();
    app.use_clipboard(Box::new(clipboard.clone()));
    // `<C-`>` in either keymap (VS Code's has no `:` line).
    keys(&mut app, "<C-`>");
    keys(&mut app, "echo alpha; echo beta; echo gamma<CR>");
    wait_for(&mut app, "gamma");
    (app, clipboard)
}

fn shell_text(app: &App) -> Vec<String> {
    app.editor()
        .terminals()
        .shown()
        .map(|t| t.text_lines())
        .unwrap_or_default()
}

fn wait_for(app: &mut App, line: &str) {
    let start = std::time::Instant::now();
    while !shell_text(app).iter().any(|l| l == line)
        && start.elapsed() < std::time::Duration::from_secs(3)
    {
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        shell_text(app).iter().any(|l| l == line),
        "{line:?} in {:?}",
        shell_text(app)
    );
}

fn unnamed(app: &App) -> String {
    app.editor()
        .registers
        .get(None)
        .map(|r| r.text.clone())
        .unwrap_or_default()
}

fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
    app.handle_mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
}

/// Where `text` is on screen: its column and row.
fn on_screen(app: &mut App, text: &str) -> (u16, u16) {
    let rows = render(app, W, H).rows;
    let y = rows
        .iter()
        .rposition(|r| r.contains(text))
        .unwrap_or_else(|| panic!("{text:?} on screen:\n{}", rows.join("\n")));
    let x = rows[y].find(text).unwrap();
    (rows[y][..x].chars().count() as u16, y as u16)
}

#[test]
fn search_and_yank_from_the_shells_text_then_i_goes_back() {
    let (mut app, _) = with_shell("");
    keys(&mut app, "<C-\\><C-n>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(app.editor().in_scrollback());

    // Drawn in the panel, without line numbers, where the shell was.
    let screen = render(&mut app, W, H);
    let beta = screen
        .rows
        .iter()
        .rposition(|r| r.contains("beta"))
        .unwrap();
    assert!(
        screen.rows[beta].starts_with(" beta"),
        "a column in, no numbers: {:?}",
        screen.rows[beta]
    );

    keys(&mut app, "?beta<CR>yiw");
    assert_eq!(unnamed(&app), "beta");
    keys(&mut app, "jVy");
    assert_eq!(unnamed(&app), "gamma");
    assert!(app.editor().registers.get(None).unwrap().linewise);

    // Edits are refused, with a message.
    let before = app.editor().document.buffer().text();
    keys(&mut app, "dd");
    assert_eq!(app.editor().document.buffer().text(), before);
    let message = app.editor().message().map(|m| m.text.clone());
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.contains("can't be changed")),
        "{message:?}"
    );

    // `i`: the shell again, which gets what's typed.
    keys(&mut app, "i");
    assert_eq!(app.editor().mode(), Mode::Terminal);
    assert!(!app.editor().in_scrollback());
    keys(&mut app, "echo delta<CR>");
    wait_for(&mut app, "delta");
}

#[test]
fn ctrl_w_k_goes_to_the_window_it_came_from() {
    let (mut app, _) = with_shell("");
    keys(&mut app, "<C-\\><C-n>");
    keys(&mut app, "<C-w>k");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(!app.editor().in_scrollback());
    assert!(!app.editor().document.is_read_only());
    assert!(app.editor().terminal_panel().is_some(), "still open");
    // And back to the shell with <Space>t.
    keys(&mut app, "<Space>t");
    assert_eq!(app.editor().mode(), Mode::Terminal);
}

#[test]
fn a_drag_over_the_shell_selects_its_text() {
    let (mut app, _) = with_shell("");
    keys(&mut app, "<C-\\><C-n><C-w>k");
    let (x, y) = on_screen(&mut app, "beta");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 1, y);
    assert!(app.editor().in_scrollback());
    assert_eq!(app.editor().mode(), Mode::Visual);
    assert_eq!(app.editor().windows().active(), PANEL_WINDOW);
    // Drawn as it was: the text didn't move.
    assert_eq!(on_screen(&mut app, "beta"), (x, y));
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 3, y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 3, y);
    keys(&mut app, "y");
    assert_eq!(unnamed(&app), "beta");
}

#[test]
fn with_vscode_keys_a_drag_selects_ctrl_c_copies_and_typing_goes_to_the_shell() {
    let (mut app, clipboard) = with_shell("keymap = \"vscode\"\n");
    let (x, y) = on_screen(&mut app, "gamma");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    assert_eq!(app.editor().mode(), Mode::Terminal);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 1, y);
    render(&mut app, W, H);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 5, y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 5, y);
    assert!(app.editor().in_scrollback());
    keys(&mut app, "<C-c>");
    assert_eq!(clipboard.contents(), "gamma");

    // A key typed: the shell's again, and it gets it.
    keys(&mut app, "e");
    assert_eq!(app.editor().mode(), Mode::Terminal);
    assert!(!app.editor().in_scrollback());
    keys(&mut app, "cho epsilon<CR>");
    wait_for(&mut app, "epsilon");

    // A click (no drag) in a selection: the shell too.
    let (x, y) = on_screen(&mut app, "gamma");
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x + 2, y);
    render(&mut app, W, H);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x + 2, y);
    assert!(app.editor().in_scrollback());
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x, y);
    assert_eq!(app.editor().mode(), Mode::Terminal);
    assert!(!app.editor().in_scrollback());
}
