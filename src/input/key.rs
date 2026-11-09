//! A terminal-independent key representation and Vim-style key notation.
//!
//! Notation examples: `x`, `gg`, `<C-q>`, `<Esc>`, `<CR>`, `<S-Tab>`, `<A-x>`,
//! `<Space>`, `<lt>` (a literal `<`).

use std::fmt;

use crossterm::event::{self as ct, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    /// Marks the start of text pasted into the terminal, so macros and `.`
    /// can hold a paste: the keys up to [`KeyCode::PasteEnd`] are its text
    /// (`<CR>` for a line break, `<Tab>` for a tab), put in as one paste
    /// rather than typed. See [`paste_keys`].
    PasteStart,
    PasteEnd,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    /// Only used for non-character keys; for characters, shift is folded
    /// into the character itself (`A` rather than `<S-a>`).
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

impl KeyPress {
    pub fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self { code, modifiers }.normalized()
    }

    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, Modifiers::default())
    }

    pub fn ctrl(c: char) -> Self {
        let modifiers = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        Self::new(KeyCode::Char(c), modifiers)
    }

    /// Ensures equal keys compare equal regardless of how they were produced.
    fn normalized(mut self) -> Self {
        if let KeyCode::Char(c) = self.code {
            self.modifiers.shift = false;
            if self.modifiers.ctrl {
                self.code = KeyCode::Char(c.to_ascii_lowercase());
            }
        }
        self
    }

    /// The character this key would type, if it's unmodified text.
    pub fn as_text(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.modifiers.ctrl && !self.modifiers.alt => Some(c),
            _ => None,
        }
    }

    /// The control character for Ctrl plus a letter (`<C-w>` is `\x17`),
    /// so commands that take a character can tell `<C-r><C-w>` apart.
    pub fn as_control_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c)
                if self.modifiers.ctrl && !self.modifiers.alt && c.is_ascii_alphabetic() =>
            {
                Some(((c.to_ascii_lowercase() as u8) - b'a' + 1) as char)
            }
            _ => None,
        }
    }

    /// The character this key stands for inside a paste (see
    /// [`paste_keys`]).
    pub fn as_pasted(&self) -> Option<char> {
        match self.code {
            KeyCode::Enter => Some('\n'),
            KeyCode::Tab => Some('\t'),
            _ => self.as_text(),
        }
    }

    /// The key older terminals send for this one, for the keys they can't
    /// tell apart but the kitty keyboard protocol can: `<C-[>` is `<Esc>`,
    /// `<C-i>` is `<Tab>` and `<C-m>` is `<CR>`.
    pub fn legacy_equivalent(&self) -> Option<KeyPress> {
        let Modifiers { ctrl, alt, shift } = self.modifiers;
        if !ctrl || alt || shift {
            return None;
        }
        let code = match self.code {
            KeyCode::Char('[') => KeyCode::Esc,
            KeyCode::Char('i') => KeyCode::Tab,
            KeyCode::Char('m') => KeyCode::Enter,
            _ => return None,
        };
        Some(KeyPress::plain(code))
    }

    /// The digit value, if this is an unmodified digit key.
    pub fn as_digit(&self) -> Option<usize> {
        self.as_text()?.to_digit(10).map(|d| d as usize)
    }

    /// Converts a crossterm event. Returns `None` for releases and unsupported keys.
    pub fn from_event(event: ct::KeyEvent) -> Option<Self> {
        if event.kind == KeyEventKind::Release {
            return None;
        }
        let code = match event.code {
            ct::KeyCode::Char(c) => KeyCode::Char(c),
            ct::KeyCode::Enter => KeyCode::Enter,
            ct::KeyCode::Esc => KeyCode::Esc,
            ct::KeyCode::Backspace => KeyCode::Backspace,
            ct::KeyCode::Delete => KeyCode::Delete,
            ct::KeyCode::Tab => KeyCode::Tab,
            ct::KeyCode::BackTab => {
                let modifiers = Modifiers {
                    shift: true,
                    ..Modifiers::default()
                };
                return Some(Self::new(KeyCode::Tab, modifiers));
            }
            ct::KeyCode::Left => KeyCode::Left,
            ct::KeyCode::Right => KeyCode::Right,
            ct::KeyCode::Up => KeyCode::Up,
            ct::KeyCode::Down => KeyCode::Down,
            ct::KeyCode::Home => KeyCode::Home,
            ct::KeyCode::End => KeyCode::End,
            ct::KeyCode::PageUp => KeyCode::PageUp,
            ct::KeyCode::PageDown => KeyCode::PageDown,
            ct::KeyCode::F(n) => KeyCode::F(n),
            _ => return None,
        };
        let modifiers = Modifiers {
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
            shift: event.modifiers.contains(KeyModifiers::SHIFT),
        };
        // Without the kitty protocol, crossterm reads the bytes Ctrl+\ ] ^ _
        // send (0x1c-0x1f) as Ctrl+4 to Ctrl+7 (which share them).
        let code = match code {
            KeyCode::Char(c @ '4'..='7') if modifiers.ctrl => {
                KeyCode::Char(['\\', ']', '^', '_'][(c as u8 - b'4') as usize])
            }
            code => code,
        };
        Some(Self::new(code, modifiers))
    }
}

