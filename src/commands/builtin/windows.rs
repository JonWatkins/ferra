//! Window commands: splitting, moving between and closing windows.

use std::path::PathBuf;

use crate::commands::{CommandRegistry, CommandResult, Context};
use crate::editor::windows::{Direction, SplitDirection};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "split_horizontal",
        "Split the window top and bottom, optionally opening [file]",
        |cx| split(cx, SplitDirection::Horizontal),
    );
    r.alias("sp", "split_horizontal");
    r.alias("split", "split_horizontal");

    r.register(
        "split_vertical",
        "Split the window side by side, optionally opening [file]",
        |cx| split(cx, SplitDirection::Vertical),
    );
    r.alias("vs", "split_vertical");
    r.alias("vsplit", "split_vertical");

    r.register("window_next", "Move to the next window", |cx| {
        cx.editor.focus_next_window();
        Ok(())
    });
    for (name, description, direction) in [
        (
            "window_left",
            "Move to the window on the left",
            Direction::Left,
        ),
        (
            "window_right",
            "Move to the window on the right",
            Direction::Right,
        ),
        ("window_up", "Move to the window above", Direction::Up),
        ("window_down", "Move to the window below", Direction::Down),
    ] {
        r.register(name, description, move |cx| {
            cx.editor.focus_window_in(direction);
            Ok(())
        });
    }

    r.register(
        "window_close",
        "Close the window (its buffer stays open)",
        |cx| cx.editor.close_window(),
    );
    r.alias("close", "window_close");
    r.alias("clo", "window_close");

    r.register("window_only", "Close every other window", |cx| {
        cx.editor.only_window();
        Ok(())
    });
    r.alias("only", "window_only");
    r.alias("on", "window_only");
}

/// Splits, then opens the file argument (if any) in the new window.
fn split(cx: &mut Context, direction: SplitDirection) -> CommandResult {
    cx.editor.split_window(direction);
    if !cx.args.is_empty() {
        let path = PathBuf::from(cx.args.join(" "));
        cx.editor.open(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commands::builtin::test_util::{editor, run, run_with_args};
    use crate::core::Position;
    use crate::editor::{Mode, WindowId};

    #[test]
    fn split_close_and_only() {
        let mut e = editor("x", Position::default(), Mode::Normal);
        run(&mut e, "split_vertical", None);
        run(&mut e, "split_horizontal", None);
        assert_eq!(e.windows().count(), 3);
        run(&mut e, "window_close", None);
        assert_eq!(e.windows().count(), 2);
        run(&mut e, "window_only", None);
        assert_eq!(e.windows().count(), 1);
        run(&mut e, "window_close", None);
        assert_eq!(e.message().unwrap().text, "Cannot close the last window");
    }

    #[test]
    fn moving_between_windows() {
        let mut e = editor("x", Position::default(), Mode::Normal);
        run(&mut e, "split_vertical", None);
        run(&mut e, "window_left", None);
        assert_eq!(e.windows().active(), WindowId(0));
        run(&mut e, "window_next", None);
        assert_eq!(e.windows().active(), WindowId(1));
    }

    #[test]
    fn split_can_open_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.txt");
        std::fs::write(&path, "other\n").unwrap();
        let mut e = editor("first", Position::default(), Mode::Normal);
        run_with_args(&mut e, "split_vertical", &[path.to_str().unwrap()]);
        assert_eq!(e.document.buffer().text(), "other");
        let (left, _) = e.window_view(WindowId(0)).unwrap();
        assert_eq!(left.buffer().text(), "first");
    }

    #[test]
    fn quit_closes_windows_before_quitting() {
        let mut e = editor("x", Position::default(), Mode::Normal);
        e.document.insert_str("unsaved ");
        run(&mut e, "split_vertical", None);
        run(&mut e, "quit", None);
        assert_eq!(e.windows().count(), 1, "closed the window");
        assert!(!e.should_quit());
        run(&mut e, "quit", None);
        assert!(
            !e.should_quit(),
            "last window: unsaved changes still protected"
        );
        run(&mut e, "quit_all", None);
        assert!(!e.should_quit());
    }
}
