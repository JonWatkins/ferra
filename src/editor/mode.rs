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
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Normal, Mode::Insert, Mode::Command];

    /// Lowercase identifier, used for config keys and theme scopes.
    pub const fn id(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Insert => "insert",
            Mode::Command => "command",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id().to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_uppercase_id() {
        assert_eq!(Mode::Insert.to_string(), "INSERT");
    }
}
