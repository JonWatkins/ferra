//! Terminal side effects that ratatui doesn't cover.

use std::io::{self, stdout};

use crossterm::{
    cursor::SetCursorStyle,
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
};

use crate::config::CursorShape;

pub fn set_cursor_shape(shape: CursorShape) -> io::Result<()> {
    let style = match shape {
        CursorShape::Block => SetCursorStyle::SteadyBlock,
        CursorShape::Bar => SetCursorStyle::SteadyBar,
        CursorShape::Underline => SetCursorStyle::SteadyUnderScore,
    };
    execute!(stdout(), style)
}

pub fn restore_cursor_shape() -> io::Result<()> {
    execute!(stdout(), SetCursorStyle::DefaultUserShape)
}

/// Asks the terminal to deliver pastes as one event rather than as typed
/// keys, so pasted text isn't run through auto-indent or key bindings.
pub fn enable_bracketed_paste() -> io::Result<()> {
    execute!(stdout(), EnableBracketedPaste)
}

pub fn disable_bracketed_paste() -> io::Result<()> {
    execute!(stdout(), DisableBracketedPaste)
}
