//! Merge conflicts in a buffer: `]x` / `[x` between them, and keeping our
//! side, their side or both (`<Space>co` / `ct` / `cb`), each one undo
//! step. The conflicts themselves are found with the git changes (before
//! drawing), for tinting and the hints on their markers.

use super::{Editor, Message, VirtualText};
use crate::core::{Position, motion};
use crate::git::{self, Conflict};

/// Which side of a conflict to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Ours,
    Theirs,
    Both,
}

impl Keep {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ours" => Some(Keep::Ours),
            "theirs" => Some(Keep::Theirs),
            "both" => Some(Keep::Both),
            _ => None,
        }
    }
}

/// The hint after a conflict's markers.
const OURS_HINT: &str = "current change · <Space>co keep · <Space>cb keep both";
const THEIRS_HINT: &str = "incoming change · <Space>ct keep";

/// The virtual text that names each conflict's sides, on its markers.
pub fn conflict_hints(conflicts: &[Conflict]) -> Vec<VirtualText> {
    let hint = |row, text: &str| VirtualText {
        at: Position::new(row, 0),
        text: text.to_string(),
        style: "ui.virtual.conflict".into(),
        source: "conflicts".into(),
        end_of_line: true,
    };
    conflicts
        .iter()
        .flat_map(|c| [hint(c.start, OURS_HINT), hint(c.end, THEIRS_HINT)])
        .collect()
}

impl Editor {
    /// The active buffer's conflicts, as its text is now.
    fn conflicts_now(&self) -> Vec<Conflict> {
        let buffer = self.document.buffer();
        let lines: Vec<String> = (0..buffer.line_count())
            .map(|row| buffer.line(row).into_owned())
            .collect();
        git::conflicts(lines.iter().map(String::as_str))
    }

    /// `]x` / `[x`: to the `count`th next (or previous) conflict.
    pub fn goto_conflict(&mut self, forward: bool, count: usize) -> Result<(), String> {
        let conflicts = self.conflicts_now();
        if conflicts.is_empty() {
            return Err("No merge conflicts in this file".into());
        }
        let mut from = self.document.cursor().row;
        let mut target = None;
        for _ in 0..count.max(1) {
            let next = if forward {
                conflicts.iter().position(|c| c.start > from)
            } else {
                conflicts.iter().rposition(|c| c.start < from)
            };
            match next {
                Some(i) => {
                    target = Some(i);
                    from = conflicts[i].start;
                }
                None => break,
            }
        }
        let index = target.ok_or(if forward {
            "No more conflicts below"
        } else {
            "No more conflicts above"
        })?;
        self.push_jump();
        self.document
            .set_cursor(Position::new(conflicts[index].start, 0));
        self.reveal_cursor();
        self.set_message(Message::info(format!(
            "Conflict {} of {}: <Space>co keeps ours, ct theirs, cb both",
            index + 1,
            conflicts.len()
        )));
        Ok(())
    }

    /// Opening a conflicted file: to its first conflict.
    pub fn goto_first_conflict(&mut self) {
        self.document.set_cursor(Position::new(0, 0));
        if let Some(first) = self.conflicts_now().first() {
            self.document.set_cursor(Position::new(first.start, 0));
            self.reveal_cursor();
        }
    }

