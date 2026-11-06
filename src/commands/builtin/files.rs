//! File and session commands: `:w`, `:e`, `:q` and friends.
//!
//! A trailing `!` (`cx.bang`) overrides the safety checks, as in Vim.

use std::path::PathBuf;

use crate::commands::{ArgCompletion, CommandRegistry, CommandResult, Context};
use crate::editor::UNSAVED_CHANGES;
use crate::events::Event;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "write",
        "Save the file, or write to [path] (! to overwrite)",
        write,
    )
    .complete_args(ArgCompletion::Path);
    r.alias("w", "write");

    r.register("write_quit", "Save the file and quit", |cx| {
        write(cx)?;
        quit(cx)
    })
    .complete_args(ArgCompletion::Path);
    r.alias("wq", "write_quit");

    r.register("exit", "Save if there are changes, then quit (ZZ)", |cx| {
        if cx.editor.document.is_modified() || !cx.args.is_empty() {
            write(cx)?;
        }
        quit(cx)
    });
    r.alias("x", "exit");
    r.alias("xit", "exit");

    r.register(
        "quit",
        "Close the window, or quit from the last one (! to discard changes)",
        quit,
    );
    r.alias("q", "quit");
    r.register(
        "quit_all",
        "Quit, unless any buffer has unsaved changes (! to discard them)",
        quit_all,
    );
    r.alias("qa", "quit_all");
    r.alias("qall", "quit_all");

    r.register(
        "write_all",
        "Save every buffer with unsaved changes",
        write_all,
    );
    r.alias("wa", "write_all");
    r.register(
        "write_all_quit",
        "Save every buffer with unsaved changes, then quit",
        |cx| {
            write_all(cx)?;
            quit_all(cx)
        },
    );
    r.alias("wqa", "write_all_quit");
    r.alias("xa", "write_all_quit");

    r.register(
        "check_disk_changes",
        "Reload buffers whose files changed on disk; warn about modified ones",
        |cx| {
            cx.editor.check_disk_changes();
            Ok(())
        },
    );
    r.alias("checktime", "check_disk_changes");

    r.register(
        "force_quit",
        "Quit, discarding unsaved changes (ZQ)",
        |cx| {
            cx.editor.quit();
            Ok(())
        },
    );

    r.register(
        "edit",
        "Open [path] in a buffer, or reload the current file (! to discard changes)",
        |cx| {
            if let Some(path) = path_arg(cx) {
                // The current buffer stays open, so nothing is lost.
                return cx.editor.open(&path).map_err(|e| e.to_string());
            }
            if cx.editor.document.is_modified() && !cx.bang {
                return Err(UNSAVED_CHANGES.into());
            }
            cx.editor.reload().map_err(|e| e.to_string())
        },
    )
    .complete_args(ArgCompletion::Path);
    r.alias("e", "edit");
}

/// `:q`: closes the window if there are others (its buffer stays open, so
/// nothing is lost); from the last window, quits like `:qa`.
fn quit(cx: &mut Context) -> CommandResult {
    if cx.editor.windows().count() > 1 {
        return cx.editor.close_window();
    }
    quit_all(cx)
}

/// Quits unless a buffer has unsaved changes (or `!` was given).
fn quit_all(cx: &mut Context) -> CommandResult {
    if !cx.bang
        && let Some(doc) = cx.editor.first_modified_buffer()
    {
        if doc.id() == cx.editor.document.id() {
            return Err(UNSAVED_CHANGES.into());
        }
        return Err(format!(
            "No write since last change for buffer {} \"{}\" (add ! to override)",
            doc.id(),
            doc.name()
        ));
    }
    cx.editor.quit();
    Ok(())
}

/// `:wa`: saves each modified buffer in turn (running its before-save hooks
/// as the active buffer), then returns to the buffer we started in.
fn write_all(cx: &mut Context) -> CommandResult {
    let start = cx.editor.document.id();
    let mut result = Ok(());
    for id in cx.editor.modified_buffer_ids() {
        cx.editor.switch_to(id);
        let saved = cx
            .registry
            .fire(&Event::BeforeSave, cx.editor)
            .and_then(|()| cx.editor.save(None, cx.bang).map_err(|e| e.to_string()));
        if let Err(error) = saved {
            result = Err(format!("Buffer {id}: {error}"));
            break;
        }
    }
    cx.editor.switch_to(start);
    if result.is_ok() {
        cx.editor
            .set_message(crate::editor::Message::info("All buffers written"));
    }
    result
}

