//! `:set`: reading and changing `[editor]` options while running.
//!
//! Options are named as in the config file (`tab-width`, `search.case`),
//! and also by the Vim names people type out of habit (`ts`, `et`, `nu`,
//! `rnu`, `ic`, `scs`...). A change goes through the same deserializing
//! as the config file, so it's checked the same way: the options are
//! turned into a TOML table, one value replaced, and the table read back.
//!
//! The forms, as in Vim (several can follow one `:set`):
//!
//! | Form | Does |
//! |------|------|
//! | `name=value` (or `name:value`) | Sets it |
//! | `name+=n`, `name-=n` | Adds to or takes from a number |
//! | `name` | Turns a switch on; shows any other option |
//! | `noname`, `name!` (or `invname`) | Turns a switch off; flips it |
//! | `name?` | Shows it |
//! | `name&` | Puts back its default |
//!
//! Plain `:set` shows the options that differ from the defaults.

use toml::{Table, Value};

use super::EditorOptions;

/// What `:set` did, for its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetResult {
    /// `name=value` for each option shown or changed, in order.
    pub shown: Vec<String>,
    /// The canonical names of the options that changed.
    pub changed: Vec<String>,
}

/// Applies `:set`'s `args` to `options`. Nothing changes if any part is
/// wrong.
pub fn set(options: &mut EditorOptions, args: &[String]) -> Result<SetResult, String> {
    let defaults = table_of(&EditorOptions::default());
    let mut table = table_of(options);
    let mut result = SetResult {
        shown: Vec::new(),
        changed: Vec::new(),
    };
    if args.is_empty() {
        for name in leaf_names(&table) {
            if get(&table, &name) != get(&defaults, &name) {
                result.shown.push(show(&table, &name));
            }
        }
        if result.shown.is_empty() {
            result
                .shown
                .push("All options are at their defaults".into());
        }
        return Ok(result);
    }
    for arg in args {
        apply(&mut table, &defaults, arg, &mut result)?;
    }
    *options = Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| e.message().trim().to_string())?;
    Ok(result)
}

/// Every option's name (dotted for nested ones), for completion.
pub fn option_names() -> Vec<String> {
    let mut names = leaf_names(&table_of(&EditorOptions::default()));
    names.extend(ALIASES.iter().map(|(alias, _)| alias.to_string()));
    names.extend(SWITCHES.iter().map(|s| s.name.to_string()));
    names.sort();
    names
}

/// Vim's names for options ferra has under another name.
const ALIASES: &[(&str, &str)] = &[
    ("ts", "tab-width"),
    ("tabstop", "tab-width"),
    ("et", "expand-tab"),
    ("expandtab", "expand-tab"),
    ("ai", "auto-indent"),
    ("autoindent", "auto-indent"),
    ("cul", "cursorline"),
    ("so", "scroll-off"),
    ("scrolloff", "scroll-off"),
    ("hls", "search.highlight"),
    ("hlsearch", "search.highlight"),
    ("ws", "search.wrap"),
    ("wrapscan", "search.wrap"),
    ("tm", "key-timeout"),
    ("timeoutlen", "key-timeout"),
    ("syntax", "syntax-highlighting"),
];

/// Vim switches that are values of one of ferra's options here: `nu` and
/// `rnu` pick `line-numbers`, `ic` and `scs` pick `search.case`.
struct Switch {
    name: &'static str,
    option: &'static str,
    on: fn(&str) -> &'static str,
    off: fn(&str) -> &'static str,
    is_on: fn(&str) -> bool,
}

const SWITCHES: &[Switch] = &[
    Switch {
        name: "number",
        option: "line-numbers",
        on: |now| {
            if now == "relative" {
                "relative"
            } else {
                "absolute"
            }
        },
        off: |_| "none",
        is_on: |now| now != "none",
    },
    Switch {
        name: "relativenumber",
        option: "line-numbers",
        on: |_| "relative",
        off: |now| {
            if now == "relative" {
                "absolute"
            } else {
                now_static(now)
            }
        },
        is_on: |now| now == "relative",
    },
    Switch {
        name: "ignorecase",
        option: "search.case",
        on: |now| {
            if now == "smart" {
                "smart"
            } else {
                "insensitive"
            }
        },
        off: |_| "sensitive",
        is_on: |now| now != "sensitive",
    },
    Switch {
        name: "smartcase",
        option: "search.case",
        on: |_| "smart",
        off: |now| {
            if now == "smart" {
                "insensitive"
            } else {
                now_static(now)
            }
        },
        is_on: |now| now == "smart",
    },
];

