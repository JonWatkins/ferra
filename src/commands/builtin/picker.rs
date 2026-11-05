//! Picker popups: the buffer list and the command palette, plus the
//! commands that drive an open picker.

use crate::commands::{CommandRegistry, Invocation};
use std::path::PathBuf;

use crate::editor::picker::{Picker, PickerAction, PickerItem};
use crate::editor::{Message, Mode};
use crate::files;

/// Commands that make no sense to run from the palette: they drive the
/// prompt or picker itself, or need a key typed after them.
fn hidden_from_palette(registry: &CommandRegistry, name: &str) -> bool {
    name.starts_with("command_line_")
        || name.starts_with("picker_")
        || name == "command_palette"
        || registry.takes_char(name)
}

pub fn register(r: &mut CommandRegistry) {
    r.register("buffer_picker", "Pick an open buffer to switch to", |cx| {
        let alternate = cx.editor.alternate_buffer();
        let items = cx
            .editor
            .buffers()
            .into_iter()
            .map(|buffer| {
                let mut detail = format!("#{}", buffer.id);
                if buffer.modified {
                    detail.push_str(" [+]");
                }
                if buffer.active {
                    detail.push_str(" (current)");
                }
                PickerItem {
                    label: buffer.name,
                    detail,
                    action: PickerAction::SwitchBuffer(buffer.id),
                }
            })
            .collect();
        let mut picker = Picker::new("Buffers", items);
        // Start on the alternate buffer, so open + Enter toggles between two.
        if let Some(id) = alternate {
            picker = picker.with_selected(&PickerAction::SwitchBuffer(id));
        }
        cx.editor.open_picker(picker);
        Ok(())
    });
    r.alias("ls", "buffer_picker");
    r.alias("buffers", "buffer_picker");

    r.register(
        "file_picker",
        "Pick a file to open, from the current directory or [dir]",
        |cx| {
            let given = (!cx.args.is_empty()).then(|| PathBuf::from(cx.args.join(" ")));
            let root = given.clone().unwrap_or_else(|| PathBuf::from("."));
            let options = cx.editor.options.file_picker.into();
            let list = files::list_files(&root, &options);
            if list.truncated {
                cx.editor.set_message(Message::info(format!(
                    "Showing the first {} files (see max-files)",
                    list.files.len()
                )));
            }
            let items = list
                .files
                .into_iter()
                .map(|relative| {
                    // Files under the current directory open by their
                    // relative path, which keeps buffer names short.
                    let path = match &given {
                        Some(dir) => dir.join(&relative),
                        None => relative.clone(),
                    };
                    PickerItem {
                        label: relative.to_string_lossy().into_owned(),
                        detail: String::new(),
                        action: PickerAction::OpenFile(path),
                    }
                })
                .collect();
            cx.editor.open_picker(Picker::new("Files", items));
            Ok(())
        },
    );
    r.alias("files", "file_picker");

    r.register("command_palette", "Pick a command to run", |cx| {
        let items = cx
            .registry
            .commands()
            .into_iter()
            .filter(|c| !hidden_from_palette(cx.registry, &c.name))
            .map(|c| PickerItem {
                label: c.name.clone(),
                detail: c.description.clone(),
                action: PickerAction::RunCommand(c.name.clone()),
            })
            .collect();
        cx.editor.open_picker(Picker::new("Commands", items));
        Ok(())
    });

    r.register("picker_next", "Select the next picker item", |cx| {
        if let Some(picker) = cx.editor.picker_mut() {
            picker.move_selection(1);
        }
        Ok(())
    });
    r.register("picker_prev", "Select the previous picker item", |cx| {
        if let Some(picker) = cx.editor.picker_mut() {
            picker.move_selection(-1);
        }
        Ok(())
    });
    r.register(
        "picker_backspace",
        "Delete the last filter character",
        |cx| {
            if let Some(picker) = cx.editor.picker_mut() {
                picker.pop();
            }
            Ok(())
        },
    );
    r.register("picker_cancel", "Close the picker", |cx| {
        cx.editor.set_mode(Mode::Normal);
        Ok(())
    });
    r.register(
        "picker_accept",
        "Choose the selected picker item",
        |cx| match cx.editor.accept_picker() {
            Some(PickerAction::SwitchBuffer(id)) => {
                cx.editor.switch_to(id);
                Ok(())
            }
            Some(PickerAction::OpenFile(path)) => cx.editor.open(&path).map_err(|e| e.to_string()),
            Some(PickerAction::RunCommand(name)) => {
                cx.registry.execute(&name, cx.editor, Invocation::default())
            }
            None => Ok(()),
        },
    );
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::commands::builtin::test_util::run;
    use crate::editor::{Editor, Mode};

    fn type_text(e: &mut Editor, text: &str) {
        text.chars().for_each(|c| e.insert_text_char(c));
    }

    fn labels(e: &Editor) -> Vec<String> {
        let picker = e.picker().expect("picker open");
        picker
            .matches()
            .iter()
            .map(|m| picker.item(m).label.clone())
            .collect()
    }

    fn with_files(names: &[&str]) -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = Editor::default();
        for name in names {
            let path = dir.path().join(name);
            fs::write(&path, format!("{name}\n")).unwrap();
            editor.open(&path).unwrap();
        }
        (dir, editor)
    }

    #[test]
    fn buffer_picker_lists_buffers_and_starts_on_alternate() {
        let (_dir, mut e) = with_files(&["one.rs", "two.rs", "three.rs"]);
        run(&mut e, "buffer_picker", None);
        assert_eq!(e.mode(), Mode::Picker);
        assert_eq!(labels(&e).len(), 3);
        let selected = e.picker().unwrap().selected_item().unwrap();
        assert!(selected.label.ends_with("two.rs"), "{}", selected.label);
        assert_eq!(selected.detail, "#2");

        run(&mut e, "picker_accept", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.document.buffer().text(), "two.rs");
    }

    #[test]
    fn buffer_picker_filters_by_typing() {
        let (_dir, mut e) = with_files(&["alpha.rs", "beta.rs"]);
        run(&mut e, "buffer_picker", None);
        // The temp paths are long, so scattered letters can match both; the
        // file name match must rank first.
        type_text(&mut e, "alpha");
        assert!(labels(&e)[0].ends_with("alpha.rs"), "{:?}", labels(&e));
        run(&mut e, "picker_accept", None);
        assert_eq!(e.document.buffer().text(), "alpha.rs");
    }

    #[test]
    fn buffer_picker_marks_modified_and_current() {
        let (_dir, mut e) = with_files(&["a.rs"]);
        e.document.insert_str("x");
        run(&mut e, "buffer_picker", None);
        let selected = e.picker().unwrap().selected_item().unwrap();
        assert_eq!(selected.detail, "#1 [+] (current)");
    }

    #[test]
    fn file_picker_lists_and_opens_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("README.md"), "# hi\n").unwrap();

        let mut e = Editor::default();
        let root = dir.path().to_str().unwrap();
        crate::commands::builtin::test_util::run_with_args(&mut e, "file_picker", &[root]);
        assert_eq!(e.mode(), Mode::Picker);
        assert_eq!(
            labels(&e),
            ["README.md", "src/main.rs"],
            "relative to the root"
        );

        type_text(&mut e, "main");
        run(&mut e, "picker_accept", None);
        assert_eq!(e.document.buffer().text(), "fn main() {}");
        assert!(e.document.path().unwrap().ends_with("src/main.rs"));
    }

    #[test]
    fn file_picker_reports_truncation() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            fs::write(dir.path().join(name), "").unwrap();
        }
        let mut e = Editor::default();
        e.options.file_picker.max_files = 2;
        let root = dir.path().to_str().unwrap();
        crate::commands::builtin::test_util::run_with_args(&mut e, "file_picker", &[root]);
        assert_eq!(labels(&e).len(), 2);
        assert!(e.message().unwrap().text.contains("first 2 files"));
    }

    #[test]
    fn command_palette_runs_the_chosen_command() {
        let mut e = Editor::default();
        run(&mut e, "command_palette", None);
        let all = labels(&e);
        assert!(all.contains(&"write".to_string()));
        assert!(
            !all.iter().any(|l| l.starts_with("picker_")),
            "internal commands hidden"
        );
        assert!(
            !all.contains(&"find_char_forward".to_string()),
            "needs a char"
        );

        type_text(&mut e, "insert_mode");
        run(&mut e, "picker_accept", None);
        assert_eq!(e.mode(), Mode::Insert);
    }

    #[test]
    fn navigation_and_cancel() {
        let mut e = Editor::default();
        run(&mut e, "command_palette", None);
        let first = e.picker().unwrap().selected_item().unwrap().label.clone();
        run(&mut e, "picker_next", None);
        run(&mut e, "picker_prev", None);
        assert_eq!(e.picker().unwrap().selected_item().unwrap().label, first);

        type_text(&mut e, "zzzz");
        run(&mut e, "picker_backspace", None);
        assert_eq!(e.picker().unwrap().query(), "zzz");

        run(&mut e, "picker_cancel", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert!(e.picker().is_none());
    }

    #[test]
    fn accepting_with_no_match_just_closes() {
        let mut e = Editor::default();
        run(&mut e, "command_palette", None);
        type_text(&mut e, "no command looks like this");
        run(&mut e, "picker_accept", None);
        assert_eq!(e.mode(), Mode::Normal);
    }
}
