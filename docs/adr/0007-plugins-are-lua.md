# Plugins are Lua, in the same registry as built-ins

Plugins are Lua 5.4, embedded through `mlua` with Lua bundled, so users install nothing. A plugin's commands, key bindings, hooks, status line elements, views and decorations are registered into the same `CommandRegistry`, keymap and hooks the built-ins use, so they get the palette, `:`, undo, `.` and hooks with no special cases.

We chose Lua over a WASM runtime because plugins are small scripts that want to be easy to write and read (as Neovim's are), and over a separate process because commands run synchronously against the editor. Sandboxing untrusted plugins isn't a goal yet; if it becomes one, a WASM runtime could sit behind the same command and hook interfaces.
