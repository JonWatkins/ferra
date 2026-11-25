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
//!
//! The menu bar (`editor/menu_bar.rs`, drawn by `ui/menu_bar.rs`) takes the
//! screen's top row before anything else is laid out. Its menus are data
//! (`MENUS`: titles, letters, and items naming commands, with arguments after a
//! space); an open menu is an ordinary popup (named `menu`, at
//! `PopupPosition::At` under its title, so placed against the whole screen),
//! whose own keys run `menu_run` (close it, then run the item's command through
//! the registry), `menu_prev` and `menu_next`. The keys shown beside items come
//! from the keymap: the app gives the editor a map of them
//! (`refresh_menu_hints`) whenever the keymap changes. Each menu also has a
//! command (`menu_file`...) for keys: `<A-f>`, and the characters a Mac's
//! Option key types (`ƒ`) in normal mode and the sidebar, so ⌥ works without
//! the terminal sending it as Alt.
//!
//! **Option on macOS, Alt elsewhere, with no configuration.** Key labels show
//! ⌥F on macOS and Alt+F elsewhere. A Mac terminal sends Option+F as the
//! character it types (`ƒ`) unless it's set to send Option as Alt, so ferra
//! reads those characters as ⌥ keys where they can't be text (menus, the
//! sidebar, popups, normal mode) and the key is bound, for US and UK layouts.
//! ⌥E, ⌥U, ⌥I, ⌥N and ⌥\` are accent keys the Mac holds back, so menus avoid
//! those letters. The first time ferra sees such a character it says once how
//! to send Option as Alt (iTerm2: Left Option key → Esc+, keeping the right one
//! for `#` and `€` on UK keyboards).

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
            Some(("New File", "new_file")),
            Some(("Open File…", "file_picker")),
            Some(("Open Buffer…", "buffer_picker")),
            None,
            Some(("Save", "save")),
            Some(("Save As…", "save_as")),
            Some(("Save All", "write_all")),
            None,
            Some(("Close Tab", "tab_close")),
            Some(("Settings", "settings")),
            Some(("Keyboard Shortcuts", "keybindings")),
            Some(("Color Theme…", "theme")),
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
            Some(("Problems", "problems")),
            Some(("Output", "output")),
            Some(("Terminal", "terminal_toggle")),
            Some(("Toggle Panel", "panel")),
            None,
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
            Some(("Open Link", "open_url")),
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
            Some(("New Terminal", "terminal_new")),
            Some(("Next Terminal", "terminal_next")),
            Some(("Previous Terminal", "terminal_prev")),
            Some(("Close Terminal", "terminal_close")),
        ],
    },
    Menu {
        title: "Agent",
        letter: 'a',
        items: &[
            Some(("Toggle Agent Sidebar", "agent_sidebar")),
            None,
            Some(("New Conversation", "agent_new")),
            None,
            Some(("Change Mode…", "agent_mode")),
            Some(("Choose Model…", "agent_model")),
            Some(("Agent Settings…", "agent_config")),
            Some(("Choose Agent…", "agent_choose")),
            Some(("Show Tool Output", "agent_expand")),
            None,
            Some(("Stop Turn", "agent_cancel")),
            Some(("Stop Agent", "agent_stop")),
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
            Some(("Keyboard Shortcuts", "keybindings")),
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
    ('a', 'å'),
    ('h', '˙'),
];

/// The right-click menu in the text.
pub const TEXT_MENU: &[Option<(&str, &str)>] = &[
    Some(("Go to Definition", "goto_definition")),
    Some(("Go to Declaration", "goto_declaration")),
    Some(("Go to Type Definition", "goto_type_definition")),
    Some(("Go to Implementation", "goto_implementation")),
    Some(("Find All References", "goto_references")),
    None,
    Some(("Rename Symbol", "rename_prompt")),
    Some(("Format Document", "format")),
    Some(("Code Actions", "code_actions")),
    None,
    Some(("Cut", "cut")),
    Some(("Copy", "copy")),
    Some(("Paste", "paste_clipboard")),
    None,
    Some((REVEAL, "reveal_in_file_manager")),
    Some(("Copy Path", "copy_path")),
    Some(("Copy Relative Path", "copy_relative_path")),
    Some(("File History", "git_history")),
];

/// The right-click menu in the file tree.
pub const TREE_MENU: &[Option<(&str, &str)>] = &[
    Some(("New File", "tree_new_file")),
    Some(("New Folder", "tree_new_folder")),
    None,
    Some((REVEAL, "tree_reveal_in_file_manager")),
    Some(("Find in Files", "project_search")),
    None,
    Some(("Copy Path", "tree_copy_path")),
    Some(("Copy Relative Path", "tree_copy_relative_path")),
    None,
    Some(("Rename", "tree_rename")),
    Some(("Move to Bin", "tree_trash")),
    None,
    Some(("Collapse All", "sidebar_collapse_all")),
];

/// The right-click menu on the file tree's empty space: the project's
/// folder.
pub const TREE_ROOT_MENU: &[Option<(&str, &str)>] = &[
    Some(("New File", "tree_new_file_at_root")),
    Some(("New Folder", "tree_new_folder_at_root")),
    None,
    Some(("Find in Files", "project_search")),
    Some(("Refresh", "sidebar_refresh")),
    Some(("Collapse All", "sidebar_collapse_all")),
];

const REVEAL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else {
    "Show in File Manager"
};

/// The menu bar's state: which menu is open (or that a right-click menu
/// is), its items, and the keys to show for commands (from the keymap,
/// given by the app).
#[derive(Debug, Default)]
pub struct MenuState {
    open: Option<usize>,
    items: Vec<Option<(String, String)>>,
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
        // Under the menu bar's row.
        self.show_menu(menu.items, x, 1);
        self.menu.open = Some(index);
        Ok(())
    }

    /// A right-click: the text's menu, or the file tree's (`tree`), at
    /// screen position `x`, `y`.
    pub fn context_menu(&mut self, tree: bool, x: u16, y: u16) {
        self.show_menu(if tree { TREE_MENU } else { TEXT_MENU }, x, y);
        self.menu.open = None;
    }

    /// Any menu of `items` at `x`, `y` (the status bar's).
    pub fn show_menu_at(&mut self, items: &[Option<(&str, &str)>], x: u16, y: u16) {
        self.show_menu(items, x, y);
        self.menu.open = None;
    }

    /// A right-click on the file tree's empty space.
    pub fn tree_root_menu(&mut self, x: u16, y: u16) {
        self.show_menu(TREE_ROOT_MENU, x, y);
        self.menu.open = None;
    }

    /// Shows `items` as a menu at `x`, `y` (with focus).
    fn show_menu(&mut self, items: &[Option<(&str, &str)>], x: u16, y: u16) {
        let width = items
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
        let lines = items
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
            position: PopupPosition::At { x, y },
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
        self.menu.items = items
            .iter()
            .map(|item| item.map(|(label, command)| (label.to_string(), command.to_string())))
            .collect();
    }

    /// Whether a menu, the bar's or a right-click's, is open.
    pub fn any_menu_open(&self) -> bool {
        self.popup().is_some_and(|p| p.name == "menu")
    }

    /// `<CR>` (or a click) on the open menu's item: closes the menu and
    /// returns the command to run, with its arguments.
    pub fn menu_chosen(&mut self) -> Result<Option<(String, Vec<String>)>, String> {
        if !self.any_menu_open() {
            return Err("No menu is open".into());
        }
        let selected = self.popup().and_then(|p| p.selected()).unwrap_or(0);
        let item = self.menu.items.get(selected).cloned().flatten();
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
