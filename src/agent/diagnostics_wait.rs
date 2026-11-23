//! Waiting for the language servers to have their say about the files an
//! agent change touched, so the change's answer can tell the agent what's
//! wrong in them now ([`DiagnosticsWait`]).
//!
//! The first report after a change isn't enough: rust-analyzer starts
//! `cargo check` a moment after a save, and reports the old check's
//! results (moved to where their lines are now) before the new one's. So
//! the answer waits until the servers are quiet: not busy, and no new
//! diagnostics for [`SETTLED`]. A server that says nothing new for
//! [`SILENT`] after the change (or after it was last busy) has nothing new
//! to say; after [`DIAGNOSTICS_WAIT`] the answer goes anyway, saying which
//! server is still checking.
//!
//! Pure: it's handed each file's diagnostics state and the time, so tests
//! walk it along a made-up timeline.

use std::time::{Duration, Instant};

use crate::editor::diagnostics::DocDiagnostics;

/// How long a change's answer waits for its files' diagnostics, at most.
pub const DIAGNOSTICS_WAIT: Duration = Duration::from_secs(30);
/// How long the servers must have been quiet (no new diagnostics, not
/// busy) before what they've said counts. Also how long a server set up
/// for a file's language gets to open it.
pub const SETTLED: Duration = Duration::from_millis(1000);
/// A server that hasn't said anything new this long after the change (or
/// after it was last busy) has nothing new to say.
pub const SILENT: Duration = Duration::from_secs(4);

/// What came of waiting on a change's diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    Waiting,
    /// The servers have had their say: what they report now counts.
    Reported,
    /// Nothing checks these files.
    Unchecked,
    /// Still checking when the wait ran out: which server.
    StillChecking(String),
}

/// One change's wait. Each [`poll`](Self::poll) is handed the changed
/// files' state in the order they were given to [`new`](Self::new).
#[derive(Debug, Clone)]
pub struct DiagnosticsWait {
    /// Each file's diagnostics version when the change was made.
    versions: Vec<u64>,
    started: Instant,
    /// When a server was last seen busy (checking, indexing).
    last_busy: Option<Instant>,
}

impl DiagnosticsWait {
    /// Starts waiting at `started`, on files whose state is `documents`.
    pub fn new(started: Instant, documents: &[DocDiagnostics]) -> Self {
        Self {
            versions: documents.iter().map(|d| d.version).collect(),
            started,
            last_busy: None,
        }
    }