const SWITCH_ALIASES: &[(&str, &str)] = &[
    ("nu", "number"),
    ("rnu", "relativenumber"),
    ("ic", "ignorecase"),
    ("scs", "smartcase"),
];

/// The `&'static str` among the values switches use, for leaving a value
/// as it is.
fn now_static(now: &str) -> &'static str {
    [
        "absolute",
        "relative",
        "none",
        "smart",
        "sensitive",
        "insensitive",
    ]
    .into_iter()
    .find(|v| *v == now)
    .unwrap_or("none")
}

fn switch(name: &str) -> Option<&'static Switch> {
    let name = SWITCH_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, full)| full);
    SWITCHES.iter().find(|s| s.name == name)
}

fn canonical(name: &str) -> &str {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, full)| full)
}

/// One `:set` argument.
fn apply(
    table: &mut Table,
    defaults: &Table,
    arg: &str,
    result: &mut SetResult,
) -> Result<(), String> {
    // `name=value`, `name:value`, `name+=n`, `name-=n`.
    if let Some(at) = arg.find(['=', ':']) {
        let (name, value) = (&arg[..at], &arg[at + 1..]);
        let (name, op) = match name.strip_suffix(['+', '-']) {
            Some(stripped) => (stripped, name.chars().last()),
            None => (name, None),
        };
        if let Some(switch) = switch(name) {
            return Err(format!(
                "{}: takes no value (:set {0} or :set no{0})",
                switch.name
            ));
        }
        let name = canonical(name);
        let current = get(table, name).ok_or_else(|| format!("Unknown option: {name}"))?;
        let value = match op {
            None => parse_value(name, value, current)?,
            Some(op) => {
                let Value::Integer(now) = current else {
                    return Err(format!("{name}: {op}= needs a number option"));
                };
                let by: i64 = value
                    .trim()
                    .parse()
                    .map_err(|_| format!("{name}: not a number: {value}"))?;
                Value::Integer(if op == '+' { now + by } else { now - by })
            }
        };
        return assign(table, name, value, result);
    }
    if let Some(name) = arg.strip_suffix('?') {
        let name = switch(name).map_or_else(|| canonical(name), |s| s.option);
        get(table, name).ok_or_else(|| format!("Unknown option: {name}"))?;
        result.shown.push(show(table, name));
        return Ok(());
    }
    if let Some(name) = arg.strip_suffix('&') {
        let name = switch(name).map_or_else(|| canonical(name), |s| s.option);
        let default = get(defaults, name)
            .cloned()
            .ok_or_else(|| format!("Unknown option: {name}"))?;
        return assign(table, name, default, result);
    }
    let (name, change) = if let Some(name) = arg.strip_suffix('!') {
        (name, Toggle::Flip)
    } else if let Some(name) = arg.strip_prefix("inv").filter(|n| is_switchable(table, n)) {
        (name, Toggle::Flip)
    } else if let Some(name) = arg.strip_prefix("no").filter(|n| is_switchable(table, n)) {
        (name, Toggle::Off)
    } else {
        (arg, Toggle::On)
    };
    if let Some(switch) = switch(name) {
        let now = match get(table, switch.option) {
            Some(Value::String(now)) => now.clone(),
            _ => return Err(format!("Unknown option: {}", switch.option)),
        };
        let on = match change {
            Toggle::On => true,
            Toggle::Off => false,
            Toggle::Flip => !(switch.is_on)(&now),
        };
        let value = if on {
            (switch.on)(&now)
        } else {
            (switch.off)(&now)
        };
        return assign(table, switch.option, Value::String(value.into()), result);
    }
    let name = canonical(name);
    match get(table, name) {
        Some(Value::Boolean(now)) => {
            let on = match change {
                Toggle::On => true,
                Toggle::Off => false,
                Toggle::Flip => !now,
            };
            assign(table, name, Value::Boolean(on), result)
        }
        Some(_) if matches!(change, Toggle::On) => {
            // `:set tab-width` shows it, as in Vim.
            result.shown.push(show(table, name));
            Ok(())
        }
        Some(_) => Err(format!("{name}: not a switch (on or off)")),
        None => Err(format!("Unknown option: {name}")),
    }
}

