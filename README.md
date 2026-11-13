<p align="center">
  <img src="docs/logo.svg" alt="ferra" width="460">
</p>

<p align="center">
  A modal terminal text editor written in Rust, inspired by Vim, Helix,
  Emacs and VS Code.
</p>

> [!WARNING]
> **ferra is in early development.** It changes quickly, has rough edges,
> and its keys, configuration and plugin API may change between versions.
> It isn't ready to be anyone's main editor yet.

## Features

- **Modal editing, Vim-style:** motions, operators, text objects, visual
  and block selections, registers, macros, marks, `.` to repeat, `:s` with
  ranges, folding.
- **Language servers:** diagnostics, completion, hover, go to definition
  and references, rename, code actions, formatting, signature help,
  symbols and inlay hints.
- **Syntax highlighting and indentation** with tree-sitter, for Rust,
  Python, JavaScript and TypeScript, Go, C and C++, and more.
- **An interface like VS Code's:** a sidebar with the file tree, a find and
  replace box, a terminal panel, pickers for files and commands, split
  windows, and the mouse.
- **Git:** changes in the gutter, Source Control in the sidebar, a
  full-screen git view (stage hunks or single lines, commit, switch and merge
  branches, push and pull, a file's history and blame), merge conflicts
  resolved in the editor, and blame for the cursor's line.
- **Lua plugins:** commands, keys, hooks, status line items, popups,
  sidebar views, pickers, input boxes and decorations.
- **Configurable:** every key, option and colour, in one TOML file.

## Install

ferra needs [Rust](https://www.rust-lang.org/tools/install) (stable).

```sh
git clone https://github.com/JonWatkins/ferra.git
cd ferra
cargo install --path .
```

Then `ferra notes.md` opens a file, and `ferra` on its own shows a splash
screen with ways to begin. Language features need the language's server
installed (`rust-analyzer`, `pyright`...; `:lsp` shows what's found), and
the git features need `git`.

## Getting started

| Keys | |
|------|---|
| `i`, `<Esc>` | Start typing, and stop |
| `:w`, `:q`, `ZZ` | Save, quit, save and quit |
| `u`, `<C-r>` | Undo, redo |
| `<Space>f`, `<Space>b` | Find a file, switch buffer |
| `<Space>p` | Every command, to search and run |
| `<Space>e` | The sidebar: files, Git, plugins' views |
| `<Space>/`, `<Space>h` | Find, find and replace |
| `<Space>t` | The terminal panel |
| `<Space>g`, `<Space>l`, `<Space>B` | The git view, the file's history, its blame |
| `K`, `gd`, `<Space>a` | Hover, go to definition, code actions |

`<Space>` is the leader; everything else is in the [user guide](docs/GUIDE.md).

## Configuration

Put your settings in `~/.config/ferra/config.toml`. They're laid over
[the defaults](src/config/default.toml), which list every option, so you
only write what you change:

```toml
[editor]
tab-width = 2
line-numbers = "absolute"   # or "relative", "none"
wrap = false                # scroll long lines sideways

[keys.insert]
"jk" = "normal_mode"
```

## Plugins

Lua files in `~/.config/ferra/plugins/` add commands, keys, hooks and
interface. For example:

```lua
editor.command("hello", "Say hello", function(ctx)
  ctx:message("Hello from " .. (ctx:path() or "a new buffer"))
end)
editor.map("normal", "<Space>H", "hello")
```

See [examples/plugins](examples/plugins) and the guide's
[Lua plugins](docs/GUIDE.md#lua-plugins) section.

Plugins others have published install from inside ferra:
`:plugin_install owner/repo` clones a GitHub repository (after asking:
plugins run with your permissions), `:plugin_list` shows what's installed
and `:plugin_remove name` removes one. See
[Installing plugins](docs/GUIDE.md#installing-plugins).

## Documentation

- [User guide](docs/GUIDE.md): every key, command and option.
- [Architecture](docs/ARCHITECTURE.md): how ferra is built, and what's next.

## Development

```sh
cargo run -- notes.md
cargo test
cargo clippy --all-targets -- -D warnings
```
