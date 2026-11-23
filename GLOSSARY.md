# ferra

A terminal code editor with a coding agent beside it. These are the words the code and its docs use for ferra's own concepts.

## Editing

**Buffer**:
An open file (or unsaved text) in the editor, with its own cursor and undo history; one or more windows may show it.
_Avoid_: document (the code's name for it), file (a buffer may have none yet)

**Window**:
A view onto a buffer in the editor area, with its own cursor and scroll position, split beside or below others.
_Avoid_: pane, split, view

**Command**:
A named action (`delete_line`, `git_commit`) that keys, the `:` line, menus, the palette and plugins all run the same way.
_Avoid_: action, function

**Keymap**:
The preset that decides how keys behave: VS Code's (the default, without modes) or Vim's.
_Avoid_: key bindings (the user's own changes on top of it)

**Picker**:
A box of items to choose from, filtered by typing: files, buffers, commands, symbols, places.
_Avoid_: quick open, palette (the picker of commands)

**Popup**:
A small box over the editor that asks something or lists choices, from the editor or a plugin.
_Avoid_: dialog, modal

**Sidebar view**:
One thing the sidebar can show: the file tree, Search, Source Control, or a plugin's list.
_Avoid_: panel (the bottom area), tab

**Panel**:
The area along the bottom with Problems, Output, Terminal and Locations tabs.
_Avoid_: bottom bar, console

**Virtual text**:
Text drawn in a buffer that isn't in the file: inlay hints, line blame, a plugin's decorations.
_Avoid_: ghost text, annotation

## Extending

**Plugin**:
Lua that adds commands, keys, hooks, views or decorations, registered the same way as the editor's own.
_Avoid_: extension, add-on

**Plugin package**:
A plugin installed from a git repository, described by its `ferra-plugin.toml` and pinned in `plugins.lock`.
_Avoid_: package (alone), bundle

**Language pack**:
A plugin package that adds a language: file types, comment markers, options, a language server, and grammars compiled on install.
_Avoid_: language plugin, grammar package

**Hook**:
Commands the config or a plugin runs when an event happens (a file opened, a buffer about to be saved).
_Avoid_: callback, listener

## The coding agent

**Agent**:
A coding assistant (Claude Code, over ACP) running beside the editor in the agent sidebar, working on the user's project through agent tools.
_Avoid_: assistant, bot, AI

**Agent tool**:
One thing the agent can ask the editor to do or tell it (read a file, rename a symbol, show locations), offered to the agent over MCP.
_Avoid_: command (that word is for the editor's named commands), MCP tool

**Agent change**:
A change the agent makes to the user's files through the editor, after which the files that were saved before it are saved again and those with the user's unsaved work are left unsaved.
_Avoid_: agent edit, agent write

**Settled**:
The language servers have had their say about the files an agent change touched: they've reported since and gone quiet, or said nothing new for long enough, or the wait for them ran out. The change's answer to the agent waits for this, so it can say what's wrong in those files now.
_Avoid_: done, finished (the servers may still report more later)

## Language servers

**LSP query**:
One question for a language server about a document, such as hover, definitions, rename or code actions, carrying that question's own arguments.
_Avoid_: feature (the kind of question, without its arguments)

**LSP request**:
An LSP query waiting to be sent: the query, the place in the document it's about, and its reply.
_Avoid_: message (the protocol's word for what goes over the wire)

**Answer**:
What an LSP query gets back: exactly one per query, either a result or the reason there is none (no server, still starting, not offered, stopped, an error, nothing there).
_Avoid_: response (the server's protocol reply, which an answer is made from), result

**Reply**:
Who asked an LSP query and where its answer goes: a key press, the pointer, typing, a view, or an agent tool call.
_Avoid_: caller, origin, manual

**Edits**:
Changes to documents a language server proposes, already in the editor's own terms, for the asker to apply.
_Avoid_: workspace edit (the protocol's form)

**Action list**:
The code actions a server offered for one place, numbered and kept by whoever asked (the screen's picker, or the agent) to make one from while the text is unchanged.
_Avoid_: stash
