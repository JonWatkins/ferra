//! ferra's log: `$XDG_STATE_HOME/ferra/ferra.log` (else
//! `~/.local/state/ferra/ferra.log`), opened with `:log`.
//!
//! A release build logs only panics: the message, where, and a backtrace.
//! A debug build ([`DEBUG`]) also logs, to chase a bug down: each start,
//! the errors shown on the message line, the keys, clicks and commands
//! before a panic, and the editor's state when the event loop caught one.
//! Even then, keys typed into the terminal panel are never noted (a
//! password at a sudo prompt).
//!
//! Nothing is written until [`init`] (or [`init_at`]) is called, so tests
//! and library users don't write to the user's log.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// The log grows to this before it's moved to `ferra.log.old`.
const MAX_SIZE: u64 = 1024 * 1024;

/// How many keys and commands are kept for a panic's entry.
const RECENT: usize = 60;

/// Whether this build logs more than panics.
pub const DEBUG: bool = cfg!(debug_assertions);

static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);
static RECENT_INPUT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while it was held (in a panic hook, say) leaves it usable.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// `$XDG_STATE_HOME/ferra/ferra.log`, falling back to
/// `~/.local/state/ferra/ferra.log`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("ferra").join("ferra.log"))
}

/// Starts logging to the default path. Returns where.
pub fn init() -> Option<PathBuf> {
    let path = default_path()?;
    init_at(&path);
    Some(path)
}

/// Starts logging to `path` (made if needed; a big log is moved aside).
pub fn init_at(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_SIZE) {
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
    *lock(&PATH) = Some(path.to_path_buf());
    ENABLED.store(true, Ordering::Relaxed);
}

/// Where the log is written, once it's started.
pub fn path() -> Option<PathBuf> {
    lock(&PATH).clone()
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Appends an entry: a time, a level and `text` (indented after its first
/// line). Failing to write is silently ignored: the log mustn't take the
/// editor down.
pub fn write(level: &str, text: &str) {
    if !enabled() {
        return;
    }
    let Some(path) = path() else { return };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let mut entry = format!("{} {level:<5} ", timestamp(SystemTime::now()));
    for (i, line) in text.trim_end().lines().enumerate() {
        if i > 0 {
            entry.push_str("\n    ");
        }
        entry.push_str(line);
    }
    entry.push('\n');
    let _ = file.write_all(entry.as_bytes());
}

pub fn error(text: &str) {
    write("ERROR", text);
}

/// An entry only debug builds write.
pub fn debug(level: &str, text: &str) {
    if DEBUG {
        write(level, text);
    }
}

/// Remembers a key pressed, click or command run, for the next panic's
/// entry (debug builds only).
pub fn note_input(what: impl FnOnce() -> String) {
    if !DEBUG || !enabled() {
        return;
    }
    let mut recent = lock(&RECENT_INPUT);
    if recent.len() == RECENT {
        recent.pop_front();
    }
    recent.push_back(what());
}

/// The keys and commands noted lately, oldest first, on one line.
pub fn recent_input() -> String {
    lock(&RECENT_INPUT)
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A panic's entry: its message, where, the thread, the input that led to
/// it and a backtrace.
pub fn panic_entry(info: &std::panic::PanicHookInfo<'_>) -> String {
    let message = panic_message(info.payload());
    let place = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "an unknown place".into());
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("unnamed");
    let backtrace = std::backtrace::Backtrace::force_capture();
    let recent = if DEBUG {
        format!("recent input: {}\n", recent_input())
    } else {
        String::new()
    };
    format!(
        "panic in thread {thread} at {place}: {message}\nversion: {}\n{recent}backtrace:\n{backtrace}",
        env!("CARGO_PKG_VERSION"),
    )
}

/// A panic's message, from its payload.
pub fn panic_message(payload: &dyn std::any::Any) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "(no message)".into()
    }
}

/// `2026-11-14 09:30:05` (UTC) for `time`.
pub fn timestamp(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00");
        // 2026-11-14 09:30:05 UTC.
        let t = UNIX_EPOCH + Duration::from_secs(1_794_648_605);
        assert_eq!(timestamp(t), "2026-11-14 09:30:05");
        // A leap day.
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(timestamp(t), "2000-02-29 00:00:00");
    }

    #[test]
    fn panic_messages_from_either_payload() {
        assert_eq!(panic_message(&"boom"), "boom");
        assert_eq!(panic_message(&String::from("bang")), "bang");
        assert_eq!(panic_message(&3), "(no message)");
    }
}
