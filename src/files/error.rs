use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Why opening or saving a file failed. Displays as a user-facing message.
#[derive(Debug)]
pub enum FileError {
    /// Saving a document that has never had a path.
    NoFileName,
    /// The file's bytes aren't valid in its encoding (opening), or the text
    /// has characters its encoding can't represent (saving).
    Encoding {
        path: PathBuf,
        message: String,
    },
    /// The file changed on disk after it was read.
    ChangedOnDisk {
        path: PathBuf,
    },
    /// Saving to a path that already holds a file this document didn't read.
    AlreadyExists {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl FileError {
    pub fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::NoFileName => f.write_str("No file name"),
            FileError::Encoding { path, message } => {
                write!(f, "\"{}\" {message}", path.display())
            }
            FileError::ChangedOnDisk { path } => write!(
                f,
                "\"{}\" has changed on disk since it was read (add ! to overwrite)",
                path.display()
            ),
            FileError::AlreadyExists { path } => {
                write!(
                    f,
                    "\"{}\" already exists (add ! to overwrite)",
                    path.display()
                )
            }
            FileError::Io { path, source } => write!(f, "\"{}\": {source}", path.display()),
        }
    }
}

impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FileError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
