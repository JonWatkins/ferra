//! Reading and writing files on disk.
//!
//! This is the only module that touches the filesystem. It deals in raw
//! file contents; converting them to buffer text (line endings, final
//! newline) is [`core::TextFormat`](crate::core::TextFormat)'s job.

mod error;

pub use error::FileError;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A file's contents and when it was last modified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContents {
    pub text: String,
    pub modified: Option<SystemTime>,
}

/// Reads a UTF-8 text file. Returns `Ok(None)` if it doesn't exist, so a
/// new file can be created by opening its path and saving.
pub fn read(path: &Path) -> Result<Option<FileContents>, FileError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(FileError::io(path, source)),
    };
    let text = String::from_utf8(bytes).map_err(|_| FileError::NotUtf8 {
        path: path.to_path_buf(),
    })?;
    Ok(Some(FileContents {
        text,
        modified: modified_time(path),
    }))
}

/// The file's last-modified time, or `None` if it doesn't exist (or the
/// platform can't say).
pub fn modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Writes `contents` to `path` atomically, returning the new modified time.
///
/// The data goes to a temporary file in the same directory, is flushed to
/// disk, and is then renamed over the target, so a crash or full disk never
/// leaves a half-written file. An existing file's permissions are kept, and
/// a symlink is followed so the link itself survives.
pub fn write(path: &Path, contents: &str) -> Result<Option<SystemTime>, FileError> {
    let target = resolve_symlink(path);
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };

    let (temp_path, mut temp) =
        create_temp_file(&dir, &target).map_err(|e| FileError::io(path, e))?;
    let result = (|| {
        temp.write_all(contents.as_bytes())?;
        temp.sync_all()?;
        if let Ok(existing) = fs::metadata(&target) {
            fs::set_permissions(&temp_path, existing.permissions())?;
        }
        fs::rename(&temp_path, &target)
    })();

    if let Err(e) = result {
        let _ = fs::remove_file(&temp_path);
        return Err(FileError::io(path, e));
    }
    Ok(modified_time(&target))
}

/// The real file behind `path`, if `path` is a symlink.
fn resolve_symlink(path: &Path) -> PathBuf {
    let is_symlink = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    if is_symlink {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

/// Creates a new, uniquely named hidden file next to `target`.
fn create_temp_file(dir: &Path, target: &Path) -> io::Result<(PathBuf, File)> {
    let name = target
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    for attempt in 0..100 {
        let candidate = dir.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a temporary file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn read_missing_file_is_none() {
        let dir = dir();
        assert_eq!(read(&dir.path().join("nope.txt")).unwrap(), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = dir();
        let path = dir.path().join("a.txt");
        let modified = write(&path, "hello\n").unwrap();
        let contents = read(&path).unwrap().unwrap();
        assert_eq!(contents.text, "hello\n");
        assert_eq!(contents.modified, modified);
    }

    #[test]
    fn write_replaces_existing_contents_and_leaves_no_temp_files() {
        let dir = dir();
        let path = dir.path().join("a.txt");
        write(&path, "first version, longer").unwrap();
        write(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1, "temp file left behind");
    }

    #[test]
    fn invalid_utf8_is_refused() {
        let dir = dir();
        let path = dir.path().join("bin");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read(&path), Err(FileError::NotUtf8 { .. })));
    }

    #[test]
    fn reading_a_directory_is_an_error() {
        let dir = dir();
        assert!(matches!(read(dir.path()), Err(FileError::Io { .. })));
    }

    #[test]
    fn writing_into_missing_directory_is_an_error() {
        let dir = dir();
        let path = dir.path().join("missing").join("a.txt");
        let err = write(&path, "x").unwrap_err();
        assert!(err.to_string().contains("a.txt"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let path = dir.path().join("script.sh");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        write(&path, "new").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[cfg(unix)]
    #[test]
    fn write_follows_symlinks() {
        let dir = dir();
        let real = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write(&link, "new").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
    }
}
