//! Command-line arguments: `ferra [OPTIONS] [FILE]`.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: ferra [OPTIONS] [FILE]...

Arguments:
  [FILE]...  Files to open, one buffer each; created on save if missing

Options:
  -h, --help     Print help
  -V, --version  Print version";

/// What the program was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum CliAction {
    Edit { files: Vec<PathBuf> },
    PrintHelp,
    PrintVersion,
}

/// Parses arguments (excluding the program name).
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<CliAction, String> {
    let mut files = Vec::new();
    let mut options_done = false;

    for arg in args {
        if !options_done {
            match arg.to_str() {
                Some("-h" | "--help") => return Ok(CliAction::PrintHelp),
                Some("-V" | "--version") => return Ok(CliAction::PrintVersion),
                Some("--") => {
                    options_done = true;
                    continue;
                }
                Some(flag) if flag.starts_with('-') && flag != "-" => {
                    return Err(format!("unknown option '{flag}'"));
                }
                _ => {}
            }
        }
        files.push(PathBuf::from(arg));
    }

    Ok(CliAction::Edit { files })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<CliAction, String> {
        parse(args.iter().map(OsString::from))
    }

    fn edit(files: &[&str]) -> CliAction {
        CliAction::Edit {
            files: files.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn no_arguments_opens_scratch() {
        assert_eq!(parse_strs(&[]), Ok(edit(&[])));
    }

    #[test]
    fn single_file() {
        assert_eq!(parse_strs(&["notes.md"]), Ok(edit(&["notes.md"])));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_strs(&["--help"]), Ok(CliAction::PrintHelp));
        assert_eq!(parse_strs(&["a.txt", "-h"]), Ok(CliAction::PrintHelp));
        assert_eq!(parse_strs(&["-V"]), Ok(CliAction::PrintVersion));
    }

    #[test]
    fn double_dash_allows_dash_prefixed_file_names() {
        assert_eq!(parse_strs(&["--", "-weird.txt"]), Ok(edit(&["-weird.txt"])));
        assert_eq!(parse_strs(&["--", "--help"]), Ok(edit(&["--help"])));
    }

    #[test]
    fn rejects_unknown_options_and_accepts_many_files() {
        assert!(
            parse_strs(&["--frobnicate"])
                .unwrap_err()
                .contains("--frobnicate")
        );
        assert_eq!(parse_strs(&["a", "b"]), Ok(edit(&["a", "b"])));
    }

    #[cfg(unix)]
    #[test]
    fn accepts_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![b'f', 0xff]);
        assert_eq!(
            parse([raw.clone()]),
            Ok(CliAction::Edit {
                files: vec![PathBuf::from(raw)]
            })
        );
    }
}
