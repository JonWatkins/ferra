# Most languages are language packs, not built in

Only a core of languages is built in (Markdown and its inline grammar, TOML, JSON, YAML, Bash: ferra's own files and the ones met everywhere). Every other language is a language pack, a plugin package bringing file types, comment markers, options, a language server, and tree-sitter grammars that are compiled on install with the system's C compiler and loaded at runtime, with Neovim's queries (`highlights.scm`, `injections.scm`, `indents.scm`).

We chose this over building every grammar in because the grammars dominate the binary: moving Rust, Python, JavaScript, TypeScript/TSX, Go, C, C++, HTML and CSS out took the release binary from 21 MB to 12 MB, and new languages no longer need a release. The cost is needing a C compiler on install, which ferra offers to set up when one's missing.

## Consequences

- The moved languages are still ferra's test languages: the `test-languages` cargo feature compiles their grammars and rules in for the tests.
- Reading Neovim's queries means following their capture names and conventions, not designing our own.
