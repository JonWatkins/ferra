//! The menu bar along the top of the screen, as a desktop app's (and
//! Fresh's): File, Edit, View, Go, Git, Terminal, Help. Each menu is a
//! list of commands with their keys; it opens with a click on its title,
//! F10 (the first menu), or Alt (⌥ on macOS) and its letter.
//!
//! An open menu is a popup (`Popup`, named `menu`) under its title: the
//! popup's keys move through it, `<CR>` (or a click) runs the item
//! (`menu_run`), `←`/`→` open the menu beside it.
//!
//! The letters avoid ⌥E, ⌥U, ⌥I, ⌥N and ⌥`, which a Mac's keyboard keeps
//! for accents: Edit is `d`, Go `o`.

use std::collections::HashMap;

use super::Editor;
use super::popup::{Popup, PopupPosition, PopupSpan};

/// One menu: its title, the letter that opens it with Alt (shown
/// underlined), and its items: a label and the command it runs (with any
/// arguments after a space), or `None` for a separator.
pub struct Menu {
    pub title: &'static str,
    pub letter: char,
    pub items: &'static [Option<(&'static str, &'static str)>],
}

pub const MENUS: &[Menu] = &[
    Menu {
        title: "File",
        letter: 'f',
        items: &[
            Some(("Open File…", "file_picker")),
            Some(("Open Buffer…", "buffer_picker")),
            None,
            Some(("Save", "write")),
            Some(("Save All", "write_all")),
            None,
            Some(("Close Buffer", "buffer_delete")),
            Some(("Settings", "settings")),
            Some(("Edit config.toml", "open_config")),
            None,
            Some(("Quit", "quit_all")),
        ],
    },
    Menu {
        title: "Edit",
        letter: 'd',
        items: &[
            Some(("Undo", "undo")),
            Some(("Redo", "redo")),
            None,
            Some(("Find", "find_open")),
            Some(("Replace", "find_replace_open")),
            Some(("Find in Files", "project_search")),
            None,
            Some(("Format Document", "format")),
            Some(("Rename Symbol", "rename_prompt")),
            Some(("Code Actions", "code_actions")),
        ],
    },
    Menu {
        title: "View",
        letter: 'v',
        items: &[
            Some(("Command Palette", "command_palette")),
            None,
            Some(("Explorer", "activity files")),
            Some(("Search", "activity search")),
            Some(("Source Control", "activity git")),
            Some(("Toggle Sidebar", "sidebar_toggle")),
            None,
            Some(("Terminal", "terminal_toggle")),
            Some(("Split Right", "split_vertical")),
            Some(("Split Down", "split_horizontal")),
        ],
    },
    Menu {
        title: "Go",
        letter: 'o',
        items: &[
            Some(("Go to File…", "file_picker")),
            Some(("Go to Symbol in File…", "symbol_picker")),
            Some(("Go to Symbol in Project…", "project_symbol_picker")),
            None,
            Some(("Go to Definition", "goto_definition")),
            Some(("Back", "jump_back")),
            Some(("Forward", "jump_forward")),
            None,
            Some(("Next Problem", "goto_next_diagnostic")),
            Some(("Previous Problem", "goto_prev_diagnostic")),
            Some(("Problems…", "diagnostics_picker")),
        ],
    },
    Menu {
        title: "Git",
        letter: 'g',
        items: &[
            Some(("Git View", "git_gui_open")),
            Some(("Source Control", "activity git")),
            None,
            Some(("Blame", "git_blame_view")),
            Some(("File History", "git_history")),
            Some(("Next Change", "git_next_change")),
            Some(("Previous Change", "git_prev_change")),
            None,
            Some(("Fetch", "git_fetch")),
            Some(("Pull", "git_pull")),
            Some(("Push", "git_push")),
        ],
    },
    Menu {
        title: "Terminal",
        letter: 't',
        items: &[
            Some(("Toggle Terminal", "terminal_toggle")),
            Some(("Focus Terminal", "terminal_focus")),
            Some(("Close Terminal", "terminal_close")),
        ],
    },
    Menu {
        title: "Help",
        letter: 'h',
        items: &[
            Some(("Command Palette", "command_palette")),
            None,
            Some(("Browse Plugins…", "plugin_browse")),
            Some(("Installed Plugins", "plugin_list")),
            None,
            Some(("Settings", "settings")),
        ],
    },
];

/// The character a Mac's Option key types with each menu's letter (US and
/// UK layouts), so ⌥F opens File even when the terminal doesn't send
/// Option as Alt.
pub const MAC_OPTION_LETTERS: &[(char, char)] = &[
    ('f', 'ƒ'),
    ('d', '∂'),
    ('v', '√'),
    ('o', 'ø'),
    ('g', '©'),
    ('t', '†'),
    ('h', '˙'),
];

/// The menu bar's state: which menu is open, and the keys to show for
/// commands (from the keymap, given by the app).
#[derive(Debug, Default)]
pub struct MenuState {
    open: Option<usize>,
    hints: HashMap<String, String>,
}

