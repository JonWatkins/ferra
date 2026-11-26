//! The bottom panel: Problems (by file, Enter or a click goes to one),
//! Output (the messages shown), and its tabs.

mod common;

use std::path::PathBuf;

use common::{app, cursor, keys, render};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ferra::App;
use ferra::core::Position;
use ferra::editor::Mode;
use ferra::editor::diagnostics::{Diagnostic, Severity};
use ferra::editor::panel::PanelTab;

const W: u16 = 90;
const H: u16 = 24;

fn problem(row: usize, col: usize, severity: Severity, message: &str) -> Diagnostic {
    Diagnostic {
        start: Position::new(row, col),
        end: Position::new(row, col + 1),
        severity,
        message: message.into(),
        source: Some("rustc".into()),
    }
}

/// Two files open, with problems.
fn with_problems() -> (tempfile::TempDir, App, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    std::fs::write(&a, "fn a() {\n    let x = 1;\n}\n").unwrap();
    std::fs::write(&b, "fn b() {}\n").unwrap();
    let mut app = app();
    app.open(&a);
    let a_id = app.editor().document.id();
    app.open(&b);
    let b_id = app.editor().document.id();
    app.editor_mut().set_diagnostics(
        a_id,
        vec![
            problem(1, 8, Severity::Warning, "unused variable: `x`"),
            problem(0, 3, Severity::Error, "mismatched types"),
        ],
    );
    app.editor_mut()
        .set_diagnostics(b_id, vec![problem(0, 3, Severity::Error, "b is wrong")]);
    (dir, app, a, b)
}

fn screen(app: &mut App) -> String {
    render(app, W, H).rows.join("\n")
}

#[test]
fn problems_by_file_and_enter_goes_to_one() {
    let (_dir, mut app, a, _) = with_problems();
    keys(&mut app, ":problems<CR>");
    assert_eq!(app.editor().panel_shown(), Some(PanelTab::Problems));
    assert_eq!(app.editor().mode(), Mode::Panel);
    let shown = screen(&mut app);
    assert!(shown.contains("PROBLEMS 3"), "{shown}");
    assert!(shown.contains("a.rs  2"), "{shown}");
    assert!(shown.contains("⊗ mismatched types  rustc"), "{shown}");
    assert!(shown.contains("[Ln 2, Col 9]"), "{shown}");
    // a.rs's first, by place; j skips b.rs's file row.
    keys(&mut app, "j");
    keys(&mut app, "<CR>");
    assert_eq!(app.editor().document.path(), Some(a.as_path()));
    assert_eq!(cursor(&app), (1, 8), "the unused variable");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(
        app.editor().panel_shown(),
        Some(PanelTab::Problems),
        "still open"
    );
}

#[test]
fn none_says_so() {
    let mut app = app();
    keys(&mut app, ":problems<CR>");
    assert!(screen(&mut app).contains("No problems have been found"));
}

#[test]
fn output_keeps_the_messages_shown() {
    let mut app = app();
    keys(&mut app, ":nonsense<CR>:echo hello<CR>");
    keys(&mut app, ":output<CR>");
    let shown = screen(&mut app);
    assert!(shown.contains("OUTPUT"), "{shown}");
    assert!(shown.contains("Unknown command: nonsense"), "{shown}");
}

