//! The chat as lines for the agent sidebar: what you asked, what the
//! agent said (its Markdown: headings, lists, code, tables fitted to the
//! sidebar's width), its thoughts, tool calls (opened, their output and
//! changes line by line), its plan and notices. The sidebar wraps them
//! (`ui/agent.rs`).

use super::chat::{AgentChat, ChatEntry, ToolCall, ToolDiff, ToolStatus, short_path};
use crate::editor::popup::PopupSpan;

/// A line of the transcript, before it's wrapped to the sidebar's width.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatLine {
    /// The text, each piece with its theme scope.
    pub spans: Vec<PopupSpan>,
    /// Columns its wrapped rows are indented by (under a bullet...).
    pub indent: u16,
    /// A scope the whole row's painted with (what you asked).
    pub fill: Option<String>,
    /// A click on it opens or closes this tool call.
    pub tool: Option<String>,
}

/// At most this many lines of a tool's output are shown, opened.
const TOOL_LINES: usize = 20;
/// And of each of its changes.
const DIFF_LINES: usize = 40;

impl AgentChat {
    /// The transcript as lines, for the sidebar `width` columns wide (which
    /// wraps them; tables are fitted to it here).
    pub fn transcript(&self, width: u16) -> Vec<ChatLine> {
        let chat = self;
        let mut lines: Vec<ChatLine> = Vec::new();
        let gap = |lines: &mut Vec<ChatLine>| {
            if lines.last().is_some_and(|l| !l.spans.is_empty()) {
                lines.push(ChatLine::default());
            }
        };
        for entry in &chat.entries {
            match entry {
                ChatEntry::User(text) => {
                    gap(&mut lines);
                    let fill = Some("ui.sidebar.agent.user".to_string());
                    for (i, line) in text.lines().enumerate() {
                        let mark = if i == 0 { "› " } else { "  " };
                        lines.push(ChatLine {
                            spans: vec![
                                span(mark, "ui.sidebar.agent.user.mark"),
                                span(line, "ui.sidebar.agent.user"),
                            ],
                            indent: 2,
                            fill: fill.clone(),
                            tool: None,
                        });
                    }
                }
                ChatEntry::Agent(text) => {
                    gap(&mut lines);
                    markdown_lines(text, usize::from(width), &mut lines);
                }
                ChatEntry::Thought(text) => {
                    gap(&mut lines);
                    for line in text.lines().filter(|l| !l.trim().is_empty()) {
                        lines.push(ChatLine {
                            spans: vec![span(line, "ui.sidebar.agent.thought")],
                            ..ChatLine::default()
                        });
                    }
                }
                ChatEntry::Tool(tool) => {
                    if !matches!(lines.last(), Some(l) if l.tool.is_some()) {
                        gap(&mut lines);
                    }
                    tool_lines(tool, chat.is_expanded(&tool.id), &mut lines);
                }
                ChatEntry::Plan(steps) => {
                    gap(&mut lines);
                    lines.push(ChatLine {
                        spans: vec![span("Plan", "ui.sidebar.agent.heading")],
                        ..ChatLine::default()
                    });
                    for step in steps {
                        let (mark, style) = match step.status.as_str() {
                            "completed" => ("☑ ", "ui.sidebar.agent.plan.done"),
                            "in_progress" => ("▸ ", "ui.sidebar.agent.plan.current"),
                            _ => ("☐ ", "ui.sidebar.agent.plan"),
                        };
                        lines.push(ChatLine {
                            spans: vec![span(mark, style), span(&step.text, style)],
                            indent: 2,
                            ..ChatLine::default()
                        });
                    }
                }
                ChatEntry::Notice(text) => {
                    gap(&mut lines);
                    lines.push(ChatLine {
                        spans: vec![span(text, "ui.sidebar.agent.notice")],
                        ..ChatLine::default()
                    });
                }
            }
        }
        lines
    }
}

fn span(text: &str, style: &str) -> PopupSpan {
    PopupSpan {
        text: text.to_string(),
        style: Some(style.to_string()),
    }
}

