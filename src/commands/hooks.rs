//! Hooks: handlers that run when an [`Event`] fires.
//!
//! Hooks are registered on the [`CommandRegistry`] next to commands, so the
//! registry stays the single extension point plugins use. A hook gets the
//! same [`Context`] as a command, so it can edit the document or run other
//! commands.

use std::collections::HashMap;

use super::{CommandRegistry, CommandResult, Context, Invocation};
use crate::editor::Editor;
use crate::events::{Event, EventKind};

type HookHandler = Box<dyn Fn(&mut Context, &Event) -> CommandResult>;

struct Hook {
    /// Shown in error messages, e.g. `config` or a plugin's name.
    source: String,
    handler: HookHandler,
}

#[derive(Default)]
pub(super) struct Hooks {
    by_kind: HashMap<EventKind, Vec<Hook>>,
}

impl CommandRegistry {
    /// Runs `handler` whenever an event of `kind` fires. Hooks run in the
    /// order they were added.
    pub fn on(
        &mut self,
        kind: EventKind,
        source: impl Into<String>,
        handler: impl Fn(&mut Context, &Event) -> CommandResult + 'static,
    ) {
        self.hooks.by_kind.entry(kind).or_default().push(Hook {
            source: source.into(),
            handler: Box::new(handler),
        });
    }

    /// Runs the given commands, in order, whenever an event of `kind` fires.
    /// This is what `[hooks]` in config uses.
    pub fn on_run_commands(&mut self, kind: EventKind, commands: Vec<String>) {
        self.on(kind, "config", move |cx, _| {
            for name in &commands {
                cx.registry
                    .execute(name, cx.editor, Invocation::default())?;
            }
            Ok(())
        });
    }

    /// Runs every hook for `event`. Stops at the first error, which names
    /// the event and the hook's source.
    pub fn fire(&self, event: &Event, editor: &mut Editor) -> CommandResult {
        let Some(hooks) = self.hooks.by_kind.get(&event.kind()) else {
            return Ok(());
        };
        for hook in hooks {
            let mut cx = Context {
                editor: &mut *editor,
                registry: self,
                count: None,
                register: None,
                args: &[],
                bang: false,
                char_arg: None,
            };
            (hook.handler)(&mut cx, event)
                .map_err(|e| format!("{} hook ({}): {e}", event.kind().id(), hook.source))?;
        }
        Ok(())
    }

    /// Whether any hook is registered for `kind`.
    pub fn has_hooks(&self, kind: EventKind) -> bool {
        self.hooks.by_kind.get(&kind).is_some_and(|h| !h.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::editor::Mode;

    #[test]
    fn hooks_run_in_order_for_their_event_only() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = CommandRegistry::new();
        for name in ["first", "second"] {
            let log = log.clone();
            registry.on(EventKind::BeforeSave, name, move |_, _| {
                log.borrow_mut().push(name);
                Ok(())
            });
        }

        let mut editor = Editor::default();
        registry.fire(&Event::BufferChanged, &mut editor).unwrap();
        assert!(log.borrow().is_empty());
        registry.fire(&Event::BeforeSave, &mut editor).unwrap();
        assert_eq!(*log.borrow(), ["first", "second"]);
    }

    #[test]
    fn hooks_receive_the_event() {
        let mut registry = CommandRegistry::new();
        registry.on(EventKind::ModeChanged, "test", |cx, event| {
            if let Event::ModeChanged {
                to: Mode::Insert, ..
            } = event
            {
                cx.editor.document.insert_str("entered");
            }
            Ok(())
        });
        let mut editor = Editor::default();
        let event = Event::ModeChanged {
            from: Mode::Normal,
            to: Mode::Insert,
        };
        registry.fire(&event, &mut editor).unwrap();
        assert_eq!(editor.document.buffer().text(), "entered");
    }

    #[test]
    fn errors_stop_later_hooks_and_name_the_source() {
        let mut registry = CommandRegistry::new();
        registry.on(EventKind::BeforeSave, "linter", |_, _| Err("bad".into()));
        registry.on(EventKind::BeforeSave, "never", |_, _| {
            panic!("should not run")
        });
        let err = registry
            .fire(&Event::BeforeSave, &mut Editor::default())
            .unwrap_err();
        assert_eq!(err, "before-save hook (linter): bad");
    }

    #[test]
    fn config_hooks_run_commands() {
        let mut registry = CommandRegistry::with_builtins();
        registry.on_run_commands(EventKind::BeforeSave, vec!["insert_mode".into()]);
        let mut editor = Editor::default();
        registry.fire(&Event::BeforeSave, &mut editor).unwrap();
        assert_eq!(editor.mode(), Mode::Insert);
        assert!(registry.has_hooks(EventKind::BeforeSave));
        assert!(!registry.has_hooks(EventKind::AfterSave));
    }
}
