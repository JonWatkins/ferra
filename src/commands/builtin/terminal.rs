//! The terminal panel's commands (`editor/terminal_panel.rs`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "terminal_toggle",
        "Open the terminal panel at the bottom and go to it (starting a shell); from it, hide it",
        |cx| cx.editor.toggle_terminal(),
    );
    r.alias("terminal", "terminal_toggle");
    r.alias("term", "terminal_toggle");
    r.register(
        "terminal_focus",
        "Go to the terminal panel, opening it (and starting a shell) if need be",
        |cx| cx.editor.focus_terminal(),
    );
    r.register(
        "terminal_leave",
        "From the terminal panel back to the editor, leaving it open",
        |cx| {
            cx.editor.leave_terminal();
            Ok(())
        },
    );
    r.register(
        "terminal_new",
        "Start another shell in the terminal panel and go to it",
        |cx| cx.editor.new_terminal(),
    );
    r.alias("terminal-new", "terminal_new");
    r.register(
        "terminal_next",
        "Show the terminal panel's next shell",
        |cx| cx.editor.cycle_terminal(cx.count() as isize),
    );
    r.alias("terminal-next", "terminal_next");
    r.register(
        "terminal_prev",
        "Show the terminal panel's previous shell",
        |cx| cx.editor.cycle_terminal(-(cx.count() as isize)),
    );
    r.alias("terminal-prev", "terminal_prev");
    r.register(
        "terminal_close",
        "End the terminal panel's shell shown (the panel closes with the last)",
        |cx| {
            cx.editor.close_terminal();
            Ok(())
        },
    );
    r.alias("terminal-close", "terminal_close");
    r.register(
        "terminal_scroll_up",
        "Scroll the terminal panel back a page",
        |cx| {
            cx.editor.terminal_scroll(cx.count() as isize);
            Ok(())
        },
    );
    r.register(
        "terminal_scroll_down",
        "Scroll the terminal panel forward a page",
        |cx| {
            cx.editor.terminal_scroll(-(cx.count() as isize));
            Ok(())
        },
    );
}
