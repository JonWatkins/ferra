# Language server answers are structured; the asker presents them

`lsp/` turns an LSP query into protocol requests and the server's reply into a typed answer (locations, hover text, edits, code actions, or why there is none), and hands it back with the query's reply untouched. It never looks at who asked: picker, popup and message wording lives in the editor's screen presenter, and text for the agent lives in its own presenter beside the agent tools. Each query gets exactly one answer, and server edits come back as edits for the asker to apply, so the agent can save around its own changes.

We chose this over keeping the formatting in `lsp/` behind a reply sink (a smaller change), because that left `lsp/` branching on "is this the agent?" about a dozen times, writing the agent's wording, and keeping per-call state for it. That arrangement had already hidden a bug where code actions on an empty line never answered the agent.

## Consequences

- A new agent tool that uses a language server adds a query variant and agent wording, and does not touch the protocol code's presentation.
- Staleness (an answer arriving after the document changed) is decided by each presenter, not by `lsp/`.
- Waiting for a server that is still starting is decided by the reply: the agent waits, a key press answers at once.
- `lsp/` keeps nothing between queries: a code action or a completion item carries the server's own form of itself (which only `lsp/` reads) back in the query that makes or resolves it, and the asker keeps its own lists.
- A question `lsp/` sends to a server in parts (the agent's code actions at a symbol and for its line) is still one query with one answer.
- The editor records the files it moves (the agent's moves and the file tree's) and `lsp/` tells the servers, so it never needs to know who moved them.
- Edits a server pushes while running a code action's command (`workspace/applyEdit`) are for the query that ran it: the asker makes them as they come (the server waits on them), and the command's end is the query's one answer, so the agent's are one agent change. A server that answers the command before pushing its edits gets them made and shown on the screen only.
- One exception remains: `:format` (and format-on-save) calls into `lsp/` and waits for the answer itself, since saving has to block on it.
