//! Managing plugin packages: `:plugin_install`, `:plugin_update`,
//! `:plugin_reload`, `:plugin_list` and `:plugin_remove` (the file and git
//! work is in `plugins/packages.rs`).
//!
//! What changes your setup asks first, in a popup: installing shows where
//! the code comes from and that it runs with full access; updating shows
//! the new commits. On `y` the work is handed to the app as a
//! [`PackageRequest`] (`take_plugin_request`), which runs it on a thread
//! and, when it's done, records it and loads the plugins into the Lua host
//! commands can't reach. Reloading is asked of the app the same way.

use super::picker::{Picker, PickerAction, PickerItem};
use super::popup::{Popup, PopupLine, PopupSpan};
use super::{Editor, Message};
use crate::plugins::packages::{self, Checked, Incoming, Index, PluginPaths, Source};

#[derive(Debug, Default)]
pub struct PackageState {
    /// Where packages and their records go; `None` when there's no home
    /// folder (managing plugins is then refused).
    paths: Option<PluginPaths>,
    /// The job the open popup is asking about.
    asking: Option<PackageJob>,
    /// A job confirmed, waiting for the app to start it.
    request: Option<PackageJob>,
    /// What the running job is doing (`Installing x`), if one is.
    running: Option<String>,
    /// `:plugin_reload` (or a change that needs one), for the app, and
    /// whether to say it's done.
    reload: Option<bool>,
    /// Packs offered this session (each is offered once).
    offered: Vec<String>,
    /// A file to offer a pack for once the directory's been fetched.
    offer_after_refresh: Option<std::path::PathBuf>,
    /// The directory was fetched (or tried) this session.
    refreshed: bool,
    /// The pack the open offer is about, for `d`.
    declining: Option<String>,
}

/// Work on plugin packages, for the app to run on a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageJob {
    /// Clone a new plugin. `taken` are the names in use.
    Install { source: Source, taken: Vec<String> },
    /// Fetch these plugins (all, if empty) and list their new commits.
    Check(Vec<String>),
    /// Move plugins to the commits a check found.
    Update(Vec<Incoming>),
    /// Clone these plugins from `plugins.toml`, at their locked commits.
    Restore(Vec<String>),
    /// Fetch the plugin directory's index.
    Browse(Source),
    /// Fetch it quietly, to offer language packs from.
    Refresh(Source),
}

#[derive(Debug, Clone)]
pub struct PackageRequest {
    pub paths: PluginPaths,
    pub job: PackageJob,
}

fn span(text: &str, style: Option<&str>) -> PopupSpan {
    PopupSpan {
        text: text.to_string(),
        style: style.map(str::to_string),
    }
}

fn line(text: &str) -> PopupLine {
    vec![span(text, None)]
}

/// `y` and `n` with what they do.
fn answers(yes: &str) -> PopupLine {
    vec![
        span("y", Some("ui.popup.title")),
        span(&format!(" {yes}   "), None),
        span("n", Some("ui.popup.title")),
        span(" cancel", None),
    ]
}

const POPUPS: [&str; 4] = [
    "plugin-install",
    "plugin-update",
    "plugin-restore",
    "plugin-suggest",
];

