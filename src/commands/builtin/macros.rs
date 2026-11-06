//! Macros: `q{register}` records keys, `q` stops, `@{register}` plays them.
//!
//! The commands only ask; the app records and replays keys, since only it
//! sees them (see `app/macros.rs`).

use crate::commands::CommandRegistry;
use crate::editor::MacroRequest;

/// Registers a macro can be recorded into (uppercase appends).
fn valid_register(c: char) -> bool {
    c.is_ascii_alphabetic() || c.is_ascii_digit()
}

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "macro_record",
        "Record a macro into {register} (q), or stop recording",
        |cx| {
            if cx.editor.recording_macro().is_some() {
                cx.editor.request_macro(MacroRequest::StopRecording);
            } else {
                cx.editor.request_char("macro_start");
            }
            Ok(())
        },
    );

    r.register(
        "macro_start",
        "Start recording a macro into {register}",
        |cx| {
            let register = cx.char_arg.ok_or("missing register")?;
            if !valid_register(register) {
                return Err(format!("Invalid register for a macro: {register}"));
            }
            cx.editor
                .request_macro(MacroRequest::StartRecording(register));
            Ok(())
        },
    )
    .takes_char();

    r.register(
        "macro_play",
        "Play the macro in {register} [count] times (@); @@ repeats, @: reruns the last :",
        |cx| {
            let register = cx.char_arg.ok_or("missing register")?;
            if !(valid_register(register) || register == '@' || register == ':') {
                return Err(format!("Invalid register for a macro: {register}"));
            }
            let count = cx.count();
            cx.editor
                .request_macro(MacroRequest::Play { register, count });
            Ok(())
        },
    )
    .takes_char();
}
