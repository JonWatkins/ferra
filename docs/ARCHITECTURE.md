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

## Text storage

`core::Buffer` wraps a `ropey::Rope`, so inserts, deletes and line lookups
are O(log n) in the file size. Lines are `\n`-separated (file line endings
are `TextFormat`'s job). `Buffer::line` returns `Cow<str>`, which is borrowed
when the line lies in one rope chunk, so the common case doesn't allocate.
`Buffer` still has only two mutators, `insert_text` and `delete_range`, so
the switch to a rope didn't touch anything outside `core::buffer`.

## Buffers

`Editor::document` is always the active buffer; the others wait in
`Editor::background`, least recently used first (`editor/buffers.rs`).
Switching swaps a background document into place, so the rest of the
editor only ever deals with "the document". Each `Document` has a
`DocumentId` (shown from 1), its own cursor and its own undo history. The app
keeps each buffer's scroll position (`App::other_views`).

`Editor::open` switches to an already-open file (comparing canonical paths)
or opens a new buffer. A pristine scratch buffer is replaced rather than
kept, and its number is reused. `:q` and the write-and-quit commands check
every buffer for unsaved changes. `:wa` saves each modified buffer as the
active one, so its before-save hooks run.

## Windows

`editor/windows/` holds split windows. A `Window` is a view onto a buffer:
a `DocumentId` and its own cursor. `Layout` is a tree of horizontal and
vertical splits whose children share space equally, separated by one-cell
lines.

- `Layout::arrange` computes window and separator areas in plain cells, with
  no UI types. Directional moves (`<C-w>h`) use the same geometry on a fixed
  nominal area, since direction only depends on relative positions. Nothing
  is ever placed outside the given area, however small.
- The active window's buffer is always `Editor::document`, so commands are
  unchanged. Focusing another window saves the active window's cursor into
  its `Window`, then swaps the target's buffer in and restores its cursor.
  Two windows on one buffer each keep their own cursor.
- Buffers interact with windows in two places: a buffer shown in another
  window is never dropped when a new file replaces it, and closing a buffer
  points any window showing it at the next buffer.
- The UI (`ui/windows.rs`) draws each window's `EditorView`, with that
  window's cursor and scroll state (kept per window and buffer, in
  `App::views`). The cursor line and selection appear only in the active
  window. The app refreshes syntax highlights for every buffer on screen
  before drawing.
- `:q` closes the window and quits only from the last one; `:qa` quits.

## Pickers

`editor/picker/` holds the picker state: items with an action
(`SwitchBuffer` or `RunCommand`), the query, and fuzzy-matched results
(`fuzzy.rs`, a small dependency-free matcher that favours word starts and
consecutive runs). `Mode::Picker` has its own `[keys.picker]` table; unbound
printable keys filter the list. The command palette lists the registry's
commands, except ones that drive the prompt or picker or need a character.
`ui/picker.rs` draws the popup over the editor pane, cutting labels (paths)
from the start and descriptions from the end.

