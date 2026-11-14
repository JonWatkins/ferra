//! The settings editor (`editor/settings.rs`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "settings",
        "Open the settings editor (:settings tab shows the settings matching tab)",
        |cx| {
            let query = cx.args.join(" ");
            cx.editor.settings_open(Some(&query));
            Ok(())
        },
    );
    r.register("settings_close", "Close the settings editor", |cx| {
        cx.editor.settings_close();
        Ok(())
    });
    r.register(
        "settings_down",
        "In the settings editor: the next setting (or category)",
        |cx| {
            cx.editor.settings_move(1);
            Ok(())
        },
    );
    r.register(
        "settings_up",
        "In the settings editor: the previous setting (or category)",
        |cx| {
            cx.editor.settings_move(-1);
            Ok(())
        },
    );
    r.register(
        "settings_first",
        "In the settings editor: the first setting",
        |cx| {
            cx.editor.settings_jump(false);
            Ok(())
        },
    );
    r.register(
        "settings_last",
        "In the settings editor: the last setting",
        |cx| {
            cx.editor.settings_jump(true);
            Ok(())
        },
    );
    r.register(
        "settings_pane",
        "In the settings editor: between the categories and the settings",
        |cx| {
            cx.editor.settings_switch_pane();
            Ok(())
        },
    );
    r.register(
        "settings_change",
        "In the settings editor: flip a switch, take the next choice, or type a new value",
        |cx| cx.editor.settings_activate(),
    );
    r.register(
        "settings_next",
        "In the settings editor: the next choice, one more, or a switch flipped",
        |cx| cx.editor.settings_step(1),
    );
    r.register(
        "settings_prev",
        "In the settings editor: the previous choice, one less, or a switch flipped",
        |cx| cx.editor.settings_step(-1),
    );
    r.register(
        "settings_reset",
        "In the settings editor: put the setting back to its default",
        |cx| cx.editor.settings_reset(),
    );
    r.register(
        "settings_search",
        "In the settings editor: search the settings",
        |cx| {
            cx.editor.settings_search();
            Ok(())
        },
    );
    r.register(
        "settings_edit_toml",
        "Close the settings editor and open config.toml to edit by hand",
        |cx| cx.editor.settings_edit_toml(),
    );
}
