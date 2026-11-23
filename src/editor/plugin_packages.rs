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
//!
//! `:plugin_install` (`editor/plugin_packages.rs`) parses the source
//! (`owner/repo` is GitHub; URLs and paths pass through to git, with `@tag`
//! split off where it can't be part of the URL) and asks in a popup. `y` leaves
//! an `InstallRequest` on the editor, the way the file picker asks for a
//! listing, since commands can't reach the app's `PluginHost`; the app starts
//! it as a background job (`Update::PluginInstalled`). The job clones into a
//! hidden folder beside the others (git with prompts off and ssh in batch mode,
//! as for push), checks the manifest (name, `main` inside the plugin, the
//! `ferra` version range), refuses a name already taken, and only then renames
//! it into place, so a failed install leaves nothing. The app then records it
//! and loads it: `PluginHost::load_package` puts the plugin's `lua/` folder
//! first on `package.path`, so `require` finds its modules, and runs the main
//! file under the plugin's name. At startup `App::load_plugins` loads every
//! package in `plugins.toml` before `init.lua` and `plugins/*.lua`; one listed
//! but not cloned is reported.
//!
//! Offering language packs (`Editor::plugin_saw`, on each `Opened` event): if
//! ferra has no language for the file, `packages::suggest` reads the language
//! packs' manifests from the copy of the directory (`.index`, only entries kept
//! in the directory itself) and matches the file against their `[languages]`
//! with the same detection the config uses, skipping packs installed, declined
//! (`.declined` in the data folder) or offered this session. With no copy, or
//! one over a day old, a quiet `Refresh` job fetches it first and the offer
//! follows (`plugin_index_refreshed`). Events can open popups and ask for
//! background work now, so `dispatch_events` starts both after each round.
//! `:plugin_install name` looks a bare name up in the same copy.

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
    /// `:plugin_browse tag`: only the directory's plugins with that tag.
    browse_tag: Option<String>,
    /// Packs offered this session (each is offered once).
    offered: Vec<String>,
    /// A file to offer a pack for once the directory's been fetched.
    offer_after_refresh: Option<std::path::PathBuf>,
    /// The directory was fetched (or tried) this session.
    refreshed: bool,
    /// The pack the open offer is about, for `d`.
    declining: Option<String>,
    /// A C compiler's been offered this session.
    compiler_offered: bool,
    /// The command `y` runs to install one.
    compiler_command: Option<String>,
}

/// What `d` records when a C compiler is offered (a name no pack has).
const COMPILER_DECLINED: &str = ".c-compiler";

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

