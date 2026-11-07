//! The clipboard behind `"+` and `"*`, behind a trait so tests (and
//! headless setups) can use an in-memory one instead.
//!
//! It's the operating system's clipboard, or, over SSH or where there is
//! none, the terminal's: set with an OSC 52 escape sequence, which most
//! terminals (and tmux with `set-clipboard on`) pass to the clipboard of
//! the machine they run on.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

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

/// Which clipboard `"+` and `"*` use (`clipboard-provider` in `[editor]`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardProvider {
    /// The system clipboard, or the terminal's over SSH or when there's no
    /// system clipboard (a server with no display).
    #[default]
    Auto,
    /// Only the system clipboard.
    System,
    /// Only the terminal's (OSC 52).
    Terminal,
}

/// The clipboard for `provider`, deciding whether this is an SSH session
/// from the environment.
pub fn for_provider(provider: ClipboardProvider) -> Box<dyn Clipboard> {
    match provider {
        ClipboardProvider::System => Box::new(LazySystemClipboard::default()),
        ClipboardProvider::Terminal => Box::new(TerminalClipboard::stdout()),
        ClipboardProvider::Auto => {
            let over_ssh = ["SSH_TTY", "SSH_CONNECTION", "SSH_CLIENT"]
                .iter()
                .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()));
            Box::new(AutoClipboard::new(
                Box::new(LazySystemClipboard::default()),
                TerminalClipboard::stdout(),
                over_ssh,
            ))
        }
    }
}

/// The terminal's clipboard: copying sends `ESC ] 52 ; c ; <base64> BEL`,
/// which the terminal puts on the clipboard of the machine it runs on.
/// Terminals rarely let programs read the clipboard back, so reading gives
/// the text last copied here; pasting from elsewhere is the terminal's own
/// paste (Cmd/Ctrl+Shift+V), which ferra takes as a bracketed paste.
pub struct TerminalClipboard {
    out: Box<dyn Write>,
    last: String,
}

impl TerminalClipboard {
    pub fn new(out: Box<dyn Write>) -> Self {
        Self {
            out,
            last: String::new(),
        }
    }

    /// Writes to standard output, the terminal ferra draws on.
    pub fn stdout() -> Self {
        Self::new(Box::new(std::io::stdout()))
    }
}

impl Clipboard for TerminalClipboard {
    fn get(&mut self) -> Result<String, String> {
        Ok(self.last.clone())
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        self.out
            .write_all(osc52(text).as_bytes())
            .and_then(|()| self.out.flush())
            .map_err(|e| format!("Clipboard: {e}"))?;
        self.last = text.to_string();
        Ok(())
    }
}

/// The escape sequence that sets the terminal's clipboard to `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Standard base64, with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The system clipboard, falling back to the terminal's when it can't be
/// used; over SSH, the terminal's, since the system clipboard there is the
/// remote machine's.
pub struct AutoClipboard {
    system: Box<dyn Clipboard>,
    terminal: TerminalClipboard,
    over_ssh: bool,
}

impl AutoClipboard {
    pub fn new(system: Box<dyn Clipboard>, terminal: TerminalClipboard, over_ssh: bool) -> Self {
        Self {
            system,
            terminal,
            over_ssh,
        }
    }
}

impl Clipboard for AutoClipboard {
    fn get(&mut self) -> Result<String, String> {
        if self.over_ssh {
            return self.terminal.get();
        }
        self.system.get().or_else(|_| self.terminal.get())
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        if self.over_ssh {
            return self.terminal.set(text);
        }
        self.system.set(text).or_else(|_| self.terminal.set(text))
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

    /// What a `TerminalClipboard` wrote, readable from the test.
    #[derive(Clone, Default)]
    struct Written(Rc<RefCell<Vec<u8>>>);

    impl Write for Written {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Written {
        fn text(&self) -> String {
            String::from_utf8(self.0.borrow().clone()).unwrap()
        }
    }

    struct Unavailable;

    impl Clipboard for Unavailable {
        fn get(&mut self) -> Result<String, String> {
            Err("no display".into())
        }
        fn set(&mut self, _: &str) -> Result<(), String> {
            Err("no display".into())
        }
    }

    #[test]
    fn base64_matches_the_rfc_examples() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (text, encoded) in cases {
            assert_eq!(base64(text.as_bytes()), encoded, "{text:?}");
        }
        assert_eq!(base64("é\n".as_bytes()), "w6kK");
    }

    #[test]
    fn the_terminal_clipboard_sends_osc_52() {
        let written = Written::default();
        let mut clipboard = TerminalClipboard::new(Box::new(written.clone()));
        assert_eq!(clipboard.get().unwrap(), "", "nothing copied yet");
        clipboard.set("foo").unwrap();
        assert_eq!(written.text(), "\x1b]52;c;Zm9v\x07");
        assert_eq!(clipboard.get().unwrap(), "foo", "the last text copied");
    }

    #[test]
    fn auto_uses_the_terminal_over_ssh_or_without_a_system_clipboard() {
        let written = Written::default();
        let system = MemoryClipboard::default();
        let terminal = TerminalClipboard::new(Box::new(written.clone()));
        let mut local = AutoClipboard::new(Box::new(system.clone()), terminal, false);
        local.set("a").unwrap();
        assert_eq!(system.contents(), "a");
        assert_eq!(written.text(), "", "the system clipboard worked");

        let terminal = TerminalClipboard::new(Box::new(written.clone()));
        let mut ssh = AutoClipboard::new(Box::new(system.clone()), terminal, true);
        ssh.set("b").unwrap();
        assert_eq!(system.contents(), "a", "not the remote machine's");
        assert_eq!(written.text(), osc52("b"));
        assert_eq!(ssh.get().unwrap(), "b");

        let written = Written::default();
        let terminal = TerminalClipboard::new(Box::new(written.clone()));
        let mut headless = AutoClipboard::new(Box::new(Unavailable), terminal, false);
        headless.set("c").unwrap();
        assert_eq!(written.text(), osc52("c"));
        assert_eq!(headless.get().unwrap(), "c");
    }

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
