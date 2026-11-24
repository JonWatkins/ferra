//! Telling whether a file changed since it was read or written.
//!
//! - A [`FileStamp`] is what's compared: the modified time and size, and on
//!   Unix the inode's change time and the file's identity (device and inode).
//!   The change time is set by the kernel on every write, and on `touch -r` or
//!   any other change of the modified time, so a program that puts the old
//!   modified time back is still noticed; the identity catches a file replaced
//!   by a rename.
//! - Times only tell changes apart at the filesystem's resolution (a second
//!   on HFS+, 2 s on FAT), so a change made in the same tick as the stamp,
//!   leaving the size alone, looks like no change. Like Git's "racy" index
//!   entries, a stamp whose times are within [`RACY_WINDOW`] of when it was
//!   taken keeps a hash of the contents too, and [`recheck`] compares the
//!   file's contents with it. Once the window has passed, any later change
//!   has later times, so the next [`recheck`] that finds the file unchanged
//!   hands back a stamp without the hash, and checking is a `stat` again.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, Metadata};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// How recent a stamp's times can be before they're not trusted on their
/// own: longer than the coarsest timestamp resolution in use (FAT's 2 s).
const RACY_WINDOW: Duration = Duration::from_secs(3);

/// What's compared to tell whether a file changed. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    meta: Meta,
    /// A hash of the contents, kept while the times are too recent to
    /// trust.
    contents: Option<u64>,
}

/// The parts of a file's metadata a change shows in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Meta {
    modified: Option<SystemTime>,
    len: u64,
    /// The inode's change time, as seconds and nanoseconds.
    #[cfg(unix)]
    changed: (i64, i64),
    /// Device and inode.
    #[cfg(unix)]
    identity: (u64, u64),
}

impl FileStamp {
    /// The file's size when stamped.
    pub fn len(&self) -> u64 {
        self.meta.len
    }

    /// Whether the file was empty when stamped.
    pub fn is_empty(&self) -> bool {
        self.meta.len == 0
    }
}

/// What [`recheck`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recheck {
    /// The file hasn't changed. The stamp is the one to keep in place of
    /// the old: the same, or one that's cheaper to check.
    Same(FileStamp),
    /// It has: its stamp now, or `None` if it's gone.
    Changed(Option<FileStamp>),
}

/// The file's stamp, or `None` if it doesn't exist. Reads the file if the
/// stamp needs a hash of it.
pub fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = meta(&fs::metadata(path).ok()?);
    Some(with_contents(meta, || fs::read(path).ok()))
}

/// The stamp of `path`, which holds `bytes` (just read or written), hashing
/// those rather than reading the file again. A change made since they were
/// read or written then shows as a different hash.
pub(super) fn stamp_holding(path: &Path, bytes: &[u8]) -> Option<FileStamp> {
    let meta = meta(&fs::metadata(path).ok()?);
    Some(with_contents(meta, || Some(bytes.to_vec())))
}

/// Compares the file at `path` with `known`, a stamp taken of it earlier.
pub fn recheck(path: &Path, known: &FileStamp) -> Recheck {
    let Ok(metadata) = fs::metadata(path) else {
        return Recheck::Changed(None);
    };
    let now = meta(&metadata);
    if now != known.meta {
        return Recheck::Changed(Some(with_contents(now, || fs::read(path).ok())));
    }
    let Some(hash) = known.contents else {
        return Recheck::Same(*known);
    };
    let Ok(bytes) = fs::read(path) else {
        return Recheck::Changed(None);
    };
    let stamp = FileStamp {
        meta: now,
        contents: is_racy(&now).then(|| hash_of(&bytes)),
    };
    if hash_of(&bytes) == hash {
        Recheck::Same(stamp)
    } else {
        Recheck::Changed(Some(FileStamp {
            contents: Some(hash_of(&bytes)),
            ..stamp
        }))
    }
}

/// A stamp of `meta`, with a hash of the contents from `read` if its times
/// are too recent to trust.
fn with_contents(meta: Meta, read: impl FnOnce() -> Option<Vec<u8>>) -> FileStamp {
    let contents = if is_racy(&meta) {
        read().map(|bytes| hash_of(&bytes))
    } else {
        None
    };
    FileStamp { meta, contents }
}

fn meta(metadata: &Metadata) -> Meta {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Meta {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        #[cfg(unix)]
        changed: (metadata.ctime(), metadata.ctime_nsec()),
        #[cfg(unix)]
        identity: (metadata.dev(), metadata.ino()),
    }
}

/// Whether a change in the same tick as these times could leave them as
/// they are: they're within [`RACY_WINDOW`] of now (or in the future).
fn is_racy(meta: &Meta) -> bool {
    let now = SystemTime::now();
    let recent = |time: SystemTime| match now.duration_since(time) {
        Ok(age) => age < RACY_WINDOW,
        Err(_) => true,
    };
    #[cfg(unix)]
    let changed = u64::try_from(meta.changed.0)
        .ok()
        .map(|secs| SystemTime::UNIX_EPOCH + Duration::new(secs, meta.changed.1 as u32));
    #[cfg(not(unix))]
    let changed: Option<SystemTime> = None;
    // No times to go on: only the contents can tell.
    if meta.modified.is_none() && changed.is_none() {
        return true;
    }
    meta.modified.is_some_and(recent) || changed.is_some_and(recent)
}

fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;

    /// Writes `contents` to `path` and sets its modified time to `time`.
    fn write_at(path: &Path, contents: &str, time: SystemTime) {
        fs::write(path, contents).unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn an_untouched_file_is_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "one\n").unwrap();
        let known = stamp(&path).unwrap();
        assert!(matches!(recheck(&path, &known), Recheck::Same(_)));
    }

    #[test]
    fn only_recent_times_need_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "one\n").unwrap();
        let mut known = stamp(&path).unwrap();
        assert!(known.contents.is_some(), "just written");

        known.meta.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        #[cfg(unix)]
        {
            known.meta.changed = (1, 0);
        }
        assert!(!is_racy(&known.meta));
        known.meta.modified = Some(SystemTime::now() + Duration::from_secs(60));
        assert!(is_racy(&known.meta), "in the future");
    }

    #[test]
    fn a_same_size_change_in_the_same_tick_is_noticed_by_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let time = SystemTime::now();
        write_at(&path, "one\n", time);
        let known = stamp(&path).unwrap();
        write_at(&path, "two\n", time);
        assert!(matches!(recheck(&path, &known), Recheck::Changed(Some(_))));
    }

    #[cfg(unix)]
    #[test]
    fn putting_an_old_modified_time_back_is_noticed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let old = SystemTime::now() - Duration::from_secs(3600);
        write_at(&path, "one\n", old);
        let mut known = stamp(&path).unwrap();
        known.contents = None; // as if stamped long ago
        std::thread::sleep(Duration::from_millis(20));
        write_at(&path, "two\n", old);
        assert!(matches!(recheck(&path, &known), Recheck::Changed(Some(_))));
    }

    #[test]
    fn a_deleted_file_has_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "one\n").unwrap();
        let known = stamp(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(recheck(&path, &known), Recheck::Changed(None));
    }
}