/// How old the copy of the plugin directory can be before it's fetched
/// again for offering language packs.
const INDEX_FRESH: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

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

    /// Fails while a job is running: one at a time.
    fn require_idle(&self) -> Result<(), String> {
        match &self.packages.running {
            Some(doing) => Err(format!("Still {}…", lowercase_first(doing))),
            None => Ok(()),
        }
    }

    /// Opens a yes/no popup about `job`.
    fn ask(&mut self, name: &str, title: &str, lines: Vec<PopupLine>, job: PackageJob) {
        self.packages.asking = Some(job);
        self.open_popup(Popup {
            name: name.into(),
            title: title.into(),
            lines,
            keys: vec![
                ("y".into(), "plugin_confirmed".into()),
                ("n".into(), "popup_close".into()),
            ],
            focus: true,
            ..Popup::default()
        });
    }

    /// `:plugin_install owner/repo[@tag]`: asks before installing. With no
    /// source, offers to restore the plugins in `plugins.toml` that aren't
    /// cloned.
    pub fn plugin_ask_install(&mut self, spec: &str) -> Result<(), String> {
        let paths = self.require_plugin_paths()?;
        self.require_idle()?;
        if spec.trim().is_empty() {
            return self.plugin_ask_restore(&paths);
        }
        let source = self.by_name_in_directory(&paths, spec.trim())?;
        let wanted = packages::wanted(&paths)?;
        if let Some((name, _)) = wanted.iter().find(|(_, w)| source.same_as(w)) {
            let what = match &source.path {
                Some(path) => format!("{}//{path}", source.spec),
                None => source.spec.clone(),
            };
            return Err(format!("{what} is already installed as {name}"));
        }
        let url = source.url();
        let mut lines = vec![vec![
            span("Install the plugin from ", None),
            span(&url, Some("ui.popup.title")),
        ]];
        if let Some(path) = &source.path {
            lines.push(vec![
                span("folder ", None),
                span(path, Some("ui.popup.title")),
            ]);
        }
        if let Some(tag) = &source.tag {
            lines.push(vec![span("at ", None), span(tag, Some("ui.popup.title"))]);
        }
        lines.extend(trust_warning());
        lines.push(answers("install"));
        let taken = wanted.into_keys().collect();
        self.ask(
            "plugin-install",
            "Install plugin",
            lines,
            PackageJob::Install { source, taken },
        );
        Ok(())
    }

    /// `spec` as a source: a plugin directory entry's name (`zig`), else
    /// a repository (`owner/repo`, a URL or a path).
    fn by_name_in_directory(&self, paths: &PluginPaths, spec: &str) -> Result<Source, String> {
        if spec.contains(['/', ':', '@']) || packages::check_name(spec).is_err() {
            return Source::parse(spec);
        }
        let directory =
            Source::parse(&self.options.plugin_index).map_err(|e| format!("plugin-index: {e}"))?;
        match packages::find_in_index(paths, &directory, spec) {
            Some(entry) => Source::parse(&entry.source(&directory)),
            None => Err(format!(
                "No plugin named {spec} in the plugin directory (:plugin_browse lists them)"
            )),
        }
    }

    /// After a file opens: if ferra has no language for it, offers the
    /// plugin directory's language pack for it (fetching the directory in
    /// the background first, when there's no recent copy).
    pub fn plugin_saw(&mut self, event: &crate::events::Event) {
        let crate::events::Event::Opened { path } = event else {
            return;
        };
        if !self.options.plugin_suggestions || self.language_of(&self.document).is_some() {
            return;
        }
        if self.document.path().is_none_or(|p| p != path.as_path()) {
            return;
        }
        let Some(paths) = self.packages.paths.clone() else {
            return;
        };
        let Ok(directory) = Source::parse(&self.options.plugin_index) else {
            return;
        };
        let cached = packages::cached_index(&paths, &directory);
        let fresh = cached.as_ref().is_some_and(|(_, age)| *age < INDEX_FRESH);
        if !fresh && !self.packages.refreshed && self.packages.running.is_none() {
            self.packages.refreshed = true;
            self.packages.offer_after_refresh = Some(path.clone());
            self.start_plugin_job(PackageJob::Refresh(directory.clone()));
        }
        if let Some((entries, _)) = cached {
            self.offer_language_pack(&paths, &directory, &entries, path);
        }
    }

    /// The directory was fetched quietly: offers a pack for the file that
    /// asked, if it's still the one shown.
    pub fn plugin_index_refreshed(&mut self, index: Option<packages::Index>) {
        let Some(path) = self.packages.offer_after_refresh.take() else {
            return;
        };
        let (Some(index), Some(paths)) = (index, self.packages.paths.clone()) else {
            return;
        };
        let Ok(directory) = Source::parse(&self.options.plugin_index) else {
            return;
        };
        let shown = self.document.path().is_some_and(|p| p == path.as_path());
        if shown && self.language_of(&self.document).is_none() {
            self.offer_language_pack(&paths, &directory, &index.entries, &path);
        }
    }

    fn offer_language_pack(
        &mut self,
        paths: &PluginPaths,
        directory: &Source,
        entries: &[packages::IndexEntry],
        path: &std::path::Path,
    ) {
        if self.popup().is_some() || self.packages.running.is_some() {
            return;
        }
        let mut skip = packages::declined(paths);
        skip.extend(self.packages.offered.iter().cloned());
        if let Ok(wanted) = packages::wanted(paths) {
            skip.extend(wanted.into_keys());
        }
        let first_line = self.document.buffer().line(0);
        let Some(offer) = packages::suggest(paths, directory, entries, path, &first_line, &skip)
        else {
            return;
        };
        let Ok(source) = Source::parse(&offer.source) else {
            return;
        };
        self.packages.offered.push(offer.name.clone());
        let file = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let mut lines = vec![
            vec![
                span(&file, Some("ui.popup.title")),
                span(" is ", None),
                span(&offer.language, Some("ui.popup.title")),
                span(". The plugin directory's language pack for it:", None),
            ],
            vec![],
            vec![
                span(&format!("  {}  ", offer.name), Some("ui.popup.title")),
                span(&offer.description, None),
            ],
            vec![span(&format!("  {}", source), Some("comment"))],
        ];
        lines.extend(trust_warning());
        lines.push(vec![
            span("y", Some("ui.popup.title")),
            span(" install   ", None),
            span("n", Some("ui.popup.title")),
            span(" not now   ", None),
            span("d", Some("ui.popup.title")),
            span(&format!(" don't ask about {}", offer.name), None),
        ]);
        let taken = packages::wanted(paths)
            .map(|w| w.into_keys().collect())
            .unwrap_or_default();
        self.packages.declining = Some(offer.name.clone());
        self.ask(
            "plugin-suggest",
            &format!("Install {}?", offer.name),
            lines,
            PackageJob::Install { source, taken },
        );
        if let Some(popup) = self.popup_mut() {
            popup.keys.push(("d".into(), "plugin_decline".into()));
        }
    }

    /// `d` when a language pack is offered: don't offer it again.
    pub fn plugin_decline(&mut self) -> Result<(), String> {
        if self.popup().is_none_or(|p| p.name != "plugin-suggest") {
            return Err("Nothing to decline".into());
        }
        self.close_popup();
        self.packages.asking = None;
        let name = self.packages.declining.take().ok_or("Nothing to decline")?;
        let paths = self.require_plugin_paths()?;
        packages::decline(&paths, &name)?;
        self.set_message(Message::info(format!(
            "Won't offer {name} again (:plugin_install {name} installs it)"
        )));
        Ok(())
    }

    fn plugin_ask_restore(&mut self, paths: &PluginPaths) -> Result<(), String> {
        let missing = packages::missing(paths)?;
        if missing.is_empty() {
            return Err("Usage: :plugin_install owner/repo[@tag] (or a git URL)".into());
        }
        let wanted = packages::wanted(paths)?;
        let mut lines = vec![
            line("plugins.toml lists plugins that aren't installed:"),
            vec![],
        ];
        for name in &missing {
            let source = &wanted[name];
            let at = source
                .tag
                .as_deref()
                .map(|t| format!("@{t}"))
                .unwrap_or_default();
            lines.push(vec![
                span(&format!("  {name}  "), Some("ui.popup.title")),
                span(&format!("{}{at}", source.source), None),
            ]);
        }
        lines.extend(trust_warning());
        lines.push(answers("install them"));
        self.ask(
            "plugin-restore",
            "Install missing plugins",
            lines,
            PackageJob::Restore(missing),
        );
        Ok(())
    }

    /// `:plugin_update [name...]`: looks for new commits in the
    /// background, then asks (`plugin_show_updates`).
    pub fn plugin_check_updates(&mut self, names: &[String]) -> Result<(), String> {
        let paths = self.require_plugin_paths()?;
        self.require_idle()?;
        let wanted = packages::wanted(&paths)?;
        if wanted.is_empty() {
            return Err("No plugins installed (:plugin_install owner/repo)".into());
        }
        if let Some(name) = names.iter().find(|n| !wanted.contains_key(*n)) {
            return Err(format!("No plugin named {name} is installed"));
        }
        self.start_plugin_job(PackageJob::Check(names.to_vec()));
        Ok(())
    }

    /// What a check found: asks to update, or says there's nothing new.
    pub fn plugin_show_updates(&mut self, checked: Checked) {
        let notes = checked.skipped.join("; ");
        if checked.incoming.is_empty() {
            let text = match (checked.current.len(), notes.is_empty()) {
                (_, false) if checked.current.is_empty() => format!("Nothing to update: {notes}"),
                (_, false) => format!("The others are up to date; {notes}"),
                (1, true) => format!("{} is up to date", checked.current[0]),
                (_, true) => "Plugins are up to date".to_string(),
            };
            self.set_message(Message::info(text));
            return;
        }
        let mut lines = Vec::new();
        for incoming in &checked.incoming {
            let count = incoming.commits.len();
            lines.push(vec![
                span(&incoming.name, Some("ui.popup.title")),
                span(
                    &format!(
                        "  {} → {}  ({count} new commit{})",
                        short(&incoming.from),
                        short(&incoming.to),
                        if count == 1 { "" } else { "s" }
                    ),
                    Some("comment"),
                ),
            ]);
            for commit in &incoming.commits {
                lines.push(line(&format!("  {commit}")));
            }
            lines.push(vec![]);
        }
        for skipped in &checked.skipped {
            lines.push(vec![span(skipped, Some("comment"))]);
        }
        if !checked.skipped.is_empty() {
            lines.push(vec![]);
        }
        lines.push(line(
            "Read what changed: the new code runs with your permissions.",
        ));
        lines.push(answers("update"));
        self.ask(
            "plugin-update",
            "Update plugins",
            lines,
            PackageJob::Update(checked.incoming),
        );
    }

    /// `:plugin_browse`: fetches the plugin directory (the `plugin-index`
    /// option) in the background, then lists it (`plugin_show_index`).
    pub fn plugin_browse(&mut self) -> Result<(), String> {
        self.require_plugin_paths()?;
        self.require_idle()?;
        let source =
            Source::parse(&self.options.plugin_index).map_err(|e| format!("plugin-index: {e}"))?;
        self.start_plugin_job(PackageJob::Browse(source));
        Ok(())
    }

    /// The directory's plugins in the picker; choosing one installs it
    /// (asking first, as `:plugin_install` does).
    pub fn plugin_show_index(&mut self, index: Index) {
        let installed: Vec<(String, packages::Wanted)> = self
            .packages
            .paths
            .as_ref()
            .and_then(|p| packages::wanted(p).ok())
            .map(|w| w.into_iter().collect())
            .unwrap_or_default();
        let directory = Source::parse(&self.options.plugin_index).ok();
        if index.entries.is_empty() {
            self.set_message(Message::info("The plugin directory has no plugins yet"));
            return;
        }
        let items = index
            .entries
            .into_iter()
            .map(|entry| {
                let mut detail = entry.description.clone();
                if entry.language {
                    detail = format!("[language] {detail}");
                }
                if !entry.tags.is_empty() {
                    detail = format!("{detail}  #{}", entry.tags.join(" #"));
                }
                let source = match &directory {
                    Some(directory) => entry.source(directory),
                    None => entry.repo.clone().unwrap_or_default(),
                };
                let same =
                    |w: &packages::Wanted| Source::parse(&source).is_ok_and(|s| s.same_as(w));
                if installed
                    .iter()
                    .any(|(name, w)| *name == entry.name || same(w))
                {
                    detail = format!("(installed) {detail}");
                }
                PickerItem {
                    label: entry.name,
                    detail: detail.trim().to_string(),
                    action: PickerAction::RunWith("plugin_install".into(), vec![source]),
                }
            })
            .collect();
        self.open_picker(Picker::new("Plugins", items));
        self.clear_message();
        if let Some(why) = index.stale {
            self.set_message(Message::error(format!(
                "Couldn't update the plugin directory ({why}); showing the last copy"
            )));
        }
    }

    /// `y` in an install, update or restore popup: the job starts in the
    /// background.
    pub fn plugin_confirmed(&mut self) -> Result<(), String> {
        let job = self.packages.asking.take();
        // Only from the popup: once it's closed (`n`, `<Esc>`), the
        // question's gone.
        if self
            .popup()
            .is_none_or(|p| !POPUPS.contains(&p.name.as_str()))
        {
            return Err("Nothing to confirm".into());
        }
        self.close_popup();
        let job = job.ok_or("Nothing to confirm")?;
        self.start_plugin_job(job);
        Ok(())
    }

    fn start_plugin_job(&mut self, job: PackageJob) {
        let doing = match &job {
            PackageJob::Install { source, .. } => format!("Installing {source}"),
            PackageJob::Check(names) if names.is_empty() => "Checking plugins for updates".into(),
            PackageJob::Check(names) => format!("Checking {} for updates", names.join(", ")),
            PackageJob::Update(incoming) => format!(
                "Updating {}",
                incoming
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PackageJob::Restore(names) => format!("Installing {}", names.join(", ")),
            PackageJob::Browse(_) => "Getting the plugin directory".into(),
            PackageJob::Refresh(_) => "Updating the plugin directory".into(),
        };
        if !matches!(job, PackageJob::Refresh(_)) {
            self.set_message(Message::info(format!("{doing}…")));
        }
        self.packages.running = Some(doing);
        self.packages.request = Some(job);
    }

    /// A job confirmed, for the app to run.
    pub fn take_plugin_request(&mut self) -> Option<PackageRequest> {
        let job = self.packages.request.take()?;
        let paths = self.packages.paths.clone()?;
        Some(PackageRequest { paths, job })
    }

    /// The app finished the job (well or not).
    pub fn plugin_job_done(&mut self) {
        self.packages.running = None;
    }

    /// `:plugin_reload`: asks the app to load every plugin again, saying
    /// so when it's done if `announce`.
    pub fn request_plugin_reload(&mut self, announce: bool) {
        self.packages.reload = Some(announce);
    }

    pub fn take_plugin_reload(&mut self) -> Option<bool> {
        self.packages.reload.take()
    }

    /// `:plugin_remove name`. What it registered stays until ferra
    /// restarts.
    pub fn plugin_remove(&mut self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Usage: :plugin_remove name (:plugin_list shows them)".into());
        }
        let paths = self.require_plugin_paths()?;
        self.require_idle()?;
        packages::remove(&paths, name)?;
        // Loading the rest again drops what it registered.
        self.request_plugin_reload(false);
        self.set_message(Message::info(format!("Removed {name}")));
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
                line.push(span(&format!(" {}", short(commit)), Some("comment")));
            }
            if !plugin.languages.is_empty() {
                line.push(span(
                    &format!("  [{}]", plugin.languages.join(", ")),
                    Some("type"),
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

/// That a plugin runs with full access, for the install popups.
fn trust_warning() -> Vec<PopupLine> {
    vec![
        vec![],
        line("A plugin runs with your permissions: it can read and change"),
        line("your files and run programs. Only install code you trust."),
        vec![],
    ]
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect())
}
