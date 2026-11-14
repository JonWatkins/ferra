//! The keybinding editor (`editor/keybindings.rs`).

use crate::commands::CommandRegistry;
use crate::editor::keybindings::Recording;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "keybindings",
        "Open the keybinding editor (:keybindings save shows the bindings matching save)",
        |cx| {
            let query = cx.args.join(" ");
            cx.editor.keybindings_request(&query);
            Ok(())
        },
    );
    r.register("keybindings_close", "Close the keybinding editor", |cx| {
        cx.editor.keybindings_close();
        Ok(())
    });
    for (name, delta, what) in [
        ("keybindings_down", 1, "the next binding"),
        ("keybindings_up", -1, "the previous binding"),
        ("keybindings_page_down", 20, "a page of bindings down"),
        ("keybindings_page_up", -20, "a page of bindings up"),
    ] {
        r.register(
            name,
            format!("In the keybinding editor: {what}"),
            move |cx| {
                cx.editor.keybindings_move(delta);
                Ok(())
            },
        );
    }
    r.register(
        "keybindings_first",
        "In the keybinding editor: the first binding",
        |cx| {
            cx.editor.keybindings_jump(false);
            Ok(())
        },
    );
    r.register(
        "keybindings_last",
        "In the keybinding editor: the last binding",
        |cx| {
            cx.editor.keybindings_jump(true);
            Ok(())
        },
    );
    r.register(
        "keybindings_search",
        "In the keybinding editor: search by command or description",
        |cx| {
            cx.editor.keybindings_search();
            Ok(())
        },
    );
    r.register(
        "keybindings_search_keys",
        "In the keybinding editor: search by pressing the keys",
        |cx| cx.editor.keybindings_record(Recording::Search),
    );
    r.register(
        "keybindings_change",
        "In the keybinding editor: press new keys for the binding, in place of its own",
        |cx| cx.editor.keybindings_record(Recording::Change),
    );
    r.register(
        "keybindings_add",
        "In the keybinding editor: press more keys for the command, besides its others",
        |cx| cx.editor.keybindings_record(Recording::Add),
    );
    r.register(
        "keybindings_remove",
        "In the keybinding editor: take the binding away",
        |cx| cx.editor.keybindings_remove(),
    );
    r.register(
        "keybindings_reset",
        "In the keybinding editor: put back the keymap's binding",
        |cx| cx.editor.keybindings_reset(),
    );
    r.register(
        "keybindings_edit_toml",
        "Close the keybinding editor and open config.toml to edit by hand",
        |cx| {
            cx.editor.keybindings_close();
            cx.editor.open_config()
        },
    );
}
