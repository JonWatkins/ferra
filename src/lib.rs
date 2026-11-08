//! A modal, terminal-based text editor inspired by Vim, Helix and Emacs.
//!
//! The crate is layered so that each part can be tested in isolation and,
//! eventually, split into separate crates (see `docs/ARCHITECTURE.md`):
//!
//! - [`core`]: pure text model and motions. No UI or terminal dependencies.
//! - [`editor`]: editor state (document, mode, command line, messages).
//! - [`commands`]: the named command registry. Every action the user can take
//!   is a command, which makes everything bindable and (later) scriptable.
//! - [`events`]: editor events that hooks (and later plugins) react to.
//! - [`input`]: key notation, keymaps and pending-key/count handling.
//! - [`config`]: TOML configuration (options, theme, keybindings).
//! - [`files`]: reading and writing files on disk (atomic saves).
//! - [`clipboard`]: the system clipboard (and an in-memory one for tests).
//! - [`cli`]: command-line argument parsing.
//! - [`plugins`]: Lua plugins (commands, hooks and key bindings).
//! - [`syntax`]: tree-sitter syntax highlighting.
//! - [`lsp`]: language servers: diagnostics, kept in sync with the buffers.
//! - [`ui`]: rendering, including the custom editor pane widget.
//! - [`app`]: wires everything together and owns the event loop.

pub mod app;
pub mod cli;
pub mod clipboard;
pub mod commands;
pub mod config;
pub mod core;
pub mod editor;
pub mod events;
pub mod files;
pub mod input;
pub mod lsp;
pub mod plugins;
pub mod syntax;
pub mod ui;

pub use app::App;
pub use config::Config;
