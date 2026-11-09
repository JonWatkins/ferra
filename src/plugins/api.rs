//! The `ctx` object Lua callbacks receive, and calling into Lua.
//!
//! `ctx` methods (rows and columns are 1-based, columns count characters):
//!
//! | Method | |
//! |--------|---|
//! | `ctx:line(row)` | Text of a line (without line break), or `nil` |
//! | `ctx:line_count()` | Number of lines |
//! | `ctx:text()` | The whole buffer |
//! | `ctx:set_line(row, text)` | Replace a line's text |
//! | `ctx:insert_line(row, text)` | Insert a line so it becomes `row` (`line_count() + 1` appends) |
//! | `ctx:delete_line(row)` | Delete a line |
//! | `ctx:insert(text)` | Insert at the cursor, moving past it |
//! | `ctx:cursor()` | `row, col` |
//! | `ctx:set_cursor(row, col)` | Move the cursor (clamped) |
//! | `ctx:path()` | The file's path, or `nil` |
//! | `ctx:mode()` | `"normal"`, `"insert"`, ... |
//! | `ctx:message(text)` | Show a message |
//! | `ctx:run(name, ...)` | Run any command, with optional arguments |
//! | `ctx:option(name)` | An option's value here (`"tab-width"`, `"search.case"`) |
//! | `ctx:set_option(name, value [, "local"])` | Set it everywhere, or for this window or buffer |
//! | `ctx:popup{...}` | Open a popup (see `plugins/popup.rs`), replacing any other |
//! | `ctx:popup_update{lines, title, footer, selected}` | Change the open popup |
//! | `ctx:popup_close()` | Close it (without its `on_close`) |
//! | `ctx:popup_selected()` | The selected line of a list popup, or `nil` |
//! | `ctx:popup_name()` | The open popup's `name`, or `nil` |
//!
//! Fields: `ctx.count` (number or `nil`) and `ctx.args` (a list of strings).
//! Edits go through the document like built-in commands, so they're undoable
//! and repeatable with `.`. Raising a Lua error makes the command fail with
//! that message.

use std::cell::RefCell;

use mlua::{Lua, RegistryKey, Table, Value, Variadic};

use crate::commands::{CommandResult, Context, Invocation};
use crate::core::Position;
use crate::editor::Message;
use crate::events::Event;

/// Calls the Lua function stored at `key` with a `ctx` for `cx` (and the
/// event, for hooks). `ctx` only works during the call.
pub fn call(
    lua: &Lua,
    key: &RegistryKey,
    cx: &mut Context,
    event: Option<&Event>,
) -> CommandResult {
    call_returning::<()>(lua, key, cx, event)
}

