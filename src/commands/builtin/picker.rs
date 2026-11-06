//! Picker popups: buffers, files, the command palette and registers, plus
//! the commands that drive an open picker.

use crate::commands::{ArgCompletion, CommandRegistry, Invocation};
use std::path::PathBuf;

use crate::editor::picker::{Picker, PickerAction, PickerItem};
use crate::editor::{FileListRequest, Mode};

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
            // The picker opens straight away; the app lists the files in
            // the background and adds them as they're found.
            let given = (!cx.args.is_empty()).then(|| PathBuf::from(cx.args.join(" ")));
            let root = given.clone().unwrap_or_else(|| PathBuf::from("."));
            let job = cx.editor.new_job_id();
            cx.editor
                .open_picker(Picker::new("Files", Vec::new()).loading_from(job));
            cx.editor.request_file_list(FileListRequest {
                job,
                root,
                base: given,
                options: cx.editor.options.file_picker.into(),
            });
            Ok(())
        },
    )
    .complete_args(ArgCompletion::Path);
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

    r.register(
        "register_picker",
        "List the registers; choosing one pastes it",
        |cx| {
            let items: Vec<PickerItem> = cx
                .editor
                .register_entries()
                .into_iter()
                .map(|(name, value)| PickerItem {
                    label: format!("\"{name}  {}", register_preview(&value.text)),
                    detail: register_detail(&value),
                    action: PickerAction::Paste(name),
                })
                .collect();
            if items.is_empty() {
                return Err("All registers are empty".into());
            }
            cx.editor
                .open_picker(Picker::new("Registers", items).keep_label_start());
            Ok(())
        },
    );
    r.alias("registers", "register_picker");
    r.alias("reg", "register_picker");

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
                if id != cx.editor.document.id() {
                    cx.editor.push_jump();
                }
                cx.editor.switch_to(id);
                Ok(())
            }
            Some(PickerAction::OpenFile(path)) => cx.editor.open(&path).map_err(|e| e.to_string()),
            Some(PickerAction::RunCommand(name)) => {
                cx.registry.execute(&name, cx.editor, Invocation::default())
            }
            Some(PickerAction::Paste(register)) => {
                let invocation = Invocation {
                    register: Some(register),
                    ..Invocation::default()
                };
                cx.registry.execute("paste_after", cx.editor, invocation)
            }
            None => Ok(()),
        },
    );
}

/// Longest register preview shown, in characters.
const PREVIEW_CHARS: usize = 60;

/// One line of a register's text: line breaks shown as `⏎`, tabs as
/// spaces, cut short with `…`.
fn register_preview(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| match c {
            '\n' => '⏎',
            '\t' => ' ',
            c if c.is_control() => '?',
            c => c,
        })
        .collect();
    if flat.chars().count() > PREVIEW_CHARS {
        flat.chars().take(PREVIEW_CHARS - 1).chain(['…']).collect()
    } else {
        flat
    }
}

/// `3 lines` for linewise text; nothing for a piece of a line.
fn register_detail(value: &crate::editor::Register) -> String {
    if value.blockwise {
        return "block".into();
    }
    if !value.linewise {
        return String::new();
    }
    match value.text.lines().count().max(1) {
        1 => "1 line".into(),
        n => format!("{n} lines"),
    }
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

    #[test]
    fn register_previews_fit_on_one_line() {
        use super::{PREVIEW_CHARS, register_preview};
        assert_eq!(register_preview("a\n\tb"), "a⏎ b");
        let long = "x".repeat(100);
        let preview = register_preview(&long);
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn register_picker_lists_filled_registers_and_pastes_the_choice() {
        use crate::editor::Register;
        use crate::editor::registers::StoreKind;
        let mut e = Editor::with_document(crate::editor::Document::with_text("x"));
        assert_eq!(
            crate::commands::CommandRegistry::with_builtins()
                .execute("register_picker", &mut e, Default::default())
                .unwrap_err(),
            "All registers are empty"
        );
        e.registers
            .store(Some('b'), Register::linewise("one\ntwo"), StoreKind::Yank)
            .unwrap();
        run(&mut e, "register_picker", None);
        assert_eq!(labels(&e), ["\"\"  one⏎two", "\"b  one⏎two"]);
        assert_eq!(
            e.picker().unwrap().selected_item().unwrap().detail,
            "2 lines"
        );

        type_text(&mut e, "b");
        run(&mut e, "picker_accept", None);
        assert_eq!(e.mode(), Mode::Normal);
        assert_eq!(e.document.buffer().text(), "x\none\ntwo");
    }
}
