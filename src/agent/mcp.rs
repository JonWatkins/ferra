//! ferra's tools for the coding agent, served over MCP (the Model Context
//! Protocol). The agent starts MCP servers itself, as programs it talks to
//! over their stdin and stdout, so ferra gives it one to start:
//! `ferra --mcp-bridge <socket>`, which only passes messages between its
//! stdin and stdout and a socket the running editor listens on
//! ([`McpServer`]). The socket is the user's alone (in their temporary
//! folder, readable and writable only by them) and goes when the editor
//! does.
//!
//! Messages are JSON-RPC, one per line, as with ACP; what they ask is
//! answered in `tools.rs`, with the editor at hand.

use std::collections::HashMap;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use super::client::{read_message, write_message};

/// What came in on a connection (the agent can make more than one).
#[derive(Debug)]
pub enum McpIncoming {
    Message(u64, Value),
    Closed(u64),
}

#[cfg(unix)]
pub struct McpServer {
    path: PathBuf,
    receiver: Receiver<McpIncoming>,
    writers: Arc<Mutex<HashMap<u64, std::os::unix::net::UnixStream>>>,
    stop: Arc<AtomicBool>,
}

#[cfg(unix)]
impl McpServer {
    /// Listens on a new socket in the temporary folder.
    pub fn start() -> io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("ferra-{}-{n}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let (sender, receiver) = channel();
        let writers: Arc<Mutex<HashMap<u64, std::os::unix::net::UnixStream>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (Arc::clone(&writers), Arc::clone(&stop));
        thread::spawn(move || {
            for (conn, stream) in (0u64..).zip(listener.incoming()) {
                if stopping.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let Ok(writer) = stream.try_clone() else {
                    continue;
                };
                shared.lock().expect("not poisoned").insert(conn, writer);
                let sender = sender.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream);
                    loop {
                        match read_message(&mut reader) {
                            Ok(Some(message)) => {
                                if sender.send(McpIncoming::Message(conn, message)).is_err() {
                                    return;
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::InvalidData => continue,
                            Ok(None) | Err(_) => break,
                        }
                    }
                    let _ = sender.send(McpIncoming::Closed(conn));
                });
            }
        });
        Ok(Self {
            path,
            receiver,
            writers,
            stop,
        })
    }

    /// The socket, for the bridge to connect to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn try_recv(&self) -> Option<McpIncoming> {
        self.receiver.try_recv().ok()
    }

    /// Sends `message` on connection `conn` (gone: dropped).
    pub fn send(&self, conn: u64, message: &Value) {
        let mut writers = self.writers.lock().expect("not poisoned");
        if let Some(writer) = writers.get_mut(&conn)
            && write_message(writer, message).is_err()
        {
            writers.remove(&conn);
        }
    }

    pub fn forget(&self, conn: u64) {
        self.writers.lock().expect("not poisoned").remove(&conn);
    }
}

#[cfg(unix)]
impl Drop for McpServer {
    fn drop(&mut self) {
        // The listening thread wakes to a connection, sees it should stop.
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::os::unix::net::UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `ferra --mcp-bridge <socket>`: passes stdin to the socket and what comes
/// back to stdout, until either ends.
#[cfg(unix)]
pub fn bridge(socket: &Path) -> io::Result<()> {
    use std::io::Write;
    let stream = std::os::unix::net::UnixStream::connect(socket)?;
    let mut from_editor = stream.try_clone()?;
    thread::spawn(move || {
        let mut out = io::stdout().lock();
        let _ = io::copy(&mut from_editor, &mut out);
        let _ = out.flush();
        // The editor's gone: so is the bridge.
        std::process::exit(0);
    });
    let mut to_editor = &stream;
    io::copy(&mut io::stdin().lock(), &mut to_editor)?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

#[cfg(not(unix))]
pub fn bridge(_: &Path) -> io::Result<()> {
    Err(io::Error::other("the agent's tools need Unix sockets"))
}
