//! Plugin packages (`editor/plugin_packages.rs`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "plugin_install",
        "Install a plugin from GitHub (owner/repo[@tag]) or any git URL, after asking; \
         with nothing, install what plugins.toml lists but isn't installed",
        |cx| {
            let spec = cx.args.join(" ");
            cx.editor.plugin_ask_install(&spec)
        },
    );
    r.register(
        "plugin_browse",
        "Pick a plugin to install from the plugin directory (the plugin-index option)",
        |cx| cx.editor.plugin_browse(),
    );
    r.register(
        "plugin_update",
        "Look for new commits of the plugins (or the ones named), show them and ask to update",
        |cx| {
            let names = cx.args.to_vec();
            cx.editor.plugin_check_updates(&names)
        },
    );
    r.register(
        "plugin_confirmed",
        "y when asked to install or update plugins: go ahead",
        |cx| cx.editor.plugin_confirmed(),
    );
    r.register(
        "plugin_decline",
        "d when a language pack is offered: don't offer it again",
        |cx| cx.editor.plugin_decline(),
    );
    r.register(
        "plugin_reload",
        "Load every plugin again, in a fresh Lua state (after changing one)",
        |cx| {
            cx.editor.request_plugin_reload(true);
            Ok(())
        },
    );
    r.register("plugin_remove", "Remove an installed plugin", |cx| {
        let name = cx.args.join(" ");
        cx.editor.plugin_remove(name.trim())
    });
    r.register(
        "plugin_list",
        "List the installed plugins, their versions and commits",
        |cx| cx.editor.plugin_list(),
    );
}