/// `text` pasted into the terminal, as keys: `<PasteStart>`, a key per
/// character (line breaks as `<CR>`, tabs as `<Tab>`), then `<PasteEnd>`.
pub fn paste_keys(text: &str) -> Vec<KeyPress> {
    let text = text.chars().map(|c| {
        KeyPress::plain(match c {
            '\n' => KeyCode::Enter,
            '\t' => KeyCode::Tab,
            c => KeyCode::Char(c),
        })
    });
    std::iter::once(KeyPress::plain(KeyCode::PasteStart))
        .chain(text)
        .chain(std::iter::once(KeyPress::plain(KeyCode::PasteEnd)))
        .collect()
}

/// Parses a key sequence like `gg` or `<C-w>v` into key presses.
pub fn parse_keys(notation: &str) -> Result<Vec<KeyPress>, String> {
    let mut keys = Vec::new();
    let mut rest = notation;

    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
            && end > 1
        {
            keys.push(parse_bracketed(&rest[1..end])?);
            rest = &rest[end + 1..];
        } else {
            keys.push(KeyPress::plain(KeyCode::Char(c)));
            rest = &rest[c.len_utf8()..];
        }
    }

    if keys.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(keys)
}

/// Parses the inside of `<...>`, e.g. `C-q`, `Esc`, `S-Tab`.
fn parse_bracketed(inner: &str) -> Result<KeyPress, String> {
    let mut modifiers = Modifiers::default();
    let mut name = inner;

    while name.len() > 2 && name.as_bytes()[1] == b'-' {
        match name.as_bytes()[0].to_ascii_uppercase() {
            b'C' => modifiers.ctrl = true,
            b'A' | b'M' => modifiers.alt = true,
            b'S' => modifiers.shift = true,
            _ => return Err(format!("unknown modifier in <{inner}>")),
        }
        name = &name[2..];
    }

    let code = match name.to_ascii_lowercase().as_str() {
        "esc" => KeyCode::Esc,
        "cr" | "enter" | "return" => KeyCode::Enter,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "lt" => KeyCode::Char('<'),
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "pastestart" => KeyCode::PasteStart,
        "pasteend" => KeyCode::PasteEnd,
        lower => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => match lower.strip_prefix('f').and_then(|n| n.parse().ok()) {
                    Some(n) => KeyCode::F(n),
                    None => return Err(format!("unknown key <{inner}>")),
                },
            }
        }
    };

    Ok(KeyPress::new(code, modifiers))
}

impl KeyPress {
    /// Whether terminals only send this key apart from others with the
    /// kitty keyboard protocol (`<C-`>` is otherwise `<C-Space>`, `<C-CR>`
    /// is `<CR>`).
    pub fn needs_enhanced_keyboard(&self) -> bool {
        let m = self.modifiers;
        match self.code {
            KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Esc => {
                m.ctrl || (m.shift && self.code != KeyCode::Tab)
            }
            KeyCode::Char(c) if m.ctrl => {
                !(c.is_ascii_alphabetic() || matches!(c, '@' | '[' | '\\' | ']' | '^' | '_' | ' '))
            }
            _ => false,
        }
    }
}

