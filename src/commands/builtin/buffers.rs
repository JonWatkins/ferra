//! Buffer commands: switching between and closing open buffers.
//! (`:e` opens buffers, in `files.rs`; the picker is in `picker.rs`.)

use crate::commands::CommandRegistry;
use crate::editor::UNSAVED_CHANGES;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "buffer_next",
        "Switch to the next buffer ([count] times)",
        |cx| {
            let n = cx.count() as isize;
            cx.editor.switch_relative(n);
            Ok(())
        },
    );
    r.alias("bn", "buffer_next");
    r.alias("bnext", "buffer_next");

    r.register(
        "buffer_prev",
        "Switch to the previous buffer ([count] times)",
        |cx| {
            let n = cx.count() as isize;
            cx.editor.switch_relative(-n);
            Ok(())
        },
    );
    r.alias("bp", "buffer_prev");
    r.alias("bprevious", "buffer_prev");

    r.register(
        "buffer",
        "Switch to a buffer by number, # (alternate) or part of its name",
        |cx| {
            let query = cx.args.join(" ");
            if query.is_empty() {
                return Err("Argument required: a buffer number or name".into());
            }
            let id = cx.editor.find_buffer(&query)?;
            cx.editor.switch_to(id);
            Ok(())
        },
    );
    r.alias("b", "buffer");

    r.register(
        "buffer_alternate",
        "Switch to the most recently used other buffer",
        |cx| {
            if cx.editor.switch_alternate() {
                Ok(())
            } else {
                Err("No alternate buffer".into())
            }
        },
    );

    r.register(
        "buffer_delete",
        "Close the current buffer (! to discard unsaved changes)",
        |cx| {
            if cx.editor.document.is_modified() && !cx.bang {
                return Err(UNSAVED_CHANGES.into());
            }
            cx.editor.close_buffer();
            Ok(())
        },
    );
    r.alias("bd", "buffer_delete");
    r.alias("bdelete", "buffer_delete");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::commands::Invocation;
    use crate::commands::builtin::test_util::run_with;
    use crate::editor::Editor;

    fn run(editor: &mut Editor, name: &str, args: &[&str], bang: bool) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let invocation = Invocation {
            args: &args,
            bang,
            ..Invocation::default()
        };
        run_with(editor, name, invocation);
    }

    fn message(editor: &Editor) -> String {
        editor.message().map(|m| m.text.clone()).unwrap_or_default()
    }

    fn file(dir: &Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("{name}\n")).unwrap();
        path
    }

    /// An editor with `a.txt`, `b.txt` and `c.txt` open, `c.txt` active.
    fn three_files(dir: &Path) -> Editor {
        let mut editor = Editor::default();
        for name in ["a.txt", "b.txt", "c.txt"] {
            editor.open(&file(dir, name)).unwrap();
        }
        editor
    }

    fn active(editor: &Editor) -> String {
        editor.document.buffer().text()
    }

    #[test]
    fn next_prev_and_by_number_or_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = three_files(dir.path());
        run(&mut e, "buffer_next", &[], false);
        assert_eq!(active(&e), "a.txt");
        run(&mut e, "buffer_prev", &[], false);
        assert_eq!(active(&e), "c.txt");
        run(&mut e, "buffer", &["2"], false);
        assert_eq!(active(&e), "b.txt");
        run(&mut e, "buffer", &["c.t"], false);
        assert_eq!(active(&e), "c.txt");
        run(&mut e, "buffer", &["#"], false);
        assert_eq!(active(&e), "b.txt");
    }

    #[test]
    fn buffer_without_argument_is_an_error() {
        let mut e = Editor::default();
        run(&mut e, "buffer", &[], false);
        assert!(message(&e).contains("Argument required"));
    }

    #[test]
    fn delete_refuses_unsaved_changes_unless_bang() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = three_files(dir.path());
        e.document.insert_str("x");
        run(&mut e, "buffer_delete", &[], false);
        assert_eq!(message(&e), crate::editor::UNSAVED_CHANGES);
        assert_eq!(e.buffer_count(), 3);
        run(&mut e, "buffer_delete", &[], true);
        assert_eq!(e.buffer_count(), 2);
        assert_eq!(active(&e), "b.txt", "most recently used remains");
    }

    #[test]
    fn quit_names_another_modified_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = three_files(dir.path());
        run(&mut e, "buffer", &["1"], false);
        e.document.insert_str("x");
        run(&mut e, "buffer", &["3"], false);
        run(&mut e, "quit", &[], false);
        assert!(!e.should_quit());
        assert!(message(&e).contains("for buffer 1"), "{}", message(&e));
        assert!(message(&e).contains("a.txt"), "{}", message(&e));
        run(&mut e, "quit", &[], true);
        assert!(e.should_quit());
    }

    #[test]
    fn write_all_saves_every_modified_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = three_files(dir.path());
        for id in ["1", "2"] {
            run(&mut e, "buffer", &[id], false);
            e.document.insert_str("new ");
        }
        run(&mut e, "write_all", &[], false);
        assert_eq!(message(&e), "All buffers written");
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "new a.txt\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "new b.txt\n"
        );
        assert_eq!(active(&e), "new b.txt", "back where we started");
        assert!(e.first_modified_buffer().is_none());
    }

    #[test]
    fn write_all_reports_unnamed_buffers() {
        let mut e = Editor::default();
        e.document.insert_str("x");
        run(&mut e, "write_all", &[], false);
        assert_eq!(message(&e), "Buffer 1: No file name");
    }
}
