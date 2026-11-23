# Every action is a named command

Every action is a command in one registry, with a name and description (`move_down`, `delete_line`, `quit`). Config binds keys to command names, the `:` line runs any command by name or alias, the palette and menus list them, plugins register theirs the same way as built-ins, and macros and `.` replay the keys that ran them. So a new feature is reachable from keys, `:`, menus and plugins at once, and plugin commands get undo, `.` and hooks with no special cases.

## Consequences

- A handler takes a `Context` (the editor, the registry, and the `Invocation`: count, register, arguments, range, `!`), grouped so its signature stays stable as input features are added.
- `tests/extensibility.rs` holds this contract.