impl fmt::Display for KeyPress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char('<') => "lt".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "CR".into(),
            KeyCode::Esc => "Esc".into(),
            KeyCode::Backspace => "BS".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::Left => "Left".into(),
            KeyCode::Right => "Right".into(),
            KeyCode::Up => "Up".into(),
            KeyCode::Down => "Down".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PageUp".into(),
            KeyCode::PageDown => "PageDown".into(),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::PasteStart => "PasteStart".into(),
            KeyCode::PasteEnd => "PasteEnd".into(),
        };

        let Modifiers { ctrl, alt, shift } = self.modifiers;
        let is_plain_char = matches!(self.code, KeyCode::Char(c) if c != ' ' && c != '<');
        if is_plain_char && !ctrl && !alt {
            return f.write_str(&name);
        }

        f.write_str("<")?;
        if ctrl {
            f.write_str("C-")?;
        }
        if alt {
            f.write_str("A-")?;
        }
        if shift {
            f.write_str("S-")?;
        }
        write!(f, "{name}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_key(c: char) -> KeyPress {
        KeyPress::plain(KeyCode::Char(c))
    }

    #[test]
    fn parses_plain_sequences() {
        assert_eq!(
            parse_keys("gg").unwrap(),
            vec![char_key('g'), char_key('g')]
        );
    }

    #[test]
    fn parses_modifiers() {
        assert_eq!(parse_keys("<C-q>").unwrap(), vec![KeyPress::ctrl('q')]);
        let alt_x = parse_keys("<A-x>").unwrap()[0];
        assert!(alt_x.modifiers.alt);
        let shift_tab = parse_keys("<S-Tab>").unwrap()[0];
        assert_eq!(shift_tab.code, KeyCode::Tab);
        assert!(shift_tab.modifiers.shift);
    }

    #[test]
    fn ctrl_chars_are_case_insensitive() {
        assert_eq!(parse_keys("<C-Q>").unwrap(), parse_keys("<c-q>").unwrap());
    }

    #[test]
    fn parses_named_keys_case_insensitively() {
        assert_eq!(parse_keys("<esc>").unwrap()[0].code, KeyCode::Esc);
        assert_eq!(parse_keys("<CR>").unwrap()[0].code, KeyCode::Enter);
        assert_eq!(parse_keys("<Space>").unwrap(), vec![char_key(' ')]);
        assert_eq!(parse_keys("<F12>").unwrap()[0].code, KeyCode::F(12));
    }

    #[test]
    fn parses_mixed_sequences() {
        let keys = parse_keys("<C-w>v").unwrap();
        assert_eq!(keys, vec![KeyPress::ctrl('w'), char_key('v')]);
    }

    #[test]
    fn lone_angle_bracket_is_literal() {
        assert_eq!(parse_keys("<").unwrap(), vec![char_key('<')]);
        assert_eq!(
            parse_keys("<>").unwrap(),
            vec![char_key('<'), char_key('>')]
        );
        assert_eq!(parse_keys("<lt>").unwrap(), vec![char_key('<')]);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(parse_keys("<Nope>").is_err());
        assert!(parse_keys("<X-a>").is_err());
        assert!(parse_keys("").is_err());
    }

    #[test]
    fn display_round_trips() {
        for notation in [
            "g", "<C-q>", "<Esc>", "<Space>", "<S-Tab>", "<A-x>", "<lt>", "<F5>",
        ] {
            let key = parse_keys(notation).unwrap()[0];
            assert_eq!(key.to_string(), notation);
        }
    }

    #[test]
    fn shift_is_folded_into_chars() {
        let shifted = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(KeyPress::new(KeyCode::Char('A'), shifted), char_key('A'));
    }

    #[test]
    fn converts_crossterm_events() {
        let event = ct::KeyEvent::new(ct::KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert_eq!(KeyPress::from_event(event), Some(KeyPress::ctrl('q')));

        let legacy = ct::KeyEvent::new(ct::KeyCode::Char('4'), KeyModifiers::CONTROL);
        assert_eq!(
            KeyPress::from_event(legacy)
                .map(|k| k.to_string())
                .as_deref(),
            Some("<C-\\>"),
            "the byte Ctrl+\\ sends"
        );
        let mut release = ct::KeyEvent::new(ct::KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(KeyPress::from_event(release), None);
    }

    #[test]
    fn text_and_digits() {
        assert_eq!(char_key('a').as_text(), Some('a'));
        assert_eq!(KeyPress::ctrl('a').as_text(), None);
        assert_eq!(char_key('7').as_digit(), Some(7));
        assert_eq!(char_key('x').as_digit(), None);
    }

    #[test]
    fn pastes_round_trip_through_notation() {
        let keys = paste_keys("a <b>\n\tc");
        let notation: String = keys.iter().map(ToString::to_string).collect();
        assert_eq!(notation, "<PasteStart>a<Space><lt>b><CR><Tab>c<PasteEnd>");
        assert_eq!(parse_keys(&notation).unwrap(), keys);
        let text: String = keys.iter().filter_map(KeyPress::as_pasted).collect();
        assert_eq!(text, "a <b>\n\tc");
    }

    #[test]
    fn keys_older_terminals_merge() {
        let legacy = |k: &str| parse_keys(k).unwrap()[0].legacy_equivalent();
        assert_eq!(legacy("<C-[>"), Some(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(legacy("<C-i>"), Some(KeyPress::plain(KeyCode::Tab)));
        assert_eq!(legacy("<C-m>"), Some(KeyPress::plain(KeyCode::Enter)));
        assert_eq!(legacy("<C-a>"), None);
        assert_eq!(legacy("<C-A-i>"), None);
        assert_eq!(legacy("i"), None);
    }
}
