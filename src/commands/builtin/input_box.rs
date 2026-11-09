//! A plugin's input box (`editor/input_box.rs`): submitting, cancelling and
//! editing its field, bound in `[keys.input]`.

use crate::commands::{CommandRegistry, Invocation};
use crate::editor::text_field::TextField;

/// An edit to the input box's field.
type FieldEdit = fn(&mut TextField);

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "input_submit",
        "Input box: run the plugin's command with the text",
        |cx| match cx.editor.take_input_box() {
            Some(input) => {
                let args = [input.field.text().to_string()];
                cx.registry
                    .execute(&input.on_submit, cx.editor, Invocation::with_args(&args))
            }
            None => Ok(()),
        },
    );
    r.register(
        "input_cancel",
        "Input box: close it without the text",
        |cx| match cx.editor.take_input_box().and_then(|i| i.on_cancel) {
            Some(command) => cx
                .registry
                .execute(&command, cx.editor, Invocation::default()),
            None => Ok(()),
        },
    );
    let edits: [(&str, &str, FieldEdit); 8] = [
        (
            "input_backspace",
            "Input box: delete the char before the cursor",
            |f| f.backspace(),
        ),
        (
            "input_delete",
            "Input box: delete the char under the cursor",
            |f| f.delete(),
        ),
        (
            "input_delete_word",
            "Input box: delete the word before the cursor",
            |f| f.delete_word(),
        ),
        (
            "input_delete_to_start",
            "Input box: delete to the start",
            |f| f.delete_to_start(),
        ),
        ("input_left", "Input box: cursor left", |f| f.left()),
        ("input_right", "Input box: cursor right", |f| f.right()),
        ("input_home", "Input box: to the start", |f| f.home()),
        ("input_end", "Input box: to the end", |f| f.end()),
    ];
    for (name, description, edit) in edits {
        r.register(name, description, move |cx| {
            cx.editor.input_box_edit(edit);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::{CommandRegistry, Invocation};
    use crate::editor::input_box::InputBox;
    use crate::editor::{Editor, Mode};

    #[test]
    fn submitting_runs_the_command_with_the_text() {
        let mut registry = CommandRegistry::with_builtins();
        registry.register("greet", "", |cx| {
            let name = cx.args.first().cloned().unwrap_or_default();
            cx.editor.document.insert_str(&format!("hi {name}"));
            Ok(())
        });
        let mut editor = Editor::default();
        editor.open_input_box(InputBox::new("Name", "world", "greet"));
        for name in ["input_end", "input_backspace"] {
            registry
                .execute(name, &mut editor, Invocation::default())
                .unwrap();
        }
        editor.insert_text_char('!');
        registry
            .execute("input_submit", &mut editor, Invocation::default())
            .unwrap();
        assert_eq!(editor.document.buffer().text(), "hi worl!");
        assert_eq!(editor.mode(), Mode::Normal);

        editor.open_input_box(InputBox::new("Name", "", "greet"));
        registry
            .execute("input_cancel", &mut editor, Invocation::default())
            .unwrap();
        assert_eq!(editor.document.buffer().text(), "hi worl!", "nothing run");
        assert!(editor.input_box().is_none());
    }
}
