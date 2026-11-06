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

        let event = match event {
            Some(event) => Value::Table(event_table(lua, event)?),
            None => Value::Nil,
        };
        function.call::<()>((ctx, event))
    })
    .map_err(|e| describe(&e))
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
