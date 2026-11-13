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
        |cx| cx.editor.git_gui_enter(),
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
    use crate::git::RemoteOp;
    let remotes: [(&str, &str, RemoteOp); 3] = [
        (
            "git_fetch",
            "Fetch from the remotes, in the background (f)",
            RemoteOp::Fetch,
        ),
        (
            "git_pull",
            "Pull into the current branch, in the background (p)",
            RemoteOp::Pull,
        ),
        (
            "git_push",
            "Push the current branch, in the background (P)",
            RemoteOp::Push,
        ),
    ];
    for (name, description, op) in remotes {
        r.register(name, description, move |cx| cx.editor.git_remote(op));
    }
    r.register("git_branch", "Switch branch, from a list of them", |cx| {
        cx.editor.git_branch_picker()
    });
    r.register(
        "git_switch",
        "Switch to the branch named (:git_switch name)",
        |cx| {
            let name = cx
                .args
                .first()
                .ok_or("Which branch? (:git_switch name)")?
                .clone();
            cx.editor.git_switch_branch(&name)
        },
    );
    r.register(
        "git_new_branch",
        "Make a branch where you are and switch to it (n in the git view asks for its name)",
        |cx| match cx.args.first() {
            Some(name) => {
                let name = name.clone();
                cx.editor.git_new_branch(&name)
            }
            None => cx.editor.git_ask_new_branch(),
        },
    );
    r.register(
        "git_delete_branch",
        "Git view: delete the selected branch, after asking (d)",
        |cx| cx.editor.git_ask_delete_branch(false),
    );
    r.register(
        "git_force_delete_branch",
        "Git view: delete the selected branch even if it isn't merged, after asking (D)",
        |cx| cx.editor.git_ask_delete_branch(true),
    );
    r.register(
        "git_delete_branch_confirmed",
        "Delete the branch asked about (y in the question)",
        |cx| cx.editor.git_delete_branch_confirmed(),
    );
    r.register(
        "git_blame",
        "Show or hide who last changed the cursor's line, after it",
        |cx| {
            let on = !cx.editor.options.git_blame;
            cx.editor
                .set_option("git-blame", toml::Value::Boolean(on), false)?;
            cx.editor.set_message(crate::editor::Message::info(if on {
                "Line blame on"
            } else {
                "Line blame off"
            }));
            Ok(())
        },
    );
    r.register(
        "git_amend",
        "Fold what's staged into the last commit, editing its message (C)",
        |cx| cx.editor.git_ask_amend(),
    );
    r.register(
        "git_amend_message",
        "Amend the last commit with the message given",
        |cx| {
            let message = if cx.arg_text.is_empty() {
                cx.args.join(" ")
            } else {
                cx.arg_text.to_string()
            };
            cx.editor.git_amend(&message)
        },
    );
    r.register(
        "git_stash",
        "Put every change away in a stash, asking for a message (z)",
        |cx| cx.editor.git_ask_stash(),
    );
    r.register(
        "git_stash_message",
        "Stash every change with the message given",
        |cx| {
            let message = if cx.arg_text.is_empty() {
                cx.args.join(" ")
            } else {
                cx.arg_text.to_string()
            };
            cx.editor.git_stash(&message)
        },
    );
    r.register("git_stash_list", "Pick a stash to put back (Z)", |cx| {
        cx.editor.git_stash_picker()
    });
    r.register(
        "git_stash_pop",
        "Put back the stash named (:git_stash_pop stash@{0}), and drop it",
        |cx| {
            let name = cx
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| "stash@{0}".into());
            cx.editor.git_stash_pop(&name)
        },
    );
    r.register(
        "git_history",
        "The current file's history: the commits that changed it, in the git view (<Space>l)",
        |cx| match cx.args.first() {
            Some(path) => {
                let path = path.clone();
                cx.editor.git_show_history(&path)
            }
            None => cx.editor.git_file_history(),
        },
    );
    r.register(
        "git_blame_view",
        "Who last changed each line of the current file, in the git view (<Space>B)",
        |cx| cx.editor.git_blame_view(),
    );
    r.register(
        "git_gui_blame",
        "Git view: the selected file's blame, or back out of it (B)",
        |cx| cx.editor.git_gui_blame(),
    );
    r.register(
        "git_gui_history",
        "Git view: the selected file's history, or back to the whole log (H)",
        |cx| cx.editor.git_gui_history(),
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
