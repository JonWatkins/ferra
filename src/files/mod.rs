//! Reading and writing files on disk.
//!
//! This is the only module that touches the filesystem. It deals in raw
//! bytes; converting them to buffer text (encoding, line endings, final
//! newline) is [`core::TextFormat`](crate::core::TextFormat)'s job.

mod complete;
mod error;
mod walk;

pub use complete::{complete_path, complete_path_in};
pub use error::FileError;
pub use walk::{FileList, WalkOptions, list_files, walk_files};

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A file's bytes, and its stamp when they were read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContents {
    pub bytes: Vec<u8>,
    pub stamp: Option<FileStamp>,
}

/// What's compared to tell whether a file changed: its modified time and
/// size. The size catches a change made within the filesystem's timestamp
/// resolution of an earlier one (often a second, or 2 s on FAT), unless it
/// left the size the same too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

/// Reads a file. Returns `Ok(None)` if it doesn't exist, so a new file can
/// be created by opening its path and saving.
pub fn read(path: &Path) -> Result<Option<FileContents>, FileError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(FileError::io(path, source)),
    };
    Ok(Some(FileContents {
        stamp: stamp(path),
        bytes,
    }))
}

/// The file's stamp, or `None` if it doesn't exist.
pub fn stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

/// Writes `contents` to `path`, returning the file's new stamp.
///
/// Normally the data goes to a temporary file in the same directory, is
/// flushed to disk, and is then renamed over the target, so a crash or full
/// disk never leaves a half-written file. An existing file's permissions
/// are kept, and a symlink is followed so the link itself survives.
///
/// Renaming gives the file a new identity, though, so (like Vim's
/// `backupcopy=auto`) the file is written in place instead when that would
/// lose something: when it has other hard-linked names, when its owner or
/// group can't be kept, or when no temporary file can be made next to it
/// (a read-only directory). See [`write_in_place`].
pub fn write(path: &Path, contents: &[u8]) -> Result<Option<FileStamp>, FileError> {
    let target = resolve_symlink(path);
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let existing = fs::metadata(&target).ok();

    if existing.as_ref().is_some_and(has_other_names) {
        write_in_place(&target, &dir, contents).map_err(|e| FileError::io(path, e))?;
        return Ok(stamp(&target));
    }
    let (temp_path, mut temp) = match create_temp_file(&dir, &target) {
        Ok(created) => created,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && existing.is_some() => {
            write_in_place(&target, &dir, contents).map_err(|e| FileError::io(path, e))?;
            return Ok(stamp(&target));
        }
        Err(e) => return Err(FileError::io(path, e)),
    };
    if let Some(existing) = &existing
        && !keep_owner(&temp_path, existing)
    {
        drop(temp);
        let _ = fs::remove_file(&temp_path);
        write_in_place(&target, &dir, contents).map_err(|e| FileError::io(path, e))?;
        return Ok(stamp(&target));
    }

    let result = (|| {
        temp.write_all(contents)?;
        temp.sync_all()?;
        if let Some(existing) = &existing {
            fs::set_permissions(&temp_path, existing.permissions())?;
        }
        fs::rename(&temp_path, &target)
    })();

    if let Err(e) = result {
        let _ = fs::remove_file(&temp_path);
        return Err(FileError::io(path, e));
    }
    Ok(stamp(&target))
}

/// Whether the file has other hard-linked names, which a rename would leave
/// pointing at the old contents.
#[cfg(unix)]
fn has_other_names(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(not(unix))]
fn has_other_names(_: &fs::Metadata) -> bool {
    false
}

/// Gives the temporary file the original's owner and group, so the rename
/// keeps them. `false` if that's not allowed (someone else's file).
#[cfg(unix)]
fn keep_owner(temp: &Path, original: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(current) = fs::metadata(temp) else {
        return false;
    };
    if current.uid() == original.uid() && current.gid() == original.gid() {
        return true;
    }
    std::os::unix::fs::chown(temp, Some(original.uid()), Some(original.gid())).is_ok()
}

