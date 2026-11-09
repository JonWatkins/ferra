//! The sidebar's commands: opening it and moving about its file tree.

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "sidebar_focus",
        "Show the file tree in the sidebar and go to it, at the current file; from it, back to the editor (<Space>e)",
        |cx| {
            cx.editor.toggle_sidebar_focus();
            Ok(())
        },
    );
    r.register(
        "sidebar_toggle",
        "Open or close the sidebar's file tree",
        |cx| {
            cx.editor.toggle_sidebar();
            Ok(())
        },
    );
    r.alias("explorer", "sidebar_toggle");
    r.alias("Explore", "sidebar_toggle");
    r.register("sidebar_down", "Down a row in the file tree", |cx| {
        cx.editor.sidebar_move(cx.count() as isize);
        Ok(())
    });
    r.register("sidebar_up", "Up a row in the file tree", |cx| {
        cx.editor.sidebar_move(-(cx.count() as isize));
        Ok(())
    });
    r.register("sidebar_first", "To the file tree's first row", |cx| {
        cx.editor.sidebar_first();
        Ok(())
    });
    r.register("sidebar_last", "To the file tree's last row", |cx| {
        cx.editor.sidebar_last();
        Ok(())
    });
    r.register(
        "sidebar_choose",
        "Open or close the selected folder, or open the file",
        |cx| cx.editor.sidebar_choose(),
    );
    r.register(
        "sidebar_expand",
        "Open the selected folder (or go into it), or open the file",
        |cx| cx.editor.sidebar_expand(),
    );
    r.register(
        "sidebar_collapse",
        "Close the selected folder, or go to the folder it's in",
        |cx| {
            cx.editor.sidebar_collapse();
            Ok(())
        },
    );
    r.register(
        "sidebar_collapse_all",
        "Close every folder in the file tree",
        |cx| {
            cx.editor.sidebar_collapse_all();
            Ok(())
        },
    );
    r.register(
        "sidebar_refresh",
        "List the file tree's files again",
        |cx| {
            cx.editor.sidebar_refresh();
            Ok(())
        },
    );
}
