//! The settings the settings editor shows: every `[editor]` option, read
//! from the embedded `default.toml`. Its comment there is the setting's
//! description, its value the default (and so its kind), and the choices
//! of one that takes names (`line-numbers`: "absolute", "relative",
//! "none") come from asking the config's own checks, so they can't drift
//! from the code.

use std::sync::LazyLock;

use toml::Value;

use super::{DEFAULT_CONFIG, EditorOptions, set};

/// What kind of value a setting takes, so the editor shows the right
/// control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Number,
    /// One of these names.
    Choice(Vec<String>),
    /// Any text (checked as config is when it's set).
    Text,
    /// A list (the status line's elements): edited in `config.toml`.
    List,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    /// Its name under `[editor]`: `tab-width`, `search.case`.
    pub path: String,
    /// `Tab Width`, `Case`.
    pub title: String,
    pub category: &'static str,
    pub description: String,
    pub default: Value,
    pub kind: Kind,
}

/// The categories, in the order they're listed.
pub const CATEGORIES: &[&str] = &[
    "Editing",
    "Display",
    "Workbench",
    "Search",
    "Files",
    "Git",
    "Language servers",
    "Keyboard and mouse",
    "Terminal",
    "Cursor shape",
    "Plugins",
];

/// Which category each option is in: a whole table (`search`), or a name.
fn category_of(path: &str) -> &'static str {
    let table = path.split('.').next().unwrap_or(path);
    match table {
        "tab-width" | "expand-tab" | "detect-indent" | "auto-indent" | "text-width"
        | "auto-wrap" | "format-on-save" => "Editing",
        "line-numbers"
        | "wrap"
        | "wrap-indent"
        | "wrap-marker"
        | "cursorline"
        | "scroll-off"
        | "syntax-highlighting" => "Display",
        "menu-bar" | "activity-bar" | "icons" | "sidebar-width" | "statusline"
        | "statusline-style" | "statusline-layout" => "Workbench",
        "search" => "Search",
        "file-picker" | "fallback-encoding" | "watch-files" => "Files",
        "git" | "git-blame" => "Git",
        "lsp" | "auto-complete" | "highlight-references" | "inlay-hints" => "Language servers",
        "keymap" | "mouse" | "kitty-keyboard" | "key-timeout" | "clipboard"
        | "clipboard-provider" => "Keyboard and mouse",
        "terminal-height" | "shell" => "Terminal",
        "cursor-shape" => "Cursor shape",
        "plugin-index" | "plugin-suggestions" => "Plugins",
        _ => "Other",
    }
}

/// Tables whose category is named for them: their settings are titled by
/// their own names alone (`Case`, not `Search: Case`).
const CATEGORY_TABLES: &[&str] = &["search", "cursor-shape"];

static SETTINGS: LazyLock<Vec<Setting>> = LazyLock::new(read_settings);

/// Every setting, by category (in `CATEGORIES` order), then as in
/// `default.toml`.
pub fn settings() -> &'static [Setting] {
    &SETTINGS
}

fn read_settings() -> Vec<Setting> {
    let doc: toml_edit::DocumentMut = DEFAULT_CONFIG.parse().expect("default.toml is valid");
    let editor = doc["editor"].as_table().expect("default.toml has [editor]");
    let mut out = Vec::new();
    collect(editor, "", None, &mut out);
    out.sort_by_key(|s| {
        CATEGORIES
            .iter()
            .position(|c| *c == s.category)
            .unwrap_or(CATEGORIES.len())
    });
    out
}

/// The settings in `table` (named `prefix`), with the table's own comment
/// for those that have none.
fn collect(table: &toml_edit::Table, prefix: &str, fallback: Option<&str>, out: &mut Vec<Setting>) {
    for (key, item) in table.iter() {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        let comment = table
            .key(key)
            .and_then(|k| k.leaf_decor().prefix())
            .and_then(|p| p.as_str())
            .map(comment_text)
            .unwrap_or_default();
        match item {
            toml_edit::Item::Table(sub) => {
                let own = sub
                    .decor()
                    .prefix()
                    .and_then(|p| p.as_str())
                    .map(comment_text)
                    .unwrap_or_default();
                collect(sub, &path, Some(&own), out);
            }
            toml_edit::Item::Value(value) => {
                let bare = value.clone().decorated("", "");
                let default = format!("v = {bare}")
                    .parse::<toml::Table>()
                    .expect("default.toml's values parse")["v"]
                    .clone();
                let description = if comment.is_empty() {
                    fallback.unwrap_or_default().to_string()
                } else {
                    comment
                };
                out.push(Setting {
                    title: title_of(&path),
                    category: category_of(&path),
                    kind: kind_of(&path, &default),
                    description,
                    default,
                    path,
                });
            }
            _ => {}
        }
    }
}

