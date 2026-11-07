use crate::commands::CommandRegistry;
use crate::editor::Message;

pub fn register(r: &mut CommandRegistry) {
    r.register("undo", "Undo [count] changes", |cx| {
        for _ in 0..cx.count() {
            if !cx.editor.document.undo() {
                cx.editor
                    .set_message(Message::info("Already at oldest change"));
                break;
            }
        }
        // Show what changed, even in a closed fold.
        cx.editor.reveal_cursor();
        Ok(())
    });
    r.register(
        "repeat_last_change",
        "Repeat the last change; [count] replaces its count (.)",
        |cx| {
            cx.editor.request_repeat(cx.count);
            Ok(())
        },
    );
    r.register("redo", "Redo [count] changes", |cx| {
        for _ in 0..cx.count() {
            if !cx.editor.document.redo() {
                cx.editor
                    .set_message(Message::info("Already at newest change"));
                break;
            }
        }
        // Show what changed, even in a closed fold.
        cx.editor.reveal_cursor();
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run};
    use crate::core::Position;
    use crate::editor::Mode;

    #[test]
    fn undo_and_redo_with_counts() {
        let mut e = editor("abc", Position::new(0, 0), Mode::Normal);
        run(&mut e, "delete_char", None);
        run(&mut e, "delete_char", None);
        assert_eq!(e.document.buffer().text(), "c");

        run(&mut e, "undo", Some(2));
        assert_eq!(e.document.buffer().text(), "abc");
        run(&mut e, "redo", None);
        assert_eq!(e.document.buffer().text(), "bc");
    }

    #[test]
    fn reports_when_nothing_to_undo() {
        let mut e = editor("abc", Position::new(0, 0), Mode::Normal);
        run(&mut e, "undo", None);
        assert_eq!(e.message().unwrap().text, "Already at oldest change");
        run(&mut e, "redo", None);
        assert_eq!(e.message().unwrap().text, "Already at newest change");
    }
}
