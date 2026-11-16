//! The breadcrumbs (`breadcrumbs`): above the text, the file's path from
//! the working folder and the symbols the cursor is in, as VS Code's.
//!
//! ```text
//!  src › app › mod.rs › ƒ App › ƒ run
//! ```
//!
//! A folder clicked shows in the sidebar; a symbol, the cursor goes to it.
//! Too long for the row, it loses crumbs from the start. Each window has
//! its own; the symbols are only known for the active one's buffer.

use std::path::PathBuf;

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
use unicode_width::UnicodeWidthStr;

use crate::config::Theme;
use crate::editor::outline::kind_icon;
use crate::editor::{Editor, WindowId};

/// What a crumb is, for a click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Crumb {
    Folder(PathBuf),
    File,
    /// A symbol: its index in the outline.
    Symbol(usize),
}

/// Where each crumb went, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BreadcrumbsMap {
    pub row: Rect,
    pub crumbs: Vec<(u16, u16, Crumb)>,
    /// The window they're for.
    pub window: WindowId,
}

const SEPARATOR: &str = " › ";

/// The crumbs: text, scope and what they are.
fn crumbs(editor: &Editor, window: WindowId) -> Vec<(String, &'static str, Crumb)> {
    let mut out = Vec::new();
    let Some((document, _)) = editor.window_view(window) else {
        return out;
    };
    let Some(path) = document.path() else {
        out.push((
            document.name().into_owned(),
            "ui.breadcrumbs.file",
            Crumb::File,
        ));
        return out;
    };
    let here = std::env::current_dir().unwrap_or_default();
    let here = std::fs::canonicalize(&here).unwrap_or(here);
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let shown = full.strip_prefix(&here).unwrap_or(&full);
    let parts: Vec<_> = shown.iter().collect();
    let mut at = if shown.is_relative() {
        here.clone()
    } else {
        PathBuf::new()
    };
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        let text = part.to_string_lossy().into_owned();
        if i + 1 == parts.len() {
            let icons = editor.options.icons;
            let text = if icons == crate::config::Icons::Nerd {
                let (glyph, _) = super::icons::for_file(&full, &editor.languages);
                format!("{glyph} {text}")
            } else {
                text
            };
            out.push((text, "ui.breadcrumbs.file", Crumb::File));
        } else if !text.is_empty() && text != "/" {
            out.push((text, "ui.breadcrumbs", Crumb::Folder(at.clone())));
        }
    }
    let active = window == editor.windows().active();
    if let Some(symbols) = editor.outline().filter(|_| active) {
        for index in editor.symbols_at_cursor() {
            let symbol = &symbols[index];
            let icon = kind_icon(&symbol.kind, editor.options.icons);
            out.push((
                format!("{icon} {}", symbol.name),
                "ui.breadcrumbs.symbol",
                Crumb::Symbol(index),
            ));
        }
    }
    out
}

/// The crumbs that fit in `width`, from the end: their text, scope, kind
/// and column (from 0), and whether some were left out at the start.
fn laid_out(
    editor: &Editor,
    window: WindowId,
    width: u16,
) -> (Vec<(String, &'static str, Crumb, u16)>, bool) {
    let all = crumbs(editor, window);
    let mut first = 0;
    let total = |from: usize| -> usize {
        let crumbs = &all[from..];
        crumbs.iter().map(|(t, ..)| t.width()).sum::<usize>()
            + SEPARATOR.width() * crumbs.len().saturating_sub(1)
            + 1
            + if from > 0 { 2 } else { 0 }
    };
    while first + 1 < all.len() && total(first) > width as usize {
        first += 1;
    }
    let mut x: u16 = if first > 0 { 3 } else { 1 };
    let mut out = Vec::new();
    for (i, (text, scope, crumb)) in all.into_iter().enumerate().skip(first) {
        if i > first {
            x += SEPARATOR.width() as u16;
        }
        let w = text.width() as u16;
        out.push((text, scope, crumb, x));
        x += w;
    }
    (out, first > 0)
}

pub struct Breadcrumbs<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
    window: WindowId,
}

impl<'a> Breadcrumbs<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme, window: WindowId) -> Self {
        Self {
            editor,
            theme,
            window,
        }
    }

    pub fn map(editor: &Editor, row: Rect, window: WindowId) -> BreadcrumbsMap {
        let (crumbs, _) = laid_out(editor, window, row.width);
        BreadcrumbsMap {
            row,
            crumbs: crumbs
                .into_iter()
                .map(|(text, _, crumb, x)| (row.x + x, row.x + x + text.width() as u16, crumb))
                .filter(|(from, _, _)| *from < row.right())
                .collect(),
            window,
        }
    }
}

impl Widget for Breadcrumbs<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        buf.set_style(area, theme.get("ui.breadcrumbs"));
        let (crumbs, cut) = laid_out(self.editor, self.window, area.width);
        let separator = theme.get("ui.breadcrumbs.separator");
        if cut {
            buf.set_string(area.x + 1, area.y, "…", separator);
        }
        for (i, (text, scope, _, x)) in crumbs.iter().enumerate() {
            if i > 0 {
                let at = area.x + x - SEPARATOR.width() as u16;
                if at < area.right() {
                    buf.set_string(at, area.y, SEPARATOR, separator);
                }
            }
            let at = area.x + x;
            if at >= area.right() {
                break;
            }
            let room = (area.right() - at) as usize;
            buf.set_stringn(at, area.y, text, room, theme.get(scope));
        }
    }
}
