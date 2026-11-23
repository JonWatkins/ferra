# `app` and `ui` on top, `core` underneath, no terminal in between

Three rules hold the modules apart:

- **`app` and `ui` are on top.** `app` owns everything and runs the loop; `ui` draws, only reading editor state. Nothing else refers to either, so the editor never calls the app: it queues requests (language server queries, background jobs, macros, agent messages) that the app acts on.
- **`core` is underneath.** Text, positions, motions, ranges and undo history depend on nothing else in the crate.
- **No terminal in between.** Only `app` and `input` (turning crossterm's events into key presses) use crossterm; only `ui`, `app` and the theme (`config::theme`'s styles) use ratatui. So `editor`, `commands`, `lsp`, `agent`, `plugins` and the rest are tested directly, without a terminal.

Between `core` and the top, modules aren't layered among themselves: `editor` holds the state and uses types from `input` (`KeyPress`), `config` (`EditorOptions`), `terminal`, `git` and `plugins::packages`, and `input`, `config` and `commands` use the editor's (`Mode`). We chose to allow that, over a strict layering, because these are shared value types, not calls back up, and splitting them into a lower types module would scatter each concept over two places.

## Consequences

- The modules between could become crates of their own only together, or after their shared types move down.
- All mutation goes through commands, apart from literal text insertion.
