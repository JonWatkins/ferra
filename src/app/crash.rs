//! What happens when ferra panics.
//!
//! Every panic is written to the log (`crate::log`) with a backtrace.
//! Then:
//!
//! - In the event loop (drawing, a key, a background job's results) it's
//!   caught: the editor reports it on the message line and carries on, so
//!   a bug doesn't lose unsaved work. If drawing keeps failing, the loop
//!   gives up, saving unsaved buffers to the recovery folder first.
//! - On another thread (a language server's reader, a background job) only
//!   that thread stops; the editor is told on its next turn.
//! - Anywhere else, the terminal is put back (mouse reporting and the
//!   keyboard protocol off too, which ratatui doesn't do) before the
//!   message is printed.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;

use crossterm::event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture};
use crossterm::execute;

use crate::log;

thread_local! {
    /// Set while `catching` runs, so the hook leaves the terminal alone.
    static CATCHING: Cell<bool> = const { Cell::new(false) };
}

static UI_THREAD: Mutex<Option<ThreadId>> = Mutex::new(None);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The last panic on another thread, for the editor to report.
static THREAD_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Installs the panic hook, on top of whatever was there (ratatui's,
/// which restores the terminal, and color-eyre's, which prints the
/// report). The calling thread is the UI's.
pub fn install() {
    *UI_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        log::error(&log::panic_entry(info));
        let ui = *UI_THREAD.lock().unwrap_or_else(|e| e.into_inner());
        if ui != Some(std::thread::current().id()) {
            let name = std::thread::current()
                .name()
                .unwrap_or("a background thread")
                .to_string();
            let message = log::panic_message(info.payload());
            *THREAD_PANIC.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("{name} crashed: {message}"));
            return;
        }
        if CATCHING.with(Cell::get) {
            return;
        }
        restore_terminal();
        previous(info);
        if let Some(path) = log::path() {
            eprintln!("\nThe details are in {}", path.display());
        }
    }));
}

/// Turns off what ferra turned on in the terminal (ratatui's own restore,
/// raw mode and the alternate screen, follows in its hook).
fn restore_terminal() {
    let mut out = std::io::stdout();
    let _ = execute!(
        out,
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange
    );
    let _ = super::terminal::disable_keyboard_enhancement();
    let _ = super::terminal::set_pointer_shape("default");
    let _ = super::terminal::restore_cursor_shape();
}

/// Runs `f`, catching a panic in it: `Err` with the panic's message (the
/// hook has logged it). The terminal is left as it is.
pub fn catching<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let was = CATCHING.with(|c| c.replace(true));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    CATCHING.with(|c| c.set(was));
    result.map_err(|payload| log::panic_message(&*payload))
}

/// A panic on another thread since the last call, if there was one.
pub fn take_thread_panic() -> Option<String> {
    THREAD_PANIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}
