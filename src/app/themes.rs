//! Colour themes: ferra's own (`ferra-dark`) and those theme packs bring
//! (`[themes]` in a package's manifest), chosen with `color-theme`. The
//! chosen one's colours go over ferra's own, and the user's `[theme]` over
//! both. `:theme` picks one, showing each as the selection moves.

use std::collections::BTreeMap;

use super::App;
use crate::config::Theme;
use crate::editor::picker::{Picker, PickerAction, PickerItem};
use crate::editor::{Message, Mode};

/// ferra's own theme's id and name.
pub const BUILT_IN: &str = "ferra-dark";
const BUILT_IN_NAME: &str = "Ferra Dark";

/// The picker's title (the app previews the selected theme while it's
/// open).
pub const PICKER_TITLE: &str = "Color Theme";

/// A theme a pack brought: its name, kind and colours.
#[derive(Debug, Clone)]
pub struct InstalledTheme {
    pub name: String,
    pub kind: Option<String>,
    pub colors: toml::Table,
    /// The pack it came from.
    pub pack: String,
}

#[derive(Debug, Default)]
pub struct Themes {
    pub installed: BTreeMap<String, InstalledTheme>,
    /// The theme in use's id.
    pub applied: String,
    /// The theme being previewed in the picker, made once.
    pub preview: Option<(String, Theme)>,
}

impl App {
    /// The theme packs' themes from the package in `dir` (its manifest's
    /// `[themes]`). Returns what went wrong.
    pub(super) fn add_pack_themes(&mut self, dir: &std::path::Path) -> Vec<String> {
        let Ok(manifest) = crate::plugins::packages::Manifest::read(dir) else {
            return Vec::new();
        };
        let mut errors = Vec::new();
        for (id, spec) in &manifest.themes {
            let path = dir.join(&spec.file);
            let colors = std::fs::read_to_string(&path)
                .map_err(|e| format!("{}: themes.{id}: {}: {e}", manifest.name, spec.file))
                .and_then(|text| {
                    text.parse::<toml::Table>().map_err(|e| {
                        format!(
                            "{}: themes.{id}: {}: {}",
                            manifest.name,
                            spec.file,
                            e.message()
                        )
                    })
                })
                .and_then(|colors| {
                    // Checked as config is: a bad colour says where.
                    crate::config::theme_for(Some(&colors), &toml::Table::new())
                        .map(|_| colors)
                        .map_err(|e| format!("{}: themes.{id}: {e}", manifest.name))
                });
            match colors {
                Ok(colors) => {
                    self.themes.installed.insert(
                        id.clone(),
                        InstalledTheme {
                            name: spec.name.clone(),
                            kind: spec.kind.clone(),
                            colors,
                            pack: manifest.name.clone(),
                        },
                    );
                }
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    /// The theme `id` (ferra's own, an installed one), with the user's
    /// `[theme]` over it; `None` for one that isn't installed.
    fn theme_named(&self, id: &str) -> Option<Theme> {
        let pack = match id {
            BUILT_IN => None,
            id => Some(&self.themes.installed.get(id)?.colors),
        };
        crate::config::theme_for(pack, &self.user_theme).ok()
    }

    /// Puts the theme `color-theme` names in place, if it isn't already.
    /// One that isn't installed says so (and ferra's own is used).
    pub(super) fn follow_theme_option(&mut self) {
        let wanted = self.editor.global_options().color_theme.clone();
        if wanted == self.themes.applied {
            return;
        }
        self.themes.applied = wanted.clone();
        match self.theme_named(&wanted) {
            Some(theme) => self.theme = theme,
            None => {
                if let Some(theme) = self.theme_named(BUILT_IN) {
                    self.theme = theme;
                }
                self.editor.set_message(Message::error(format!(
                    "No theme {wanted} installed (:theme lists them; :plugin_browse theme finds more)"
                )));
            }
        }
    }

    /// After plugins load again: the theme made again (a pack's may have
    /// come or gone).
    pub(super) fn reapply_theme(&mut self) {
        self.themes.applied.clear();
        self.themes.preview = None;
        self.follow_theme_option();
    }

    /// The theme to draw with: the one the theme picker's selection names,
    /// while it's open, else the one in use.
    pub(super) fn drawing_theme(&mut self) -> &Theme {
        let selected = self
            .editor
            .picker()
            .filter(|p| p.title() == PICKER_TITLE)
            .and_then(|p| p.selected_item())
            .and_then(|item| match &item.action {
                PickerAction::RunWith(_, args) => args.first().cloned(),
                _ => None,
            });
        let Some(id) = selected else {
            self.themes.preview = None;
            return &self.theme;
        };
        if self.themes.preview.as_ref().is_none_or(|(at, _)| *at != id) {
            self.themes.preview = self.theme_named(&id).map(|theme| (id, theme));
        }
        match &self.themes.preview {
            Some((_, theme)) => theme,
            None => &self.theme,
        }
    }

    /// `:theme`: a picker of the themes (ferra's own and installed packs'),
    /// the one in use selected; each is shown as the selection moves to it,
    /// and choosing one saves it. Last, finding more in the plugin
    /// directory.
    pub fn theme_picker(&mut self) {
        let current = self.editor.global_options().color_theme.clone();
        let mut items = vec![PickerItem {
            label: BUILT_IN_NAME.into(),
            detail: format!("dark  {BUILT_IN}  built in"),
            action: PickerAction::RunWith("color_theme".into(), vec![BUILT_IN.into()]),
        }];
        items.extend(self.themes.installed.iter().map(|(id, theme)| {
            PickerItem {
                label: theme.name.clone(),
                detail: format!(
                    "{}  {id}  from {}",
                    theme.kind.as_deref().unwrap_or(""),
                    theme.pack
                )
                .trim_start()
                .to_string(),
                action: PickerAction::RunWith("color_theme".into(), vec![id.clone()]),
            }
        }));
        items.push(PickerItem {
            label: "Browse more themes…".into(),
            detail: "the plugin directory's theme packs".into(),
            action: PickerAction::RunWith("plugin_browse".into(), vec!["theme".into()]),
        });
        let selected = PickerAction::RunWith("color_theme".into(), vec![current]);
        self.editor
            .open_picker(Picker::new(PICKER_TITLE, items).with_selected(&selected));
        if self.editor.mode() != Mode::Picker {
            self.editor.set_mode(Mode::Picker);
        }
    }
}
