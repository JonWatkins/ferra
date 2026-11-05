//! Search commands. Typing at the prompt and finishing or cancelling it go
//! through the shared command-line commands (`command_line.rs`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register("search_forward", "Search forward for a pattern (/)", |cx| {
        cx.editor.open_search(true);
        Ok(())
    });
    r.register(
        "search_backward",
        "Search backward for a pattern (?)",
        |cx| {
            cx.editor.open_search(false);
            Ok(())
        },
    );
    r.register(
        "search_next",
        "Jump to the next match of the last search (n)",
        |cx| {
            let n = cx.count();
            cx.editor.search_next(n, false)
        },
    );
    r.register(
        "search_prev",
        "Jump to the previous match of the last search (N)",
        |cx| {
            let n = cx.count();
            cx.editor.search_next(n, true)
        },
    );
    r.register(
        "search_word_forward",
        "Search forward for the word under the cursor (*)",
        |cx| {
            let n = cx.count();
            cx.editor.search_word(n, true)
        },
    );
    r.register(
        "search_word_backward",
        "Search backward for the word under the cursor (#)",
        |cx| {
            let n = cx.count();
            cx.editor.search_word(n, false)
        },
    );
    r.register(
        "clear_search_highlight",
        "Hide search highlights until the next search (:noh)",
        |cx| {
            cx.editor.clear_search_highlight();
            Ok(())
        },
    );
    r.alias("noh", "clear_search_highlight");
    r.alias("nohlsearch", "clear_search_highlight");
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run};
    use crate::core::Position;
    use crate::editor::{Editor, Mode, Prompt};

    fn type_text(e: &mut Editor, text: &str) {
        text.chars().for_each(|c| e.insert_text_char(c));
    }

    #[test]
    fn slash_search_through_the_command_line() {
        let mut e = editor("one two two", Position::new(0, 0), Mode::Normal);
        run(&mut e, "search_forward", None);
        assert_eq!(e.command_line.prompt(), Prompt::SearchForward);
        type_text(&mut e, "two");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.document.cursor(), Position::new(0, 4));

        run(&mut e, "search_next", None);
        assert_eq!(e.document.cursor(), Position::new(0, 8));
        run(&mut e, "search_prev", None);
        assert_eq!(e.document.cursor(), Position::new(0, 4));
    }

    #[test]
    fn cancel_and_backspace_restore_the_cursor() {
        let mut e = editor("abc xyz", Position::new(0, 0), Mode::Normal);
        run(&mut e, "search_forward", None);
        type_text(&mut e, "xy");
        assert_eq!(e.document.cursor(), Position::new(0, 4));
        run(&mut e, "command_line_cancel", None);
        assert_eq!(e.document.cursor(), Position::new(0, 0));

        run(&mut e, "search_forward", None);
        type_text(&mut e, "x");
        run(&mut e, "command_line_backspace", None);
        assert_eq!(
            e.document.cursor(),
            Position::new(0, 0),
            "empty pattern: back at origin"
        );
        assert_eq!(e.mode(), Mode::Command);
        run(&mut e, "command_line_backspace", None);
        assert_eq!(e.mode(), Mode::Normal);
    }

    #[test]
    fn colon_prompt_is_unaffected() {
        let mut e = editor("a\nb", Position::new(0, 0), Mode::Normal);
        run(&mut e, "command_mode", None);
        assert_eq!(e.command_line.prompt(), Prompt::Command);
        type_text(&mut e, "2");
        run(&mut e, "command_line_execute", None);
        assert_eq!(e.document.cursor(), Position::new(1, 0));
    }

    #[test]
    fn noh_alias() {
        let mut e = editor("a a", Position::new(0, 0), Mode::Normal);
        run(&mut e, "search_word_forward", None);
        assert!(e.search_highlight().is_some());
        run(&mut e, "command_mode", None);
        type_text(&mut e, "noh");
        run(&mut e, "command_line_execute", None);
        assert!(e.search_highlight().is_none());
    }
}
