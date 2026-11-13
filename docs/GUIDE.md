# ferra user guide

Everything ferra does, key by key. For a short introduction see the
[README](../README.md); for how it's built, [ARCHITECTURE.md](ARCHITECTURE.md).

## Starting

```sh
ferra notes.md   # open (or create) a file
ferra a.rs b.rs  # open several files, one buffer each
ferra            # start on the splash screen
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
| Normal  | `za` `zc` `zo` | Toggle / close / open the fold at the cursor (a line and the lines indented under it) |
| Normal  | `zM` `zR`, `zf` + motion | Close / open every fold; fold any lines (`zfip`, visual `zf`) |
| Normal  | `zj` `zk` | Next / previous line that starts a fold |
| Normal  | `r` + char | Replace characters (`3rx`) |
| Normal  | `R` | Replace mode: type over the text (`<BS>` puts it back; `3Rab<Esc>` types it three times) |
| Normal  | `>` `<` + motion, `>>` `<<` | Indent / outdent lines (`>ip`, `3>>`; visual `>` `<`) |
| Normal  | `<S-Tab>` | Outdent the line, like `<<` (`3<S-Tab>`) |
| Insert  | `<S-Tab>` `<C-d>`, `<C-t>` | Outdent / indent the line by one level as you type, the cursor staying on its text (`<Tab>` puts in a level at the cursor) |
| Normal  | `=` + motion, `==` | Re-indent lines by the language's rules (`gg=G` for the whole file; visual `=`) |
| Normal  | `gu` `gU` `g~` + motion, `guu`... | Lowercase / uppercase / toggle case (visual `u` `U` `~`) |
| Normal  | `gcc`, `gc` + motion (`gcap`, `gcj`), visual `gc` | Toggle comments on those lines, using the file's language |
| Normal  | `gq` + motion (`gqip`, `gqq`), visual `gq`; `gw` | Reflow those lines to `text-width` (79), keeping indentation, comment markers and list items; `gw` leaves the cursor put. `auto-wrap` breaks lines as you type past it |
| Normal  | `~` | Toggle case under the cursor and move on |
| Normal  | `<C-a>` `<C-x>` (`5<C-a>`) | Add to / take from the number under or after the cursor: decimal (`-` signs, leading zeros kept), `0x` hex, `0b` binary |
| Visual  | `<C-a>` `<C-x>`, `g<C-a>` `g<C-x>` | The same for the first number on each selected line; the `g` forms make a sequence (+1, +2, +3...) |
| Normal  | `J`, `gJ` | Join lines (`3J`, visual `J`); `gJ` joins exactly, without adding a space or removing indentation |
| Normal  | `i` `a` `I` `A` `o` `O` | Enter insert mode; with a count, what you type goes in that many times (`80i-<Esc>`, `3oitem<Esc>`) |
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
| Normal  | `gi` | Insert where insert mode was last left in this buffer (`` `^ `` goes there) |
| Normal  | `gf` | Open the file named under the cursor (beside this file, or in the working directory); `name:12` or `name:12:5` goes to that line and column |
| Normal  | `<C-^>` (Ctrl-6) | Switch to the alternate buffer (`:b#`) |
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
| Visual  | `>` `<`, `<Tab>` `<S-Tab>` | Shift the selected lines (`3>` three times); `.` repeats on as many lines |
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
| Insert  | `<C-x><C-f>` | Complete a file name: from the directory the path before the cursor names, beside the current file (`<C-x><C-f>` again goes into a directory) |
| Insert  | `<C-x><C-l>` | Complete the whole line from the lines that start with what you've typed on it, nearest above first |
| Insert  | `<C-s>` | The signature of the call you're typing (it also comes up by itself after `(`) |
| Insert  | `<C-Space>` `<C-x><C-o>` | The language server's completions (they also come up by themselves as you type: see [Language servers](#language-servers)) |
| Insert  | `<Tab>` | With the language server's completions up: take the chosen one (or the first) |
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

## Find and replace

Besides Vim's `/` and `:s`, there's a find box like VS Code's, at the top
right of the window: `<Space>/` to find, `<Space>h` to find and replace
(in normal or visual mode).

```text
╭────────────────────────────────────────────────────────────╮
│ Find       total                                   2 of 3  │
│                                                            │
│ Replace    NUM                                             │
│                                                            │
│  Aa  Match case ⌃A    W  Whole word ⌃W    .*  Regex ⌃R     │
╰──────── ⏎ replace one  ⌃⏎ replace all  Tab find  Esc close ╯
```

