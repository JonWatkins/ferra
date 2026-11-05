# terminal-editor

A modal, fully configurable terminal text editor written in Rust, inspired by
Vim, Helix and Emacs.

```sh
cargo run -- notes.md   # open (or create) a file
cargo run -- a.rs b.rs  # open several files, one buffer each
cargo run               # start with an empty scratch buffer
cargo test              # run all tests
cargo clippy --all-targets -- -D warnings
```

Save with `:w`, quit with `:q` (or `ZZ` to save and quit).

## Default keys

| Mode    | Keys | Action |
|---------|------|--------|
| Normal  | `h` `j` `k` `l` / arrows | Move |
| Normal  | `w` `b` `e` | Word forward / backward / end |
| Normal  | `0` `^` `$` | Line start / first non-blank / end |
| Normal  | `gg` `G` (`5gg`, `5G`) | First / last line (or line 5) |
| Normal  | `f` `t` `F` `T` + char | To / just before the next (or previous) char on the line |
| Normal  | `;` `,` | Repeat the last `f`/`t`/`F`/`T` / in the other direction |
| Normal  | `%` | Matching bracket |
| Normal  | `{` `}` | Previous / next empty line (paragraph) |
| Normal  | `r` + char | Replace characters (`3rx`) |
| Normal  | `i` `a` `I` `A` `o` `O` | Enter insert mode |
| Normal  | `d` `c` `y` + motion/text object | Delete / change / yank (`dw`, `c$`, `yiw`, `di(`, `2d3w`) |
| Normal  | `dd` `cc` `yy` | Operate on whole lines |
| Normal  | `x` `X` `D` `C` `s` `S` `Y` | Shortcuts (`dl` `dh` `d$` `c$` `cl` `cc` `yy`) |
| Normal  | `p` `P` | Paste after / before |
| Normal  | `"a` (prefix) | Use register `a` (`"ayy`, `"ap`; `"A` appends, `"_` discards, `"0` last yank) |
| Normal  | `u` `<C-r>` | Undo / redo |
| Normal  | `.` | Repeat the last change (`3.` replaces its count) |
| Normal  | `/` `?` | Search forward / backward (regex; matches preview as you type) |
| Normal  | `n` `N` | Next / previous match |
| Normal  | `*` `#` | Search forward / backward for the word under the cursor |
| Normal  | `v` `V` `gv` | Select characters / lines / reselect the last selection |
| Visual  | motions, text objects | Extend the selection / select an object (`viw`, `vi(`) |
| Visual  | `d` `x` `c` `s` `y` | Delete / change / yank the selection |
| Visual  | `D` `X` `C` `S` `Y` | Same, on whole lines |
| Visual  | `p` `P` | Replace the selection (`p` keeps the replaced text, so it can swap) |
| Visual  | `o` | Move to the other end of the selection |
| Visual  | `v` `V` `<Esc>` | Switch selection kind / leave |
| Normal  | `:` | Command line |
| Normal  | `3j`, `2dd`, `3u`, ... | Counts work with every command |
| Insert  | `<Esc>` | Back to normal mode |
| Command | `<CR>` / `<Esc>` | Run / cancel |

Text objects: `iw` `aw`, `i"` `a"`, `i'` `a'`, `` i` `` `` a` ``, `i(` `a(`
(`ib`), `i{` `a{` (`iB`), `i[` `a[`, `i<` `a<`.

## Buffers and the picker

| Keys / command | Action |
|----------------|--------|
| `<Space>b`, `:ls` | Buffer picker (starts on the previous buffer, so `<Space>b<CR>` toggles) |
| `<Space>f`, `:files [dir]` | File picker: fuzzy-find a file under the current directory (respects `.gitignore`) |
| `<Space>p` | Command palette: every command, with its description |
| `]b` `[b`, `:bn` `:bp` | Next / previous buffer |
| `:b 2`, `:b name`, `:b #` | Switch by number, part of the name, or to the previous buffer |
| `:bd` | Close the buffer (`:bd!` discards changes) |
| `:wa`, `:wqa`, `:qa` | Write all / write all and quit / quit |

In a picker, type to fuzzy-filter, move with `<C-n>`/`<C-p>` (or arrows,
`<Tab>`), `<CR>` to choose, `<Esc>` to close. Each buffer keeps its own
cursor, scroll position and undo history. `:e path` opens another buffer
without closing the current one, and `:q` refuses while *any* buffer has
unsaved changes.

## Files

| Command | Action |
|---------|--------|
| `:w` / `:w path` | Save, or write to `path` (a scratch buffer adopts it) |
| `:wq`, `:x`, `ZZ` | Save and quit (`:x`/`ZZ` only write if there are changes) |
| `:q` | Quit; refuses if there are unsaved changes |
| `:q!`, `ZQ` | Quit, discarding changes |
| `:e path` / `:e!` | Open a file in a new buffer (or switch to it) / reload the current one |

A trailing `!` overrides safety checks: unsaved changes, a file that changed
on disk since it was read, or writing over an existing file. Saves are
atomic and keep line endings (LF/CRLF), a missing final newline, file
permissions and symlinks. Files that aren't UTF-8 are refused rather than
risk corrupting them.

Search patterns use Rust [regex syntax](https://docs.rs/regex/latest/regex/#syntax),
plus Vim's `\<` and `\>` word boundaries. Case is "smart" by default
(case-sensitive only if the pattern has a capital letter). Matches stay
highlighted until `:noh` or `<Esc>` in normal mode. See `[editor.search]` in the defaults.

Any command can also be run by name, e.g. `:goto_line 10`, or `:42` to jump
to a line.

## Configuration

Create `~/.config/terminal-editor/config.toml` (or under `$XDG_CONFIG_HOME`).
It's merged over [the defaults](src/config/default.toml), which document
every option. Example:

```toml
[editor]
tab-width = 2
line-numbers = "absolute"   # "absolute" | "relative" | "none"
scroll-off = 8

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

### Hooks

Run commands when something happens:

```toml
[hooks]
before-save = ["trim_trailing_whitespace"]
```

Events: `opened`, `before-save`, `after-save`, `mode-changed`,
`buffer-changed`. A `before-save` command that fails cancels the save.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and the
plugin roadmap.
