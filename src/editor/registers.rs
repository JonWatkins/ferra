//! Vim-style registers: named clipboards for yanked and deleted text.
//!
//! | Register | Behaviour |
//! |----------|-----------|
//! | `"`      | Unnamed. Written by every yank/delete; read by default. |
//! | `a`-`z`  | Named. Writing also updates `"`. |
//! | `A`-`Z`  | Appends to the lowercase register. |
//! | `0`      | The last yank into the unnamed register. |
//! | `1`-`9`  | Deletes of a line or more, newest first: each shifts the rest down. |
//! | `-`      | The last delete within one line. |
//! | `_`      | Black hole. Writes are discarded. |
//! | `+`, `*` | The system clipboard (both the same here). |
//! | `/`, `:`, `%` | Read-only: the last search, the last `:` command, the file name. |

use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::clipboard::Clipboard;

pub const UNNAMED: char = '"';
pub const YANK: char = '0';
pub const SMALL_DELETE: char = '-';
pub const BLACK_HOLE: char = '_';
pub const CLIPBOARD: char = '+';
pub const SELECTION: char = '*';
pub const LAST_SEARCH: char = '/';
pub const LAST_COMMAND: char = ':';
pub const FILE_NAME: char = '%';

/// Register names in the order `:registers` lists them.
const LISTED: &str = "\"0123456789abcdefghijklmnopqrstuvwxyz-+";

/// How the system clipboard is used (`clipboard` in `[editor]`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMode {
    /// Only `"+` and `"*` use the system clipboard.
    #[default]
    Registers,
    /// Every yank and delete also goes to the system clipboard, and `p`
    /// pastes from it (Vim's `clipboard=unnamedplus`).
    Unnamed,
    /// Never touch the system clipboard; `"+` is an ordinary register.
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    /// Linewise content (from `dd`, `yy`, ...) is pasted as whole lines.
    pub linewise: bool,
    /// A visual block: one line of `text` per row, pasted as a column.
    pub blockwise: bool,
}

impl Register {
    pub fn charwise(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            linewise: false,
            blockwise: false,
        }
    }

    pub fn linewise(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            linewise: true,
            blockwise: false,
        }
    }

    /// A block's rows, joined with line breaks.
    pub fn blockwise(rows: &[String]) -> Self {
        Self {
            text: rows.join("\n"),
            linewise: false,
            blockwise: true,
        }
    }

    /// `self` followed by `other`. Mixing in a linewise part makes the result
    /// linewise, with the parts on separate lines.
    fn appended(&self, other: &Register) -> Register {
        if self.blockwise || other.blockwise {
            let text = format!("{}\n{}", self.text, other.text);
            let rows: Vec<String> = text.split('\n').map(str::to_string).collect();
            Register::blockwise(&rows)
        } else if self.linewise || other.linewise {
            Register::linewise(format!("{}\n{}", self.text, other.text))
        } else {
            Register::charwise(format!("{}{}", self.text, other.text))
        }
    }
}

/// Why text is being stored, which decides whether `0`, `1`-`9` or `-` is
/// updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Yank,
    Delete,
    /// A delete over a jump motion (`d%`, `d}`, `d'a`): goes to `1` even
    /// within one line, as in Vim.
    DeleteOverJump,
}

#[derive(Default)]
pub struct Registers {
    registers: HashMap<char, Register>,
    clipboard: Option<Box<dyn Clipboard>>,
    mode: ClipboardMode,
    /// A clipboard write that failed, for the app to report. Failures never
    /// cancel the yank or delete they're part of.
    clipboard_error: Option<String>,
}

impl fmt::Debug for Registers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registers")
            .field("registers", &self.registers)
            .field("mode", &self.mode)
            .field("has_clipboard", &self.clipboard.is_some())
            .finish()
    }
}