#[test]
fn tabs_esc_and_hiding() {
    let (_dir, mut app, ..) = with_problems();
    keys(&mut app, ":problems<CR><Tab>");
    assert_eq!(app.editor().panel_shown(), Some(PanelTab::Output));
    keys(&mut app, "<S-Tab>");
    assert_eq!(app.editor().panel_shown(), Some(PanelTab::Problems));
    // Esc: back to the text, the panel staying; :panel hides it.
    keys(&mut app, "<Esc>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(app.editor().panel_shown(), Some(PanelTab::Problems));
    keys(&mut app, ":panel<CR>");
    assert_eq!(app.editor().panel_shown(), None);
    // And back, at the same tab.
    keys(&mut app, ":panel<CR>");
    assert_eq!(app.editor().panel_shown(), Some(PanelTab::Problems));
}

#[test]
fn clicks_switch_tabs_and_go_to_problems() {
    let (_dir, mut app, _, b) = with_problems();
    keys(&mut app, ":problems<CR>");
    let rows = render(&mut app, W, H).rows;
    let click = |app: &mut App, x: u16, y: u16| {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            app.handle_mouse(MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            });
        }
    };
    // The panel's row (the text shows it too, after the line).
    let y = rows
        .iter()
        .rposition(|r| r.contains("⊗ b is wrong"))
        .unwrap();
    click(&mut app, 10, y as u16);
    assert_eq!(app.editor().document.path(), Some(b.as_path()));
    assert_eq!(cursor(&app), (0, 3));

    let rows = render(&mut app, W, H).rows;
    let y = rows.iter().position(|r| r.contains("OUTPUT")).unwrap();
    let x = rows[y].find("OUTPUT").unwrap() as u16;
    click(&mut app, x + 1, y as u16);
    assert_eq!(app.editor().panel_shown(), Some(PanelTab::Output));
}

#[test]
fn the_file_picker_says_what_to_type() {
    let mut app = app();
    keys(&mut app, ":file_picker<CR>");
    let shown = screen(&mut app);
    assert!(shown.contains("> Search files by name"), "{shown}");
    keys(&mut app, "x");
    assert!(!screen(&mut app).contains("Search files by name"));
}

/// Where `needle` starts on screen row `row`, in columns.
fn column(row: &str, needle: &str) -> u16 {
    row[..row.find(needle).unwrap()].chars().count() as u16
}

#[test]
fn more_than_one_shell_lists_them_on_the_right() {
    let mut app = common::app_with_config("[editor]\nshell = \"/bin/sh\"\n");
    let click = |app: &mut App, x: u16, y: u16| {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            app.handle_mouse(MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            });
        }
    };
    keys(&mut app, ":terminal<CR>");
    let one = screen(&mut app);
    assert!(!one.contains("│ $ sh"), "one shell, no list:\n{one}");

    // Ctrl+Shift+` starts another, shown, and both are listed.
    keys(&mut app, "<C-~>");
    assert_eq!(app.editor().terminals().len(), 2);
    assert_eq!(app.editor().terminals().shown_index(), 1);
    let rows = render(&mut app, W, H).rows;
    let listed: Vec<usize> = (0..rows.len())
        .filter(|&y| rows[y].contains("│ $ sh"))
        .collect();
    assert_eq!(listed.len(), 2, "{}", rows.join("\n"));

    // A click on the first shows it.
    let x = column(&rows[listed[0]], "$ sh");
    click(&mut app, x, listed[0] as u16);
    assert_eq!(app.editor().terminals().shown_index(), 0);
    assert_eq!(app.editor().mode(), Mode::Terminal);

    // Ctrl+PageDown goes to the next.
    keys(&mut app, "<C-PageDown>");
    assert_eq!(app.editor().terminals().shown_index(), 1);

    // The title row's + starts another, × ends the one shown.
    let rows = render(&mut app, W, H).rows;
    let title = rows.iter().rposition(|r| r.contains("TERMINAL")).unwrap();
    let plus = column(&rows[title], "+  ×");
    click(&mut app, plus, title as u16);
    assert_eq!(app.editor().terminals().len(), 3);
    assert_eq!(app.editor().terminals().shown_index(), 2);
    click(&mut app, plus + 3, title as u16);
    assert_eq!(app.editor().terminals().len(), 2);
    assert_eq!(app.editor().terminals().shown_index(), 1);

    // From the editor (in the terminal, `:` goes to the shell).
    keys(
        &mut app,
        "<C-\\><C-n>:terminal-close<CR>:terminal-close<CR>",
    );
    assert!(app.editor().terminals().is_empty());
    assert_eq!(app.editor().panel_shown(), None);
}

