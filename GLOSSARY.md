# ferra

A terminal code editor with a coding agent beside it. These are the words the code and its docs use for ferra's own concepts.

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

## Language servers

**LSP query**:
One question for a language server about a document, such as hover, definitions, rename or code actions, carrying that question's own arguments.
_Avoid_: LSP request (that is the protocol message a query becomes), feature

**Answer**:
What an LSP query gets back: exactly one per query, either a result or the reason there is none (unsupported, server still starting, server stopped, server error).
_Avoid_: response, result

**Reply**:
Who asked an LSP query and where its answer goes: a key press, the pointer, typing, a view, or an agent tool call.
_Avoid_: caller, origin, manual

**Edits**:
Changes to documents a language server proposes, already in the editor's own terms, for the asker to apply.
_Avoid_: workspace edit (the protocol's form)
