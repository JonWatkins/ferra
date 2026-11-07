//! Built-in commands, grouped by area.
//!
//! Each submodule exposes a `register` function. Command names are
//! `snake_case` and are the identifiers used in keybinding config.

mod buffers;
mod command_line;
mod editing;
mod ex_commands;
mod files;
mod history;
mod jumps;
mod macros;
mod marks;
mod modes;
mod motions;
mod operators;
mod options;
mod picker;
mod search;
mod text_objects;
mod visual;
mod windows;

use super::CommandRegistry;

pub(super) fn register_all(registry: &mut CommandRegistry) {
    motions::register(registry);
    text_objects::register(registry);
    operators::register(registry);
    editing::register(registry);
    history::register(registry);
    files::register(registry);
    buffers::register(registry);
    visual::register(registry);
    search::register(registry);
    picker::register(registry);
    windows::register(registry);
    ex_commands::register(registry);
    marks::register(registry);
    macros::register(registry);
    jumps::register(registry);
    modes::register(registry);
    command_line::register(registry);
    options::register(registry);
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::commands::{CommandRegistry, Invocation};
    use crate::core::Position;
    use crate::editor::{Document, Editor, Mode};

    /// Builds an editor with `text`, cursor at `cursor`, in `mode`.
    pub fn editor(text: &str, cursor: Position, mode: Mode) -> Editor {
        let mut editor = Editor::with_document(Document::with_text(text));
        editor.set_mode(mode);
        editor.document.set_cursor(cursor);
        editor
    }

    /// Runs a built-in command with an optional count.
    pub fn run(editor: &mut Editor, name: &str, count: Option<usize>) {
        let invocation = Invocation {
            count,
            ..Invocation::default()
        };
        run_with(editor, name, invocation);
    }

    pub fn run_with_args(editor: &mut Editor, name: &str, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        run_with(editor, name, Invocation::with_args(&args));
    }

    /// Runs a built-in command exactly as the app does: errors become
    /// messages, then cursor rules and undo boundaries are applied.
    pub fn run_with(editor: &mut Editor, name: &str, invocation: Invocation) {
        let registry = CommandRegistry::with_builtins();
        if let Err(e) = registry.execute(name, editor, invocation) {
            editor.set_message(crate::editor::Message::error(e));
        }
        editor.finish_command();
    }
}
