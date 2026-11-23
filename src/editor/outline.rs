//! The file's symbols, as a language server gives them: the Outline view
//! in the sidebar (a built-in view of the plugin kind, its lines made
//! here), and the breadcrumbs above the text (`breadcrumbs`). Both ask for
//! them while they're shown, again after edits (at most twice a second),
//! and show where the cursor is in them.

use std::time::{Duration, Instant};

use super::lsp::{LspQuery, LspRequest, Reply};
use super::popup::{PopupLine, PopupSpan};
use super::sidebar_views::ViewSpec;
use super::{DocumentId, Editor, Mode};
use crate::config::Icons;
use crate::core::Position;

/// The Outline view's name (`:sidebar_view outline`, `activity outline`).
pub const OUTLINE: &str = "outline";

/// One symbol, in the file's order (a parent before what's in it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineSymbol {
    pub name: String,
    /// `function`, `struct`... (`features::symbol_kind`'s names).
    pub kind: String,
    /// How deep it is: 0 at the top of the file.
    pub depth: usize,
    /// Where it starts and ends (all of it), and its name.
    pub start: Position,
    pub end: Position,
    pub at: Position,
}

/// The symbols for a buffer, as it was at `edit_seq`.
#[derive(Debug, Clone, Default)]
pub struct Outline {
    document: Option<DocumentId>,
    edit_seq: u64,
    pub symbols: Vec<OutlineSymbol>,
    /// The last asking: for which buffer, at which edit, when.
    asked: Option<(DocumentId, u64, Instant)>,
}

/// How long after asking it asks again: for edits since, and with no
/// answer for the same text.
const ASK_EVERY: Duration = Duration::from_millis(500);
const ASK_AGAIN: Duration = Duration::from_secs(3);

/// A symbol kind's icon: plain, or the Nerd Font codicon.
pub fn kind_icon(kind: &str, icons: Icons) -> &'static str {
    let nerd = icons == Icons::Nerd;
    let pick = |plain: &'static str, codicon: &'static str| if nerd { codicon } else { plain };
    match kind {
        "function" | "method" | "constructor" => pick("ƒ", "\u{ea8c}"),
        "class" | "object" => pick("◆", "\u{eb5b}"),
        "struct" => pick("◆", "\u{ea91}"),
        "interface" => pick("◇", "\u{eb61}"),
        "enum" => pick("∈", "\u{ea95}"),
        "enum member" => pick("∙", "\u{eb5e}"),
        "field" => pick("•", "\u{eb5f}"),
        "property" => pick("•", "\u{eb65}"),
        "variable" => pick("ν", "\u{ea88}"),
        "constant" => pick("π", "\u{eb5d}"),
        "module" | "namespace" | "package" => pick("▣", "\u{ea8b}"),
        "key" => pick("⚷", "\u{ea93}"),
        "operator" => pick("±", "\u{eb64}"),
        "event" => pick("⚡", "\u{ea86}"),
        "file" => pick("▤", "\u{eb60}"),
        _ => pick("·", "\u{eb63}"),
    }
}

impl Editor {
    /// The Outline view, among the sidebar's (it's kept across plugin
    /// reloads).
    pub(super) fn add_outline_view(&mut self) {
        let icon = if self.options.icons == Icons::Nerd {
            "\u{eb86}"
        } else {
            "≣"
        };
        self.add_sidebar_views(vec![ViewSpec {
            name: OUTLINE.into(),
            title: "Outline".into(),
            keys: vec![
                ("<CR>".into(), "outline_go".into()),
                ("l".into(), "outline_go".into()),
                ("<Right>".into(), "outline_go".into()),
            ],
            refresh: Vec::new(),
            icon: Some(icon.into()),
        }]);
    }

    /// Whether the Outline view is what the sidebar shows.
    pub fn outline_shown(&self) -> bool {
        self.sidebar().is_some() && self.plugin_view().is_some_and(|v| v.spec.name == OUTLINE)
    }

    /// The symbols for the buffer shown, if they're its (possibly from a
    /// few edits ago).
    pub fn outline(&self) -> Option<&[OutlineSymbol]> {
        (self.outline.document == Some(self.document.id())).then_some(&self.outline.symbols[..])
    }

    /// Before drawing: asks the language server for the buffer's symbols
    /// if the Outline or the breadcrumbs show them and they're out of date
    /// (not again for the same edit, nor more than twice a second).
    pub fn ask_for_outline(&mut self) {
        let wanted = self.outline_shown() || self.options.breadcrumbs;
        if !wanted || self.document.path().is_none() {
            return;
        }
        let id = self.document.id();
        let buffer = self.document.buffer();
        let seq = buffer.edit_seq();
        if self.outline.document == Some(id) && self.outline.edit_seq == seq {
            return;
        }
        // Not again soon after asking: for the same edit, a few seconds
        // (the server may still be starting), for later ones, half a second.
        if let Some((doc, at_seq, when)) = self.outline.asked
            && doc == id
            && when.elapsed() < if at_seq == seq { ASK_AGAIN } else { ASK_EVERY }
        {
            return;
        }
        self.outline.asked = Some((id, seq, Instant::now()));
        self.lsp_requests.push(LspRequest {
            query: LspQuery::Outline,
            document: id,
            position: self.document.cursor(),
            buffer_id: buffer.id(),
            edit_seq: seq,
            reply: Reply::View,
        });
    }

