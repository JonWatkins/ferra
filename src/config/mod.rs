//! Configuration loading.
//!
//! The defaults live in `default.toml` (embedded at compile time) and are the
//! single source of truth. A user config is deep-merged on top, so users only
//! write what they want to change.

pub mod edit;
mod languages;
mod options;
pub mod schema;
pub mod set;
mod theme;

pub use languages::{CommentTokens, FileType, Languages, ServerConfig};
pub use options::{
    AutoWrap, CursorShape, CursorShapes, EditorOptions, FallbackEncoding, FilePickerOptions, Icons,
    KeymapPreset, LineNumbers, SearchCase, SearchOptions, StatusElement, StatuslineLayout,
    StatuslineMode, StatuslineStyle,
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

pub(crate) const DEFAULT_CONFIG: &str = include_str!("default.toml");
const APP_DIR: &str = "ferra";

static DEFAULT: LazyLock<Config> =
    LazyLock::new(|| Config::from_toml("").expect("embedded default config must be valid"));

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub editor: EditorOptions,
    /// Which files are in which language.
    pub languages: Languages,
    pub theme: Theme,
    pub keys: Keymap,
    /// Commands to run when events fire: `before-save = ["trim_trailing_whitespace"]`.
    pub hooks: HashMap<EventKind, Vec<String>>,
    /// The user's own `[keys]`, laid over whichever keymap is chosen (so
    /// switching `keymap` keeps them).
    #[serde(skip)]
    pub user_keys: Table,
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
        // The languages that are packs now, for ferra's own tests.
        #[cfg(feature = "test-languages")]
        merge(
            &mut merged,
            include_str!("test_languages.toml")
                .parse()
                .map_err(|e| invalid(&e))?,
        );
        let user: Table = user.parse().map_err(|e| invalid(&e))?;
        let user_keys = match user.get("keys") {
            Some(Value::Table(keys)) => keys.clone(),
            _ => Table::new(),
        };
        merge(&mut merged, user);
        // The keys: the chosen keymap's, then the user's over them.
        let preset = match merged
            .get("editor")
            .and_then(|e| e.get("keymap"))
            .and_then(Value::as_str)
        {
            Some("vscode") => KeymapPreset::Vscode,
            _ => KeymapPreset::Vim,
        };
        merged.insert(
            "keys".into(),
            Value::Table(layered_keys(preset, &user_keys).map_err(|e| invalid(&e))?),
        );

        let mut config: Config = Value::Table(merged).try_into().map_err(|e| invalid(&e))?;
        config.user_keys = user_keys;
        config
            .languages
            .check_options(&config.editor)
            .map_err(|e| invalid(&e))?;
        Ok(config)
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

/// `$XDG_CONFIG_HOME/ferra/config.toml`, falling back to
/// `~/.config/ferra/config.toml`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join(APP_DIR).join("config.toml"))
}

/// The keys for `preset`, with `user`'s (`Config::user_keys`) over them:
/// for switching `keymap` while running.
pub fn keymap_for(preset: KeymapPreset, user: &Table) -> Result<Keymap, String> {
    let keys = layered_keys(preset, user)?;
    Value::Table(keys)
        .try_into()
        .map_err(|e: toml::de::Error| e.message().trim().to_string())
}

/// The keymaps' editing modes: Vim's (normal, operator, visual, insert),
/// and VS Code's (insert, and visual inheriting it).
const VIM_KEYMAP: &str = include_str!("keymaps/vim.toml");
const VSCODE_KEYMAP: &str = include_str!("keymaps/vscode.toml");

