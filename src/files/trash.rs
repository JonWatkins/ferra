//! The file manager's side of things: moving to the bin (rather than
//! deleting for good), and showing a file in Finder or the like.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Moves `path` (a file or a folder) to the bin: macOS's `~/.Trash`, or the
/// freedesktop trash elsewhere (`$XDG_DATA_HOME/Trash`), with what's needed
/// to put it back. Returns where it went.
pub fn trash(path: &Path) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("No home folder (HOME isn't set)")?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: can't be moved to the bin", path.display()))?;
    if cfg!(target_os = "macos") {
        let bin = home.join(".Trash");
        std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        let to = free_name(&bin, name);
        return match move_to(path, &to) {
            Ok(()) => Ok(to),
            // Without Full Disk Access the bin can't be written to
            // directly: Finder can.
            Err(error) => finder_delete(path).map(|()| bin).map_err(|_| error),
        };
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".local/share"), PathBuf::from)
        .join("Trash");
    let files = base.join("files");
    let info = base.join("info");
    for dir in [&files, &info] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let to = free_name(&files, name);
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let stamp = crate::log::timestamp(std::time::SystemTime::now()).replace(' ', "T");
    let record = format!(
        "[Trash Info]\nPath={}\nDeletionDate={stamp}\n",
        absolute.display()
    );
    let info_file = info.join(format!(
        "{}.trashinfo",
        to.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&info_file, record).map_err(|e| format!("{}: {e}", info_file.display()))?;
    move_to(path, &to).map(|()| to)
}

/// Asks Finder to move `path` to the bin.
fn finder_delete(path: &Path) -> Result<(), String> {
    let absolute = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let quoted = absolute
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = format!("tell application \"Finder\" to delete POSIX file \"{quoted}\"");
    let status = Command::new("osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Finder couldn't move it".into())
    }
}

/// `dir/name`, or `dir/name 2`, `name 3`... if that's taken.
fn free_name(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let name = name.to_string_lossy();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} {n}{ext}")))
        .find(|p| !p.exists())
        .expect("a free name")
}

/// Renames `from` to `to`, copying (then removing) across filesystems.
fn move_to(from: &Path, to: &Path) -> Result<(), String> {
    let error = |e: std::io::Error| format!("{}: {e}", from.display());
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) if from.is_file() => {
            std::fs::copy(from, to).map_err(error)?;
            std::fs::remove_file(from).map_err(error)
        }
        Err(e) => Err(error(e)),
    }
}

/// Shows `path` in the system's file manager (Finder selects it).
pub fn reveal(path: &Path) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        });
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the file manager: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_names_get_a_number() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("a 2.txt"), "").unwrap();
        assert_eq!(
            free_name(dir.path(), "a.txt".as_ref()),
            dir.path().join("a 3.txt")
        );
        assert_eq!(free_name(dir.path(), "b".as_ref()), dir.path().join("b"));
    }
}
