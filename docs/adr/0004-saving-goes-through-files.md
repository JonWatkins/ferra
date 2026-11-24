# A document's file is saved only through `files`, and nothing is substituted

Saving goes through `files`: a temp file in the same folder, fsynced, then renamed over the target, keeping permissions and following symlinks; the file is overwritten in place (after a backup copy) only where a rename would lose something (other hard links, an owner or group that can't be kept, a folder that won't take a temp file). Text is converted by `core::TextFormat`, which refuses bytes invalid in the file's encoding and characters the encoding can't hold rather than substitute anything, so an untouched file round-trips byte for byte.

We chose atomic writes over writing in place because a crash or a full disk mid-save must never leave a half-written file; and refusing over substituting because a silent `?` or U+FFFD in a saved file is data loss nobody notices.

## Consequences

- Every other change to the user's files goes through `files` too: a new file or folder (`create_file`, `create_dir`: the folders they're in made, and never replacing what's there, even something that appeared a moment ago), moving one (`rename`: never onto something that's there, checked just before the move, as std has no rename that refuses), and the bin (`trash`). They're called by the file tree (`editor/file_ops.rs`) and the agent's tools, which keep only what it means for the buffers and the tree.
- ferra's own data isn't bound by this (the session, logs, crash reports, installed plugins and grammars, the config folder), and reading files for display (diagnostics, git, completion) happens where it's needed.
