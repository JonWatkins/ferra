//! Installing and removing plugin packages (`:plugin_install`,
//! `:plugin_list`, `:plugin_remove`; the files are `plugins/packages.rs`).
//!
//! Installing asks first, in a popup showing where the code comes from and
//! that it will run with full access. On `y` the clone is handed to the app
//! (`take_plugin_install`), which runs it on a thread and, when it's done,
//! records it and loads it into the Lua host the commands can't reach.

use super::popup::{Popup, PopupSpan};
use super::{Editor, Message};
use crate::plugins::packages::{self, PluginPaths, Source};

#[derive(Debug, Default)]
pub struct PackageState {
    /// Where packages and their records go; `None` when there's no home
    /// folder (installing is then refused).
    paths: Option<PluginPaths>,
    /// The install the popup is asking about.
    asking: Option<Source>,
    /// An install confirmed, waiting for the app to start it.
    request: Option<Source>,
    /// The install running.
    installing: Option<Source>,
}

/// An install for the app to run.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub paths: PluginPaths,
    pub source: Source,
    /// Names already taken, from `plugins.toml`.
    pub taken: Vec<String>,
}

fn span(text: &str, style: Option<&str>) -> PopupSpan {
    PopupSpan {
        text: text.to_string(),
        style: style.map(str::to_string),
    }
}

impl Editor {
    pub fn set_plugin_paths(&mut self, paths: PluginPaths) {
        self.packages.paths = Some(paths);
    }

    pub fn plugin_paths(&self) -> Option<&PluginPaths> {
        self.packages.paths.as_ref()
    }

    fn require_plugin_paths(&self) -> Result<PluginPaths, String> {
        self.packages
            .paths
            .clone()
            .ok_or_else(|| "No folder for plugins (is HOME set?)".to_string())
    }

    /// `:plugin_install owner/repo[@tag]`: asks before installing.
    pub fn plugin_ask_install(&mut self, spec: &str) -> Result<(), String> {
        if spec.trim().is_empty() {
            return Err("Usage: :plugin_install owner/repo[@tag] (or a git URL)".into());
        }
        let source = Source::parse(spec)?;
        let paths = self.require_plugin_paths()?;
        if let Some(running) = &self.packages.installing {
            return Err(format!("Still installing {running}…"));
        }
        if let Some((name, _)) = packages::wanted(&paths)?
            .into_iter()
            .find(|(_, w)| w.source == source.spec)
        {
            return Err(format!("{} is already installed as {name}", source.spec));
        }
        let url = source.url();
        let mut lines = vec![vec![
            span("Install the plugin from ", None),
            span(&url, Some("ui.popup.title")),
        ]];
        if let Some(tag) = &source.tag {
            lines.push(vec![span("at ", None), span(tag, Some("ui.popup.title"))]);
        }
        lines.extend([
            vec![],
            vec![span(
                "A plugin runs with your permissions: it can read and change",
                None,
            )],
            vec![span(
                "your files and run programs. Only install code you trust.",
                None,
            )],
            vec![],
            vec![
                span("y", Some("ui.popup.title")),
                span(" install   ", None),
                span("n", Some("ui.popup.title")),
                span(" cancel", None),
            ],
        ]);
        self.packages.asking = Some(source);
        self.open_popup(Popup {
            name: "plugin-install".into(),
            title: "Install plugin".into(),
            lines,
            keys: vec![
                ("y".into(), "plugin_install_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..Popup::default()
        });
        Ok(())
    }

    /// `y` in that popup: the install starts in the background.
    pub fn plugin_install_confirmed(&mut self) -> Result<(), String> {
        let source = self.packages.asking.take();
        // Only from the popup: once it's closed (`n`, `<Esc>`), the
        // question's gone.
        if self.popup().is_none_or(|p| p.name != "plugin-install") {
            return Err("Nothing to install".into());
        }
        self.close_popup();
        let source = source.ok_or("Nothing to install")?;
        self.set_message(Message::info(format!("Installing {source}…")));
        self.packages.installing = Some(source.clone());
        self.packages.request = Some(source);
        Ok(())
    }

    /// The install confirmed, for the app to run.
    pub fn take_plugin_install(&mut self) -> Option<InstallRequest> {
        let source = self.packages.request.take()?;
        let paths = self.packages.paths.clone()?;
        let taken = packages::wanted(&paths)
            .map(|w| w.into_keys().collect())
            .unwrap_or_default();
        Some(InstallRequest {
            paths,
            source,
            taken,
        })
    }

    /// The app finished the install (well or not).
    pub fn plugin_install_done(&mut self) {
        self.packages.installing = None;
    }

    /// `:plugin_remove name`. What it registered stays until ferra
    /// restarts.
    pub fn plugin_remove(&mut self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Usage: :plugin_remove name (:plugin_list shows them)".into());
        }
        let paths = self.require_plugin_paths()?;
        packages::remove(&paths, name)?;
        self.set_message(Message::info(format!(
            "Removed {name}; its commands stay until ferra restarts"
        )));
        Ok(())
    }

    /// The names in `plugins.toml`, for completing `:plugin_remove`.
    pub fn plugin_names(&self) -> Vec<String> {
        self.packages
            .paths
            .as_ref()
            .and_then(|p| packages::wanted(p).ok())
            .map(|w| w.into_keys().collect())
            .unwrap_or_default()
    }

    /// `:plugin_list`: a popup of what's installed.
    pub fn plugin_list(&mut self) -> Result<(), String> {
        let paths = self.require_plugin_paths()?;
        let listed = packages::list(&paths)?;
        if listed.is_empty() {
            self.set_message(Message::info(
                "No plugins installed (:plugin_install owner/repo)",
            ));
            return Ok(());
        }
        let width = listed.iter().map(|p| p.name.len()).max().unwrap_or(0);
        let mut lines = Vec::new();
        for plugin in &listed {
            let mut line = vec![span(
                &format!("{:width$}", plugin.name),
                Some("ui.popup.title"),
            )];
            if let Some(version) = &plugin.version {
                line.push(span(&format!("  {version}"), None));
            }
            line.push(span(&format!("  {}", plugin.source), Some("comment")));
            if let Some(commit) = &plugin.commit {
                line.push(span(
                    &format!(" {}", &commit[..commit.len().min(7)]),
                    Some("comment"),
                ));
            }
            match &plugin.problem {
                Some(problem) => {
                    line.push(span(&format!("  ({problem})"), Some("ui.decoration.error")))
                }
                None if !plugin.description.is_empty() => {
                    line.push(span(&format!("  {}", plugin.description), None));
                }
                None => {}
            }
            lines.push(line);
        }
        self.open_popup(Popup {
            name: "plugin-list".into(),
            title: format!("Plugins ({})", listed.len()),
            lines,
            footer: format!(" {} ", paths.data.display()),
            focus: true,
            select: true,
            ..Popup::default()
        });
        Ok(())
    }
}
