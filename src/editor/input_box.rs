//! A box asking for a line of text, for plugins (`ctx:input{...}`), like
//! VS Code's input box: a title, the field (its starting text selected, so
//! typing replaces it), and a placeholder while it's empty. `<CR>` runs
//! the plugin's command with what was typed as its argument; `<Esc>`
//! cancels. It has focus in `Mode::Input`, with `[keys.input]`.

use super::text_field::TextField;
use super::{Editor, Mode};

#[derive(Debug, Clone, Default)]
pub struct InputBox {
    pub title: String,
    pub field: TextField,
    /// Shown, dimmed, while the field's empty.
    pub placeholder: String,
    /// The command run with the text as its argument.
    pub on_submit: String,
    /// A command run when it's cancelled, if any.
    pub on_cancel: Option<String>,
}

impl InputBox {
    pub fn new(title: &str, value: &str, on_submit: &str) -> Self {
        let mut field = TextField::default();
        field.set_selected(value);
        Self {
            title: title.to_string(),
            field,
            on_submit: on_submit.to_string(),
            ..Self::default()
        }
    }
}

impl Editor {
    pub fn input_box(&self) -> Option<&InputBox> {
        self.input_box.as_ref()
    }

    /// Opens `input`, with focus.
    pub fn open_input_box(&mut self, input: InputBox) {
        // A popup or picker it was asked from gives way.
        self.popup = None;
        self.input_box = Some(input);
        self.set_mode(Mode::Input);
    }

    /// Edits the field (`edit` is one of `TextField`'s).
    pub fn input_box_edit(&mut self, edit: impl FnOnce(&mut TextField)) {
        if let Some(input) = &mut self.input_box {
            edit(&mut input.field);
        }
    }

    /// Closes the box, back in normal mode, returning it.
    pub fn take_input_box(&mut self) -> Option<InputBox> {
        let input = self.input_box.take()?;
        if self.mode() == Mode::Input {
            self.set_mode(Mode::Normal);
        }
        Some(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_starting_text_and_closing_returns_it() {
        let mut editor = Editor::default();
        editor.open_input_box(InputBox::new("Rename", "old", "rename_to"));
        assert_eq!(editor.mode(), Mode::Input);
        editor.insert_text_char('n');
        editor.insert_text_char('e');
        editor.insert_text_char('w');
        assert_eq!(editor.input_box().unwrap().field.text(), "new");
        let input = editor.take_input_box().unwrap();
        assert_eq!(
            (input.field.text(), input.on_submit.as_str()),
            ("new", "rename_to")
        );
        assert_eq!(editor.mode(), Mode::Normal);
        assert!(editor.input_box().is_none());
    }
}
