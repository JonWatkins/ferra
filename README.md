# ferra

```text
  __
 / _| ___ _ __ _ __ __ _
| |_ / _ \ '__| '__/ _` |
|  _|  __/ |  | | | (_| |
|_|  \___|_|  |_|  \__,_|
```

A modal, fully configurable terminal text editor written in Rust, inspired by
Vim, Helix and Emacs.

```sh
cargo run -- notes.md   # open (or create) a file
cargo run -- a.rs b.rs  # open several files, one buffer each
cargo run               # start on the splash screen
cargo install --path .  # install the `ferra` command
cargo test              # run all tests
cargo clippy --all-targets -- -D warnings
```

Save with `:w`, quit with `:q` (or `ZZ` to save and quit).

Started without a file, ferra shows a splash screen: the logo, the version
and a few ways to begin (`:e <file>`, `<Space>f` to find a file, `<Space>p`
for every command, `i` to start writing, `:q`). The keys listed are the ones
actually bound, so your own bindings show up there. It goes away once you
type, open a file or split the window.

## Default keys

| Mode    | Keys | Action |
|---------|------|--------|
| Normal  | `h` `j` `k` `l` / arrows | Move |
| Normal  | `w` `b` `e` | Word forward / backward / end |
| Normal  | `0` `^` `$` | Line start / first non-blank / end |
| Normal  | `gg` `G` (`5gg`, `5G`) | First / last line (or line 5) |
| Normal  | `f` `t` `F` `T` + char | To / just before the next (or previous) char on the line |
| Normal  | `;` `,` | Repeat the last `f`/`t`/`F`/`T` / in the other direction |
| Normal  | `%`, `50%` | Matching bracket; with a count, that percent of the way through the file |
| Normal  | `{` `}` | Previous / next empty line (paragraph) |
| Normal  | `<C-d>` `<C-u>`, `<C-f>` `<C-b>` (`<PageDown>` `<PageUp>`) | Scroll half a screen / a screen down or up, the cursor with it (`3<C-d>`: 3 lines) |
| Normal  | `<C-e>` `<C-y>` | Scroll a line down / up, the cursor staying put while it's in sight |
| Normal  | `zz` `zt` `zb` (`z.` `z<CR>` `z-`) | Scroll the cursor line to the middle / top / bottom (and go to its first non-blank) |
| Normal  | `H` `M` `L` (`3H`) | Top / middle / bottom line on screen; motions too (`dL`) |
| Normal  | `gj` `gk` (`g<Down>` `g<Up>`) | Down / up a screen row, through the rows of a wrapped line |
| Normal  | `r` + char | Replace characters (`3rx`) |
| Normal  | `>` `<` + motion, `>>` `<<` | Indent / outdent lines (`>ip`, `3>>`; visual `>` `<`) |
| Normal  | `=` + motion, `==` | Re-indent lines by the language's rules (`gg=G` for the whole file; visual `=`) |
| Normal  | `gu` `gU` `g~` + motion, `guu`... | Lowercase / uppercase / toggle case (visual `u` `U` `~`) |
| Normal  | `gcc`, `gc` + motion (`gcap`, `gcj`), visual `gc` | Toggle comments on those lines, using the file's language |
| Normal  | `~` | Toggle case under the cursor and move on |
| Normal  | `<C-a>` `<C-x>` (`5<C-a>`) | Add to / take from the number under or after the cursor: decimal (`-` signs, leading zeros kept), `0x` hex, `0b` binary |
| Visual  | `<C-a>` `<C-x>`, `g<C-a>` `g<C-x>` | The same for the first number on each selected line; the `g` forms make a sequence (+1, +2, +3...) |
| Normal  | `J`, `gJ` | Join lines (`3J`, visual `J`); `gJ` joins exactly, without adding a space or removing indentation |
| Normal  | `i` `a` `I` `A` `o` `O` | Enter insert mode |
| Normal  | `d` `c` `y` + motion/text object | Delete / change / yank (`dw`, `c$`, `yiw`, `di(`, `2d3w`) |
| Normal  | `dd` `cc` `yy` | Operate on whole lines |
| Normal  | `x` `X` `D` `C` `s` `S` `Y` | Shortcuts (`dl` `dh` `d$` `c$` `cl` `cc` `yy`) |
| Normal  | `p` `P` | Paste after / before |
| Normal  | `"a` (prefix) | Use register `a` (`"ayy`, `"ap`; `"A` appends, `"_` discards, `"0` last yank) |
| Normal  | `"/`, `":`, `"%`, `".` | Read-only registers: the last search, the last `:` command, the file name (`"%p`), the last inserted text |
| Normal  | `"1`-`"9`, `"-` | Recent deletes of a line or more (`"1` newest), and the last delete within a line. `"1p` then `.` pastes `"2`, `"3`... |
| Normal  | `u` `<C-r>` | Undo / redo |
| Normal  | `.` | Repeat the last change (`3.` replaces its counts: after `d2w`, deletes three words; later `.` keep the 3). After a visual change, it acts on as much text again from the cursor |
| Normal  | `q{a-z}` … `q` | Record a macro (`qA` appends); the statusline shows `recording @a` |
| Normal  | `@{a-z}`, `@@`, `@:` | Play a macro (`10@a`), the last one, or the last `:` command |
| Normal  | `m{a-z}`, `m{A-Z}` | Set a mark in this buffer / a global mark that jumps across buffers |
| Normal  | `'{mark}`, `` `{mark} `` | Jump to a mark's line / exact position (also after operators: `d'a`) |
| Normal  | `''`, ` `` ` | Back to where the cursor was before the last jump (again to toggle) |
| Normal  | `<C-o>`, `<C-i>` / `<Tab>` | Back / forward through the jump list, across buffers (`3<C-o>`) |
| Normal  | `g;`, `g,` | Older / newer place in the change list; `'.` is the last change |
| Normal  | `/` `?` | Search forward / backward (regex; matches preview as you type) |
| Normal  | `n` `N` | Next / previous match |
| Visual  | `/` `?` | Search extends the selection to the match (`v/foo<CR>d`); `<Esc>` keeps the selection as it was |
| Normal  | `d/foo<CR>`, `y?bar<CR>`, `dn`, `cN` | Searches work as motions after an operator: up to (not including) the match; `d2/x` uses the second match |
| Normal  | `*` `#` | Search forward / backward for the word under the cursor |
| Normal  | `v` `V` `<C-v>` `gv` | Select characters / lines / a block / reselect the last selection |
| Visual  | motions, text objects | Extend the selection / select an object (`viw`, `vi(`) |
| Visual  | `d` `x` `c` `s` `y` | Delete / change / yank the selection |
| Visual  | `D` `X` `C` `S` `Y` | Same, on whole lines |
| Visual  | `p` `P` | Replace the selection (`p` keeps the replaced text, so it can swap) |
| Visual  | `>` `<` | Shift the selected lines (`3>` three times); `.` repeats on as many lines |
| Visual  | `o` | Move to the other end of the selection (`O`: other side of the row, in a block) |
| Visual  | `v` `V` `<C-v>` `<Esc>` | Switch selection kind / leave |
| Block   | `d` `y` `c` `u` `U` `~` | Act on the rectangle; `y` then `p` pastes it back as a column |
| Block   | `I` `A` | Type on the first row; `<Esc>` copies it to every row (`A` pads short rows) |
| Block   | `$` | Extend every row to its own end (`$A;` appends to each line) |
| Block   | `D` `C` | Delete / change from the block to each row's end |
| Block   | `p` | A one-line register fills every row; a block replaces it column for column |
| Block   | `>` `<` | Shift the text from the block's left edge (text left of the block stays put) |
| Normal  | `:` | Command line |
| Normal  | `3j`, `2dd`, `3u`, ... | Counts work with every command |
| Insert  | `<Esc>` | Back to normal mode |
| Insert  | `<C-r>{register}` | Insert a register's text (`<C-r>"` the last yank, `<C-r>.` the last insert, `<C-r>+` the clipboard) |
| Insert  | `<C-n>` `<C-p>` | Complete the word before the cursor from the open buffers' words (nearest first); again for the next / previous match, round to what you typed |
| Insert  | `<C-y>` `<C-e>` | While completing: keep the match / put back what you typed (any other key keeps it and carries on) |
| Command | `<CR>` / `<Esc>` | Run / cancel |

Text objects: `iw` `aw`, `ip` `ap` (paragraph), `i"` `a"`, `i'` `a'`, `` i` `` `` a` ``, `i(` `a(`
(`ib`), `i{` `a{` (`iB`), `i[` `a[`, `i<` `a<`.

**System clipboard:** `"+` (and `"*`) is the system clipboard: `"+yy`
copies a line out, `"+p` pastes text in (text ending in a line break pastes
as whole lines). Set `clipboard = "unnamed"` under `[editor]` to make every
yank, delete and `p` use it, or `"off"` to never touch it.

Over SSH (or on a machine with no system clipboard), `"+` copies to the
clipboard of the computer you're sitting at instead, through your terminal
(an OSC 52 escape sequence, supported by iTerm2, kitty, WezTerm, Alacritty,
Windows Terminal, foot and others; in tmux, `set -g set-clipboard on`).
That only goes one way: paste with your terminal (Cmd/Ctrl+Shift+V).
`clipboard-provider = "system"` or `"terminal"` picks one always. Pasting into the
terminal (Cmd/Ctrl+V) inserts the text exactly as copied, with no
auto-indent, as one undo step. `.` and macros repeat pastes too.

**Indentation:** in Rust, JSON, TOML, JavaScript, TypeScript, Go, C, C++,
Bash, HTML and CSS, indentation follows the syntax tree. Enter and `o`
start a line at the right depth (Enter between `{}` opens an indented line
inside), a closing bracket typed first on a line jumps back to its opening
line's level, and `=` re-indents (`==`, `=ip`, `gg=G`). So do Bash's
`fi`, `done`, `else` and friends, and closing tags in HTML. Other files keep
the indentation of the line above, one level more after an opening bracket
or a trailing `:` in Python and YAML. In Python, a line after `return`,
`pass`, `break`... goes back a level, and typing the `:` of `else:`,
`elif ...:`, `except ...:` or `finally:` lines it up with its `if` or
`try`.

Each file is indented the way it already is: tabs, or however many spaces
its lines step in by, worked out from its first 1000 lines when it's
opened (and again on `:e!`). That covers `>>`, Tab, `=` and new lines.
Files with nothing indented yet use `expand-tab` and `tab-width`, and
`detect-indent = false` turns detection off. `:indent` shows the style in
use; `:indent tabs`, `:indent 2` (or `:indent spaces 2`) set it for the
buffer, and `:indent auto` looks at the text again.

Macros are stored in their register as key notation (`A;<Esc>j`), so
`"ap` pastes one to edit and `"ay$` yanks it back. Playback stops at the
first failing command. Text pasted into the terminal while recording is
kept between `<PasteStart>` and `<PasteEnd>` (`A<PasteStart>two<CR>lines<PasteEnd><Esc>`),
and goes back in as a paste: exactly, without auto-indent or insert-mode
bindings like `jk`.

## The command line

The `:` and `/`/`?` prompts support:

| Keys | Action |
|------|--------|
| `<Tab>` / `<S-Tab>` | Complete command names, file paths (`:e`, `:w`, `:sp`, `:vs`, `:files`) and buffer names (`:b`); again to cycle |
| `<Up>` `<Down>` (`<C-p>` `<C-n>`) | History, matching what you've typed (`:e<Up>` recalls earlier `:e` commands). Separate for `:` and searches |
| `<Left>` `<Right>`, `<Home>` `<End>` (`<C-b>` `<C-e>`) | Move the cursor |
| `<Del>`, `<C-w>`, `<C-u>` | Delete a character / the word before the cursor / everything before it |
| `<C-r>{register}` | Insert a register's first line (`/<C-r>"` searches for what you yanked) |
| `<C-r><C-w>`, `<C-r><C-a>` | Insert the word / WORD under the cursor (`:s/<C-r><C-w>/new/`) |

## Substitute and ranges

`:[range]s/pattern/replacement/[flags]` replaces matches, using the same
regex syntax as search:

```text
:s/foo/bar/          first `foo` on this line
:%s/foo/bar/g        every `foo` in the file
:5,10s/\s+$//        trailing spaces on lines 5-10
:s/(\w+), (\w+)/\2, \1/   swap two words (\1, \2 = groups; & = whole match)
:s#/usr#/opt#        any punctuation works as the delimiter
:s/b/~!/             ~ is the previous replacement (\~ for a literal ~)
```

Flags: `g` every match on the line, `c` confirm each one, `i`/`I`
ignore/respect case, `n` count only, `e` no error if nothing matches. An
empty pattern reuses the last search, and the substituted pattern becomes the
search for `n`. The whole substitution is one undo step, even with `c`.

With `c`, each match is shown with `replace with … (y/n/a/q/l)?`: `y`
replaces it, `n` skips it, `a` replaces it and all the rest, `l` replaces it
and stops, `q` or `<Esc>` stops.

`&` (or `:&`, or `:s` alone) repeats the last substitution on this line
without its flags; `:&&` keeps them; `g&` repeats it on every line, with its
flags, for the last search. They take ranges too (`:%&&`).

`:[range]g/pattern/command` runs a `:` command on every line matching the
pattern (the whole file by default); `:v` or `:g!` on every line that
doesn't. The lines are found first, so deleting some doesn't skip others:

```text
:g/TODO/d            delete every line with TODO
:v/\S/d              delete blank lines
:g/^fn/s/foo/bar/g   substitute only on lines starting with fn
:g/pattern           count matching lines and go to the last
:g/^fn/normal >>     indent every line starting with fn
```

`:[range]normal keys` (`:norm`) runs normal-mode keys as if typed: on each
line of the range from its start, or at the cursor with no range.
`:%normal A;` appends `;` to every line. An unfinished command (still in
insert mode) ends as if with `<Esc>`, and the whole run is one undo step.
Keys use the same notation as config and macros (`A;<lt>Esc>0x` types
`A;`, presses Esc, then `0x`); text that isn't valid notation, like
`i<div>`, is typed as it is.

`|` puts several commands on one line: `:%s/a/b/g | w`. `:g`, `:v` and
`:normal` take everything after them, so `:g/TODO/s/TODO/DONE/ | normal A!`
runs both commands on each TODO line. In `:s`, a `|` inside the pattern or
replacement is regex alternation (`:s/cat|dog/pet/g`); only one after it
starts the next command. In other commands, `\|` is a literal `|`.

Ranges work before any `:` command: `%` (the whole file), `5`, `5,10`, `.`
(this line), `$` (the last), offsets like `.+3` and `$-1`, and `'<,'>` (the
last visual selection; `:` in visual mode fills it in). A range alone jumps
to its line (`:42`, `:$`). `:[range]d` and `:[range]y` delete or yank whole
lines.

## Buffers and the picker

| Keys / command | Action |
|----------------|--------|
| `<Space>b`, `:ls` | Buffer picker (starts on the previous buffer, so `<Space>b<CR>` toggles) |
| `<Space>f`, `:files [dir]` | File picker: fuzzy-find a file under the current directory (respects `.gitignore`). It opens at once and fills in as files are found, so you can start typing straight away |
| `<Space>p` | Command palette: every command, with its description |
| `<Space>r`, `:reg` | Register picker: what each register holds; choosing one pastes it after the cursor |
| `]b` `[b`, `:bn` `:bp` | Next / previous buffer |
| `:b 2`, `:b name`, `:b #` | Switch by number, part of the name, or to the previous buffer |
| `:bd` | Close the buffer (`:bd!` discards changes) |
| `:wa`, `:wqa` | Write all / write all and quit |

In a picker, type to fuzzy-filter, move with `<C-n>`/`<C-p>` (or arrows,
`<Tab>`), `<CR>` to choose, `<Esc>` to close. Each buffer keeps its own
cursor, scroll position and undo history. `:e path` opens another buffer
without closing the current one, and `:q` refuses while *any* buffer has
unsaved changes.

## Windows

| Keys / command | Action |
|----------------|--------|
| `<C-w>v`, `:vs [file]` | Split side by side (optionally opening a file) |
| `<C-w>s`, `:sp [file]` | Split top and bottom |
| `<C-w>h` `j` `k` `l` (or arrows), `<C-w>w` | Move to the window in that direction / the next one |
| `<C-w>c`, `:close` | Close the window (its buffer stays open) |
| `<C-w>o`, `:only` | Close every other window |
| `<C-w>+` `<C-w>-` | Taller / shorter by [count] lines (`5<C-w>+`) |
| `<C-w>>` `<C-w><` | Wider / narrower by [count] columns |
| `<C-w>_` `<C-w>\|` | As tall / as wide as possible (or [count] lines / columns) |
| `<C-w>=` | Make every window the same size |
| `<C-w>x` | Swap with the next window in the row or column |
| `<C-w>r` `<C-w>R` | Rotate the windows in the row or column down / up |
| `<C-w>H` `J` `K` `L` | Move the window to the far left / bottom / top / right (full height or width) |
| `:resize N`, `:res +N`, `:res -N` | Set the height, or change it (`:resize` alone: as tall as possible) |
| `:vertical resize N` | The same for the width (`:vert res +5`) |
| `:q`, `<C-w>q` | Close the window; quits from the last one |
| `:qa` | Quit (refuses while any buffer has unsaved changes; `:qa!` doesn't) |

Each window has its own cursor and scroll position, so two windows can show
different parts of the same file. Edits show up in every window on that
buffer.

Like Neovim, every window has its own status line: the active one shows
the mode and is highlighted, the others show their file and cursor
position, dimmed. Between stacked windows the status line takes the place
of the separator, so it costs no space. `statusline = "global"` under
`[editor]` gives one status line for the whole screen instead (Neovim's
`laststatus=3`).

What the active status line shows is up to you, as in Helix: list elements
for its left, center and right.

```toml
[editor.statusline-layout]
left = ["mode", "file-base-name", "modified"]
center = ["file-type"]
right = ["file-line-ending", "separator", "position-percentage", "position"]
```

Elements: `mode`, `file-name` (shortened from the left to fit),
`file-base-name`, `modified`, `position`, `position-percentage`,
`total-lines`, `file-type`, `file-encoding` (only when not plain UTF-8),
`file-line-ending`, `indent` (`2 spaces`, `tabs`), `pending-keys`,
`recording`, `separator` and `spacer`.
Elements with nothing to show take no space.

Window sizes are kept as proportions, so they survive resizing the terminal.
Splitting or closing a window evens out its neighbours again, as Vim does.

## Files

| Command | Action |
|---------|--------|
| `:w` / `:w path` | Save, or write to `path` (a scratch buffer adopts it) |
| `:wq`, `:x`, `ZZ` | Save and quit (`:x`/`ZZ` only write if there are changes) |
| `:q` | Quit; refuses if there are unsaved changes |
| `:q!`, `ZQ` | Quit, discarding changes |
| `:e path` / `:e!` | Open a file in a new buffer (or switch to it) / reload the current one (`u` brings back what it discarded) |
| `:checktime` | Check every open file for changes made by other programs |
| `:fenc`, `:fenc utf-8` | Show the file's encoding, or convert it (saved on `:w`) |
| `:indent`, `:indent tabs` / `2` / `auto` | Show how the buffer indents, or set it (see Indentation) |

A trailing `!` overrides safety checks: unsaved changes, a file that changed
on disk since it was read, or writing over an existing file. Saves are
atomic and keep line endings (LF/CRLF), a missing final newline, file
permissions and symlinks. Hard-linked files, files owned by someone else,
and files in read-only directories are saved in place, so every name still
sees the change and the owner stays the same (the original is backed up
until the write succeeds).

Files keep their encoding. A byte-order mark (UTF-8 or UTF-16) is
recognised and kept; otherwise valid UTF-8 is UTF-8, and anything else is
read as Windows-1252 (Latin-1 plus a few symbols), set by
`fallback-encoding` under `[editor]` (`"shift_jis"`, `"gbk"`..., or
`"utf-8"` to refuse such files). The encoding shows in the status line when
it isn't plain UTF-8, and saving writes it back the same way. A character
the encoding can't hold stops the save with a message (convert with
`:fenc utf-8`), so nothing is silently replaced.

When another program changes an open file (git, a formatter, a tool
editing alongside you), the editor notices within a second while you're
idle in normal mode, and also when its terminal comes back into focus or
you switch to that buffer. A buffer without unsaved changes reloads itself,
keeping the cursor and marks (`u` undoes the reload). One with unsaved
changes keeps them and warns you: `:e!` loads the new version, `:w!` keeps
yours. `watch-files = false` leaves it to focus, buffer switches and
`:checktime`.

Search patterns use Rust [regex syntax](https://docs.rs/regex/latest/regex/#syntax),
plus Vim's `\<` and `\>` word boundaries. Case is "smart" by default
(case-sensitive only if the pattern has a capital letter). Matches stay
highlighted until `:noh` or `<Esc>` in normal mode. See `[editor.search]` in the defaults.

Any command can also be run by name, e.g. `:goto_line 10`, or `:42` to jump
to a line.

## Syntax highlighting

These languages are highlighted with tree-sitter, out of the box: Rust,
Python, JavaScript (with JSX), TypeScript, TSX, Go, C, C++, Bash, TOML,
JSON, YAML, HTML, CSS and Markdown. The status line shows the language.

Languages inside other languages are highlighted too: Markdown's bold,
italics, links and code spans, fenced code blocks in their own language
(` ```rust `, ` ```py `), `<script>` and `<style>` in HTML, and the
contents of Rust macro calls.

Each file's syntax tree is kept and updated as you edit, and only the lines
on screen are highlighted, so typing stays instant in files of tens of
thousands of lines.

A file's language comes from its name (`Makefile`, `.bashrc`), its extension,
or for scripts without one, the `#!` line (`#!/usr/bin/env python3`). The
rules are the `[languages]` table in the config, so you can add your own:

```toml
[languages.python]
extensions = ["py", "pyi", "star"]   # replaces the list, so keep the defaults

[languages.bash]
file-names = [".bashrc", "my-tool"]
```

Colours are theme scopes (`keyword`,
`function`, `string`, `comment`, `type`...), each falling back to its
parent, so `function.method` uses `function` unless you set it:

```toml
[theme]
"keyword" = { fg = "#ff79c6", modifiers = ["bold"] }
"comment" = "#6272a4"
```

Turn it off with `syntax-highlighting = false` under `[editor]`.

## Configuration

Create `~/.config/ferra/config.toml` (or under `$XDG_CONFIG_HOME`).
It's merged over [the defaults](src/config/default.toml), which document
every option. Example:

```toml
[editor]
tab-width = 2
expand-tab = true           # indent with spaces; false for tabs
detect-indent = true        # but indent each file the way it already is
auto-indent = true          # indent new lines (by the language's rules where it has them)
wrap = false                # scroll long lines sideways rather than wrapping them
line-numbers = "absolute"   # "absolute" | "relative" | "none"
scroll-off = 8
statusline = "global"       # one status line, not one per window
key-timeout = 500           # ms to wait for the rest of a sequence (j of jk)

[editor.cursor-shape]
insert = "underline"

[theme]
"ui.statusline.normal" = { fg = "black", bg = "#ff79c6", modifiers = ["bold"] }

[keys.insert]
"jk" = "normal_mode"

[keys.normal]
"<C-q>" = "quit"
"x" = ""                    # unbind a default
```

Any `[editor]` option can also be changed while running with `:set`, by
its name in the config (nested ones with a dot), Vim-style:

| Command | Does |
|---------|------|
| `:set tab-width=2`, `:set line-numbers=absolute` | Sets an option |
| `:set expand-tab`, `:set noexpand-tab`, `:set expand-tab!` | Turns a switch on, off, or over |
| `:set scroll-off+=2` | Adds to (or `-=` takes from) a number |
| `:set tab-width?` | Shows an option |
| `:set tab-width&` | Puts back its default |
| `:set` | Lists the options you've changed |

Several can go on one line (`:set ts=2 et`), and Tab completes names.
Vim's names work for the common ones: `ts`, `et`, `ai`, `so`, `cul`,
`hls`, `ws`, `nu`/`rnu` (for `line-numbers`) and `ic`/`scs` (for
`search.case`). Changes last until ferra quits; put them in the config to
keep them.

As in Vim, some options can also be set for one window or one buffer:
`:setlocal nowrap` in a split leaves other windows wrapping, and
`:setlocal ts=2` applies to this buffer wherever it's shown. Windows have
their own `wrap`, `line-numbers`, `cursorline` and `scroll-off` (a split
starts with those of the window it came from); buffers their own
`tab-width`, `expand-tab` and `auto-indent`. `:set` changes an option
everywhere (and drops this window's or buffer's own value), `:setglobal`
only where there's no value of its own, and `:setlocal` alone lists this
window's and buffer's own values. Lists are written as in the config, without spaces
(`:set statusline-layout.right=["position"]`).

With `jk` bound in insert mode, typing `j` waits up to `key-timeout`
milliseconds for the `k`; after that the `j` is typed as text. A key bound
both on its own and as the start of a longer binding (`g` and `gg`) runs the
short binding when the timeout passes. Alt keys you haven't bound act as
Esc followed by the key, since terminals send them the same way, so a fast
`<Esc>j` is never mistaken for Alt+j.

In terminals with the kitty keyboard protocol (kitty, WezTerm, foot,
Ghostty, Alacritty, recent iTerm2), ferra asks for it, so keys arrive
unambiguously: a fast `<Esc>j` doesn't run a bound `<A-j>` either, and
`<C-i>` and `<C-m>` can be bound apart from `<Tab>` and `<CR>` (unbound,
they act as those, and `<C-[>` is always `<Esc>`). `kitty-keyboard = false`
turns it off.

### Hooks

Run commands when something happens:

```toml
[hooks]
before-save = ["trim_trailing_whitespace"]
```

Events: `opened`, `before-save`, `after-save`, `mode-changed`,
`buffer-changed`. A `before-save` command that fails cancels the save.

## Lua plugins

Put Lua files in `~/.config/ferra/plugins/` (loaded in name order,
after `~/.config/ferra/init.lua`). Plugins register commands,
hooks and key bindings through the global `editor` table:

```lua
-- ~/.config/ferra/plugins/todo.lua
editor.command("todo", "Insert a TODO comment above the cursor", function(ctx)
  local row = ctx:cursor()
  local indent = ctx:line(row):match("^%s*")
  ctx:insert_line(row, indent .. "// TODO: ")
end)
editor.map("normal", "<Space>t", "todo")

editor.on("before-save", function(ctx, event)
  for row = 1, ctx:line_count() do
    ctx:set_line(row, (ctx:line(row):gsub("%s+$", "")))
  end
end)
```

`editor.command(name, [description,] fn)`, `editor.on(event, fn)` and
`editor.map(mode, keys, command)` are available while loading. Callbacks get
a `ctx`:

| `ctx` method | |
|--------------|---|
| `line(row)`, `line_count()`, `text()` | Read the buffer (rows and columns start at 1) |
| `set_line(row, text)`, `insert_line(row, text)`, `delete_line(row)`, `insert(text)` | Edit it (undoable, repeatable with `.`) |
| `cursor()`, `set_cursor(row, col)` | The cursor |
| `path()`, `mode()` | The file and the current mode |
| `message(text)` | Show a message |
| `run(name, ...)` | Run any command, with arguments |

Plus `ctx.count` and `ctx.args`. Hooks also get an `event` table (`kind`,
and `path` or `from`/`to` where relevant). Raising a Lua `error` cancels the
command (or the save, in a `before-save` hook) and shows the message, with
the file and line it came from; use `error("message", 0)` for messages meant
for users. A
plugin that fails to load is reported without stopping the others. Plugin
commands appear in the command palette and can override built-in ones.

[`examples/plugins/toolkit.lua`](examples/plugins/toolkit.lua) is a
complete example (`:todo`, `:duplicate_line`, `:sort_paragraph`,
`:word_count`, `:insert_date` and an after-save hook). Try it with:

```sh
mkdir -p ~/.config/ferra/plugins
ln -s "$PWD/examples/plugins/toolkit.lua" ~/.config/ferra/plugins/
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design.
