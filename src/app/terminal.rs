//! Terminal side effects that ratatui doesn't cover.

use std::io::{self, stdout};

use crossterm::{
    cursor::SetCursorStyle,
    event::{DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange},
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

/// Asks the terminal to report focus changes, so the editor can check for
/// files changed by other programs when it comes back into focus. (tmux
/// needs `set -g focus-events on` to pass these through.)
pub fn enable_focus_reporting() -> io::Result<()> {
    execute!(stdout(), EnableFocusChange)
}

pub fn disable_focus_reporting() -> io::Result<()> {
    execute!(stdout(), DisableFocusChange)
}