#[cfg(not(unix))]
fn keep_owner(_: &Path, _: &fs::Metadata) -> bool {
    true
}

/// Overwrites `target` itself, keeping its identity (hard links, owner,
/// inode). Less safe than a rename, so the original is copied to a backup
/// first (next to it, or failing that in the system temp directory). If
/// writing fails, the original is put back; if even that fails, the error
/// says where the backup is.
fn write_in_place(target: &Path, dir: &Path, contents: &[u8]) -> io::Result<()> {
    let (backup_path, backup) = create_temp_file(dir, target)
        .or_else(|_| create_temp_file(&std::env::temp_dir(), target))?;
    drop(backup);
    if let Err(e) = fs::copy(target, &backup_path) {
        let _ = fs::remove_file(&backup_path);
        return Err(e);
    }

    let written = (|| {
        let mut file = OpenOptions::new().write(true).truncate(true).open(target)?;
        file.write_all(contents)?;
        file.sync_all()
    })();
    match written {
        Ok(()) => {
            let _ = fs::remove_file(&backup_path);
            Ok(())
        }
        Err(e) => match fs::copy(&backup_path, target) {
            Ok(_) => {
                let _ = fs::remove_file(&backup_path);
                Err(e)
            }
            Err(_) => Err(io::Error::new(
                e.kind(),
                format!(
                    "{e}; the file may be incomplete, its previous contents are in {}",
                    backup_path.display()
                ),
            )),
        },
    }
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
        let stamp = write(&path, b"hello\n").unwrap();
        let contents = read(&path).unwrap().unwrap();
        assert_eq!(contents.bytes, b"hello\n");
        assert_eq!(contents.stamp, stamp);
        assert_eq!(stamp.unwrap().len, 6);
    }

    #[test]
    fn write_replaces_existing_contents_and_leaves_no_temp_files() {
        let dir = dir();
        let path = dir.path().join("a.txt");
        write(&path, b"first version, longer").unwrap();
        write(&path, b"second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1, "temp file left behind");
    }

    #[test]
    fn any_bytes_are_read_as_they_are() {
        // Decoding them is TextFormat's job.
        let dir = dir();
        let path = dir.path().join("bin");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(read(&path).unwrap().unwrap().bytes, [0xff, 0xfe, 0x00]);
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
        let err = write(&path, b"x").unwrap_err();
        assert!(err.to_string().contains("a.txt"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let path = dir.path().join("script.sh");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        write(&path, b"new").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[cfg(unix)]
    #[test]
    fn write_follows_symlinks() {
        let dir = dir();
        let real = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write(&link, b"new").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "new");
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_hard_links() {
        use std::os::unix::fs::MetadataExt;
        let dir = dir();
        let path = dir.path().join("a.txt");
        let other = dir.path().join("b.txt");
        fs::write(&path, b"old\n").unwrap();
        fs::hard_link(&path, &other).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();

        write(&path, b"new\n").unwrap();
        assert_eq!(
            fs::read_to_string(&other).unwrap(),
            "new\n",
            "the other name sees the edit"
        );
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode, "same file");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            2,
            "no backup left behind"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_into_a_read_only_directory_overwrites_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        let path = locked.join("a.txt");
        fs::write(&path, b"old\n").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        let result = write(&path, b"new\n");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        result.unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(fs::read_dir(&locked).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_new_file_in_a_read_only_directory_still_fails() {
        use std::os::unix::fs::PermissionsExt;
        let dir = dir();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let result = write(&locked.join("new.txt"), b"x");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn in_place_writes_leave_no_backups_whether_or_not_they_work() {
        let dir = dir();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"original\n").unwrap();
        // A directory can't be copied as a backup, so this fails before
        // writing anything. (A write failing halfway, which restores the
        // backup, can't easily be simulated.)
        let not_a_file = dir.path().join("sub");
        fs::create_dir(&not_a_file).unwrap();
        assert!(write_in_place(&not_a_file, dir.path(), b"x").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
        write_in_place(&path, dir.path(), b"replaced\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "replaced\n");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            2,
            "no backups left"
        );
    }
}