/// The `[keys]` for `preset`: the ones every keymap shares (default.toml's),
/// the preset's over them, then `user`'s.
fn layered_keys(preset: KeymapPreset, user: &Table) -> Result<Table, String> {
    let defaults: Table = DEFAULT_CONFIG
        .parse()
        .map_err(|e: toml::de::Error| e.to_string())?;
    let mut keys = match defaults.get("keys") {
        Some(Value::Table(keys)) => keys.clone(),
        _ => Table::new(),
    };
    let (name, text) = match preset {
        KeymapPreset::Vim => ("vim", VIM_KEYMAP),
        KeymapPreset::Vscode => ("vscode", VSCODE_KEYMAP),
    };
    {
        let mut layer: Table = text
            .parse()
            .map_err(|e: toml::de::Error| format!("{name} keymap: {e}"))?;
        if let Some(Value::Table(more)) = layer.remove("keys") {
            merge(&mut keys, more);
        }
        // A mode that starts with another's keys, its own over them.
        if let Some(Value::Table(inherit)) = layer.remove("inherit") {
            for (mode, from) in inherit {
                let Some(from) = from.as_str() else { continue };
                let mut base = match keys.get(from) {
                    Some(Value::Table(base)) => base.clone(),
                    _ => Table::new(),
                };
                if let Some(Value::Table(own)) = keys.remove(&mode) {
                    merge(&mut base, own);
                }
                keys.insert(mode, Value::Table(base));
            }
        }
    }
    merge(&mut keys, user.clone());
    Ok(keys)
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
        assert_eq!(config.editor, Config::default().editor);
        assert_eq!(config.editor.keymap, KeymapPreset::Vscode, "VS Code's keys");
    }

    /// Two notations for one key (`<A-S-f>` is `<A-F>`) in a mode would
    /// leave which command it runs to chance.
    #[test]
    fn no_key_is_bound_twice() {
        use std::collections::HashMap;
        for preset in [KeymapPreset::Vim, KeymapPreset::Vscode] {
            let keys = layered_keys(preset, &Table::new()).unwrap();
            for (mode, bindings) in keys {
                let Value::Table(bindings) = bindings else {
                    continue;
                };
                let mut seen: HashMap<Vec<crate::input::KeyPress>, (String, String)> =
                    HashMap::new();
                for (notation, command) in bindings {
                    let command = command.as_str().unwrap_or_default().to_string();
                    let parsed = crate::input::parse_keys(&notation).unwrap();
                    if let Some((other, other_command)) =
                        seen.insert(parsed, (notation.clone(), command.clone()))
                        && other_command != command
                    {
                        panic!(
                            "{preset:?} [keys.{mode}]: {notation} ({command}) and {other} ({other_command}) are one key"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn keymaps_are_layered_with_the_users_keys_on_top() {
        use crate::editor::Mode;
        use crate::input::{Lookup, parse_keys};
        let key = |s: &str| parse_keys(s).unwrap();
        let user = "[keys.insert]\n\"<C-e>\" = \"write\"\n[keys.sidebar]\n\"Q\" = \"quit\"\n";

        let vim = Config::from_toml(&format!("[editor]\nkeymap = \"vim\"\n{user}")).unwrap();
        assert_eq!(
            vim.keys.lookup(Mode::Normal, &key("d")),
            Lookup::Command("operator_delete")
        );
        assert_eq!(
            vim.keys.lookup(Mode::Insert, &key("<Esc>")),
            Lookup::Command("normal_mode")
        );
        assert_eq!(
            vim.keys.lookup(Mode::Insert, &key("<C-e>")),
            Lookup::Command("write")
        );

        let vscode = Config::from_toml(user).unwrap();
        assert_eq!(
            vscode.keys.lookup(Mode::Insert, &key("<C-s>")),
            Lookup::Command("write")
        );
        assert_eq!(
            vscode.keys.lookup(Mode::Insert, &key("<Esc>")),
            Lookup::Command("escape")
        );
        assert_eq!(
            vscode.keys.lookup(Mode::Insert, &key("<C-e>")),
            Lookup::Command("write"),
            "the user's over it"
        );
        // A selection has typing's keys.
        assert_eq!(
            vscode.keys.lookup(Mode::Visual, &key("<C-c>")),
            Lookup::Command("copy")
        );
        // No normal mode; the shared modes have their keys, and the user's.
        assert_eq!(vscode.keys.lookup(Mode::Normal, &key("d")), Lookup::NoMatch);
        assert_eq!(
            vscode.keys.lookup(Mode::Sidebar, &key("j")),
            Lookup::Command("sidebar_down")
        );
        assert_eq!(
            vscode.keys.lookup(Mode::Sidebar, &key("Q")),
            Lookup::Command("quit")
        );

        // Built again for the other keymap, the user's keys come along.
        let again = keymap_for(KeymapPreset::Vim, &vscode.user_keys).unwrap();
        assert_eq!(
            again.lookup(Mode::Normal, &key("d")),
            Lookup::Command("operator_delete")
        );
        assert_eq!(
            again.lookup(Mode::Sidebar, &key("Q")),
            Lookup::Command("quit")
        );
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

    #[test]
    fn statusline_layouts_are_checked() {
        let config = Config::from_toml(
            "[editor.statusline-layout]\nleft = [\"mode\"]\nright = [\"position\"]",
        )
        .unwrap();
        assert_eq!(config.editor.statusline_layout.left, [StatusElement::Mode]);
        assert!(config.editor.statusline_layout.center.is_empty());

        let error = Config::from_toml("[editor.statusline-layout]\nleft = [\"clock\"]")
            .unwrap_err()
            .to_string();
        assert!(error.contains("clock"), "{error}");
    }
}
