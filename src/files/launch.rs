//! Handing a URL or file to the system's opener (`gx`): `open` on macOS,
//! the URL protocol handler on Windows, `xdg-open` elsewhere. It's started
//! and left to run; only failing to start it is an error.

use std::process::{Command, Stdio};

/// Opens `target` (a URL or an absolute path) the way the desktop would.
pub fn open_externally(target: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        // Not `cmd /c start`, which would read `&` in a URL as a command
        // separator.
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        Command::new("xdg-open")
    };
    command
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open \"{target}\": {e}"))
}
