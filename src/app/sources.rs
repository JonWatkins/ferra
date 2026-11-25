//! What the event loop takes in besides input: background jobs, the
//! terminal panel's shell, git's jobs, language servers and agents, each
//! a [`Source`] in one table ([`SOURCES`]).
//!
//! Each time round, the loop asks whether any source is busy (then it
//! wakes every `BACKGROUND_POLL` to look, rather than waiting for input),
//! and calls [`App::update`]: every source takes in what arrived and sends
//! what's waiting, in the table's order, again while any of them changed
//! something (up to [`ROUNDS`] times). So what one source hands another
//! is taken in the same time round: an agent's tool asks a language server
//! (agents), the request goes out and the answer comes back (language
//! servers), and the agent gets it (agents again).
//!
//! One entry isn't input at all: the file tree following the active buffer
//! is in the table so it catches every switch, however made, before the
//! draw.
//!
//! A panic in one source is caught on its own: the message line names it,
//! and the others still run.

use super::{App, crash};
use crate::editor::agent::AgentStatus;
use crate::ui;

/// The most times round the sources one update goes, while they keep
/// changing things.
pub const ROUNDS: usize = 8;

/// Something the event loop takes in from, besides input: of the app, or
/// (in tests) of `T`.
pub struct Source<T = App> {
    /// What it is, for a panic's message ("updating the terminal").
    pub name: &'static str,
    /// Whether it's waiting on something that'll arrive by itself (then
    /// the loop keeps looking).
    pub busy: fn(&T) -> bool,
    /// Takes in what arrived and sends what's waiting. Returns whether
    /// anything changed that's shown.
    pub update: fn(&mut T) -> bool,
}

/// Updates each of `sources`, in order, again while any of them changed
/// something (up to [`ROUNDS`] times). A panic in one is caught and handed
/// to `caught` with its name; the others still run. Returns whether
/// anything changed (or panicked).
pub fn update_all<T>(
    on: &mut T,
    sources: &[Source<T>],
    mut caught: impl FnMut(&mut T, &str, &str),
) -> bool {
    let mut shown = false;
    for _ in 0..ROUNDS {
        let mut changed = false;
        for source in sources {
            match crash::catching(|| (source.update)(on)) {
                Ok(it_changed) => changed |= it_changed,
                Err(panic) => {
                    caught(on, source.name, &panic);
                    shown = true;
                }
            }
        }
        shown |= changed;
        if !changed {
            break;
        }
    }
    shown
}

/// Every source, in the order they're updated.
pub static SOURCES: &[Source] = &[
    Source {
        name: "background jobs",
        busy: |app| app.background.busy(),
        update: |app| app.handle_background(),
    },
    Source {
        name: "the terminal",
        busy: |app| app.editor.terminal_running(),
        update: |app| app.editor.update_terminal(),
    },
    Source {
        name: "the git job",
        busy: |app| app.editor.git_job_running(),
        update: |app| app.editor.update_git_job(),
    },
    Source {
        name: "the git view's diff",
        busy: |app| app.editor.git_gui_diff_loading(),
        update: |app| app.editor.update_git_diff(),
    },
    Source {
        name: "the line's blame",
        busy: |app| app.editor.git_blame_waiting(),
        update: |app| app.editor.update_git_blame(),
    },
    Source {
        name: "the language servers",
        busy: |app| app.lsp.borrow().active(),
        update: App::update_lsp,
    },
    Source {
        name: "the file tree following the buffer",
        busy: |_| false,
        update: |app| app.editor.follow_in_tree(),
    },
    Source {
        name: "the agent",
        busy: |app| app.agents.running() || app.editor.agent_chat().wants_attention(),
        update: App::update_agents,
    },
];

impl App {
    /// Whether any source is busy.
    pub(super) fn sources_busy(&self) -> bool {
        SOURCES.iter().any(|source| (source.busy)(self))
    }

    /// Takes in what every source has, again while anything changes (see
    /// the module's docs). Public so tests can drive it. Returns whether
    /// anything changed that's shown.
    pub fn update(&mut self) -> bool {
        update_all(self, SOURCES, |app, name, panic| {
            app.caught_panic(&format!("updating {name}"), panic);
        })
    }

    /// Sends the agent what the editor wants sent, and takes in what it
    /// said; the sidebar's spinner turns while it works.
    fn update_agents(&mut self) -> bool {
        let changed = self.agents.update(&mut self.editor);
        let spinning = self.editor.agent_chat().shown()
            && matches!(
                self.editor.agent_chat().status(),
                AgentStatus::Busy | AgentStatus::Starting
            );
        let frame = spinning.then(ui::agent::spinner_frame);
        let turned = frame.is_some() && frame != self.agent_spinner;
        self.agent_spinner = frame;
        changed || turned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_source_has_its_own_name() {
        let mut names: Vec<_> = SOURCES.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SOURCES.len());
    }

    /// Two made-up sources: an agent that asks a server, and the server.
    #[derive(Default)]
    struct World {
        asked: bool,
        sent: bool,
        answered: bool,
        heard: bool,
        updates: usize,
        caught: Vec<String>,
    }

    fn quiet(_: &World) -> bool {
        false
    }

    /// The agent (first in the table): asks, then takes the answer in.
    fn agent(w: &mut World) -> bool {
        w.updates += 1;
        if !w.asked {
            w.asked = true;
            return true;
        }
        if w.answered && !w.heard {
            w.heard = true;
            return true;
        }
        false
    }

    /// The server (after it): sends what was asked, answers next time.
    fn server(w: &mut World) -> bool {
        if w.asked && !w.sent {
            w.sent = true;
            return true;
        }
        if w.sent && !w.answered {
            w.answered = true;
            return true;
        }
        false
    }

    fn never_caught(_: &mut World, name: &str, panic: &str) {
        panic!("{name}: {panic}");
    }

    #[test]
    fn what_sources_hand_each_other_is_taken_in_by_one_update() {
        let sources = [
            Source {
                name: "agent",
                busy: quiet,
                update: agent,
            },
            Source {
                name: "server",
                busy: quiet,
                update: server,
            },
        ];
        let mut world = World::default();
        assert!(update_all(&mut world, &sources, never_caught));
        assert!(world.heard, "the agent heard the answer");
        // And then it's quiet: nothing changed, nothing shown.
        assert!(!update_all(&mut world, &sources, never_caught));
    }

    #[test]
    fn a_source_that_keeps_changing_is_given_a_few_rounds() {
        let sources = [Source {
            name: "chatty",
            busy: quiet,
            update: |w: &mut World| {
                w.updates += 1;
                true
            },
        }];
        let mut world = World::default();
        assert!(update_all(&mut world, &sources, never_caught));
        assert_eq!(world.updates, ROUNDS);
    }

    #[test]
    fn a_source_that_panics_is_named_and_the_rest_still_run() {
        let sources = [
            Source {
                name: "broken",
                busy: quiet,
                update: |_: &mut World| panic!("oops"),
            },
            Source {
                name: "agent",
                busy: quiet,
                update: agent,
            },
        ];
        let mut world = World::default();
        let shown = update_all(&mut world, &sources, |w, name, panic| {
            w.caught.push(format!("{name}: {panic}"));
        });
        assert!(shown);
        assert!(world.asked, "the agent still ran");
        assert_eq!(world.caught[0], "broken: oops");
    }
}
