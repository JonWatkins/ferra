# Git is the `git` program, not a library

Everything ferra does with Git (status, staged text, diffs against the index, blame, log, staging, committing, branches, fetch, pull and push, merges) runs the `git` program and reads its plumbing output; line diffs for the gutter are made in-process with `similar`. There's no Git library.

We chose this over `gix` or `git2` because running `git` gets its exact behaviour, the user's config, hooks, credential helpers and ssh setup for free, and the user can reproduce anything ferra does at a shell. The cost is a process per call, so the slow ones (fetch, pull, push, a large diff or log) run in the background.
