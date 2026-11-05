# Architecture

A modal terminal editor. Vim's keys and grammar, Helix's architecture, and
Emacs's "every action is a named command" extensibility.

## Layers

Dependencies only point downwards. Lower layers never know about higher ones.

```
 main.rs ─▶ app        event loop, wiring, terminal side effects
             │
     ┌───────┼──────────┬──────────┐
     ▼       ▼          ▼          ▼
    ui     input     commands    config
     │       │          │          │
     └───────┴────┬─────┴──────────┘
                  ▼
               editor      Editor, Document, Mode, CommandLine, Message
                  │
                  ▼
                core       Buffer, Position, motions (pure, no deps)
```

| Module      | Responsibility | Terminal/UI deps? |
|-------------|----------------|-------------------|
| `core`      | Text storage and pure motion functions. | No |
| `editor`    | Editor state and mode transition rules. | No |
| `commands`  | Named command registry plus built-in commands. | No |
| `input`     | Key notation, per-mode keymap trie, counts and pending sequences. | crossterm (event conversion only) |
| `config`    | Embedded `default.toml` deep-merged with the user config; options, theme, keymap. | ratatui `Style` (theme) |
| `ui`        | Widgets: our own `EditorView` (gutter, viewport, cursorline), `StatusLine`, `CommandLineView`. | ratatui |
| `app`       | Owns everything and runs the loop. | ratatui + crossterm |

`core`, `editor` and `commands` have no terminal dependencies, so they can be
unit tested directly. They can also become their own crates later
(`editor-core`, `editor-view`, `editor-term`, as Helix does) without
restructuring.

## Key handling flow

```
crossterm KeyEvent
  → KeyPress::from_event            (input/key.rs)
  → InputHandler::handle            (input/mod.rs: counts, pending keys)
      → Keymap::lookup(mode, keys)  (input/keymap.rs: trie)
  → Outcome::Execute { command, count } | InsertChar | Pending | Unbound
  → CommandRegistry::execute        (commands/mod.rs)
  → Editor::enforce_cursor_rules    (normal mode can't sit past the last char)
```

The UI only *reads* state. All mutation goes through commands, apart from
literal text insertion.

## Commands: the extension point

Every action is a named command such as `move_down`, `delete_line` or `quit`.
This means:

- **Config** binds keys to command names (`"jk" = "normal_mode"`).
- **The `:` command line** runs any command by name or alias (`:q`, `:goto_line 10`, `:42`).
- **Plugins** will call `CommandRegistry::register(name, description, handler)`,
  exactly as built-ins do. Handlers receive a `Context` with the editor, the
  registry (so they can compose other commands), a count and arguments.
- **Discoverability**: `CommandRegistry::commands()` lists names and
  descriptions, ready for a command palette or completion.

`tests/extensibility.rs` covers this contract.

## Plugin roadmap (not yet implemented)

The current design leaves room for these:

1. **Scripting runtime.** Lua (via `mlua`) is the likely first choice because
   it's familiar to Neovim users. A plugin's `register_command` call maps
   straight onto `CommandRegistry::register`. The handler closure is
   deliberately not `Send`, so a non-thread-safe Lua state can live inside it.
2. **Hooks/events.** Add an `events` module with an event enum
   (`ModeChanged`, `BufferChanged`, `KeyPressed`, ...) and a hook registry
   that `App` fires after commands run. Plugins subscribe to it.
3. **Plugin keymaps.** `Keymap::bind` already supports runtime binding.
4. **Plugin UI.** Statusline segments and gutter columns become lists of
   components that plugins can contribute to.
5. **Isolation.** If sandboxing becomes a goal, a WASM runtime can sit behind
   the same command and hook interfaces.

## Configuration

- `src/config/default.toml` is embedded with `include_str!` and is the single
  source of truth for defaults.
- The user file (`$XDG_CONFIG_HOME/terminal-editor/config.toml`, otherwise
  `~/.config/terminal-editor/config.toml`) is deep-merged over the defaults.
  Users only write what they change. Binding a key to `""` removes it.
- Unknown fields, bad colours and bad key notation are errors. A broken config
  doesn't stop startup: the editor falls back to the defaults and shows the
  error.
- Themes use dot-separated scopes that fall back to their parent
  (`ui.linenr.selected` → `ui.linenr`), which leaves room for syntax scopes.

## Testing strategy

- **Unit tests** sit next to the code in `#[cfg(test)] mod tests`. Pure logic
  (motions, keymap, input state machine, viewport, gutter, theme parsing) is
  tested exhaustively.
- **Command tests** use `commands::builtin::test_util` to run a command
  against an editor, the same way the app does.
- **Widget tests** render into a `ratatui::buffer::Buffer` and assert on the
  text.
- **Integration tests** (`tests/`) drive the full `App` with Vim key notation
  (`keys(&mut app, "ihello<Esc>dd")`) and render through `TestBackend`.

## Known gaps / next steps

- Undo/redo, which needs an edit transaction log in `core`.
- Operator-pending mode (`d{motion}`, `c`, `y`) and registers. Motions are
  already pure functions, ready to be used as operator targets.
- Visual mode and selections.
- File open/save and multiple buffers/windows (`Document` → `View` split).
- Timeout for ambiguous insert-mode sequences such as `jk` (Vim's `timeoutlen`).
- Rope-based buffer (e.g. `ropey`) for large files. `Buffer`'s API hides the
  storage, so this is a local change.
