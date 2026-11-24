//! Ending everything a shell started, not just the shell.
//!
//! A shell in the panel is started in a session of its own (portable-pty
//! calls `setsid`), so its session ID is its pid, and every program it
//! starts inherits that ID, including ones started with `nohup` or that
//! ignore the hangup, which outlive the shell. Hanging up (what closing a
//! terminal app's window does) leaves those running; `end_session` finds
//! every process still in the session (`getsid`), asks them to stop
//! (SIGTERM), and kills what's left after a moment (SIGKILL).
//!
//! The kernel doesn't give out a pid again while it's a live session's ID,
//! so the session's ID can't come to name someone else's processes while
//! any of its own are left. A program that starts its own session (a
//! daemon, `setsid`) has left it, and isn't touched.

use std::time::{Duration, Instant};

/// How long programs get to stop when asked, before they're killed.
const GRACE: Duration = Duration::from_millis(300);

/// Ends every process left in session `sid` (the shell's pid).
#[cfg(unix)]
pub fn end_session(sid: u32) {
    let Ok(sid) = libc::pid_t::try_from(sid) else {
        return;
    };
    let left = members(sid);
    if left.is_empty() {
        return;
    }
    for &pid in &left {
        // SAFETY: a plain signal to a pid in the shell's session.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let start = Instant::now();
    while start.elapsed() < GRACE {
        if members(sid).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for pid in members(sid) {
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

#[cfg(not(unix))]
pub fn end_session(_sid: u32) {}

/// The processes in session `sid` (never ferra itself).
#[cfg(unix)]
fn members(sid: libc::pid_t) -> Vec<libc::pid_t> {
    let me = std::process::id() as libc::pid_t;
    all_pids()
        .into_iter()
        .filter(|&pid| pid > 1 && pid != me)
        // SAFETY: `getsid` only reads; a pid that's gone gives -1.
        .filter(|&pid| unsafe { libc::getsid(pid) } == sid)
        .collect()
}

/// Every process's pid.
#[cfg(target_os = "macos")]
fn all_pids() -> Vec<libc::pid_t> {
    // SAFETY: a null buffer asks how many there are.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // Room for some started meanwhile.
    let mut pids: Vec<libc::pid_t> = vec![0; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: `pids` has room for `bytes`; it returns how many it wrote.
    let found = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(found.max(0) as usize);
    pids
}

/// Every process's pid.
#[cfg(all(unix, not(target_os = "macos")))]
fn all_pids() -> Vec<libc::pid_t> {
    std::fs::read_dir("/proc")
        .map(|dir| {
            dir.flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}
