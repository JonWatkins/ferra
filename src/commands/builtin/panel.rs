//! The bottom panel (`editor/panel.rs`): Problems, Output, Terminal.

use crate::commands::CommandRegistry;
use crate::editor::panel::PanelTab;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "panel",
        "Show the bottom panel (Problems, Output, Terminal) at its last tab, or hide it (Ctrl+J)",
        |cx| cx.editor.panel_toggle(),
    );
    r.register(
        "problems",
        "The bottom panel's Problems: every error and warning in the open files (Ctrl+Shift+M)",
        |cx| cx.editor.panel_show(PanelTab::Problems),
    );
    r.register(
        "locations",
        "The bottom panel's Locations: the places the coding agent listed, to step through",
        |cx| {
            if cx.editor.panel().locations.is_none() {
                return Err("No locations: the agent lists them (show_locations)".into());
            }
            cx.editor.panel_show(PanelTab::Locations)
        },
    );
    r.register(
        "output",
        "The bottom panel's Output: the messages shown, and what language servers logged (Ctrl+Shift+U)",
        |cx| cx.editor.panel_show(PanelTab::Output),
    );
    for (name, delta, what) in [
        (
            "panel_down",
            1,
            "the next problem (or the Output down a line)",
        ),
        (
            "panel_up",
            -1,
            "the previous problem (or the Output up a line)",
        ),
        (
            "panel_page_down",
            10,
            "ten problems on (or the Output down)",
        ),
        ("panel_page_up", -10, "ten problems back (or the Output up)"),
    ] {
        r.register(name, format!("In the panel: {what}"), move |cx| {
            cx.editor.panel_move(delta);
            Ok(())
        });
    }
    r.register(
        "panel_first",
        "In the panel: the first problem (the Output's start)",
        |cx| {
            cx.editor.panel_jump(false);
            Ok(())
        },
    );
    r.register(
        "panel_last",
        "In the panel: the last problem (the Output's end)",
        |cx| {
            cx.editor.panel_jump(true);
            Ok(())
        },
    );
    r.register(
        "panel_open",
        "In the panel's Problems: go to the selected problem",
        |cx| cx.editor.panel_open_selected(),
    );
    r.register("panel_next_tab", "In the panel: the next tab", |cx| {
        cx.editor.panel_cycle(1)
    });
    r.register("panel_prev_tab", "In the panel: the previous tab", |cx| {
        cx.editor.panel_cycle(-1)
    });
    r.register(
        "panel_leave",
        "In the panel: back to the text, leaving it open",
        |cx| {
            cx.editor.panel_leave();
            Ok(())
        },
    );
}