/// A tool call: its title (a click opens it), what it changed, and opened,
/// its output and the changes line by line.
fn tool_lines(tool: &ToolCall, expanded: bool, lines: &mut Vec<ChatLine>) {
    let (mark, style) = match tool.status {
        ToolStatus::Pending => ("○ ", "ui.sidebar.agent.tool.pending"),
        ToolStatus::Running => ("◐ ", "ui.sidebar.agent.tool.running"),
        ToolStatus::Done => ("✓ ", "ui.sidebar.agent.tool.done"),
        ToolStatus::Failed => ("✗ ", "ui.sidebar.agent.tool.failed"),
    };
    let more = !tool.text.trim().is_empty() || !tool.diffs.is_empty();
    let mut spans = vec![
        span(mark, style),
        span(&tool.title, "ui.sidebar.agent.tool"),
    ];
    if more {
        let arrow = if expanded { " ▾" } else { " ▸" };
        spans.push(span(arrow, "ui.sidebar.agent.hint"));
    }
    let id = Some(tool.id.clone());
    lines.push(ChatLine {
        spans,
        indent: 2,
        fill: None,
        tool: id.clone(),
    });
    for diff in &tool.diffs {
        let (added, removed) = diff_counts(diff);
        lines.push(ChatLine {
            spans: vec![
                span(
                    &format!("  {} ", short_path(&diff.path)),
                    "ui.sidebar.agent.tool",
                ),
                span(&format!("+{added}"), "ui.sidebar.agent.added"),
                span(" ", "ui.sidebar.agent.tool"),
                span(&format!("-{removed}"), "ui.sidebar.agent.removed"),
            ],
            indent: 4,
            fill: None,
            tool: id.clone(),
        });
    }
    if !expanded {
        return;
    }
    let output: Vec<&str> = tool.text.lines().collect();
    for line in output.iter().take(TOOL_LINES) {
        lines.push(ChatLine {
            spans: vec![span(&format!("  {line}"), "ui.sidebar.agent.output")],
            indent: 2,
            ..ChatLine::default()
        });
    }
    if output.len() > TOOL_LINES {
        lines.push(ChatLine {
            spans: vec![span(
                &format!("  … {} more lines", output.len() - TOOL_LINES),
                "ui.sidebar.agent.hint",
            )],
            ..ChatLine::default()
        });
    }
    for diff in &tool.diffs {
        diff_lines(diff, lines);
    }
}

/// Lines added and removed by a change.
fn diff_counts(diff: &ToolDiff) -> (usize, usize) {
    let old = diff.old.as_deref().unwrap_or_default();
    let mut counts = (0, 0);
    for change in similar::TextDiff::from_lines(old, &diff.new).iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => counts.0 += 1,
            similar::ChangeTag::Delete => counts.1 += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    counts
}

/// A change line by line: what was taken out and put in, with a line
/// around each.
fn diff_lines(diff: &ToolDiff, lines: &mut Vec<ChatLine>) {
    let old = diff.old.as_deref().unwrap_or_default();
    let text = similar::TextDiff::from_lines(old, &diff.new);
    lines.push(ChatLine {
        spans: vec![span(
            &format!("  {}", short_path(&diff.path)),
            "ui.sidebar.agent.heading",
        )],
        ..ChatLine::default()
    });
    let mut shown = 0;
    let mut left = 0;
    for (i, group) in text.grouped_ops(1).iter().enumerate() {
        if i > 0 && shown < DIFF_LINES {
            lines.push(ChatLine {
                spans: vec![span("  ⋯", "ui.sidebar.agent.hint")],
                ..ChatLine::default()
            });
        }
        for op in group {
            for change in text.iter_changes(op) {
                if shown >= DIFF_LINES {
                    left += 1;
                    continue;
                }
                shown += 1;
                let (sign, style) = match change.tag() {
                    similar::ChangeTag::Insert => ("+ ", "ui.sidebar.agent.added"),
                    similar::ChangeTag::Delete => ("- ", "ui.sidebar.agent.removed"),
                    similar::ChangeTag::Equal => ("  ", "ui.sidebar.agent.output"),
                };
                let line = change.value().trim_end_matches(['\n', '\r']);
                lines.push(ChatLine {
                    spans: vec![span(&format!("  {sign}{line}"), style)],
                    indent: 4,
                    ..ChatLine::default()
                });
            }
        }
    }
    if left > 0 {
        lines.push(ChatLine {
            spans: vec![span(
                &format!("  … {left} more lines"),
                "ui.sidebar.agent.hint",
            )],
            ..ChatLine::default()
        });
    }
}

