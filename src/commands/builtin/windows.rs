//! Window commands: splitting, moving between, resizing and closing windows.

use std::path::PathBuf;

use crate::commands::{ArgCompletion, CommandRegistry, CommandResult, Context};
use crate::editor::windows::{Direction, SplitDirection};

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "split_horizontal",
        "Split the window top and bottom, optionally opening [file]",
        |cx| split(cx, SplitDirection::Horizontal),
    )
    .complete_args(ArgCompletion::Path);
    r.alias("sp", "split_horizontal");
    r.alias("split", "split_horizontal");

    r.register(
        "split_vertical",
        "Split the window side by side, optionally opening [file]",
        |cx| split(cx, SplitDirection::Vertical),
    )
    .complete_args(ArgCompletion::Path);
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

    r.register(
        "window_taller",
        "Make the window [count] lines taller (<C-w>+)",
        |cx| grow(cx, SplitDirection::Horizontal, true),
    );
    r.register(
        "window_shorter",
        "Make the window [count] lines shorter (<C-w>-)",
        |cx| grow(cx, SplitDirection::Horizontal, false),
    );
    r.register(
        "window_wider",
        "Make the window [count] columns wider (<C-w>>)",
        |cx| grow(cx, SplitDirection::Vertical, true),
    );
    r.register(
        "window_narrower",
        "Make the window [count] columns narrower (<C-w><)",
        |cx| grow(cx, SplitDirection::Vertical, false),
    );
    r.register(
        "window_max_height",
        "Make the window as tall as possible, or [count] lines (<C-w>_)",
        |cx| set_size(cx, SplitDirection::Horizontal),
    );
    r.register(
        "window_max_width",
        "Make the window as wide as possible, or [count] columns (<C-w>|)",
        |cx| set_size(cx, SplitDirection::Vertical),
    );
    r.register(
        "window_exchange",
        "Swap the window with the next one in its row or column (<C-w>x)",
        |cx| cx.editor.exchange_window(),
    );
    r.register(
        "window_rotate_down",
        "Rotate the windows in this row or column down/right (<C-w>r)",
        |cx| cx.editor.rotate_windows(true),
    );
    r.register(
        "window_rotate_up",
        "Rotate the windows in this row or column up/left (<C-w>R)",
        |cx| cx.editor.rotate_windows(false),
    );
    for (name, description, edge) in [
        (
            "window_move_far_left",
            "Move the window to the far left, full height (<C-w>H)",
            Direction::Left,
        ),
        (
            "window_move_far_right",
            "Move the window to the far right, full height (<C-w>L)",
            Direction::Right,
        ),
        (
            "window_move_top",
            "Move the window to the top, full width (<C-w>K)",
            Direction::Up,
        ),
        (
            "window_move_bottom",
            "Move the window to the bottom, full width (<C-w>J)",
            Direction::Down,
        ),
    ] {
        r.register(name, description, move |cx| {
            cx.editor.move_window_to_edge(edge);
            Ok(())
        });
    }
    r.register(
        "window_equalize",
        "Make every window the same size (<C-w>=)",
        |cx| {
            cx.editor.equalize_windows();
            Ok(())
        },
    );
    r.register(
        "resize",
        "Set the window's height: [N] lines, +N/-N more or fewer, or as tall as possible",
        |cx| {
            resize(
                cx,
                SplitDirection::Horizontal,
                cx.args.first().map(String::as_str),
            )
        },
    );
    r.alias("res", "resize");
    r.register(
        "vertical",
        "Run the next command for widths/side by side: vertical resize [N], vertical split",
        |cx| {
            let (command, rest) = cx.args.split_first().ok_or("Usage: vertical resize [N]")?;
            match command.as_str() {
                "resize" | "res" => resize(
                    cx,
                    SplitDirection::Vertical,
                    rest.first().map(String::as_str),
                ),
                "split" | "sp" | "new" => {
                    cx.editor.split_window(SplitDirection::Vertical);
                    Ok(())
                }
                other => Err(format!("Not supported after :vertical: {other}")),
            }
        },
    );
    r.alias("vert", "vertical");
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

/// `<C-w>+ - > <`: grows or shrinks by the count.
fn grow(cx: &mut Context, axis: SplitDirection, bigger: bool) -> CommandResult {
    let n = u16::try_from(cx.count()).unwrap_or(u16::MAX);
    cx.editor.resize_window(axis, |current, _| {
        if bigger {
            current.saturating_add(n)
        } else {
            current.saturating_sub(n)
        }
    });
    Ok(())
}

/// `<C-w>_` / `<C-w>|`: the count as the size, or as big as possible.
fn set_size(cx: &mut Context, axis: SplitDirection) -> CommandResult {
    let count = cx.count.map(|n| u16::try_from(n).unwrap_or(u16::MAX));
    cx.editor
        .resize_window(axis, |_, most| count.unwrap_or(most));
    Ok(())
}

/// `:resize [N | +N | -N]`, and `:vertical resize`.
fn resize(cx: &mut Context, axis: SplitDirection, arg: Option<&str>) -> CommandResult {
    let parse = |digits: &str| {
        digits
            .parse::<u16>()
            .map_err(|_| format!("Invalid size: {}", arg.unwrap_or_default()))
    };
    match arg {
        None => cx.editor.resize_window(axis, |_, most| most),
        Some(arg) => {
            if let Some(n) = arg.strip_prefix('+') {
                let n = parse(n)?;
                cx.editor
                    .resize_window(axis, |current, _| current.saturating_add(n))
            } else if let Some(n) = arg.strip_prefix('-') {
                let n = parse(n)?;
                cx.editor
                    .resize_window(axis, |current, _| current.saturating_sub(n))
            } else {
                let n = parse(arg)?;
                cx.editor.resize_window(axis, |_, _| n)
            }
        }
    };
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
