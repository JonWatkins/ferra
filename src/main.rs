use std::process::ExitCode;

use ferra::cli::{self, CliAction};
use ferra::{App, Config, editor::Message};

fn main() -> color_eyre::Result<ExitCode> {
    color_eyre::install()?;

    let files = match cli::parse(std::env::args_os().skip(1)) {
        Ok(CliAction::Edit { files }) => files,
        Ok(CliAction::PrintHelp) => {
            println!("A modal terminal text editor.\n\n{}", cli::USAGE);
            return Ok(ExitCode::SUCCESS);
        }
        Ok(CliAction::PrintVersion) => {
            println!("ferra {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::SUCCESS);
        }
        Err(error) => {
            eprintln!("ferra: {error}\n\n{}", cli::USAGE);
            return Ok(ExitCode::from(2));
        }
    };

    // A broken config shouldn't stop the editor from starting: fall back to
    // the defaults and tell the user what went wrong.
    let (config, config_error) = match Config::load() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error)),
    };

    // From here on, panics go to the log (`:log`).
    ferra::log::init();
    ferra::log::debug(
        "INFO",
        &format!("ferra {} started", env!("CARGO_PKG_VERSION")),
    );

    let mut app = App::new(config);
    // Where the settings editor saves.
    app.editor_mut()
        .set_config_file(ferra::config::default_path());
    if let Some(paths) = ferra::plugins::packages::PluginPaths::standard() {
        app.set_plugin_paths(paths);
    }
    // Plugins load before files open, so their `opened` hooks see them.
    let plugin_error = ferra::config::config_dir().and_then(|dir| app.load_plugins(&dir));
    app.open_all(&files);
    if let Some(error) = plugin_error {
        app.editor_mut().set_message(Message::error(error));
    }
    // Shown last so it isn't hidden by the file's "opened" message.
    if let Some(error) = config_error {
        app.editor_mut()
            .set_message(Message::error(error.to_string()));
    }

    ratatui::run(|terminal| app.run(terminal))?;
    Ok(ExitCode::SUCCESS)
}
