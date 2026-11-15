//! New files, saving one with no name, and the file tree's file
//! operations (`editor/file_ops.rs`).

use crate::commands::{CommandRegistry, Invocation};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "new_file",
        "A new, empty buffer to write a file in (Ctrl+N)",
        |cx| {
            cx.editor.new_buffer();
            Ok(())
        },
    );
    r.alias("enew", "new_file");
    r.register(
        "save",
        "Save the file, asking for a name if it has none (Ctrl+S)",
        |cx| {
            if cx.editor.document.path().is_none() {
                cx.editor.ask_save_as();
                return Ok(());
            }
            cx.registry
                .execute("write", cx.editor, Invocation::default())
        },
    );
    r.register(
        "save_as",
        "Save the buffer under a new name, asked for",
        |cx| {
            cx.editor.ask_save_as();
            Ok(())
        },
    );
    r.register(
        "tree_new_file",
        "In the file tree: a new file in the selected folder (asks its name)",
        |cx| cx.editor.tree_ask_new(false),
    );
    r.register(
        "tree_new_folder",
        "In the file tree: a new folder in the selected folder (asks its name)",
        |cx| cx.editor.tree_ask_new(true),
    );
    r.register(
        "tree_new_file_at_root",
        "In the file tree: a new file in the project's folder (asks its name)",
        |cx| cx.editor.tree_ask_new_at_root(false),
    );
    r.register(
        "tree_new_folder_at_root",
        "In the file tree: a new folder in the project's folder (asks its name)",
        |cx| cx.editor.tree_ask_new_at_root(true),
    );
    r.register(
        "tree_create_file",
        "Make the file named (the new file box's answer)",
        |cx| cx.editor.tree_create(&cx.args.join(" "), false),
    );
    r.register(
        "tree_create_folder",
        "Make the folder named (the new folder box's answer)",
        |cx| cx.editor.tree_create(&cx.args.join(" "), true),
    );
    r.register(
        "tree_rename",
        "In the file tree: rename the selected file or folder (asks the new name)",
        |cx| cx.editor.tree_ask_rename(),
    );
    r.register(
        "tree_rename_to",
        "Rename to the name given (the rename box's answer)",
        |cx| cx.editor.tree_rename(&cx.args.join(" ")),
    );
    r.register(
        "tree_trash",
        "In the file tree: move the selected file or folder to the bin (asks first)",
        |cx| cx.editor.tree_ask_trash(),
    );
    r.register(
        "tree_trash_confirmed",
        "Move it to the bin (the answer when asked)",
        |cx| cx.editor.tree_trash(),
    );
    for (name, relative, tree, what) in [
        ("copy_path", false, false, "the file's full path"),
        (
            "copy_relative_path",
            true,
            false,
            "the file's path from the working directory",
        ),
        (
            "tree_copy_path",
            false,
            true,
            "the selected file's full path",
        ),
        (
            "tree_copy_relative_path",
            true,
            true,
            "the selected file's path from the working directory",
        ),
    ] {
        r.register(name, format!("Copy {what} to the clipboard"), move |cx| {
            cx.editor.copy_path(relative, tree)
        });
    }
    r.register(
        "language",
        "Treat the buffer as language [name] (auto: as its name says); a picker without one",
        |cx| {
            cx.editor
                .choose_language(cx.args.first().map(String::as_str))
        },
    );
    r.register(
        "line_ending",
        "Save the buffer with [lf | crlf] line endings from now on",
        |cx| {
            cx.editor
                .choose_line_ending(cx.args.first().map(String::as_str))
        },
    );
    r.register(
        "goto_line_prompt",
        "Ask for a line (and line:column) to go to (Ctrl+G with VS Code's keys)",
        |cx| {
            cx.editor.ask_goto_line();
            Ok(())
        },
    );
    r.register(
        "goto_line_column",
        "Go to the line given (12, or 12:4 for a column too)",
        |cx| cx.editor.goto_line_column(&cx.args.join(" ")),
    );
    r.register(
        "outline_go",
        "In the Outline: go to the selected symbol",
        |cx| cx.editor.outline_go(),
    );
    r.register(
        "reveal_in_file_manager",
        "Show the file in Finder (or the file manager)",
        |cx| cx.editor.reveal_in_file_manager(false),
    );
    r.register(
        "tree_reveal_in_file_manager",
        "Show the selected file in Finder (or the file manager)",
        |cx| cx.editor.reveal_in_file_manager(true),
    );
}
