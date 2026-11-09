//! Terminal side effects that ratatui doesn't cover.

use std::io::{self, Write, stdout};

use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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

/// Asks the terminal to report clicks, drags and the wheel (`mouse`).
pub fn set_mouse_capture(on: bool) -> io::Result<()> {
    if on {
        execute!(stdout(), EnableMouseCapture)
    } else {
        execute!(stdout(), DisableMouseCapture)
    }
}

/// Sets the mouse pointer's shape (`"pointer"`, `"text"`, `"ew-resize"`...,
/// CSS names) with OSC 22. kitty, Ghostty, WezTerm and foot take CSS names;
/// iTerm2 only takes X11 ones (and only from 3.7 while the mouse is
/// reported), so it gets those. Other terminals ignore it.
pub fn set_pointer_shape(shape: &str) -> io::Result<()> {
    let name = if is_iterm2() {
        x11_pointer(shape)
    } else {
        shape
    };
    let mut out = stdout();
    write!(out, "\x1b]22;{name}\x1b\\")?;
    out.flush()
}

/// Whether this is iTerm2 (`LC_TERMINAL` survives ssh and tmux).
fn is_iterm2() -> bool {
    let is = |var: &str, value: &str| std::env::var(var).is_ok_and(|v| v == value);
    is("TERM_PROGRAM", "iTerm.app") || is("LC_TERMINAL", "iTerm2")
}

/// The X11 cursor name for a CSS one.
pub fn x11_pointer(css: &str) -> &'static str {
    match css {
        "pointer" => "hand2",
        "text" => "xterm",
        "ew-resize" => "sb_h_double_arrow",
        "ns-resize" => "sb_v_double_arrow",
        _ => "left_ptr",
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn css_pointer_names_have_x11_ones_for_iterm2() {
        assert_eq!(super::x11_pointer("pointer"), "hand2");
        assert_eq!(super::x11_pointer("text"), "xterm");
        assert_eq!(super::x11_pointer("ns-resize"), "sb_v_double_arrow");
        assert_eq!(super::x11_pointer("default"), "left_ptr");
    }
}
