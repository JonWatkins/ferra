# Language server answers are structured; the asker presents them

`lsp/` turns an LSP query into protocol requests and the server's reply into a typed answer (locations, hover text, edits, code actions, or why there is none), and hands it back with the query's reply untouched. It never looks at who asked: picker, popup and message wording lives in the editor's screen presenter, and text for the agent lives in its own presenter beside the agent tools. Each query gets exactly one answer, and server edits come back as edits for the asker to apply, so the agent can save around its own changes.

We chose this over keeping the formatting in `lsp/` behind a reply sink (a smaller change), because that left `lsp/` branching on "is this the agent?" about a dozen times, writing the agent's wording, and keeping per-call state for it. That arrangement had already hidden a bug where code actions on an empty line never answered the agent.

## Consequences

- A new agent tool that uses a language server adds a query variant and agent wording, and does not touch the protocol code's presentation.
- Staleness (an answer arriving after the document changed) is decided by each presenter, not by `lsp/`.
- Waiting for a server that is still starting is decided by the reply: the agent waits, a key press answers at once.
