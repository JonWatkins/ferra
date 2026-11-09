//! Lua plugins.
//!
//! Plugins are Lua files loaded at startup: `init.lua` in the config
//! directory, then every `plugins/*.lua` in name order. They use a global
//! `editor` table to register commands, hooks, key bindings and status line
//! elements:
//!
//! ```lua
//! editor.command("hello", "Insert a greeting", function(ctx)
//!   ctx:insert("hello from Lua")
//! end)
//! editor.on("before-save", function(ctx, event) ... end)
//! editor.map("normal", "<Space>h", "hello")
//! editor.status("words", function(ctx) return "42 words" end)
//! ```
//!
//! Registrations made while a file loads are collected, then installed into
//! the command registry and keymap, the same ones built-in commands use. A
//! callback gets a `ctx` object (see `api.rs`) that's only valid during the
//! call, so a plugin can never hold on to the editor.

mod api;
mod popup;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use mlua::{Function, Lua, RegistryKey, Value};

use crate::commands::CommandRegistry;
use crate::editor::Mode;
use crate::editor::popup::PopupLine;
use crate::editor::sidebar_views::ViewSpec;
use crate::events::EventKind;
use crate::input::{Keymap, parse_keys};

/// What plugins asked for while loading, waiting to be installed.
#[derive(Default)]
struct Registrations {
    commands: Vec<(String, String, RegistryKey)>,
    hooks: Vec<(EventKind, String, RegistryKey)>,
    maps: Vec<(String, String, String)>,
    statuses: Vec<(String, RegistryKey)>,
    /// Sidebar views: what they are, and their `lines` functions.
    views: Vec<(ViewSpec, RegistryKey)>,
}

pub struct PluginHost {
    lua: Rc<Lua>,
    pending: Rc<RefCell<Registrations>>,
    /// The file being loaded, used to name hooks in error messages.
    current: Rc<RefCell<String>>,
    /// Status line functions (`editor.status`), by name.
    statuses: RefCell<std::collections::HashMap<String, RegistryKey>>,
    /// Sidebar views' `lines` functions, by name.
    views: RefCell<std::collections::HashMap<String, RegistryKey>>,
    /// Views installed since the app last took them (for the editor).
    new_views: RefCell<Vec<ViewSpec>>,
}

impl PluginHost {
    pub fn new() -> Result<Self, String> {
        let host = Self {
            lua: Rc::new(Lua::new()),
            pending: Rc::default(),
            current: Rc::default(),
            statuses: RefCell::default(),
            views: RefCell::default(),
            new_views: RefCell::default(),
        };
        host.install_globals().map_err(|e| api::describe(&e))?;
        Ok(host)
    }

    /// Runs a plugin's source. Its registrations are kept for `install`.
    pub fn load(&self, name: &str, source: &str) -> Result<(), String> {
        *self.current.borrow_mut() = name.to_string();
        self.lua
            .load(source)
            .set_name(format!("@{name}"))
            .exec()
            .map_err(|e| format!("{name}: {}", api::describe(&e)))
    }

