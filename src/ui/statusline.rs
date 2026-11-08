//! Status lines. The active window's shows the mode, document name, pending
//! keys and position; other windows' (with `statusline = "windows"`) show
//! just their document's name and their cursor's position, dimmed.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

use unicode_width::UnicodeWidthStr;

use super::text::truncate_start;
use crate::config::{StatusElement, Theme};
use crate::core::{LineEnding, Position};
use crate::editor::{Document, Editor, Mode, SelectionKind};

pub struct StatusLine<'a> {
    theme: &'a Theme,
    kind: Kind<'a>,
}

enum Kind<'a> {
    Active {
        editor: &'a Editor,
        /// Partially typed count/key sequence, e.g. `"2g"`.
        pending: &'a str,
    },
    Inactive {
        document: &'a Document,
        cursor: Position,
    },
}

impl<'a> StatusLine<'a> {
    /// The active window's status line.
    pub fn new(editor: &'a Editor, theme: &'a Theme, pending: &'a str) -> Self {
        Self {
            theme,
            kind: Kind::Active { editor, pending },
        }
    }

    /// Another window's status line: its document and cursor.
    pub fn inactive(document: &'a Document, cursor: Position, theme: &'a Theme) -> Self {
        Self {
            theme,
            kind: Kind::Inactive { document, cursor },
        }
    }
}

impl Widget for StatusLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.kind {
            Kind::Active { editor, pending } => Active {
                editor,
                theme: self.theme,
                pending,
            }
            .render(area, buf),
            Kind::Inactive { document, cursor } => {
                render_inactive(document, cursor, self.theme, area, buf);
            }
        }
    }
}

/// ` name [+]` on the left and `row:col` on the right, dimmed.
fn render_inactive(
    document: &Document,
    cursor: Position,
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
) {
    let style = theme.get("ui.statusline.inactive");
    buf.set_style(area, style);
    let right = Line::from(format!(" {}:{} ", cursor.row + 1, cursor.col + 1)).right_aligned();
    let modified = if document.is_modified() { " [+]" } else { "" };
    let budget = (area.width as usize).saturating_sub(right.width() + modified.width() + 2);
    let name = truncate_start(&document.name(), budget);
    Line::from(format!(" {name}{modified} ")).render(area, buf);
    right.render(area, buf);
}

/// The fields the active status line reads.
struct Active<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
    pending: &'a str,
}

