//! Scope-based themes, e.g. `ui.linenr.selected`.
//!
//! Scopes are dot-separated and fall back to their parent, which keeps themes
//! short and leaves room for syntax scopes (`keyword.control`, ...) later.

use std::collections::HashMap;
use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

/// A style as written in config: either just a foreground colour, or a table.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StyleSpec {
    Fg(String),
    Full {
        fg: Option<String>,
        bg: Option<String>,
        /// The colour of an underline (with the `underlined` modifier), for
        /// terminals that can show one.
        underline: Option<String>,
        #[serde(default)]
        modifiers: Vec<String>,
    },
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(try_from = "HashMap<String, StyleSpec>")]
pub struct Theme {
    styles: HashMap<String, Style>,
}

impl Theme {
    /// Resolves a scope, falling back through parent scopes. Unknown scopes
    /// resolve to the empty style.
    pub fn get(&self, scope: &str) -> Style {
        let mut scope = scope;
        loop {
            if let Some(style) = self.styles.get(scope) {
                return *style;
            }
            match scope.rsplit_once('.') {
                Some((parent, _)) => scope = parent,
                None => return Style::default(),
            }
        }
    }

    pub fn set(&mut self, scope: impl Into<String>, style: Style) {
        self.styles.insert(scope.into(), style);
    }
}

impl TryFrom<HashMap<String, StyleSpec>> for Theme {
    type Error = String;

    fn try_from(specs: HashMap<String, StyleSpec>) -> Result<Self, String> {
        let styles = specs
            .into_iter()
            .map(|(scope, spec)| {
                let style = parse_style(&spec).map_err(|e| format!("theme '{scope}': {e}"))?;
                Ok((scope, style))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { styles })
    }
}

fn parse_style(spec: &StyleSpec) -> Result<Style, String> {
    match spec {
        StyleSpec::Fg(fg) => Ok(Style::new().fg(parse_color(fg)?)),
        StyleSpec::Full {
            fg,
            bg,
            underline,
            modifiers,
        } => {
            let mut style = Style::new();
            if let Some(fg) = fg {
                style = style.fg(parse_color(fg)?);
            }
            if let Some(bg) = bg {
                style = style.bg(parse_color(bg)?);
            }
            if let Some(color) = underline {
                style = style.underline_color(parse_color(color)?);
            }
            for name in modifiers {
                style = style.add_modifier(parse_modifier(name)?);
            }
            Ok(style)
        }
    }
}

fn parse_color(s: &str) -> Result<Color, String> {
    Color::from_str(s).map_err(|_| format!("invalid color '{s}'"))
}

fn parse_modifier(s: &str) -> Result<Modifier, String> {
    Ok(match s {
        "bold" => Modifier::BOLD,
        "dim" => Modifier::DIM,
        "italic" => Modifier::ITALIC,
        "underlined" => Modifier::UNDERLINED,
        "reversed" => Modifier::REVERSED,
        "crossed_out" => Modifier::CROSSED_OUT,
        _ => return Err(format!("invalid modifier '{s}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(toml: &str) -> Result<Theme, String> {
        let specs: HashMap<String, StyleSpec> = toml::from_str(toml).map_err(|e| e.to_string())?;
        Theme::try_from(specs)
    }

    #[test]
    fn parses_shorthand_and_full_styles() {
        let theme = theme(
            r##"
            "ui.text" = "#ff0000"
            "ui.linenr" = { fg = "blue", bg = "#000000", modifiers = ["bold", "italic"] }
            "##,
        )
        .unwrap();

        assert_eq!(theme.get("ui.text"), Style::new().fg(Color::Rgb(255, 0, 0)));
        assert_eq!(
            theme.get("ui.linenr"),
            Style::new()
                .fg(Color::Blue)
                .bg(Color::Rgb(0, 0, 0))
                .add_modifier(Modifier::BOLD | Modifier::ITALIC)
        );
    }

    #[test]
    fn falls_back_to_parent_scope() {
        let theme = theme(r#""ui.linenr" = "red""#).unwrap();
        assert_eq!(theme.get("ui.linenr.selected"), Style::new().fg(Color::Red));
        assert_eq!(theme.get("other.scope"), Style::default());
    }

    #[test]
    fn rejects_invalid_colors() {
        let err = theme(r#""ui.text" = "not-a-colour""#).unwrap_err();
        assert!(
            err.contains("ui.text") && err.contains("invalid color"),
            "{err}"
        );
    }

    #[test]
    fn rejects_invalid_modifiers() {
        let err = theme(r#""ui.text" = { modifiers = ["sparkly"] }"#).unwrap_err();
        assert!(err.contains("invalid modifier"), "{err}");
    }
}
