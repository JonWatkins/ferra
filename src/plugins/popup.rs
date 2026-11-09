//! Reading a popup from Lua (`ctx:popup{...}`), checked so a mistake is the
//! plugin's error, not a popup that half works.
//!
//! ```lua
//! ctx:popup({
//!   name = "todos",                 -- for the plugin's commands
//!   title = "TODOs",
//!   lines = {
//!     "plain text",
//!     { { "bold part", "ui.popup.title" }, { " then plain" } },
//!   },
//!   footer = "⏎ open  q close",
//!   position = "center",            -- or "cursor", "top-right"
//!   width = 60, height = 12,        -- optional: else it fits its lines
//!   select = true,                  -- a list with a selected line
//!   selected = 1,
//!   focus = true,                   -- false: shown until the next key
//!   keys = { ["<CR>"] = "todos_open" },
//!   on_close = "todos_closed",      -- run when q / <Esc> closes it
//! })
//! ```

use mlua::{Table, Value};

use crate::commands::CommandRegistry;
use crate::editor::popup::{Popup, PopupLine, PopupPosition, PopupSpan};
use crate::input::parse_keys;

/// A popup from its Lua table.
pub fn from_table(table: &Table, registry: &CommandRegistry) -> Result<Popup, String> {
    let field = |name: &str| -> Result<Value, String> {
        table
            .get::<Value>(name)
            .map_err(|e| format!("popup.{name}: {e}"))
    };
    let string = |name: &str| -> Result<Option<String>, String> {
        match field(name)? {
            Value::Nil => Ok(None),
            Value::String(s) => Ok(Some(s.to_string_lossy())),
            Value::Integer(i) => Ok(Some(i.to_string())),
            other => Err(format!(
                "popup.{name} should be a string, not {}",
                other.type_name()
            )),
        }
    };
    let number = |name: &str| -> Result<Option<i64>, String> {
        match field(name)? {
            Value::Nil => Ok(None),
            Value::Integer(i) => Ok(Some(i)),
            Value::Number(n) if n.fract() == 0.0 => Ok(Some(n as i64)),
            other => Err(format!(
                "popup.{name} should be a whole number, not {}",
                other.type_name()
            )),
        }
    };
    let flag = |name: &str, default: bool| -> Result<bool, String> {
        match field(name)? {
            Value::Nil => Ok(default),
            Value::Boolean(b) => Ok(b),
            other => Err(format!(
                "popup.{name} should be true or false, not {}",
                other.type_name()
            )),
        }
    };
    let size = |name: &str| -> Result<Option<u16>, String> {
        number(name)?
            .map(|n| {
                u16::try_from(n)
                    .ok()
                    .filter(|&n| n >= 3)
                    .ok_or(format!("popup.{name} should be 3 or more"))
            })
            .transpose()
    };

    let mut keys = Vec::new();
    if let Value::Table(map) = field("keys")? {
        for pair in map.pairs::<String, String>() {
            let (notation, command) =
                pair.map_err(|e| format!("popup.keys: key = \"command name\" pairs ({e})"))?;
            parse_keys(&notation).map_err(|e| format!("popup.keys: {e}"))?;
            check_command(registry, &command, "popup.keys")?;
            keys.push((notation, command));
        }
        keys.sort();
    }
    let on_close = string("on_close")?;
    if let Some(command) = &on_close {
        check_command(registry, command, "popup.on_close")?;
    }
    let position = match string("position")? {
        Some(name) => PopupPosition::parse(&name)?,
        None => PopupPosition::default(),
    };

    let mut popup = Popup {
        name: string("name")?.unwrap_or_else(|| "popup".into()),
        title: string("title")?.unwrap_or_default(),
        lines: lines(&field("lines")?)?,
        footer: string("footer")?.unwrap_or_default(),
        position,
        width: size("width")?,
        height: size("height")?,
        select: flag("select", false)?,
        focus: flag("focus", true)?,
        keys,
        on_close,
        ..Popup::default()
    };
    if let Some(selected) = number("selected")? {
        popup.select((selected.max(1) - 1) as usize);
    }
    Ok(popup)
}

fn check_command(registry: &CommandRegistry, command: &str, what: &str) -> Result<(), String> {
    if registry.contains(command) {
        Ok(())
    } else {
        Err(format!("{what}: no command named {command}"))
    }
}