impl Registers {
    /// Connects the system clipboard, used as `mode` says.
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>, mode: ClipboardMode) {
        self.clipboard = Some(clipboard);
        self.mode = mode;
    }

    /// The last clipboard write that failed, if any (taken once).
    pub fn take_clipboard_error(&mut self) -> Option<String> {
        self.clipboard_error.take()
    }

    /// Whether register `name` is backed by the system clipboard right now.
    fn uses_clipboard(&self, name: Option<char>) -> bool {
        self.clipboard.is_some()
            && match self.mode {
                ClipboardMode::Off => false,
                ClipboardMode::Registers => matches!(name, Some(CLIPBOARD | SELECTION)),
                ClipboardMode::Unnamed => {
                    matches!(name, None | Some(UNNAMED | CLIPBOARD | SELECTION))
                }
            }
    }

    /// Reads a register for pasting. Clipboard registers come from the
    /// system clipboard, where text ending in a line break pastes as whole
    /// lines (as in Vim).
    pub fn read(&mut self, name: Option<char>) -> Result<Option<Register>, String> {
        if self.uses_clipboard(name) {
            let clipboard = self.clipboard.as_mut().expect("checked by uses_clipboard");
            let text = clipboard.get()?.replace("\r\n", "\n");
            if text.is_empty() && name.is_none() {
                // Nothing on the clipboard: fall back to the unnamed register.
                return Ok(self.get(None).cloned());
            }
            return Ok(Some(match text.strip_suffix('\n') {
                Some(lines) => Register::linewise(lines),
                None => Register::charwise(text),
            }));
        }
        Ok(self.get(name).cloned())
    }

    /// Reads a register; `None` means the unnamed register.
    pub fn get(&self, name: Option<char>) -> Option<&Register> {
        let name = name.unwrap_or(UNNAMED).to_ascii_lowercase();
        self.registers.get(&name)
    }

    /// Sets register `name` directly, without touching the unnamed or yank
    /// registers (used for recording macros). Uppercase appends.
    pub fn set(&mut self, name: char, value: Register) -> Result<(), String> {
        match name {
            'a'..='z' | '0'..='9' | SMALL_DELETE | UNNAMED => {
                self.registers.insert(name, value);
            }
            'A'..='Z' => {
                let lower = name.to_ascii_lowercase();
                let merged = match self.registers.get(&lower) {
                    Some(existing) => existing.appended(&value),
                    None => value,
                };
                self.registers.insert(lower, merged);
            }
            _ => return Err(format!("Invalid register: \"{name}")),
        }
        Ok(())
    }

    /// Stores `value` into register `name` (`None` means unnamed).
    pub fn store(
        &mut self,
        name: Option<char>,
        value: Register,
        kind: StoreKind,
    ) -> Result<(), String> {
        if self.uses_clipboard(name) {
            let text = if value.linewise {
                format!("{}\n", value.text)
            } else {
                value.text.clone()
            };
            let clipboard = self.clipboard.as_mut().expect("checked by uses_clipboard");
            if let Err(error) = clipboard.set(&text) {
                self.clipboard_error = Some(error);
            }
        }
        let name = name.unwrap_or(UNNAMED);
        let value = match name {
            BLACK_HOLE => return Ok(()),
            CLIPBOARD | SELECTION => {
                self.registers.insert(CLIPBOARD, value.clone());
                value
            }
            UNNAMED => {
                match kind {
                    StoreKind::Yank => {
                        self.registers.insert(YANK, value.clone());
                    }
                    StoreKind::Delete if !value.linewise && !value.text.contains('\n') => {
                        self.registers.insert(SMALL_DELETE, value.clone());
                    }
                    StoreKind::Delete | StoreKind::DeleteOverJump => {
                        self.shift_numbered(value.clone());
                    }
                }
                value
            }
            'a'..='z' | '0'..='9' | SMALL_DELETE => {
                self.registers.insert(name, value.clone());
                value
            }
            'A'..='Z' => {
                let lower = name.to_ascii_lowercase();
                let merged = match self.registers.get(&lower) {
                    Some(existing) => existing.appended(&value),
                    None => value,
                };
                self.registers.insert(lower, merged.clone());
                merged
            }
            _ => return Err(format!("Invalid register: \"{name}")),
        };
        self.registers.insert(UNNAMED, value);
        Ok(())
    }

    /// Every register holding text, in display order. `"+` is the last text
    /// stored there, not read from the system clipboard.
    pub fn entries(&self) -> Vec<(char, &Register)> {
        LISTED
            .chars()
            .filter_map(|name| Some((name, self.registers.get(&name)?)))
            .collect()
    }

    /// Puts `value` in `1`, moving `1`-`8` down one; `9` falls off.
    fn shift_numbered(&mut self, value: Register) {
        for n in (b'1'..b'9').rev() {
            if let Some(older) = self.registers.remove(&(n as char)) {
                self.registers.insert((n + 1) as char, older);
            }
        }
        self.registers.insert('1', value);
    }
}

