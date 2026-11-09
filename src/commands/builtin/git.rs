//! Git commands (`editor/git.rs`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "git_next_change",
        "To the next git change in the file ([count] on) (]c)",
        |cx| {
            let n = cx.count();
            cx.editor.goto_git_change(true, n)
        },
    );
    r.register(
        "git_prev_change",
        "To the previous git change in the file ([c)",
        |cx| {
            let n = cx.count();
            cx.editor.goto_git_change(false, n)
        },
    );
    r.register(
        "git_stage",
        "Source Control: stage the selected file (s)",
        |cx| cx.editor.git_stage_selected(),
    );
    r.register(
        "git_unstage",
        "Source Control: unstage the selected file (u)",
        |cx| cx.editor.git_unstage_selected(),
    );
    r.register("git_stage_all", "Stage every change (S)", |cx| {
        cx.editor.git_stage_all(true)
    });
    r.register("git_unstage_all", "Unstage everything (U)", |cx| {
        cx.editor.git_stage_all(false)
    });
    r.register(
        "git_discard",
        "Source Control: throw away the selected file's unstaged changes, after asking (x)",
        |cx| cx.editor.git_ask_discard(),
    );
    r.register(
        "git_discard_confirmed",
        "Throw away the changes asked about (y in the question)",
        |cx| cx.editor.git_discard_confirmed(),
    );
    r.register(
        "git_commit",
        "Commit what's staged, asking for the message (c)",
        |cx| cx.editor.git_ask_commit(),
    );
    r.register(
        "git_commit_message",
        "Commit what's staged with the message given (:git_commit_message text)",
        |cx| {
            let message = if cx.arg_text.is_empty() {
                cx.args.join(" ")
            } else {
                cx.arg_text.to_string()
            };
            cx.editor.git_commit(&message)
        },
    );
    r.register("git_open", "Source Control: open the selected file", |cx| {
        cx.editor.git_open_selected()
    });
    r.register("git_view", "Open the sidebar at Source Control", |cx| {
        cx.editor.open_sidebar_view("git")
    });
    r.register(
        "git_refresh",
        "Read what git has staged again, for the gutter and status line",
        |cx| {
            cx.editor.git_refresh_all();
            Ok(())
        },
    );
}
