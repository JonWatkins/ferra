//! What `editor_state` tells the coding agent about where the user is.

mod common;

use std::fs;

use common::{app, keys, render};
use ferra::core::Position;
use ferra::editor::diagnostics::{Diagnostic, Severity};
use ferra::editor::outline::OutlineSymbol;
use ferra::editor::panel::PanelTab;

fn symbol(name: &str, kind: &str, depth: usize, start: usize, end: usize) -> OutlineSymbol {
    OutlineSymbol {
        name: name.into(),
        kind: kind.into(),
        depth,
        start: Position::new(start, 0),
        end: Position::new(end, 0),
        at: Position::new(start, 0),
    }
}

#[test]
fn editor_state_says_what_this_and_here_are() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    let text: String = (1..=60).map(|n| format!("line {n}\n")).collect();
    fs::write(&path, text).unwrap();
    let mut app = app();
    app.open(&path);
    render(&mut app, 80, 20);

    // The outline the breadcrumbs ask for: the cursor's in bar, in impl Foo.
    let id = app.editor().document.id();
    let seq = app.editor().document.buffer().edit_seq();
    app.editor_mut().set_outline(
        id,
        seq,
        vec![
            symbol("impl Foo", "object", 0, 2, 30),
            symbol("bar", "method", 1, 5, 12),
            symbol("baz", "method", 1, 14, 20),
        ],
    );
    keys(&mut app, "3Gix<Esc>9Giy<Esc>");
    render(&mut app, 80, 20);
    let state = app.editor().agent_editor_state();
    assert!(
        state.contains("In: impl Foo > bar (method, lines 6-13)"),
        "{state}"
    );
    assert!(state.contains("Showing lines 1-"), "{state}");
    assert!(
        state.contains("Last changed here (newest first): line 9, 3"),
        "{state}"
    );
    // The prompt's note says it too.
    let context = app.editor().agent_context().unwrap();
    assert!(context.contains("In: impl Foo > bar"), "{context}");

    // Scrolled down: the lines shown follow.
    keys(&mut app, "50G");
    render(&mut app, 80, 20);
    let state = app.editor().agent_editor_state();
    assert!(!state.contains("Showing lines 1-"), "{state}");
    assert!(!state.contains("In: "), "outside every symbol: {state}");

    // The note's lines all end alike (no full stops after the first).
    assert!(
        context.lines().skip(1).all(|l| !l.ends_with('.')),
        "{context}"
    );

    // The Problems panel, with a problem selected.
    app.editor_mut().set_diagnostics(
        id,
        vec![Diagnostic {
            start: Position::new(4, 0),
            end: Position::new(4, 4),
            severity: Severity::Error,
            message: "mismatched types".into(),
            source: Some("rustc".into()),
        }],
    );
    app.editor_mut().panel_show(PanelTab::Problems).unwrap();
    let state = app.editor().agent_editor_state();
    assert!(
        state.contains("Problems panel: open, selected") && state.contains("mismatched types"),
        "{state}"
    );
    // And the note sent with each message, so "this problem" needs no tool.
    let context = app.editor().agent_context().unwrap();
    assert!(
        context
            .lines()
            .any(|l| l.starts_with("Problems panel: open, selected ")
                && l.ends_with("a.rs:5: error: mismatched types (rustc)")),
        "{context}"
    );
}
