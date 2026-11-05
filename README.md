# terminal-editor

A modal, fully configurable terminal text editor written in Rust, inspired by
Vim, Helix and Emacs.

```sh
cargo run            # start the editor
cargo test           # run all tests
cargo clippy --all-targets -- -D warnings
```

Quit with `:q`.

## Default keys

| Mode    | Keys | Action |
|---------|------|--------|
| Normal  | `h` `j` `k` `l` / arrows | Move |
| Normal  | `w` `b` `e` | Word forward / backward / end |
| Normal  | `0` `^` `$` | Line start / first non-blank / end |
| Normal  | `gg` `G` (`5gg`, `5G`) | First / last line (or line 5) |
| Normal  | `i` `a` `I` `A` `o` `O` | Enter insert mode |
| Normal  | `d` `c` `y` + motion/text object | Delete / change / yank (`dw`, `c$`, `yiw`, `di(`, `2d3w`) |
| Normal  | `dd` `cc` `yy` | Operate on whole lines |
| Normal  | `x` `X` `D` `C` `s` `S` `Y` | Shortcuts (`dl` `dh` `d$` `c$` `cl` `cc` `yy`) |
| Normal  | `p` `P` | Paste after / before |
| Normal  | `"a` (prefix) | Use register `a` (`"ayy`, `"ap`; `"A` appends, `"_` discards, `"0` last yank) |
| Normal  | `u` `<C-r>` | Undo / redo |
| Normal  | `:` | Command line |
| Normal  | `3j`, `2dd`, `3u`, ... | Counts work with every command |
| Insert  | `<Esc>` | Back to normal mode |
| Command | `<CR>` / `<Esc>` | Run / cancel |

Text objects: `iw` `aw`, `i"` `a"`, `i'` `a'`, `` i` `` `` a` ``, `i(` `a(`
(`ib`), `i{` `a{` (`iB`), `i[` `a[`, `i<` `a<`.

Command line: `:q`, `:42` (go to line), or any command by name, e.g.
`:goto_line 10`.

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

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and the
plugin roadmap.