    /// Loads `init.lua` and then `plugins/*.lua` (in name order) from
    /// `config_dir`. Returns the errors, one per failing file; the other
    /// files still load.
    pub fn load_dir(&self, config_dir: &Path) -> Vec<String> {
        let mut files = Vec::new();
        let init = config_dir.join("init.lua");
        if init.is_file() {
            files.push(init);
        }
        if let Ok(entries) = std::fs::read_dir(config_dir.join("plugins")) {
            let mut plugins: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "lua"))
                .collect();
            plugins.sort();
            files.extend(plugins);
        }

        let mut errors = Vec::new();
        for path in files {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let result = std::fs::read_to_string(&path)
                .map_err(|e| format!("{name}: {e}"))
                .and_then(|source| self.load(&name, &source));
            if let Err(error) = result {
                errors.push(error);
            }
        }
        errors
    }

    /// Installs everything registered since the last call into `registry`
    /// and `keymap`. Returns problems, such as a binding to an unknown mode.
    pub fn install(&self, registry: &mut CommandRegistry, keymap: &mut Keymap) -> Vec<String> {
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        let mut errors = Vec::new();

        for (name, description, key) in pending.commands {
            let lua = self.lua.clone();
            let key = Rc::new(key);
            registry.register(name, description, move |cx| api::call(&lua, &key, cx, None));
        }
        for (kind, source, key) in pending.hooks {
            let lua = self.lua.clone();
            let key = Rc::new(key);
            registry.on(kind, source, move |cx, event| {
                api::call(&lua, &key, cx, Some(event))
            });
        }
        self.statuses.borrow_mut().extend(pending.statuses);
        for (spec, key) in pending.views {
            let mut ok = true;
            for (keys, command) in &spec.keys {
                if let Err(e) = parse_keys(keys) {
                    errors.push(format!(
                        "editor.view {}: invalid keys '{keys}': {e}",
                        spec.name
                    ));
                    ok = false;
                }
                if !registry.contains(command) {
                    errors.push(format!(
                        "editor.view {}: unknown command '{command}'",
                        spec.name
                    ));
                    ok = false;
                }
            }
            if ok {
                self.views.borrow_mut().insert(spec.name.clone(), key);
                self.new_views.borrow_mut().push(spec);
            }
        }
        for (mode, keys, command) in pending.maps {
            let Some(mode_value) = Mode::ALL.iter().find(|m| m.id() == mode) else {
                errors.push(format!("editor.map: unknown mode '{mode}'"));
                continue;
            };
            match parse_keys(&keys) {
                Ok(parsed) => keymap.bind(*mode_value, &parsed, command.clone()),
                Err(e) => errors.push(format!("editor.map: invalid keys '{keys}': {e}")),
            }
            if !registry.contains(&command) {
                errors.push(format!("editor.map: unknown command '{command}'"));
            }
        }
        errors
    }

    /// Whether a status line function called `name` is registered.
    pub fn has_status(&self, name: &str) -> bool {
        self.statuses.borrow().contains_key(name)
    }

    /// Calls status line function `name`: its text (`nil` is nothing).
    pub fn status(&self, name: &str, cx: &mut crate::commands::Context) -> Result<String, String> {
        let statuses = self.statuses.borrow();
        let key = statuses
            .get(name)
            .ok_or_else(|| format!("No status function {name}"))?;
        let text: Option<String> = api::call_returning(&self.lua, key, cx, None)?;
        Ok(text.unwrap_or_default())
    }

    /// Views installed since last asked, for the editor's sidebar.
    pub fn take_new_views(&self) -> Vec<ViewSpec> {
        std::mem::take(&mut *self.new_views.borrow_mut())
    }

    /// Calls sidebar view `name`'s `lines` function.
    pub fn view_lines(
        &self,
        name: &str,
        cx: &mut crate::commands::Context,
    ) -> Result<Vec<PopupLine>, String> {
        let views = self.views.borrow();
        let key = views
            .get(name)
            .ok_or_else(|| format!("No sidebar view {name}"))?;
        let value: Value = api::call_returning(&self.lua, key, cx, None)?;
        popup::lines(&value).map_err(|e| e.replace("popup.lines", "lines"))
    }

    /// The global `editor` table plugins register through.
    fn install_globals(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        let editor = lua.create_table()?;
        editor.set("version", env!("CARGO_PKG_VERSION"))?;

        let pending = self.pending.clone();
        editor.set(
            "command",
            lua.create_function(
                move |lua, (name, second, third): (String, Value, Option<Function>)| {
                    // editor.command(name, fn) or editor.command(name, description, fn)
                    let (description, function) = match (second, third) {
                        (Value::Function(f), None) => (String::new(), f),
                        (Value::String(d), Some(f)) => (d.to_str()?.to_string(), f),
                        _ => {
                            return Err(mlua::Error::runtime(
                                "usage: editor.command(name, [description,] function)",
                            ));
                        }
                    };
                    let key = lua.create_registry_value(function)?;
                    pending.borrow_mut().commands.push((name, description, key));
                    Ok(())
                },
            )?,
        )?;

        let pending = self.pending.clone();
        let current = self.current.clone();
        editor.set(
            "on",
            lua.create_function(move |lua, (event, function): (String, Function)| {
                let kind: EventKind = toml::Value::String(event.clone())
                    .try_into()
                    .map_err(|_| mlua::Error::runtime(format!("unknown event '{event}'")))?;
                let key = lua.create_registry_value(function)?;
                let source = current.borrow().clone();
                pending.borrow_mut().hooks.push((kind, source, key));
                Ok(())
            })?,
        )?;

        let pending = self.pending.clone();
        editor.set(
            "map",
            lua.create_function(move |_, (mode, keys, command): (String, String, String)| {
                pending.borrow_mut().maps.push((mode, keys, command));
                Ok(())
            })?,
        )?;

        let pending = self.pending.clone();
        editor.set(
            "status",
            lua.create_function(move |lua, (name, function): (String, Function)| {
                let key = lua.create_registry_value(function)?;
                pending.borrow_mut().statuses.push((name, key));
                Ok(())
            })?,
        )?;

        // editor.view(name, { title, lines = fn, keys, refresh })
        let pending = self.pending.clone();
        editor.set(
            "view",
            lua.create_function(move |lua, (name, spec): (String, mlua::Table)| {
                let usage =
                    |what: &str| mlua::Error::runtime(format!("editor.view {name}: {what}"));
                let lines: Function = spec
                    .get::<Option<Function>>("lines")?
                    .ok_or_else(|| usage("needs a lines function"))?;
                let title = spec
                    .get::<Option<String>>("title")?
                    .unwrap_or_else(|| name.clone());
                let mut keys = Vec::new();
                if let Some(map) = spec.get::<Option<mlua::Table>>("keys")? {
                    for pair in map.pairs::<String, String>() {
                        keys.push(pair.map_err(|_| usage("keys are key = \"command\" pairs"))?);
                    }
                }
                keys.sort();
                let mut refresh = Vec::new();
                if let Some(events) = spec.get::<Option<Vec<String>>>("refresh")? {
                    for event in events {
                        let kind: EventKind = toml::Value::String(event.clone())
                            .try_into()
                            .map_err(|_| usage(&format!("unknown event '{event}'")))?;
                        refresh.push(kind);
                    }
                }
                if name == "files" {
                    return Err(usage("\"files\" is the file tree's name"));
                }
                let key = lua.create_registry_value(lines)?;
                let spec = ViewSpec {
                    name: name.clone(),
                    title,
                    keys,
                    refresh,
                };
                pending.borrow_mut().views.push((spec, key));
                Ok(())
            })?,
        )?;

        lua.globals().set("editor", editor)
    }
}