#[test]
fn the_list_of_shells_is_resized_by_dragging_its_line() {
    // With VS Code's keys (no modes), as with Vim's.
    let mut app = common::app_with_config("[editor]\nshell = \"/bin/sh\"\nkeymap = \"vscode\"\n");
    let mouse = |app: &mut App, kind: MouseEventKind, x: u16, y: u16| {
        app.handle_mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    };
    keys(&mut app, "<C-`><C-~>");
    let rows = render(&mut app, W, H).rows;
    let y = rows
        .iter()
        .position(|r| r.contains("│ $ sh"))
        .unwrap_or_else(|| panic!("{}", rows.join("\n")));
    let line = column(&rows[y], "│ $ sh");
    assert_eq!(W - line - 1, 22, "terminal-list-width's columns");

    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        line,
        y as u16,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        line - 10,
        y as u16,
    );
    mouse(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        line - 10,
        y as u16,
    );
    assert_eq!(app.editor().options.terminal_list_width, 32);
    let rows = render(&mut app, W, H).rows;
    assert_eq!(column(&rows[y], "│ $ sh"), line - 10, "the line moved");
    assert_eq!(app.editor().terminals().len(), 2, "a drag isn't a click");

    // At most half the panel, however far it's dragged.
    let line = line - 10;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        line,
        y as u16,
    );
    mouse(
        &mut app,
        MouseEventKind::Drag(MouseButton::Left),
        2,
        y as u16,
    );
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), 2, y as u16);
    let rows = render(&mut app, W, H).rows;
    assert_eq!(column(&rows[y], "│ $ sh"), W / 2 - 1, "half of it");
}

#[test]
fn a_program_in_the_terminal_that_asks_for_the_mouse_gets_it() {
    let dir = tempfile::tempdir().unwrap();
    // Asks for presses, releases and drags, in SGR's encoding, then shows
    // what it reads (`^[` for Esc).
    let script = dir.path().join("mouse.sh");
    std::fs::write(
        &script,
        "printf '\\033[?1002h\\033[?1006hready\\n'; stty -echo raw; cat -v\n",
    )
    .unwrap();
    let mut app = common::app_with_config("[editor]\nshell = \"/bin/sh\"\n");
    let mouse = |app: &mut App, kind: MouseEventKind, x: u16, y: u16| {
        app.handle_mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
    };
    let read = |app: &App| app.editor().terminal_panel().unwrap().text_lines().concat();
    let wait_for = |app: &mut App, what: &str| {
        let start = std::time::Instant::now();
        while !read(app).contains(what) && start.elapsed() < std::time::Duration::from_secs(3) {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(read(app).contains(what), "{what:?} in {:?}", read(app));
    };
    keys(&mut app, ":terminal<CR>");
    keys(&mut app, &format!("sh {}<CR>", script.display()));
    wait_for(&mut app, "ready");
    keys(&mut app, "<C-\\><C-n>");
    assert_eq!(app.editor().mode(), Mode::Normal);

    // The shell's screen starts a column in, under the title row.
    let rows = render(&mut app, W, H).rows;
    let title = rows.iter().rposition(|r| r.contains("TERMINAL")).unwrap() as u16;
    let (x, y) = (1 + 4, title + 1 + 2);

    // A click: the panel gets focus, and the program the press.
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
    assert_eq!(app.editor().mode(), Mode::Terminal);
    wait_for(&mut app, "^[[<0;5;3M");

    // Dragged off the top, it's still the program's (at its top row).
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), x, 0);
    wait_for(&mut app, "^[[<32;5;1M");
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x, 0);
    wait_for(&mut app, "^[[<0;5;1m");

    // The wheel goes to it too, rather than scrolling the history.
    mouse(&mut app, MouseEventKind::ScrollDown, x, y);
    wait_for(&mut app, "^[[<65;5;3M");
    assert_eq!(
        app.editor().terminal_panel().unwrap().screen().scrollback(),
        0
    );
}