The file picker lists files with `files::list_files`, which uses the `ignore`
crate (ripgrep's walker): `.gitignore`, `.ignore` and git excludes are
honoured even outside a repository, hidden files are optional, `.git` is
never listed, and listing stops at `max-files` (`[editor.file-picker]`). The
fuzzy matcher gives matches in a path's file name extra weight, so `main`
finds `src/main.rs` before `domain/lib.rs`.

## Syntax highlighting

`syntax/` wraps tree-sitter:

- `languages.rs` lists each language: name, extensions/file names, grammar
  and queries. Adding a language is one entry plus its grammar crate.
- `highlight.rs` runs `tree-sitter-highlight` and turns its events into
  spans per line, in char columns. `HIGHLIGHT_NAMES` are the names queries'
  captures are mapped to, and they double as theme scopes.
- `SyntaxEngine` compiles each language's queries on first use and caches
  highlights per document, keyed on `Document::text_version`. That counter
  changes on every edit, undo, redo and reload, unlike `edit_count`, which
  only `.` uses. Documents over 4M chars aren't highlighted.

The app asks the engine for the active buffer's highlights when drawing.
The editor pane colours text with them first, then draws search matches and
the selection on top.

**Performance.** The whole document is re-highlighted when it changes: about
30 ms for a 5,000-line Rust file in a release build, and a few ms for
typical files. The event loop handles all pending keys before redrawing, so
fast typing, key repeat and pastes cost one re-highlight per frame, not one
per key.

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

Operators that transform text in place (`>`, `<`, `gu`, `gU`, `g~`) don't
touch registers. `apply_operator` hands them to `editor/transform.rs`:

- `shift_lines` measures each line's indent, adds or removes one level, and
  rebuilds it with spaces (`expand-tab`) or tabs plus spaces, as Vim does,
  so mixed indentation gets tidied rather than made worse.
- `change_case` replaces the range with its re-cased text (which may change
  length, e.g. `ß` → `SS`).

`J` and auto-indent live there too. Auto-indent is `indent_for_new_line`:
the current line's indent, plus a level after an opening bracket. Typing a
closing bracket on a line that's only indentation removes a level.

The editor pane draws tabs as spaces up to the next tab stop (`tab-width`).
`ui/editor_view/line.rs` is the one place that maps character columns to
display cells, so the cursor, text and highlights stay aligned with tabs
and wide characters.

Motion details:

- A target function returns `None` when the motion fails (`fx` with no `x`):
  the cursor stays put and a pending operator is cancelled.
- `Range::from_motion` applies Vim's `:h exclusive` rules. An exclusive
  motion ending at the start of a later line covers whole lines if it began
  at or before the first non-blank (so `d}` removes a paragraph cleanly),
  and otherwise stops at the end of the previous line. `dw` is
  special-cased first, as in Vim: if the last word ends its line, the
  operation stops there.
- `f`/`t`/`F`/`T` store a `CharSearch` on the editor for `;` and `,`.

Single-key shortcuts (`x`, `D`, `C`, `s`, `S`, `Y`, `X`) apply an operator
over a fixed motion. Pastes read registers (`editor/registers.rs`).

### Visual mode

- One `Mode::Visual`, with the kind (`Char` for `v`, `Line` for `V`) on the
  editor's `Selection` (`editor/selection.rs`). Both kinds share one
  `[keys.visual]` table, cursor shape and theme scope. The statusline shows
  `V-LINE` for line selections.
- A selection is the anchor plus the cursor, and `Selection::range` turns
  it into the same `core::Range` that motions and text objects produce.
  So nothing new is needed to operate on it:
  - Motions just move the cursor, which extends the selection.
  - Text objects call `Editor::select_range` instead of applying an operator.
  - Operator commands call `Editor::take_selection`, which ends visual mode
    and returns the range, then apply the operator to it right away.
- Leaving visual mode remembers the selection for `gv`.
- The editor pane highlights the selection (`ui.selection`), including one
  cell for a selected line break so selected empty lines are visible.

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

### Search

- `core::search::SearchPattern` compiles a pattern (Rust `regex`, with Vim's
  `\<`/`\>` translated to `\b`, and smart case) and finds the next or
  previous match from a position, wrapping if allowed. Matches don't span
  lines.
- The bottom-line prompt is shared. `CommandLine` has a `Prompt` (`:`, `/`,
  `?`), so search reuses command mode's keymap and editing, and
  `command_line_execute` hands search prompts to `Editor::finish_search`.
- Typing at a search prompt calls `Editor::update_search_preview`, which
  moves the cursor to the first match (incremental search). Leaving the
  prompt any other way puts the cursor back where it started.
- `Editor::search_highlight` gives the UI the pattern to highlight: the one
  being typed, or the last search unless `:noh` hid it.

### The command line

`editor/command_line/` holds the prompt's text and cursor, its editing
operations, and two `History`s (one for `:`, one for searches). The
histories are browsed by prefix, as in Vim. `<Tab>` completion is split in
two:

- The commands layer (`commands/builtin/command_line.rs`) works out the
  candidates, since it can see the registry and the filesystem: command
  names and aliases for the first word, and otherwise whatever the command
  declares with `Command::complete_args` (`ArgCompletion::Path` or
  `ArgCompletion::Buffer`).
- `CommandLine` applies them. A single candidate is inserted and completion
  ends, so `<Tab>` again goes into a directory. Several candidates are
  cycled, with a counter in the prompt.

Any other edit ends completion and history browsing. Edits at a search
prompt re-run the live preview.

### Ranges and `:s`

`command_line_execute` parses `[range] name[!] [arguments]`
(`commands/ex/`, pure functions with their own tests). The range becomes
`Invocation::range` (0-based, inclusive rows). The text after the name is
passed untouched as `Invocation::arg_text`, as well as split into `args`,
because `:s` patterns can contain spaces. The command name stops at the first
character that can't be part of a name, so `s/a/b/` needs no space. A range
with no command goes to its last line, which is what makes `:42` work.

`:s` (`commands/builtin/ex_commands.rs`) compiles the pattern as a
`SearchPattern` and translates Vim's replacement syntax (`&`, `\1`, `\n`)
into the `regex` crate's. It edits from the bottom of the range upwards, so
a replacement that adds line breaks doesn't shift the rows still to be
processed, and it sets the last search so `n` and highlighting follow.

### Repeat (`.`)

`app/repeat.rs` records the keys of each command sequence that starts and
ends idle (normal mode, nothing partially typed): `dw`, `ct)foo<Esc>`,
`vjd`. If `Document::edit_count` moved during it, the sequence becomes the
last change. Undo/redo don't count as edits, and sequences that went
through the `:` line are skipped. `.` is an ordinary command that sets
`Editor::request_repeat`; the app then feeds the recorded keys back
through the normal input path. That's why recording happens at the app
level: only the app sees keys.

Recording keys rather than commands makes every change repeatable,
including insert sessions and future plugin commands, without each command
having to support it. A leading count is stored separately so `3.`
replaces it, and a `"x` register prefix is kept.

### Commands that take a character

A command registered with `.takes_char()` (`f`, `t`, `F`, `T`, `r`) gets
the next key as its argument. When the app sees such a command without a
character, it calls `InputHandler::await_char`. The next printable key, even
one bound to a command (`f:`), completes it via `Invocation::char_arg`;
anything else cancels it. Marks (`m`), macros (`q`, `@`) and plugins can use
the same mechanism.

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

## Events and hooks

`events::Event` lists what hooks can observe: `Opened`, `BeforeSave`,
`AfterSave`, `ModeChanged` and `BufferChanged`. Hooks are registered on the
`CommandRegistry` with `on(kind, source, handler)`, next to commands, so the
registry stays the single extension point. A hook gets the same `Context` as
a command.

There are two delivery styles:

- **Before** events fire synchronously from the command doing the work
  (`write` fires `BeforeSave` with `CommandRegistry::fire`). A hook can edit
  the document first, or return an error to cancel.
- **After** events are queued with `Editor::emit`, because the editor has no
  access to the registry. The app dispatches them once each key has been
  handled (`App::dispatch_events`). That's also when it emits
  `BufferChanged` if the key edited the text. Hooks can queue further
  events, so dispatch repeats, but stops after 8 rounds with an error so
  two hooks can't trigger each other forever. A hook's own edits form a
  separate undo step.

`[hooks]` in config maps an event to a list of commands
(`CommandRegistry::on_run_commands`), and unknown command names are
reported at startup, like the keymap.

## Lua plugins

`plugins/` embeds Lua 5.4 through `mlua`, with Lua bundled so users need
nothing installed. `PluginHost` owns the Lua state and sets up the global
`editor` table. While a file loads, `editor.command`, `editor.on` and
`editor.map` only record registrations, keeping each Lua function in the
Lua registry. `PluginHost::install` then adds them to the same
`CommandRegistry` and `Keymap` the built-ins use. Each plugin command or
hook is an ordinary Rust closure holding the Lua state and the function's
registry key, so plugin commands get the palette, `:` execution, key
binding, undo, `.` and hooks with no special cases.

A callback runs inside `Lua::scope` (`plugins/api.rs`). The `ctx` methods
are scoped functions that borrow the command's `Context` through a
`RefCell`, so Lua can reach the editor only for the duration of the call: a
`ctx` saved and used later gives a Lua error, not undefined behaviour.
`ctx:run` executes any command, including other plugin commands, re-entering
Lua safely. Lua errors become the command's error (first line, without the
traceback).