impl Active<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        buf.set_style(area, self.theme.get("ui.statusline"));
        let layout = &self.editor.options.statusline_layout;
        let sections = [&layout.left, &layout.center, &layout.right];

        // File names get whatever the other elements leave, so a long path
        // is shortened from the left and everything else stays visible.
        let others: usize = sections
            .iter()
            .flat_map(|section| section.iter())
            .filter(|&e| *e != StatusElement::FileName)
            .map(|e| self.element(e, 0).width())
            .sum();
        let names = sections
            .iter()
            .flat_map(|section| section.iter())
            .filter(|&e| *e == StatusElement::FileName)
            .count();
        let budget = (area.width as usize).saturating_sub(others + 2 * names) / names.max(1);

        let line = |section: &[StatusElement]| {
            Line::from(
                section
                    .iter()
                    .map(|e| self.element(e, budget))
                    .collect::<Vec<_>>(),
            )
        };
        let (left, center, right) = (
            line(&layout.left),
            line(&layout.center),
            line(&layout.right),
        );
        let (lw, cw, rw) = (left.width(), center.width(), right.width());
        left.render(area, buf);
        right.right_aligned().render(area, buf);
        // The center section only when it fits between the other two.
        if cw > 0 && lw + cw + rw <= area.width as usize {
            let x = (area.width as usize - cw) / 2;
            let x = x.clamp(lw, area.width as usize - rw - cw) as u16;
            center.render(Rect::new(area.x + x, area.y, cw as u16, 1), buf);
        }
    }

    /// One element's text and style. `name_budget` is the room for the
    /// file name's text.
    fn element(&self, element: &StatusElement, name_budget: usize) -> Span<'static> {
        let editor = self.editor;
        let document = &editor.document;
        let cursor = document.cursor();
        let styled = |text: String, scope: &str| Span::styled(text, self.theme.get(scope));
        match element {
            StatusElement::Mode => {
                let mode = editor.mode();
                let label = match editor.selection() {
                    _ if mode == Mode::Command && editor.command_line.prompt().is_search() => {
                        "SEARCH".to_string()
                    }
                    Some(s) if s.kind == SelectionKind::Line => "V-LINE".to_string(),
                    Some(s) if s.kind == SelectionKind::Block => "V-BLOCK".to_string(),
                    _ if editor.replacing() => "REPLACE".to_string(),
                    _ => mode.to_string(),
                };
                let scope = if editor.replacing() {
                    "replace"
                } else {
                    mode.id()
                };
                styled(format!(" {label} "), &format!("ui.statusline.{scope}"))
            }
            StatusElement::FileName => Span::raw(format!(
                " {} ",
                truncate_start(&document.name(), name_budget)
            )),
            StatusElement::FileBaseName => {
                let name = document.name();
                let base = std::path::Path::new(name.as_ref())
                    .file_name()
                    .map_or_else(|| name.to_string(), |n| n.to_string_lossy().into_owned());
                Span::raw(format!(" {base} "))
            }
            StatusElement::Modified => {
                let text = if document.is_modified() { "[+] " } else { "" };
                styled(text.to_string(), "ui.statusline.modified")
            }
            StatusElement::Position => {
                Span::raw(format!(" {}:{} ", cursor.row + 1, cursor.col + 1))
            }
            StatusElement::PositionPercentage => {
                let lines = document.buffer().line_count();
                let percent = (cursor.row + 1) * 100 / lines.max(1);
                Span::raw(format!(" {percent}% "))
            }
            StatusElement::TotalLines => {
                Span::raw(format!(" {} lines ", document.buffer().line_count()))
            }
            StatusElement::FileType => Span::raw(
                editor
                    .language_of(document)
                    .map(|name| format!(" {name} "))
                    .unwrap_or_default(),
            ),
            StatusElement::FileEncoding => {
                let encoding = document.format().encoding;
                Span::raw(if encoding.is_plain_utf8() {
                    String::new()
                } else {
                    format!(" {} ", encoding.name())
                })
            }
            StatusElement::FileLineEnding => Span::raw(match document.format().line_ending {
                LineEnding::Lf => " LF ".to_string(),
                LineEnding::Crlf => " CRLF ".to_string(),
            }),
            StatusElement::Indent => Span::raw(format!(" {} ", editor.indent_style())),
            StatusElement::Diagnostics => {
                use crate::editor::diagnostics::Severity;
                let all = editor.diagnostics(document.id());
                let count = |s: Severity| all.iter().filter(|d| d.severity == s).count();
                let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
                let mut text = String::new();
                if errors > 0 {
                    text.push_str(&format!(" E{errors}"));
                }
                if warnings > 0 {
                    text.push_str(&format!(" W{warnings}"));
                }
                let scope = if errors > 0 {
                    "ui.statusline.diagnostic.error"
                } else {
                    "ui.statusline.diagnostic.warning"
                };
                if text.is_empty() {
                    Span::raw("")
                } else {
                    styled(format!("{text} "), scope)
                }
            }
            StatusElement::Plugin(name) => Span::styled(
                editor
                    .plugin_status(name)
                    .filter(|text| !text.is_empty())
                    .map(|text| format!(" {text} "))
                    .unwrap_or_default(),
                self.theme.get(&format!("ui.statusline.plugin.{name}")),
            ),
            StatusElement::PendingKeys => {
                styled(format!("{} ", self.pending), "ui.statusline.pending")
            }
            StatusElement::Recording => styled(
                editor
                    .recording_macro()
                    .map(|r| format!("recording @{r}  "))
                    .unwrap_or_default(),
                "ui.statusline.recording",
            ),
            StatusElement::Separator => Span::raw("│"),
            StatusElement::Spacer => Span::raw(" "),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position;
    use crate::editor::{Document, Mode};

    fn render(editor: &Editor, pending: &str) -> String {
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        StatusLine::new(editor, &Theme::default(), pending).render(area, &mut buf);
        (0..area.width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    #[test]
    fn shows_mode_name_and_position() {
        let mut editor = Editor::with_document(Document::with_text("abc\ndef"));
        editor.document.set_cursor(Position::new(1, 2));
        let text = render(&editor, "");
        assert!(text.starts_with(" NORMAL  [scratch]"), "{text:?}");
        assert!(text.ends_with(" 2:3 "), "{text:?}");
    }

    #[test]
    fn shows_current_mode() {
        let mut editor = Editor::default();
        editor.set_mode(Mode::Insert);
        assert!(render(&editor, "").starts_with(" INSERT "));
    }

    #[test]
    fn shows_modified_marker() {
        let mut editor = Editor::default();
        assert!(!render(&editor, "").contains("[+]"));
        editor.document.insert_str("x");
        assert!(render(&editor, "").contains(" [scratch] [+] "));
    }

    #[test]
    fn long_names_are_shortened_from_the_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deeply/nested/directory/structure/file.rs");
        let (document, _) = Document::open(&path).unwrap();
        let mut editor = Editor::with_document(document);
        editor.document.insert_str("x");

        let text = render(&editor, "");
        assert!(text.contains("…"), "{text:?}");
        assert!(text.contains("/file.rs [+]"), "{text:?}");
        assert!(text.contains(" rust "), "language shown: {text:?}");
        assert!(text.ends_with(" 1:2 "), "{text:?}");
    }

    #[test]
    fn shows_visual_and_visual_line() {
        let mut editor = Editor::default();
        editor.toggle_visual(SelectionKind::Char);
        assert!(render(&editor, "").starts_with(" VISUAL "));
        editor.toggle_visual(SelectionKind::Line);
        assert!(render(&editor, "").starts_with(" V-LINE "));
    }

    #[test]
    fn shows_search_label() {
        let mut editor = Editor::default();
        editor.open_search(true);
        assert!(render(&editor, "").starts_with(" SEARCH "));
    }

    #[test]
    fn shows_macro_recording() {
        let mut editor = Editor::default();
        editor.set_recording_macro(Some('q'));
        assert!(render(&editor, "").contains("recording @q"));
    }

    #[test]
    fn shows_pending_keys() {
        let text = render(&Editor::default(), "3g");
        assert!(text.contains("3g"), "{text:?}");
    }

    fn layout(
        left: &[StatusElement],
        center: &[StatusElement],
        right: &[StatusElement],
    ) -> crate::config::StatuslineLayout {
        crate::config::StatuslineLayout {
            left: left.to_vec(),
            center: center.to_vec(),
            right: right.to_vec(),
        }
    }

    #[test]
    fn a_custom_layout_shows_its_elements_in_order() {
        use StatusElement::*;
        let mut editor = Editor::with_document(Document::with_text("a\nb\nc\nd"));
        editor.document.set_cursor(crate::core::Position::new(1, 0));
        editor.options.statusline_layout = layout(
            &[FileLineEnding, Separator, TotalLines],
            &[],
            &[PositionPercentage, Mode],
        );
        let text = render(&editor, "");
        assert!(text.starts_with(" LF │ 4 lines "), "{text:?}");
        assert!(text.ends_with(" 50%  NORMAL "), "{text:?}");
    }

    #[test]
    fn indent_shows_the_buffers_indent_style() {
        use StatusElement::*;
        let mut editor = Editor::default();
        editor.options.statusline_layout = layout(&[Indent], &[], &[]);
        assert!(render(&editor, "").starts_with(" 4 spaces "));
        editor
            .document
            .choose_indent(Some(crate::core::IndentUnit::Tabs));
        assert!(render(&editor, "").starts_with(" tabs "));
    }

    #[test]
    fn the_center_is_centered_and_long_names_make_room_for_it() {
        use StatusElement::*;
        let mut editor = Editor::default();
        editor.options.statusline_layout = layout(&[Mode], &[FileBaseName], &[Position]);
        let text = render(&editor, "");
        let center = text.find("[scratch]").unwrap();
        assert!((15..=17).contains(&center), "{text:?}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/very/long/path/that/fills/the/line.rs");
        let (document, _) = Document::open(&path).unwrap();
        let mut editor = Editor::with_document(document);
        editor.options.statusline_layout = layout(&[FileName], &[Mode], &[Position]);
        let text = render(&editor, "");
        assert!(
            text.contains("…") && text.contains("line.rs") && text.contains("NORMAL"),
            "a long name is shortened to leave room for the rest: {text:?}"
        );
    }

    #[test]
    fn an_empty_layout_is_a_blank_line() {
        let mut editor = Editor::default();
        editor.options.statusline_layout = layout(&[], &[], &[]);
        assert_eq!(render(&editor, "").trim(), "");
    }
}
