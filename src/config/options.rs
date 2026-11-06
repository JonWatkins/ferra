use serde::Deserialize;

use crate::core::search::CaseMode;
use crate::editor::Mode;
use crate::editor::registers::ClipboardMode;
use crate::files::WalkOptions;

/// Behavioural options from the `[editor]` config section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct EditorOptions {
    pub tab_width: usize,
    /// Indent with spaces (`tab-width` of them) rather than tab characters.
    pub expand_tab: bool,
    /// New lines start with the indentation of the line they came from.
    pub auto_indent: bool,
    pub line_numbers: LineNumbers,
    pub cursorline: bool,
    pub scroll_off: usize,
    pub syntax_highlighting: bool,
    /// Milliseconds to wait for the rest of a key sequence that's also a
    /// prefix of a longer one (`jk` in insert mode). 0 waits forever.
    pub key_timeout: u64,
    /// How the system clipboard is used: "registers", "unnamed" or "off".
    pub clipboard: ClipboardMode,
    pub cursor_shape: CursorShapes,
    pub search: SearchOptions,
    pub file_picker: FilePickerOptions,
}

/// The `[editor.file-picker]` config section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct FilePickerOptions {
    /// List hidden files and directories.
    pub hidden: bool,
    /// Skip files ignored by `.gitignore` and friends.
    pub git_ignore: bool,
    /// Stop listing after this many files.
    pub max_files: usize,
}

impl From<FilePickerOptions> for WalkOptions {
    fn from(options: FilePickerOptions) -> Self {
        Self {
            hidden: options.hidden,
            git_ignore: options.git_ignore,
            max_files: options.max_files,
        }
    }
}

/// The `[editor.search]` config section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SearchOptions {
    pub case: SearchCase,
    /// Keep matches of the last search highlighted (until `:noh`).
    pub highlight: bool,
    /// Wrap around the end (or start) of the buffer.
    pub wrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchCase {
    /// Case-insensitive unless the pattern has an uppercase letter.
    Smart,
    Sensitive,
    Insensitive,
}

impl From<SearchCase> for CaseMode {
    fn from(case: SearchCase) -> Self {
        match case {
            SearchCase::Smart => CaseMode::Smart,
            SearchCase::Sensitive => CaseMode::Sensitive,
            SearchCase::Insensitive => CaseMode::Insensitive,
        }
    }
}

impl Default for EditorOptions {
    fn default() -> Self {
        super::Config::default().editor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineNumbers {
    Absolute,
    /// Relative to the cursor line, with the cursor line itself absolute.
    Relative,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorShapes {
    pub normal: CursorShape,
    pub insert: CursorShape,
    pub command: CursorShape,
    pub operator: CursorShape,
    pub visual: CursorShape,
    pub picker: CursorShape,
}

impl CursorShapes {
    pub fn for_mode(&self, mode: Mode) -> CursorShape {
        match mode {
            Mode::Normal => self.normal,
            Mode::Insert => self.insert,
            Mode::Command => self.command,
            Mode::OperatorPending => self.operator,
            Mode::Visual => self.visual,
            Mode::Picker => self.picker,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_shape_per_mode() {
        let shapes = CursorShapes {
            normal: CursorShape::Block,
            insert: CursorShape::Bar,
            command: CursorShape::Underline,
            operator: CursorShape::Block,
            visual: CursorShape::Block,
            picker: CursorShape::Bar,
        };
        assert_eq!(shapes.for_mode(Mode::Insert), CursorShape::Bar);
        assert_eq!(shapes.for_mode(Mode::Command), CursorShape::Underline);
    }
}
