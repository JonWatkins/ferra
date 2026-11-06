//! The system clipboard, behind a trait so tests (and headless setups) can
//! use an in-memory one instead.

use std::cell::RefCell;
use std::rc::Rc;

pub trait Clipboard {
    fn get(&mut self) -> Result<String, String>;
    fn set(&mut self, text: &str) -> Result<(), String>;
}

/// The operating system's clipboard (macOS, Windows, X11/Wayland).
pub struct SystemClipboard {
    inner: arboard::Clipboard,
}

impl SystemClipboard {
    /// Connects to the system clipboard, which can fail (e.g. no display on
    /// a Linux server).
    pub fn new() -> Result<Self, String> {
        arboard::Clipboard::new()
            .map(|inner| Self { inner })
            .map_err(|e| format!("System clipboard unavailable: {e}"))
    }
}

impl Clipboard for SystemClipboard {
    fn get(&mut self) -> Result<String, String> {
        self.inner.get_text().map_err(|e| format!("Clipboard: {e}"))
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        self.inner
            .set_text(text.to_string())
            .map_err(|e| format!("Clipboard: {e}"))
    }
}

/// The system clipboard, connected on first use, so starting the editor (or
/// running tests that never touch `"+`) doesn't talk to the OS at all.
#[derive(Default)]
pub struct LazySystemClipboard {
    connection: Option<Result<SystemClipboard, String>>,
}

impl LazySystemClipboard {
    fn connect(&mut self) -> Result<&mut SystemClipboard, String> {
        self.connection
            .get_or_insert_with(SystemClipboard::new)
            .as_mut()
            .map_err(|e| e.clone())
    }
}

impl Clipboard for LazySystemClipboard {
    fn get(&mut self) -> Result<String, String> {
        self.connect()?.get()
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        self.connect()?.set(text)
    }
}

/// A clipboard kept in memory. Clones share the same contents, so a test can
/// keep one to check (or fill) what the editor sees.
#[derive(Debug, Default, Clone)]
pub struct MemoryClipboard {
    contents: Rc<RefCell<String>>,
}

impl MemoryClipboard {
    pub fn contents(&self) -> String {
        self.contents.borrow().clone()
    }

    pub fn set_contents(&self, text: &str) {
        *self.contents.borrow_mut() = text.to_string();
    }
}

impl Clipboard for MemoryClipboard {
    fn get(&mut self) -> Result<String, String> {
        Ok(self.contents())
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        self.set_contents(text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_clipboard_clones_share_contents() {
        let shared = MemoryClipboard::default();
        let mut clipboard = shared.clone();
        clipboard.set("copied").unwrap();
        assert_eq!(shared.contents(), "copied");
        shared.set_contents("from outside");
        assert_eq!(clipboard.get().unwrap(), "from outside");
    }
}
