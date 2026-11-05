//! The named command registry.
//!
//! Every user-facing action is a named command (`move_down`, `delete_line`,
//! `quit`, ...). Keymaps bind keys to command *names*, and the `:` command
//! line runs commands by name or alias. This indirection is the editor's main
//! extension point: config can rebind anything, and plugins will register
//! their own commands through the same [`CommandRegistry::register`] API.

mod builtin;

use std::collections::HashMap;

use crate::editor::Editor;

/// Commands report failures as a user-facing message.
pub type CommandResult = Result<(), String>;

type Handler = Box<dyn Fn(&mut Context) -> CommandResult>;

/// Everything a command can see and change while running.
pub struct Context<'a> {
    pub editor: &'a mut Editor,
    /// The registry, so commands can run other commands (e.g. `:` execution).
    pub registry: &'a CommandRegistry,
    /// Numeric prefix typed before the key (e.g. the `3` in `3j`).
    pub count: Option<usize>,
    /// Arguments passed on the command line (e.g. `:goto_line 10`).
    pub args: &'a [String],
}

impl Context<'_> {
    /// The count, defaulting to 1.
    pub fn count(&self) -> usize {
        self.count.unwrap_or(1).max(1)
    }
}

pub struct Command {
    pub name: String,
    pub description: String,
    handler: Handler,
}

#[derive(Default)]
pub struct CommandRegistry {
    commands: HashMap<String, Command>,
    aliases: HashMap<String, String>,
}

impl CommandRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry containing all built-in commands.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        builtin::register_all(&mut registry);
        registry
    }

    /// Registers (or replaces) a command.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: impl Fn(&mut Context) -> CommandResult + 'static,
    ) {
        let name = name.into();
        let command = Command {
            name: name.clone(),
            description: description.into(),
            handler: Box::new(handler),
        };
        self.commands.insert(name, command);
    }

    /// Adds an alternative name for a command (e.g. `q` for `quit`).
    pub fn alias(&mut self, alias: impl Into<String>, target: impl Into<String>) {
        self.aliases.insert(alias.into(), target.into());
    }

    /// Looks up a command by name or alias.
    pub fn get(&self, name: &str) -> Option<&Command> {
        let name = self.aliases.get(name).map_or(name, String::as_str);
        self.commands.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// All registered commands, sorted by name.
    pub fn commands(&self) -> Vec<&Command> {
        let mut commands: Vec<_> = self.commands.values().collect();
        commands.sort_by(|a, b| a.name.cmp(&b.name));
        commands
    }

    pub fn execute(
        &self,
        name: &str,
        editor: &mut Editor,
        count: Option<usize>,
        args: &[String],
    ) -> CommandResult {
        let command = self
            .get(name)
            .ok_or_else(|| format!("Unknown command: {name}"))?;
        let mut cx = Context {
            editor,
            registry: self,
            count,
            args,
        };
        (command.handler)(&mut cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_commands_can_be_executed() {
        let mut registry = CommandRegistry::new();
        registry.register("bye", "Quit", |cx| {
            cx.editor.quit();
            Ok(())
        });

        let mut editor = Editor::default();
        registry.execute("bye", &mut editor, None, &[]).unwrap();
        assert!(editor.should_quit());
    }

    #[test]
    fn aliases_resolve_to_commands() {
        let mut registry = CommandRegistry::new();
        registry.register("quit", "Quit", |_| Ok(()));
        registry.alias("q", "quit");
        assert_eq!(registry.get("q").unwrap().name, "quit");
    }

    #[test]
    fn unknown_commands_return_error() {
        let registry = CommandRegistry::new();
        let err = registry
            .execute("nope", &mut Editor::default(), None, &[])
            .unwrap_err();
        assert_eq!(err, "Unknown command: nope");
    }

    #[test]
    fn count_and_args_are_passed_through() {
        let mut registry = CommandRegistry::new();
        registry.register("check", "", |cx| {
            assert_eq!(cx.count(), 3);
            assert_eq!(cx.args, ["a"]);
            Ok(())
        });
        registry
            .execute("check", &mut Editor::default(), Some(3), &["a".into()])
            .unwrap();
    }

    #[test]
    fn builtins_are_sorted_and_described() {
        let registry = CommandRegistry::with_builtins();
        let commands = registry.commands();
        assert!(commands.windows(2).all(|w| w[0].name < w[1].name));
        assert!(commands.iter().all(|c| !c.description.is_empty()));
    }
}
