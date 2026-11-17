//! The agent's `show_locations`: places to step through in the panel's
//! Locations tab.

mod common;

use std::fs;

use common::{app, cursor, keys, render};
use ferra::agent::tools::{Called, call};
use ferra::editor::Mode;
use serde_json::json;

#[test]
fn the_agents_places_are_stepped_through_in_the_panel() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    fs::write(&a, "fn one() {}\nfn two() {}\nfn three() {}\n").unwrap();
    fs::write(&b, "let x = 1;\nlet y = parse(x);\n").unwrap();
    let mut app = app();
    app.open(&a);
    let args = json!({"title": "Callers of parse to update", "locations": [
        {"path": b.to_str().unwrap(), "line": 2, "column": 9, "note": "passes an int"},
        {"path": a.to_str().unwrap(), "line": 3},
        {"path": a.to_str().unwrap(), "line": 1, "note": "the first"},
    ]});
    let said = call(app.editor_mut(), 1, "show_locations", &args);
    assert_eq!(
        said,
        Called::Done(Ok(
            "Showing 3 places in 2 files in the panel's Locations tab (the user steps \
             through them with j/k and ↵)"
                .into()
        ))
    );
    // Shown, focus where it was.
    assert_eq!(app.editor().mode(), Mode::Normal);
    let screen = render(&mut app, 100, 30).rows.join("\n");
    assert!(screen.contains("LOCATIONS 3"), "{screen}");
    assert!(screen.contains("Callers of parse to update"), "{screen}");
    assert!(screen.contains("passes an int"), "{screen}");
    // No note: the line's text. a.rs's places in order.
    let first = screen.find("the first").expect(&screen);
    let third = screen.rfind("fn three() {}").expect(&screen);
    assert!(first < third, "{screen}");
    assert!(screen.contains("[Ln 2, Col 9]"), "{screen}");

    // The user steps to the second (b.rs is first: given first) and goes.
    keys(&mut app, ":locations<CR>");
    assert_eq!(app.editor().mode(), Mode::Panel);
    let state = app.editor().agent_editor_state();
    assert!(
        state.contains("Locations tab (\"Callers of parse to update\"): open, selected 1 of 3, "),
        "{state}"
    );
    assert!(state.contains("b.rs:2:9: passes an int"), "{state}");
    keys(&mut app, "j<CR>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert!(app.editor().document.name().ends_with("a.rs"));
    assert_eq!(cursor(&app), (0, 0), "a.rs:1, the first");

    // What can't be shown: said.
    let mut wrong = |args| match call(app_editor(&mut app), 2, "show_locations", &args) {
        Called::Done(Err(e)) => e,
        other => panic!("{other:?}"),
    };
    fn app_editor(app: &mut ferra::App) -> &mut ferra::editor::Editor {
        app.editor_mut()
    }
    let e = wrong(json!({"title": "x", "locations": [{"path": a.to_str().unwrap(), "line": 9}]}));
    assert!(e.ends_with("has 3 lines, not 9"), "{e}");
    let e = wrong(json!({"title": "x", "locations": [{"path": "no/such.rs", "line": 1}]}));
    assert!(e.contains("no such file"), "{e}");
    let e = wrong(json!({"title": "x", "locations": [{"path": a.to_str().unwrap(), "line": 0}]}));
    assert_eq!(e, "location 1: lines start at 1, not 0");
    let folder = dir.path().to_str().unwrap().to_string();
    let e = wrong(json!({"title": "x", "locations": [{"path": folder, "line": 1}]}));
    assert!(e.ends_with("is a folder, not a file"), "{e}");
    let e = wrong(json!({"title": "x", "locations": []}));
    assert_eq!(e, "No locations given");
}
