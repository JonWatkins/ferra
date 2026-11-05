use std::process::ExitCode;

use terminal_editor::cli::{self, CliAction};
use terminal_editor::{App, Config, editor::Message};

fn main() -> color_eyre::Result<ExitCode> {
    color_eyre::install()?;

    let files = match cli::parse(std::env::args_os().skip(1)) {
        Ok(CliAction::Edit { files }) => files,
        Ok(CliAction::PrintHelp) => {
            println!("A modal terminal text editor.\n\n{}", cli::USAGE);
            return Ok(ExitCode::SUCCESS);
        }
        Ok(CliAction::PrintVersion) => {
            println!("terminal-editor {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::SUCCESS);
        }
        Err(error) => {
            eprintln!("terminal-editor: {error}\n\n{}", cli::USAGE);
            return Ok(ExitCode::from(2));
        }
    };

    // A broken config shouldn't stop the editor from starting: fall back to
    // the defaults and tell the user what went wrong.
    let (config, config_error) = match Config::load() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error)),
    };

    let mut app = App::new(config);
    app.open_all(&files);
    // Shown last so it isn't hidden by the file's "opened" message.
    if let Some(error) = config_error {
        app.editor_mut()
            .set_message(Message::error(error.to_string()));
    }

    ratatui::run(|terminal| app.run(terminal))?;
    Ok(ExitCode::SUCCESS)
}