/// What the agent said, as lines: Markdown's headings, lists, code blocks,
/// tables (fitted to `width`), `code` and **bold** styled; the rest as it
/// is.
fn markdown_lines(text: &str, width: usize, lines: &mut Vec<ChatLine>) {
    let mut in_code = false;
    let source: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < source.len() {
        let line = source[i];
        i += 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(ChatLine {
                spans: vec![span(&format!("  {line}"), "ui.sidebar.agent.code")],
                indent: 2,
                ..ChatLine::default()
            });
            continue;
        }
        // A table: a row of cells, then a row of dashes under them.
        if trimmed.starts_with('|')
            && let Some(aligns) = source.get(i).and_then(|l| table_aligns(l))
        {
            let mut rows = vec![table_cells(trimmed)];
            i += 1;
            while let Some(row) = source
                .get(i)
                .map(|l| l.trim())
                .filter(|l| l.starts_with('|'))
            {
                rows.push(table_cells(row));
                i += 1;
            }
            table_lines(&rows, &aligns, width, lines);
            continue;
        }
        // A rule: `---`, `***` or `___`.
        let bare: String = trimmed.chars().filter(|c| *c != ' ').collect();
        if bare.len() >= 3
            && ['-', '*', '_']
                .iter()
                .any(|&mark| bare.chars().all(|c| c == mark))
        {
            lines.push(ChatLine {
                spans: vec![span(&"─".repeat(width.max(1)), "ui.sidebar.agent.hint")],
                ..ChatLine::default()
            });
            continue;
        }
        // A quote: a bar in front, the text dimmed.
        if let Some(quote) = trimmed.strip_prefix('>') {
            let quote = quote.strip_prefix(' ').unwrap_or(quote);
            let mut spans = vec![span("▎ ", "ui.sidebar.agent.hint")];
            spans.extend(inline_spans(quote).into_iter().map(|mut s| {
                if s.style.as_deref() == Some("ui.sidebar.agent.text") {
                    s.style = Some("ui.sidebar.agent.thought".into());
                }
                s
            }));
            lines.push(ChatLine {
                spans,
                indent: 2,
                ..ChatLine::default()
            });
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            lines.push(ChatLine {
                spans: vec![span(heading, "ui.sidebar.agent.heading")],
                ..ChatLine::default()
            });
            continue;
        }
        let lead = line.len() - trimmed.len();
        let bullet = ["- ", "* ", "+ "]
            .iter()
            .find_map(|b| trimmed.strip_prefix(b));
        let (mark, rest, indent) = match bullet {
            Some(rest) => (format!("{}• ", " ".repeat(lead)), rest, lead + 2),
            None => {
                // `1. `: kept, with what follows indented under it.
                let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
                if digits > 0 && trimmed[digits..].starts_with(". ") {
                    let mark = &line[..lead + digits + 2];
                    (
                        mark.to_string(),
                        &trimmed[digits + 2..],
                        mark.chars().count(),
                    )
                } else {
                    (String::new(), line, 0)
                }
            }
        };
        let mut spans = Vec::new();
        if !mark.is_empty() {
            spans.push(span(&mark, "ui.sidebar.agent.bullet"));
        }
        spans.extend(inline_spans(rest));
        lines.push(ChatLine {
            spans,
            indent: indent as u16,
            ..ChatLine::default()
        });
    }
}

/// How a table's column is lined up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

/// A table row's cells: split at its `|`s (not `\|`), trimmed.
fn table_cells(row: &str) -> Vec<String> {
    let row = row.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let row = match row.strip_suffix('|') {
        Some(inner) if !inner.ends_with('\\') => inner,
        _ => row,
    };
    let mut cells = vec![String::new()];
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let cell = cells.last_mut().expect("one");
                cell.push(c);
                cell.extend(chars.next());
            }
            '|' => cells.push(String::new()),
            _ => cells.last_mut().expect("one").push(c),
        }
    }
    cells.iter().map(|c| c.trim().to_string()).collect()
}

