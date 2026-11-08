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
| `lsp`       | Language server clients: processes, JSON-RPC, document sync, diagnostics. | No |
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

A few things happen around this. Terminals send Alt+x as Esc then x, so
Esc quickly followed by a key arrives as Alt+key. Where the terminal has
the kitty keyboard protocol, `App::run` asks for it
(`terminal::enable_keyboard_enhancement`, with only
`DISAMBIGUATE_ESCAPE_CODES`; crossterm's query waits for the terminal's
answer, and one that doesn't answer is used the old way), and pops it on
exit. Esc then arrives on its own, so a bound `<A-j>` only runs for Alt+j.
Otherwise, or for an Alt key with no binding in the current mode,
`App::split_alt` turns it back into Esc and the key (Neovim does the
same), before it's recorded for macros.

The protocol also reports `<C-[>`, `<C-i>` and `<C-m>` apart from `<Esc>`,
`<Tab>` and `<CR>`, which older terminals send for them.
`App::as_legacy` maps them back (`KeyPress::legacy_equivalent`): `<C-[>`
always, since Vim users press it as Esc, and the other two unless the
current mode's bindings use them (`Keymap::uses`), so binding `<C-i>` in
normal mode is how to have it apart from Tab there.

And when a key sequence is left half typed, the event loop polls with a
deadline (`editor.key-timeout`, from the last key) instead of blocking.
When it passes, `InputHandler::timeout` settles the sequence: keys bound on
their own (`g` alongside `gg`) run (`Keymap::command_at`); in insert and
command mode the first key is typed as text and the rest looked up again;
otherwise (`g` in normal mode) it keeps waiting, like Vim. Counts,
registers and `f`-style arguments never time out. Keys and timeouts share
`App::tracked`, which records sequences for `.`.

The splash screen (`ui/splash.rs`) is drawn over the editor pane while
`Editor::splash_visible`: it was asked for (`open_all` with no files) and
the editor still holds just the untouched starting buffer in one window.
After every key the app calls `update_splash`, which turns it off for good
once that stops being true, so undoing back to an empty buffer doesn't
bring it back. Its menu is built when drawing, with `Keymap::keys_for`
finding the shortest keys bound to each command, so rebindings show up and
unbound commands are left out.

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

The scroll position stays in the UI (`ViewState`, per window and buffer),
since it's settled while drawing, when the cursor has to be kept in sight.
After drawing a window the UI records where its view starts
(`Windows::note_top`), and scrolling commands (`commands/builtin/
scrolling.rs`: `<C-d>`, `<C-f>`, `<C-e>`, `zz`, `H`...) read it back
through `Windows::view_top`, which also applies the scroll the next draw
will do for the cursor's current row (`windows::follow_cursor`, shared with
the UI's `Viewport`), so commands run before a redraw (a macro's) agree
with it. A command moves the view by asking for a new first row
(`Windows::request_top`), which the UI applies before drawing that window
next. The window's height comes from the layout (`Windows::height`), so it
works before the first draw too. Like Vim, a scroll that leaves the
cursor inside the `scroll-off` margin moves the cursor out of it, not the
view back. `H`, `M` and `L` are a linewise jump motion (`SCREEN_LINE`)
given the row, so operators take them (`dL`).

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
  queries. `indent.rs` holds the indentation rules (see
  [Indentation](#indentation)). A language built on another (TypeScript on JavaScript, C++ on C)
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
  because the `inline` node has hidden children. Rust's macro calls and
  `macro_rules!` bodies are injected as Rust.
- `highlight.rs` highlights a range of rows from a syntax tree. It follows
  `tree-sitter-highlight`'s rules (which ferra used before keeping trees),
  and the tests check it gives the same spans on samples of every language
  and on this repo's own Rust, Markdown and TOML:
  - The locals and highlights queries are compiled as one query, so their
    captures arrive in document order. A local definition records the
    highlight its node gets, and references to it in scope take that
    highlight. Patterns with `(#is-not? local)` skip locals.
  - Of several highlight patterns matching one node, the last wins.
  - Highlights paint over earlier ones in the order they start (by
    position; deeper layers first at the same position; otherwise query
    order). So a node's highlight shows over its parent's, unless the
    parent's pattern came later in the query: TOML's
    `(pair (bare_key)) @property` colours the key, though
    `(bare_key) @type` matches it first. An injected layer wins a range its
    host highlights exactly.
  - Injections in the rows are parsed from the parts of the text their
    nodes cover (`set_included_ranges`, children excluded unless
    `include-children`), up to 4 deep. Matches of an `injection.combined`
    pattern within the rows are parsed together.
  - With locals, the query starts at the top-level node holding the first
    row, so definitions above the rows are seen.

  `HIGHLIGHT_NAMES` are the names captures map to (the one with the most
  parts all in the capture's name), and they double as theme scopes.
- `SyntaxTrees` (`trees.rs`), held by the editor (`Editor::syntax_trees`)
  so indentation and highlighting share it, keeps a tree per document.
  `Buffer` logs every edit (insert and delete, so undo, redo and reloads
  too) as byte offsets and points; asked for a tree, it feeds the edits
  since the tree was built to `Tree::edit`, then re-parses reusing the old
  tree, reading the rope chunk by chunk. A different buffer (`Buffer::id`;
  clones get new ids), a different language or a log that's moved past (it
  keeps the last 4,096 edits) means a full parse. Documents over 4M chars
  aren't parsed. Asking needs only `&self` (a `RefCell` inside), since
  drawing borrows the editor immutably; trees come back as clones, which
  share their nodes.
- `SyntaxEngine` compiles each language's queries on first use, and
  highlights from the trees.

Each window asks the engine for the rows it's about to show
(`ViewState::visible_rows` scrolls a copy of its viewport the way drawing
will). Highlights are cached per document, up to four row ranges (two
windows, say), until the text changes. The editor pane colours text with
them first, then draws search matches and the selection on top.

**Performance.** In a release build, on a 25,600-line Rust file: opening it
(the first parse) takes about 65 ms; a keystroke after that about 2 ms
(1.1 ms without macro injections), where re-highlighting the whole file
took 140 ms; scrolling a line about 0.3 ms. The event loop handles all
pending keys before redrawing, so fast typing and pastes cost one update
per frame, not one per key.

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
  rebuilds it with spaces or tabs plus spaces (the buffer's
  [indent style](#indentation)), as Vim does,
  so mixed indentation gets tidied rather than made worse.
- `change_case` replaces the range with its re-cased text (which may change
  length, e.g. `ß` → `SS`).

`J` and indentation live there too; see [Indentation](#indentation).

### Soft wrap

With `wrap` (the default), a line wider than the window continues on the
next screen rows. `core/wrap.rs` works out where: `row_starts` gives the
char column each row starts at, breaking after a space where it can (a
space at the edge hangs past it rather than pushing the word down),
mid-word only for a word wider than the window, and before a wide char
the edge would cut. Display columns count from the line's start, as
without wrapping, so tabs don't move when wrapping is toggled.

A wrapped line's later rows are narrower: with `wrap-indent` they start
at the line's indentation, after `wrap-marker` (`↪ `, drawn in
`ui.wrap-marker`), like Vim's `breakindent` and `showbreak`
(`TextLayout::continued`, dropped to just the marker when it would leave
less than half the width). `row_starts_indented` breaks those rows at the
narrower width, drawing shifts their text, highlights and cursor right
by `TextLayout::offset`, and `gj`/`gk` count it in the screen column they
keep.

`editor::windows::TextLayout` (the text width beside the gutter, whether
to wrap, the tab width, the continued rows' indent) is the one place that
knows how a window lays out lines, shared by the UI and the commands:

- Drawing (`EditorView::render`) walks lines from the view's top, drawing
  each of a line's rows (`draw_row`) with the line number on its first
  only, and clips syntax, search and selection highlights to each row's
  chars (a selected line break shows on the last row).
- `windows::follow_cursor` keeps the cursor in sight counting screen rows:
  `scroll-off` is in rows, the view always starts at a line's start, and
  a long jump starts looking a screen above the cursor rather than
  measuring every line on the way.
- `gj`/`gk` move through a line's rows, aiming for a screen column that
  `Editor::wanted_screen_column` remembers while the cursor stays where
  they put it, so a short row on the way doesn't lose it. Without wrap
  they're `j`/`k`.
- `TextLayout::bottom` (the last line at least partly shown) is what `L`
  and the scrolling commands use.

`j`/`k` still move by lines, as in Vim, and `<C-d>` and friends scroll by
lines too.

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

### Indentation

Indentation follows the language's rules where it has them
(`syntax/indent.rs`), and a heuristic otherwise (`editor/transform.rs`).

- **Rules** are a small tree-sitter query per language, written for ferra:
  `@indent` marks a node whose lines after its first go one level in (a
  block, an argument list, an HTML element) and `@outdent` a token that,
  starting a line, puts it back at the level of the node it closes (`}`,
  `fi`, `else`, `</div>`). An outdent only counts when its parent is that
  node, so Go's `switch`, which has no rule, doesn't cancel the block
  around it with its `}`. Rust, JSON, TOML, JavaScript, TypeScript, TSX,
  Go, C, C++, Bash, HTML and CSS have rules. Python and YAML don't (there
  indentation is meaning, not layout), nor does Markdown.
- **A line's indentation is relative**: that of the line where its
  innermost open `@indent` node starts, plus a level (or nothing, for an
  outdent line). So nodes opened on one line (`foo(bar(`) add one level,
  and a file indented by a deeper base keeps it. Each row's innermost node
  is found by laying nodes over the rows they cover in start order, so
  later (nested) ones win.
- **Only the rows asked for are looked at**: `compute` takes the kept tree
  (`SyntaxTrees`), runs the query over just their bytes (which catches the
  nodes around them, and closers on them), and reads the indentation of
  lines above from the buffer. So Enter, `o` or a closing bracket costs
  about the same in any size of file: `o` on line 12,000 of a 25,600-line
  Rust file takes 0.7 ms in release, down from 128 ms when every new line
  parsed the whole file. The exception is a node closed by a made-up token
  that ends before the rows (the `{` of `if (x) {` on the line above, which
  tree-sitter may close at once): the made-up tokens before the rows are
  found by walking only the branches with errors, and the query is run
  around each.
- **Unclosed brackets while typing**: tree-sitter either makes up the
  closing token (`MISSING`) or leaves the bracket loose in an `ERROR` node.
  Either way the node is taken to be open to the end of the text, which is
  what makes Enter after `fn main() {` indent. Bash's `then` and `do`
  count as opening brackets there (`fi`, `done` closing; `else`, `elif`
  both). A made-up closer is never an `@outdent`: the `fi` tree-sitter
  supplies for an unfinished `if` sits at the start of the empty line
  being opened, and would otherwise pull it out of the block.
- Lines starting inside a multi-line string, comment or raw text
  (`<script>`) are left alone.
- **`=`** is `Operator::Reindent`, over the lines its range touches, so
  `==`, `=ip`, `gg=G`, visual `=` and `.` all work. Blank lines are
  emptied. A language without rules, or a file without a language, gets a
  message instead.
- **New lines** (Enter, `o`, `O`; `indent_for_opened_line`) are
  indented by the rules once the line exists, so text carried onto it
  (`}` after Enter) counts. Without rules, the line it was opened from is
  copied, plus a level after an opening bracket, or a trailing `:` in
  Python and YAML (and YAML's `: |` and `: >`), and less a level after
  Python's `return`, `pass`, `raise`, `break` and `continue`. Enter
  between a pair of brackets (`{|}`) puts the closing one on its own line
  too, with the cursor on an indented line between.
- **Typing re-indents the line** when what's typed settles where it goes
  (`type_with_indent`), like Vim's `indentkeys`:
  - A closing bracket first on a line: by the rules, or without rules one
    level less.
  - With rules, the line's first word while it's one of the language's
    keywords (`indent::keywords`: Bash's `fi`, `done`, `esac`, `else`,
    `elif`, `then`, `do`), and the `>` ending a closing tag (`</div>`)
    first on a line in HTML and JSX. The indentation the keyword replaced
    is remembered (`Editor::keyword_indent`), and put back if the word
    carries on (`fi` → `file`), since the half-typed line may not parse
    as anything better.
  - Python has no rules, so the `:` ending an `else:`, `elif`, `except`
    or `finally` line moves it to the nearest line above that starts the
    block it continues (`if`/`elif` for `elif`; `try`/`except` for
    `except`...), looking only at lines no deeper than it and than every
    line between. Waiting for the `:` means `elsewhere = 1` is left alone.
- New lines are skipped with `auto-indent = false`; `=` isn't. Files over
  the highlighting limit fall back to the heuristic.
- **The indent style** (`Editor::indent_style`, an `IndentUnit`: tabs or
  N spaces) is the buffer's `:indent` choice, else what `core/indent.rs`
  detected when the file was read or reloaded (with `detect-indent`),
  else `expand-tab` and `tab-width`. Detection looks at the first 1000
  lines: mostly tab-indented means tabs; otherwise the commonest step by
  which a line's indent grows over the previous non-blank line's, ignoring
  one-space steps, `*` comment continuations and lines mixing tabs in
  (alignment, not indentation), with ties going to the narrower step.
  Everything that indents (`>`/`<`, block shifts, Tab, `=`, new lines,
  closing brackets) goes through it; `tab-width` still sets how wide a
  tab is drawn. The `indent` status line element shows it.

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
  and only `$` and vertical moves keep it. `gv` remembers `to_eol` along
  with the corners.
- `Selection::range` gives a block's whole lines, so line-based commands
  (`J`, `:`, `X`, `Y`) work unchanged. Character operators check
  `Editor::take_block` first and go to `editor/block.rs`, which edits row
  by row from the bottom up so earlier rows' positions stay put.
- A tab or wide character cut by an edge is split as in Vim:
  `Block::row` counts its cells outside the block, a delete leaves those as
  spaces, and a yank gives the cells inside as spaces, so the text around
  the block stays put on screen. `I` (or `c`) at a column in the middle of
  a tab turns the tab into spaces first.
- `>` / `<` shift from the block's left edge: the whitespace starting
  there grows or shrinks by one indent (`<` never takes whitespace from
  left of the block) and is rewritten as tabs or spaces as the buffer indents.
  Rows that stop short of the block, or are blank inside it, are skipped.
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
  - **External changes:** the file's `FileStamp` (modified time and
    size) at read/write time. A save refuses to overwrite a file whose
    stamp has changed since, or an existing file the document never read,
    unless forced with `!`. The size catches a change made within the
    timestamp resolution of the last one, unless that kept the size too.
- Noticing external changes: `Document::disk_state` compares the file's
  stamp with the one it last read or wrote (`Changed`, `Deleted`).
  `Editor::check_disk_changes` runs on `FocusGained` (the app turns on
  terminal focus reporting) and `:checktime`, and `switch_to` checks the
  buffer it switches to. With `watch-files` (on by default), the event
  loop also wakes once a second (`WATCH_INTERVAL`, one more deadline
  beside the key timeout's) and runs it through
  `App::check_files_while_idle`, which does nothing unless the app is
  idle (normal mode, no keys pending), so a reload never lands mid-insert
  or under a pending operator. That's a `stat` per open file a second,
  rather than OS file-change notifications, which need a dependency and
  per-platform care (and miss changes on some network filesystems). An unmodified buffer reloads with
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

(`:sort` is parsed and done there too, `commands/ex/sort.rs`: a stable sort
of keys made from each line, the text after or of a pattern's first match,
lowercased with `i` or the first number with `n`. `u` drops lines whose
keys compare equal to the one before, and the command replaces only the
range, as one change.)

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

A change made by an operator on a visual selection is repeated on the same
*amount* of text from the cursor, as in Vim (`visual-repeat`), not by
replaying the motion: after `ved` on a two-letter word, `.` deletes two
characters wherever it's used. Taking a selection (`take_selection`,
`take_block`, block `I`/`A`) notes its `VisualShape`: a number of
characters on one line, a number of lines and the end column on the last,
a number of lines, or a block's rows and display width (and `$`). The app
collects it after each key (`Editor::take_visual_shape`), and the recorder
pairs it with where that key binding began (the start of `"ad` or `3>`),
which it tracks by noting each key that arrives with nothing half typed.
The last change then keeps only the keys from there on, and `.` first
calls `Editor::select_shape`, which selects that much from the cursor, cut
short at a line's or the buffer's end. A count given to `.` is ignored
for these, since the size comes from the selection. A count on a visual
`>` or `<` shifts that many times.

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

Leaving insert mode sets the buffer's `^` mark (an ordinary local mark, so
it follows edits), which `gi` and `` `^ `` go back to. `gf`
(`Editor::open_file_under_cursor`) takes the run of file-name characters
under the cursor, splits off a `:line[:col]` suffix, and opens the first of
"beside the current file" and "from the working directory" that exists,
through `open`, so it's a jump and `<C-^>` comes back.

### Folding

Fold regions come from indentation (`core/fold.rs`): `region_from(row)` is
the row and the lines after it indented more (blank lines among them
included), plus a closing-bracket line at the row's own indent;
`region_around(row)` is that, or the region of the nearest line above, no
deeper, that reaches the row. `za`/`zc` close the region around the
cursor, `zM` every outermost one, `zf` any operator range
(`Operator::Fold`).

Closed folds live on the `Document` (`document/folds.rs`), as sorted,
non-overlapping line ranges in a `RefCell`, so readers with `&Document`
(drawing) can bring them up to date: like the syntax trees, they replay
the buffer's edit log, so undo and reloads move them as well. A fold after
an edit shifts with its lines, an edit within its first line leaves it,
and anything else touching it opens it. A different buffer, or a log
that's moved past, opens them all.

Everything that walks lines treats a closed fold as one: `j`/`k` turn
their count of visible lines into buffer lines (`over_folds`, via
`Document::step_visible_lines`), `gj`/`gk` step with
`next_visible_line`, `enforce_cursor_rules` keeps the cursor on a fold's
first line, `apply_operator_over` widens line ranges to whole folds,
`TextLayout::rows_in` gives a closed fold one screen row (for keeping the
cursor in sight, the view's last line and the highlight prediction), and
drawing shows the first line, unwrapped, with a dim `⋯ N more lines`
(`ui.fold`). `Editor::reveal_cursor` opens folds at the cursor after
searches, jump motions other than `gg`/`G`, mark jumps, undo and redo, as
Vim's default `foldopen` does.

### Reflowing text (`gq`)

`gq` is `Operator::Format` (`gw` with `keep_cursor`) over the lines its
range touches. `core/format.rs` does the work, a paragraph at a time:
blank lines (or a comment marker alone) separate paragraphs and are kept,
as does a change between comment and code. Each paragraph's words are
joined with single spaces and broken greedily at `text-width` (0 means 79,
Vim's fallback; it's buffer-local), every line starting with the first
line's leader: its indentation and the language's line-comment marker with
its space (block comments aren't leaders, so Markdown's `#` headings stay
put). A list item (`- `, `* `, `+ `, `1. `, `1) `) starts its own
paragraph, its later lines indented to its text. The lines are replaced as
one change; the cursor goes to the last one's first non-blank, or with `gw`
stays where it was. `gqq`, `gqgq` and `gwgw` come from operator-pending `q`,
`gq` and `gw` bound to `current_line`.

`auto-wrap` (buffer-local, `off`/`comments`/`all`, like Vim's
`formatoptions` `c`/`t`) does the same as you type: after a non-blank char
lands past `text-width`, `Editor::auto_wrap` breaks the line at the last
space within the width, starting the new line with
`core::format::continuation` (the leader `gq` would use), and moves the
cursor with the text. It's part of the insert session, so one undo and
`.` replays the keys, which wrap the same way.

### Counted inserts

`i`, `a`, `I`, `A`, `gi`, `o` and `O` pass their count to
`Editor::repeat_insert`, which the editor keeps while inserting. When
insert mode ends, `set_mode` calls `finish_insert_repeat` (after
`finish_replace`, before what was typed is stored as `".`), which puts in
the session's typed text (`Editor::typed`, so pastes, completions and
backspaces are as they ended up) the other `count - 1` times: at the
cursor, or for `o`/`O` each on a new line after the current one with its
indentation. It's all one insert session, so one undo step, and `.`
replays the count with the keys.

### Replace mode

`R` (`editor/replace.rs`) is insert mode with a `Replacing` state, as
Vim's replace mode shares insert mode's keys: typed chars (and pastes)
take the place of the char under the cursor, or add at a line's end, and
each records what it overwrote, so `<BS>` puts that back (over text it
didn't type it just moves left). A line break is added without deleting
anything. On leaving, `set_mode` calls `finish_replace` before what was
typed is stored, which types it again for a count (`3Rab`). The status
line shows `REPLACE` (`ui.statusline.replace`) and the cursor takes
`cursor-shape.replace`. `.` replays the keys like any insert.

### Adding to numbers

`<C-a>`/`<C-x>` find a number with `core::increment`: the one the cursor
is in (looking back for a `0x`/`0b` prefix) or the next on the line, read
as decimal (a `-` right before the digits is its sign), hex or binary, as
Neovim's default `nrformats` (no octal). Hex and binary keep their width,
wrapping like an unsigned number of that many digits, and the case of
their letters; decimal with leading zeros keeps its width. Only the
number's chars are replaced, so marks and text around it stay put. In
visual mode (`editor/increment.rs`) each line's selected chars are
searched (whole lines, the charwise part, or the block's columns) and, as
in Vim, only the digits inside them count; `g<C-a>` multiplies the step by
how many numbers have changed so far. The selection is taken like an
operator's, so `.` repeats it on as many lines.

### Word completion

`<C-n>`/`<C-p>` in insert mode complete the word before the cursor
(`editor/completion.rs`). The first press gathers matches: words starting
with what's typed, from the cursor outward through the buffer (forward
for `<C-n>`, backward for `<C-p>`, wrapping, the cursor's own line split
at the cursor), then the other open buffers, without repeats, up to 500.
The nearest goes in at once; later presses step through the matches and
what was typed, as a ring. Each step replaces the word in the buffer, so
the text is always what you see, and the `".` register's text follows.

`<C-x><C-f>` and `<C-x><C-l>` start the same kind of completion with
other matches (`start_completion` is shared): the entries of the
directory the path before the cursor names (relative to the current
file's directory, as completion plugins do, rather than Vim's working
directory; `~/` and absolute paths too), only the part after the last `/`
replaced, directories with a trailing `/`, hidden ones once a `.` is
typed; or the lines (trimmed) that start with the current line's text
after its indentation, nearest above first, then below, then other
buffers. `<C-n>`/`<C-p>` then step through whichever is open.

Completion ends with the first key that doesn't use it: `App::process_key`
clears a flag before each key and ends a completion no completion command
set it for. That covers typing on, moving, `<Esc>` and pastes, and works
the same during macros and `.`. `<C-y>` ends it keeping the match;
`<C-e>` puts back what was typed first. The menu (`ui/completion_menu.rs`)
is drawn at the cursor in the `ui.popup` colours, lined up with the word,
below its row or above when there's no room, showing up to 10 matches and
scrolling to the selected one.

### Clipboard and pasting

`clipboard::Clipboard` is a two-method trait (`get`, `set`). The app gives
the registers the one `clipboard-provider` names (`clipboard::for_provider`);
tests use `MemoryClipboard`, so no test touches the real clipboard:

- `"system"`: `LazySystemClipboard` (the `arboard` crate, connected on
  first use, so startup never talks to the OS).
- `"terminal"`: `TerminalClipboard` writes OSC 52 (`ESC ] 52 ; c ;`, the
  text in base64, `BEL`) to standard output, between frames, and the
  terminal sets the clipboard of the machine it runs on. Reading the
  terminal's clipboard back (an OSC 52 query) is often refused and would
  mean waiting for a reply among the input events, so `get` gives the text
  last copied; pasting from elsewhere is the terminal's own paste.
- `"auto"` (default, `AutoClipboard`): the terminal's when `SSH_TTY`,
  `SSH_CONNECTION` or `SSH_CLIENT` is set (the system clipboard there is
  the remote machine's), otherwise the system one, falling back to the
  terminal's when the system clipboard can't be used (no display).

tmux passes OSC 52 on only with `set-clipboard on`; ferra doesn't wrap it
in tmux's passthrough sequence, which tmux also disables by default.

`Registers` decides which register names use it, per
`ClipboardMode`: `"+`/`"*` only (default), also the unnamed register, or
never. Pastes go through `Registers::read`, which can fail and returns an
owned value. Text ending in a line break reads as linewise, as in Vim. A
failed clipboard write never cancels the yank or delete: the text is still
stored, and the error is reported after the command.

Bracketed paste is on while the editor runs. `Event::Paste` goes to
`App::handle_paste`, bypassing key bindings and auto-indent. A paste that
brings its own indentation replaces the auto-indent on an otherwise blank
line.

So that `.` and macros can repeat a paste, it's recorded as keys, as
Neovim does internally: `<PasteStart>`, a key per character (`<CR>` for a
line break, `<Tab>` for a tab), `<PasteEnd>` (`input::paste_keys`). The
keys are recorded as a whole and the text put in at once (not fed through
`handle_key` one by one, which would be slow for a big paste). When keys
are replayed (a macro, `.`, `:normal`), `process_key` collects those
between the markers into `App::paste` instead of running them as bindings,
then puts the text in the same way. A collecting paste isn't idle, so `.`
records it as part of the change it's in. A `<PasteStart>` with no
`<PasteEnd>` (a macro edited by hand) is put in when the playback ends,
so it can't swallow later keys. A count on `.` doesn't multiply a paste.

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

`editor.status(name, fn)` registers a status line function for a
`plugin:<name>` element (`StatusElement::Plugin`, parsed from config
strings, so the names are checked: one no plugin registers is reported
after loading). The status line is drawn with only `&Editor`, so before
each draw `App::refresh_plugin_statuses` calls the functions in the
layout, with an ordinary `ctx`, and stores their text on the editor
(`Editor::set_plugin_status`); an error is shown once until it changes,
and the element is left empty. `ctx:option`/`ctx:set_option` go through
`Editor::option_value`/`Editor::set_option`, which use the same
TOML-table checks as `:set` (and `:setlocal` with `"local"`), converting
Lua values to TOML and back (tables are lists when keyed 1, 2, 3...).

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

Planned: plugin UI, the rest of LSP support (diagnostics are done), an
agent harness, an embedded terminal and Git support. They share three foundations, which should come first.

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

Started: see [Language servers](#language-servers) for the client and
diagnostics. Still to come, in order: a picker list of diagnostics, hover (a float), go to
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

## Language servers

`lsp/` is a client per server, configured per language in `[languages]`
(`language-server = { command, args, roots, language-id, settings,
enabled }`, as Helix does). None are installed for you; a missing one is
reported once.

- `transport.rs`: JSON-RPC messages with `Content-Length` headers.
- `convert.rs`: positions and `file://` URIs. Editor columns are chars;
  LSP columns are UTF-16 units, or bytes if the server takes `utf-8` from
  `general.positionEncodings` at start-up.
- `client.rs`: one server. `Client::start` spawns it (through a `Spawner`,
  which tests replace with a server in the same process over pipes), starts
  a thread that reads its output into the `Lsp`'s channel, and sends
  `initialize`. Its answer gives the encoding and sync kind; then come
  `initialized` and `workspace/didChangeConfiguration` with the settings
  (pyright waits for that before it checks anything). `change` sends a
  document's edits since its last version: the one edit from the buffer's
  edit log, in place, when there's exactly one and the server counts bytes
  and syncs incrementally; otherwise the whole text. `saved` sends
  `didSave` when the document's save count (`Document::saves`) has moved,
  after its changes; rust-analyzer runs `cargo check` then. Documents
  are found by the URI they were opened as (a new file's path is relative
  and can't be canonicalized), else by path; a document whose path
  changed (`:saveas`) is closed and reopened under the new one.
- `mod.rs`: `Lsp`, owned by `App`. `update`, called before each draw,
  drains the channel (answers, server requests, `publishDiagnostics`,
  `showMessage` errors, servers that closed) and then syncs: each buffer
  with a file and a language that has a server gets a client for its
  (language, root), started if need be, and is opened or changed there;
  documents no longer open are closed. Server requests are answered:
  `workspace/configuration` with each `section` of the settings (`{}`
  where there's none: the vscode servers ignore `null`),
  `workspace/workspaceFolders` with the root, anything else with `null`.
  The event loop polls every 30 ms while a server runs. On quit,
  `shutdown` then `exit`, waiting up to 300 ms before the processes are
  killed.

Diagnostics live on the `Editor` by document (`editor/diagnostics.rs`),
sorted, with positions converted to chars. `EditorView` draws them: an
underline in `diagnostic.<severity>` (with the underline colour), the line
number in `ui.diagnostic.<severity>`, and the first message after the
line's text. They're replaced by the server's next list rather than moved
with edits, so between an edit and the server's answer they can sit a
little off.

## Configuration

- `src/config/default.toml` is embedded with `include_str!` and is the single
  source of truth for defaults.
- The user file (`$XDG_CONFIG_HOME/ferra/config.toml`, otherwise
  `~/.config/ferra/config.toml`) is deep-merged over the defaults.
  Users only write what they change. Binding a key to `""` removes it.
- Unknown fields, bad colours and bad key notation are errors. A broken config
  doesn't stop startup: the editor falls back to the defaults and shows the
  error.
- Themes use dot-separated scopes that fall back to their parent
  (`ui.linenr.selected` → `ui.linenr`), which leaves room for syntax scopes.
- `:set` (`config/set.rs`) changes `[editor]` options while running.
  `EditorOptions` is serialized to a TOML table, the named value
  (dotted for nested tables) replaced, and the table deserialized back, so
  a value is checked exactly as in the config file and the message names
  the option. Text is taken without quotes and coerced by the current
  value's type; `+=`/`-=` work on numbers. Vim names map to ferra's
  (`ts` → `tab-width`), and Vim's `nu`, `rnu`, `ic` and `scs` switches
  pick values of `line-numbers` and `search.case`. Nearly every option is
  read where it's used, so it applies at once; `clipboard` and
  `clipboard-provider` are re-applied to the registers, and
  `kitty-keyboard` waits for the next start.
- Local options, as in Vim: the editor keeps the global options and, per
  window and per buffer, a sparse TOML table of `:setlocal` values
  (`config::set::scope` says which options may: `wrap`, `wrap-indent`,
  `wrap-marker`, `line-numbers`, `cursorline`, `scroll-off` per window; `tab-width`, `expand-tab`,
  `auto-indent`, `text-width`, `auto-wrap` per buffer). `Editor::options` is always what's in force
  for the active window and buffer (global, then the buffer's language's
  `options` from `[languages]`, then the buffer's, then the window's, by
  `set::with_locals`), recomputed by `refresh_options` when the active
  buffer (`activate`) or window (`focus_window`) changes, or when
  `finish_command` sees the buffer's language has (a `#!` typed, a scratch
  buffer saved as `Makefile`), so the rest of the code reads options as
  before. Drawing asks `Editor::options_for` per window. `:set` works on
  the options in force (so `:set ts?` shows them), copies the changes to
  the global options and drops the active window's and buffer's own
  values for them (`follow_global_options`), except that where the
  buffer's language sets one, the buffer gets the new value as its own,
  so `:set et` in a Go buffer takes effect there, as in Vim;
  `:setglobal` changes only the global ones; `:setlocal` stores values in
  the tables and refuses global-only options. A split copies its window's
  table. Language options are checked when the config loads
  (`Languages::check_options`): buffer options only, valid values.

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

- Language servers give diagnostics only, so far: no hover, go to
  definition, references, completion, formatting, rename or code actions,
  and no list of diagnostics in a picker. Servers' `$/progress` and
  `window/logMessage` are ignored (so `:lsp` says "running" while
  rust-analyzer is still indexing), as are dynamic registrations and
  `workspace/applyEdit`. A server that stops isn't restarted (there's no
  `:lsp-restart` yet), and changing its config needs a restart of ferra. Diagnostics don't move with edits until
  the server sends new ones.

- Without the kitty keyboard protocol (tmux, Terminal.app, older xterms),
  Alt keys that *are* bound can't be told apart from a fast Esc then that
  key (terminals send the same bytes), so binding `<A-j>` in insert mode
  means a quick `<Esc>j` runs it there. tmux's own `extended-keys` (CSI u)
  mode isn't asked for.
- Indentation: no tree rules for Python or YAML (typing them uses the
  heuristics above, and `=` declines), nor keywords for languages other
  than Bash and Python (Lua's `end`, Ruby's). Vim's mix of a `shiftwidth`
  narrower than tabs (4-column levels made of tabs and spaces) isn't
  supported. Window options (`wrap`) can't be set per language.
- Completion: words, file names and whole lines (no dictionaries,
  omni-completion or LSP, nor Vim's other `<C-x>` modes), matched by
  prefix (not fuzzily). `.` replays the `<C-n>` keys, so it completes again
  rather than repeating the word chosen, which differs if the buffer has.
- Formatting: no external `formatprg`, block comments' middle lines
  (` * ` in `/* */`) aren't leaders, and two spaces after a sentence
  become one. `auto-wrap` only breaks the line being typed; it doesn't
  rejoin a paragraph after an edit (Vim's `formatoptions=a`).
- Folding: only by indentation (no syntax-tree or marker folds); closing an
  outer fold forgets the folds inside it; no fold column; folds aren't
  saved with the file; relative line numbers count hidden lines.
- `gf` takes the name as written: no `path`/`suffixesadd` search, and
  `gx` (open a URL) isn't there.
- `"=` (the expression register).
- Soft wrap: a line taller than the whole window can't be scrolled through (the view
  starts at a line's start; Vim shows `@` rows, Neovim smooth-scrolls),
  and `<C-d>`/`<C-e>` scroll by lines, not screen rows.
- `<C-f>`/`<C-b>` move the cursor with the view, like `<C-d>`, rather than
  to the edge of the new screen as in Vim.
- Local options cover the ten above; others (`statusline-layout`,
  `search.*`...) are global, so the status line layout is one setting for
  the active window. Plugin status functions run every frame, so a slow
  one slows drawing.
- External changes are noticed by polling once a second, not OS file
  notifications; and a change that keeps both the modified time (within
  the filesystem's resolution) and the size is missed.
- Extended attributes and ACLs (e.g. macOS file tags) aren't carried over
  by the rename used for ordinary saves.
- Guessing legacy encodings (telling Shift-JIS from Windows-1252 by
  content): files that aren't UTF-8 use the one configured fallback.
- Reading the terminal's clipboard (an OSC 52 query), so `"+p` over SSH
  could paste what was copied on the local machine; for now that's the
  terminal's own paste.
- Syntax: files over 4M chars aren't parsed (the first parse is the
  cost), so they get no highlighting and heuristic indentation; and
  combined injections (JavaScript's `css\`...\``) only gather the matches
  in the rows being drawn.
- Loading grammars and queries at runtime, Helix-style, instead of
  compiling them in (each adds to the binary: 15 languages make it about
  17 MB in release, up from 8 MB with 5).
- `.` and macros replay keys without timing, so a short binding that ran
  because the key timeout passed (`g` alongside `gg`) replays as waiting
  for the longer one. Text typed by a timeout in insert mode replays fine.
