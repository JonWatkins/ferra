//! Asking the user from Lua: `ctx:pick{...}` (the fuzzy picker, with the
//! plugin's items) and `ctx:input{...}` (a box for a line of text). Either
//! runs a plugin command with the answer as its argument (`ctx.args[1]`).
//!
//! ```lua
//! ctx:pick({
//!   title = "Colour",
//!   items = { "red", { label = "Green", detail = "#00ff00", value = "green" } },
//!   on_choose = "set_colour",     -- run with the item's value (or label)
//! })
//! ctx:input({
//!   title = "New name",
//!   value = "old",                -- to start with, selected
//!   placeholder = "a name",       -- shown while it's empty
//!   on_submit = "rename_to",      -- run with the text
//!   on_cancel = "rename_cancelled", -- optional
//! })
//! ```

use mlua::{Table, Value};

use crate::commands::CommandRegistry;
use crate::editor::input_box::InputBox;
use crate::editor::picker::{Picker, PickerAction, PickerItem};

fn command(
    table: &Table,
    registry: &CommandRegistry,
    what: &str,
    field: &str,
) -> Result<Option<String>, String> {
    let name: Option<String> = table
        .get(field)
        .map_err(|e| format!("{what}.{field} should be a command name ({e})"))?;
    match name {
        Some(name) if !registry.contains(&name) => {
            Err(format!("{what}.{field}: no command named {name}"))
        }
        other => Ok(other),
    }
}

fn text(table: &Table, what: &str, field: &str) -> Result<String, String> {
    let value: Option<String> = table
        .get(field)
        .map_err(|e| format!("{what}.{field} should be text ({e})"))?;
    Ok(value.unwrap_or_default())
}

/// A picker from `ctx:pick`'s table.
pub fn picker(table: &Table, registry: &CommandRegistry) -> Result<Picker, String> {
    let on_choose = command(table, registry, "pick", "on_choose")?
        .ok_or("pick needs on_choose, the command to run")?;
    let title = text(table, "pick", "title")?;
    let items: Table = table
        .get::<Option<Table>>("items")
        .map_err(|e| format!("pick.items should be a list ({e})"))?
        .ok_or("pick needs items")?;
    let mut list = Vec::new();
    for (i, item) in items.sequence_values::<Value>().enumerate() {
        let item = item.map_err(|e| e.to_string())?;
        let (label, detail, value) = match item {
            Value::String(s) => {
                let label = s.to_string_lossy();
                (label.clone(), String::new(), label)
            }
            Value::Integer(n) => (n.to_string(), String::new(), n.to_string()),
            Value::Table(item) => {
                let label = text(&item, "pick.items[]", "label")?;
                if label.is_empty() {
                    return Err(format!("pick.items[{}] needs a label", i + 1));
                }
                let detail = text(&item, "pick.items[]", "detail")?;
                let value = match item.get::<Value>("value").map_err(|e| e.to_string())? {
                    Value::Nil => label.clone(),
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(n) => n.to_string(),
                    other => {
                        return Err(format!(
                            "pick.items[{}].value should be text, not {}",
                            i + 1,
                            other.type_name()
                        ));
                    }
                };
                (label, detail, value)
            }
            other => {
                return Err(format!(
                    "pick.items[{}] should be text or {{ label, detail, value }}, not {}",
                    i + 1,
                    other.type_name()
                ));
            }
        };
        list.push(PickerItem {
            label,
            detail,
            action: PickerAction::RunWith(on_choose.clone(), vec![value]),
        });
    }
    Ok(Picker::new(&title, list))
}

/// An input box from `ctx:input`'s table.
pub fn input_box(table: &Table, registry: &CommandRegistry) -> Result<InputBox, String> {
    let on_submit = command(table, registry, "input", "on_submit")?
        .ok_or("input needs on_submit, the command to run")?;
    let mut input = InputBox::new(
        &text(table, "input", "title")?,
        &text(table, "input", "value")?,
        &on_submit,
    );
    input.placeholder = text(table, "input", "placeholder")?;
    input.on_cancel = command(table, registry, "input", "on_cancel")?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(lua: &mlua::Lua, source: &str) -> Table {
        lua.load(source).eval().unwrap()
    }

    fn registry() -> CommandRegistry {
        let mut registry = CommandRegistry::new();
        registry.register("chosen", "", |_| Ok(()));
        registry
    }

    #[test]
    fn reads_a_pick() {
        let lua = mlua::Lua::new();
        let picker = picker(
            &eval(&lua, r##"{ title = "Colour", items = { "red", { label = "Green", detail = "#0f0", value = "green" } }, on_choose = "chosen" }"##),
            &registry(),
        )
        .unwrap();
        assert_eq!(picker.title(), "Colour");
        assert_eq!(picker.total(), 2);
        let items: Vec<&PickerItem> = picker.matches().iter().map(|m| picker.item(m)).collect();
        let green = items[1];
        assert_eq!(
            (green.label.as_str(), green.detail.as_str()),
            ("Green", "#0f0")
        );
        assert_eq!(
            green.action,
            PickerAction::RunWith("chosen".into(), vec!["green".into()])
        );
        let red = items[0];
        assert_eq!(
            red.action,
            PickerAction::RunWith("chosen".into(), vec!["red".into()])
        );
    }

    #[test]
    fn reads_an_input_box() {
        let lua = mlua::Lua::new();
        let input = input_box(
            &eval(&lua, r#"{ title = "Name", value = "old", placeholder = "a name", on_submit = "chosen" }"#),
            &registry(),
        )
        .unwrap();
        assert_eq!((input.title.as_str(), input.field.text()), ("Name", "old"));
        assert!(input.field.is_selected());
        assert_eq!(input.placeholder, "a name");
    }

    #[test]
    fn mistakes_are_errors() {
        let lua = mlua::Lua::new();
        let pick_error = |source| picker(&eval(&lua, source), &registry()).unwrap_err();
        assert_eq!(
            pick_error(r#"{ items = {} }"#),
            "pick needs on_choose, the command to run"
        );
        assert_eq!(
            pick_error(r#"{ items = {}, on_choose = "nope" }"#),
            "pick.on_choose: no command named nope"
        );
        assert!(
            pick_error(r#"{ items = { true }, on_choose = "chosen" }"#).contains("pick.items[1]")
        );
        assert!(
            pick_error(r#"{ items = { {} }, on_choose = "chosen" }"#).contains("needs a label")
        );
        let input_error = |source| input_box(&eval(&lua, source), &registry()).unwrap_err();
        assert_eq!(
            input_error("{}"),
            "input needs on_submit, the command to run"
        );
        assert!(
            input_error(r#"{ on_submit = "chosen", on_cancel = "gone" }"#)
                .contains("no command named gone")
        );
    }
}