    /// Whether the servers have had their say, as of `now`.
    pub fn poll(&mut self, now: Instant, documents: &[DocDiagnostics]) -> Settled {
        debug_assert_eq!(documents.len(), self.versions.len());
        let in_grace = now - self.started < SETTLED;
        // A server set up for a file's language may yet open it: it gets
        // a moment's grace.
        let opening = documents.iter().any(|d| d.server.is_none() && d.may_open);
        let checked: Vec<_> = documents
            .iter()
            .zip(&self.versions)
            .filter_map(|(d, &version)| d.server.as_ref().map(|s| (d, version, s)))
            .collect();
        if checked.is_empty() {
            return if opening && in_grace {
                Settled::Waiting
            } else {
                Settled::Unchecked
            };
        }
        let busy = checked.iter().find(|(_, _, (_, busy))| busy.is_some());
        if busy.is_some() {
            self.last_busy = Some(now);
        }
        let quiet = |at: Option<Instant>| at.is_none_or(|at| now - at >= SETTLED);
        let reported = checked.iter().all(|&(d, version, _)| d.version > version);
        let last_report = checked.iter().filter_map(|(d, _, _)| d.set_at).max();
        if busy.is_none() && quiet(self.last_busy) && quiet(last_report) && !(opening && in_grace) {
            if reported {
                return Settled::Reported;
            }
            // Nothing new for a while since the change (or since it was
            // last busy): nothing new to say.
            if now - self.last_busy.unwrap_or(self.started) >= SILENT {
                return Settled::Reported;
            }
        }
        if now - self.started >= DIAGNOSTICS_WAIT {
            let (_, _, (server, _)) = busy.unwrap_or(&checked[0]);
            return Settled::StillChecking(server.clone());
        }
        Settled::Waiting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A file's state: `version` reports so far, the last at `set_at`.
    fn file(version: u64, set_at: Option<Instant>) -> DocDiagnostics {
        DocDiagnostics {
            version,
            set_at,
            server: None,
            may_open: false,
        }
    }

    fn open_in(mut d: DocDiagnostics, server: &str, busy: Option<&str>) -> DocDiagnostics {
        d.server = Some((server.into(), busy.map(Into::into)));
        d.may_open = true;
        d
    }

    fn opening(mut d: DocDiagnostics) -> DocDiagnostics {
        d.may_open = true;
        d
    }

    #[test]
    fn a_file_nothing_checks_is_unchecked_at_once() {
        let t0 = Instant::now();
        let docs = [file(0, None)];
        let mut wait = DiagnosticsWait::new(t0, &docs);
        assert_eq!(wait.poll(t0, &docs), Settled::Unchecked);
    }

    #[test]
    fn a_server_gets_a_moment_to_open_the_file() {
        let t0 = Instant::now();
        let docs = [opening(file(0, None))];
        let mut wait = DiagnosticsWait::new(t0, &docs);
        assert_eq!(wait.poll(t0 + ms(500), &docs), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(1000), &docs), Settled::Unchecked);
    }