/// The columns' alignments, if `row` is a table's row of dashes
/// (`|:--|--:|:-:|`).
fn table_aligns(row: &str) -> Option<Vec<Align>> {
    let row = row.trim();
    if !row.starts_with('|') {
        return None;
    }
    table_cells(row)
        .iter()
        .map(|cell| {
            let dashes = cell.trim_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
                return None;
            }
            Some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// Between a table's columns.
const COLUMN_GAP: &str = " │ ";

/// A table's lines: its columns lined up, a rule under the header, each
/// column as wide as its widest cell; if they don't fit `width`, the widest
/// columns are narrowed and their cells wrapped.
fn table_lines(rows: &[Vec<String>], aligns: &[Align], width: usize, lines: &mut Vec<ChatLine>) {
    let columns = rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(aligns.len());
    if columns == 0 {
        return;
    }
    // Each cell as styled chars; the header's bold.
    let cells: Vec<Vec<Vec<(char, String)>>> = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            (0..columns)
                .map(|c| {
                    let text = row.get(c).map_or("", String::as_str);
                    inline_spans(text)
                        .into_iter()
                        .flat_map(|s| {
                            let style = match s.style.as_deref() {
                                Some("ui.sidebar.agent.text") if r == 0 => "ui.sidebar.agent.bold",
                                other => other.unwrap_or("ui.sidebar.agent.text"),
                            }
                            .to_string();
                            s.text
                                .chars()
                                .map(move |ch| (ch, style.clone()))
                                .collect::<Vec<_>>()
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let char_w = |c: char| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    let mut widths: Vec<usize> = (0..columns)
        .map(|c| {
            cells
                .iter()
                .map(|row| row[c].iter().map(|(ch, _)| char_w(*ch)).sum::<usize>())
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    let gaps = (columns - 1) * COLUMN_GAP.chars().count();
    while widths.iter().sum::<usize>() + gaps > width {
        let (widest, &w) = widths
            .iter()
            .enumerate()
            .max_by_key(|(_, w)| **w)
            .expect("a column");
        if w <= 4 {
            break;
        }
        widths[widest] -= 1;
    }
    let rule_style = "ui.sidebar.agent.hint";
    for (r, row) in cells.iter().enumerate() {
        let wrapped: Vec<Vec<Vec<(char, String)>>> = row
            .iter()
            .zip(&widths)
            .map(|(cell, &w)| wrap_cell(cell, w))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for k in 0..height {
            let mut spans = Vec::new();
            for (c, cell) in wrapped.iter().enumerate() {
                if c > 0 {
                    spans.push(span(COLUMN_GAP, rule_style));
                }
                let piece = cell.get(k).map_or(&[][..], Vec::as_slice);
                let used: usize = piece.iter().map(|(ch, _)| char_w(*ch)).sum();
                let room = widths[c].saturating_sub(used);
                let (before, after) = match aligns.get(c).copied().unwrap_or(Align::Left) {
                    Align::Left => (0, room),
                    Align::Right => (room, 0),
                    Align::Center => (room / 2, room - room / 2),
                };
                if before > 0 {
                    spans.push(span(&" ".repeat(before), "ui.sidebar.agent.text"));
                }
                for (ch, style) in piece {
                    match spans.last_mut() {
                        Some(last) if last.style.as_deref() == Some(style.as_str()) => {
                            last.text.push(*ch);
                        }
                        _ => spans.push(span(&ch.to_string(), style)),
                    }
                }
                // The last column needs no padding after it.
                if after > 0 && c + 1 < columns {
                    spans.push(span(&" ".repeat(after), "ui.sidebar.agent.text"));
                }
            }
            lines.push(ChatLine {
                spans,
                ..ChatLine::default()
            });
        }
        if r == 0 {
            let rule: Vec<String> = widths.iter().map(|&w| "─".repeat(w)).collect();
            lines.push(ChatLine {
                spans: vec![span(&rule.join("─┼─"), rule_style)],
                ..ChatLine::default()
            });
        }
    }
}

/// A cell's styled chars as lines `width` wide, broken at spaces (or
/// anywhere, for a word too long).
fn wrap_cell(cell: &[(char, String)], width: usize) -> Vec<Vec<(char, String)>> {
    let char_w = |c: char| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    let mut out = Vec::new();
    let mut line: Vec<(char, String)> = Vec::new();
    let mut used = 0;
    for (ch, style) in cell {
        let w = char_w(*ch);
        if used + w > width && !line.is_empty() {
            let carry = match line.iter().rposition(|(c, _)| *c == ' ') {
                Some(at) if *ch != ' ' => line.split_off(at + 1),
                _ => Vec::new(),
            };
            while line.last().is_some_and(|(c, _)| *c == ' ') {
                line.pop();
            }
            out.push(std::mem::take(&mut line));
            line = carry;
            used = line.iter().map(|(c, _)| char_w(*c)).sum();
            if *ch == ' ' {
                continue;
            }
        }
        line.push((*ch, style.clone()));
        used += w;
    }
    out.push(line);
    out
}

/// Markdown's backslash escapes (`\_`, `\*`, `\|`) as the chars they stand for.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

/// A line's `code` and **bold** pieces, styled.
fn inline_spans(text: &str) -> Vec<PopupSpan> {
    let mut spans = Vec::new();
    let ticks = text.matches('`').count();
    for (i, piece) in text.split('`').enumerate() {
        if piece.is_empty() {
            continue;
        }
        // Between backticks (an unclosed one's text is kept as it is).
        if i % 2 == 1 && i < ticks {
            spans.push(span(piece, "ui.sidebar.agent.code.inline"));
            continue;
        }
        let piece = if i % 2 == 1 {
            format!("`{piece}")
        } else {
            unescape(piece)
        };
        for (j, part) in piece.split("**").enumerate() {
            if part.is_empty() {
                continue;
            }
            let style = if j % 2 == 1 {
                "ui.sidebar.agent.bold"
            } else {
                "ui.sidebar.agent.text"
            };
            spans.push(span(part, style));
        }
    }
    spans
}

/// Each line's text, for tests.
#[cfg(test)]
pub(super) fn text_of(lines: &[ChatLine]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.text.as_str()).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::agent::AgentEvent;

    #[test]
    fn tool_calls_open_to_show_their_changes() {
        let mut chat = AgentChat::default();
        chat.event(AgentEvent::ToolStarted(ToolCall {
            id: "t1".into(),
            title: "Edit a.rs".into(),
            status: ToolStatus::Done,
            diffs: vec![ToolDiff {
                path: "a.rs".into(),
                old: Some("one\ntwo\nthree\n".into()),
                new: "one\nTWO\nthree\n".into(),
            }],
            ..ToolCall::default()
        }));
        let closed = text_of(&chat.transcript(80));
        assert_eq!(closed, ["✓ Edit a.rs ▸", "  a.rs +1 -1"]);
        chat.toggle_tool("t1");
        let open = text_of(&chat.transcript(80));
        assert!(open.contains(&"  - two".to_string()), "{open:?}");
        assert!(open.contains(&"  + TWO".to_string()), "{open:?}");
        assert_eq!(open[0], "✓ Edit a.rs ▾");
        chat.toggle_tool("t1");
        assert_eq!(text_of(&chat.transcript(80)), closed);
        chat.toggle_all_tools();
        assert_eq!(text_of(&chat.transcript(80))[0], "✓ Edit a.rs ▾");
    }

    #[test]
    fn markdown_is_styled() {
        let mut lines = Vec::new();
        markdown_lines(
            "# Title\nUse `cargo` **now**\n- one\n```\nlet x;\n```",
            80,
            &mut lines,
        );
        let text = text_of(&lines);
        assert_eq!(text, ["Title", "Use cargo now", "• one", "  let x;"]);
        assert_eq!(
            lines[1].spans[1].style.as_deref(),
            Some("ui.sidebar.agent.code.inline")
        );
        assert_eq!(
            lines[1].spans[3].style.as_deref(),
            Some("ui.sidebar.agent.bold")
        );
        assert_eq!(lines[2].indent, 2);
    }

    #[test]
    fn tables_are_lined_up_and_escapes_dropped() {
        let mut lines = Vec::new();
        markdown_lines(
            "Usage:\n| MCP server | Usage |\n|:--|--:|\n| ccd\\_turn | 8% |\n| claude-in-chrome | 12% |\nafter",
            80,
            &mut lines,
        );
        assert_eq!(
            text_of(&lines),
            [
                "Usage:",
                "MCP server       │ Usage",
                "─────────────────┼──────",
                "ccd_turn         │    8%",
                "claude-in-chrome │   12%",
                "after",
            ]
        );
        // The header's bold, the rule's dim.
        assert_eq!(
            lines[1].spans[0].style.as_deref(),
            Some("ui.sidebar.agent.bold")
        );
        assert_eq!(
            lines[2].spans[0].style.as_deref(),
            Some("ui.sidebar.agent.hint")
        );

        // Too wide: the wide column's cells wrap inside it.
        let mut lines = Vec::new();
        markdown_lines(
            "| Name | What it does |\n|---|---|\n| a | one two three four five six |",
            20,
            &mut lines,
        );
        let text = text_of(&lines);
        assert!(text.iter().all(|l| l.chars().count() <= 20), "{text:?}");
        assert_eq!(text[0], "Name │ What it does");
        assert_eq!(text[2], "a    │ one two three");
        assert_eq!(text[3], "     │ four five six");

        // Escapes go outside code, stay inside it; | in a cell.
        let mut lines = Vec::new();
        markdown_lines("a\\_b `c\\_d`\n| x \\| y |\n|:-:|", 80, &mut lines);
        assert_eq!(text_of(&lines), ["a_b c\\_d", "x | y", "─────"]);
    }

    #[test]
    fn rules_and_quotes_are_drawn() {
        let mut lines = Vec::new();
        markdown_lines(
            "a\n---\n> Approximate, `only` here\n* * *\n- item",
            6,
            &mut lines,
        );
        assert_eq!(
            text_of(&lines),
            [
                "a",
                "──────",
                "▎ Approximate, only here",
                "──────",
                "• item"
            ]
        );
        assert_eq!(lines[2].indent, 2);
        assert_eq!(
            lines[2].spans[1].style.as_deref(),
            Some("ui.sidebar.agent.thought")
        );
    }
}
