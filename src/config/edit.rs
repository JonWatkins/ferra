//! Changing one option or key binding in the user's `config.toml` in
//! place, as the settings and keybinding editors do: the rest of the file
//! (comments, order, spacing) is left as it was.

use std::path::Path;

use toml::Value;
use toml_edit::{DocumentMut, Item, Table};

/// Sets `[editor]` option `path` (`tab-width`, `search.case`) to `value`
/// in the config file at `file` (made if it isn't there), or, with `None`,
/// takes it out so the default applies.
pub fn write_option(file: &Path, path: &str, value: Option<&Value>) -> Result<(), String> {
    let names: Vec<&str> = std::iter::once("editor").chain(path.split('.')).collect();
    write_at(file, &names, value)
}

/// Binds `keys` (notation) in `[keys.<mode>]` to `command` in the config
/// file at `file` ("" unbinds a default), or with `None` takes the line out.
pub fn write_key(file: &Path, mode: &str, keys: &str, command: Option<&str>) -> Result<(), String> {
    let value = command.map(|c| Value::String(c.to_string()));
    write_at(file, &["keys", mode, keys], value.as_ref())
}

/// The `[keys]` table of the config file at `file` (empty without one).
pub fn read_keys(file: &Path) -> Result<toml::Table, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(toml::Table::new()),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| format!("{}: {}", file.display(), e.message()))?;
    Ok(match table.get("keys") {
        Some(Value::Table(keys)) => keys.clone(),
        _ => toml::Table::new(),
    })
}

fn write_at(file: &Path, names: &[&str], value: Option<&Value>) -> Result<(), String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let updated = with_path(&text, names, value).map_err(|e| format!("{}: {e}", file.display()))?;
    if updated == text {
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::files::write(file, updated.as_bytes())
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", file.display()))
}

/// `text` (a config file) with `[editor]` option `path` set to `value`, or
/// taken out with `None`.
pub fn with_option(text: &str, path: &str, value: Option<&Value>) -> Result<String, String> {
    let names: Vec<&str> = std::iter::once("editor").chain(path.split('.')).collect();
    with_path(text, &names, value)
}

/// `text` with the value at `names` (tables, then the key) set, or taken
/// out with `None`.
fn with_path(text: &str, names: &[&str], value: Option<&Value>) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("can't change it: {}", e.message()))?;
    let (leaf, names) = names.split_last().expect("a name");
    let leaf = *leaf;
    match value {
        Some(value) => {
            let mut table = doc.as_table_mut();
            for (depth, name) in names.iter().enumerate() {
                let item = table.entry(name).or_insert_with(|| {
                    let mut new = Table::new();
                    // `[editor.search]` alone, without an empty `[editor]`.
                    new.set_implicit(depth + 1 < names.len());
                    Item::Table(new)
                });
                table = match item {
                    Item::Table(t) => t,
                    _ => return Err(format!("{name} isn't a table")),
                };
            }
            let new = to_edit(value)?;
            match table.get_mut(leaf) {
                // Keeping the comment before it and after it.
                Some(Item::Value(old)) => {
                    let decor = old.decor().clone();
                    *old = new;
                    *old.decor_mut() = decor;
                }
                _ => {
                    table.insert(leaf, Item::Value(new));
                }
            }
        }
        None => {
            let mut table = doc.as_table_mut();
            let mut chain: Vec<String> = Vec::new();
            for name in names {
                match table.get_mut(name) {
                    Some(Item::Table(t)) => table = t,
                    _ => return Ok(text.to_string()),
                }
                chain.push((*name).to_string());
            }
            table.remove(leaf);
            // A table left empty by it goes too (but not one with a
            // comment of its own).
            prune(doc.as_table_mut(), &chain);
        }
    }
    Ok(doc.to_string())
}

