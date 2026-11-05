//! Built-in commands, grouped by area.
//!
//! Each submodule exposes a `register` function. Command names are
//! `snake_case` and are the identifiers used in keybinding config.

mod command_line;
mod editing;
mod modes;
mod movement;

use super::CommandRegistry;

pub(super) fn register_all(registry: &mut CommandRegistry) {
    movement::register(registry);
    editing::register(registry);
    modes::register(registry);
    command_line::register(registry);
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::commands::CommandRegistry;
    use crate::core::Position;
    use crate::editor::{Document, Editor, Mode};

    /// Builds an editor with `text`, cursor at `cursor`, in `mode`.
    pub fn editor(text: &str, cursor: Position, mode: Mode) -> Editor {
        let mut editor = Editor::with_document(Document::with_text(text));
        editor.set_mode(mode);
        editor.document.set_cursor(cursor);
        editor
    }

    /// Runs a built-in command, applying normal-mode cursor rules afterwards
    /// exactly as the app does.
    pub fn run(editor: &mut Editor, name: &str, count: Option<usize>) {
        run_with_args(editor, name, count, &[]);
    }

    pub fn run_with_args(editor: &mut Editor, name: &str, count: Option<usize>, args: &[&str]) {
        let registry = CommandRegistry::with_builtins();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        if let Err(e) = registry.execute(name, editor, count, &args) {
            editor.set_message(crate::editor::Message::error(e));
        }
        editor.enforce_cursor_rules();
    }
}
