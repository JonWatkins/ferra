use terminal_editor::{App, Config, editor::Message};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    // A broken config shouldn't stop the editor from starting: fall back to
    // the defaults and tell the user what went wrong.
    let (config, error) = match Config::load() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error)),
    };

    let mut app = App::new(config);
    if let Some(error) = error {
        app.editor_mut()
            .set_message(Message::error(error.to_string()));
    }

    ratatui::run(|terminal| app.run(terminal))?;
    Ok(())
}
