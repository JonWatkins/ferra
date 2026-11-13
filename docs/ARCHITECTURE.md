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

## Virtual text

A `Document` keeps `VirtualText`: text drawn before a char that isn't in
the file, with a theme scope and a source (`"inlay"`) so a source's pieces
are replaced together. It's moved along with edits as diagnostics are
(`virtual_text_after_insert`/`_delete`); typing where a piece sits pushes
it along, and deleting around it drops it.

`Document::display_line(row)` gives a line with its virtual text put in
(`None` for the many lines with none, which keep the old path), and maps
its columns: `to_display(col)` is where char `col` is drawn (after the
virtual text before it), `caret_at(col)` the gap before that virtual text
(where an insert-mode cursor goes, and where a range ending at `col`
ends), and `to_real` maps back. Everything that lays lines out uses the
display line: `TextLayout::rows_in` and `follow_cursor` (so scrolling
counts the extra rows), the editor pane (wrapping, the cursor, the
virtual text's colours over everything), `ViewState::position_at` for
clicks, and `gj`/`gk`. Everything that comes in buffer columns (syntax
spans, selections, search matches, diagnostics, references) is mapped as
it's drawn; search runs on the real text, so hints never match.

Virtual text marked `end_of_line` (git blame) is left out of the display
line and drawn after the line's own text on its last row, after any
diagnostic message, cut off at the window's edge: it takes no room, so
nothing else changes.

A `Document` also keeps `RangeHighlight`s: ranges drawn in a theme scope,
moved along with edits (text typed inside one stretches it; one whose text
is all deleted goes), drawn after syntax colours and before search
matches. Plugins make both through `ctx:decorate` (`plugins/api.rs`), with
the source `plugin:<group>`, and `ctx:clear_decorations(group)` takes a
group's away together (`Document::clear_decorations`).

Inlay hints (`LspFeature::InlayHint`) are asked for the whole buffer once
its text has been still for 300 ms and a ready server has it
(`Lsp::ask_for_inlay_hints`), again after each change and when the server
sends `workspace/inlayHint/refresh` (rust-analyzer does once it's
indexed). An answer for text that's changed since is dropped. Labels made
of parts are joined, and `paddingLeft`/`paddingRight` become spaces.

## The mouse

`ui::draw` returns a `ScreenMap` of where things went: each window's text
area and the lines between windows, the sidebar (its tabs, list rows and
edge), the terminal panel, and whichever box is open (the picker's and a
popup's list rows with the index of the first item shown, the find box's
fields and toggles from `FindView::hits`, an input box's field). The app
keeps the last one, and `app/mouse.rs` works out what an event is over from
it. Text positions come from the window's `ViewState`, which records each
screen row as it's drawn (`DrawnRow`: the buffer row, the chars on it, the
display column it starts at and the wrap indent), so
`ViewState::position_at` maps a cell back through wrapping, folds, tabs,
wide chars and sideways scrolling without laying anything out again.

A click goes first to a box with focus (`click_in_box`): the picker
chooses or closes, a list popup runs its `<CR>` command, the find box flips
toggles and moves between fields (a click outside it closes it and goes on
to the text), an input box places its cursor or cancels. Otherwise an edge
starts a resize drag (`Drag::Sidebar` sets `sidebar-width`, `Drag::Panel`
`terminal-height`, `Drag::Split` resizes the window before the line through
`resize_window`), and anything else is a click on a window, the sidebar or
the panel. A window click focuses it and sets the cursor; double and triple
clicks (same cell, within 400 ms) run `visual_mode` + `inner_word` or
`visual_line_mode`; a drag from it starts visual mode and moves the cursor,
kept inside the window.

As the pointer moves, `App::pointer_for` names its shape (CSS names:
`pointer`, `text`, `ew-resize`, `ns-resize`, `default`); the event loop
sends it with OSC 22 when it changes, and pointer moves don't redraw.
iTerm2 only knows X11 names (`hand2`, `xterm`, `sb_h_double_arrow`), so
when `TERM_PROGRAM` or `LC_TERMINAL` says it's iTerm2,
`terminal::x11_pointer` translates; it honours them only from 3.7.0 (before
that, it ignores them while mouse reporting is on). Mouse
reporting is turned on and off with the `mouse` option, each time round
the event loop.

## The sidebar

`editor/explorer.rs` holds the file tree: the root (the starting folder,
canonicalised), the set of open folders, each folder's listing (from
`files::list_dir`, cached until `refresh`) and the rows, rebuilt depth
first from the open folders whenever one opens or closes, keeping the
selected path selected. `reveal` opens the folders down to a file (how
`<Space>e` starts at the current file), and a save calls `saw_file`, which
lists the file's folder again if the file is new to it.

The tree has focus in `Mode::Sidebar`, with its own `[keys.sidebar]` table
(counts work, as in normal mode); the `sidebar_*` commands
(`commands/builtin/sidebar.rs`) drive it. Opening a file goes back to
normal mode and through `Editor::open`, named relative to the current
folder as the file picker names them. The `Explorer` is made the first time
it's needed and kept when the sidebar closes, so its open folders are still
open next time.

`ui/sidebar.rs` draws it. `ui::draw` takes the sidebar's columns (plus a
`│`) from the left before the windows are laid out, so splits, resizing and
per-window status lines work within what's left. With per-window status
lines the sidebar runs down to the command line; with the global one, the
status line keeps the full width under it. Popups (the picker) are still
centred on the whole screen. The view scrolls (a `Cell` in the explorer) to
keep the selected row in sight, and the terminal cursor sits on that row's
name while the sidebar has focus.

