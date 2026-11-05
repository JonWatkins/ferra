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
               editor      Editor, Document, Mode, Registers, operators
                  │   ╲
                  ▼    ▼
                core   files   core: Buffer, History, TextFormat, motions (pure)
                               files: the only code that touches the disk
```

| Module      | Responsibility | Terminal/UI deps? |
|-------------|----------------|-------------------|
| `core`      | Text storage, undo history, ranges, and pure motion and text-object functions. | No |
| `editor`    | Editor state, mode transition rules, registers, applying operators. | No |
| `commands`  | Named command registry plus built-in commands. | No |
| `input`     | Key notation, per-mode keymap trie, counts and pending sequences. | crossterm (event conversion only) |
| `files`     | Reading files and atomic writes (temp file + fsync + rename, permissions, symlinks). | No |
| `config`    | Embedded `default.toml` deep-merged with the user config; options, theme, keymap. | ratatui `Style` (theme) |
| `cli`       | Command-line argument parsing. | No |
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

## Editing model

### Undo

- `Buffer` has exactly two mutators: `insert_text` and `delete_range`.
  `Document` wraps them and records each edit as a `core::Change`. Nothing
  else can change the text, so every edit is undoable.
- `core::History` groups changes into transactions. Adjacent inserts and
  backspaces merge, so typing a word is stored as one change.
- `Editor::finish_command` runs after every command and closes the undo step
  unless the editor is in insert mode. So, as in Vim, each normal-mode
  command is one step, and an insert session undoes together with the command
  that started it (`o`, `cw`, `A`, ...).

### Operators

Vim's grammar is `[count]["x]operator[count](motion | text-object)`:

1. An operator command (`operator_delete`, ...) calls `Editor::begin_operator`,
   which enters `Mode::OperatorPending` and stores the count and register.
2. Keys are now looked up in `[keys.operator]` (Vim's `omap`). Counts are
   still accepted, so `2d3w` works.
3. **Motions are data** (`commands/builtin/motions.rs`): a target function plus
   a `MotionKind` (exclusive, inclusive or linewise). A motion command takes
   the pending operator if there is one and applies it over
   `Range::from_motion`. Otherwise it moves the cursor. One definition serves
   both `w` and `dw`.
4. **Text objects** (`core::text_object`) return a `Range` directly. `dd`,
   `cc` and `yy` are the `current_line` text object.
5. `Editor::apply_operator` stores the text in a register, then deletes,
   changes (deletes and enters insert mode) or yanks.

Single-key shortcuts (`x`, `D`, `C`, `s`, `S`, `Y`, `X`) apply an operator
over a fixed motion. Pastes read registers (`editor/registers.rs`).

### Files

- `core::TextFormat` splits file contents into buffer text (always `\n`, no
  final newline) and the format to write back (LF/CRLF, final newline). An
  untouched file round-trips byte for byte.
- `files` is the only module that touches the disk. Writes go to a temp file
  in the same directory, are fsynced, then renamed over the target. Existing
  permissions are copied and symlinks are followed.
- `Document` holds the path, format and two pieces of save state:
  - **Modified tracking:** `History` gives each committed state a
    `Revision`. The document is modified when its revision differs from the
    saved one, so undoing back to the save clears `[+]`.
  - **External changes:** the file's mtime at read/write time. A save
    refuses to overwrite a file whose mtime has changed since, or an
    existing file the document never read, unless forced with `!`.
- On the `:` line, a trailing `!` sets `Invocation::bang`, which file
  commands use to override those checks.

## Commands: the extension point

Every action is a named command such as `move_down`, `delete_line` or `quit`.
This means:

- **Config** binds keys to command names (`"jk" = "normal_mode"`).
- **The `:` command line** runs any command by name or alias (`:q`, `:goto_line 10`, `:42`).
- **Plugins** will call `CommandRegistry::register(name, description, handler)`,
  exactly as built-ins do. Handlers receive a `Context` with the editor, the
  registry (so they can compose other commands), and the `Invocation` (count,
  register and arguments). Invocation details are grouped into one struct so
  the signature stays stable as input features are added.
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

- Visual mode, which would reuse text objects and `apply_operator` directly.
- `.` repeat, which needs the last change's command, invocation and inserted
  text recorded together.
- More motions and operators: `f`/`t`, `%`, `}`/`{`, `>`/`<`, `gu`/`gU`, `J`.
- Numbered delete registers `1`-`9` and the small-delete register `-`.
- Multiple buffers/windows (`Document` → `View` split), so `:e` doesn't
  replace the only document.
- Asking before overwriting on external changes, and reloading
  automatically when the file changes and there are no local edits.
- Writing through hard links keeps the old inode's other names pointing at
  the old contents (the rename replaces the file). Vim's `backupcopy`
  handles this; we don't yet.
- Non-UTF-8 encodings are refused rather than supported.
- Timeout for ambiguous key sequences such as `jk` in insert mode (Vim's
  `timeoutlen`).
- Rope-based buffer (e.g. `ropey`) for large files. `Buffer` has only two
  mutators, so this is a local change.
