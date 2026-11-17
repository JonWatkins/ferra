//! Messages to another program (a language server, the coding agent, the
//! agent's MCP connection), written on a thread of their own. Writing on
//! the editor's thread could block it: a pipe or socket only holds so much,
//! and a program that isn't reading for a while (the agent between turns, a
//! server busy loading a project) would freeze the editor until it read
//! again. Here, sending only queues the message.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;

use serde_json::Value;

pub struct Outbox {
    sender: Sender<Value>,
    failed: Arc<AtomicBool>,
}

impl Outbox {
    /// Writes each message sent to `out` with `write` (the protocol's
    /// framing), in order, on a thread named `name`.
    pub fn spawn<W, F>(mut out: W, name: &str, mut write: F) -> Self
    where
        W: Write + Send + 'static,
        F: FnMut(&mut W, &Value) -> io::Result<()> + Send + 'static,
    {
        let (sender, receiver) = channel::<Value>();
        let failed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&failed);
        let spawned = thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                for message in receiver {
                    if write(&mut out, &message).is_err() {
                        flag.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            });
        if spawned.is_err() {
            failed.store(true, Ordering::Relaxed);
        }
        Self { sender, failed }
    }

    /// Queues `message`. False once writing has failed (the program
    /// stopped reading, or is gone).
    pub fn send(&self, message: Value) -> bool {
        !self.failed() && self.sender.send(message).is_ok()
    }

    /// Whether a write failed.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_reader_that_doesnt_read_doesnt_hold_up_the_sender() {
        // A pipe nobody reads: its buffer fills, and a write blocks.
        let (_reader, writer) = std::io::pipe().unwrap();
        let outbox = Outbox::spawn(writer, "test-outbox", |out, message| {
            serde_json::to_writer(&mut *out, message)?;
            out.flush()
        });
        let big = Value::String("x".repeat(64 * 1024));
        let start = Instant::now();
        for _ in 0..8 {
            assert!(outbox.send(big.clone()));
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "queued, not written in the sender's time"
        );
    }

    #[test]
    fn a_gone_reader_is_noticed() {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let outbox = Outbox::spawn(writer, "test-outbox", |out, message| {
            serde_json::to_writer(&mut *out, message)?;
            out.flush()
        });
        outbox.send(Value::Null);
        let start = Instant::now();
        while !outbox.failed() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!outbox.send(Value::Null), "failed: says so");
    }
}