## The terminal panel

`terminal/` runs a shell in a pseudo-terminal (`portable-pty`). A thread
reads its output into a channel; `Terminal::pump`, called by the event
loop (which polls while a shell runs, as it does for language servers),
feeds it to a `vt100` parser, whose screen the panel draws. The parser's
callbacks answer the queries programs wait on (`ESC [ 6 n`, the device
attributes) and take the window title. Keys go back as the bytes a
terminal sends (`terminal::key_bytes`: control characters, xterm's
modifier parameters, application cursor mode); pastes are bracketed when
the program asked.

The editor owns the `Terminal` (`editor/terminal_panel.rs`) and whether the
panel's open; hiding it keeps the shell. `Mode::Terminal` has focus there.
In it the input layer returns `Outcome::PassThrough` with the keys of any
sequence that isn't bound in `[keys.terminal]` (including a bound prefix
that goes no further, or times out), so only those bindings are kept from
the shell. Without the kitty protocol crossterm reads Ctrl+\ ] ^ _ as
Ctrl+4 to 7; `KeyPress::from_event` maps them back.

`ui/terminal.rs` draws the title row and the screen cell by cell (indexed
colours stay the user's palette). `ui::draw` takes the panel's rows from
the bottom of the windows' side, under their status lines. The panel notes
the size it was drawn at, and the app resizes the pty and parser to it
after drawing. Its key hints come from the keymap like the find box's;
`Keymap::hint_keys` ranks keys that need the kitty protocol (`<C-`>`,
`<C-CR>`) after ones every terminal sends.

## Git

`git/` reads by running `git` (`git show :./file` for a file's staged text,
`symbolic-ref` for the branch), so it behaves exactly as git does with
nothing to link, and diffs in-process with `similar` (Myers, with a 50 ms
deadline) so the gutter keeps up with typing. `line_changes` turns the diff
into hunks of added, changed and removed lines in the buffer's rows (a
removal next to an insertion is one change, as in VS Code).

`editor/git.rs` keeps each open file's staged text and hunks, and the
branch. The staged text is read when a file opens or is saved (the app
passes events to `git_saw`), when the terminal regains focus, and when
leaving the terminal panel (git may have run in either); the hunks are
diffed again before drawing for buffers whose edit count changed. The
editor pane draws a hunk's mark in the gutter's padding after the line
number (`git_mark`), `]c`/`[c` jump between hunks, and the status line's
`git` element shows the branch and counts.

`git::status` reads `git status --porcelain=v1 -z -uall` into a
`RepoStatus` (each file's staged and unstaged letter, untracked as `U`,
conflicts as `!`); `stage`, `unstage` (`restore --staged`, or `rm --cached`
before the first commit), `discard` (`restore`, or `clean` for an untracked
file) and `commit` run git and return its own error message. The editor
reads the status with the staged text (on open, save, focus and leaving
the terminal panel) and when Source Control is shown, and after every
action. `git_tree_marks` gives the file tree each changed file's letter by
path, and `•` for the folders above it.

Source Control is a built-in sidebar view between the files and plugins'
views (`SidebarShows::Git`, the `Git` tab). Its rows (`GitRow`: headers and
files, staged first) come from the status; its keys (`GIT_VIEW_KEYS`) are
bound in `Mode::Sidebar` the way a plugin view's are, and the sidebar's
moving, `R` and clicks act on it. Discarding asks in a built-in popup
(whose `y` runs `git_discard_confirmed`), and committing in an input box
(whose submit runs `git_commit_message`). A popup opened from the sidebar
goes back to it when it closes.

Line blame (`Editor::git_blame_tick`, each time round the event loop)
waits for the cursor to rest 400 ms on a line in normal mode, then runs
`git blame --porcelain -L n,n --contents -` on a thread with the buffer's
text on stdin, so unsaved edits count (their lines come back as git's
all-zeros "not committed" commit). The answer is shown only if the cursor's
still on that line and the text hasn't changed, as end-of-line virtual
text from the source `"blame"`, and taken away when either changes or
another mode starts. While it waits, the event loop keeps polling.

The git view (`editor/git_gui.rs`, `ui/git_gui.rs`) is `Mode::Git` with
`[keys.git]`: a `GitGui` with the focused panel, the branches
(`git::branches`), the last 300 commits (`git::log`) and the diff panel's
`Diff`. The files panel uses Source Control's rows and selection, so the
two agree. Whenever the selection or focus changes, the diff is made for
what the focused list has selected: `git::file_diff` (`diff --cached` for
a staged row, `diff --no-index /dev/null` for an untracked file),
`show_commit`, or a branch's log. `Diff::parse` splits it into lines by
kind and finds the hunks, each with its file header, so `hunk_patch` can
make a patch of one hunk; staging it is `git apply --cached` with the patch
on stdin, unstaging the same with `--reverse` (as lazygit does). Picking
lines (`GitGui::line`, and `anchor` for a range) makes a patch of part of
a hunk (`Diff::lines_patch`): chosen lines kept, and of the rest, applied
forward, removed lines turned into context and added ones dropped (in
reverse the other way round, as the new side is what's in the index),
with the header's counts worked out again. After the diff's read again,
the selected line goes to the nearest changed one left. After any
action the status, branches, commits and diff are read again, and the
selection stays on the same file in the same section when it's still
there. Commit and discard ask in the input box and popup, which go back to
the git view (they remember the mode they came from). `ui::draw` draws the
view over everything but the command line and maps its panels and lists
for the mouse (`GitGuiMap`).

Branches are `git switch` (`-c` for a new one) and `git branch -d`/`-D`;
`ahead_behind` is `rev-list --left-right --count HEAD...@{upstream}`,
read with the status. Fetch, pull and push (`git::start_remote`) run on a
thread that reads git's `--progress` lines (redrawn with `\r`) from stderr
and sends them, then the result, through a channel; the editor keeps the
receiver (`GitState::job`, one at a time) and the event loop pumps it
while it runs, as it does the terminal panel's shell. They run with
`GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=true` and (unless you've set your own
ssh command) `ssh -o BatchMode=yes`, so nothing can stop to ask on the
terminal ferra is drawing on. A branch's first push adds
`--set-upstream <first remote> <branch>`.

Amending is `commit --amend -m` with the last message (`log -1
--format=%B`) put in the input box to edit; the title says if the branch
isn't ahead of its upstream (the commit's been pushed). Stashing is `stash
push --include-untracked`, the list `stash list --format=%gd%x00%gs`, and
putting one back `stash pop <name>`. Like switching branch, both refuse
while buffers have unsaved changes (`git_check_saved`) and reload open
files after. With the git view open, a picker, box or `:` line asked from
it comes back to it when it closes (`Editor::set_mode`).

A file's history (`GitGui::history`) swaps the commits panel's log for
`log --follow --name-only -- <path>` (`git::file_log`), keeping the
file's name in each commit. A commit's diff is then `show -- <name>`
(`git::show_commit_file`), with the name in the commit before it added
when they differ, so a rename shows as one rather than as a new file.
`<Esc>` (`git_gui_back`) leaves the history before it closes the view.

A commit's diff and a branch's log are read on a thread (`spawn_diff`,
a `DiffJob` in `GitGui::diff_job`), since `git show` takes tens of
milliseconds and a held `j` repeats faster than that: the selection
moves at once and the diff follows, the event loop taking it with
`pump_git_gui_diff`. Only the latest request is kept; an earlier one's
answer is dropped with its receiver. Commits' diffs are cached (up to
100, by hash and paths), so moving back is instant. A file's diff stays
synchronous, as staging a hunk needs it exactly as it is.

A file's blame (`GitGui::blame`, a `BlameView`) is drawn over the panels
while it's there, and the view's movement keys act on it first. It's
`blame --porcelain --contents -` (`git::blame_file`) given the buffer
encoded as it would be saved (`TextFormat::encode`), so unchanged lines
match what's committed; from the files panel it's the file on disk.
A change side by side (`GitGui::side`, a `SideView`) is the same kind of
page: `git::side_by_side` diffs the two texts with `similar` and lines
them up into `SideRow`s (unchanged lines level, a changed run's removed
and added lines paired, filler on the shorter side), the old text from
`git show :path` or `HEAD:path` (`git::blob`), the new from the file or
the index. Both sides are scratch documents (`SIDE_DOCUMENTS`)
highlighted like the blame's. Opened from the sidebar (`<CR>` and clicks
in the Git tab), `<Esc>` closes the view back to the sidebar
(`return_to`).

Each line's heat is its commit's rank by time among the file's
commits, in quarters; the code is highlighted from a scratch `Document`
(`BLAME_DOCUMENT`), as the hover's is. `<CR>` opens the line's commit
(selected in the log, or read on its own
if it's older than the log goes). Opened from a buffer, `<Esc>` closes
the whole view.

Merge conflicts: `git::conflicts` finds complete conflicts in a file's
lines (`<<<<<<<`, an optional diff3 `|||||||`, `=======`, `>>>>>>>`, each
seven characters then the end of the line or a space; a stray marker
isn't one). `update_git_changes` finds them along with the hunks, so the
editor pane can tint each side's rows (`EditorView::conflicts`,
`ui.conflict.*`) and the markers get end-of-line virtual text (source
`"conflicts"`) naming the keys. `editor/conflicts.rs` reads them afresh
from the buffer for `]x`/`[x` and for keeping a side: the conflict's rows
are deleted and the kept lines inserted in their place, in one command,
so one undo step. `git::merge` is `merge --no-edit`; when it fails with
`MERGE_HEAD` there, it stopped on conflicts. `git::merging` reads
`MERGE_MSG` (without git's comment lines) while there's a `MERGE_HEAD`;
the editor reads it with the status. Conflicted files (`FileStatus::
conflicted`, `!`) get their own section first in `git_rows`; staging one
(`git_check_resolved`) is refused while its buffer's unsaved or the
file still has markers, and the commit box is allowed with nothing
staged during a merge and starts with its message. `M` asks before
merging, or before `merge --abort` while a merge is going on.

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
per frame, not one per key. Debug builds compile the dependencies
optimized (`[profile.dev.package."*"]` in `Cargo.toml`): tree-sitter,
ratatui and serde_json do most of a frame's work, and unoptimized they
made typing lag (27 ms of CPU per key with TypeScript's completions,
against 4 ms).

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

### The find box

`editor/find.rs` holds the find box's state (`FindState`): two
`TextField`s (`editor/text_field.rs`, a one-line field whose text can
start selected, for reuse by later boxes), which has focus, the toggles,
and where finding as you type starts. Every change to the find field or a
toggle compiles a pattern with `SearchPattern::from_regex` (the text
escaped unless regex is on, wrapped in `\b(?:...)\b` for whole words, no
Vim translation) and moves the cursor to the first match at or after that
start; `<CR>` moves the start along. `find_match` walks lines for the next
non-empty match either way, wrapping, so the box never lands on an empty
regex match.

The box has focus in `Mode::Find` (its own `[keys.find]`, text typed into
the field). `search_highlight` returns its pattern while it's open, and
`find_current_match` the match at the cursor, which the editor pane draws
with `ui.search.current`. The count is made the first time it's drawn after
the pattern or the text changes (keyed by the document's id and edit
count, so drawing doesn't search again), up to 10,000. Replacing goes
through `SearchPattern::next_replacement` / `replace` (a literal
replacement has `$` doubled); each command is an undo step as usual, so
replace all is one. Closing hands the pattern to `set_last_search` and
clears the highlight (`clear_search_highlight`), as VS Code does; `n`/`N`
turn it back on. The app treats keys in find mode like the `:` line:
replacements aren't repeated by `.`.

The keys the box shows come from the keymap (`Keymap::hint_keys`: the
binding without Alt if there is one, since macOS terminals don't send
Option as Alt by default, then the shortest, then the fewest modifiers),
labelled by `ui::find::key_label` (`⌃A` on macOS, `C-a` elsewhere). The app
passes them to `ui::draw` in `KeyHints`, with the splash screen's.

`ui/find.rs` draws it at the top right of the active window
(`windows::draw_windows` now also returns that window's area), the field
scrolled to keep its cursor in view.

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
is drawn at the cursor in the `ui.popup` colours with a border like the
hover's, its text lined up with the word, below its row or above when
there's no room, showing up to 10 matches (each label with the letters
typed in `ui.popup.match`, and its detail dimmed beside it) and scrolling
to the selected one, with `n/total` in the border when they don't all fit.

#### Completion from language servers

`editor/server_completion.rs` uses the same `WordCompletion` (its
`labels`, `details` and `server` part are for this), so the menu, `<C-n>`,
`<C-p>`, `<C-e>` and the `.` bookkeeping are shared. What differs:

- It's asked for, not gathered. `end_completion_unless_used` notes whether
  the key changed the text (`text_before_key`), and if so `auto_complete`
  queues an `LspRequest` (feature `Completion`) when two letters of a word
  are typed, or with the character before the cursor as a possible
  trigger: `Lsp` drops that unless it's one of the server's
  `triggerCharacters`. Requests made as you type are quiet: no message
  when there's no server or it can't complete. `<C-Space>` asks anyway.
- Nothing goes in until chosen (`selected` starts at `None`). The answer
  is kept whole (`ServerCompletion::all`) and narrowed locally on each key
  that keeps the cursor in the same word (`refilter_server_completion`),
  instead of ending: items starting with what's typed first, then
  starting with it in another case, then fuzzy matches whose first letter
  starts a word in the item (`fm` for `flat_map`, not `0` for `log10`),
  each tier in the server's `sortText` order. A list marked `isIncomplete` is asked for
  again as typing goes on (rust-analyzer marks all of them so): an
  answer that arrives while a newer completion request for the buffer
  is waiting isn't read (`handle_response`), and reading one reads the
  word's line once rather than per item, as narrowing takes the text
  typed from the buffer once rather than per item (there can be a
  thousand: TypeScript's). An answer is used if the editor is still
  completing the same word (`still_completing`: same buffer and row, same
  word start, insert mode), however much more of it was typed, but never
  replaces an item already chosen with `<C-n>`.
- Items become `ServerItem`s in `lsp/features.rs`: the text to put in (the
  `textEdit`'s new text, or `insertText`, or the label) and where it goes
  in (`start`: the edit's start when that's before the word on its line,
  as the TypeScript server's `?.` and clangd's `->` replace the `.` typed,
  else the word's start), the filter and sort text, a detail
  (`labelDetails`, `detail`, or the kind's name) and the other edits
  (`additionalTextEdits`). The completion's `start` is the earliest item
  start, and each item's text is laid out from there (`.` + `area`), so
  they all replace the same text and `<C-n>` steps through them as for
  words; each is matched against what's typed from its own start or the
  word's, whichever matches better (TypeScript's filter text has the `.`,
  clangd's doesn't, which it only sends with `editsNearCursor`).
- Snippets (ferra says it supports them: the CSS and HTML servers
  complete nothing otherwise) go in as text (`expand_snippet`): later
  placeholders' text kept, the first stop's left empty, and taking the
  item puts the cursor there (`call(|)`), or at `$0`. There's no moving
  on to later stops.
- `<C-y>` and `<Tab>` (`insert_tab` checks for the menu) take the chosen
  item or the first, and make its other edits (last first, the cursor
  moved with text edited before it). Items whose import comes only on
  asking (the server has `resolveProvider`, or the item carries `data`, as
  rust-analyzer's do though it claims not to resolve) keep a
  `(answer, index)`; `Lsp` keeps the last four answers' raw items, and
  taking one sends `completionItem/resolve` with it, making the
  `additionalTextEdits` that come back.

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

### Plugin packages

Installed plugins are git clones with a `ferra-plugin.toml` manifest
(`plugins/packages.rs`, which does only file and git work, so it can run
on a thread). `PluginPaths` names the config folder (`plugins.toml`, what
was asked for; `plugins.lock`, the commit of each) and the data folder
holding one clone per plugin name. `main` gives the standard paths to the
app; tests give temporary folders.

`:plugin_install` (`editor/plugin_packages.rs`) parses the source
(`owner/repo` is GitHub; URLs and paths pass through to git, with `@tag`
split off where it can't be part of the URL) and asks in a popup. `y`
leaves an `InstallRequest` on the editor, the way the file picker asks for
a listing, since commands can't reach the app's `PluginHost`; the app
starts it as a background job (`Update::PluginInstalled`). The job clones
into a hidden folder beside the others (git with prompts off and ssh in
batch mode, as for push), checks the manifest (name, `main` inside the
plugin, the `ferra` version range), refuses a name already taken, and only
then renames it into place, so a failed install leaves nothing. The app
then records it and loads it: `PluginHost::load_package` puts the
plugin's `lua/` folder first on `package.path`, so `require` finds its
modules, and runs the main file under the plugin's name. At startup
`App::load_plugins` loads every package in `plugins.toml` before
`init.lua` and `plugins/*.lua`; one listed but not cloned is reported.

`:plugin_remove` deletes the clone and the two entries; what the plugin
registered stays until restart, since unregistering is the next round's
`:plugin_reload` (a fresh Lua state, re-registering everything).

### Plugin popups

`ctx:popup{...}` (`plugins/popup.rs`) reads a Lua table into an
`editor::popup::Popup`: styled lines (spans with a theme scope), title,
footer, position, size, whether it's a list, whether it takes focus, its
own keys and an `on_close` command. Everything is checked as it's read
(key notation, that each command exists), so a mistake is the plugin's
error rather than a popup that half works. The popup's keys name commands
rather than holding Lua functions, so they go through the registry like any
key: undo, hooks, errors and `ctx` work with no special cases, and the
editor holds no Lua state.

A popup with focus is `Mode::Popup`. When one opens, the app makes a copy of
the keymap with the popup's keys bound in that mode (`App::popup_keymap`)
and uses it while the mode lasts; `[keys.popup]` has the moving and
closing commands (`commands/builtin/popup.rs`). `popup_close` runs the
popup's `on_close` through `cx.registry`; `ctx:popup_close()` from the
plugin doesn't, so a plugin closing its own popup can't loop. A popup
without focus is dropped after the next key that didn't open it, like the
hover. `ui/popup.rs` sizes it to its lines (within the windows' area),
places it centred, by the cursor or at the top right of the active window,
scrolls it to keep the selection in view, and shows where it's at in the
bottom border.

### Asking the user from plugins

`ctx:pick` and `ctx:input` (`plugins/prompts.rs`) follow popups: the table
is checked as it's read, and the answer goes to a command by name. A pick
is an ordinary `Picker` whose items carry `PickerAction::RunWith(command,
[value])`, so filtering, drawing and keys are the built-in picker's. An
input is an `InputBox` (`editor/input_box.rs`: a title, a `TextField` from
the find box, a placeholder, and the commands to run), with focus in
`Mode::Input` and its own `[keys.input]`; leaving that mode any way closes
it. `input_submit` takes the box and runs `on_submit` with the text as its
argument. `ui/input_box.rs` draws it at the top centre of the windows, its
key hints from the keymap like the find box's.

### Plugin sidebar views

`editor.view(name, spec)` records a `ViewSpec` (title, keys, the events that
refresh it) and keeps the `lines` function in the plugin host
(`PluginHost::view_lines`). After installing, the app hands new specs to
the editor (`editor/sidebar_views.rs`), which keeps each view's lines, its
selection and scroll, and a *stale* flag. The editor can't call Lua, so it
only marks views stale: when shown, on `R`, after the events in `refresh`
(the app passes every event to `plugin_views_saw` as it fires hooks), on
`ctx:view_refresh`, and when the current buffer isn't the one its lines
were made for. Before drawing, `App::make_plugin_view` calls the function
of the view shown if it's stale, so a view nobody can see costs nothing.

The sidebar's commands act on whichever view is shown (`sidebar_move` moves
a plugin view's selection, `R` remakes it, the tree's own commands do
nothing there). A view's keys are bound like a popup's: when the view
shown changes, the app rebuilds its keymap copy with them in
`Mode::Sidebar`. `ui/sidebar.rs` draws the tabs (`SidebarTabs`) when
there's more than the file tree, and the view (`PluginViewWidget`).

## Plugin roadmap

Done: named commands, runtime keybinding, event hooks and Lua plugins. Next:

1. **A wider API**, driven by real plugins: selections, registers, search,
   buffers, options, and creating pickers from Lua.
2. **Plugin UI**: started with popups (see [Plugin popups](#plugin-popups));
   views in the sidebar (see [Plugin sidebar views](#plugin-sidebar-views)),
   asking the user (pickers and input boxes, see [Asking the user from
   plugins](#asking-the-user-from-plugins)), and decorations (see [Virtual
   text](#virtual-text)); still to come are windows and scratch buffers
   (see "Plugin windows and decorations" below).
3. **Registering at runtime.** Registrations made inside a callback (rather
   than while loading) are collected but not yet installed.
4. **Isolation.** If sandboxing untrusted plugins becomes a goal, a WASM
   runtime could sit behind the same command and hook interfaces.
5. **Packages** (see [Plugin packages](#plugin-packages)). Phase 1 round 1
   is done: `ferra-plugin.toml` packages, `:plugin_install`,
   `:plugin_list`, `:plugin_remove`, `plugins.toml` and `plugins.lock`.
   Next: `:plugin_update` (showing the incoming commits first) and
   `:plugin_reload` (a fresh Lua state, dropping what plugins registered);
   reinstalling what `plugins.lock` lists on a new machine. Phase 2:
   `:plugin_browse`, picking from the `ferra-plugins` directory's
   `index.toml`. Phase 3: language packs, whose grammar's C source is
   compiled on install.

## Roadmap: deferred features

Done: the planned LSP support, the VS Code-like interface (sidebar, find
box, terminal panel, mouse), plugin UI (popups, sidebar views, prompts,
decorations) and the everyday Git features. Still to come: plugin windows
and scratch buffers, the agent harness, more of the embedded terminal, and
the rest of Git. They share three foundations, started below.

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
   the editor pane after syntax highlighting. Started: inline virtual text
   and at the end of a line, and highlighted ranges (see [Virtual
   text](#virtual-text)), used by inlay hints, line blame and plugins'
   decorations; git's gutter marks are drawn by the pane. Still to come:
   gutter signs and status line segments from plugins.

### Plugin windows and decorations

Built on the foundations, exposed to Lua in the style of Neovim's
`nvim_open_win` and extmarks:

- `editor.buffer.create()` / `set_lines`: a scratch buffer a plugin fills.
- `editor.window.open(buffer, { float = { row, col, width, height, border,
  title } })`, or as a split; then `focus`, `close`, `set_config`.
- Buffer-local `editor.map` and `on("buffer-closed")`, so a plugin's modal
  handles its own keys.
- Decorations: done for the current buffer as `ctx:decorate` (virtual
  text and highlights, cleared by group); still to come are gutter signs,
  and decorating buffers other than the current one.
- Status line and gutter components that plugins add to the built-in ones.

### LSP support

Started: see [Language servers](#language-servers) for the client,
diagnostics, hover and going to definitions and references, and [Word
completion](#word-completion) for completion. Inlay hints are done too
(see [Virtual text](#virtual-text)); that's every feature planned.

### Agent harness

An AI agent working inside the editor: a chat buffer in a split or float,
streaming replies through the event channel. Its tools are the command
registry (open, search, read, edit), so every change it makes is an
ordinary edit: undoable, repeatable and visible. Proposed edits can be
shown as a diff first, using decorations, and accepted or rejected per hunk.
Built as a plugin-level feature on the same APIs, so a different agent
could be swapped in.

### Embedded terminal

Started: the terminal panel (see [The terminal panel](#the-terminal-panel)).
Still to come: Neovim's terminal-normal mode, where the scrollback is an
ordinary read-only buffer to search and yank from, more than one shell,
and a terminal in a window rather than the panel.

### Interface

A VS Code-like layout, so the editor can be driven without many commands:
the sidebar's file tree and the find box (done; see [The
sidebar](#the-sidebar), [The find box](#the-find-box), [The terminal
panel](#the-terminal-panel), plugin popups and views, and [The
mouse](#the-mouse)). The sidebar holds more than files: Git's Source
Control is a view in it, and plugins add their own.

### Git support

Started: changes in the gutter, `]c`/`[c`, the branch, the file tree's
marks, Source Control in the sidebar, and the full-screen git view with
diffs and staging hunks, branches, and fetch, pull and push in the
background, amending and stashes, a file's history and its blame, and a
read-only side-by-side diff, and merging with conflicts resolved in the
editor, and staging single lines (see [Git](#git)). Deferred: making
the side-by-side diff editable.

The everyday Git features, without leaving the editor:

- **Change signs in the gutter**: added, changed and removed lines compared
  with the index (or `HEAD`), updated as you type, plus a status line
  segment with the branch and counts.
- **Hunks**: `]c`/`[c` to move between changes, preview a hunk's diff in a
  float, and stage, unstage or reset (undo) a hunk, or the whole file.
- **Diff views**: the buffer side by side with the index, `HEAD` or any
  revision, scrolling together with matching lines aligned; also a diff of
  the whole working tree to browse file by file. Done read-only (the git
  view's side-by-side page). **Deferred: make it editable**, as VS Code's
  is: the right side the real buffer in the editor area (a window pair
  scrolling together, with filler rows in the window), the diff kept
  up to date as you type, and staging or reverting a change from it.
- **Blame**: the author and summary of the current line as virtual text
  (optional), and a blame view of the whole file that opens a commit's
  diff (both done).
- **Status and commits**: a picker of changed files (stage or unstage from
  it), writing the commit message in a buffer and committing on save,
  amending, and branches (list, switch, create) in a picker.
- **History**: the log of a file (done) or a line range, opening any
  version.
- **Conflicts**: jumping between conflict markers and keeping ours, theirs
  or both, as one undo step each (done, with merging and aborting a
  merge from the git view).

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
  The event loop checks every 30 ms while a server runs, but only redraws
  when `update` says something happened (a message handled, a request
  sent, a server's state changed), or after input. On quit,
  `shutdown` then `exit`, waiting up to 300 ms before the processes are
  killed.

Diagnostics live on their `Document` (set through `editor/diagnostics.rs`),
sorted, with positions converted to chars. `EditorView` draws them: an
underline in `diagnostic.<severity>` (with the underline colour), the line
number in `ui.diagnostic.<severity>` and the message after the line's
text, both on the line it starts on (one over a whole file, like
rust-analyzer's "not included in the module tree", would otherwise repeat
on every line); empty lines inside a range aren't underlined.

Between an edit and the server's next list, diagnostics move with the
text, as marks do: each `Document` keeps its own (`Editor::diagnostics`
and `set_diagnostics` reach them by id), and its `insert` and `delete`
shift those after the change (a start moves with text put in right at it,
an end doesn't, so typing after a word doesn't stretch its underline;
text deleted from under one shrinks it). Undo and redo report the changes
they make (`History::undo_noting`), so diagnostics follow them too. A list
the server says is for an older version than ferra last sent
(`publishDiagnostics`' `version`) is left out: it would put back places
the edits moved diagnostics from, and a newer one follows.

Requests from the editor (`K`, `gd`, `gr`...) keep the layers apart: the
commands only queue an `LspRequest` (`editor/lsp.rs`: the feature, the
document, the cursor, and the buffer's id and edit count). `Lsp::update`
takes them after syncing, so the server has the text they're about, and
`lsp/features.rs` sends each to the server with that document open, if it
said it can answer (`hoverProvider`, `definitionProvider`...), or says why
not. The answer is dropped unless the editor is still where it was asked
(same buffer, text and cursor, in normal mode), and stale-answer errors
(`ContentModified`, cancelled) are ignored.

- Hover: the contents (Markdown, a marked string, or a list of them) become
  one Markdown text in a document of its own (`HOVER_DOCUMENT`, never a
  buffer). `ui/hover.rs` highlights it as Markdown with `highlight_as`, so
  code blocks get their language's highlighting through the usual
  injections, and draws it in a popup by the cursor: fences hidden, as are
  Markdown's emphasis and code-span markers, headings' `#`s and links'
  URLs (from the inline grammar's spans), blank runs squeezed, `---` drawn
  as a rule, long lines wrapped between words. Inside code blocks
  Markdown's own `text.*` styles are dropped so they don't show through.
  The popup closes on the next key unless that's `K` again, which scrolls
  a page (the draw records how many rows there are and fit).
- Go to: `Location`, `Location[]` and `LocationLink[]` answers become
  `Location`s with char columns, converted with each line's text (from the
  buffer if the file is open, else read from disk once per file). One is
  gone to as a jump (`goto_location`, which opens the file by its path
  relative to the current folder when it's under it); several open a
  picker whose items are `PickerAction::GoTo`.
- Formatting waits for its answer, unlike the rest: `:wq` has to save the
  formatted text, so the edits must be made before the save goes on. The
  editor can't see the `Lsp` (it's in the app), so it holds a `Formatter`
  (`editor/lsp.rs`) that the app fills with an `LspFormatter`, sharing the
  `Lsp` through an `Rc<RefCell>` (only ever borrowed by a command or the
  draw, never both). `Lsp::format` syncs the buffer, sends
  `textDocument/formatting` with `tab-width` and `expand-tab`, and handles
  everything servers send until the answer comes or 2 seconds pass. The
  edits go through `Editor::apply_text_edits`: last first, the cursor
  moved along by those before it and kept at its place in one that
  replaces its text; in a command they're one undo step.
  `format-on-save` is a buffer option (so it can be set per language); a
  built-in `BeforeSave` hook runs `format(quiet)` before the config's
  hooks, and a failure doesn't stop the save, only adds to its message
  (`note_for_save`).
- Rename: `:rename name` queues an `LspRequest` carrying the name (`text`);
  `<F2>` opens the command line with `rename <word>` in it. The answer is
  a `WorkspaceEdit` (`changes` by URI, or `documentChanges`; ferra says it
  takes the latter but no file operations, so creating, renaming and
  deleting files are left out and counted). `apply_workspace_edit` makes
  each file's edits: in its buffer if it's open, else in one opened in
  the background (`buffer_for_path`: no jump, not shown, left modified),
  through `edit_document`, which makes them one undo step of their own in
  any buffer (`Document::apply_edits`, the same as formatting's). The
  message counts changes and files, and the other files left unsaved.
- Code actions: `<Space>a` queues a `CodeAction` request for the cursor,
  or a visual selection's range (in `word_start`..`end`; `position` stays
  the cursor, so `still_at` works). `Lsp` keeps each document's
  diagnostics as the server sent them (`raw_diagnostics`) and sends back
  those on the range's lines, which most quick fixes need. The answer's
  actions (less those `disabled`) are kept in a `Stash`, like completion
  items, and listed in a picker whose items are
  `PickerAction::CodeAction(answer, index)`. Choosing one queues an
  `ApplyCodeAction`, which `make_code_action` carries out: a bare
  `Command` is sent to `workspace/executeCommand`; a `CodeAction` has its
  `edit` made (`apply_workspace_edit`) and then its `command` run; one
  with neither is resolved first (`codeAction/resolve`, a
  `ResolveCodeAction` request, made only if the text hasn't changed).
  Commands often make their changes by sending `workspace/applyEdit`
  back, which `Lsp` answers by making the edit (with the action's title
  in the message).
- Highlighting references: `Lsp::update` notes where the cursor is
  (buffer, place, edit count); once it's rested there 150 ms in normal
  mode it queues a quiet `DocumentHighlight` request (once per place).
  The answer is kept with the edit count it's for (`set_references`), and
  `references_shown` gives the view the ranges only while the text is
  unchanged, in normal mode, with the cursor on one of them, so moving off
  or editing hides them at once. Drawn under search matches and the
  selection, in `ui.highlight.reference`.
- Progress and restarting: ferra says it takes `workDoneProgress`, so
  servers send `$/progress` (begin, report, end, by token; their
  `window/workDoneProgress/create` request is answered like any other).
  Each `Client` keeps the work begun and not ended; the first server busy
  with something gives the status line's `lsp-progress` element its text
  (`rust-analyzer: Indexing 45%`, else the title and message) and `:lsp`
  its "running (…)". `:lsp-restart [name]` (the command line takes a name
  with `-`s when it's a command's, so `:s-a-b-` is still `:s`) leaves a
  request on the editor that `Lsp::update` takes before syncing: the
  servers it names (the buffer's, by command or language, or `all`) are
  sent `shutdown` and dropped (which ends their processes), their
  diagnostics cleared, and which commands weren't found or failed is
  forgotten; the sync after starts what the open files need.
- Diagnostics pickers: `<Space>d` lists the buffer's diagnostics,
  `<Space>D` every buffer's and those for files that aren't open: a
  `publishDiagnostics` for a file with no buffer is placed by the file's
  text read from disk and kept on the editor by path
  (`set_file_diagnostics`), and opening the file gives its buffer those
  until the server reports again. Errors come first, then by file and
  place; the label is `path:line:col  message` (the path relative to the
  current folder if it's in it), which a narrow picker shortens from the
  start. Built by the editor alone: no request.
- Symbols: `<Space>s` asks for `textDocument/documentSymbol`, whose
  answer is nested `DocumentSymbol`s (flattened, each child's detail
  naming its parent: "method in Circle") or flat `SymbolInformation`s
  (their `containerName`); `<Space>S` opens an empty picker marked
  `searched_by_server` and asks `workspace/symbol` for its query, and
  `search_symbols_after_key` asks again whenever the query changes, the
  answer replacing the picker's items if the query's still the same
  (older answers with a newer request waiting aren't read). Places are
  found as for go to definition (`resolve_places`: the line from the
  buffer, or the file read once), and the items are
  `PickerAction::GoTo`.
- Signature help (`editor/signature.rs`, drawn by `ui/signature.rs`): after
  each key that changes the text in insert mode, a `SignatureHelp` request
  is queued with the character before the cursor; `Lsp` sends it if that's
  one of the server's trigger or retrigger characters (`(`, `,`), or on any
  change while a signature is showing (`isRetrigger`), so the active
  parameter follows the cursor. An answer with a newer request waiting
  isn't read, as for completion. The active signature's label is shown
  with its active parameter (by substring, or UTF-16 offsets) in
  `ui.popup.match`, and the first paragraph of its documentation (code
  blocks left out) wrapped to at most four rows. No signature in the
  answer closes it; so does leaving insert mode. It goes above the line,
  so the completion menu can be below, or below when there's no room.

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

- Virtual text: inline and at the end of a line, but not lines of their
  own; line blame is for the cursor's line only (no whole-file blame
  view); inlay hints are asked for the whole buffer (fine
  for most files, slow for huge ones), can't be clicked or hovered for
  more, and aren't resolved (`inlayHint/resolve`); visual block selections
  count columns without them.

- Git: the gutter compares with what's staged only (not `HEAD`); no
  rebasing, or continuing a rebase
  or cherry-pick stopped on conflicts (their markers are resolved the
  same way, but the merge state shown is a merge's only); one fetch, pull or push at a time,
  with no way to cancel one, and none that needs a password typed; the git view's diffs aren't syntax
  coloured, and huge commits are read whole; the file
  tree's marks are for the repository the current file is in; no marks with `line-numbers =
  "none"` (there's no gutter); a file's staged text is read again only on
  open, save, focus and leaving the terminal panel; submodules and files
  git can't show as text are skipped.

- Mouse: no right-click menu; dragging past a window's edge doesn't
  scroll it; no mouse in the terminal panel's programs (the wheel scrolls
  its history); the pointer shape needs a terminal with OSC 22 (iTerm2 from
3.7).
- Plugin popups: one at a time; an input box is a single line (no
  multi-line input, no validating as you type, no history); keys run commands, not Lua functions; no mouse; a plugin
  can't add a view to the bottom panel yet. Sidebar views are flat lists
  (no tree, folding or icons), made whole each time (no partial updates).

- Terminal panel: one shell (no tabs or splits); no mouse (scrolling,
  selecting) and no copying from it with the editor's keys (Neovim's
  terminal-normal mode); its scrollback can't be searched; the panel can't
  be resized by dragging (only `terminal-height`); programs' cursor shapes
  and the bell are ignored; on Windows it's untested.

- Find box: one file only (no search across the project, VS Code's
  `Ctrl+Shift+F`, yet); no history of what was found; no "in selection"
  toggle or preserve-case replace; matches don't span lines (no `\n` in
  the pattern or the replacement); it can cover the matches in the
  window's top lines. The `<Space>/` and `<Space>h` keys leave Vim's
  `<C-f>` alone; bind `"<C-f>" = "find_open"` for VS Code's.

- Sidebar: no creating, renaming, moving or deleting files from it; no
  mouse; it shows the current file only when `<Space>e` reveals it (it
  doesn't follow buffer switches); files deleted or made by other programs
  show up on `R`; no colours for Git status or diagnostics; no filtering
  by typing (`<Space>f` does that); the root is always the starting
  folder.

- Language servers: no sign that code actions are available; the project
  symbol picker shows what the server found, which ferra then filters by
  its own fuzzy match (a server that matches more loosely may have some
  left out); signature help shows one signature
  (no keys to step through overloads); code actions that create, rename
  or delete files leave that part out; rename doesn't check the name first (`prepareRename`), and doesn't
  rename files, no range formatting (`=` and `gq` stay ferra's own), and no list of diagnostics in a
  picker. Hover is Markdown made plain (no tables, and lists keep their
  `-`); it can't be focused, searched or copied from, and it isn't shown
  in insert mode. `window/logMessage` and dynamic registrations are
  ignored. A server that stops isn't restarted by itself (`:lsp-restart`
  does it), and changing its config takes a `:lsp-restart`.
  Diagnostics moved with edits can be a little off until the server's
  next list where an edit replaced their text; marks still don't follow
  undo.

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
- Completion: words, file names, whole lines and the language server's
  (no dictionaries, nor Vim's other `<C-x>` modes); words matched by
  prefix (not fuzzily). Server completions: snippets have no tab stops to
  move between (only the first is where the cursor goes), documentation isn't shown beside the menu, and items are filtered by
  ferra rather than asked for again on each key (unless the server says
  its list is incomplete). `<Tab>` takes an item only while the menu's
  up; `<CR>` always starts a new line. `.` replays the `<C-n>` keys, so it completes again
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