enum Toggle {
    On,
    Off,
    Flip,
}

/// Whether `name` is an on/off option (or a Vim switch), so `no` and `inv`
/// in front of it are prefixes rather than part of a name.
fn is_switchable(table: &Table, name: &str) -> bool {
    switch(name).is_some() || matches!(get(table, canonical(name)), Some(Value::Boolean(_)))
}

fn assign(
    table: &mut Table,
    name: &str,
    value: Value,
    result: &mut SetResult,
) -> Result<(), String> {
    let slot = get_mut(table, name).ok_or_else(|| format!("Unknown option: {name}"))?;
    let changed = *slot != value;
    *slot = value;
    // Checked now, so the message names the option at fault.
    let checked: Result<EditorOptions, _> = Value::Table(table.clone()).try_into();
    if let Err(error) = checked {
        let error: toml::de::Error = error;
        return Err(format!("{name}: {}", error.message().trim()));
    }
    if changed && !result.changed.iter().any(|c| c == name) {
        result.changed.push(name.to_string());
    }
    result.shown.push(show(table, name));
    Ok(())
}

/// `text` as a value for an option whose current value is `current`:
/// words for text options (no quotes needed), numbers, `true`/`false`
/// (or `on`/`off`), and TOML for lists (`[...]`).
fn parse_value(name: &str, text: &str, current: &Value) -> Result<Value, String> {
    let text = text.trim();
    match current {
        Value::String(_) => {
            let unquoted = text
                .strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(text);
            Ok(Value::String(unquoted.to_string()))
        }
        Value::Integer(_) => text
            .parse()
            .map(Value::Integer)
            .map_err(|_| format!("{name}: not a number: {text}")),
        Value::Boolean(_) => match text {
            "true" | "on" | "yes" | "1" => Ok(Value::Boolean(true)),
            "false" | "off" | "no" | "0" => Ok(Value::Boolean(false)),
            _ => Err(format!("{name}: on or off, not {text}")),
        },
        _ => {
            let parsed: Table = format!("v = {text}")
                .parse()
                .map_err(|_| format!("{name}: not a value: {text}"))?;
            Ok(parsed["v"].clone())
        }
    }
}

fn table_of(options: &EditorOptions) -> Table {
    match Value::try_from(options) {
        Ok(Value::Table(table)) => table,
        _ => Table::new(),
    }
}

fn get<'a>(table: &'a Table, name: &str) -> Option<&'a Value> {
    let mut parts = name.split('.');
    let mut value = table.get(parts.next()?)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

fn get_mut<'a>(table: &'a mut Table, name: &str) -> Option<&'a mut Value> {
    let mut parts = name.split('.');
    let mut value = table.get_mut(parts.next()?)?;
    for part in parts {
        value = value.as_table_mut()?.get_mut(part)?;
    }
    Some(value)
}

/// `name=value`, with text unquoted.
fn show(table: &Table, name: &str) -> String {
    match get(table, name) {
        Some(Value::String(text)) => format!("{name}={text}"),
        Some(value) => format!("{name}={value}"),
        None => name.to_string(),
    }
}