/// Removes the tables along `chain` (deepest first) that are left empty.
fn prune(root: &mut Table, chain: &[String]) {
    if chain.is_empty() {
        return;
    }
    let (last, parents) = chain.split_last().expect("not empty");
    let mut table = &mut *root;
    for name in parents {
        match table.get_mut(name) {
            Some(Item::Table(t)) => table = t,
            _ => return,
        }
    }
    let empty = table.get(last).and_then(Item::as_table).is_some_and(|t| {
        t.is_empty()
            && t.decor()
                .prefix()
                .and_then(|p| p.as_str())
                .is_none_or(|p| !p.contains('#'))
    });
    if empty {
        table.remove(last);
        prune(root, parents);
    }
}

/// A `toml` value as a `toml_edit` one.
fn to_edit(value: &Value) -> Result<toml_edit::Value, String> {
    let mut table = toml::Table::new();
    table.insert("v".into(), value.clone());
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    doc["v"]
        .as_value()
        .map(|v| v.clone().decorated(" ", ""))
        .ok_or_else(|| "not a value".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str, path: &str, value: Value) -> String {
        with_option(text, path, Some(&value)).unwrap()
    }

    #[test]
    fn into_an_empty_file() {
        assert_eq!(
            set("", "tab-width", Value::Integer(2)),
            "[editor]\ntab-width = 2\n"
        );
        assert_eq!(
            set("", "search.case", Value::String("sensitive".into())),
            "[editor.search]\ncase = \"sensitive\"\n"
        );
    }

    #[test]
    fn comments_and_the_rest_stay() {
        let text = "# My config\n[editor]\n# Two, like the team's.\ntab-width = 4 # was 8\nwrap = false\n\n[theme]\n\"ui.text\" = \"white\"\n";
        assert_eq!(
            set(text, "tab-width", Value::Integer(2)),
            "# My config\n[editor]\n# Two, like the team's.\ntab-width = 2 # was 8\nwrap = false\n\n[theme]\n\"ui.text\" = \"white\"\n"
        );
        // A new one goes at the end of [editor].
        let added = set(text, "cursorline", Value::Boolean(false));
        assert!(
            added.contains("wrap = false\ncursorline = false\n"),
            "{added}"
        );
        assert!(added.contains("[theme]"));
    }

    #[test]
    fn taking_one_out() {
        let text = "[editor]\ntab-width = 2\nwrap = false\n";
        assert_eq!(
            with_option(text, "tab-width", None).unwrap(),
            "[editor]\nwrap = false\n"
        );
        // Not there: nothing changes.
        assert_eq!(with_option(text, "cursorline", None).unwrap(), text);
        // A table it empties goes too.
        let text = "[editor]\nwrap = false\n\n[editor.search]\ncase = \"sensitive\"\n";
        assert_eq!(
            with_option(text, "search.case", None).unwrap(),
            "[editor]\nwrap = false\n"
        );
        assert_eq!(
            with_option("[editor]\nwrap = false\n", "wrap", None).unwrap(),
            ""
        );
        // One with its own comment stays.
        let text = "# Mine.\n[editor]\nwrap = false\n";
        assert_eq!(
            with_option(text, "wrap", None).unwrap(),
            "# Mine.\n[editor]\n"
        );
    }

    #[test]
    fn dotted_and_inline_forms() {
        let text = "[editor]\nsearch.case = \"smart\"\n";
        assert_eq!(
            set(text, "search.case", Value::String("insensitive".into())),
            "[editor]\nsearch.case = \"insensitive\"\n"
        );
    }

    #[test]
    fn key_bindings() {
        let text = "[keys.insert]\n\"<C-k>\" = \"write\"\n";
        let set = |text: &str, keys: &str, command: Option<&str>| {
            let value = command.map(|c| Value::String(c.into()));
            with_path(text, &["keys", "insert", keys], value.as_ref()).unwrap()
        };
        assert_eq!(
            set(text, "<C-s>", Some("")),
            "[keys.insert]\n\"<C-k>\" = \"write\"\n\"<C-s>\" = \"\"\n"
        );
        assert_eq!(set(text, "<C-k>", None), "");
        assert_eq!(
            set("", "gg", Some("goto_file_start")),
            "[keys.insert]\ngg = \"goto_file_start\"\n"
        );
    }

    #[test]
    fn a_broken_file_is_left_alone() {
        assert!(with_option("[editor\n", "wrap", Some(&Value::Boolean(true))).is_err());
    }
}
