use std::fmt;

use serde::Deserialize;

/// The editor's modal state, as in Vim.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    /// Typing a `:` command on the command line.
    Command,
    /// An operator (`d`, `c`, `y`) is waiting for a motion or text object.
    #[serde(rename = "operator")]
    OperatorPending,
    /// Selecting text (`v` characterwise, `V` linewise). The kind of
    /// selection lives on the editor, so both share one keymap and style.
    Visual,
    /// A picker popup (buffers, commands) has focus.
    Picker,
}

impl Mode {
    pub const ALL: [Mode; 6] = [
        Mode::Normal,
        Mode::Insert,
        Mode::Command,
        Mode::OperatorPending,
        Mode::Visual,
        Mode::Picker,
    ];

    /// Lowercase identifier, used for config keys and theme scopes.
    pub const fn id(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Insert => "insert",
            Mode::Command => "command",
            Mode::OperatorPending => "operator",
            Mode::Visual => "visual",
            Mode::Picker => "picker",
        }
    }

    /// Whether unbound printable keys are typed as text in this mode.
    pub const fn accepts_text(self) -> bool {
        matches!(self, Mode::Insert | Mode::Command | Mode::Picker)
    }

    /// Whether a numeric count prefix (`3j`, `d2w`) can be typed.
    pub const fn accepts_count(self) -> bool {
        matches!(self, Mode::Normal | Mode::OperatorPending | Mode::Visual)
    }

    /// Whether a `"x` register prefix can be typed.
    pub const fn accepts_register(self) -> bool {
        matches!(self, Mode::Normal | Mode::Visual)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Operator-pending is a sub-state of normal mode; the pending keys
        // are shown separately in the status line, as with Vim's `showcmd`.
        match self {
            Mode::OperatorPending => f.write_str("NORMAL"),
            other => f.write_str(&other.id().to_uppercase()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_uppercase_label() {
        assert_eq!(Mode::Insert.to_string(), "INSERT");
        assert_eq!(Mode::OperatorPending.to_string(), "NORMAL");
    }

    #[test]
    fn text_and_count_acceptance() {
        assert!(Mode::Insert.accepts_text() && Mode::Command.accepts_text());
        assert!(!Mode::Normal.accepts_text() && !Mode::OperatorPending.accepts_text());
        assert!(Mode::Normal.accepts_count() && Mode::OperatorPending.accepts_count());
        assert!(Mode::Visual.accepts_count() && Mode::Visual.accepts_register());
        assert!(!Mode::Insert.accepts_count());
        assert!(!Mode::OperatorPending.accepts_register());
    }
}
