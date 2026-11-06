//! Search commands. Typing at the prompt and finishing or cancelling it go
//! through the shared command-line commands (`command_line.rs`).
//!
//! After an operator, searches are motions: `d/foo<CR>` deletes up to the
//! match, `yn` yanks up to the next one.

use super::motions::{self, MotionArgs};
use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::core::Position;
use crate::editor::PendingOperator;

pub fn register(r: &mut CommandRegistry) {
    r.register("search_forward", "Search forward for a pattern (/)", |cx| {
        open_search(cx, true);
        Ok(())
    });
    r.register(
        "search_backward",
        "Search backward for a pattern (?)",
        |cx| {
            open_search(cx, false);
            Ok(())
        },
    );
    r.register(
        "search_next",
        "Jump to the next match of the last search (n)",
        |cx| search_next(cx, false),
    );
    r.register(
        "search_prev",
        "Jump to the previous match of the last search (N)",
        |cx| search_next(cx, true),
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

/// `/` or `?`. After an operator, a count typed before the `/` (`d2/x`)
/// joins the operator's, choosing which match.
fn open_search(cx: &mut Context, forward: bool) {
    if let Some(operator) = cx.editor.pending_operator()
        && cx.count.is_some()
    {
        let count = motions::multiply_counts(operator.count, cx.count);
        cx.editor
            .begin_operator(operator.operator, count, operator.register);
    }
    cx.editor.open_search(forward);
}

/// `n`/`N`: moves to the next match, or with an operator waiting (`dn`),
/// applies it up to there.
fn search_next(cx: &mut Context, reverse: bool) -> CommandResult {
    let Some(operator) = cx.editor.pending_operator() else {
        let n = cx.count();
        return cx.editor.search_next(n, reverse);
    };
    let count = motions::multiply_counts(operator.count, cx.count).unwrap_or(1);
    let hit = match cx.editor.find_next(count, reverse) {
        Ok(hit) => hit,
        Err(error) => {
            // No match: the operator is cancelled, as with any failed motion.
            cx.editor.take_pending_operator();
            return Err(error);
        }
    };
    cx.editor.take_pending_operator();
    cx.editor.report_search(&hit);
    operate_to(cx, operator, hit.position)
}

/// Applies `operator` from the cursor up to a search match.
pub(super) fn operate_to(
    cx: &mut Context,
    operator: PendingOperator,
    target: Position,
) -> CommandResult {
    // The count already chose which match; don't apply it again.
    cx.editor
        .begin_operator(operator.operator, None, operator.register);
    let args = MotionArgs {
        target: Some(target),
        ..MotionArgs::default()
    };
    cx.count = None;
    motions::run(cx, &motions::SEARCH_MATCH, args)
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
