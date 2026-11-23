# Dependencies point down; the editor knows nothing of the terminal

Modules depend only on those below them: `core` (text, history, pure motions) at the bottom; `editor` (state, modes, registers, operators) on it; `commands`, `input`, `config`, `lsp`, `agent`, `plugins` and `ui` on those; and `app` at the top, owning everything and running the loop. `core`, `editor` and `commands` have no terminal or UI dependencies (no ratatui, no crossterm), so they're tested directly, and could become crates of their own (as Helix's are) without restructuring. Lower modules never reach up: the editor queues requests (language server queries, background jobs, macros) that the app acts on, rather than calling it.

The `ui` only reads editor state; everything that changes it goes through commands, apart from typing text.