impl Editor {
    /// The menu open, if one is (its popup has to be the one shown).
    pub fn open_menu(&self) -> Option<usize> {
        self.menu
            .open
            .filter(|_| self.popup().is_some_and(|p| p.name == "menu"))
    }

    /// The keys shown beside each command (`<Space>f`), from the keymap.
    pub fn set_menu_hints(&mut self, hints: HashMap<String, String>) {
        self.menu.hints = hints;
    }

    /// Opens menu `index` under its title (at screen column `x`), or with
    /// F10 or Alt and its letter. Opening the open one again closes it.
    pub fn menu_open(&mut self, index: usize, x: u16) -> Result<(), String> {
        let menu = MENUS.get(index).ok_or("No such menu")?;
        if self.open_menu() == Some(index) {
            self.close_popup();
            return Ok(());
        }
        let width = menu
            .items
            .iter()
            .flatten()
            .map(|(label, command)| {
                let hint = self
                    .menu
                    .hints
                    .get(*command)
                    .map_or(0, |h| h.chars().count());
                label.chars().count() + if hint > 0 { hint + 4 } else { 0 }
            })
            .max()
            .unwrap_or(0);
        let lines = menu
            .items
            .iter()
            .map(|item| match item {
                None => vec![PopupSpan {
                    text: "─".repeat(width),
                    style: Some("ui.menu.separator".into()),
                }],
                Some((label, command)) => {
                    let hint = self.menu.hints.get(*command).cloned().unwrap_or_default();
                    let gap = width.saturating_sub(label.chars().count() + hint.chars().count());
                    vec![
                        PopupSpan {
                            text: format!("{label}{}", " ".repeat(gap)),
                            style: None,
                        },
                        PopupSpan {
                            text: hint,
                            style: Some("ui.menu.key".into()),
                        },
                    ]
                }
            })
            .collect();
        if self.popup().is_some() {
            self.close_popup();
        }
        self.open_popup(Popup {
            name: "menu".into(),
            title: String::new(),
            lines,
            // Under the menu bar's row.
            position: PopupPosition::At { x, y: 1 },
            select: true,
            focus: true,
            keys: vec![
                ("<CR>".into(), "menu_run".into()),
                ("<Left>".into(), "menu_prev".into()),
                ("h".into(), "menu_prev".into()),
                ("<Right>".into(), "menu_next".into()),
                ("l".into(), "menu_next".into()),
                ("<F10>".into(), "popup_close".into()),
            ],
            ..Popup::default()
        });
        self.menu.open = Some(index);
        Ok(())
    }

    /// `<CR>` (or a click) on the open menu's item: closes the menu and
    /// returns the command to run, with its arguments.
    pub fn menu_chosen(&mut self) -> Result<Option<(String, Vec<String>)>, String> {
        let index = self.open_menu().ok_or("No menu is open")?;
        let selected = self.popup().and_then(|p| p.selected()).unwrap_or(0);
        let item = MENUS[index].items.get(selected).copied().flatten();
        let Some((_, command)) = item else {
            // A separator: nothing to run.
            return Ok(None);
        };
        self.close_popup();
        self.menu.open = None;
        let mut words = command.split_whitespace().map(str::to_string);
        let name = words.next().unwrap_or_default();
        Ok(Some((name, words.collect())))
    }

    /// `←` / `→` in an open menu: the menu beside it (at column `x`, where
    /// the app drew its title).
    pub fn menu_beside(&mut self, delta: isize) -> Option<usize> {
        let index = self.open_menu()?;
        Some((index as isize + delta).rem_euclid(MENUS.len() as isize) as usize)
    }

    /// The menu whose letter is `letter`.
    pub fn menu_for_letter(letter: char) -> Option<usize> {
        let letter = letter.to_ascii_lowercase();
        MENUS.iter().position(|m| m.letter == letter)
    }
}

/// Where each title goes in the bar, starting at `x`: its columns.
pub fn title_spans(x: u16) -> Vec<(u16, u16)> {
    let mut x = x + 1;
    MENUS
        .iter()
        .map(|menu| {
            let width = menu.title.chars().count() as u16 + 2;
            let span = (x, x + width);
            x += width;
            span
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandRegistry;

    #[test]
    fn every_menu_item_is_a_command() {
        let registry = CommandRegistry::with_builtins();
        for menu in MENUS {
            for (label, command) in menu.items.iter().flatten() {
                let name = command.split_whitespace().next().unwrap();
                assert!(
                    registry.contains(name),
                    "{}: {label} runs {name}",
                    menu.title
                );
            }
        }
    }

    #[test]
    fn letters_are_unique_and_avoid_the_macs_accent_keys() {
        let letters: Vec<char> = MENUS.iter().map(|m| m.letter).collect();
        let mut unique = letters.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), letters.len());
        for letter in letters {
            assert!(
                !"euin`".contains(letter),
                "{letter} is an accent key on a Mac"
            );
            let title = MENUS[Editor::menu_for_letter(letter).unwrap()].title;
            assert!(
                title.to_lowercase().contains(letter),
                "{title} has {letter}"
            );
            assert!(MAC_OPTION_LETTERS.iter().any(|(l, _)| *l == letter));
        }
    }
}
