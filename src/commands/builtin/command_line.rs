use crate::commands::ex::{RangeContext, parse_range};
use crate::commands::{ArgCompletion, CommandRegistry, CommandResult, Context, Invocation};
use crate::editor::{CommandLine, CompletionContext, Editor, Mode, SearchDone};
use crate::files;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "command_line_execute",
        "Run the command typed on the command line",
        |cx| {
            if cx.editor.command_line.prompt().is_search() {
                return match cx.editor.finish_search()? {
                    SearchDone::Moved => Ok(()),
                    SearchDone::Operator { operator, target } => {
                        super::search::operate_to(cx, operator, target)
                    }
                };
            }
            let input = cx.editor.command_line.take();
            cx.editor.set_mode(Mode::Normal);
            run_command_line(cx, &input)
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

/// Runs `[range] name[!] [arguments]`. A range on its own (`:42`, `:$`)
/// goes to its last line.
fn run_command_line(cx: &mut Context, input: &str) -> CommandResult {
    let doc = &cx.editor.document;
    let range_context = RangeContext {
        cursor_row: doc.cursor().row,
        line_count: doc.buffer().line_count(),
        visual: cx.editor.last_visual_rows(),
    };
    let (range, rest) = parse_range(input, &range_context)?;
    let rest = rest.trim_start();

    // The name is a run of letters, digits and `_` (so `s/a/b/` needs no
    // space), or failing that the first word (for aliases like `q!`).
    let name_len = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name_len = if name_len == 0 {
        rest.find(char::is_whitespace).unwrap_or(rest.len())
    } else {
        name_len
    };
    let (name, after) = rest.split_at(name_len);
    if name.is_empty() {
        if let Some((_, last)) = range {
            let line = (last + 1).to_string();
            return cx
                .registry
                .execute("goto_line", cx.editor, Invocation::with_args(&[line]));
        }
        return Ok(());
    }

    // A `!` straight after the name forces the command (`:q!`, `:w!`),
    // unless a command is literally registered with the `!` in its name.
    let (name, bang, after) = match after.strip_prefix('!') {
        Some(rest) if !cx.registry.contains(&format!("{name}!")) => (name.to_string(), true, rest),
        Some(rest) => (format!("{name}!"), false, rest),
        None => (name.to_string(), false, after),
    };
    let arg_text = after.trim_start();
    let args: Vec<String> = arg_text.split_whitespace().map(String::from).collect();
    let invocation = Invocation {
        args: &args,
        bang,
        range,
        arg_text,
        ..Invocation::default()
    };
    cx.registry.execute(&name, cx.editor, invocation)
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
    fn ranges_on_their_own_go_to_a_line() {
        for (input, row) in [("$", 2), ("1", 0), ("2,3", 2), ("%", 2)] {
            let mut e = command_editor("a\nb\nc", input);
            run(&mut e, "command_line_execute", None);
            assert_eq!(e.document.cursor().row, row, "{input}");
        }
    }

    #[test]
    fn names_end_at_punctuation() {
        let mut e = command_editor("a", "goto_line3");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.message().unwrap().text, "Unknown command: goto_line3");
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
