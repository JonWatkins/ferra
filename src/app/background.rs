//! Work done off the UI thread, reporting back through a channel.
//!
//! A job runs on its own thread and sends [`Update`]s; the event loop drains
//! them between keys (it polls rather than blocks while a job is running)
//! and applies them to the editor. Today the only job is the file picker's
//! listing, which streams files into the open picker so a large tree never
//! freezes the editor. Later jobs (language servers, a terminal) can send
//! their own kinds of update the same way.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use super::App;
use crate::editor::picker::PickerItem;
use crate::editor::{FileListRequest, Message};
use crate::files;

/// What a background job reports.
pub(super) enum Update {
    /// Some of the files found for picker job `job`.
    Files {
        job: u64,
        base: Option<PathBuf>,
        paths: Vec<PathBuf>,
    },
    /// The listing for `job` ended (finished, cut short at `max_files`, or
    /// cancelled).
    FilesDone {
        job: u64,
        truncated: bool,
        max_files: usize,
    },
    /// A plugin install finished.
    PluginInstalled(Result<crate::plugins::packages::Installed, String>),
}

pub(super) struct Background {
    sender: Sender<Update>,
    receiver: Receiver<Update>,
    /// Jobs started whose final update hasn't arrived yet.
    running: usize,
    /// The file listing in progress, and the flag that stops it.
    file_job: Option<(u64, Arc<AtomicBool>)>,
}

impl Default for Background {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            running: 0,
            file_job: None,
        }
    }
}

impl Background {
    /// Whether any job is still running, so the event loop should keep
    /// checking for updates.
    pub(super) fn busy(&self) -> bool {
        self.running > 0
    }

    /// Lists files on a new thread, stopping any listing already running.
    fn list_files(&mut self, request: FileListRequest) {
        self.cancel_file_listing();
        let cancel = Arc::new(AtomicBool::new(false));
        self.file_job = Some((request.job, Arc::clone(&cancel)));
        self.running += 1;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let FileListRequest {
                job,
                root,
                base,
                options,
            } = request;
            let truncated = files::walk_files(&root, &options, &cancel, |paths| {
                let _ = sender.send(Update::Files {
                    job,
                    base: base.clone(),
                    paths,
                });
            });
            let _ = sender.send(Update::FilesDone {
                job,
                truncated,
                max_files: options.max_files,
            });
        });
    }

    /// Clones and checks a plugin on a new thread.
    fn install_plugin(&mut self, request: crate::editor::InstallRequest) {
        self.running += 1;
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result =
                crate::plugins::packages::install(&request.paths, &request.source, &request.taken);
            let _ = sender.send(Update::PluginInstalled(result));
        });
    }

    fn cancel_file_listing(&mut self) {
        if let Some((_, cancel)) = self.file_job.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl App {
    /// Starts the background work the last command asked for.
    pub(super) fn start_background_requests(&mut self) {
        if let Some(request) = self.editor.take_file_list_request() {
            self.background.list_files(request);
        }
        if let Some(request) = self.editor.take_plugin_install() {
            self.background.install_plugin(request);
        }
    }

    /// Applies every update that has arrived, without waiting.
    /// Applies what background jobs sent. Returns whether there was any.
    pub(super) fn handle_background(&mut self) -> bool {
        let mut any = false;
        while let Ok(update) = self.background.receiver.try_recv() {
            self.apply_update(update);
            any = true;
        }
        self.stop_unwanted_listing();
        any
    }

    /// Waits for every running job to finish, applying their updates. For
    /// tests, which don't run the event loop.
    pub fn wait_for_background(&mut self) {
        while self.background.busy() {
            match self.background.receiver.recv() {
                Ok(update) => self.apply_update(update),
                Err(_) => break,
            }
        }
        self.stop_unwanted_listing();
    }

    fn apply_update(&mut self, update: Update) {
        match update {
            Update::PluginInstalled(result) => {
                self.background.running -= 1;
                self.editor.plugin_install_done();
                let message = match result {
                    Ok(installed) => self.finish_plugin_install(&installed),
                    Err(error) => Message::error(format!("Plugin not installed: {error}")),
                };
                self.editor.set_message(message);
            }
            Update::Files { job, base, paths } => {
                if let Some(picker) = self.picker_loading(job) {
                    let items = paths
                        .into_iter()
                        .map(|path| PickerItem::file(path, base.as_deref()))
                        .collect();
                    picker.extend(items);
                }
            }
            Update::FilesDone {
                job,
                truncated,
                max_files,
            } => {
                self.background.running -= 1;
                if self
                    .background
                    .file_job
                    .as_ref()
                    .is_some_and(|(j, _)| *j == job)
                {
                    self.background.file_job = None;
                }
                if let Some(picker) = self.picker_loading(job) {
                    picker.finish_loading();
                    if truncated {
                        self.editor.set_message(Message::info(format!(
                            "Showing the first {max_files} files (see max-files)"
                        )));
                    }
                }
            }
        }
    }

    /// The open picker, if it's the one job `job` is filling.
    fn picker_loading(&mut self, job: u64) -> Option<&mut crate::editor::picker::Picker> {
        self.editor
            .picker_mut()
            .filter(|picker| picker.loading() == Some(job))
    }

    /// Stops a listing whose picker has been closed (or replaced).
    fn stop_unwanted_listing(&mut self) {
        let wanted = self.editor.picker().and_then(|p| p.loading());
        let job = self.background.file_job.as_ref().map(|(job, _)| *job);
        if job.is_some() && job != wanted {
            self.background.cancel_file_listing();
        }
    }
}
