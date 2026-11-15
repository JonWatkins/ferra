//! What clicking the status bar's items does, as in VS Code: the branch
//! picks a branch, the problems list them, `Ln 12, Col 4` asks for a line,
//! `Spaces: 4` changes the indentation, `UTF-8` the encoding, `LF` the line
//! endings, the language the language. The commands behind them work from
//! the `:` line and keys too (`:language`, `:line_ending`, `:goto_line_prompt`).

use super::input_box::InputBox;
use super::picker::{Picker, PickerAction, PickerItem};
use super::{Editor, Message};
use crate::core::LineEnding;

/// The indentation menu: a label and the `:indent` it runs.
pub const INDENT_MENU: &[Option<(&str, &str)>] = &[
    Some(("Indent with 2 Spaces", "indent_style 2")),
    Some(("Indent with 4 Spaces", "indent_style 4")),
    Some(("Indent with 8 Spaces", "indent_style 8")),
    Some(("Indent with Tabs", "indent_style tabs")),
    None,
    Some(("Detect from the File", "indent_style auto")),
];

/// The encoding menu: save with another encoding.
pub const ENCODING_MENU: &[Option<(&str, &str)>] = &[
    Some(("Save as UTF-8", "file_encoding utf-8")),
    Some(("Save as UTF-8 with BOM", "file_encoding utf-8-bom")),
    Some(("Save as UTF-16 LE", "file_encoding utf-16le")),
    Some(("Save as UTF-16 BE", "file_encoding utf-16be")),
    Some(("Save as Windows 1252", "file_encoding windows-1252")),
    Some(("Save as Latin-1", "file_encoding latin1")),
    Some(("Save as Shift JIS", "file_encoding shift_jis")),
];

/// The line endings menu.
pub const LINE_ENDING_MENU: &[Option<(&str, &str)>] = &[
    Some(("LF (Linux, macOS)", "line_ending lf")),
    Some(("CRLF (Windows)", "line_ending crlf")),
];

impl Editor {
    /// `:language name` treats the buffer as `name` (`auto`: as its name
    /// says); with no name, a picker of them.
    pub fn choose_language(&mut self, name: Option<&str>) -> Result<(), String> {
        let Some(name) = name else {
            let current = self.language_of(&self.document).map(str::to_string);
            let mut items = vec![PickerItem {
                label: "Auto (from the file's name)".into(),
                detail: String::new(),
                action: PickerAction::RunWith("language".into(), vec!["auto".into()]),
            }];
            let mut names: Vec<&str> = self.languages.names().collect();
            names.sort_unstable();
            items.extend(names.into_iter().map(|name| PickerItem {
                label: name.to_string(),
                detail: if current.as_deref() == Some(name) {
                    "current".into()
                } else {
                    String::new()
                },
                action: PickerAction::RunWith("language".into(), vec![name.to_string()]),
            }));
            self.open_picker(Picker::new("Language", items));
            return Ok(());
        };
        if name == "auto" {
            self.document.set_language(None);
        } else {
            if !self.languages.names().any(|n| n == name) {
                return Err(format!("No language {name} (:language lists them)"));
            }
            self.document.set_language(Some(name.to_string()));
        }
        self.refresh_options();
        let shown = self
            .language_of(&self.document)
            .unwrap_or("plain text")
            .to_string();
        self.set_message(Message::info(format!("Language: {shown}")));
        Ok(())
    }

    /// `:line_ending lf` / `crlf`: saves with those line endings from now
    /// on; with neither, says which it has.
    pub fn choose_line_ending(&mut self, ending: Option<&str>) -> Result<(), String> {
        let ending = match ending.map(str::to_ascii_lowercase).as_deref() {
            None => {
                let current = match self.document.format().line_ending {
                    LineEnding::Lf => "LF",
                    LineEnding::Crlf => "CRLF",
                };
                self.set_message(Message::info(current));
                return Ok(());
            }
            Some("lf" | "unix") => LineEnding::Lf,
            Some("crlf" | "dos" | "windows") => LineEnding::Crlf,
            Some(other) => return Err(format!("Line endings are lf or crlf, not {other}")),
        };
        self.document.set_line_ending(ending);
        let name = if ending == LineEnding::Lf {
            "LF"
        } else {
            "CRLF"
        };
        self.set_message(Message::info(format!("Will save with {name} line endings")));
        Ok(())
    }

    /// `Ln 12, Col 4` clicked (`:goto_line_prompt`): asks for a line (and,
    /// after a `:`, a column).
    pub fn ask_goto_line(&mut self) {
        let mut input = InputBox::new("Go to line", "", "goto_line_column");
        input.placeholder = format!("1 to {} (line:column)", self.document.buffer().line_count());
        self.open_input_box(input);
    }

    /// The "Go to line" box's answer: `12` or `12:4`.
    pub fn goto_line_column(&mut self, text: &str) -> Result<(), String> {
        let text = text.trim();
        let (line, column) = match text.split_once(':') {
            Some((line, column)) => (line, Some(column)),
            None => (text, None),
        };
        let line: usize = line
            .trim()
            .parse()
            .map_err(|_| format!("Not a line number: {text}"))?;
        let buffer = self.document.buffer();
        let row = line.max(1).min(buffer.line_count()) - 1;
        let col = column
            .and_then(|c| c.trim().parse::<usize>().ok())
            .map_or(0, |c| c.saturating_sub(1).min(buffer.line_len(row)));
        self.push_jump();
        self.document
            .set_cursor(crate::core::Position::new(row, col));
        self.enforce_cursor_rules();
        Ok(())
    }

    /// A status bar menu (indentation, encoding, line endings) at `x`, `y`.
    pub fn status_menu(&mut self, items: &[Option<(&str, &str)>], x: u16, y: u16) {
        self.show_menu_at(items, x, y);
    }
}
