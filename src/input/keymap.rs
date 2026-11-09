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
    /// and `gg`), lookups wait for the longer one; the shorter one runs when
    /// the key timeout passes (see [`Keymap::command_at`]).
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

    /// The command bound to exactly `keys`, even if longer bindings start
    /// with them. Used when the key timeout passes mid-sequence.
    pub fn command_at(&self, mode: Mode, keys: &[KeyPress]) -> Option<&str> {
        let mut node = self.modes.get(&mode)?;
        for key in keys {
            node = node.children.get(key)?;
        }
        node.command.as_deref()
    }

    /// The shortest key sequence bound to `command` in `mode`, for showing
    /// users which keys to press (the splash screen). Ties go to the
    /// alphabetically first, so the answer doesn't change between runs.
    pub fn keys_for(&self, mode: Mode, command: &str) -> Option<Vec<KeyPress>> {
        fn walk(
            node: &Node,
            command: &str,
            path: &mut Vec<KeyPress>,
            best: &mut Option<Vec<KeyPress>>,
        ) {
            if node.command.as_deref() == Some(command) {
                let better = best
                    .as_ref()
                    .is_none_or(|b| (path.len(), keys_text(path)) < (b.len(), keys_text(b)));
                if better {
                    *best = Some(path.clone());
                }
            }
            for (key, child) in &node.children {
                path.push(*key);
                walk(child, command, path, best);
                path.pop();
            }
        }
        fn keys_text(keys: &[KeyPress]) -> String {
            keys.iter().map(ToString::to_string).collect()
        }
        let mut best = None;
        walk(self.modes.get(&mode)?, command, &mut Vec::new(), &mut best);
        best
    }

    /// The keys to show for `command` in `mode` in the interface's hints:
    /// one without Alt (which macOS terminals don't send by default), then
    /// one that doesn't need the kitty keyboard protocol (`<C-`>`), then
    /// the shortest, then with the fewest modifiers (`<Up>` over `<C-p>`).
    pub fn hint_keys(&self, mode: Mode, command: &str) -> Option<Vec<KeyPress>> {
        fn walk(
            node: &Node,
            command: &str,
            path: &mut Vec<KeyPress>,
            found: &mut Vec<Vec<KeyPress>>,
        ) {
            if node.command.as_deref() == Some(command) {
                found.push(path.clone());
            }
            for (key, child) in &node.children {
                path.push(*key);
                walk(child, command, path, found);
                path.pop();
            }
        }
        let mut found = Vec::new();
        walk(self.modes.get(&mode)?, command, &mut Vec::new(), &mut found);
        found.into_iter().min_by_key(|keys| {
            let alt = keys.iter().any(|k| k.modifiers.alt);
            let enhanced = keys.iter().any(|k| k.needs_enhanced_keyboard());
            let modifiers: usize = keys
                .iter()
                .map(|k| usize::from(k.modifiers.ctrl) + usize::from(k.modifiers.shift))
                .sum();
            let text: String = keys.iter().map(ToString::to_string).collect();
            (alt, enhanced, keys.len(), modifiers, text)
        })
    }

    /// Whether `key` appears anywhere in `mode`'s bindings.
    pub fn uses(&self, mode: Mode, key: KeyPress) -> bool {
        fn walk(node: &Node, key: KeyPress) -> bool {
            node.children
                .iter()
                .any(|(k, child)| *k == key || walk(child, key))
        }
        self.modes.get(&mode).is_some_and(|node| walk(node, key))
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
    fn uses_finds_a_key_anywhere_in_a_modes_bindings() {
        let map = keymap(&[(Mode::Normal, "<C-w><C-i>", "x"), (Mode::Insert, "jk", "y")]);
        let ctrl_i = keys("<C-i>")[0];
        assert!(map.uses(Mode::Normal, ctrl_i), "second in a sequence");
        assert!(!map.uses(Mode::Insert, ctrl_i), "another mode's");
        assert!(map.uses(Mode::Insert, keys("k")[0]));
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

    #[test]
    fn command_at_finds_a_binding_hidden_by_a_longer_one() {
        let map = keymap(&[(Mode::Normal, "g", "short"), (Mode::Normal, "gg", "long")]);
        assert_eq!(map.lookup(Mode::Normal, &keys("g")), Lookup::Pending);
        assert_eq!(map.command_at(Mode::Normal, &keys("g")), Some("short"));
        assert_eq!(map.command_at(Mode::Normal, &keys("x")), None);
        assert_eq!(map.command_at(Mode::Insert, &keys("g")), None);
    }

    #[test]
    fn hint_keys_prefer_bindings_without_alt() {
        let mut map = Keymap::default();
        map.bind(Mode::Find, &parse_keys("<A-c>").unwrap(), "case");
        map.bind(Mode::Find, &parse_keys("<C-a>").unwrap(), "case");
        map.bind(Mode::Find, &parse_keys("<A-r>").unwrap(), "regex");
        let shown = |c| map.hint_keys(Mode::Find, c).map(|k| k[0].to_string());
        assert_eq!(shown("case").as_deref(), Some("<C-a>"));
        assert_eq!(
            shown("regex").as_deref(),
            Some("<A-r>"),
            "if that's all there is"
        );
        assert_eq!(shown("nothing"), None);
    }

    #[test]
    fn hint_keys_prefer_keys_every_terminal_sends() {
        let mut map = Keymap::default();
        map.bind(Mode::Normal, &parse_keys("<C-`>").unwrap(), "panel");
        map.bind(Mode::Normal, &parse_keys("<Space>t").unwrap(), "panel");
        let keys = map.hint_keys(Mode::Normal, "panel").unwrap();
        assert_eq!(keys.len(), 2, "works without the kitty protocol");
    }

    #[test]
    fn keys_for_finds_the_shortest_binding() {
        let map = keymap(&[
            (Mode::Normal, "<Space>f", "file_picker"),
            (Mode::Normal, "<C-p>", "file_picker"),
            (Mode::Normal, "gg", "goto_file_start"),
        ]);
        let shown = |c| {
            map.keys_for(Mode::Normal, c)
                .map(|k| k.iter().map(ToString::to_string).collect::<String>())
        };
        assert_eq!(shown("file_picker").as_deref(), Some("<C-p>"));
        assert_eq!(shown("goto_file_start").as_deref(), Some("gg"));
        assert_eq!(shown("nothing"), None);
        assert_eq!(map.keys_for(Mode::Insert, "file_picker"), None);
    }
}