Loading order: `init.lua`, then `plugins/*.lua` by name, so later files can
override earlier commands and bindings. Errors are collected per file; a
broken file doesn't stop the rest. `main` loads plugins before opening the
startup files, so `opened` hooks see them.

## Plugin roadmap

Done: named commands, runtime keybinding, event hooks and Lua plugins. Next:

1. **A wider API**, driven by real plugins: selections, registers, search,
   buffers, options, and creating pickers from Lua.
2. **Plugin UI.** Statusline segments and gutter columns become lists of
   components that plugins can contribute to.
3. **Registering at runtime.** Registrations made inside a callback (rather
   than while loading) are collected but not yet installed.
4. **Isolation.** If sandboxing untrusted plugins becomes a goal, a WASM
   runtime could sit behind the same command and hook interfaces.

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

- Visual block mode (`<C-v>`) and visual-mode `:` ranges (`:'<,'>`).
- Search as a motion for operators (`d/foo`).
- `:s` follow-ups: the `c` (confirm) flag, `&`/`:&&` to repeat the last
  substitution, and `:g/pattern/command`.
- `Esc` immediately followed by another key in the same terminal read is
  parsed as Alt+key (this is how terminals encode Alt). Vim uses a short
  `ttimeout` to tell them apart; we don't yet. It only shows up with very
  fast input, such as pasted keystrokes.
- `.` with a count only replaces a count typed *before* the change: after
  `d2w`, `3.` replays as `3d2w` (six words) where Vim deletes three.
- Language-aware indentation (tree-sitter indent queries), and `=` to
  re-indent. Auto-indent currently copies the previous line and reacts to
  brackets only.
- `N%`, `gJ` (join without spaces), and `.` repeating a visual operation on
  the same amount of text, as Vim does.
- Numbered delete registers `1`-`9` and the small-delete register `-`.
- Resizing windows (`<C-w>+`, `<C-w>>`, `<C-w>=`) and moving them around.
- Window-local options, and a statusline per window.
- File listing runs on the UI thread. It's capped by `max-files`, but very
  large trees could be walked in the background and streamed into the picker.
- Asking before overwriting on external changes, and reloading
  automatically when the file changes and there are no local edits.
- Writing through hard links keeps the old inode's other names pointing at
  the old contents (the rename replaces the file). Vim's `backupcopy`
  handles this; we don't yet.
- Non-UTF-8 encodings are refused rather than supported.
- Incremental syntax parsing (feeding edits to tree-sitter and reusing the
  previous tree), and highlighting only the visible part of very large
  files. Language injections (e.g. regex or SQL inside strings) are turned
  off.
- More languages, and choosing a language for files without an extension.
- Timeout for ambiguous key sequences such as `jk` in insert mode (Vim's
  `timeoutlen`).
