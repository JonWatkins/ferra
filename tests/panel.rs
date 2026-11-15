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