const POPUPS: [&str; 5] = [
    "plugin-compiler",
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

    /// A grammar couldn't be built for want of a C compiler (`grammar`
    /// names it): offers, once a session, to install one, by running the
    /// command for this system in the terminal panel.
    pub fn offer_compiler(&mut self, grammar: &str) {
        if self.packages.compiler_offered || self.popup().is_some() {
            return;
        }
        let Some(paths) = self.packages.paths.clone() else {
            return;
        };
        if packages::declined(&paths)
            .iter()
            .any(|n| n == COMPILER_DECLINED)
        {
            return;
        }
        let Some(install) = crate::plugins::grammars::compiler_install(
            std::env::consts::OS,
            &crate::plugins::grammars::os_release(),
        ) else {
            return;
        };
        self.packages.compiler_offered = true;
        let lines = vec![
            line(&format!(
                "The {grammar} grammar is built from source with a C compiler,"
            )),
            line(&format!("and there isn't one. Install {}?", install.what)),
            vec![],
            vec![span(
                &format!("  {}", install.command),
                Some("ui.popup.title"),
            )],
            vec![],
            line("It runs in the terminal panel, where you can see it and"),
            line("answer what it asks. When it's done, :plugin_reload builds"),
            line("the grammars."),
            vec![],
            vec![
                span("y", Some("ui.popup.title")),
                span(" install   ", None),
                span("n", Some("ui.popup.title")),
                span(" not now   ", None),
                span("d", Some("ui.popup.title")),
                span(" don't ask again", None),
            ],
        ];
        self.packages.compiler_command = Some(install.command);
        self.open_popup(Popup {
            name: "plugin-compiler".into(),
            title: "A C compiler is needed".into(),
            lines,
            keys: vec![
                ("y".into(), "plugin_install_compiler".into()),
                ("n".into(), "popup_close".into()),
                ("d".into(), "plugin_decline_compiler".into()),
            ],
            focus: true,
            ..Popup::default()
        });
    }

    /// `y` when a C compiler is offered: runs the install command in the
    /// terminal panel.
    pub fn plugin_install_compiler(&mut self) -> Result<(), String> {
        if self.popup().is_none_or(|p| p.name != "plugin-compiler") {
            return Err("Nothing to install".into());
        }
        self.close_popup();
        let command = self
            .packages
            .compiler_command
            .take()
            .ok_or("Nothing to install")?;
        self.focus_terminal()?;
        self.terminal_paste(&command);
        self.terminal_keys(&[crate::input::KeyPress::plain(crate::input::KeyCode::Enter)]);
        self.set_message(Message::info(format!(
            "Running {command}; when it's finished, :plugin_reload builds the grammars"
        )));
        Ok(())
    }

    /// `d` when a C compiler is offered: don't offer again.
    pub fn plugin_decline_compiler(&mut self) -> Result<(), String> {
        if self.popup().is_none_or(|p| p.name != "plugin-compiler") {
            return Err("Nothing to decline".into());
        }
        self.close_popup();
        self.packages.compiler_command = None;
        let paths = self.require_plugin_paths()?;
        packages::decline(&paths, COMPILER_DECLINED)?;
        self.set_message(Message::info(
            "Won't offer a C compiler again; grammars need one to build",
        ));
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
    pub fn plugin_browse(&mut self, tag: Option<&str>) -> Result<(), String> {
        self.packages.browse_tag = tag.map(str::to_string);
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
        let tag = self.packages.browse_tag.take();
        let title = match &tag {
            Some(tag) => format!("Plugins: {tag}"),
            None => "Plugins".into(),
        };
        let items: Vec<PickerItem> = index
            .entries
            .into_iter()
            .filter(|entry| tag.as_ref().is_none_or(|t| entry.tags.contains(t)))
            .map(|entry| {
                // A table: ✓ if installed, what it is, what it does, and
                // any other tags (light, dark...).
                let kind = if entry.language || entry.tags.iter().any(|t| t == "language") {
                    "language"
                } else if entry.tags.iter().any(|t| t == "theme") {
                    "theme"
                } else {
                    "plugin"
                };
                let others: Vec<&str> = entry
                    .tags
                    .iter()
                    .map(String::as_str)
                    .filter(|t| !matches!(*t, "language" | "theme"))
                    .collect();
                let mut description = entry.description.clone();
                if !others.is_empty() {
                    description = format!("{description} · {}", others.join(", "));
                }
                let source = match &directory {
                    Some(directory) => entry.source(directory),
                    None => entry.repo.clone().unwrap_or_default(),
                };
                let same =
                    |w: &packages::Wanted| Source::parse(&source).is_ok_and(|s| s.same_as(w));
                let is_installed = installed
                    .iter()
                    .any(|(name, w)| *name == entry.name || same(w));
                let mark = if is_installed { "✓" } else { " " };
                PickerItem {
                    label: entry.name,
                    detail: format!("{mark} {kind:<9} {description}"),
                    action: PickerAction::RunWith("plugin_install".into(), vec![source]),
                }
            })
            .collect();
        if items.is_empty() {
            self.set_message(Message::info(
                "The plugin directory has nothing like that yet",
            ));
            return;
        }
        self.open_picker(
            Picker::new(title, items)
                .aligned_details()
                .keep_label_start()
                .with_placeholder("Search plugins by name (✓ installed)"),
        );
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
        // A table: name, version, what it is, what it does; where it's
        // from, dimmed, last.
        let width = listed.iter().map(|p| p.name.len()).max().unwrap_or(0);
        let version_width = listed
            .iter()
            .map(|p| p.version.as_deref().unwrap_or("").len())
            .max()
            .unwrap_or(0);
        let mut lines = Vec::new();
        for plugin in &listed {
            let kind = if !plugin.languages.is_empty() {
                "language"
            } else if !plugin.themes.is_empty() {
                "theme"
            } else {
                "plugin"
            };
            let version = plugin.version.as_deref().unwrap_or("");
            let mut line = vec![
                span(&format!("{:width$}", plugin.name), Some("ui.popup.title")),
                span(&format!("  {version:version_width$}"), None),
                span(&format!("  {kind:<8}"), Some("ui.popup.detail")),
            ];
            match &plugin.problem {
                Some(problem) => {
                    line.push(span(&format!("  ({problem})"), Some("ui.decoration.error")))
                }
                None => line.push(span(&format!("  {}", plugin.description), None)),
            }
            let mut from = format!("    {}", plugin.source);
            if let Some(commit) = &plugin.commit {
                from = format!("{from} {}", short(commit));
            }
            line.push(span(&from, Some("comment")));
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
