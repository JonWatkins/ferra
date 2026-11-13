//! Plugin packages (`editor/plugin_packages.rs`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "plugin_install",
        "Install a plugin from GitHub (owner/repo[@tag]) or any git URL, after asking",
        |cx| {
            let spec = cx.args.join(" ");
            cx.editor.plugin_ask_install(&spec)
        },
    );
    r.register(
        "plugin_install_confirmed",
        "y when asked to install a plugin: install it",
        |cx| cx.editor.plugin_install_confirmed(),
    );
    r.register(
        "plugin_remove",
        "Remove an installed plugin (its commands stay until ferra restarts)",
        |cx| {
            let name = cx.args.join(" ");
            cx.editor.plugin_remove(name.trim())
        },
    );
    r.register(
        "plugin_list",
        "List the installed plugins, their versions and commits",
        |cx| cx.editor.plugin_list(),
    );
}