Each toggle shows its key and is highlighted when on; the bottom edge
shows the keys for the field you're in. They're the keys actually bound
(rebind them in `[keys.find]` and the box follows), drawn as `⌃A` on macOS
and `C-a` elsewhere. It finds as you
type: the cursor goes to the first match, every match is
highlighted (the current one in its own colour), and the box counts them.
It starts with the selection, or the word under the cursor (or, once
you've searched, what you looked for last), selected so typing replaces it.

| Keys (in the box) | Action |
|-------------------|--------|
| `<CR>`, `<Down>` (`<C-n>`) | Next match; in the replace field, replace this match and go to the next |
| `<Up>` (`<C-p>`), `<S-CR>` | Previous match (`<S-CR>` needs the kitty keyboard protocol) |
| `<Tab>` | Between the find and replace fields (showing the replace field) |
| `<C-a>`, `<C-w>`, `<C-r>` | Toggle match case (`Aa`), whole words (`W`), regex (`.*`). VS Code's `<A-c>` `<A-w>` `<A-r>` work too where the terminal sends Alt (on macOS, with Option set to act as Alt) |
| `<C-CR>` (`<A-a>`) | Replace all (one undo step). `<C-CR>` needs the kitty keyboard protocol (Ghostty, kitty, WezTerm, iTerm2); elsewhere use `<A-a>` |
| `<Left>` `<Right>`, `<Home>` `<End>` (`<C-e>`), `<BS>` `<Del>` `<A-BS>` `<C-u>` | Edit the field |
| `<Esc>` | Close it, the cursor left at the match |

Text is found as it's typed, ignoring case, unless the toggles say
otherwise; with `.*` it's a Rust regex, and the replacement can use `$1`,
`${name}`. Closing it clears the highlights, as VS Code does; `n` and `N`
carry on with the same search and highlight its matches again (`:noh`
hides them). The colours are the
`ui.find.*` scopes and `ui.search.current`.

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

`|` puts several commands on one line: `:%s/a/b/g | w`. `:g`, `:v`,
`:normal` and `:sort` take everything after them, so `:g/TODO/s/TODO/DONE/ | normal A!`
runs both commands on each TODO line. In `:s`, a `|` inside the pattern or
replacement is regex alternation (`:s/cat|dog/pet/g`); only one after it
starts the next command. In other commands, `\|` is a literal `|`.

Ranges work before any `:` command: `%` (the whole file), `5`, `5,10`, `.`
(this line), `$` (the last), offsets like `.+3` and `$-1`, and `'<,'>` (the
last visual selection; `:` in visual mode fills it in). A range alone jumps
to its line (`:42`, `:$`). `:[range]d` and `:[range]y` delete or yank whole
lines.

### Folding

Folds come from indentation, so they work in any language: a line folds
with the lines indented under it, and a closing `}`, `)` or `]` after them.
A closed fold shows as its first line and `⋯ N more lines`; `j` and `k`
step over it, and `dd`, `yy`, `>>` and other line operators take all of it.
Searches, mark jumps and undo open a fold they land in; changing text
inside one opens it too.

### Sorting

`:sort` sorts the lines in a range (`:'<,'>sort` from a visual selection),
or the whole file, as one undo step:

```
:sort              by the text (capitals before lowercase)
:sort i            ignoring case
:sort n            by the first number in each line (lines without one first)
:sort! u           reversed, keeping only the first of identical lines
:sort /=\s*/       by what follows the first `=` on each line
:sort r /\d\+/     by the match itself
```

As in Vim, the sort is stable, lines a pattern doesn't match keep their
order before the sorted ones, and `//` is the last search.

## Buffers and the picker

| Keys / command | Action |
|----------------|--------|
| `<Space>b`, `:ls` | Buffer picker (starts on the previous buffer, so `<Space>b<CR>` toggles) |
| `<Space>f`, `:files [dir]` | File picker: fuzzy-find a file under the current directory (respects `.gitignore`). It opens at once and fills in as files are found, so you can start typing straight away |
| `<Space>p` | Command palette: every command, with its description |
| `<Space>r`, `:reg` | Register picker: what each register holds; choosing one pastes it after the cursor |
| `]b` `[b`, `:bn` `:bp` | Next / previous buffer |
| `]d` `[d` | Next / previous diagnostic (see [Language servers](#language-servers)) |
| `K` | Hover: what the language server says about the symbol under the cursor (`K` again scrolls) |
| `gd` `gD` `gy` | Go to the definition, declaration, or type's definition |
| `gr` `gI` | References, implementations: gone to if there's one, else listed in a picker |
| `<Space>d`, `<Space>D` | Diagnostics: this file's errors and warnings, or the whole project's, in a picker (errors first) |
| `<Space>s`, `<Space>S` | Symbols: this file's (functions, types, methods), or search the project's as you type, to go to one |
| `<Space>a` | Code actions: the language server's fixes and changes for the cursor (or the visual selection), in a picker |
| `<F2>` `<Space>n`, `:rename name` | Rename the symbol under the cursor everywhere (the keys start `:rename` with its name to edit) |
| `:b 2`, `:b name`, `:b #` | Switch by number, part of the name, or to the previous buffer |
| `:bd` | Close the buffer (`:bd!` discards changes) |
| `:wa`, `:wqa` | Write all / write all and quit |

In a picker, type to fuzzy-filter, move with `<C-n>`/`<C-p>` (or arrows,
`<Tab>`), `<CR>` to choose, `<Esc>` to close. Each buffer keeps its own
cursor, scroll position and undo history. `:e path` opens another buffer
without closing the current one, and `:q` refuses while *any* buffer has
unsaved changes.

## The mouse

With `mouse = true` (the default):

- **Click** in the text to move the cursor there (and to that window, if
  it's another one); in insert mode you stay in insert mode. **Double
  click** selects the word, **triple click** the line, and **dragging**
  selects (visual mode, so `d`, `y`, `c`... act on it).
- **The wheel** scrolls what's under the pointer: a window, the sidebar,
  the terminal panel's history, the picker or a popup.
- In the **sidebar**, a click on a file opens it, on a folder opens or
  closes it, and on a tab shows that view. A click on the **terminal
  panel** goes to it.
- In the **picker** and plugins' list popups, a click chooses. In the
  **find box**, a click flips a toggle or puts the cursor in a field. A
  click outside one of these closes it (from the find box, the click then
  goes on into the text).
- **Drag** the sidebar's edge, the terminal panel's title row, or the line
  between two windows to resize them.

In terminals that let programs set the mouse pointer (kitty, Ghostty,
WezTerm, foot, and iTerm2 from 3.7), it's a hand over things to click, a
text bar over text, and arrows over edges to drag. iTerm2 3.6 and earlier
keep their own pointer while a program uses the mouse.

While the terminal app reports the mouse, its own text selection needs
Shift held while dragging (Option in iTerm2 and Terminal.app); `:set
nomouse` (or `mouse = false`) hands the mouse back to the terminal.

## The sidebar

`<Space>e` opens the file tree on the left, with the current file selected,
and moves to it; `<Space>e` (or `<Esc>`) goes back to the editor, leaving
it open. `:explorer` (or `:Explore`) opens and closes it, as does `q` in it.

```text
 FERRA                        │   1 fn main() {
 ▾ src                        │   2     println!("hi");
   ▸ ui                       │   3 }
     main.rs                ● │
   Cargo.toml                 │
```

| Keys (in the tree) | Action |
|--------------------|--------|
| `j` `k`, arrows | Down / up (with a count: `3j`) |
| `gg` `G` | First / last row |
| `<CR>` | Open or close the folder, or open the file |
| `l`, `→` | Open the folder (or go into an open one), or open the file |
| `h`, `←` | Close the folder, or go to the folder it's in |
| `zM` | Close every folder |
| `R` | List the files again (a saved new file shows up by itself) |
| `<Tab>` `<S-Tab>` | The next / previous view, when plugins add some (see [Lua plugins](#lua-plugins)) |
| `:` `<Space>f` `<Space>b` `<Space>p` | As in normal mode |

It lists the folder ferra started in, folders first, leaving out what
`.gitignore` ignores and `.git` (dotfiles like `.github` are shown). The
current file is highlighted and files with unsaved changes have a `●`.
`sidebar-width` (30 columns, at most half the screen) sets its width; the
colours are the `ui.sidebar.*` theme scopes.

## The terminal panel

`<Space>t` (or `:terminal`) opens a shell in a panel under the windows, as
in VS Code, and moves to it; the shell starts the first time. While it has
focus, every key goes to the shell, except:

| Keys (in the panel) | Action |
|---------------------|--------|
| `<C-\><C-n>` | Back to the editor, leaving the panel open (`<Space>t` goes back to it) |
| ``<C-`>`` | Hide the panel; the shell keeps running, and `<Space>t` shows it again |
| `<S-PageUp>` `<S-PageDown>` | Scroll back through what's gone off the top (typing goes back to the bottom) |

The title row shows the shell's title and these keys. Colours, full-screen
programs (`htop`, `vim`, `less`) and pastes work. When the shell exits, a
key closes the panel; `:terminal-close` ends it any time. `terminal-height`
(12 rows) sets its height, and `shell` the shell (`$SHELL` by default).
``<C-`>`` needs a terminal with the kitty keyboard protocol (Ghostty,
kitty, WezTerm, iTerm2); elsewhere use `<C-\><C-n>`.

## Git

In a git repository, the gutter marks what's changed since you last
staged (`git add`), as VS Code's does: a green bar beside added lines, a
blue one beside changed lines, and a red mark where lines were removed.
They follow your edits as you type.

```text
   1▎fn main() {          (changed)
   2▁    let a = 1;       (lines removed below)
   3     let b = 2;
   4▎    let c = 3;       (added)
```

| Keys / command | Action |
|----------------|--------|
| `]c` `[c` | Next / previous change (with a count, that many on) |
| `:git_refresh` | Read what's staged again (also done on save, when the terminal regains focus, and on leaving the terminal panel) |

The status line shows the branch and the file's changed lines (`⎇ main
+1 ~1 -1`); it's the `git` element of `statusline-layout`.

Resting the cursor on a line in normal mode shows who last changed it,
after it in grey, as VS Code's GitLens does:

```text
   12     let files = parse(args);  Jon Watkins, 3 days ago • Parse the command line
```

It counts your unsaved edits (a line you've added says `You • not
committed yet`), runs in the background, and goes while you type.
`:git_blame` (or `git-blame = false`) turns it off.

### In the sidebar

The file tree colours changed files and shows git's letter beside them (`M`
modified, `A` added, `U` untracked, `!` a conflict), and tints folders with
changes inside. The sidebar's **Git** tab (`<Tab>` in the sidebar, or
`:git_view`) is Source Control, as in VS Code:

```text
  Files   Git
 SOURCE CONTROL        ⎇ main
 STAGED (1)
  M lib.rs  src
 CHANGES (1)
  U notes.md

 s stage  u unstage
 x discard  c commit
```

| Keys (in the Git tab) | Action |
|-----------------------|--------|
| `j` `k`, click | Select a file |
| `<CR>`, double click | Open it |
| `s` / `u` | Stage / unstage it |
| `S` / `U` | Stage / unstage everything |
| `x` | Throw away its unstaged changes (asks first; an untracked file is deleted) |
| `c` | Commit what's staged: asks for the message |
| `R` | Read git's status again |

### The git view

`<Space>g` (or `:git`) opens a full-screen git view, in the style of
lazygit, over the editor; `q` closes it.

```text
╭ [1] Files ───────────╮╭ [4] long.txt (unstaged) ─────────────╮
│ STAGED (1)           ││ @@ -1,6 +1,6 @@                       │
│  M long.txt          ││  line 2                               │
│ CHANGES (1)          ││ -line 3                               │
│  M long.txt          ││ +LINE 3                               │
╰──────────────────────╯│  line 4                               │
╭ [2] Branches (2) ────╮│                                       │
│ * main               ││                                       │
│   feature            ││                                       │
╰──────────────────────╯│                                       │
╭ [3] Commits ─────────╮│                                       │
│ f165f72 thirty lines ││                                       │
╰──────────────────────╯╰───────────────────────────────────────╯
 j/k hunks  s stage the hunk  ⌃d/⌃u scroll  Esc back  q close
```

The right-hand panel shows the diff of what's selected: a file's unstaged
or staged changes, a commit, or a branch's log.

| Keys (in the git view) | Action |
|------------------------|--------|
| `1` `2` `3` `4`, `<Tab>` `<S-Tab>`, click | Go to a panel |
| `j` `k`, click | Move (between hunks, in a file's diff) |
| `<CR>`, `l` | Into the file's diff, to pick hunks |
| `<Esc>`, `h` | Back out of the diff (or close the view) |
| `s`, `<Space>` | Stage the file (or unstage a staged one), or the hunk |
| `u` | Unstage the file, or the hunk |
| `a` / `A` | Stage / unstage everything |
| `x` | Discard the file's unstaged changes (asks first) |
| `c` | Commit what's staged (asks for the message) |
| `<C-d>` `<C-u>`, `J` `K`, the wheel | Scroll the diff |
| `R` | Read everything again |
| `f` / `p` / `P` | Fetch / pull / push, in the background |
| `<CR>`, `<Space>` (on a branch) | Switch to it |
| `n` | Make a branch where you are, and switch to it (asks for the name) |
| `d` / `D` | Delete the selected branch (asks first) / even if it isn't merged |
| `C` | Amend the last commit: what's staged goes into it, and its message opens to edit (says if it's been pushed) |
| `z` | Stash every change, untracked files too (asks for an optional message) |
| `Z` | Pick a stash to put back (it's dropped once it's back) |
| `H` | The selected file's history, in place of the commits; `H` or `<Esc>` there goes back to the whole log |
| `B` | The selected file's blame: who last changed each line |
| `q` | Close |

Fetching, pulling and pushing run in the background with git's progress
as the message (`Pushing: Writing objects: 45%`); the first push of a
branch sets its upstream, as `git push -u` does. They use your credential
helpers and ssh agent, and fail with a message rather than wait for a
password to be typed. Stashing and putting a stash back are refused
while buffers have unsaved changes, as switching branch is; open files
show the result. The sidebar's Git tab takes `f`, `p`, `P`, `C`, `z`
and `Z` too,
`:git_branch` picks a branch to switch to from anywhere, and the status
line shows how far ahead and behind its upstream the branch is (`⎇ main
↑2 ↓1`). Switching is refused while buffers have unsaved changes; after
it (and a pull), open files show what's now on disk.

A file's history lists the commits that changed it, newest first,
following it back through renames, and each shows only what it did to
that file (a rename as one). `<Space>l` (or `:git_history`) opens it for
the file you're editing; `:git_history path` for any file in the
repository.

`<Space>B` (or `:git_blame_view`) shows the whole file you're editing
with who last changed each line, as it is now (lines you've changed say
`Not committed`). Each run of lines from one commit is headed by its
summary, author and age, with a bar down it coloured by how recent the
change is among the file's (brightest for the newest; `ui.git.blame.*`
in the theme). The code is highlighted, the selected line's commit is
picked out, and its hash, author and date are along the bottom. `<CR>` opens that commit's diff, `H` the
file's history, and `<Esc>` goes back to where you were.

`git = false` turns all of this off. ferra runs the `git` command, so it
needs git installed; commits run your hooks, and their errors show as
messages.

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
`recording`, `separator`, `spacer`, and `plugin:<name>` for what a Lua
plugin's `editor.status` function returns (see [Lua plugins](#lua-plugins)).
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

A language can also set `tab-width`, `expand-tab`, `auto-indent`,
`text-width` and `auto-wrap` for its buffers, over `[editor]` (a buffer's
`:setlocal`, or a `:set` in it, still wins). By default Go and Makefiles
indent with tabs, and commit messages (`COMMIT_EDITMSG`) wrap at 72 as
you type:

```toml
[languages.python]
options = { tab-width = 4, text-width = 88 }
```

Languages without a built-in grammar (`make`, `git-commit`, or ones you
add) aren't highlighted, but still get their comment marker and options.

Colours are theme scopes (`keyword`,
`function`, `string`, `comment`, `type`...), each falling back to its
parent, so `function.method` uses `function` unless you set it:

```toml
[theme]
"keyword" = { fg = "#ff79c6", modifiers = ["bold"] }
"comment" = "#6272a4"
```

Turn it off with `syntax-highlighting = false` under `[editor]`.

## Language servers

ferra talks to language servers for errors and warnings as you type, and
to show and go to what the cursor is on. It
doesn't install them: each language names its server in the `[languages]`
table, and ferra starts it when you open a file in that language, in the
project's root (the nearest folder with one of the server's `roots`, such
as `Cargo.toml`, else the one with `.git`).

| Language | Server | Install |
|----------|--------|---------|
| Rust | `rust-analyzer` | `rustup component add rust-analyzer` |
| Python | `pyright-langserver` | `npm i -g pyright` |
| JavaScript, TypeScript, TSX | `typescript-language-server` | `npm i -g typescript typescript-language-server` |
| Go | `gopls` | `go install golang.org/x/tools/gopls@latest` |
| C, C++ | `clangd` | Xcode tools, or `brew install llvm` |
| Bash | `bash-language-server` | `npm i -g bash-language-server` |
| TOML | `taplo` | `cargo install taplo-cli --features lsp` |
| JSON, HTML, CSS | `vscode-json-language-server`, ... | `npm i -g vscode-langservers-extracted` |
| YAML | `yaml-language-server` | `npm i -g yaml-language-server` |
| Markdown | `marksman` | `brew install marksman` |

If a server isn't installed, opening such a file says so once, quietly,
and everything else works as before. `:lsp` lists the servers: running
(and what they're busy with), starting, stopped (and why) or not found.
While a server works, the status line says what on (`rust-analyzer:
Indexing 45%`). `:lsp-restart` restarts the buffer's server (or `:lsp-restart
gopls`, or `all`), for one that's stuck or stopped or whose config you
changed, and tries again servers that weren't found, so one installed
since starts without restarting ferra.

Problems are underlined, with the line number coloured and the first one's
message after the line (`● expected i32, found &str`). `]d` and `[d` go to
the next and previous one and show its message in full; the status line
counts errors and warnings (`E1 W2`).

`K` shows what the server knows about the symbol under the cursor (its
signature and documentation) in a popup, with code highlighted in its
language; `K` again scrolls a long one, and any other key closes it.
`gd` goes to the definition, `gD` the declaration and `gy` the definition
of its type; `gr` lists the references and `gI` the implementations (of a
trait or interface). A single place is gone to straight away, opening its
file if need be; several are listed in a picker, with each line's text.
Each go-to is a jump, so `<C-o>` comes back.

In insert mode the server's completions come up as you type: after two
letters of a word, or a character it completes after (`.`, `::`). The menu
shows each with its type or kind, the letters typed picked out. Nothing
goes in until you choose: `<C-n>` and `<C-p>` put the items in turn,
`<Tab>` or `<C-y>` takes the chosen one (or the first), and `<C-e>` puts
back what you typed. Typing more of the word narrows the list (those
starting with what you typed first); anything else (a space, `<Esc>`,
moving) closes it. Taking an item also makes the edits that come with it,
such as rust-analyzer adding `use std::collections::HashMap;`. Set
`auto-complete = false` under `[editor]` to have it only on `<C-Space>`
or `<C-x><C-o>`. `<C-n>` without the menu up still completes words from
the buffers, as in Vim.

Typing a call (`area(`) shows its signature above the line, with the
argument you're on picked out and its documentation below; it follows
as you type the arguments, and goes when the call's closed or you leave
insert mode. `<C-s>` asks for it anywhere in a call.

`:format` (or `:fmt`) has the server format the file, as one change (`u`
undoes it), keeping the cursor on its text. To format on every save, set
`format-on-save`, for one language or all:

```toml
[languages.rust]
options = { format-on-save = true }   # or format-on-save = true in [editor]
```

Saving waits up to 2 seconds for the server; if it can't format (none
running, an error, too slow), the file is saved as it is and the message
says why.

`<F2>` (or `<Space>n`) renames the symbol under the cursor: it starts
`:rename` with the current name for you to edit, and on `<CR>` the server
changes it everywhere it's used, in this file and others. Each file's
changes are one undo step; files that weren't open are opened in the
background and left unsaved, as the message says, so `:wa` saves them
(or `:b` to look first).

Resting the cursor on a symbol in normal mode highlights its other uses
in view (`highlight-references = false` under `[editor]` turns it off).

Inlay hints show what a server knows but the code doesn't say, in grey in
the text: variables' types, parameters' names, closing labels (`let
files: Vec<PathBuf> = cli::parse(args: …)`). They're asked for once
you've paused typing, move with your edits in between, and take room like
text: lines wrap around them, and the cursor, clicks and `gj`/`gk` step
over them. `:inlay_hints` (or `inlay-hints = false`) turns them off; their
colours are `ui.virtual.inlay-hint.type` and `.parameter`.

`<Space>d` lists this file's errors and warnings in a picker, errors
first, to filter and go to; `<Space>D` lists every open file's and those
the servers have found in files that aren't open (rust-analyzer checks
the whole crate on `:w`). Opening one of those shows its diagnostics
straight away.

`<Space>s` lists the file's symbols (`area`, "method in Circle") in a
picker, to filter and go to; `<Space>S` searches the whole project's, the
server looking again as you type. Either is a jump, so `<C-o>` comes back.

`<Space>a` lists what the server can do at the cursor, or for the
selection in visual mode: quick fixes for the diagnostics there
(rust-analyzer's "Import `std::collections::HashMap`"), and refactors
(TypeScript's "Convert parameters to destructured object", gopls's "Add
test for Area"). Choosing one makes its changes, in any files it names,
as with rename.

Some servers check more when you save: rust-analyzer finds syntax errors
as you type, but type and borrow errors come from `cargo check`, which it
runs on `:w`. And it only checks files that are part of a crate (in the
module tree from `main.rs` or `lib.rs`); for others it says "This file is
not included in any crates".

```toml
[languages.python]
language-server = { command = "pylsp" }          # another server

[languages.json]
language-server = { command = "vscode-json-language-server", args = ["--stdio"],
                    settings = { json = { validate = { enable = true } } } }

[languages.yaml]
language-server = { enabled = false }            # off for one language
```

`settings` are the server's own settings, as in its documentation. Turn
them all off with `lsp = false` under `[editor]`.

## Configuration

Create `~/.config/ferra/config.toml` (or under `$XDG_CONFIG_HOME`).
It's merged over [the defaults](../src/config/default.toml), which document
every option. Example:

```toml
[editor]
tab-width = 2
expand-tab = true           # indent with spaces; false for tabs
detect-indent = true        # but indent each file the way it already is
auto-indent = true          # indent new lines (by the language's rules where it has them)
wrap = false                # scroll long lines sideways rather than wrapping them
wrap-marker = ""             # no ↪ before wrapped lines' later rows
text-width = 72             # the width gq and gw reflow to
auto-wrap = "comments"      # break lines typed past text-width: "off", "comments", "all"
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
their own `wrap`, `wrap-indent`, `wrap-marker`, `line-numbers`,
`cursorline` and `scroll-off` (a split
starts with those of the window it came from); buffers their own
`tab-width`, `expand-tab`, `auto-indent` and `text-width`. `:set` changes an option
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

`editor.command(name, [description,] fn)`, `editor.on(event, fn)`,
`editor.map(mode, keys, command)` and `editor.status(name, fn)` are
available while loading. A status function returns text for the status
line; put it in the layout as `"plugin:name"`:

```lua
editor.status("words", function(ctx)
  local n = 0
  for _ in ctx:text():gmatch("%S+") do n = n + 1 end
  return n .. " words"
end)
```

```toml
[editor.statusline-layout]
right = ["plugin:words", "position"]
``` Callbacks get
a `ctx`:

| `ctx` method | |
|--------------|---|
| `line(row)`, `line_count()`, `text()` | Read the buffer (rows and columns start at 1) |
| `set_line(row, text)`, `insert_line(row, text)`, `delete_line(row)`, `insert(text)` | Edit it (undoable, repeatable with `.`) |
| `cursor()`, `set_cursor(row, col)` | The cursor |
| `path()`, `mode()` | The file and the current mode |
| `message(text)` | Show a message |
| `run(name, ...)` | Run any command, with arguments |
| `option(name)`, `set_option(name, value [, "local"])` | Read an option (`"tab-width"`, `"search.case"`), or set it everywhere or for this window or buffer, checked as config is |
| `popup{...}`, `popup_update{...}`, `popup_close()`, `popup_selected()`, `popup_name()` | Show a popup (below) and work with it |
| `decorate{...}`, `clear_decorations([group])` | Virtual text and highlighted ranges in the buffer (below) |
| `view_selected()`, `view_name()`, `view_refresh([name])` | The sidebar view shown, and making one again (below) |
| `pick{...}`, `input{...}` | Ask the user: a picker of the plugin's items, or a line of text (below) |

Plus `ctx.count` and `ctx.args`. Hooks also get an `event` table (`kind`,
and `path` or `from`/`to` where relevant). Raising a Lua `error` cancels the
command (or the save, in a `before-save` hook) and shows the message, with
the file and line it came from; use `error("message", 0)` for messages meant
for users. A
plugin that fails to load is reported without stopping the others. Plugin
commands appear in the command palette and can override built-in ones.

### Popups

`ctx:popup{...}` shows a bordered window over the editor: text, or a list
to choose from, with its own keys that run the plugin's commands.

```lua
editor.command("fruit", "Pick a fruit", function(ctx)
  ctx:popup({
    name = "fruit",                  -- so commands can tell popups apart
    title = "Fruit",
    lines = {
      "apple",                       -- a line is a string,
      { { "cherry", "keyword" }, { " (red)" } },  -- or spans: text, theme scope
    },
    footer = "⏎ insert  q close",    -- in the bottom border
    select = true,                   -- a list: j/k move a selected line
    keys = { ["<CR>"] = "fruit_insert" },  -- its own keys, run commands
    on_close = "fruit_closed",       -- optional: run when q/<Esc> closes it
    -- position = "center" | "cursor" | "top-right"; width, height;
    -- selected = 1; focus = false shows it until the next key, like a hover
  })
end)

editor.command("fruit_insert", function(ctx)
  local chosen = ({ "apple", "cherry" })[ctx:popup_selected()]
  ctx:popup_close()
  ctx:insert(chosen)
end)
```

While it has focus, its own keys come first, then `[keys.popup]`: `j`/`k`
(and arrows, `<C-n>`/`<C-p>`) move the selection or scroll, `<C-d>`/`<C-u>`
by half a page, `gg`/`G` to the ends, `q`/`<Esc>` close it, `:` for
commands. `ctx:popup_update{ lines = ..., title = ..., footer = ...,
selected = n }` changes it in place. A mistake in the table (an unknown
command in `keys`, a bad key) is the plugin's error, shown as a message.

### Asking the user

`ctx:pick{...}` opens the picker with the plugin's items, filtered as you
type; `ctx:input{...}` opens a box at the top of the window for a line of
text, its starting text selected so typing replaces it. Either runs a
plugin command with the answer as `ctx.args[1]`:

```lua
editor.command("colour", function(ctx)
  ctx:pick({
    title = "Colour",
    items = { "red", { label = "Green", detail = "#00ff00", value = "green" } },
    on_choose = "set_colour",          -- gets the item's value (or label)
  })
end)

editor.command("rename_thing", function(ctx)
  ctx:input({
    title = "New name",
    value = "old",                     -- to start with
    placeholder = "a name",            -- shown while it's empty
    on_submit = "rename_to",           -- gets the text
    on_cancel = "rename_cancelled",    -- optional, on <Esc>
  })
end)
```

In the box, `<CR>` submits, `<Esc>` cancels, and the usual keys edit
(`<Left>` `<Right>`, `<Home>` `<End>`, `<BS>` `<Del>`, `<C-w>`, `<C-u>`);
they're `[keys.input]`.
[`examples/plugins/snippets.lua`](../examples/plugins/snippets.lua) picks a
snippet to insert (`<Space>i`) and asks what to wrap a word in
(`<Space>w`).

### Decorations

`ctx:decorate{...}` marks up the current buffer without changing its text:
virtual text in a line or after it (as inlay hints and blame are drawn),
or a highlighted range. Decorations follow your edits, and belong to a
`group` the plugin names, so it can clear and redo them together.

```lua
-- A red background on columns 9 to 11 of line 1 (1-based, the last
-- column included, as string.find gives them).
ctx:decorate({ group = "mine", line = 1, col = 9, to_col = 11, style = "ui.decoration.error" })
-- Grey text after line 2 (or, without end_of_line, before column `col`).
ctx:decorate({ group = "mine", line = 2, text = "← too long", end_of_line = true, style = "ui.decoration.text" })
ctx:clear_decorations("mine")
```

`to_line` makes a range span lines. Any theme scope works as a `style`;
`ui.decoration.error`, `.warning`, `.info` (backgrounds) and
`ui.decoration.text` are there for plugins.
[`examples/plugins/tidy.lua`](../examples/plugins/tidy.lua) marks trailing
whitespace and says how far lines run past `text-width`, as you type.

### Sidebar views

`editor.view(name, {...})` adds a view to the sidebar, beside the file
tree: a row of tabs appears at its top, and `<Tab>`/`<S-Tab>` in the
sidebar switch views (`:sidebar_view name` opens one directly).

```lua
editor.view("todos", {
  title = "TODOs",
  -- Makes its lines (strings or spans, as in popups).
  lines = function(ctx)
    return { "first", { { "second", "keyword" } } }
  end,
  keys = { ["<CR>"] = "todos_view_go" },   -- its own keys, run commands
  refresh = { "buffer-changed" },          -- events that make it again
})
```

`j`/`k`, `gg`/`G` move the selection; its commands read it with
`ctx:view_selected()` (and `ctx:view_name()`). The `lines` function runs
when the view is shown, on `R`, when you switch buffers, after the events
in `refresh`, and when a command calls `ctx:view_refresh()`; never on
every redraw. An error in it shows in the view and as a message.

[`examples/plugins/todos.lua`](../examples/plugins/todos.lua) lists the file's
TODO, FIXME and XXX comments in a popup (`<Space>T`), `<CR>` to go to one,
and as a sidebar view that follows your edits.

[`examples/plugins/toolkit.lua`](../examples/plugins/toolkit.lua) is a
complete example (`:todo`, `:duplicate_line`, `:sort_paragraph`,
`:word_count`, `:insert_date`, `:toggle_wrap`, a `plugin:words` status
element and an after-save hook). Try it with:

```sh
mkdir -p ~/.config/ferra/plugins
ln -s "$PWD/examples/plugins/toolkit.lua" ~/.config/ferra/plugins/
```

See [docs/ARCHITECTURE.md](ARCHITECTURE.md) for the design.
