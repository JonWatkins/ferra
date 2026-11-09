//! A shell running in a pseudo-terminal, for the panel at the bottom
//! (`<Space>t`).
//!
//! `portable-pty` starts the shell; a thread reads what it prints and
//! sends it through a channel, and `pump` (called by the event loop) feeds
//! it to a `vt100` parser, whose screen the panel draws. Keys typed into
//! the panel are turned back into the bytes a terminal sends
//! (`key_bytes`). The parser answers the few queries programs wait on
//! (the cursor position, the device attributes) and takes the title.

use std::cell::Cell;
use std::io::{self, Read, Write};
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::input::{KeyCode, KeyPress};

/// Lines kept above the screen to scroll back through.
const SCROLLBACK: usize = 5000;

/// What the reader thread sends.
enum Output {
    Bytes(Vec<u8>),
    Closed,
}

/// Replies to queries, and the title, collected as the parser runs.
#[derive(Default)]
struct Callbacks {
    replies: Vec<u8>,
    title: Option<String>,
}

impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (i1, c, params) {
            // Where's the cursor? (`ESC [ 6 n`)
            (None, 'n', [[6]]) => {
                let (row, col) = screen.cursor_position();
                self.replies
                    .extend(format!("\x1b[{};{}R", row + 1, col + 1).bytes());
            }
            // Is the terminal working? (`ESC [ 5 n`)
            (None, 'n', [[5]]) => self.replies.extend(b"\x1b[0n"),
            // What are you? (primary and secondary device attributes)
            (None, 'c', _) => self.replies.extend(b"\x1b[?62;22c"),
            (Some(b'>'), 'c', _) => self.replies.extend(b"\x1b[>1;10;0c"),
            _ => {}
        }
    }
}

pub struct Terminal {
    parser: vt100::Parser<Callbacks>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Output>,
    /// The shell's name, until it sets a title.
    name: String,
    /// Set once the shell has gone: why, for the panel.
    exited: Option<String>,
    /// The size the panel was last drawn at, to resize to.
    wanted: Cell<Option<(u16, u16)>>,
}

fn nonzero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).expect("at least 1")
}

/// The shell to run: `shell` if set, else `$SHELL`, else `/bin/sh`.
pub fn shell_program(shell: &str) -> String {
    if !shell.trim().is_empty() {
        return shell.trim().to_string();
    }
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into())
}