/// [`call`], keeping what the function returns.
pub fn call_returning<R: mlua::FromLuaMulti>(
    lua: &Lua,
    key: &RegistryKey,
    cx: &mut Context,
    event: Option<&Event>,
) -> Result<R, String> {
    let function: mlua::Function = lua.registry_value(key).map_err(|e| describe(&e))?;
    let cell = RefCell::new(cx);
    lua.scope(|scope| {
        let ctx = lua.create_table()?;
        {
            let cx = cell.borrow();
            ctx.set("count", cx.count)?;
            ctx.set("args", cx.args.to_vec())?;
        }

        ctx.set(
            "line",
            scope.create_function(|_, (_, row): (Value, i64)| {
                let cx = cell.borrow();
                let buffer = cx.editor.document.buffer();
                Ok(row_index(row, buffer.line_count()).map(|r| buffer.line(r).into_owned()))
            })?,
        )?;
        ctx.set(
            "line_count",
            scope.create_function(|_, _: Value| {
                Ok(cell.borrow().editor.document.buffer().line_count())
            })?,
        )?;
        ctx.set(
            "text",
            scope
                .create_function(|_, _: Value| Ok(cell.borrow().editor.document.buffer().text()))?,
        )?;
        ctx.set(
            "set_line",
            scope.create_function(|_, (_, row, text): (Value, i64, String)| {
                let mut guard = cell.borrow_mut();
                let doc = &mut guard.editor.document;
                let r = row_index(row, doc.buffer().line_count()).ok_or_else(|| bad_row(row))?;
                let cursor = doc.cursor();
                doc.delete(Position::new(r, 0), doc.buffer().line_end(r));
                doc.insert(Position::new(r, 0), &text);
                doc.set_cursor(cursor);
                Ok(())
            })?,
        )?;
        ctx.set(
            "insert_line",
            scope.create_function(|_, (_, row, text): (Value, i64, String)| {
                let mut guard = cell.borrow_mut();
                let doc = &mut guard.editor.document;
                let count = doc.buffer().line_count();
                let cursor = doc.cursor();
                match row_index(row, count + 1) {
                    Some(r) if r < count => {
                        doc.insert(Position::new(r, 0), &format!("{text}\n"));
                    }
                    Some(_) => {
                        doc.insert(doc.buffer().line_end(count - 1), &format!("\n{text}"));
                    }
                    None => return Err(bad_row(row)),
                }
                doc.set_cursor(cursor);
                Ok(())
            })?,
        )?;
        ctx.set(
            "delete_line",
            scope.create_function(|_, (_, row): (Value, i64)| {
                let mut guard = cell.borrow_mut();
                let doc = &mut guard.editor.document;
                let r = row_index(row, doc.buffer().line_count()).ok_or_else(|| bad_row(row))?;
                let cursor = doc.cursor();
                doc.delete_lines(r, r);
                doc.set_cursor(cursor);
                Ok(())
            })?,
        )?;
        ctx.set(
            "insert",
            scope.create_function(|_, (_, text): (Value, String)| {
                cell.borrow_mut().editor.document.insert_str(&text);
                Ok(())
            })?,
        )?;
        ctx.set(
            "cursor",
            scope.create_function(|_, _: Value| {
                let cursor = cell.borrow().editor.document.cursor();
                Ok((cursor.row + 1, cursor.col + 1))
            })?,
        )?;
        ctx.set(
            "set_cursor",
            scope.create_function(|_, (_, row, col): (Value, i64, i64)| {
                let position = Position::new(row.max(1) as usize - 1, col.max(1) as usize - 1);
                cell.borrow_mut().editor.document.set_cursor(position);
                Ok(())
            })?,
        )?;
        ctx.set(
            "path",
            scope.create_function(|_, _: Value| {
                let cx = cell.borrow();
                Ok(cx
                    .editor
                    .document
                    .path()
                    .map(|p| p.to_string_lossy().into_owned()))
            })?,
        )?;
        ctx.set(
            "mode",
            scope.create_function(|_, _: Value| Ok(cell.borrow().editor.mode().id()))?,
        )?;
        ctx.set(
            "message",
            scope.create_function(|_, (_, text): (Value, String)| {
                cell.borrow_mut().editor.set_message(Message::info(text));
                Ok(())
            })?,
        )?;
        ctx.set(
            "run",
            scope.create_function(|_, (_, name, args): (Value, String, Variadic<String>)| {
                let mut guard = cell.borrow_mut();
                let cx = &mut **guard;
                let args: Vec<String> = args.into_iter().collect();
                cx.registry
                    .execute(&name, cx.editor, Invocation::with_args(&args))
                    .map_err(mlua::Error::runtime)
            })?,
        )?;

        ctx.set(
            "option",
            scope.create_function(|lua, (_, name): (Value, String)| {
                match cell.borrow().editor.option_value(&name) {
                    Some(value) => to_lua(lua, &value),
                    None => Err(mlua::Error::runtime(format!("Unknown option: {name}"))),
                }
            })?,
        )?;
        ctx.set(
            "set_option",
            scope.create_function(
                |_, (_, name, value, scope): (Value, String, Value, Option<String>)| {
                    let local = match scope.as_deref() {
                        None | Some("global") => false,
                        Some("local") => true,
                        Some(other) => {
                            return Err(mlua::Error::runtime(format!(
                                "set_option: scope is \"local\" or \"global\", not \"{other}\""
                            )));
                        }
                    };
                    let value = to_toml(&value).map_err(mlua::Error::runtime)?;
                    cell.borrow_mut()
                        .editor
                        .set_option(&name, value, local)
                        .map_err(mlua::Error::runtime)
                },
            )?,
        )?;

        // Popups (`plugins/popup.rs` reads the table).
        ctx.set(
            "popup",
            scope.create_function(|_, (_, spec): (Value, Table)| {
                let mut guard = cell.borrow_mut();
                let cx = &mut **guard;
                let popup =
                    super::popup::from_table(&spec, cx.registry).map_err(mlua::Error::runtime)?;
                cx.editor.open_popup(popup);
                Ok(())
            })?,
        )?;
        ctx.set(
            "popup_update",
            scope.create_function(|_, (_, spec): (Value, Table)| {
                let mut guard = cell.borrow_mut();
                let popup = guard
                    .editor
                    .popup_mut()
                    .ok_or_else(|| mlua::Error::runtime("No popup is open"))?;
                if let Some(title) = spec.get::<Option<String>>("title")? {
                    popup.title = title;
                }
                if let Some(footer) = spec.get::<Option<String>>("footer")? {
                    popup.footer = footer;
                }
                let lines: Value = spec.get("lines")?;
                if !lines.is_nil() {
                    popup.lines = super::popup::lines(&lines).map_err(mlua::Error::runtime)?;
                    let selected = popup.selected().unwrap_or(0);
                    popup.select(selected);
                }
                if let Some(selected) = spec.get::<Option<i64>>("selected")? {
                    popup.select((selected.max(1) - 1) as usize);
                }
                Ok(())
            })?,
        )?;
        ctx.set(
            "popup_close",
            scope.create_function(|_, _: Value| {
                cell.borrow_mut().editor.close_popup();
                Ok(())
            })?,
        )?;
        ctx.set(
            "popup_selected",
            scope.create_function(|_, _: Value| {
                Ok(cell
                    .borrow()
                    .editor
                    .popup()
                    .and_then(|p| p.selected())
                    .map(|i| i + 1))
            })?,
        )?;
        ctx.set(
            "popup_name",
            scope.create_function(|_, _: Value| {
                Ok(cell.borrow().editor.popup().map(|p| p.name.clone()))
            })?,
        )?;

        let event = match event {
            Some(event) => Value::Table(event_table(lua, event)?),
            None => Value::Nil,
        };
        function.call::<R>((ctx, event))
    })
    .map_err(|e| describe(&e))
}