/// A key's comment as one paragraph: the last block of `#` lines before it.
fn comment_text(prefix: &str) -> String {
    let mut block: Vec<&str> = Vec::new();
    for line in prefix.lines() {
        let line = line.trim();
        match line.strip_prefix('#') {
            Some(text) => block.push(text.strip_prefix(' ').unwrap_or(text).trim_end()),
            None if line.is_empty() => block.clear(),
            None => {}
        }
    }
    block.join(" ").trim().to_string()
}

/// `tab-width` → `Tab Width`; `statusline-layout.left` → `Status Line
/// Layout: Left`; `search.case` → `Case`.
fn title_of(path: &str) -> String {
    let mut parts: Vec<&str> = path.split('.').collect();
    if parts.len() > 1 && CATEGORY_TABLES.contains(&parts[0]) {
        parts.remove(0);
    }
    parts
        .iter()
        .map(|part| {
            part.split('-')
                .map(|word| match word {
                    "lsp" => "LSP".to_string(),
                    "statusline" => "Status Line".to_string(),
                    "cursorline" => "Cursor Line".to_string(),
                    word => {
                        let mut chars = word.chars();
                        chars.next().map_or_else(String::new, |c| {
                            c.to_uppercase().collect::<String>() + chars.as_str()
                        })
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(": ")
}

fn kind_of(path: &str, default: &Value) -> Kind {
    match default {
        Value::Boolean(_) => Kind::Bool,
        Value::Integer(_) => Kind::Number,
        Value::Array(_) => Kind::List,
        Value::String(_) => choices(path).map_or(Kind::Text, Kind::Choice),
        _ => Kind::Text,
    }
}

/// The names option `path` takes, if it takes only some: from the error
/// for one it doesn't ("expected one of `a`, `b`").
fn choices(path: &str) -> Option<Vec<String>> {
    let mut options = EditorOptions::default();
    let error = set::set_value(&mut options, path, Value::String("\u{1}".into())).err()?;
    let expected = &error[error.find("expected")?..];
    let names: Vec<String> = expected
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    (!names.is_empty()).then_some(names)
}

/// The setting at `path`.
pub fn find(path: &str) -> Option<&'static Setting> {
    settings().iter().find(|s| s.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_option_is_a_setting_in_a_category() {
        let all = settings();
        assert!(all.len() > 50, "{}", all.len());
        let uncategorised: Vec<&str> = all
            .iter()
            .filter(|s| s.category == "Other")
            .map(|s| s.path.as_str())
            .collect();
        assert!(uncategorised.is_empty(), "no category: {uncategorised:?}");
        let undescribed: Vec<&str> = all
            .iter()
            .filter(|s| s.description.is_empty())
            .map(|s| s.path.as_str())
            .collect();
        assert!(
            undescribed.is_empty(),
            "no comment in default.toml: {undescribed:?}"
        );
        // Each option once, and each one the options know.
        let mut paths: Vec<&str> = all.iter().map(|s| s.path.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), all.len());
        let options = EditorOptions::default();
        for path in paths {
            assert!(set::get_value(&options, path).is_some(), "{path}");
        }
    }

    #[test]
    fn kinds_titles_and_descriptions() {
        let tab = find("tab-width").unwrap();
        assert_eq!(tab.title, "Tab Width");
        assert_eq!(tab.kind, Kind::Number);
        assert_eq!(tab.default, Value::Integer(4));
        assert_eq!(tab.category, "Editing");
        assert_eq!(
            tab.description,
            "Width of a tab stop, and of one indent level."
        );

        let numbers = find("line-numbers").unwrap();
        assert_eq!(
            numbers.kind,
            Kind::Choice(vec!["absolute".into(), "relative".into(), "none".into()])
        );
        assert_eq!(find("expand-tab").unwrap().kind, Kind::Bool);
        assert_eq!(find("wrap-marker").unwrap().kind, Kind::Text);
        assert_eq!(find("shell").unwrap().kind, Kind::Text);
        assert_eq!(find("lsp").unwrap().title, "LSP");

        let case = find("search.case").unwrap();
        assert_eq!(case.title, "Case");
        assert_eq!(case.category, "Search");
        assert_eq!(
            case.kind.clone(),
            Kind::Choice(vec![
                "smart".into(),
                "sensitive".into(),
                "insensitive".into()
            ])
        );

        let left = find("statusline-layout.left").unwrap();
        assert_eq!(left.title, "Status Line Layout: Left");
        assert_eq!(left.kind, Kind::List);
        assert!(
            left.description
                .starts_with("What the active window's status line shows")
        );
    }

    #[test]
    fn listed_by_category() {
        let order: Vec<usize> = settings()
            .iter()
            .map(|s| CATEGORIES.iter().position(|c| *c == s.category).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]));
    }
}
