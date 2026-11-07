//! Terminal side effects that ratatui doesn't cover.

use std::io::{self, stdout};

use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
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

/// Asks the terminal, if it supports the kitty keyboard protocol (kitty,
/// WezTerm, foot, Ghostty, Alacritty, recent iTerm2...), to report keys
/// unambiguously: Esc on its own rather than as the start of Alt+key, and
/// Ctrl-I, Ctrl-M and Ctrl-[ apart from Tab, Enter and Esc. Returns whether
/// it did, so it can be turned off again. Asking waits briefly for the
/// terminal's answer; must be in raw mode.
pub fn enable_keyboard_enhancement() -> io::Result<bool> {
    if !crossterm::terminal::supports_keyboard_enhancement()? {
        return Ok(false);
    }
    execute!(
        stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    Ok(true)
}

pub fn disable_keyboard_enhancement() -> io::Result<()> {
    execute!(stdout(), PopKeyboardEnhancementFlags)
}
