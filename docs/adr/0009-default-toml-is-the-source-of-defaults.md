# The embedded `default.toml` is the one source of defaults

`src/config/default.toml` is embedded in the binary and is the only place defaults are written. The user's `config.toml` is deep-merged over it, so users write only what they change. The settings editor is built from it too: each setting's title, description and choices come from its comments and the config's own checks, and saving keeps the user file's comments (`toml_edit`).

We chose this over defaults in Rust code because one commented file is both the reference users read and the data the program loads, so the two can't disagree.

## Consequences

- Adding an option means a commented entry in `default.toml`, a field in the options, and a settings category; a test fails any setting left in "Other".
