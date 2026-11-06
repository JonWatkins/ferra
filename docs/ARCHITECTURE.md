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

Two things happen around this. Terminals send Alt+x as Esc then x, so Esc
quickly followed by a key arrives as Alt+key. `App::split_alt` turns an
Alt key with no binding in the current mode back into Esc and the key
(Neovim does the same), before it's recorded for macros.

And when a key sequence is left half typed, the event loop polls with a
deadline (`editor.key-timeout`, from the last key) instead of blocking.
When it passes, `InputHandler::timeout` settles the sequence: keys bound on
their own (`g` alongside `gg`) run (`Keymap::command_at`); in insert and
command mode the first key is typed as text and the rest looked up again;
otherwise (`g` in normal mode) it keeps waiting, like Vim. Counts,
registers and `f`-style arguments never time out. Keys and timeouts share
`App::tracked`, which records sequences for `.`.

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
vertical splits, separated by one-cell lines. Each split's children share
its space by weight: equal after a split or close, and set from actual cell
sizes when a window is resized, so sizes are proportions that survive the
terminal changing size. Resizing works in cells, so the UI records the area
it drew the windows in (`Windows::note_area`). `Layout::resize` changes the
innermost split in the right direction that holds the window: growing takes
cells from the windows after it first, then the ones before, and every
window keeps at least one cell.

Status lines are drawn per window by default (`statusline = "windows"`).
A window's status line goes in the row right below it, which is always
either the horizontal separator under it or the screen's status row, so
the layout is the same in both modes: the UI just draws status lines where
it would draw `─` separators. Windows side by side share the bottom row,
each drawing in its own columns. `StatusLine::new` draws the active
window's and `StatusLine::inactive` the others' (name, `[+]`, their own
cursor), in `ui.statusline.inactive`.

