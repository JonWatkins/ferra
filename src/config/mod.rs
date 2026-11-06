//! Configuration loading.
//!
//! The defaults live in `default.toml` (embedded at compile time) and are the
//! single source of truth. A user config is deep-merged on top, so users only
//! write what they want to change.

mod options;
mod theme;

pub use options::{
    CursorShape, CursorShapes, EditorOptions, FilePickerOptions, LineNumbers, SearchCase,
    SearchOptions,
};
pub use theme::{StyleSpec, Theme};

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::Deserialize;
use toml::{Table, Value};

use std::collections::HashMap;

use crate::events::EventKind;
use crate::input::Keymap;

const DEFAULT_CONFIG: &str = include_str!("default.toml");
const APP_DIR: &str = "terminal-editor";

static DEFAULT: LazyLock<Config> =
    LazyLock::new(|| Config::from_toml("").expect("embedded default config must be valid"));

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub editor: EditorOptions,
    pub theme: Theme,
    pub keys: Keymap,
    /// Commands to run when events fire: `before-save = ["trim_trailing_whitespace"]`.
    pub hooks: HashMap<EventKind, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        DEFAULT.clone()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            ConfigError::Invalid(msg) => write!(f, "invalid config: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Parses user config text and merges it over the defaults.
    pub fn from_toml(user: &str) -> Result<Self, ConfigError> {
        let invalid = |e: &dyn fmt::Display| ConfigError::Invalid(e.to_string().trim().to_string());

        let mut merged: Table = DEFAULT_CONFIG.parse().map_err(|e| invalid(&e))?;
        let user: Table = user.parse().map_err(|e| invalid(&e))?;
        merge(&mut merged, user);

        Value::Table(merged).try_into().map_err(|e| invalid(&e))
    }

    /// Loads the config from `path`. A missing file yields the defaults.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Loads the config from the default location, if there is one.
    pub fn load() -> Result<Self, ConfigError> {
        match default_path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Self::default()),
        }
    }

    /// The embedded default config, useful as a starting point for users.
    pub fn default_toml() -> &'static str {
        DEFAULT_CONFIG
    }
}

/// The directory holding `config.toml`, `init.lua` and `plugins/`.
pub fn config_dir() -> Option<PathBuf> {
    default_path().and_then(|p| p.parent().map(Path::to_path_buf))
}

/// `$XDG_CONFIG_HOME/terminal-editor/config.toml`, falling back to
/// `~/.config/terminal-editor/config.toml`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join(APP_DIR).join("config.toml"))
}

/// Recursively merges `overlay` into `base`. Tables merge; everything else replaces.
fn merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(base_table)), Value::Table(overlay_table)) => {
                merge(base_table, overlay_table);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_are_valid() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.editor.tab_width, 4);
    }

    #[test]
    fn merge_overrides_nested_values_only() {
        let mut base: Table = "[a]\nx = 1\ny = 2".parse().unwrap();
        merge(&mut base, "[a]\ny = 3\nz = 4".parse().unwrap());
        assert_eq!(base.to_string(), "[a]\nx = 1\ny = 3\nz = 4\n");
    }

    #[test]
    fn missing_file_yields_defaults() {
        let config = Config::load_from(Path::new("/definitely/not/here.toml")).unwrap();
        assert_eq!(config.editor, EditorOptions::default());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = Config::from_toml("[editor]\ntab-wdith = 2").unwrap_err();
        assert!(err.to_string().contains("tab-wdith"), "{err}");
    }

    #[test]
    fn syntax_errors_are_reported() {
        assert!(matches!(
            Config::from_toml("[editor"),
            Err(ConfigError::Invalid(_))
        ));
    }
}
