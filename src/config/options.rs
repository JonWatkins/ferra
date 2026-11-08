use serde::{Deserialize, Serialize};

use crate::core::search::CaseMode;
use crate::editor::Mode;
use crate::editor::registers::ClipboardMode;
use crate::files::WalkOptions;

/// Behavioural options from the `[editor]` config section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct EditorOptions {
    pub tab_width: usize,
    /// Indent with spaces (`tab-width` of them) rather than tab characters.
    pub expand_tab: bool,
    /// Indent each file the way it's already indented (tabs, or how many
    /// spaces), over `expand-tab` and `tab-width`.
    pub detect_indent: bool,
    /// New lines start with the indentation of the line they came from.
    pub auto_indent: bool,
    pub line_numbers: LineNumbers,
    /// The width `gq` reflows text to (0: 79, as in Vim).
    pub text_width: usize,
    /// Whether typing past `text-width` breaks the line: never, in
    /// comments, or anywhere.
    pub auto_wrap: AutoWrap,
    /// Have the language server format the buffer before each save.
    pub format_on_save: bool,
    /// Long lines continue on the next screen rows (breaking between
    /// words) rather than running off the edge.
    pub wrap: bool,
    /// A wrapped line's continued rows start at its indentation.
    pub wrap_indent: bool,
    /// Shown at the start of a wrapped line's continued rows.
    pub wrap_marker: String,
    pub cursorline: bool,
    /// A status line under every window, or one for the whole screen.
    pub statusline: StatuslineMode,
    /// What the active window's status line shows, and where.
    pub statusline_layout: StatuslineLayout,
    pub scroll_off: usize,
    pub syntax_highlighting: bool,
    /// Milliseconds to wait for the rest of a key sequence that's also a
    /// prefix of a longer one (`jk` in insert mode). 0 waits forever.
    pub key_timeout: u64,
    /// How the system clipboard is used: "registers", "unnamed" or "off".
    pub clipboard: ClipboardMode,
    /// Which clipboard: "auto", "system" or "terminal" (OSC 52).
    pub clipboard_provider: crate::clipboard::ClipboardProvider,
    /// Use the kitty keyboard protocol where the terminal supports it, so
    /// Esc, Alt+key and Ctrl-I/Tab are told apart.
    pub kitty_keyboard: bool,
    /// Start language servers for open files (see `[languages]`).
    pub lsp: bool,
    /// Show the language server's completions as you type in insert mode
    /// (otherwise only on `<C-Space>` or `<C-x><C-o>`).
    pub auto_complete: bool,
    /// Check open files for changes made by other programs every second
    /// (as well as on focus, buffer switches and `:checktime`).
    pub watch_files: bool,
    /// The encoding for files that aren't UTF-8 and have no byte-order mark.
    pub fallback_encoding: FallbackEncoding,
    pub cursor_shape: CursorShapes,
    pub search: SearchOptions,
    pub file_picker: FilePickerOptions,
}

/// The `[editor.file-picker]` config section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct SearchOptions {
    pub case: SearchCase,
    /// Keep matches of the last search highlighted (until `:noh`).
    pub highlight: bool,
    /// Wrap around the end (or start) of the buffer.
    pub wrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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

/// An encoding named in config (`"windows-1252"`, `"shift_jis"`, `"utf-8"`
/// to refuse files that aren't UTF-8), checked when the config is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackEncoding(pub &'static crate::core::Encoding);

impl Serialize for FallbackEncoding {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.name())
    }
}

impl<'de> Deserialize<'de> for FallbackEncoding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let label = String::deserialize(deserializer)?;
        crate::core::Encoding::for_label(label.trim().as_bytes())
            .map(FallbackEncoding)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown encoding: {label}")))
    }
}

/// The `[editor.statusline-layout]` section: the elements on the left, in
/// the center and on the right of the active window's status line, like
/// Helix's `[editor.statusline]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatuslineLayout {
    pub left: Vec<StatusElement>,
    #[serde(default)]
    pub center: Vec<StatusElement>,
    pub right: Vec<StatusElement>,
}