    /// The server's answer: the symbols for `document` at `edit_seq`.
    pub fn set_outline(
        &mut self,
        document: DocumentId,
        edit_seq: u64,
        symbols: Vec<OutlineSymbol>,
    ) {
        self.outline.document = Some(document);
        self.outline.edit_seq = edit_seq;
        self.outline.symbols = symbols;
        self.refresh_plugin_view(Some(OUTLINE));
    }

    /// The symbols the cursor is in, outermost first (for the breadcrumbs
    /// and to show in the Outline): their indexes.
    pub fn symbols_at_cursor(&self) -> Vec<usize> {
        let Some(symbols) = self.outline() else {
            return Vec::new();
        };
        let cursor = self.document.cursor();
        let mut path: Vec<usize> = Vec::new();
        for (i, symbol) in symbols.iter().enumerate() {
            // In it, or on its start (a server that gives only the name's
            // place, or none, has an empty range).
            if symbol.start <= cursor && (cursor < symbol.end || cursor == symbol.start) {
                path.retain(|&p| symbols[p].depth < symbol.depth);
                path.push(i);
            }
        }
        path
    }

    /// The Outline view's lines: each symbol, indented, with its icon; the
    /// one the cursor's in selected (unless you're moving about in it).
    pub fn make_outline_lines(&mut self) {
        let icons = self.options.icons;
        let lines: Vec<PopupLine> = match self.outline() {
            None => vec![vec![PopupSpan {
                text: if self.document.path().is_none() {
                    "No symbols: the buffer has no file".into()
                } else {
                    "No symbols yet (they come from a language server)".into()
                },
                style: Some("ui.sidebar.empty".into()),
            }]],
            Some([]) => vec![vec![PopupSpan {
                text: "No symbols in this file".into(),
                style: Some("ui.sidebar.empty".into()),
            }]],
            Some(symbols) => symbols
                .iter()
                .map(|s| {
                    vec![
                        PopupSpan {
                            text: format!("{}{} ", "  ".repeat(s.depth), kind_icon(&s.kind, icons)),
                            style: Some("ui.sidebar.marker".into()),
                        },
                        PopupSpan {
                            text: s.name.clone(),
                            style: None,
                        },
                    ]
                })
                .collect(),
        };
        let current = self.symbols_at_cursor().last().copied();
        let focused = self.mode() == Mode::Sidebar;
        self.set_plugin_view_lines(OUTLINE, lines);
        if !focused && let Some(current) = current {
            self.select_plugin_view_line(OUTLINE, current);
        }
    }

    /// `<CR>` (or a click) in the Outline: the cursor to that symbol.
    pub fn outline_go(&mut self) -> Result<(), String> {
        let index = self
            .plugin_view()
            .filter(|v| v.spec.name == OUTLINE)
            .and_then(|v| v.selected())
            .ok_or("No symbol selected")?;
        let symbol = self
            .outline()
            .and_then(|s| s.get(index))
            .cloned()
            .ok_or("No symbols")?;
        self.push_jump();
        self.set_mode(Mode::Normal);
        let buffer = self.document.buffer();
        let row = symbol.at.row.min(buffer.line_count().saturating_sub(1));
        let col = symbol.at.col.min(buffer.line_len(row));
        self.document.set_cursor(Position::new(row, col));
        self.enforce_cursor_rules();
        self.reveal_cursor();
        Ok(())
    }

    /// A breadcrumb clicked: symbol `index` (the cursor to it).
    pub fn go_to_symbol(&mut self, index: usize) {
        let Some(symbol) = self.outline().and_then(|s| s.get(index)).cloned() else {
            return;
        };
        self.push_jump();
        if !matches!(self.mode(), Mode::Normal | Mode::Insert) {
            self.set_mode(Mode::Normal);
        }
        self.document.set_cursor(symbol.at);
        self.enforce_cursor_rules();
        self.reveal_cursor();
    }
}

impl Editor {
    /// `:theme`: asks the app for the theme picker.
    pub fn theme_picker_request(&mut self) {
        self.theme_picker_wanted = true;
    }

    /// Whether the theme picker was asked for, for the app.
    pub fn take_theme_picker_request(&mut self) -> bool {
        std::mem::take(&mut self.theme_picker_wanted)
    }
}
