//! The find box's commands (`editor/find.rs`): opening it, editing its
//! fields, the toggles, and finding and replacing.

use crate::commands::CommandRegistry;
use crate::editor::find::FindField;
use crate::editor::text_field::TextField;

/// An edit to the find box's field with focus.
type FieldEdit = fn(&mut TextField);

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "find_open",
        "Find in the file, in a box, as you type (the selection or the word under the cursor to start)",
        |cx| {
            cx.editor.open_find(false);
            Ok(())
        },
    );
    r.register(
        "find_replace_open",
        "Find and replace in the file, in a box",
        |cx| {
            cx.editor.open_find(true);
            Ok(())
        },
    );
    r.register("find_close", "Close the find box, at the match", |cx| {
        cx.editor.close_find();
        Ok(())
    });
    r.register(
        "find_enter",
        "In the find field, the next match; in the replace field, replace the match and go to the next",
        |cx| match cx.editor.find_state().focus {
            FindField::Find => cx.editor.find_step(true),
            FindField::Replace => cx.editor.find_replace_one(),
        },
    );
    r.register("find_next", "The find box's next match", |cx| {
        cx.editor.find_step(true)
    });
    r.register("find_prev", "The find box's previous match", |cx| {
        cx.editor.find_step(false)
    });
    r.register(
        "find_replace_one",
        "Replace the match at the cursor and go to the next",
        |cx| cx.editor.find_replace_one(),
    );
    r.register(
        "find_replace_all",
        "Replace every match in the file (one undo step)",
        |cx| cx.editor.find_replace_all(),
    );
    r.register(
        "find_switch_field",
        "Between the find and replace fields",
        |cx| {
            cx.editor.find_switch_field();
            Ok(())
        },
    );
    r.register("find_toggle_case", "Find: match case, or not", |cx| {
        cx.editor.find_toggle(|f| f.match_case = !f.match_case);
        Ok(())
    });
    r.register("find_toggle_word", "Find: whole words only, or not", |cx| {
        cx.editor.find_toggle(|f| f.whole_word = !f.whole_word);
        Ok(())
    });
    r.register(
        "find_toggle_regex",
        "Find: a regular expression, or the text as it is",
        |cx| {
            cx.editor.find_toggle(|f| f.regex = !f.regex);
            Ok(())
        },
    );

    // Editing the field with focus.
    let edits: [(&str, &str, FieldEdit); 8] = [
        (
            "find_backspace",
            "Find box: delete the char before the cursor",
            |f| f.backspace(),
        ),
        (
            "find_delete",
            "Find box: delete the char under the cursor",
            |f| f.delete(),
        ),
        (
            "find_delete_word",
            "Find box: delete the word before the cursor",
            |f| f.delete_word(),
        ),
        (
            "find_delete_to_start",
            "Find box: delete to the field's start",
            |f| f.delete_to_start(),
        ),
        ("find_left", "Find box: cursor left", |f| f.left()),
        ("find_right", "Find box: cursor right", |f| f.right()),
        ("find_home", "Find box: to the field's start", |f| f.home()),
        ("find_end", "Find box: to the field's end", |f| f.end()),
    ];
    for (name, description, edit) in edits {
        r.register(name, description, move |cx| {
            cx.editor.find_edit(edit);
            Ok(())
        });
    }
}