impl super::Editor {
    /// Reads register `name` for pasting, including the read-only ones
    /// that come from elsewhere in the editor (`/`, `:`, `%`).
    pub fn read_register(&mut self, name: Option<char>) -> Result<Option<Register>, String> {
        Ok(match name {
            Some(LAST_SEARCH) => self
                .last_search_pattern()
                .map(|p| Register::charwise(p.source())),
            Some(LAST_COMMAND) => self.command_line.last_command().map(Register::charwise),
            Some(FILE_NAME) => self
                .document
                .path()
                .map(|path| Register::charwise(path.to_string_lossy())),
            _ => return self.registers.read(name),
        })
    }

    /// Every register with text, for `:registers`: the read-only ones after
    /// the rest.
    pub fn register_entries(&self) -> Vec<(char, Register)> {
        let mut entries: Vec<(char, Register)> = self
            .registers
            .entries()
            .into_iter()
            .map(|(name, value)| (name, value.clone()))
            .collect();
        let read_only = [
            (
                LAST_SEARCH,
                self.last_search_pattern().map(|p| p.source().to_string()),
            ),
            (
                LAST_COMMAND,
                self.command_line.last_command().map(str::to_string),
            ),
            (
                FILE_NAME,
                self.document
                    .path()
                    .map(|p| p.to_string_lossy().into_owned()),
            ),
        ];
        for (name, text) in read_only {
            if let Some(text) = text {
                entries.push((name, Register::charwise(text)));
            }
        }
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(registers: &Registers, name: Option<char>) -> Option<&str> {
        registers.get(name).map(|r| r.text.as_str())
    }

    #[test]
    fn unnamed_yank_also_sets_yank_register() {
        let mut r = Registers::default();
        r.store(None, Register::charwise("y"), StoreKind::Yank)
            .unwrap();
        r.store(None, Register::charwise("d"), StoreKind::Delete)
            .unwrap();
        assert_eq!(text(&r, None), Some("d"));
        assert_eq!(text(&r, Some('0')), Some("y"));
    }

    #[test]
    fn named_registers_also_set_unnamed() {
        let mut r = Registers::default();
        r.store(Some('a'), Register::charwise("x"), StoreKind::Yank)
            .unwrap();
        assert_eq!(text(&r, Some('a')), Some("x"));
        assert_eq!(text(&r, None), Some("x"));
        assert_eq!(text(&r, Some('0')), None, "named yanks don't touch 0");
    }

    #[test]
    fn uppercase_appends() {
        let mut r = Registers::default();
        r.store(Some('a'), Register::charwise("foo"), StoreKind::Yank)
            .unwrap();
        r.store(Some('A'), Register::charwise("bar"), StoreKind::Yank)
            .unwrap();
        assert_eq!(text(&r, Some('a')), Some("foobar"));
        assert_eq!(text(&r, Some('A')), Some("foobar"));
    }

    #[test]
    fn appending_linewise_joins_lines() {
        let mut r = Registers::default();
        r.store(Some('a'), Register::charwise("x"), StoreKind::Yank)
            .unwrap();
        r.store(Some('A'), Register::linewise("line"), StoreKind::Yank)
            .unwrap();
        assert_eq!(r.get(Some('a')), Some(&Register::linewise("x\nline")));
    }

    #[test]
    fn black_hole_discards() {
        let mut r = Registers::default();
        r.store(None, Register::charwise("keep"), StoreKind::Yank)
            .unwrap();
        r.store(Some('_'), Register::charwise("gone"), StoreKind::Delete)
            .unwrap();
        assert_eq!(text(&r, None), Some("keep"));
    }

    #[test]
    fn set_leaves_unnamed_alone_and_appends_uppercase() {
        let mut r = Registers::default();
        r.store(None, Register::charwise("yanked"), StoreKind::Yank)
            .unwrap();
        r.set('q', Register::charwise("ix")).unwrap();
        r.set('Q', Register::charwise("<Esc>")).unwrap();
        assert_eq!(text(&r, Some('q')), Some("ix<Esc>"));
        assert_eq!(text(&r, None), Some("yanked"));
        assert!(r.set('%', Register::charwise("x")).is_err());
    }

    fn with_clipboard(mode: ClipboardMode) -> (Registers, crate::clipboard::MemoryClipboard) {
        let clipboard = crate::clipboard::MemoryClipboard::default();
        let mut r = Registers::default();
        r.set_clipboard(Box::new(clipboard.clone()), mode);
        (r, clipboard)
    }

    #[test]
    fn plus_register_uses_the_system_clipboard() {
        let (mut r, clipboard) = with_clipboard(ClipboardMode::Registers);
        r.store(Some('+'), Register::linewise("a line"), StoreKind::Yank)
            .unwrap();
        assert_eq!(
            clipboard.contents(),
            "a line\n",
            "lines end with a line break"
        );
        assert_eq!(text(&r, None), Some("a line"), "also the unnamed register");

        clipboard.set_contents("from another app");
        assert_eq!(
            r.read(Some('*')).unwrap(),
            Some(Register::charwise("from another app"))
        );
        clipboard.set_contents("two\r\nlines\r\n");
        assert_eq!(
            r.read(Some('+')).unwrap(),
            Some(Register::linewise("two\nlines"))
        );
    }

    #[test]
    fn plain_yanks_leave_the_clipboard_alone_by_default() {
        let (mut r, clipboard) = with_clipboard(ClipboardMode::Registers);
        r.store(None, Register::charwise("internal"), StoreKind::Yank)
            .unwrap();
        assert_eq!(clipboard.contents(), "");
        assert_eq!(r.read(None).unwrap(), Some(Register::charwise("internal")));
    }

    #[test]
    fn unnamed_mode_syncs_every_yank_and_paste() {
        let (mut r, clipboard) = with_clipboard(ClipboardMode::Unnamed);
        r.store(None, Register::charwise("shared"), StoreKind::Delete)
            .unwrap();
        assert_eq!(clipboard.contents(), "shared");
        clipboard.set_contents("outside");
        assert_eq!(r.read(None).unwrap(), Some(Register::charwise("outside")));
        r.store(Some('a'), Register::charwise("named"), StoreKind::Yank)
            .unwrap();
        assert_eq!(
            clipboard.contents(),
            "outside",
            "named registers stay internal"
        );
    }

    #[test]
    fn off_mode_makes_plus_an_ordinary_register() {
        let (mut r, clipboard) = with_clipboard(ClipboardMode::Off);
        r.store(Some('+'), Register::charwise("kept"), StoreKind::Yank)
            .unwrap();
        assert_eq!(clipboard.contents(), "");
        assert_eq!(r.read(Some('+')).unwrap(), Some(Register::charwise("kept")));
    }

    struct BrokenClipboard;
    impl Clipboard for BrokenClipboard {
        fn get(&mut self) -> Result<String, String> {
            Err("no display".into())
        }
        fn set(&mut self, _: &str) -> Result<(), String> {
            Err("no display".into())
        }
    }

    #[test]
    fn clipboard_failures_dont_lose_the_text() {
        let mut r = Registers::default();
        r.set_clipboard(Box::new(BrokenClipboard), ClipboardMode::Registers);
        r.store(Some('+'), Register::charwise("safe"), StoreKind::Yank)
            .unwrap();
        assert_eq!(r.take_clipboard_error().as_deref(), Some("no display"));
        assert_eq!(text(&r, None), Some("safe"));
        assert!(r.read(Some('+')).is_err(), "reading reports the failure");
    }

    #[test]
    fn invalid_names_are_errors() {
        let mut r = Registers::default();
        let err = r.store(Some('%'), Register::charwise("x"), StoreKind::Yank);
        assert_eq!(err, Err("Invalid register: \"%".into()));
    }

    fn delete(r: &mut Registers, value: Register) {
        r.store(None, value, StoreKind::Delete).unwrap();
    }

    #[test]
    fn line_deletes_shift_through_the_numbered_registers() {
        let mut r = Registers::default();
        for i in 1..=10 {
            delete(&mut r, Register::linewise(format!("line {i}")));
        }
        assert_eq!(text(&r, Some('1')), Some("line 10"));
        assert_eq!(text(&r, Some('2')), Some("line 9"));
        assert_eq!(text(&r, Some('9')), Some("line 2"), "line 1 fell off");
        assert_eq!(text(&r, None), Some("line 10"));
    }

    #[test]
    fn small_deletes_go_to_minus_and_leave_the_numbers_alone() {
        let mut r = Registers::default();
        delete(&mut r, Register::linewise("whole"));
        delete(&mut r, Register::charwise("word"));
        assert_eq!(text(&r, Some('-')), Some("word"));
        assert_eq!(text(&r, Some('1')), Some("whole"));
        assert_eq!(text(&r, Some('2')), None);
        assert_eq!(text(&r, None), Some("word"));
    }

    #[test]
    fn a_charwise_delete_across_lines_is_numbered() {
        let mut r = Registers::default();
        delete(&mut r, Register::charwise("end\nstart"));
        assert_eq!(text(&r, Some('1')), Some("end\nstart"));
        assert_eq!(text(&r, Some('-')), None);
    }

    #[test]
    fn deletes_over_jumps_are_numbered_even_when_small() {
        let mut r = Registers::default();
        r.store(None, Register::charwise("(x)"), StoreKind::DeleteOverJump)
            .unwrap();
        assert_eq!(text(&r, Some('1')), Some("(x)"));
        assert_eq!(text(&r, Some('-')), None);
    }

    #[test]
    fn yanks_and_named_or_black_hole_deletes_skip_the_numbers() {
        let mut r = Registers::default();
        r.store(None, Register::linewise("yanked"), StoreKind::Yank)
            .unwrap();
        r.store(Some('a'), Register::linewise("named"), StoreKind::Delete)
            .unwrap();
        r.store(Some('_'), Register::linewise("gone"), StoreKind::Delete)
            .unwrap();
        r.store(Some('a'), Register::charwise("w"), StoreKind::Delete)
            .unwrap();
        assert_eq!(text(&r, Some('1')), None);
        assert_eq!(text(&r, Some('-')), None);
        assert_eq!(text(&r, Some('0')), Some("yanked"));
    }

    #[test]
    fn numbered_and_minus_can_be_written_directly() {
        let mut r = Registers::default();
        r.store(Some('3'), Register::charwise("three"), StoreKind::Yank)
            .unwrap();
        r.store(Some('-'), Register::charwise("dash"), StoreKind::Yank)
            .unwrap();
        assert_eq!(text(&r, Some('3')), Some("three"));
        assert_eq!(text(&r, Some('-')), Some("dash"));
        assert_eq!(text(&r, Some('1')), None, "no shifting");
    }
}
