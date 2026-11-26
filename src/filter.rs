//! Running a program over text: lines in on its standard input, lines back
//! from its standard output, as Vim's `:{range}!` filter. `gq` uses it for
//! `format-program` (Vim's `formatprg`).
//!
//! The command line goes to the shell (`shell`, else `$SHELL`, else
//! `/bin/sh`, with `-c`; `cmd /C` on Windows), in ferra's working
//! directory. The editor waits for it, so a program that's still going
//! after [`TIMEOUT`] is killed. Unlike Vim, a program that fails (exits
//! non-zero, or is killed) changes nothing: its error is the message, the
//! first line of what it wrote to standard error.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the editor waits for a program before killing it.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `command` through `shell` with `lines` on its standard input (each
/// ended by a newline), giving the lines it writes, or why it failed.
pub fn run(
    command: &str,
    shell: &str,
    lines: &[String],
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let mut child = shell_command(shell, command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{command}: {e}"))?;

    let mut input = String::new();
    for line in lines {
        input.push_str(line);
        input.push('\n');
    }
    // Writing, and reading each output, on threads of their own, so a
    // program that writes before it has read everything can't block on a
    // full pipe.
    let mut stdin = child.stdin.take().expect("piped");
    std::thread::spawn(move || {
        // A program that exits without reading it all closes the pipe:
        // fine, what it wrote is its answer.
        let _ = stdin.write_all(input.as_bytes());
    });
    let stdout = read_all(child.stdout.take().expect("piped"));
    let stderr = read_all(child.stderr.take().expect("piped"));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // The readers are left: something the shell started may
                // still hold the pipes open.
                return Err(format!(
                    "{command}: still running after {}s, so stopped",
                    timeout.as_secs_f32()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(format!("{command}: {e}")),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();

    if !status.success() {
        let errors = String::from_utf8_lossy(&stderr);
        let first = errors.lines().map(str::trim).find(|l| !l.is_empty());
        let code = status
            .code()
            .map_or_else(|| "was stopped".to_string(), |c| format!("exited {c}"));
        return Err(match first {
            Some(error) => format!("{command} {code}: {error}"),
            None => format!("{command} {code}"),
        });
    }
    let text = String::from_utf8(stdout).map_err(|_| format!("{command}: output isn't UTF-8"))?;
    Ok(lines_of(&text))
}

/// `text`'s lines, without their endings (`\n` or `\r\n`); a last line
/// ending in a newline is the last line, not one before an empty one.
fn lines_of(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

fn read_all(mut from: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = from.read_to_end(&mut bytes);
        bytes
    })
}

#[cfg(unix)]
fn shell_command(shell: &str, command: &str) -> Command {
    let mut c = Command::new(crate::terminal::shell_program(shell));
    c.arg("-c").arg(command);
    c
}

#[cfg(windows)]
fn shell_command(_shell: &str, command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(command);
    c
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    fn sh(command: &str, input: &[&str]) -> Result<Vec<String>, String> {
        run(command, "/bin/sh", &lines(input), TIMEOUT)
    }

    #[test]
    fn gives_the_lines_the_program_writes() {
        assert_eq!(sh("sort", &["b", "c", "a"]), Ok(lines(&["a", "b", "c"])));
        assert_eq!(
            sh("tr a-z A-Z", &["one", "", "two"]),
            Ok(lines(&["ONE", "", "TWO"]))
        );
    }

    #[test]
    fn line_endings_come_off() {
        assert_eq!(sh("printf 'a\\r\\nb\\r\\n'", &[]), Ok(lines(&["a", "b"])));
        assert_eq!(sh("printf 'no newline'", &[]), Ok(lines(&["no newline"])));
        assert_eq!(sh("printf 'a\\n\\n'", &[]), Ok(lines(&["a", ""])));
        assert_eq!(sh("true", &["gone"]), Ok(vec![]));
    }

    #[test]
    fn a_program_that_fails_says_why() {
        assert_eq!(
            sh(
                "echo out; echo '  bad input' >&2; echo more >&2; exit 3",
                &["x"]
            ),
            Err(
                "echo out; echo '  bad input' >&2; echo more >&2; exit 3 exited 3: bad input"
                    .into()
            )
        );
        assert_eq!(sh("exit 1", &[]), Err("exit 1 exited 1".into()));
        let missing = sh("no-such-program-ferra", &["x"]).unwrap_err();
        assert!(
            missing.starts_with("no-such-program-ferra exited 127: "),
            "{missing}"
        );
    }

    #[test]
    fn a_program_that_hangs_is_stopped() {
        let started = Instant::now();
        let error = run("sleep 30", "/bin/sh", &[], Duration::from_millis(200)).unwrap_err();
        assert_eq!(error, "sleep 30: still running after 0.2s, so stopped");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_lot_of_text_goes_both_ways() {
        let many: Vec<String> = (0..20_000).map(|n| format!("line {n}")).collect();
        assert_eq!(run("cat", "/bin/sh", &many, TIMEOUT), Ok(many));
    }

    #[test]
    fn output_that_isnt_utf8_is_refused() {
        assert_eq!(
            sh("printf '\\377'", &[]),
            Err("printf '\\377': output isn't UTF-8".into())
        );
    }
}
