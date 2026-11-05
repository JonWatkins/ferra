//! Per-mode key bindings, stored as a trie so multi-key sequences (`gg`, `dd`)
//! can be resolved one key at a time.

use std::collections::HashMap;

use serde::Deserialize;

use super::key::{KeyPress, parse_keys};
use crate::editor::Mode;

/// The result of looking up a (possibly partial) key sequence.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// The sequence is bound to this command.
    Command(&'a str),
    /// The sequence is a prefix of one or more bindings; wait for more keys.
    Pending,
    /// Nothing is bound to this sequence.
    NoMatch,
}

#[derive(Debug, Default, Clone)]
struct Node {
    command: Option<String>,
    children: HashMap<KeyPress, Node>,
}

/// Raw keybinding config: `mode -> key notation -> command name`.
pub type KeyBindings = HashMap<Mode, HashMap<String, String>>;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(try_from = "KeyBindings")]
pub struct Keymap {
    modes: HashMap<Mode, Node>,
}

impl Keymap {
    /// Binds `keys` in `mode` to `command`.
    ///
    /// If a sequence is both bound and a prefix of a longer binding (e.g. `g`
    /// and `gg`), the longer binding wins and the shorter one is unreachable.
    pub fn bind(&mut self, mode: Mode, keys: &[KeyPress], command: impl Into<String>) {
        let mut node = self.modes.entry(mode).or_default();
        for key in keys {
            node = node.children.entry(*key).or_default();
        }
        node.command = Some(command.into());
    }

    pub fn lookup(&self, mode: Mode, keys: &[KeyPress]) -> Lookup<'_> {
        let Some(mut node) = self.modes.get(&mode) else {
            return Lookup::NoMatch;
        };
        for key in keys {
            match node.children.get(key) {
                Some(child) => node = child,
                None => return Lookup::NoMatch,
            }
        }
        if !node.children.is_empty() {
            Lookup::Pending
        } else if let Some(command) = &node.command {
            Lookup::Command(command)
        } else {
            Lookup::NoMatch
        }
    }

    /// Every `(mode, command)` pair that is bound, for validation.
    pub fn bound_commands(&self) -> Vec<(Mode, &str)> {
        fn walk<'a>(node: &'a Node, mode: Mode, out: &mut Vec<(Mode, &'a str)>) {
            if let Some(command) = &node.command {
                out.push((mode, command));
            }
            node.children.values().for_each(|c| walk(c, mode, out));
        }
        let mut out = Vec::new();
        for (mode, node) in &self.modes {
            walk(node, *mode, &mut out);
        }
        out
    }
}

impl TryFrom<KeyBindings> for Keymap {
    type Error = String;

    /// Builds a keymap from config. Empty command names unbind the key, which
    /// lets user config remove default bindings.
    fn try_from(bindings: KeyBindings) -> Result<Self, String> {
        let mut keymap = Keymap::default();
        for (mode, map) in bindings {
            for (notation, command) in map {
                if command.is_empty() {
                    continue;
                }
                let keys = parse_keys(&notation).map_err(|e| {
                    format!("invalid key '{notation}' in [keys.{}]: {e}", mode.id())
                })?;
                keymap.bind(mode, &keys, command);
            }
        }
        Ok(keymap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(s: &str) -> Vec<KeyPress> {
        parse_keys(s).unwrap()
    }

    fn keymap(bindings: &[(Mode, &str, &str)]) -> Keymap {
        let mut map = Keymap::default();
        for (mode, k, cmd) in bindings {
            map.bind(*mode, &keys(k), *cmd);
        }
        map
    }

    #[test]
    fn single_key_lookup() {
        let map = keymap(&[(Mode::Normal, "j", "move_down")]);
        assert_eq!(
            map.lookup(Mode::Normal, &keys("j")),
            Lookup::Command("move_down")
        );
        assert_eq!(map.lookup(Mode::Normal, &keys("k")), Lookup::NoMatch);
    }

    #[test]
    fn bindings_are_per_mode() {
        let map = keymap(&[(Mode::Normal, "j", "move_down")]);
        assert_eq!(map.lookup(Mode::Insert, &keys("j")), Lookup::NoMatch);
    }

    #[test]
    fn multi_key_sequences_are_pending_until_complete() {
        let map = keymap(&[(Mode::Normal, "gg", "goto_file_start")]);
        assert_eq!(map.lookup(Mode::Normal, &keys("g")), Lookup::Pending);
        assert_eq!(
            map.lookup(Mode::Normal, &keys("gg")),
            Lookup::Command("goto_file_start")
        );
        assert_eq!(map.lookup(Mode::Normal, &keys("gx")), Lookup::NoMatch);
    }

    #[test]
    fn longer_binding_shadows_prefix() {
        let map = keymap(&[(Mode::Normal, "g", "a"), (Mode::Normal, "gg", "b")]);
        assert_eq!(map.lookup(Mode::Normal, &keys("g")), Lookup::Pending);
    }

    #[test]
    fn rebinding_replaces_command() {
        let map = keymap(&[(Mode::Normal, "x", "a"), (Mode::Normal, "x", "b")]);
        assert_eq!(map.lookup(Mode::Normal, &keys("x")), Lookup::Command("b"));
    }

    #[test]
    fn builds_from_config_and_skips_unbound() {
        let bindings = KeyBindings::from([(
            Mode::Normal,
            HashMap::from([
                ("<C-q>".to_string(), "quit".to_string()),
                ("x".to_string(), String::new()),
            ]),
        )]);
        let map = Keymap::try_from(bindings).unwrap();
        assert_eq!(
            map.lookup(Mode::Normal, &keys("<C-q>")),
            Lookup::Command("quit")
        );
        assert_eq!(map.lookup(Mode::Normal, &keys("x")), Lookup::NoMatch);
    }

    #[test]
    fn invalid_notation_is_an_error() {
        let bindings = KeyBindings::from([(
            Mode::Insert,
            HashMap::from([("<Bogus>".to_string(), "quit".to_string())]),
        )]);
        let err = Keymap::try_from(bindings).unwrap_err();
        assert!(err.contains("[keys.insert]"), "{err}");
    }

    #[test]
    fn lists_bound_commands() {
        let map = keymap(&[
            (Mode::Normal, "gg", "top"),
            (Mode::Insert, "<Esc>", "normal"),
        ]);
        let mut bound = map.bound_commands();
        bound.sort();
        assert_eq!(bound, vec![(Mode::Normal, "top"), (Mode::Insert, "normal")]);
    }
}
