use crate::commands::{ArgCompletion, CommandRegistry, CommandResult, Context, Invocation};
use crate::editor::{CommandLine, CompletionContext, Editor, Mode};
use crate::files;

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
        "Delete the character before the cursor, leaving the prompt when it's empty",
        |cx| {
            if cx.editor.command_line.is_empty() {
                cx.editor.set_mode(Mode::Normal);
            } else {
                edit(cx, |line| {
                    line.pop();
                });
            }
            Ok(())
        },
    );

    type Edit = fn(&mut CommandLine);
    let edits: [(&str, &str, Edit); 9] = [
        (
            "command_line_left",
            "Move the prompt cursor left",
            CommandLine::move_left,
        ),
        (
            "command_line_right",
            "Move the prompt cursor right",
            CommandLine::move_right,
        ),
        (
            "command_line_home",
            "Move the prompt cursor to the start",
            CommandLine::move_home,
        ),
        (
            "command_line_end",
            "Move the prompt cursor to the end",
            CommandLine::move_end,
        ),
        (
            "command_line_delete",
            "Delete the character under the prompt cursor",
            CommandLine::delete_forward,
        ),
        (
            "command_line_delete_word",
            "Delete the word before the prompt cursor",
            CommandLine::delete_word_backward,
        ),
        (
            "command_line_delete_to_start",
            "Delete everything before the prompt cursor",
            CommandLine::delete_to_start,
        ),
        (
            "command_line_history_older",
            "Show the previous prompt history entry starting with what's typed",
            |line| {
                line.history_older();
            },
        ),
        (
            "command_line_history_newer",
            "Show the next (newer) prompt history entry",
            |line| {
                line.history_newer();
            },
        ),
    ];
    for (name, description, action) in edits {
        r.register(name, description, move |cx| {
            edit(cx, action);
            Ok(())
        });
    }
    r.register(
        "command_line_complete",
        "Complete the command, file or buffer name (again for the next match)",
        |cx| complete(cx, 1),
    );
    r.register(
        "command_line_complete_prev",
        "Show the previous completion",
        |cx| complete(cx, -1),
    );
}

/// Applies an edit to the prompt, then updates a search's live preview.
fn edit(cx: &mut Context, action: impl FnOnce(&mut CommandLine)) {
    action(&mut cx.editor.command_line);
    cx.editor.update_search_preview();
}

/// `<Tab>`: cycles an in-progress completion, or starts one for the word
/// before the cursor.
fn complete(cx: &mut Context, delta: isize) -> CommandResult {
    let line = &mut cx.editor.command_line;
    if line.prompt().is_search() || line.cycle_completion(delta) {
        return Ok(());
    }
    let context = line.completion_context();
    let candidates = candidates(cx.registry, cx.editor, &context);
    cx.editor
        .command_line
        .start_completion(context.start, candidates);
    Ok(())
}

/// What the word before the cursor could be: a command name, or an
/// argument of the kind its command declares.
fn candidates(
    registry: &CommandRegistry,
    editor: &Editor,
    context: &CompletionContext,
) -> Vec<String> {
    let word = context.word.as_str();
    let Some(command) = &context.command else {
        return registry
            .all_names()
            .into_iter()
            .filter(|name| name.starts_with(word))
            .map(String::from)
            .collect();
    };
    let name = command.trim_end_matches('!');
    match registry.get(name).map(|c| c.arg_completion) {
        Some(ArgCompletion::Path) => files::complete_path(word),
        Some(ArgCompletion::Buffer) => editor
            .buffers()
            .into_iter()
            .map(|b| b.name)
            .filter(|name| name.contains(word))
            .collect(),
        Some(ArgCompletion::None) | None => Vec::new(),
    }
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
    fn backspace_at_the_start_of_a_non_empty_prompt_stays_open() {
        let mut e = command_editor("", "ab");
        run(&mut e, "command_line_home", None);
        run(&mut e, "command_line_backspace", None);
        assert_eq!(e.mode(), Mode::Command);
        assert_eq!(e.command_line.input(), "ab");
    }

    #[test]
    fn completes_command_names_and_cycles() {
        let mut e = command_editor("", "write_a");
        run(&mut e, "command_line_complete", None);
        assert_eq!(e.command_line.input(), "write_all");
        assert!(
            e.command_line.completion().is_some(),
            "write_all and write_all_quit"
        );
        run(&mut e, "command_line_complete", None);
        assert_eq!(e.command_line.input(), "write_all_quit");
        run(&mut e, "command_line_complete_prev", None);
        assert_eq!(e.command_line.input(), "write_all");
    }

    #[test]
    fn completes_buffer_names_for_b() {
        let mut e = command_editor("", "b scr");
        run(&mut e, "command_line_complete", None);
        assert_eq!(e.command_line.input(), "b [scratch]");
    }

    #[test]
    fn no_completion_for_plain_arguments_or_searches() {
        let mut e = command_editor("", "goto_line 1");
        run(&mut e, "command_line_complete", None);
        assert_eq!(e.command_line.input(), "goto_line 1");
    }

    #[test]
    fn history_through_commands() {
        let mut e = command_editor("a\nb", "goto_line 2");
        run(&mut e, "command_line_execute", None);
        e.open_command_line();
        run(&mut e, "command_line_history_older", None);
        assert_eq!(e.command_line.input(), "goto_line 2");
        run(&mut e, "command_line_history_newer", None);
        assert_eq!(e.command_line.input(), "");
    }

    #[test]
    fn cancel_returns_to_normal_mode() {
        let mut e = command_editor("", "q");
        run(&mut e, "command_line_cancel", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert!(!e.should_quit());
    }
}
