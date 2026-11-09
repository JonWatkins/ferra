//! Moving about and closing a plugin's popup (`editor/popup.rs`), bound in
//! `[keys.popup]`. The popup's own keys (from the plugin) come first.

use crate::commands::{CommandRegistry, Context, Invocation};

/// How far a move goes, from its count.
type Delta = fn(&mut Context) -> isize;

pub fn register(r: &mut CommandRegistry) {
    let moves: [(&str, &str, Delta); 4] = [
        (
            "popup_down",
            "Popup: the next line (or scroll down)",
            |cx| cx.count() as isize,
        ),
        (
            "popup_up",
            "Popup: the previous line (or scroll up)",
            |cx| -(cx.count() as isize),
        ),
        ("popup_page_down", "Popup: half a page down", |_| 10),
        ("popup_page_up", "Popup: half a page up", |_| -10),
    ];
    for (name, description, delta) in moves {
        r.register(name, description, move |cx| {
            let delta = delta(cx);
            if let Some(popup) = cx.editor.popup_mut() {
                popup.move_by(delta);
            }
            Ok(())
        });
    }
    r.register("popup_first", "Popup: the first line", |cx| {
        if let Some(popup) = cx.editor.popup_mut() {
            popup.to_end(false);
        }
        Ok(())
    });
    r.register("popup_last", "Popup: the last line", |cx| {
        if let Some(popup) = cx.editor.popup_mut() {
            popup.to_end(true);
        }
        Ok(())
    });
    r.register(
        "popup_close",
        "Close the popup (running its plugin's on_close command)",
        |cx| match cx.editor.close_popup() {
            Some(command) => cx
                .registry
                .execute(&command, cx.editor, Invocation::default()),
            None => Ok(()),
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::commands::{CommandRegistry, Invocation};
    use crate::editor::popup::{Popup, PopupSpan};
    use crate::editor::{Editor, Mode};

    #[test]
    fn moves_and_closes_running_on_close() {
        let mut registry = CommandRegistry::with_builtins();
        registry.register("said_bye", "", |cx| {
            cx.editor.document.insert_str("bye");
            Ok(())
        });
        let mut editor = Editor::default();
        editor.open_popup(Popup {
            lines: vec![vec![PopupSpan::default()]; 5],
            select: true,
            focus: true,
            on_close: Some("said_bye".into()),
            ..Popup::default()
        });
        let run = |editor: &mut Editor, name: &str, count: Option<usize>| {
            let invocation = Invocation {
                count,
                ..Invocation::default()
            };
            registry.execute(name, editor, invocation).unwrap();
        };
        run(&mut editor, "popup_down", Some(3));
        assert_eq!(editor.popup().unwrap().selected(), Some(3));
        run(&mut editor, "popup_first", None);
        assert_eq!(editor.popup().unwrap().selected(), Some(0));
        run(&mut editor, "popup_close", None);
        assert!(editor.popup().is_none());
        assert_eq!(editor.mode(), Mode::Normal);
        assert_eq!(editor.document.buffer().text(), "bye");
    }
}
