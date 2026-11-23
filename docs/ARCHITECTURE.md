# Architecture

A map of ferra's modules: what each owns, the interface others use, and where its seams are. It's for finding your way; how a part works is in the `//!` docs of the file that implements it, why it's built that way is in [`docs/adr/`](adr/), and the words are in [`GLOSSARY.md`](../GLOSSARY.md).

`app` and `ui` are on top, `core` underneath, and nothing between touches the terminal ([ADR 0002](adr/0002-app-and-ui-on-top-core-underneath.md)). Every action is a named command ([ADR 0003](adr/0003-every-action-is-a-named-command.md)).

## The bottom

**core** (`src/core/`):
The pure text model: a buffer's text (a rope), positions and ranges, motions and text objects, undo history, wrapping, folding by indentation, `gq`, block columns, and converting a file's bytes to text and back. Pure functions over a `Buffer`; depends on nothing else in the crate.

**files** (`src/files/`):
The disk: reading files, saving them atomically ([ADR 0004](adr/0004-saving-goes-through-files.md)), listing a project (`walk_files`, honouring `.gitignore`), the trash, and path completion.

## The editor

**editor** (`src/editor/`):
All of the editor's state, mutated by commands and read by the UI: the buffers (`Document`s: text, path, cursor, history, marks, folds, diagnostics, virtual text), windows, mode, registers, the command line, pickers, popups, the sidebar's views, the panel, messages. Interface: `Editor`, its methods, and the requests it queues for the app (`take_lsp_requests`, background jobs, macros, the agent's outbox). Notable files:
- `document/`: one buffer.
- `windows/`: the split layout and `TextLayout`, the one place that knows how a window lays out lines.
- `lsp.rs`: what the editor asks language servers, and the screen's side of their answers ([ADR 0001](adr/0001-lsp-answers-are-structured.md)).
- `agent.rs`, `agent_session.rs`: the agent's chat and session; `agent_tools.rs`: what its tools do in the editor, and agent changes; `agent_lsp.rs`: the agent's side of language server answers.
- `git.rs`, `git_gui.rs`, `conflicts.rs`: the gutter, Source Control and the git view.
- `explorer.rs`, `project_search.rs`, `sidebar_views.rs`: the sidebar's views.
- `modeless.rs`: VS Code's keys over the modal engine ([ADR 0006](adr/0006-vscode-keys-over-a-modal-engine.md)).

**commands** (`src/commands/`):
The named command registry and every built-in command (`builtin/`), the `:` line's ex commands (`ex/`: ranges, `:s`, `:g`, `:sort`, `|`), and running hooks. Interface: `CommandRegistry::register` and `execute`, a handler taking a `Context` (editor, registry, `Invocation`).

**input** (`src/input/`):
Turns key presses into commands: key notation (`KeyPress`, `parse_keys`), the per-mode keymap trie, and counts, registers, pending sequences and timeouts (`InputHandler`). The only module besides `app` that sees crossterm, to convert its events.

**config** (`src/config/`):
The embedded `default.toml` deep-merged with the user's file ([ADR 0009](adr/0009-default-toml-is-the-source-of-defaults.md)): options, local options and `:set`, languages, the keymap presets, themes, and the settings schema. Interface: `Config`, `EditorOptions`.

**syntax** (`src/syntax/`):
Tree-sitter: a syntax tree per buffer kept up to date with its edits (`SyntaxTrees`), highlighting the rows a window shows, injections, and indentation rules. Built-in languages are a small core; the rest are language packs ([ADR 0005](adr/0005-languages-come-as-language-packs.md)).

**events** (`src/events.rs`):
What hooks can observe (`Opened`, `BeforeSave`...), fired before the work (a hook can change it) or after.

**clipboard** (`src/clipboard.rs`):
The system or terminal (OSC 52) clipboard behind `"+`/`"*`, as a trait tests replace with `MemoryClipboard`.

## Talking to other programs

**lsp** (`src/lsp/`):
Language servers: a client per server and project, keeping open buffers in sync, diagnostics, and turning LSP queries into requests and replies into typed answers for `Editor::lsp_answered`. Never knows who asked ([ADR 0001](adr/0001-lsp-answers-are-structured.md)). Interface: `Lsp::update` (each time round the loop), `Lsp::format` (`:format`'s blocking wait). Seam: `Spawner`, which tests replace with a fake server over pipes.

**agent** (`src/agent/`):
Coding agents over ACP (`client.rs`), and ferra's tools for them over MCP (`mcp.rs`, through `ferra --mcp-bridge`): `tools.rs` is the table of tools (name, description, schema, handler), `tool_calls.rs` takes each tool call from start to answer, giving up on a language server after 60s, and an agent change's answer waits until its files' diagnostics are settled (`diagnostics_wait.rs`). Interface: `Agents::update`; tool calls are `ToolCalls::start` and `poll`, given the time. Seam: `Spawner`, which tests replace with a fake agent.

**git** (`src/git/`):
Git, by running the `git` program, with line diffs in-process ([ADR 0008](adr/0008-git-is-the-git-program.md)): status, staged text, hunks, log, blame, branches, staging, committing, merges. Plain functions over a repository path; depends on nothing else in the crate.

**terminal** (`src/terminal/`):
A shell in a pseudo-terminal for the panel: `portable-pty`, a `vt100` parser, and keys as the bytes a terminal sends.

**plugins** (`src/plugins/`):
Lua plugins ([ADR 0007](adr/0007-plugins-are-lua.md)): the host (`PluginHost`, registering into the same command registry, keymap and hooks as built-ins), the `ctx` API, popups and prompts, plugin packages (`packages.rs`: git clones with a manifest, the lock file, the directory) and grammars compiled on install (`grammars.rs`).

**outbox** (`src/outbox.rs`):
Writing to another program (a server, an agent) on a thread of its own, so a slow reader never blocks the editor.

## The top

**ui** (`src/ui/`):
Drawing, with ratatui: each piece of the screen is a widget reading `&Editor` (windows and `editor_view/`, status line, sidebar, panel, popups, pickers, the git view, the agent sidebar). As it draws, each widget records the regions it drew in a `ScreenMap` (`screen_map.rs`), in paint order, each with a `Target` saying what it is; `ui::draw` returns it. Interface: `ScreenMap::at(cell)`, what's on top there, and `Target::pointer`.

**app** (`src/app/`):
Owns everything and runs the event loop: keys and their timeouts, the mouse (`mouse.rs`: every event matches on the `Target` at its cell, so clicks, the wheel and the pointer agree), recording for `.` and macros (`repeat.rs`, `macros.rs`), `:normal`, background jobs (`background.rs`), the session, crash handling, and the background sources (`sources.rs`: one table of background jobs, the terminal, git's jobs, `Lsp` and `Agents`, each saying whether it's busy and taking in what arrived). Interface for tests: `App::update`, every source until nothing changes. The only module that sees everything.

**main, cli, log** (`src/main.rs`, `src/cli.rs`, `src/log.rs`):
Start-up and command-line arguments, and the log file (`:log`).

## Tests

Unit tests sit beside the code (`#[cfg(test)] mod tests`); pure logic is tested there exhaustively. Integration tests (`tests/`) drive the whole `App` with key notation (`keys(&mut app, "ihello<Esc>dd")`) and render through ratatui's `TestBackend`; `tests/common/mod.rs` gives an app with the classic chrome (most tests) or as shipped. Language servers and agents are fakes in the test process, behind the `Spawner` seams.

## Not here

Plans and known gaps are [GitHub issues](https://github.com/JonWatkins/ferra/issues). How to use ferra is in [`GUIDE.md`](GUIDE.md).
