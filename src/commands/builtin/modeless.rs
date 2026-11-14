//! Commands for VS Code's keys (`keymap = "vscode"`, see
//! `editor/modeless.rs` and `config/keymaps/vscode.toml`): moving with and
//! without Shift selecting, the clipboard, and line editing.

use crate::commands::{CommandRegistry, CommandResult, Context, Invocation};
use crate::editor::modeless::Move;

/// Runs command `name` (a move of Vim's) after starting or dropping the
/// selection.
fn then(cx: &mut Context, name: &str) -> CommandResult {
    cx.registry.execute(name, cx.editor, Invocation::default())
}

pub fn register(r: &mut CommandRegistry) {
    const MOVES: [(&str, &str, Move); 8] = [
        (
            "left",
            "a character left (to the line before at its start)",
            Move::Left,
        ),
        (
            "right",
            "a character right (to the next line at its end)",
            Move::Right,
        ),
        (
            "word_left",
            "to the start of the word before",
            Move::WordLeft,
        ),
        ("word_right", "past the end of the word", Move::WordRight),
        (
            "home",
            "to the first non-blank (or the line's start)",
            Move::Home,
        ),
        ("end", "to the line's end", Move::End),
        ("top", "to the file's start", Move::DocStart),
        ("bottom", "to the file's end", Move::DocEnd),
    ];
    for (name, what, to) in MOVES {
        r.register(
            format!("cursor_{name}"),
            format!("Move {what}, dropping any selection"),
            move |cx| {
                cx.editor.modeless_move(to, false);
                Ok(())
            },
        );
        r.register(
            format!("select_{name}"),
            format!("Select {what} (Shift and a move)"),
            move |cx| {
                cx.editor.modeless_move(to, true);
                Ok(())
            },
        );
    }
    // Up, down and by pages: Vim's moves, which keep the column.
    const VERTICAL: [(&str, &str, &str); 4] = [
        ("up", "a line up", "move_up"),
        ("down", "a line down", "move_down"),
        ("page_up", "a page up", "scroll_page_up"),
        ("page_down", "a page down", "scroll_page_down"),
    ];
    for (name, what, command) in VERTICAL {
        r.register(
            format!("cursor_{name}"),
            format!("Move {what}, dropping any selection (or through the completions shown)"),
            move |cx| {
                if cx.editor.server_completion_showing() {
                    match name {
                        "up" => return then(cx, "complete_previous"),
                        "down" => return then(cx, "complete_next"),
                        _ => {}
                    }
                }
                cx.editor.clear_selection();
                then(cx, command)
            },
        );
        r.register(
            format!("select_{name}"),
            format!("Select {what} (Shift and a move)"),
            move |cx| {
                cx.editor.begin_selection();
                then(cx, command)
            },
        );
    }

    r.register(
        "accept_or_newline",
        "Take the completion shown, or break the line (Enter, with VS Code's keys)",
        |cx| {
            if cx.editor.server_completion_showing() {
                cx.editor.accept_completion();
                return Ok(());
            }
            cx.editor.delete_selection();
            then(cx, "insert_newline")
        },
    );
    r.register("select_all", "Select the whole file (Ctrl+A)", |cx| {
        cx.editor.select_all();
        Ok(())
    });
    r.register(
        "select_line",
        "Select the cursor's line, and the next each time again (Ctrl+L)",
        |cx| {
            cx.editor.select_line();
            Ok(())
        },
    );
    r.register(
        "select_next_match",
        "Select the word at the cursor, or the next place the selected text is found (Ctrl+D)",
        |cx| cx.editor.select_next_match(),
    );
    r.register(
        "copy",
        "Copy the selection (or the line) to the clipboard (Ctrl+C)",
        |cx| cx.editor.copy_selection(false),
    );
    r.register(
        "cut",
        "Cut the selection (or the line) to the clipboard (Ctrl+X)",
        |cx| cx.editor.copy_selection(true),
    );
    r.register(
        "paste_clipboard",
        "Paste the clipboard over the selection, or at the cursor (Ctrl+V)",
        |cx| cx.editor.paste_clipboard(),
    );
    r.register(
        "delete_left",
        "Delete the selection, or the character before the cursor (Backspace)",
        |cx| {
            cx.editor.delete_char_or_selection(false);
            Ok(())
        },
    );
    r.register(
        "delete_right",
        "Delete the selection, or the character after the cursor (Delete)",
        |cx| {
            cx.editor.delete_char_or_selection(true);
            Ok(())
        },
    );
    r.register(
        "delete_word_left",
        "Delete the selection, or to the start of the word before (Ctrl+Backspace)",
        |cx| {
            cx.editor.delete_word_or_selection(false);
            Ok(())
        },
    );
    r.register(
        "delete_word_right",
        "Delete the selection, or to the end of the word (Ctrl+Delete)",
        |cx| {
            cx.editor.delete_word_or_selection(true);
            Ok(())
        },
    );
    r.register(
        "delete_lines",
        "Delete the selected lines, or the cursor's (Ctrl+Shift+K)",
        |cx| {
            cx.editor.delete_selected_lines();
            Ok(())
        },
    );
    for (down, name, key) in [(false, "up", "Alt+Up"), (true, "down", "Alt+Down")] {
        r.register(
            format!("move_lines_{name}"),
            format!("Move the selected lines (or the cursor's) {name} ({key})"),
            move |cx| {
                cx.editor.move_selected_lines(down);
                Ok(())
            },
        );
        r.register(
            format!("copy_lines_{name}"),
            format!("Copy the selected lines (or the cursor's) {name} (Shift+{key})"),
            move |cx| {
                cx.editor.copy_selected_lines(down);
                Ok(())
            },
        );
    }
    r.register(
        "toggle_comment",
        "Comment the selected lines (or the cursor's) in or out (Ctrl+/)",
        |cx| cx.editor.toggle_comment_selected(),
    );
    r.register(
        "indent_or_tab",
        "Indent the selected lines, or put in a tab (Tab)",
        |cx| {
            if cx.editor.selection().is_some() {
                cx.editor.shift_selected_lines(true);
                Ok(())
            } else {
                then(cx, "insert_tab")
            }
        },
    );
    r.register(
        "outdent_lines",
        "Outdent the selected lines, or the cursor's (Shift+Tab)",
        |cx| {
            cx.editor.shift_selected_lines(false);
            Ok(())
        },
    );
    r.register(
        "escape",
        "Drop the selection, completion and search highlights (Esc, with VS Code's keys)",
        |cx| {
            cx.editor.modeless_escape();
            Ok(())
        },
    );
}