/// The dotted names of every non-table value.
fn leaf_names(table: &Table) -> Vec<String> {
    fn walk(table: &Table, prefix: &str, out: &mut Vec<String>) {
        for (key, value) in table {
            let name = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                Value::Table(inner) => walk(inner, &name, out),
                _ => out.push(name),
            }
        }
    }
    let mut out = Vec::new();
    walk(table, "", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LineNumbers, SearchCase};

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    fn run(options: &mut EditorOptions, text: &str) -> Result<SetResult, String> {
        set(options, &args(text))
    }

    #[test]
    fn options_round_trip_through_a_table() {
        let options = EditorOptions::default();
        let back: EditorOptions = Value::Table(table_of(&options)).try_into().unwrap();
        assert_eq!(back, options);
    }

    #[test]
    fn numbers_text_and_switches() {
        let mut options = EditorOptions::default();
        let result = run(
            &mut options,
            "tab-width=2 line-numbers=absolute noexpand-tab",
        )
        .unwrap();
        assert_eq!(options.tab_width, 2);
        assert_eq!(options.line_numbers, LineNumbers::Absolute);
        assert!(!options.expand_tab);
        assert_eq!(
            result.shown,
            ["tab-width=2", "line-numbers=absolute", "expand-tab=false"]
        );
        assert_eq!(result.changed, ["tab-width", "line-numbers", "expand-tab"]);

        run(&mut options, "expand-tab! ts+=3").unwrap();
        assert!(options.expand_tab);
        assert_eq!(options.tab_width, 5);
        run(&mut options, "invet ts-=1 search.case=sensitive").unwrap();
        assert!(!options.expand_tab);
        assert_eq!(options.tab_width, 4);
        assert_eq!(options.search.case, SearchCase::Sensitive);
    }

    #[test]
    fn showing_and_defaults() {
        let mut options = EditorOptions::default();
        assert_eq!(run(&mut options, "ts?").unwrap().shown, ["tab-width=4"]);
        assert_eq!(
            run(&mut options, "scroll-off").unwrap().shown,
            ["scroll-off=5"]
        );
        assert_eq!(
            run(&mut options, "").unwrap().shown,
            ["All options are at their defaults"]
        );
        run(&mut options, "ts=8 cursorline!").unwrap();
        let changed = run(&mut options, "").unwrap().shown;
        assert_eq!(changed, ["cursorline=false", "tab-width=8"]);
        run(&mut options, "ts& cursorline&").unwrap();
        assert_eq!(options, EditorOptions::default());
    }

    #[test]
    fn vim_switches_for_line_numbers_and_case() {
        let mut options = EditorOptions::default();
        run(&mut options, "nornu").unwrap();
        assert_eq!(options.line_numbers, LineNumbers::Absolute);
        run(&mut options, "nonu").unwrap();
        assert_eq!(options.line_numbers, LineNumbers::None);
        run(&mut options, "rnu").unwrap();
        assert_eq!(options.line_numbers, LineNumbers::Relative);
        run(&mut options, "nu").unwrap();
        assert_eq!(options.line_numbers, LineNumbers::Relative, "nu keeps rnu");
        run(&mut options, "number!").unwrap();
        assert_eq!(options.line_numbers, LineNumbers::None);
        assert_eq!(
            run(&mut options, "nu?").unwrap().shown,
            ["line-numbers=none"]
        );

        run(&mut options, "noic").unwrap();
        assert_eq!(options.search.case, SearchCase::Sensitive);
        run(&mut options, "ic").unwrap();
        assert_eq!(options.search.case, SearchCase::Insensitive);
        run(&mut options, "scs").unwrap();
        assert_eq!(options.search.case, SearchCase::Smart);
        run(&mut options, "noscs").unwrap();
        assert_eq!(options.search.case, SearchCase::Insensitive);
    }

    #[test]
    fn lists_and_quoted_text() {
        let mut options = EditorOptions::default();
        run(&mut options, "statusline-layout.right=[\"position\"]").unwrap();
        assert_eq!(options.statusline_layout.right.len(), 1);
        run(&mut options, "fallback-encoding=\"latin1\"").unwrap();
        assert_eq!(options.fallback_encoding.0.name(), "windows-1252");
    }

    #[test]
    fn mistakes_change_nothing() {
        let mut options = EditorOptions::default();
        let errors = [
            ("nope=1", "Unknown option: nope"),
            ("ts=two", "tab-width: not a number: two"),
            (
                "line-numbers=sideways",
                "line-numbers: unknown variant `sideways`, expected one of `absolute`, `relative`, `none`",
            ),
            ("expand-tab=maybe", "expand-tab: on or off, not maybe"),
            ("nots", "Unknown option: nots"),
            ("ts!", "tab-width: not a switch (on or off)"),
            (
                "nu=1",
                "number: takes no value (:set number or :set nonumber)",
            ),
            ("line-numbers+=1", "line-numbers: += needs a number option"),
        ];
        for (arg, message) in errors {
            assert_eq!(
                run(&mut options, &format!("ts=2 {arg}")),
                Err(message.into()),
                "{arg}"
            );
            assert_eq!(options, EditorOptions::default(), "{arg} changed nothing");
        }
    }

    #[test]
    fn names_for_completion() {
        let names = option_names();
        for name in ["tab-width", "search.case", "ts", "number", "watch-files"] {
            assert!(names.iter().any(|n| n == name), "{name}");
        }
    }
}
