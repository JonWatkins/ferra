//! Editor events that hooks can react to.
//!
//! Events are how plugins (and `[hooks]` in config) observe the editor. Two
//! delivery styles exist:
//!
//! - **Before** events (`BeforeSave`) fire synchronously from the command
//!   doing the work, so a hook can change the outcome (edit the text before
//!   it's written) or veto it by returning an error.
//! - **After** events (`Opened`, `AfterSave`, `ModeChanged`,
//!   `BufferChanged`) are queued with [`Editor::emit`](crate::editor::Editor::emit)
//!   and dispatched by the app once the current key has been handled.
//!
//! `events::Event` lists what hooks can observe: `Opened`, `BeforeSave`,
//! `AfterSave`, `ModeChanged` and `BufferChanged`. Hooks are registered on the
//! `CommandRegistry` with `on(kind, source, handler)`, next to commands, so the
//! registry stays the single extension point. A hook gets the same `Context` as
//! a command.
//!
//! There are two delivery styles:
//!
//! - **Before** events fire synchronously from the command doing the work
//!   (`write` fires `BeforeSave` with `CommandRegistry::fire`). A hook can edit
//!   the document first, or return an error to cancel.
//! - **After** events are queued with `Editor::emit`, because the editor has no
//!   access to the registry. The app dispatches them once each key has been
//!   handled (`App::dispatch_events`). That's also when it emits
//!   `BufferChanged` if the key edited the text. Hooks can queue further
//!   events, so dispatch repeats, but stops after 8 rounds with an error so two
//!   hooks can't trigger each other forever. A hook's own edits form a separate
//!   undo step.

use std::path::PathBuf;

use serde::Deserialize;

use crate::editor::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A file was opened (or reloaded).
    Opened {
        path: PathBuf,
    },
    /// A save is about to happen. Hooks may edit the document; an error
    /// cancels the save.
    BeforeSave,
    /// The document was written to `path`.
    AfterSave {
        path: PathBuf,
    },
    ModeChanged {
        from: Mode,
        to: Mode,
    },
    /// The text changed while handling the last key.
    BufferChanged,
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Event::Opened { .. } => EventKind::Opened,
            Event::BeforeSave => EventKind::BeforeSave,
            Event::AfterSave { .. } => EventKind::AfterSave,
            Event::ModeChanged { .. } => EventKind::ModeChanged,
            Event::BufferChanged => EventKind::BufferChanged,
        }
    }
}

/// The kinds of event a hook can subscribe to. In config these are written
/// in kebab-case (`before-save`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Opened,
    BeforeSave,
    AfterSave,
    ModeChanged,
    BufferChanged,
}

impl EventKind {
    pub const fn id(self) -> &'static str {
        match self {
            EventKind::Opened => "opened",
            EventKind::BeforeSave => "before-save",
            EventKind::AfterSave => "after-save",
            EventKind::ModeChanged => "mode-changed",
            EventKind::BufferChanged => "buffer-changed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_match_events() {
        let changed = Event::ModeChanged {
            from: Mode::Normal,
            to: Mode::Insert,
        };
        assert_eq!(changed.kind(), EventKind::ModeChanged);
        assert_eq!(Event::BeforeSave.kind().id(), "before-save");
    }

    #[test]
    fn kinds_deserialize_from_kebab_case() {
        let kind: EventKind = toml::Value::String("after-save".into()).try_into().unwrap();
        assert_eq!(kind, EventKind::AfterSave);
    }
}
