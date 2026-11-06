//! The jump list (`<C-o>`, `<C-i>`) and change list (`g;`, `g,`).

use crate::commands::CommandRegistry;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "jump_back",
        "Go back to where you jumped from ([count] jumps, <C-o>)",
        |cx| {
            let n = cx.count();
            if cx.editor.jump_back(n) {
                Ok(())
            } else {
                Err("At the start of the jump list".into())
            }
        },
    );
    r.register(
        "jump_forward",
        "Go forward again in the jump list (<C-i>, <Tab>)",
        |cx| {
            let n = cx.count();
            if cx.editor.jump_forward(n) {
                Ok(())
            } else {
                Err("At the end of the jump list".into())
            }
        },
    );
    r.register(
        "goto_older_change",
        "Go to an earlier change position (g;)",
        |cx| {
            let n = cx.count();
            let position = cx
                .editor
                .document
                .step_change(true, n)
                .ok_or("At the start of the change list")?;
            cx.editor.document.set_cursor(position);
            Ok(())
        },
    );
    r.register(
        "goto_newer_change",
        "Go to a later change position (g,)",
        |cx| {
            let n = cx.count();
            let position = cx
                .editor
                .document
                .step_change(false, n)
                .ok_or("At the end of the change list")?;
            cx.editor.document.set_cursor(position);
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::run;
    use crate::core::Position;
    use crate::editor::{Document, Editor};

    #[test]
    fn jump_back_and_forward_with_messages_at_the_ends() {
        let mut e = Editor::with_document(Document::with_text("a\nb\nc"));
        run(&mut e, "jump_back", None);
        assert_eq!(e.message().unwrap().text, "At the start of the jump list");
        run(&mut e, "goto_file_end", None);
        assert_eq!(e.document.cursor().row, 2);
        run(&mut e, "jump_back", None);
        assert_eq!(e.document.cursor().row, 0);
        run(&mut e, "jump_forward", None);
        assert_eq!(e.document.cursor().row, 2);
        run(&mut e, "jump_forward", None);
        assert_eq!(e.message().unwrap().text, "At the end of the jump list");
    }

    #[test]
    fn change_list_navigation() {
        let mut e = Editor::with_document(Document::with_text("a\nb\nc"));
        for row in [0, 2] {
            e.document.set_cursor(Position::new(row, 0));
            e.document.insert_str("x");
        }
        e.document.set_cursor(Position::new(1, 0));
        run(&mut e, "goto_older_change", None);
        assert_eq!(e.document.cursor().row, 2);
        run(&mut e, "goto_older_change", None);
        assert_eq!(e.document.cursor().row, 0);
        run(&mut e, "goto_newer_change", None);
        assert_eq!(e.document.cursor().row, 2);
    }
}
