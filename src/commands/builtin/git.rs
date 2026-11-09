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
    // The full-screen git view (`editor/git_gui.rs`).
    r.register(
        "git_gui_open",
        "Open the git view: changed files, branches, commits and diffs (<Space>g)",
        |cx| cx.editor.open_git_gui(),
    );
    r.alias("git", "git_gui_open");
    r.register("git_gui_close", "Close the git view", |cx| {
        cx.editor.close_git_gui();
        Ok(())
    });
    r.register(
        "git_gui_back",
        "Git view: out of the diff, or close the view",
        |cx| {
            cx.editor.git_gui_back();
            Ok(())
        },
    );
    use crate::editor::git_gui::GitPanel;
    let panels: [(&str, &str, GitPanel); 4] = [
        (
            "git_gui_files",
            "Git view: the changed files (1)",
            GitPanel::Files,
        ),
        (
            "git_gui_branches",
            "Git view: the branches (2)",
            GitPanel::Branches,
        ),
        (
            "git_gui_commits",
            "Git view: the commits (3)",
            GitPanel::Commits,
        ),
        ("git_gui_diff", "Git view: the diff (4)", GitPanel::Diff),
    ];
    for (name, description, panel) in panels {
        r.register(name, description, move |cx| {
            cx.editor.git_gui_focus(panel);
            Ok(())
        });
    }
    r.register("git_gui_next_panel", "Git view: the next panel", |cx| {
        cx.editor.git_gui_cycle(1);
        Ok(())
    });
    r.register("git_gui_prev_panel", "Git view: the previous panel", |cx| {
        cx.editor.git_gui_cycle(-1);
        Ok(())
    });
    r.register(
        "git_gui_down",
        "Git view: down (the next hunk in a diff)",
        |cx| {
            cx.editor.git_gui_move(cx.count() as isize);
            Ok(())
        },
    );
    r.register(
        "git_gui_up",
        "Git view: up (the previous hunk in a diff)",
        |cx| {
            cx.editor.git_gui_move(-(cx.count() as isize));
            Ok(())
        },
    );
    r.register(
        "git_gui_scroll_down",
        "Git view: scroll the diff down",
        |cx| {
            cx.editor.git_gui_scroll(10 * cx.count() as isize);
            Ok(())
        },
    );
    r.register("git_gui_scroll_up", "Git view: scroll the diff up", |cx| {
        cx.editor.git_gui_scroll(-10 * cx.count() as isize);
        Ok(())
    });
    r.register(
        "git_gui_enter",
        "Git view: into the diff, to stage hunks",
        |cx| {
            cx.editor.git_gui_enter();
            Ok(())
        },
    );
    r.register(
        "git_gui_stage",
        "Git view: stage (or unstage) the file, or the hunk in its diff",
        |cx| cx.editor.git_gui_stage(true),
    );
    r.register(
        "git_gui_unstage",
        "Git view: unstage the file, or the hunk in its diff",
        |cx| cx.editor.git_gui_stage(false),
    );
    r.register(
        "git_refresh",
        "Read what git has staged again, for the gutter and status line",
        |cx| {
            cx.editor.git_refresh_all();
            Ok(())
        },
    );
}
