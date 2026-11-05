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
| Normal  | `x` `dd` | Delete char / line |
| Normal  | `:` | Command line |
| Normal  | `3j`, `2dd`, ... | Counts work with every command |
| Insert  | `<Esc>` | Back to normal mode |
| Command | `<CR>` / `<Esc>` | Run / cancel |

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