/// `lines`: a list of strings, or of lists of spans (`{ text, style }` or
/// `{ text = ..., style = ... }`).
pub fn lines(value: &Value) -> Result<Vec<PopupLine>, String> {
    let list = match value {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(list) => list,
        other => {
            return Err(format!(
                "popup.lines should be a list, not {}",
                other.type_name()
            ));
        }
    };
    let mut lines = Vec::new();
    for (i, item) in list.sequence_values::<Value>().enumerate() {
        let item = item.map_err(|e| e.to_string())?;
        let at = || format!("popup.lines[{}]", i + 1);
        let line = match item {
            Value::String(s) => vec![PopupSpan {
                text: s.to_string_lossy(),
                style: None,
            }],
            Value::Table(spans) => {
                let mut line = Vec::new();
                for span in spans.sequence_values::<Value>() {
                    let Value::Table(span) = span.map_err(|e| e.to_string())? else {
                        return Err(format!("{}: each span is {{ text, style }}", at()));
                    };
                    let text: Option<String> = span
                        .get::<Option<String>>("text")
                        .ok()
                        .flatten()
                        .or_else(|| span.get::<Option<String>>(1).ok().flatten());
                    let style: Option<String> = span
                        .get::<Option<String>>("style")
                        .ok()
                        .flatten()
                        .or_else(|| span.get::<Option<String>>(2).ok().flatten());
                    let text = text.ok_or_else(|| format!("{}: a span without text", at()))?;
                    line.push(PopupSpan { text, style });
                }
                line
            }
            other => {
                return Err(format!(
                    "{}: a string or a list of spans, not {}",
                    at(),
                    other.type_name()
                ));
            }
        };
        // A line's own line breaks start new lines.
        let mut current: PopupLine = Vec::new();
        for span in line {
            let mut parts = span.text.split('\n').peekable();
            while let Some(part) = parts.next() {
                if !part.is_empty() {
                    current.push(PopupSpan {
                        text: part.replace('\t', "    "),
                        style: span.style.clone(),
                    });
                }
                if parts.peek().is_some() {
                    lines.push(std::mem::take(&mut current));
                }
            }
        }
        lines.push(current);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup(lua_table: &str) -> Result<Popup, String> {
        let lua = mlua::Lua::new();
        let table: Table = lua.load(lua_table).eval().unwrap();
        let mut registry = CommandRegistry::new();
        registry.register("open_it", "", |_| Ok(()));
        from_table(&table, &registry)
    }

    #[test]
    fn reads_a_popup() {
        let p = popup(
            r#"{ name = "todos", title = "TODOs", lines = { "a\tb", { { "x", "keyword" }, { text = "y" } }, "two\nlines" },
                 position = "cursor", select = true, selected = 2, keys = { ["<CR>"] = "open_it" }, on_close = "open_it" }"#,
        )
        .unwrap();
        assert_eq!(p.name, "todos");
        assert_eq!(p.position, PopupPosition::Cursor);
        assert_eq!(p.lines.len(), 4);
        assert_eq!(p.lines[0][0].text, "a    b");
        assert_eq!(p.lines[1][0].style.as_deref(), Some("keyword"));
        assert_eq!(p.lines[1][1].text, "y");
        assert_eq!(p.lines[3][0].text, "lines");
        assert_eq!(p.selected(), Some(1));
        assert!(p.focus, "by default");
        assert_eq!(p.keys, [("<CR>".to_string(), "open_it".to_string())]);
    }

    #[test]
    fn mistakes_are_errors() {
        let error = |table| popup(table).unwrap_err();
        assert_eq!(
            error(r#"{ keys = { ["<CR>"] = "nope" } }"#),
            "popup.keys: no command named nope"
        );
        assert!(error(r#"{ keys = { ["<Bad>"] = "open_it" } }"#).starts_with("popup.keys:"));
        assert!(error(r#"{ position = "middle" }"#).contains("Unknown popup position"));
        assert!(error(r#"{ lines = { 3 } }"#).contains("popup.lines[1]"));
        assert!(error(r#"{ width = 1 }"#).contains("3 or more"));
        assert!(error(r#"{ focus = "yes" }"#).contains("true or false"));
    }
}