/// An option's value for Lua: booleans, integers, strings, and tables for
/// lists and groups.
fn to_lua(lua: &Lua, value: &toml::Value) -> mlua::Result<Value> {
    Ok(match value {
        toml::Value::Boolean(b) => Value::Boolean(*b),
        toml::Value::Integer(i) => Value::Integer(*i),
        toml::Value::Float(f) => Value::Number(*f),
        toml::Value::String(s) => Value::String(lua.create_string(s)?),
        toml::Value::Datetime(d) => Value::String(lua.create_string(d.to_string())?),
        toml::Value::Array(items) => {
            let table = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, to_lua(lua, item)?)?;
            }
            Value::Table(table)
        }
        toml::Value::Table(entries) => {
            let table = lua.create_table()?;
            for (key, item) in entries {
                table.set(key.as_str(), to_lua(lua, item)?)?;
            }
            Value::Table(table)
        }
    })
}

/// A Lua value for an option: the reverse of [`to_lua`]. A table is a list
/// if its keys are 1, 2, 3...
fn to_toml(value: &Value) -> Result<toml::Value, String> {
    Ok(match value {
        Value::Boolean(b) => toml::Value::Boolean(*b),
        Value::Integer(i) => toml::Value::Integer(*i),
        Value::Number(n) if n.fract() == 0.0 => toml::Value::Integer(*n as i64),
        Value::Number(n) => toml::Value::Float(*n),
        Value::String(s) => toml::Value::String(s.to_str().map_err(|e| e.to_string())?.to_string()),
        Value::Table(table) => {
            let len = table.raw_len();
            if len > 0 {
                let mut items = Vec::with_capacity(len);
                for i in 1..=len {
                    let item: Value = table.raw_get(i).map_err(|e| e.to_string())?;
                    items.push(to_toml(&item)?);
                }
                toml::Value::Array(items)
            } else {
                let mut entries = toml::Table::new();
                for pair in table.clone().pairs::<String, Value>() {
                    let (key, item) = pair.map_err(|e| e.to_string())?;
                    entries.insert(key, to_toml(&item)?);
                }
                toml::Value::Table(entries)
            }
        }
        other => return Err(format!("an option can't be {}", other.type_name())),
    })
}

/// The event passed to hooks, as a table: `{ kind = "after-save", path = ... }`.
fn event_table(lua: &Lua, event: &Event) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("kind", event.kind().id())?;
    match event {
        Event::Opened { path } | Event::AfterSave { path } => {
            table.set("path", path.to_string_lossy().into_owned())?;
        }
        Event::ModeChanged { from, to } => {
            table.set("from", from.id())?;
            table.set("to", to.id())?;
        }
        Event::BeforeSave | Event::BufferChanged => {}
    }
    Ok(table)
}

/// A 1-based Lua row as a 0-based index, if it's within `count` rows.
fn row_index(row: i64, count: usize) -> Option<usize> {
    (row >= 1 && (row as usize) <= count).then(|| row as usize - 1)
}

fn bad_row(row: i64) -> mlua::Error {
    mlua::Error::runtime(format!("line {row} is out of range"))
}

/// A one-line description of a Lua error, without the traceback.
pub fn describe(error: &mlua::Error) -> String {
    let root = match error {
        mlua::Error::CallbackError { cause, .. } => cause.as_ref(),
        other => other,
    };
    let text = root.to_string();
    let first = text.lines().next().unwrap_or("").trim();
    first
        .strip_prefix("runtime error: ")
        .unwrap_or(first)
        .to_string()
}