fn write(cx: &mut Context) -> CommandResult {
    let target = path_arg(cx);
    // Before-save hooks can edit the text first, or cancel with an error.
    cx.registry.fire(&Event::BeforeSave, cx.editor)?;
    cx.editor
        .save(target.as_deref(), cx.bang)
        .map_err(|e| e.to_string())
}

/// The path argument, if any. Arguments are rejoined so a path containing
/// spaces works without quoting.
fn path_arg(cx: &Context) -> Option<PathBuf> {
    (!cx.args.is_empty()).then(|| PathBuf::from(cx.args.join(" ")))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::commands::Invocation;
    use crate::commands::builtin::test_util::run_with;
    use crate::editor::{Document, Editor};

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

    fn opened(path: &Path) -> Editor {
        let mut editor = Editor::default();
        editor.open(path).unwrap();
        editor
    }

    #[test]
    fn write_saves_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let mut editor = opened(&path);
        editor.document.insert_str("hi");
        run(&mut editor, "write", &[], false);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hi\n");
        assert!(
            message(&editor).ends_with("1L, 3B written"),
            "{}",
            message(&editor)
        );
    }

    #[test]
    fn write_with_path_containing_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my notes.txt");
        let mut editor = Editor::with_document(Document::with_text("x"));
        let path_str = path.to_str().unwrap();
        let parts: Vec<&str> = path_str.split(' ').collect();
        run(&mut editor, "write", &parts, false);
        assert_eq!(fs::read_to_string(&path).unwrap(), "x\n");
    }

    #[test]
    fn write_without_file_name_is_an_error() {
        let mut editor = Editor::default();
        run(&mut editor, "write", &[], false);
        assert_eq!(message(&editor), "No file name");
    }

    #[test]
    fn quit_refuses_with_unsaved_changes_unless_bang() {
        let mut editor = Editor::with_document(Document::with_text(""));
        editor.document.insert_str("x");
        run(&mut editor, "quit", &[], false);
        assert!(!editor.should_quit());
        assert_eq!(message(&editor), crate::editor::UNSAVED_CHANGES);
        run(&mut editor, "quit", &[], true);
        assert!(editor.should_quit());
    }

    #[test]
    fn quit_without_changes() {
        let mut editor = Editor::default();
        run(&mut editor, "quit", &[], false);
        assert!(editor.should_quit());
    }

    #[test]
    fn force_quit_ignores_changes() {
        let mut editor = Editor::default();
        editor.document.insert_str("x");
        run(&mut editor, "force_quit", &[], false);
        assert!(editor.should_quit());
    }

    #[test]
    fn write_quit_does_not_quit_if_write_fails() {
        let mut editor = Editor::with_document(Document::with_text("x"));
        run(&mut editor, "write_quit", &[], false);
        assert!(!editor.should_quit());
        assert_eq!(message(&editor), "No file name");
    }

    #[test]
    fn exit_only_writes_when_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "same\n").unwrap();
        let mut editor = opened(&path);
        fs::remove_file(&path).unwrap();
        run(&mut editor, "exit", &[], false);
        assert!(editor.should_quit());
        assert!(!path.exists(), "unmodified document should not be written");

        let mut editor = opened(&path);
        editor.document.insert_str("new");
        run(&mut editor, "exit", &[], false);
        assert!(editor.should_quit());
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
    }

    #[test]
    fn edit_opens_another_file() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("b.txt");
        fs::write(&other, "bee\n").unwrap();
        let mut editor = Editor::default();
        run(&mut editor, "edit", &[other.to_str().unwrap()], false);
        assert_eq!(editor.document.buffer().text(), "bee");
    }

    #[test]
    fn edit_refuses_to_discard_changes_unless_bang() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "disk\n").unwrap();
        let mut editor = opened(&path);
        editor.document.insert_str("edited ");

        run(&mut editor, "edit", &[], false);
        assert_eq!(message(&editor), crate::editor::UNSAVED_CHANGES);
        assert_eq!(editor.document.buffer().text(), "edited disk");

        run(&mut editor, "edit", &[], true);
        assert_eq!(
            editor.document.buffer().text(),
            "disk",
            "e! reloads from disk"
        );
        assert!(!editor.document.is_modified());
    }

    #[test]
    fn edit_reports_open_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        let mut editor = Editor::default();
        run(&mut editor, "edit", &[path.to_str().unwrap()], false);
        assert!(message(&editor).contains("not valid UTF-8"));
        assert_eq!(editor.document.path(), None, "document unchanged");
    }
}
