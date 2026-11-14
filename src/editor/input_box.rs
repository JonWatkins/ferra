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
    /// It asks for a path: `<Tab>` completes folders and files.
    pub completes_paths: bool,
    /// The completions `<Tab>` goes through, the one shown, and the text
    /// they were for (typing anything else starts again).
    completion: Option<(Vec<String>, usize, String)>,
    /// Where focus goes back to: the sidebar or the git view, if it was
    /// asked from there, else normal mode.
    back_to: Option<Mode>,
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
    pub fn open_input_box(&mut self, mut input: InputBox) {
        input.back_to = match self.mode() {
            mode @ (Mode::Sidebar | Mode::Git) => Some(mode),
            Mode::Popup => self.popup.as_ref().and_then(|p| p.back_to),
            _ => None,
        };
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

    /// `<Tab>` / `<S-Tab>` in a box asking for a path: fills in as far as
    /// the folders and files that match agree, then (again) goes through
    /// them one by one. Says what there is to choose from.
    pub fn input_box_complete(&mut self, forward: bool) -> Result<(), String> {
        let Some(input) = &mut self.input_box else {
            return Ok(());
        };
        if !input.completes_paths {
            return Ok(());
        }
        let typed = input.field.text().to_string();
        // Going through the choices already offered.
        if let Some((choices, at, shown)) = &mut input.completion
            && *shown == typed
            && choices.len() > 1
        {
            let n = choices.len();
            *at = if forward {
                (*at + 1) % n
            } else {
                (*at + n - 1) % n
            };
            let choice = choices[*at].clone();
            *shown = choice.clone();
            input.field.set_text(&choice);
            input.field.end();
            return Ok(());
        }
        let choices = crate::files::complete_path(&typed);
        match choices.as_slice() {
            [] => Err(format!("Nothing matches {typed}")),
            [only] => {
                input.field.set_text(only);
                input.field.end();
                input.completion = None;
                Ok(())
            }
            many => {
                // As far as they all agree.
                let common = many.iter().skip(1).fold(many[0].clone(), |common, c| {
                    common
                        .chars()
                        .zip(c.chars())
                        .take_while(|(a, b)| a == b)
                        .map(|(a, _)| a)
                        .collect()
                });
                let text = if common.chars().count() > typed.chars().count() {
                    common
                } else {
                    // Nothing more in common: the first choice.
                    many[0].clone()
                };
                let at = many.iter().position(|c| *c == text).unwrap_or(usize::MAX);
                input.field.set_text(&text);
                input.field.end();
                // Each by its own name (a folder's with its `/`).
                let names: Vec<String> = many
                    .iter()
                    .map(|c| {
                        let trimmed = c.trim_end_matches('/');
                        let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
                        let slash = if c.ends_with('/') { "/" } else { "" };
                        format!("{name}{slash}")
                    })
                    .collect();
                let at = if at == usize::MAX { many.len() - 1 } else { at };
                input.completion = Some((many.to_vec(), at, text));
                self.set_message(super::Message::info(names.join("  ")));
                Ok(())
            }
        }
    }

    /// Closes the box, back in normal mode, returning it.
    pub fn take_input_box(&mut self) -> Option<InputBox> {
        let input = self.input_box.take()?;
        if self.mode() == Mode::Input {
            self.set_mode(input.back_to.unwrap_or(Mode::Normal));
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
