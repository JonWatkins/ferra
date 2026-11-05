//! Terminal side effects that ratatui doesn't cover.

use std::io::{self, stdout};

use crossterm::{cursor::SetCursorStyle, execute};

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
