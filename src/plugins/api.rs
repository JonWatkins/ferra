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
//! | `ctx:decorate{line, col, text / to_line, to_col, style, group, end_of_line}` | Virtual text, or a highlighted range, in the buffer |
//! | `ctx:clear_decorations([group])` | Take a group's decorations away |
//! | `ctx:pick{title, items, on_choose}` | The fuzzy picker with the plugin's items; `on_choose` runs with the item's value (see `plugins/prompts.rs`) |
//! | `ctx:input{title, value, placeholder, on_submit, on_cancel}` | A box asking for a line of text; `on_submit` runs with it |
//! | `ctx:view_selected()` | The selected line of the plugin view in the sidebar, or `nil` |
//! | `ctx:view_name()` | The sidebar view shown (`"files"` or a plugin's), or `nil` if it's closed |
//! | `ctx:view_refresh([name])` | Make a view's lines again (the one shown, by default) |
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

        // Decorations: virtual text and highlights in the current buffer.
        ctx.set(
            "decorate",
            scope.create_function(|_, (_, spec): (Value, Table)| {
                let mut guard = cell.borrow_mut();
                decorate(&mut guard.editor.document, &spec).map_err(mlua::Error::runtime)
            })?,
        )?;
        ctx.set(
            "clear_decorations",
            scope.create_function(|_, (_, group): (Value, Option<String>)| {
                let group = group.unwrap_or_else(|| "default".into());
                cell.borrow_mut()
                    .editor
                    .document
                    .clear_decorations(&format!("plugin:{group}"));
                Ok(())
            })?,
        )?;

        // Asking the user (`plugins/prompts.rs`).
        ctx.set(
            "pick",
            scope.create_function(|_, (_, spec): (Value, Table)| {
                let mut guard = cell.borrow_mut();
                let cx = &mut **guard;
                let picker =
                    super::prompts::picker(&spec, cx.registry).map_err(mlua::Error::runtime)?;
                cx.editor.close_popup();
                cx.editor.open_picker(picker);
                Ok(())
            })?,
        )?;
        ctx.set(
            "input",
            scope.create_function(|_, (_, spec): (Value, Table)| {
                let mut guard = cell.borrow_mut();
                let cx = &mut **guard;
                let input =
                    super::prompts::input_box(&spec, cx.registry).map_err(mlua::Error::runtime)?;
                cx.editor.open_input_box(input);
                Ok(())
            })?,
        )?;

        // Sidebar views.
        ctx.set(
            "view_selected",
            scope.create_function(|_, _: Value| {
                Ok(cell
                    .borrow()
                    .editor
                    .plugin_view()
                    .and_then(|v| v.selected())
                    .map(|i| i + 1))
            })?,
        )?;
        ctx.set(
            "view_name",
            scope.create_function(|_, _: Value| {
                let guard = cell.borrow();
                Ok(guard.editor.sidebar().map(|_| {
                    guard
                        .editor
                        .plugin_view()
                        .map_or("files".to_string(), |v| v.spec.name.clone())
                }))
            })?,
        )?;
        ctx.set(
            "view_refresh",
            scope.create_function(|_, (_, name): (Value, Option<String>)| {
                cell.borrow_mut()
                    .editor
                    .refresh_plugin_view(name.as_deref());
                Ok(())
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

/// `ctx:decorate{...}`: virtual text (with `text`) or a highlighted range
/// (with `to_col`, and `to_line` if it's another line) in `document`.
/// Lines and columns are 1-based; `to_col` is the last column, inclusive,
/// as `string.find` gives it.
fn decorate(document: &mut crate::editor::Document, spec: &Table) -> Result<(), String> {
    let get_number = |name: &str| -> Result<Option<i64>, String> {
        match spec.get::<Value>(name).map_err(|e| e.to_string())? {
            Value::Nil => Ok(None),
            Value::Integer(n) => Ok(Some(n)),
            Value::Number(n) if n.fract() == 0.0 => Ok(Some(n as i64)),
            other => Err(format!(
                "decorate: {name} should be a whole number, not {}",
                other.type_name()
            )),
        }
    };
    let get_text = |name: &str| -> Result<Option<String>, String> {
        match spec.get::<Value>(name).map_err(|e| e.to_string())? {
            Value::Nil => Ok(None),
            Value::String(s) => Ok(Some(s.to_string_lossy())),
            other => Err(format!(
                "decorate: {name} should be text, not {}",
                other.type_name()
            )),
        }
    };
    let buffer = document.buffer();
    let lines = buffer.line_count();
    let row = |n: i64| -> Result<usize, String> {
        if n >= 1 && (n as usize) <= lines {
            Ok(n as usize - 1)
        } else {
            Err(format!("decorate: line {n} is out of range (1 to {lines})"))
        }
    };
    let line = row(get_number("line")?.ok_or("decorate: needs a line")?)?;
    let col = |row: usize, n: Option<i64>| -> usize {
        let n = n.unwrap_or(1).max(1) as usize - 1;
        n.min(buffer.line_len(row))
    };
    let start = Position::new(line, col(line, get_number("col")?));
    let group = get_text("group")?.unwrap_or_else(|| "default".into());
    let source = format!("plugin:{group}");
    let style = get_text("style")?;
    let end_of_line = match spec
        .get::<Value>("end_of_line")
        .map_err(|e| e.to_string())?
    {
        Value::Nil => false,
        Value::Boolean(b) => b,
        other => {
            return Err(format!(
                "decorate: end_of_line should be true or false, not {}",
                other.type_name()
            ));
        }
    };
    if let Some(text) = get_text("text")? {
        let piece = crate::editor::VirtualText {
            at: start,
            text: text.replace(['\n', '\r'], " "),
            style: style.unwrap_or_else(|| "ui.virtual.inlay-hint".into()),
            source,
            end_of_line,
        };
        let mut pieces: Vec<_> = document
            .virtual_text()
            .iter()
            .filter(|v| v.source == piece.source)
            .cloned()
            .collect();
        pieces.push(piece.clone());
        document.set_virtual_text(&piece.source, pieces);
        return Ok(());
    }
    let to_line = match get_number("to_line")? {
        Some(n) => row(n)?,
        None => line,
    };
    let to_col = get_number("to_col")?
        .ok_or("decorate: needs text (virtual text) or to_col (a range to highlight)")?;
    // The last column, 1-based and inclusive: the range's end, 0-based.
    let end_col = (to_col.max(0) as usize).min(buffer.line_len(to_line));
    let end = Position::new(to_line, end_col);
    if end <= start {
        return Err("decorate: the range ends before it starts".into());
    }
    document.add_highlights(vec![crate::editor::RangeHighlight {
        start,
        end,
        style: style.unwrap_or_else(|| "ui.highlight.reference".into()),
        source,
    }]);
    Ok(())
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