/// One piece of the status line. Elements with nothing to show (`modified`
/// in an unmodified buffer, `file-encoding` for plain UTF-8) take no space.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub enum StatusElement {
    /// `NORMAL`, `INSERT`, `V-LINE`, ...
    Mode,
    /// The buffer's path, shortened from the left to fit the space left.
    FileName,
    /// Just the file's name, without its directory.
    FileBaseName,
    /// `[+]` when there are unsaved changes.
    Modified,
    /// `line:column`.
    Position,
    /// How far through the file the cursor is.
    PositionPercentage,
    /// How many lines the buffer has.
    TotalLines,
    /// The language (`rust`).
    FileType,
    /// The encoding, when it isn't plain UTF-8.
    FileEncoding,
    /// `LF` or `CRLF`.
    FileLineEnding,
    /// How the buffer indents: `2 spaces`, `tabs` (see `:indent`).
    Indent,
    /// How many errors and warnings a language server reports (`E2 W1`).
    Diagnostics,
    /// Keys typed so far for a command (`2d`).
    PendingKeys,
    /// `recording @a` while recording a macro.
    Recording,
    /// A `│` between elements.
    Separator,
    /// A space.
    Spacer,
    /// What a Lua plugin's `editor.status(name, ...)` function returns:
    /// `"plugin:name"` in config.
    Plugin(String),
}

/// The built-in elements and their names in config.
const STATUS_ELEMENTS: &[(StatusElement, &str)] = &[
    (StatusElement::Mode, "mode"),
    (StatusElement::FileName, "file-name"),
    (StatusElement::FileBaseName, "file-base-name"),
    (StatusElement::Modified, "modified"),
    (StatusElement::Position, "position"),
    (StatusElement::PositionPercentage, "position-percentage"),
    (StatusElement::TotalLines, "total-lines"),
    (StatusElement::FileType, "file-type"),
    (StatusElement::FileEncoding, "file-encoding"),
    (StatusElement::FileLineEnding, "file-line-ending"),
    (StatusElement::Indent, "indent"),
    (StatusElement::Diagnostics, "diagnostics"),
    (StatusElement::PendingKeys, "pending-keys"),
    (StatusElement::Recording, "recording"),
    (StatusElement::Separator, "separator"),
    (StatusElement::Spacer, "spacer"),
];

impl TryFrom<String> for StatusElement {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        if let Some(plugin) = name.strip_prefix("plugin:") {
            if plugin.is_empty() {
                return Err("plugin: needs a name (plugin:<name>)".into());
            }
            return Ok(StatusElement::Plugin(plugin.to_string()));
        }
        STATUS_ELEMENTS
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(element, _)| element.clone())
            .ok_or_else(|| format!("unknown status line element `{name}`"))
    }
}

impl From<StatusElement> for String {
    fn from(element: StatusElement) -> String {
        match element {
            StatusElement::Plugin(name) => format!("plugin:{name}"),
            other => STATUS_ELEMENTS
                .iter()
                .find(|(e, _)| *e == other)
                .map_or_else(String::new, |(_, n)| n.to_string()),
        }
    }
}

/// `auto-wrap`: where typing past `text-width` breaks the line (Vim's
/// `formatoptions` `c` and `t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoWrap {
    Off,
    /// Only in comments (by the language's line-comment marker).
    Comments,
    /// In any text.
    All,
}

/// Where status lines go (Neovim's `laststatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatuslineMode {
    /// Under every window, in place of the line between stacked windows
    /// (`laststatus=2`, Neovim's default).
    Windows,
    /// One line at the bottom for the active window (`laststatus=3`).
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LineNumbers {
    Absolute,
    /// Relative to the cursor line, with the cursor line itself absolute.
    Relative,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CursorShapes {
    pub normal: CursorShape,
    pub insert: CursorShape,
    pub command: CursorShape,
    pub operator: CursorShape,
    pub visual: CursorShape,
    pub picker: CursorShape,
    /// Replace mode (`R`).
    pub replace: CursorShape,
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
            replace: CursorShape::Underline,
        };
        assert_eq!(shapes.for_mode(Mode::Insert), CursorShape::Bar);
        assert_eq!(shapes.for_mode(Mode::Command), CursorShape::Underline);
    }
}
