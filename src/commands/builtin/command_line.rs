use crate::commands::{CommandRegistry, Invocation};
use crate::editor::Mode;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "command_line_execute",
        "Run the command typed on the command line",
        |cx| {
            if cx.editor.command_line.prompt().is_search() {
                return cx.editor.finish_search();
            }
            let input = cx.editor.command_line.take();
            cx.editor.set_mode(Mode::Normal);

            let mut parts = input.split_whitespace();
            let Some(name) = parts.next() else {
                return Ok(());
            };

            // `:42` jumps to line 42, as in Vim.
            if name.chars().all(|c| c.is_ascii_digit()) {
                let args = [name.to_string()];
                return cx
                    .registry
                    .execute("goto_line", cx.editor, Invocation::with_args(&args));
            }

            // A trailing `!` (`:q!`, `:w!`) forces the command, unless a
            // command is literally registered with the `!` in its name.
            let (name, bang) = match name.strip_suffix('!') {
                Some(base) if !cx.registry.contains(name) => (base, true),
                _ => (name, false),
            };
            let args: Vec<String> = parts.map(String::from).collect();
            let invocation = Invocation {
                args: &args,
                bang,
                ..Invocation::default()
            };
            cx.registry.execute(name, cx.editor, invocation)
        },
    );
    r.register(
        "command_line_cancel",
        "Leave the command line without running anything",
        |cx| {
            cx.editor.set_mode(Mode::Normal);
            Ok(())
        },
    );
    r.register(
        "command_line_backspace",
        "Delete the last character, leaving command mode when empty",
        |cx| {
            if cx.editor.command_line.pop().is_none() {
                cx.editor.set_mode(Mode::Normal);
            } else {
                cx.editor.update_search_preview();
            }
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run};
    use crate::core::Position;
    use crate::editor::{Editor, Mode};

    fn command_editor(text: &str, input: &str) -> Editor {
        let mut e = editor(text, Position::default(), Mode::Command);
        input.chars().for_each(|c| e.insert_text_char(c));
        e
    }

    #[test]
    fn executes_command_by_alias() {
        let mut e = command_editor("", "q");
        run(&mut e, "command_line_execute", None);
        assert!(e.should_quit());
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn trailing_bang_forces_command() {
        let mut e = command_editor("", "q!");
        e.document.insert_str("unsaved");
        run(&mut e, "command_line_execute", None);
        assert!(e.should_quit());
    }

    #[test]
    fn bang_on_unknown_command_reports_base_name() {
        let mut e = command_editor("", "nope!");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.message().unwrap().text, "Unknown command: nope");
    }

    #[test]
    fn executes_command_by_full_name() {
        let mut e = command_editor("a\nb", "goto_file_end");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.document.cursor().row, 1);
    }

    #[test]
    fn number_jumps_to_line() {
        let mut e = command_editor("a\nb\nc", "2");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.document.cursor().row, 1);
    }

    #[test]
    fn passes_arguments() {
        let mut e = command_editor("a\nb\nc", "goto_line 3");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.document.cursor().row, 2);
    }

    #[test]
    fn unknown_command_sets_error() {
        let mut e = command_editor("", "frobnicate");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.message().unwrap().text, "Unknown command: frobnicate");
    }

    #[test]
    fn empty_input_does_nothing() {
        let mut e = command_editor("", "");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert!(e.message().is_none());
    }

    #[test]
    fn backspace_on_empty_line_leaves_command_mode() {
        let mut e = command_editor("", "a");
        run(&mut e, "command_line_backspace", None);
        assert_eq!(e.mode(), Mode::Command);
        run(&mut e, "command_line_backspace", None);
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn cancel_returns_to_normal_mode() {
        let mut e = command_editor("", "q");
        run(&mut e, "command_line_cancel", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert!(!e.should_quit());
    }
}