    /// Resolves the conflict the cursor's in by keeping `keep`.
    pub fn keep_conflict(&mut self, keep: Keep) -> Result<(), String> {
        let conflicts = self.conflicts_now();
        let row = self.document.cursor().row;
        let conflict =
            *conflicts
                .iter()
                .find(|c| c.contains(row))
                .ok_or(if conflicts.is_empty() {
                    "No merge conflicts in this file"
                } else {
                    "The cursor isn't in a conflict (]x goes to the next)"
                })?;
        let buffer = self.document.buffer();
        let line = |row| buffer.line(row).into_owned();
        let mut kept: Vec<String> = Vec::new();
        if matches!(keep, Keep::Ours | Keep::Both) {
            kept.extend(conflict.ours().map(line));
        }
        if matches!(keep, Keep::Theirs | Keep::Both) {
            kept.extend(conflict.theirs().map(line));
        }
        let doc = &mut self.document;
        if kept.is_empty() {
            doc.delete_lines(conflict.start, conflict.end);
        } else {
            // The conflict's rows become the kept ones (the line break after
            // the last marker stays).
            let end = doc.buffer().line_end(conflict.end);
            doc.delete(Position::new(conflict.start, 0), end);
            doc.insert(Position::new(conflict.start, 0), &kept.join("\n"));
        }
        let row = conflict
            .start
            .min(doc.buffer().line_count().saturating_sub(1));
        let col = motion::first_non_blank(doc.buffer(), row);
        doc.set_cursor(Position::new(row, col));
        let left = conflicts.len() - 1;
        let what = match keep {
            Keep::Ours => "ours",
            Keep::Theirs => "theirs",
            Keep::Both => "both sides",
        };
        self.set_message(Message::info(match left {
            0 => format!("Kept {what}: no conflicts left (save, then stage it)"),
            1 => format!("Kept {what}: 1 conflict left (]x)"),
            n => format!("Kept {what}: {n} conflicts left (]x)"),
        }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Document;

    const TEXT: &str = "a\n<<<<<<< HEAD\nours\n=======\ntheirs 1\ntheirs 2\n>>>>>>> feature\nb\n\
                        <<<<<<< HEAD\nx\n=======\ny\n>>>>>>> feature";

    fn editor_with(text: &str) -> Editor {
        Editor {
            document: Document::with_text(text),
            ..Editor::default()
        }
    }

    fn editor() -> Editor {
        editor_with(TEXT)
    }

    fn text(editor: &Editor) -> String {
        editor.document.buffer().text()
    }

    #[test]
    fn jumps_between_conflicts() {
        let mut editor = editor();
        editor.goto_conflict(true, 1).unwrap();
        assert_eq!(editor.document.cursor().row, 1);
        editor.goto_conflict(true, 1).unwrap();
        assert_eq!(editor.document.cursor().row, 8);
        assert!(editor.goto_conflict(true, 1).is_err());
        editor.goto_conflict(false, 1).unwrap();
        assert_eq!(editor.document.cursor().row, 1);
        editor.document = Document::with_text("none");
        assert!(editor.goto_conflict(true, 1).is_err());
    }

    #[test]
    fn keeps_a_side_as_one_undo_step() {
        let mut editor = editor();
        editor.document.set_cursor(Position::new(4, 0));
        editor.keep_conflict(Keep::Theirs).unwrap();
        editor.document.commit();
        assert_eq!(
            text(&editor),
            "a\ntheirs 1\ntheirs 2\nb\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> feature"
        );
        assert_eq!(editor.document.cursor().row, 1);
        assert!(editor.document.undo());
        assert_eq!(text(&editor), TEXT);

        editor.document.set_cursor(Position::new(0, 0));
        assert!(editor.keep_conflict(Keep::Ours).is_err(), "not in one");
        editor.document.set_cursor(Position::new(1, 0));
        editor.keep_conflict(Keep::Both).unwrap();
        assert!(text(&editor).starts_with("a\nours\ntheirs 1\ntheirs 2\nb\n"));
        // The last one, at the end of the file.
        editor.document.set_cursor(Position::new(9, 0));
        editor.keep_conflict(Keep::Ours).unwrap();
        assert_eq!(text(&editor), "a\nours\ntheirs 1\ntheirs 2\nb\nx");
    }

    #[test]
    fn an_empty_side_drops_the_lines() {
        let mut editor = editor_with("a\n<<<<<<< HEAD\n=======\nt\n>>>>>>> f\nb");
        editor.document.set_cursor(Position::new(2, 0));
        editor.keep_conflict(Keep::Ours).unwrap();
        assert_eq!(text(&editor), "a\nb");
    }

    #[test]
    fn hints_go_on_the_markers() {
        let conflicts = git::conflicts(TEXT.lines());
        let hints = conflict_hints(&conflicts);
        let rows: Vec<usize> = hints.iter().map(|h| h.at.row).collect();
        assert_eq!(rows, [1, 6, 8, 12]);
        assert!(hints.iter().all(|h| h.end_of_line));
    }
}