The active status line is built from `[editor.statusline-layout]`: lists
of `StatusElement`s for the left, center and right (Helix's design rather
than Vim's `%`-format strings). Each element renders to one styled span,
possibly empty. A file name gets the width all the other elements leave,
shortened from the left, so everything else always stays visible; the
center is centred between the other two when there's room.

Moving windows rearranges the tree. `exchange` (`<C-w>x`) and `rotate`
(`<C-w>r`) reorder the children of the split holding the window, leaving
the sizes where they were; rotating refuses when that split holds another
split, as Vim does. `move_to_edge` (`<C-w>H` and friends) removes the window
and wraps the rest of the tree in a new split with the window on that side,
or joins the root split if it already runs that way.

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
(`SwitchBuffer`, `OpenFile`, `RunCommand` or `Paste`), the query, and fuzzy-matched results
(`fuzzy.rs`, a small dependency-free matcher that favours word starts and
consecutive runs). `Mode::Picker` has its own `[keys.picker]` table; unbound
printable keys filter the list. The command palette lists the registry's
commands, except ones that drive the prompt or picker or need a character.
`ui/picker.rs` draws the popup over the editor pane, cutting labels (paths)
from the start and descriptions from the end. A picker built with
`keep_label_start` cuts labels at the end instead: the register picker's
labels start with the register name and a one-line preview of its text,
so typing filters by either.

The read-only registers `"/`, `":` and `"%` live elsewhere in the editor
(the last search, command-line history, the document's path), so
`Editor::read_register` answers them before falling back to `Registers`;
pastes and the register picker go through it.

`".` holds the text typed in the last insert session. The editor collects
it while in insert mode (typed characters, line breaks, tabs, pastes and
`<C-r>` insertions, with backspaces taking characters back off) and stores
it when insert mode ends, if anything was typed. `<C-r>{register}`
(`insert_register`) takes the register name as its next key, like `f`;
in insert mode it inserts the text exactly, without auto-indent, and on the
`:`/`/` line the first line.

The file picker lists files with `files::walk_files`, which uses the
`ignore` crate (ripgrep's walker): `.gitignore`, `.ignore` and git excludes
are honoured even outside a repository, hidden files are optional, `.git` is
never listed, and listing stops at `max-files` (`[editor.file-picker]`). The
fuzzy matcher gives matches in a path's file name extra weight, so `main`
finds `src/main.rs` before `domain/lib.rs`.

The listing runs in the background. The `file_picker` command opens an
empty picker marked as loading a job number and leaves a
`FileListRequest`; the app (`app/background.rs`) walks the tree on a thread
that sends batches of a few hundred paths through a channel. The event loop
polls for them every 30 ms while a job runs (and blocks on input as usual
otherwise), and `Picker::extend` matches only the new items against the
query and keeps the highlighted item. Updates for a job whose picker has
gone are dropped, and closing the picker sets a flag the walker checks, so
it stops.

## Syntax highlighting

`syntax/` wraps tree-sitter:

- `languages.rs` lists each built-in language by name: its grammar and
  queries. A language built on another (TypeScript on JavaScript, C++ on C)
  lists its own highlight query first and the base one after, joined, so its
  patterns win. Adding a language is one entry, its grammar crate, and a
  `[languages.name]` table in the default config.
- Which file is in which language is config, not code:
  `config::Languages` (the `[languages]` table, shaped like Helix's
  `languages.toml`) matches the exact file name, then the extension, then
  the interpreter on a `#!` first line (via `env`, ignoring flags and
  version suffixes). The editor holds the table so the syntax engine and the
  status line agree, and detection reruns as you type, so writing a `#!`
  into a new file picks its language.
- Injections (a language inside another) are resolved by
  `injected_language`: the name the query gives (`rust`,
  `markdown_inline`) or an extension from `[languages]` (`py`, `js`), so
  Markdown code fences work with either. Injected grammars load lazily from
  the same compiled set. Markdown is two grammars: blocks, and an
  injection-only inline grammar (`INJECTION_ONLY`) for emphasis, links and
  code spans; its injection query is ours, adding `include-children`
  because the `inline` node has hidden children. Rust's macro injections
  are off for speed.
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

`Registers::store` follows Vim's rules for deletes into the unnamed
register: a line or more (or a delete over a jump motion, `d%`, `d}`, via
`StoreKind::DeleteOverJump`) shifts `"1`-`"8` down and lands in `"1`;
anything smaller goes to `"-`. Yanks go to `"0`. Naming a register, or the
black hole `"_`, skips all of these.

### Comments

`gc` is an operator (`Operator::Comment`), so it takes any motion or text
object, `gcc` comes from the operator-pending `c` = current line binding,
and `.` repeats it. It works on the lines its range touches
(`editor/comment.rs`), as Neovim does: if every non-blank line is already
commented they're uncommented, otherwise all are commented with the marker
at the shallowest indent; blank lines are left alone. The markers come from
the file's language in the `[languages]` table (`comment = "//"`, or
`block-comment = ["<!--", "-->"]` wrapping each line), so a language added
in config gets commenting even without a grammar. Lines are edited only
where they change, so marks elsewhere on them stay put.

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

### Visual block mode

- `SelectionKind::Block` is a third kind of selection. Its shape is a
  `core::Block`: rows, plus first and last *display* columns, so it lines up
  on screen with tabs and wide characters (`core/columns.rs` converts
  between character and display columns, for the UI too). A character partly
  inside the block (a tab) counts as inside. `to_eol` (`$`) makes each row
  run to its own end; motions report through `Editor::note_block_motion`,
  and only `$` and vertical moves keep it.
- `Selection::range` gives a block's whole lines, so line-based commands
  (`J`, `:`, `X`, `Y`, `>`) work unchanged. Character operators check
  `Editor::take_block` first and go to `editor/block.rs`, which edits row
  by row from the bottom up so earlier rows' positions stay put.
- Yanks store a `blockwise` register (one line per row). Pasting one
  inserts each row at the same display column on successive lines, padding
  short lines and adding lines at the end, and padding pieces to the block's
  width when text follows.
- `I`, `A` and `c` type on the first row only, with a `BlockInsert` noting
  where and which other rows. When insert mode ends (`set_mode`), the text
  typed on that row is copied to the others, inside the same undo step as
  the insert session. Typing that leaves the row (a line break) copies
  nothing, as in Vim.

### Files

- `core::TextFormat` splits file contents into buffer text (always UTF-8
  and `\n`, no final newline) and the format to write back (encoding and
  byte-order mark, LF/CRLF, final newline). An untouched file round-trips
  byte for byte. Encodings use `encoding_rs`: a byte-order mark decides,
  then valid UTF-8, then the configured fallback (Windows-1252, where every
  byte means something). Decoding refuses bytes invalid in the chosen
  encoding, and encoding refuses characters it can't hold, rather than
  substitute anything. UTF-16 is encoded by hand, since `encoding_rs`
  follows the web standard of never producing it. The document remembers
  the encoding the file has on disk, so `:fenc` converting it counts as an
  unsaved change.
- `files` is the only module that touches the disk. Writes go to a temp file
  in the same directory, are fsynced, then renamed over the target. Existing
  permissions are copied and symlinks are followed. A rename gives the file
  a new identity, so (like Vim's `backupcopy=auto`) the file is overwritten
  in place instead when it has other hard-linked names, when its owner or
  group can't be kept (the temp file is `chown`ed to match, and if that's
  refused), or when the directory doesn't allow a temp file. In-place
  writes copy the original to a backup first and put it back if writing
  fails.
- `Document` holds the path, format and two pieces of save state:
  - **Modified tracking:** `History` gives each committed state a
    `Revision`. The document is modified when its revision differs from the
    saved one, so undoing back to the save clears `[+]`.
  - **External changes:** the file's mtime at read/write time. A save
    refuses to overwrite a file whose mtime has changed since, or an
    existing file the document never read, unless forced with `!`.
- Noticing external changes: `Document::disk_state` compares the file's
  mtime with the one it last read or wrote (`Changed`, `Deleted`).
  `Editor::check_disk_changes` runs on `FocusGained` (the app turns on
  terminal focus reporting) and `:checktime`, and `switch_to` checks the
  buffer it switches to. An unmodified buffer reloads with
  `Document::reload_in_place`, which replaces only the part of the text
  between the common prefix and suffix, as one undo step, so the cursor and
  marks outside it stay put and `u` undoes it. `:e!` uses the same reload.
  A modified buffer (or a deleted file) gets one warning;
  `acknowledge_disk_state` stops it repeating until the file changes again.
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
- After an operator, searches are motions. `/` and `?` in operator-pending
  mode move the pending operator into the search state while the prompt is
  open (cancelling the prompt drops it). `finish_search` then returns
  `SearchDone::Operator` with the match instead of moving, and
  `command_line_execute` applies the operator through the exclusive
  `SEARCH_MATCH` motion, whose target the command supplies (as marks do).
  `n`/`N` do the same with `Editor::find_next`, which finds a match without
  moving. The usual exclusive rules apply, so `d/^foo` from a line start
  takes whole lines. `.` repeats it because the app records search prompts
  for `.`; only `:` prompts are left out.
- In visual mode, `open_search` keeps the selection (so it's drawn, following
  the preview) and remembers it. Finishing or cancelling the prompt goes
  through `return_from_search` (`Editor::leave_command_line` for `<Esc>` and
  backspacing out), which puts visual mode and the selection back; a finished
  search then moves the cursor, extending it.

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
The last `:s` arguments are kept on the editor (`last_substitute`), so `&`,
`:&&` and `g&` parse them again with different flags or patterns.

`:s///c` opens a `ConfirmSession` (`editor/confirm.rs`) that walks forward
through the matches, one at a time, with `SearchPattern::next_replacement`
expanding the replacement for that match. Answers arrive as the next key
(`request_char("substitute_confirm")`), so there's no extra mode. While a
session is open, `finish_command` leaves the undo step open, so the whole
session undoes at once; a non-character key (`<Esc>`) ends it.

`:g` finds the matching rows first, then asks the document to track them
(`Document::track_rows`): insertions and deletions shift tracked rows, and
rows whose line is deleted are forgotten. It then runs the command with
the cursor on each remaining row, through the same `run_command_line` as
the `:` prompt. It's one command, so it's one undo step, and `:g` inside
`:g` is refused.

`|` is split in `run_command_line`, after the command name is known, since
how a command treats `|` is a property of the command (`Command::bar`, like
Vim's `-bar`): it separates commands (the default), it's part of the
arguments (`:g`, `:v`, `:normal`, so `:g/x/a | b` hands `a | b` to each
line), or it's only a separator after a `/pattern/replacement/` (`:s`,
where `|` is regex alternation). `ex::split_bar` does the splitting; the
next command runs after this one succeeds.

`:normal` can't feed keys itself (only the app can), so it queues a
`NormalRun` (keys and position) per line, and `:g` calling it once per line
just adds to the queue. After the command, the app (`app/normal.rs`) plays
the queue like a macro: the rows are tracked through the edits the keys
make, a failing command stops the run, an unfinished one gets an `<Esc>`,
and `Editor::hold_undo` keeps the undo step open across every key.

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
having to support it. Each recorded key notes whether the input layer took
it as a count (`InputHandler::took_count`, in normal and operator-pending
mode only), since only it can tell the `2` of `d2w` from the `2` of `i2` or
`f2`. So `3.` drops every count digit and puts `3` first (`d2w` becomes
`3dw`, as in Vim), and keeps the new count for later `.`. A `"x` register
prefix is kept. A sequence counts as a change
when it edited the buffer it started in (`EditStamp`: buffer and edit
count), so switching buffers never does. If the change was a paste
from `"1`-`"8`, `.` moves the prefix on one number first, so `"1pu.u.`
steps back through older deletes.

### Macros and marks

Macros are recorded and replayed as keys, like `.`, so they live in the
app (`app/macros.rs`). `q` and `@` are ordinary commands that set an
`Editor::request_macro`, which the app acts on after the command. While
recording, every key the user presses is kept. Stopping drops the `q` that
stopped it and stores the rest in the register as key notation (via
`KeyPress`'s `Display`, which round-trips with `parse_keys`), so a macro is
ordinary register text. Playing parses it and feeds the keys through
`handle_key`. Keys a macro plays aren't recorded into a macro being
recorded, a failing command stops playback, and nesting deeper than 100
stops with an error.

Marks are kept by each `Document` (`document/marks.rs`) and adjusted by its
`insert` and `delete`, so they move with text inserted or deleted before
them. Global marks (`A`-`Z`) record which buffer they're in and reuse that
buffer's mark under the same letter. Undo applies changes below `Document`
and doesn't move marks, so positions are clamped when used. `'` and `` ` ``
are motions whose target comes from the mark (`MotionArgs::target`), so they
work after operators and in visual mode.

### Jump list and change list

A *jump* is a big move: `gg`, `G`, `%`, `{`/`}`, searches and `n`, marks,
`:42`, and switching buffers. Motions opt in with `Motion::jump()`;
other places (search, `:e`, `]b`, the picker, `goto_line`) call
`Editor::push_jump` before they move. That records where the cursor was in
the `JumpList` (`editor/jumps.rs`), which works like browser history:
`<C-o>` steps back (remembering the current spot first, so `<C-i>` can
return), and jumping while part-way back drops the entries ahead.
Entries on the same line of the same buffer are merged, the list holds 100,
and entries in closed buffers are skipped and dropped. `push_jump` also
sets the origin used by the `''` and `` ` `` marks.

Each `Document` keeps its own change list: `insert` and `delete` record
where they happened (consecutive edits on one line update the last entry).
`g;`/`g,` step through it, and `'.` is its newest entry. Positions aren't
adjusted for later edits; they're clamped when used.

### Clipboard and pasting

`clipboard::Clipboard` is a two-method trait (`get`, `set`). The app gives
the registers a `LazySystemClipboard` (the `arboard` crate, connected on
first use, so startup and tests never talk to the OS); tests use
`MemoryClipboard`. `Registers` decides which register names use it, per
`ClipboardMode`: `"+`/`"*` only (default), also the unnamed register, or
never. Pastes go through `Registers::read`, which can fail and returns an
owned value. Text ending in a line break reads as linewise, as in Vim. A
failed clipboard write never cancels the yank or delete: the text is still
stored, and the error is reported after the command.

Bracketed paste is on while the editor runs. `Event::Paste` goes to
`App::handle_paste`, bypassing key bindings and auto-indent. A paste that
brings its own indentation replaces the auto-indent on an otherwise blank
line.

### Commands that take a character

A command registered with `.takes_char()` (`f`, `t`, `F`, `T`, `r`) gets
the next key as its argument. When the app sees such a command without a
character, it calls `InputHandler::await_char`. The next printable key, even
one bound to a command (`f:`), completes it via `Invocation::char_arg`;
anything else cancels it. A command that only sometimes needs a key (`q`
takes a register to start recording, but not to stop) calls
`Editor::request_char` instead, and the app then waits for the key the same
way.

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
2. **Plugin UI**: plugins drawing their own windows and modals, and adding
   to existing ones (see "Plugin windows and decorations" below).
3. **Registering at runtime.** Registrations made inside a callback (rather
   than while loading) are collected but not yet installed.
4. **Isolation.** If sandboxing untrusted plugins becomes a goal, a WASM
   runtime could sit behind the same command and hook interfaces.

## Roadmap: deferred features

Planned, not started: plugin UI, LSP support, an agent harness, an embedded
terminal and Git support. They share three foundations, which should come first.

### Foundations

1. **Background work and an event channel.** Started: `app/background.rs`
   runs jobs on threads that report through a channel the event loop
   checks while they run; the file picker's listing is the first job. LSP
   replies, agent output and a shell's output would be more kinds of
   update (each handled like a key: it can edit, fire hooks and request a
   redraw). Long-lived jobs (a language server) will want the loop woken
   by the channel rather than polled, and plugins would use the same
   channel for timers and jobs.
2. **Floating windows and scratch buffers.** The picker is already a popup
   over the editor; generalise it into floats with a position, size, border
   and title, showing any buffer. Plus scratch buffers that belong to a
   plugin or feature (not files, never "modified"), and key bindings local
   to one buffer, so a popup can have its own `q` to close.
3. **Decorations.** Highlights over ranges, virtual text (end of line or
   inline), gutter signs and status line segments, all kept in place as
   text is edited (the same bookkeeping as marks and `track_rows`), drawn by
   the editor pane after syntax highlighting.

### Plugin windows and decorations

Built on the foundations, exposed to Lua in the style of Neovim's
`nvim_open_win` and extmarks:

- `editor.buffer.create()` / `set_lines`: a scratch buffer a plugin fills.
- `editor.window.open(buffer, { float = { row, col, width, height, border,
  title } })`, or as a split; then `focus`, `close`, `set_config`.
- Buffer-local `editor.map` and `on("buffer-closed")`, so a plugin's modal
  handles its own keys.
- `editor.decorate(buffer, range, { highlight, virtual_text, sign })`,
  returning a handle to update or clear: diagnostics, git signs, inline
  hints, search counts.
- Status line and gutter components that plugins add to the built-in ones.

### LSP support

A client per language server, configured in the `[languages]` table
(`language-server = { command = "rust-analyzer" }`, as Helix does), running
in the background and reporting through the event channel. In order:
diagnostics (decorations and a picker list), hover (a float), go to
definition and references (the jump list is ready for this), completion (a
float in insert mode), rename, code actions and formatting (as one undo step
each, through the command registry).

### Agent harness

An AI agent working inside the editor: a chat buffer in a split or float,
streaming replies through the event channel. Its tools are the command
registry (open, search, read, edit), so every change it makes is an
ordinary edit: undoable, repeatable and visible. Proposed edits can be
shown as a diff first, using decorations, and accepted or rejected per hunk.
Built as a plugin-level feature on the same APIs, so a different agent
could be swapped in.

### Embedded terminal

A shell in a window: a pseudo-terminal (`portable-pty`) feeding a terminal
emulator (`vt100` or `alacritty_terminal`) whose screen is drawn into the
window, with output arriving through the event channel. A terminal mode
sends keys to the shell, and a key (like Neovim's `<C-\><C-n>`) goes back
to normal mode, where the scrollback is an ordinary read-only buffer to
search and yank from.

### Git support

The everyday Git features, without leaving the editor:

- **Change signs in the gutter**: added, changed and removed lines compared
  with the index (or `HEAD`), updated as you type, plus a status line
  segment with the branch and counts.
- **Hunks**: `]c`/`[c` to move between changes, preview a hunk's diff in a
  float, and stage, unstage or reset (undo) a hunk, or the whole file.
- **Diff views**: the buffer side by side with the index, `HEAD` or any
  revision, scrolling together with matching lines aligned; also a diff of
  the whole working tree to browse file by file.
- **Blame**: the author and summary of the current line as virtual text
  (optional), and a blame view of the whole file that opens a commit's
  diff.
- **Status and commits**: a picker of changed files (stage or unstage from
  it), writing the commit message in a buffer and committing on save,
  amending, and branches (list, switch, create) in a picker.
- **History**: the log of a file or a line range, opening any version.
- **Conflicts**: jumping between conflict markers and keeping ours, theirs
  or both, as one undo step each.

Reading (diffs against the index and `HEAD`, blame, status) can use a Git
library in-process (`gix`, pure Rust, or `git2`), with the line diff from a
diff crate (`imara-diff` or `similar`); writing (staging, committing,
switching branches) can shell out to `git` to get its exact behaviour and
hooks. Large repositories mean doing the work in the background. It all
sits on the foundations above: decorations for signs and blame, floats for
previews, the event channel for background results, and pickers for
status, branches and history.

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

- Visual block mode: `>`/`<` shift whole lines (Vim shifts from the
  block's left edge), a tab partly inside a block is taken whole (Vim splits
  it into spaces), and `gv` doesn't remember `$`.
- Alt keys that *are* bound can't be told apart from a fast Esc then that
  key (terminals send the same bytes), so binding `<A-j>` in insert mode
  means a quick `<Esc>j` runs it.
- Language-aware indentation (tree-sitter indent queries), and `=` to
  re-indent. Auto-indent currently copies the previous line and reacts to
  brackets only.
- `.` repeating a visual operation on the same amount of text, as Vim does
  (it replays the keys, so it repeats the motion instead).
- `"=` (the expression register).
- Window-local options, and per-window or plugin-supplied status line
  elements (the layout is one setting for the active window).
- External changes are only noticed on focus, buffer switches and
  `:checktime`, not by watching files; and by mtime only, so a change within
  the filesystem's timestamp resolution of our own write can be missed.
- Extended attributes and ACLs (e.g. macOS file tags) aren't carried over
  by the rename used for ordinary saves.
- Guessing legacy encodings (telling Shift-JIS from Windows-1252 by
  content): files that aren't UTF-8 use the one configured fallback.
- Clipboard over SSH (OSC 52), and recording pastes into macros and `.`
  (both currently replay without the pasted text).
- Incremental syntax parsing (feeding edits to tree-sitter and reusing the
  previous tree), and highlighting only the visible part of very large
  files. That would also make Rust's macro injections affordable (they're
  off: 2.5x slower on macro-heavy files).
- Loading grammars and queries at runtime, Helix-style, instead of
  compiling them in (each adds to the binary: 15 languages make it about
  17 MB in release, up from 8 MB with 5).
- `.` and macros replay keys without timing, so a short binding that ran
  because the key timeout passed (`g` alongside `gg`) replays as waiting
  for the longer one. Text typed by a timeout in insert mode replays fine.
