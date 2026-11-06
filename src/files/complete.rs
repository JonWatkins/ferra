//! Completing file paths typed on the command line.

use std::path::Path;

/// Paths starting with `prefix`, relative to the current directory unless
/// `prefix` is absolute. Directories end in `/` so completion can continue
/// into them. Hidden entries are only offered once a `.` has been typed.
pub fn complete_path(prefix: &str) -> Vec<String> {
    complete_path_in(Path::new("."), prefix)
}

/// Like [`complete_path`], with relative prefixes resolved against `base`.
pub fn complete_path_in(base: &Path, prefix: &str) -> Vec<String> {
    let (dir_part, name_part) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let dir = if dir_part.starts_with('/') {
        Path::new(dir_part).to_path_buf()
    } else {
        base.join(dir_part)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut candidates: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(name_part)
                || (name.starts_with('.') && !name_part.starts_with('.'))
            {
                return None;
            }
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            Some(format!("{dir_part}{name}{}", if is_dir { "/" } else { "" }))
        })
        .collect();
    candidates.sort();
    candidates
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        for file in ["src/main.rs", "src/lib.rs", "README.md", ".hidden"] {
            fs::write(dir.path().join(file), "").unwrap();
        }
        dir
    }

    #[test]
    fn completes_names_and_marks_directories() {
        let dir = project();
        assert_eq!(complete_path_in(dir.path(), "s"), ["src/"]);
        assert_eq!(
            complete_path_in(dir.path(), "src/"),
            ["src/lib.rs", "src/main.rs", "src/nested/"]
        );
        assert_eq!(complete_path_in(dir.path(), "src/m"), ["src/main.rs"]);
    }

    #[test]
    fn hidden_files_only_after_a_dot() {
        let dir = project();
        assert_eq!(complete_path_in(dir.path(), ""), ["README.md", "src/"]);
        assert_eq!(complete_path_in(dir.path(), "."), [".hidden"]);
    }

    #[test]
    fn absolute_prefixes_and_missing_directories() {
        let dir = project();
        let absolute = format!("{}/src/l", dir.path().display());
        assert_eq!(
            complete_path_in(Path::new("/nowhere"), &absolute),
            [format!("{}/src/lib.rs", dir.path().display())]
        );
        assert!(complete_path_in(dir.path(), "missing/x").is_empty());
    }
}
