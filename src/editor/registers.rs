//! Vim-style registers: named clipboards for yanked and deleted text.
//!
//! | Register | Behaviour |
//! |----------|-----------|
//! | `"`      | Unnamed. Written by every yank/delete; read by default. |
//! | `a`-`z`  | Named. Writing also updates `"`. |
//! | `A`-`Z`  | Appends to the lowercase register. |
//! | `0`      | The last yank into the unnamed register. |
//! | `_`      | Black hole. Writes are discarded. |

use std::collections::HashMap;

pub const UNNAMED: char = '"';
pub const YANK: char = '0';
pub const BLACK_HOLE: char = '_';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub text: String,
    /// Linewise content (from `dd`, `yy`, ...) is pasted as whole lines.
    pub linewise: bool,
}

impl Register {
    pub fn charwise(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            linewise: false,
        }
    }

    pub fn linewise(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            linewise: true,
        }
    }

    /// `self` followed by `other`. Mixing in a linewise part makes the result
    /// linewise, with the parts on separate lines.
    fn appended(&self, other: &Register) -> Register {
        if self.linewise || other.linewise {
            Register::linewise(format!("{}\n{}", self.text, other.text))
        } else {
            Register::charwise(format!("{}{}", self.text, other.text))
        }
    }
}

/// Why text is being stored, which decides whether `0` is updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Yank,
    Delete,
}

#[derive(Debug, Default, Clone)]
pub struct Registers {
    registers: HashMap<char, Register>,
}

impl Registers {
    /// Reads a register; `None` means the unnamed register.
    pub fn get(&self, name: Option<char>) -> Option<&Register> {
        let name = name.unwrap_or(UNNAMED).to_ascii_lowercase();
        self.registers.get(&name)
    }

    /// Sets register `name` directly, without touching the unnamed or yank
    /// registers (used for recording macros). Uppercase appends.
    pub fn set(&mut self, name: char, value: Register) -> Result<(), String> {
        match name {
            'a'..='z' | UNNAMED => {
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
        let name = name.unwrap_or(UNNAMED);
        let value = match name {
            BLACK_HOLE => return Ok(()),
            UNNAMED => {
                if kind == StoreKind::Yank {
                    self.registers.insert(YANK, value.clone());
                }
                value
            }
            'a'..='z' => {
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

    #[test]
    fn invalid_names_are_errors() {
        let mut r = Registers::default();
        let err = r.store(Some('%'), Register::charwise("x"), StoreKind::Yank);
        assert_eq!(err, Err("Invalid register: \"%".into()));
    }
}