impl Terminal {
    /// Starts `program` (with `args`) in `cwd`, in a terminal `rows` by
    /// `cols`.
    pub fn spawn(
        program: &str,
        args: &[&str],
        cwd: &Path,
        rows: u16,
        cols: u16,
    ) -> io::Result<Self> {
        let size = PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let mut command = CommandBuilder::new(program);
        command.args(args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "ferra");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| io::Error::other(format!("{program}: {e}")))?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| io::Error::other(e.to_string()))?;

        let (sender, output) = channel();
        thread::Builder::new()
            .name("terminal-output".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sender.send(Output::Bytes(buf[..n].to_vec())).is_err() {
                                return;
                            }
                        }
                    }
                }
                let _ = sender.send(Output::Closed);
            })?;

        let name = Path::new(program)
            .file_name()
            .map_or_else(|| program.to_string(), |n| n.to_string_lossy().into_owned());
        Ok(Self {
            parser: vt100::Parser::new_with_callbacks(
                nonzero(rows),
                nonzero(cols),
                SCROLLBACK,
                Callbacks::default(),
            ),
            writer,
            master: pair.master,
            child,
            output,
            name,
            exited: None,
            wanted: Cell::new(None),
        })
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// The title the shell set, or its name.
    pub fn title(&self) -> &str {
        self.parser
            .callbacks()
            .title
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&self.name)
    }

    /// Why the shell's gone, once it has.
    pub fn exited(&self) -> Option<&str> {
        self.exited.as_deref()
    }

    pub fn is_running(&self) -> bool {
        self.exited.is_none()
    }

    /// Takes what the shell printed since last time. Returns whether
    /// anything changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.output.try_recv() {
                Ok(Output::Bytes(bytes)) => {
                    self.parser.process(&bytes);
                    changed = true;
                }
                Ok(Output::Closed) | Err(TryRecvError::Disconnected) => {
                    if self.exited.is_none() {
                        let status = self.child.wait().ok();
                        self.exited = Some(match status {
                            Some(s) if s.success() => "exited".to_string(),
                            Some(s) => format!("exited with code {}", s.exit_code()),
                            None => "exited".to_string(),
                        });
                        changed = true;
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        let replies = std::mem::take(&mut self.parser.callbacks_mut().replies);
        if !replies.is_empty() {
            self.write(&replies);
        }
        changed
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.exited.is_some() {
            return;
        }
        // A shell that's gone shows up through `pump`.
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Sends a key typed into the panel (back to the bottom of the
    /// scrollback first).
    pub fn send_key(&mut self, key: KeyPress) {
        self.parser.screen_mut().set_scrollback(0);
        let bytes = key_bytes(key, self.screen().application_cursor());
        self.write(&bytes);
    }

    /// Sends pasted text, bracketed if the program asked for that.
    pub fn paste(&mut self, text: &str) {
        self.parser.screen_mut().set_scrollback(0);
        // Line breaks as Enter sends them.
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.screen().bracketed_paste() {
            let bytes = [b"\x1b[200~", text.as_bytes(), b"\x1b[201~"].concat();
            self.write(&bytes);
        } else {
            self.write(text.as_bytes());
        }
    }

    /// Scrolls back (`rows` > 0) or forward through what's scrolled off.
    pub fn scroll(&mut self, rows: isize) {
        let screen = self.parser.screen_mut();
        let to = screen.scrollback().saturating_add_signed(rows);
        screen.set_scrollback(to);
    }

    /// Notes the size the panel's drawn at (`apply_size` resizes to it).
    pub fn note_size(&self, rows: u16, cols: u16) {
        self.wanted.set(Some((rows, cols)));
    }

    /// Resizes the terminal to the size last drawn at, if it changed.
    pub fn apply_size(&mut self) {
        let Some((rows, cols)) = self.wanted.take() else {
            return;
        };
        let (now_rows, now_cols) = self.screen().size();
        if (now_rows.get(), now_cols.get()) == (rows.max(1), cols.max(1)) {
            return;
        }
        self.parser.set_size(nonzero(rows), nonzero(cols));
        let _ = self.master.resize(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        });
    }
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("title", &self.title())
            .field("exited", &self.exited)
            .finish_non_exhaustive()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.exited.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The bytes a terminal sends for `key`. `app_cursor` is the mode where
/// arrows send `ESC O A` rather than `ESC [ A` (full-screen programs).
pub fn key_bytes(key: KeyPress, app_cursor: bool) -> Vec<u8> {
    let m = key.modifiers;
    // xterm's modifier parameter: 1 + shift + 2 alt + 4 ctrl.
    let param = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let csi = |end: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{end}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{end}").into_bytes()
        } else {
            format!("\x1b[{end}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[{n};{param}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let mut bytes = match key.code {
        KeyCode::Char(c) if m.ctrl => match control_byte(c) {
            Some(b) => vec![b],
            None => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab if m.shift => return b"\x1b[Z".to_vec(),
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Backspace if m.ctrl => vec![0x08],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return csi('A'),
        KeyCode::Down => return csi('B'),
        KeyCode::Right => return csi('C'),
        KeyCode::Left => return csi('D'),
        KeyCode::Home => return csi('H'),
        KeyCode::End => return csi('F'),
        KeyCode::Delete => return tilde(3),
        KeyCode::PageUp => return tilde(5),
        KeyCode::PageDown => return tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let end = [b'P', b'Q', b'R', b'S'][usize::from(n - 1)] as char;
            return if param > 1 {
                format!("\x1b[1;{param}{end}").into_bytes()
            } else {
                format!("\x1bO{end}").into_bytes()
            };
        }
        KeyCode::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return Vec::new(),
            };
            return tilde(code);
        }
        KeyCode::PasteStart | KeyCode::PasteEnd => return Vec::new(),
    };
    // Alt (Meta) sends ESC first.
    if m.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Ctrl plus `c` as a control character (`<C-c>` is 3).
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' | '2' | '`' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' | '/' | '-' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::input::parse_keys;

    fn bytes(notation: &str, app_cursor: bool) -> Vec<u8> {
        key_bytes(parse_keys(notation).unwrap()[0], app_cursor)
    }

    #[test]
    fn keys_become_what_a_terminal_sends() {
        assert_eq!(bytes("a", false), b"a");
        assert_eq!(bytes("é", false), "é".as_bytes());
        assert_eq!(bytes("<C-c>", false), [3]);
        assert_eq!(bytes("<C-\\>", false), [0x1c]);
        assert_eq!(bytes("<CR>", false), b"\r");
        assert_eq!(bytes("<BS>", false), [0x7f]);
        assert_eq!(bytes("<S-Tab>", false), b"\x1b[Z");
        assert_eq!(bytes("<A-b>", false), b"\x1bb");
        assert_eq!(bytes("<Up>", false), b"\x1b[A");
        assert_eq!(bytes("<Up>", true), b"\x1bOA", "full-screen programs");
        assert_eq!(bytes("<C-Right>", false), b"\x1b[1;5C");
        assert_eq!(bytes("<Del>", false), b"\x1b[3~");
        assert_eq!(bytes("<S-PageUp>", false), b"\x1b[5;2~");
        assert_eq!(bytes("<F1>", false), b"\x1bOP");
        assert_eq!(bytes("<F5>", false), b"\x1b[15~");
    }

    /// Pumps until `done` says so (or two seconds pass).
    fn wait_for(terminal: &mut Terminal, done: impl Fn(&Terminal) -> bool) {
        let start = Instant::now();
        while !done(terminal) && start.elapsed() < Duration::from_secs(2) {
            terminal.pump();
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn runs_a_program_and_shows_what_it_prints() {
        let dir = tempfile::tempdir().unwrap();
        let mut terminal = Terminal::spawn(
            "/bin/sh",
            &["-c", "printf 'hello\\033]0;my title\\007'; pwd; exit 3"],
            dir.path(),
            5,
            30,
        )
        .unwrap();
        wait_for(&mut terminal, |t| t.exited().is_some());
        let text = terminal.screen().contents();
        assert!(text.starts_with("hello"), "{text:?}");
        let dir_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(text.contains(&dir_name), "in its folder: {text:?}");
        assert_eq!(terminal.title(), "my title");
        assert_eq!(terminal.exited(), Some("exited with code 3"));
    }

    #[test]
    fn typed_keys_reach_the_program_and_queries_are_answered() {
        let dir = tempfile::tempdir().unwrap();
        // `cat` echoes lines back; the shell asks where the cursor is
        // first, and prints the answer it got.
        let script =
            "printf '\\033[6n'; IFS= read -r -d R pos; printf 'at:%s\\n' \"${pos#??}\"; cat";
        let mut terminal =
            Terminal::spawn("/bin/bash", &["-c", script], dir.path(), 5, 30).unwrap();
        wait_for(&mut terminal, |t| t.screen().contents().contains("at:"));
        assert!(
            terminal.screen().contents().contains("at:1;1"),
            "{:?}",
            terminal.screen().contents()
        );
        for key in parse_keys("hi<CR>").unwrap() {
            terminal.send_key(key);
        }
        wait_for(&mut terminal, |t| {
            t.screen().contents().matches("hi").count() >= 2
        });
        assert_eq!(
            terminal.screen().contents().matches("hi").count(),
            2,
            "typed, and echoed"
        );

        terminal.note_size(8, 40);
        terminal.apply_size();
        let (rows, cols) = terminal.screen().size();
        assert_eq!((rows.get(), cols.get()), (8, 40));
        drop(terminal);
    }
}
