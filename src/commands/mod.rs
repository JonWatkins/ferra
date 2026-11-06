//! The named command registry, and the hooks that run on editor events.
//!
//! Every user-facing action is a named command (`move_down`, `delete_line`,
//! `quit`, ...). Keymaps bind keys to command *names*, and the `:` command
//! line runs commands by name or alias. This indirection is the editor's main
//! extension point: config can rebind anything, and plugins will register
//! their own commands through the same [`CommandRegistry::register`] API.

mod builtin;
pub mod ex;
mod hooks;

use std::collections::HashMap;

use crate::editor::Editor;

/// Commands report failures as a user-facing message.
pub type CommandResult = Result<(), String>;

type Handler = Box<dyn Fn(&mut Context) -> CommandResult>;

/// How a command was invoked: the parts that come from the user's input
/// rather than editor state.
#[derive(Debug, Default, Clone, Copy)]
pub struct Invocation<'a> {
    /// Numeric prefix typed before the key (e.g. the `3` in `3j`).
    pub count: Option<usize>,
    /// Register selected with `"x` before the key (e.g. the `a` in `"ayy`).
    pub register: Option<char>,
    /// Arguments passed on the command line (e.g. `:goto_line 10`).
    pub args: &'a [String],
    /// A trailing `!` on the command line (`:q!`, `:w!`): override safety
    /// checks such as unsaved changes.
    pub bang: bool,
    /// The key typed after a command that takes one (the `x` in `fx`).
    pub char_arg: Option<char>,
    /// Lines given before a `:` command (`:5,10d`), 0-based and inclusive.
    pub range: Option<(usize, usize)>,
    /// Everything after the command name, untouched (`:s/a b/c/` keeps its
    /// spaces; `args` splits on whitespace).
    pub arg_text: &'a str,
}

impl<'a> Invocation<'a> {
    pub fn with_count(count: usize) -> Self {
        Self {
            count: Some(count),
            ..Self::default()
        }
    }

    pub fn with_args(args: &'a [String]) -> Self {
        Self {
            args,
            ..Self::default()
        }
    }
}

/// Everything a command can see and change while running.
pub struct Context<'a> {
    pub editor: &'a mut Editor,
    /// The registry, so commands can run other commands (e.g. `:` execution).
    pub registry: &'a CommandRegistry,
    pub count: Option<usize>,
    pub register: Option<char>,
    pub args: &'a [String],
    pub bang: bool,
    pub char_arg: Option<char>,
    pub range: Option<(usize, usize)>,
    pub arg_text: &'a str,
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
    /// The command needs one more key as an argument (`f{char}`, `r{char}`).
    /// The input layer waits for it and passes it as `Context::char_arg`.
    pub takes_char: bool,
    /// What `<Tab>` completes for this command's arguments on the `:` line.
    pub arg_completion: ArgCompletion,
    /// How `|` in its arguments is treated on the `:` line.
    pub bar: ex::Bar,
    handler: Handler,
}

/// What a command's arguments are, for `<Tab>` completion.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ArgCompletion {
    #[default]
    None,
    /// File paths (`:e`, `:w`, `:vs`).
    Path,
    /// Open buffer names (`:b`).
    Buffer,
}

impl Command {
    /// Marks the command as taking a character argument.
    pub fn takes_char(&mut self) -> &mut Self {
        self.takes_char = true;
        self
    }

    /// Sets how `|` in the command's arguments is treated (separating
    /// commands, by default).
    pub fn bar(&mut self, bar: ex::Bar) -> &mut Self {
        self.bar = bar;
        self
    }

    /// Sets what `<Tab>` completes for the command's arguments.
    pub fn complete_args(&mut self, completion: ArgCompletion) -> &mut Self {
        self.arg_completion = completion;
        self
    }
}

#[derive(Default)]
pub struct CommandRegistry {
    commands: HashMap<String, Command>,
    aliases: HashMap<String, String>,
    hooks: hooks::Hooks,
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
    ) -> &mut Command {
        let name = name.into();
        let command = Command {
            name: name.clone(),
            description: description.into(),
            takes_char: false,
            arg_completion: ArgCompletion::None,
            bar: ex::Bar::default(),
            handler: Box::new(handler),
        };
        self.commands.insert(name.clone(), command);
        self.commands.get_mut(&name).expect("just inserted")
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

    /// Whether the named command needs a character argument.
    pub fn takes_char(&self, name: &str) -> bool {
        self.get(name).is_some_and(|c| c.takes_char)
    }

    /// Every name a command can be run by: names and aliases, sorted.
    pub fn all_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .commands
            .keys()
            .chain(self.aliases.keys())
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
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
        invocation: Invocation,
    ) -> CommandResult {
        let command = self
            .get(name)
            .ok_or_else(|| format!("Unknown command: {name}"))?;
        let mut cx = Context {
            editor,
            registry: self,
            count: invocation.count,
            register: invocation.register,
            args: invocation.args,
            bang: invocation.bang,
            char_arg: invocation.char_arg,
            range: invocation.range,
            arg_text: invocation.arg_text,
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
        registry
            .execute("bye", &mut editor, Invocation::default())
            .unwrap();
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
    fn commands_can_take_a_char_argument() {
        let mut registry = CommandRegistry::new();
        registry.register("plain", "", |_| Ok(()));
        registry.register("find", "", |_| Ok(())).takes_char();
        assert!(registry.takes_char("find"));
        assert!(!registry.takes_char("plain"));
        assert!(!registry.takes_char("missing"));
    }

    #[test]
    fn all_names_include_aliases() {
        let mut registry = CommandRegistry::new();
        registry.register("write", "", |_| Ok(()));
        registry.alias("w", "write");
        assert_eq!(registry.all_names(), ["w", "write"]);
    }

    #[test]
    fn unknown_commands_return_error() {
        let registry = CommandRegistry::new();
        let err = registry
            .execute("nope", &mut Editor::default(), Invocation::default())
            .unwrap_err();
        assert_eq!(err, "Unknown command: nope");
    }

    #[test]
    fn invocation_is_passed_through() {
        let mut registry = CommandRegistry::new();
        registry.register("check", "", |cx| {
            assert_eq!(cx.count(), 3);
            assert_eq!(cx.register, Some('a'));
            assert_eq!(cx.args, ["x"]);
            assert_eq!(cx.char_arg, Some('z'));
            Ok(())
        });
        let args = ["x".to_string()];
        let invocation = Invocation {
            count: Some(3),
            register: Some('a'),
            args: &args,
            bang: false,
            char_arg: Some('z'),
            range: None,
            arg_text: "x",
        };
        registry
            .execute("check", &mut Editor::default(), invocation)
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
