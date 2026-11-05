//! The binary's command-line interface (only the non-interactive paths; the
//! editor itself needs a terminal).

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_terminal-editor"))
        .args(args)
        .output()
        .expect("binary runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn help_prints_usage() {
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("Usage: terminal-editor [OPTIONS] [FILE]"),
        "{stdout}"
    );
}

#[test]
fn version_prints_crate_version() {
    let (code, stdout, _) = run(&["-V"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout.trim(),
        format!("terminal-editor {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn bad_arguments_exit_with_usage_error() {
    let (code, _, stderr) = run(&["--nope"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("unknown option '--nope'"), "{stderr}");
    assert!(stderr.contains("Usage:"), "{stderr}");
}