    #[test]
    fn a_report_counts_once_it_has_been_quiet_a_moment() {
        let t0 = Instant::now();
        let before = [open_in(file(1, Some(t0 - ms(5000))), "ra", None)];
        let mut wait = DiagnosticsWait::new(t0, &before);
        assert_eq!(wait.poll(t0 + ms(100), &before), Settled::Waiting);
        let after = [open_in(file(2, Some(t0 + ms(300))), "ra", None)];
        assert_eq!(wait.poll(t0 + ms(800), &after), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(1300), &after), Settled::Reported);
    }

    #[test]
    fn the_old_checks_report_then_the_new_checks_counts() {
        // rust-analyzer: moves the old check's results at once, starts
        // cargo check, then reports the new check's.
        let t0 = Instant::now();
        let start = [open_in(file(1, Some(t0 - ms(9000))), "ra", None)];
        let mut wait = DiagnosticsWait::new(t0, &start);
        let old = [open_in(file(2, Some(t0 + ms(50))), "ra", None)];
        assert_eq!(wait.poll(t0 + ms(400), &old), Settled::Waiting);
        let checking = [open_in(
            file(2, Some(t0 + ms(50))),
            "ra",
            Some("cargo check"),
        )];
        assert_eq!(wait.poll(t0 + ms(900), &checking), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(2500), &checking), Settled::Waiting);
        // Done checking, the new report just in: not yet.
        let new = [open_in(file(3, Some(t0 + ms(2600))), "ra", None)];
        assert_eq!(wait.poll(t0 + ms(2700), &new), Settled::Waiting);
        // Quiet since it was last seen busy (2.5s), not yet since the report.
        assert_eq!(wait.poll(t0 + ms(3500), &new), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(3600), &new), Settled::Reported);
    }

    #[test]
    fn a_busy_server_keeps_it_waiting() {
        let t0 = Instant::now();
        let busy = [open_in(file(2, Some(t0 - ms(100))), "ra", Some("indexing"))];
        let mut wait = DiagnosticsWait::new(t0, &[open_in(file(1, None), "ra", None)]);
        for at in [0, 2000, 5000, 10_000, 29_000] {
            assert_eq!(wait.poll(t0 + ms(at), &busy), Settled::Waiting, "{at}ms");
        }
    }

    #[test]
    fn a_server_that_says_nothing_new_has_nothing_to_say() {
        let t0 = Instant::now();
        let docs = [open_in(file(1, Some(t0 - ms(5000))), "ra", None)];
        let mut wait = DiagnosticsWait::new(t0, &docs);
        assert_eq!(wait.poll(t0 + ms(3900), &docs), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(4000), &docs), Settled::Reported);
    }

    #[test]
    fn silence_counts_from_when_it_was_last_busy() {
        let t0 = Instant::now();
        let docs = [open_in(file(1, None), "ra", None)];
        let mut wait = DiagnosticsWait::new(t0, &docs);
        let busy = [open_in(file(1, None), "ra", Some("indexing"))];
        assert_eq!(wait.poll(t0 + ms(3000), &busy), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(6900), &docs), Settled::Waiting);
        assert_eq!(wait.poll(t0 + ms(7000), &docs), Settled::Reported);
    }

    #[test]
    fn the_wait_runs_out_naming_the_server_still_checking() {
        let t0 = Instant::now();
        let start = [
            open_in(file(0, None), "ra", None),
            open_in(file(0, None), "pyright", None),
        ];
        let mut wait = DiagnosticsWait::new(t0, &start);
        let busy = [
            open_in(file(1, Some(t0)), "ra", None),
            open_in(file(0, None), "pyright", Some("analyzing")),
        ];
        assert_eq!(wait.poll(t0 + ms(29_999), &busy), Settled::Waiting);
        assert_eq!(
            wait.poll(t0 + ms(30_000), &busy),
            Settled::StillChecking("pyright".into())
        );
    }

    #[test]
    fn a_wait_run_out_with_none_busy_names_the_first_server() {
        // Never quiet: reports keep coming.
        let t0 = Instant::now();
        let mut wait = DiagnosticsWait::new(t0, &[open_in(file(0, None), "ra", None)]);
        let chatty = |at| [open_in(file(at, Some(t0 + ms(at))), "ra", None)];
        assert_eq!(
            wait.poll(t0 + ms(29_000), &chatty(29_000)),
            Settled::Waiting
        );
        assert_eq!(
            wait.poll(t0 + ms(30_000), &chatty(30_000)),
            Settled::StillChecking("ra".into())
        );
    }

    #[test]
    fn a_file_still_being_opened_holds_the_answer_a_moment() {
        // a.rs settles at once; b.py's server hasn't opened it yet.
        let t0 = Instant::now();
        let start = [
            open_in(file(1, Some(t0 - ms(5000))), "ra", None),
            opening(file(0, None)),
        ];
        let mut wait = DiagnosticsWait::new(t0, &start);
        let a_reported = [
            open_in(file(2, Some(t0 + ms(100))), "ra", None),
            opening(file(0, None)),
        ];
        // Reported and quiet for a.rs at 0.9s would do, but b.py may open.
        let a_quiet_early = [
            open_in(file(2, Some(t0 - ms(2000))), "ra", None),
            opening(file(0, None)),
        ];
        assert_eq!(wait.poll(t0 + ms(900), &a_quiet_early), Settled::Waiting);
        // Opened in time: now it waits for b.py's report too.
        let b_open = [
            open_in(file(2, Some(t0 - ms(2000))), "ra", None),
            open_in(file(0, None), "pyright", None),
        ];
        assert_eq!(wait.poll(t0 + ms(1500), &b_open), Settled::Waiting);
        let b_reported = [
            open_in(file(2, Some(t0 - ms(2000))), "ra", None),
            open_in(file(1, Some(t0 + ms(1600))), "pyright", None),
        ];
        assert_eq!(wait.poll(t0 + ms(2600), &b_reported), Settled::Reported);
        // Never opened: after the grace it no longer holds the answer.
        let mut wait = DiagnosticsWait::new(t0, &start);
        assert_eq!(wait.poll(t0 + ms(1000), &a_reported), Settled::Waiting); // report too fresh
        assert_eq!(wait.poll(t0 + ms(1100), &a_reported), Settled::Reported);
    }
}
