use serde::Deserialize;

use crate::editor::Mode;

/// Behavioural options from the `[editor]` config section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct EditorOptions {
    pub tab_width: usize,
    pub line_numbers: LineNumbers,
    pub cursorline: bool,
    pub scroll_off: usize,
    pub cursor_shape: CursorShapes,
}

impl Default for EditorOptions {
    fn default() -> Self {
        super::Config::default().editor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineNumbers {
    Absolute,
    /// Relative to the cursor line, with the cursor line itself absolute.
    Relative,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorShapes {
    pub normal: CursorShape,
    pub insert: CursorShape,
    pub command: CursorShape,
    pub operator: CursorShape,
}

impl CursorShapes {
    pub fn for_mode(&self, mode: Mode) -> CursorShape {
        match mode {
            Mode::Normal => self.normal,
            Mode::Insert => self.insert,
            Mode::Command => self.command,
            Mode::OperatorPending => self.operator,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_shape_per_mode() {
        let shapes = CursorShapes {
            normal: CursorShape::Block,
            insert: CursorShape::Bar,
            command: CursorShape::Underline,
            operator: CursorShape::Block,
        };
        assert_eq!(shapes.for_mode(Mode::Insert), CursorShape::Bar);
        assert_eq!(shapes.for_mode(Mode::Command), CursorShape::Underline);
    }
}
