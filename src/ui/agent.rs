//! The agent sidebar (`editor/agent.rs`), right of the windows, full
//! height, as VS Code's chat:
//!
//! ```text
//! │ Claude Agent 0.107   ⠹   Default ▾ │  header: the agent, its mode
//! │ › fix the tests                    │  what you asked
//! │ Looking at them now.               │  what it said (Markdown)
//! │ ✓ Read src/lib.rs                  │  tool calls (a click opens one)
//! │ ✓ Edit src/lib.rs ▸                │
//! │   src/lib.rs +3 -1                 │
//! │ ────────────────────────────────── │
//! │ Run cargo test?                    │  its question, and the options
//! │ [1 Allow] [2 Always] [3 Reject]    │
//! │ ────────────────────────────────── │
//! │ Ask Claude…                        │  the box (it grows as you type),
//! │ Opus 5.5 ▾  High effort ▾  42k/200k│  its settings, its context used
//! │ ⏎ send  ⇧⏎ new line  ⇧⇥ mode       │
//! ```
//!
//! `/` at the start of the box lists the agent's commands over the chat:
//!
//! ```text
//! │ /compact   Clear history, keep a … │
//! │ /review    Review a pull request   │
//! │ ────────────────────────────────── │
//! │ /                                  │
//! ```

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::Style,
};
use unicode_width::UnicodeWidthChar;

use crate::config::Theme;
use crate::editor::agent::AgentStatus;
use crate::editor::agent_session::SLASH_ROWS;
use crate::editor::{Editor, Mode};

/// Rows the box grows to, at most.
const INPUT_ROWS: usize = 8;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Where the sidebar's parts were drawn, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentMap {
    pub area: Rect,
    /// The line between it and the windows (dragged to resize it).
    pub edge: Rect,
    /// The mode's chip in the header (a click lists the modes).
    pub mode: Option<Rect>,
    /// The agent's name in the header (a click lists the agents).
    pub title: Rect,
    /// The chat's rows.
    pub chat: Rect,
    /// The chat's rows that open or close a tool call, and which.
    pub tools: Vec<(u16, String)>,
    /// The question's options, as buttons: where, and which (from 1).
    pub options: Vec<(Rect, usize)>,
    /// The box (a click focuses it).
    pub input: Rect,
    /// The settings' chips under the box, and which (its id).
    pub chips: Vec<(Rect, String)>,
    /// The slash menu's rows, and which command (its row in the list).
    pub slash: Vec<(Rect, usize)>,
}

/// The frame of the spinner shown while the agent works.
pub fn spinner_frame() -> usize {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    (millis / 100) as usize % SPINNER.len()
}

/// A row of styled cells.
type Row = Vec<(char, Style)>;

/// Draws the sidebar in `area` (its edge, the column left of it, too).
/// Returns where things went, and where the cursor goes if the box has
/// focus.
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    edge: Rect,
    editor: &Editor,
    theme: &Theme,
) -> (AgentMap, Option<Position>) {
    let chat = editor.agent_chat();
    let focused = editor.mode() == Mode::Agent;
    let base = theme.get("ui.sidebar").patch(theme.get("ui.sidebar.agent"));
    buf.set_style(area, base);
    let separator = theme.get("ui.window.separator");
    for y in edge.y..edge.bottom() {
        buf[(edge.x, y)].set_symbol("│").set_style(separator);
    }
    let mut map = AgentMap {
        area,
        edge,
        ..AgentMap::default()
    };
    if area.width < 4 || area.height < 4 {
        return (map, None);
    }
    let inner_x = area.x + 1;
    let inner_w = area.width.saturating_sub(2);

    // The header: the agent, what it's doing, its mode.
    let header = Rect::new(area.x, area.y, area.width, 1);
    buf.set_style(header, base.patch(theme.get("ui.sidebar.agent.header")));
    let title = chat.title();
    let (state, state_style) = match &chat.status {
        AgentStatus::Off => (String::new(), "ui.sidebar.agent.hint"),
        AgentStatus::Starting => (
            format!("{} starting", SPINNER[spinner_frame()]),
            "ui.sidebar.agent.hint",
        ),
        AgentStatus::Ready => (String::new(), "ui.sidebar.agent.hint"),
        AgentStatus::Busy => (
            SPINNER[spinner_frame()].to_string(),
            "ui.sidebar.agent.busy",
        ),
        AgentStatus::Stopped(_) => ("stopped".into(), "ui.sidebar.agent.tool.failed"),
    };
    let mode = chat.mode_name().map(|m| format!(" {m} ▾ "));
    let mode_w = mode.as_deref().map_or(0, width_of) as u16;
    let mut x = inner_x;
    let right = area.right().saturating_sub(1);
    let title_room = right.saturating_sub(x + mode_w + 1);
    x = put(
        buf,
        x,
        area.y,
        &title,
        theme.get("ui.sidebar.agent.title"),
        x + title_room,
    );
    map.title = Rect::new(inner_x, area.y, x - inner_x, 1);
    if !state.is_empty() {
        x = put(
            buf,
            x + 1,
            area.y,
            &state,
            theme.get(state_style),
            right.saturating_sub(mode_w),
        );
    }
    let _ = x;
    if let Some(mode) = &mode
        && mode_w + 1 < inner_w
    {
        let at = right - mode_w;
        put(
            buf,
            at,
            area.y,
            mode,
            theme.get("ui.sidebar.agent.mode"),
            right,
        );
        map.mode = Some(Rect::new(at, area.y, mode_w, 1));
    }

    // The bottom up: the keys, the box, the question.
    let mut bottom = area.bottom();
    let ctrl = if cfg!(target_os = "macos") {
        "⌃"
    } else {
        "Ctrl+"
    };
    let hints = match (&chat.permission, &chat.status) {
        (Some(ask), _) => format!("1-{} answer  {ctrl}C stop", ask.options.len()),
        (None, AgentStatus::Busy) => format!("{ctrl}C stop  {ctrl}O tool output  ⇧⇥ mode"),
        _ => "⏎ send  ⇧⏎ new line  ⇧⇥ mode  esc back".to_string(),
    };
    // Room to breathe, given the rows: under the keys, between them and
    // the box, and inside the box around what's typed.
    let pad = u16::from(area.height >= 16);
    if area.height >= 12 {
        bottom -= 1 + pad;
        put(
            buf,
            inner_x,
            bottom,
            &hints,
            theme.get("ui.sidebar.agent.hint"),
            area.right() - 1,
        );
        bottom -= pad;
    }

    // The box's last row: the settings (chips), and the context used.
    let chips = editor.agent_chips();
    let chips_row = u16::from(area.height >= 12 && (!chips.is_empty() || chat.usage.is_some()));
    // Rows they take: theirs, and a gap above it (given room).
    let chips_rows = chips_row * (1 + pad);

    let (input_rows, cursor_at) = wrap_input(
        chat.input.text(),
        chat.input.cursor(),
        inner_w.saturating_sub(1),
    );
    let shown = input_rows.len().clamp(1, INPUT_ROWS);
    // Rows above the cursor's, scrolled out of the box.
    let first = cursor_at.0.saturating_sub(shown - 1);
    let input_top = bottom.saturating_sub(shown as u16 + pad + chips_rows);
    let input = Rect::new(
        area.x,
        input_top - pad,
        area.width,
        shown as u16 + 2 * pad + chips_rows,
    );
    let input_style = base.patch(theme.get("ui.sidebar.agent.input"));
    buf.set_style(input, input_style);
    map.input = input;
    if chips_row == 1 {
        // In line with what's typed: `Opus 5.5 ▾  High effort ▾    42k / 200k`.
        let y = bottom - pad - 1;
        let right = area.right() - 1;
        let hint = input_style.patch(theme.get("ui.sidebar.agent.hint"));
        let mut x = inner_x;
        for (id, chip) in chips {
            let w = width_of(&chip) as u16 + 2;
            if x + w > right {
                break;
            }
            let end = put(buf, x, y, &chip, input_style, right);
            let end = put(buf, end + 1, y, "▾", hint, right);
            map.chips.push((Rect::new(x, y, end - x, 1), id));
            x = end + 2;
        }
        // The longest of its labels that fits.
        let labels = chat.usage.as_ref().map(|u| u.labels()).unwrap_or_default();
        if let Some(usage) = labels
            .iter()
            .find(|l| !l.is_empty() && x + width_of(l) as u16 <= right)
        {
            put(buf, right - width_of(usage) as u16, y, usage, hint, right);
        }
    }
    if chat.input.text().is_empty() {
        let placeholder = format!(
            "Ask {}…",
            chat.agent_info
                .as_deref()
                .map_or_else(|| chat.title(), short_name)
        );
        put(
            buf,
            inner_x,
            input_top,
            &placeholder,
            theme.get("ui.input.placeholder"),
            area.right() - 1,
        );
    } else {
        let style = theme.get("ui.sidebar.agent.input");
        for (i, row) in input_rows.iter().skip(first).take(shown).enumerate() {
            put(
                buf,
                inner_x,
                input_top + i as u16,
                row,
                style,
                area.right() - 1,
            );
        }
    }
    let border = if focused {
        theme.get("ui.sidebar.agent.border.focused")
    } else {
        theme.get("ui.sidebar.agent.border")
    };
    let line_y = input.y.saturating_sub(1);
    line(buf, area.x, line_y, area.width, border);
    bottom = line_y;
    // `/name ` typed: what the command takes, after the cursor.
    if let Some(hint) = editor.agent_input_hint() {
        let at = inner_x + cursor_at.1 as u16;
        put(
            buf,
            at,
            input_top + (cursor_at.0 - first) as u16,
            &hint,
            theme.get("ui.input.placeholder"),
            area.right() - 1,
        );
    }
    let cursor = focused.then(|| {
        Position::new(
            inner_x + cursor_at.1 as u16,
            input_top + (cursor_at.0 - first) as u16,
        )
    });

    if let Some(ask) = &chat.permission {
        // The options as buttons, as many to a row as fit.
        let mut rows: Vec<Vec<(String, usize)>> = vec![Vec::new()];
        let mut used = 0;
        for (i, option) in ask.options.iter().enumerate() {
            let label = format!(" {} {} ", i + 1, option.name);
            let w = width_of(&label) + 1;
            if used + w > inner_w as usize && used > 0 {
                rows.push(Vec::new());
                used = 0;
            }
            used += w;
            rows.last_mut().expect("a row").push((label, i));
        }
        let title_rows = wrap(
            &[(ask.title.clone(), theme.get("ui.sidebar.agent.question"))],
            0,
            inner_w,
            None,
        );
        let height = (title_rows.len() + rows.len() + 1) as u16;
        let top = bottom.saturating_sub(height);
        if top > area.y + 2 {
            line(
                buf,
                area.x,
                top,
                area.width,
                theme.get("ui.sidebar.agent.border"),
            );
            let mut y = top + 1;
            for row in &title_rows {
                put_row(buf, inner_x, y, row, inner_w);
                y += 1;
            }
            for row in rows {
                let mut x = inner_x;
                for (label, i) in row {
                    let style = match ask.options[i].kind.as_str() {
                        kind if kind.starts_with("allow") => "ui.sidebar.agent.option.allow",
                        _ => "ui.sidebar.agent.option.reject",
                    };
                    let end = put(buf, x, y, &label, theme.get(style), area.right() - 1);
                    map.options.push((Rect::new(x, y, end - x, 1), i + 1));
                    x = end + 1;
                }
                y += 1;
            }
            bottom = top;
        }
    }

    // The chat, between the header and what's below.
    let chat_area = Rect::new(
        inner_x,
        area.y + 1,
        inner_w,
        bottom.saturating_sub(area.y + 1),
    );
    map.chat = chat_area;
    let lines = editor.agent_transcript(inner_w);
    if lines.is_empty() {
        let name = chat.title();
        let hello = [
            String::new(),
            format!("Ask {name} to change code, explain it, or run things."),
            String::new(),
            "It works in this folder; what it changes shows up here.".to_string(),
        ];
        let style = theme.get("ui.sidebar.agent.hint");
        let mut y = chat_area.y;
        for text in hello {
            for row in wrap(&[(text, style)], 0, inner_w, None) {
                if y >= chat_area.bottom() {
                    break;
                }
                put_row(buf, inner_x, y, &row, inner_w);
                y += 1;
            }
        }
        draw_slash(buf, editor, theme, chat_area, &mut map);
        return (map, cursor);
    }
    let mut rows: Vec<(Row, Option<Style>, Option<&str>)> = Vec::new();
    for line in &lines {
        let fill = line.fill.as_deref().map(|f| base.patch(theme.get(f)));
        let spans: Vec<(String, Style)> = line
            .spans
            .iter()
            .map(|s| {
                let style = s
                    .style
                    .as_deref()
                    .map_or(base, |st| base.patch(theme.get(st)));
                (s.text.clone(), fill.map_or(style, |f| f.patch(style)))
            })
            .collect();
        for row in wrap(&spans, line.indent, inner_w, fill) {
            rows.push((row, fill, line.tool.as_deref()));
        }
    }
    let height = chat_area.height as usize;
    let most = rows.len().saturating_sub(height);
    let scroll = chat.scroll.get().min(most);
    chat.scroll.set(scroll);
    let top = most - scroll;
    for (i, (row, fill, tool)) in rows.iter().skip(top).take(height).enumerate() {
        let y = chat_area.y + i as u16;
        if let Some(fill) = fill {
            buf.set_style(Rect::new(area.x, y, area.width, 1), *fill);
        }
        put_row(buf, inner_x, y, row, inner_w);
        if let Some(id) = tool {
            map.tools.push((y, id.to_string()));
        }
    }
    // More above: a mark at the top right.
    if top > 0 && chat_area.height > 0 {
        buf[(area.right() - 1, chat_area.y)]
            .set_symbol("↑")
            .set_style(theme.get("ui.sidebar.agent.hint"));
    }
    if scroll > 0 && chat_area.height > 0 {
        buf[(area.right() - 1, chat_area.bottom() - 1)]
            .set_symbol("↓")
            .set_style(theme.get("ui.sidebar.agent.hint"));
    }
    draw_slash(buf, editor, theme, chat_area, &mut map);
    (map, cursor)
}

/// The slash menu, over the foot of the chat (`chat`): a row a command,
/// its name then what it does.
fn draw_slash(buf: &mut Buffer, editor: &Editor, theme: &Theme, chat: Rect, map: &mut AgentMap) {
    let matches = editor.agent_slash_matches();
    if matches.is_empty() || chat.height == 0 {
        return;
    }
    let selected = editor.agent_slash_selected();
    let rows = matches.len().min(SLASH_ROWS).min(chat.height as usize);
    let first = (selected + 1).saturating_sub(rows);
    let name_w = matches
        .iter()
        .map(|c| width_of(&c.name) + 1)
        .max()
        .unwrap_or(0)
        .min(chat.width as usize / 2);
    let normal = theme.get("ui.popup");
    let chosen = normal.patch(theme.get("ui.popup.selected"));
    let detail = theme.get("ui.popup.detail");
    // The sidebar's full width (its padding column each side too).
    let x0 = chat.x.saturating_sub(1);
    let width = chat.width + 2;
    let top = chat.bottom() - rows as u16;
    for (i, command) in matches.iter().enumerate().skip(first).take(rows) {
        let y = top + (i - first) as u16;
        let style = if i == selected { chosen } else { normal };
        let row = Rect::new(x0, y, width, 1);
        buf.set_style(row, style);
        // Clear what the chat drew there.
        for x in row.left()..row.right() {
            buf[(x, y)].set_symbol(" ");
        }
        let limit = row.right() - 1;
        let name = format!("/{}", command.name);
        put(buf, chat.x, y, &name, style, limit);
        let at = chat.x + name_w as u16 + 2;
        let mut text = command.description.clone();
        if let Some(hint) = &command.hint
            && text.is_empty()
        {
            text = hint.clone();
        }
        put(buf, at, y, &text, style.patch(detail), limit);
        map.slash.push((row, i));
    }
}

/// "Claude Agent 0.107.0" → "Claude Agent" (for the box's placeholder).
fn short_name(info: &str) -> String {
    match info.rsplit_once(' ') {
        Some((name, version)) if version.starts_with(|c: char| c.is_ascii_digit()) => name.into(),
        _ => info.into(),
    }
}

fn width_of(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Writes `text` from column `x` up to (not past) `limit`. Returns the
/// column after it.
fn put(buf: &mut Buffer, mut x: u16, y: u16, text: &str, style: Style, limit: u16) -> u16 {
    for c in text.chars() {
        let w = c.width().unwrap_or(0) as u16;
        if w == 0 {
            continue;
        }
        if x + w > limit {
            break;
        }
        buf[(x, y)].set_char(c).set_style(style);
        x += w;
    }
    x
}

fn put_row(buf: &mut Buffer, mut x: u16, y: u16, row: &Row, width: u16) {
    let limit = x + width;
    for &(c, style) in row {
        let w = c.width().unwrap_or(0) as u16;
        if w == 0 || x + w > limit {
            continue;
        }
        buf[(x, y)].set_char(c).set_style(style);
        x += w;
    }
}

fn line(buf: &mut Buffer, x: u16, y: u16, width: u16, style: Style) {
    for x in x..x + width {
        buf[(x, y)].set_symbol("─").set_style(style);
    }
}

/// Styled text wrapped to `width` columns at spaces (or anywhere, for a
/// word too long), rows after the first indented by `indent`.
fn wrap(spans: &[(String, Style)], indent: u16, width: u16, fill: Option<Style>) -> Vec<Row> {
    let width = width.max(1) as usize;
    let indent = (indent as usize).min(width / 2);
    let pad = fill.unwrap_or_default();
    let mut rows: Vec<Row> = Vec::new();
    let mut row: Row = Vec::new();
    let mut used = 0;
    // Where the row can break: after its last space.
    let mut fold: Option<usize> = None;
    let mut start = 0;
    for (text, style) in spans {
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > width && row.len() > start {
                let carry = match fold {
                    Some(at) if at > start => row.split_off(at),
                    _ => Vec::new(),
                };
                while row.last().is_some_and(|(c, _)| *c == ' ') {
                    row.pop();
                }
                rows.push(std::mem::take(&mut row));
                row.extend(std::iter::repeat_n((' ', pad), indent));
                start = row.len();
                row.extend(carry.into_iter().skip_while(|(c, _)| *c == ' '));
                used = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
                fold = None;
                if c == ' ' && row.len() == start {
                    continue;
                }
            }
            row.push((c, *style));
            used += w;
            if c == ' ' {
                fold = Some(row.len());
            }
        }
    }
    rows.push(row);
    rows
}

/// What's typed, as rows `width` wide (broken anywhere, and at new lines),
/// and the cursor's row and column (`cursor` is in chars).
fn wrap_input(text: &str, cursor: usize, width: u16) -> (Vec<String>, (usize, usize)) {
    let width = width.max(1) as usize;
    let mut rows = vec![String::new()];
    let mut used = 0;
    let mut at = (0, 0);
    for (i, c) in text.chars().enumerate() {
        if i == cursor {
            at = (rows.len() - 1, used);
        }
        if c == '\n' {
            rows.push(String::new());
            used = 0;
            continue;
        }
        let w = c.width().unwrap_or(0);
        if used + w > width {
            rows.push(String::new());
            used = 0;
        }
        rows.last_mut().expect("a row").push(c);
        used += w;
    }
    if cursor >= text.chars().count() {
        if used >= width {
            rows.push(String::new());
            used = 0;
        }
        at = (rows.len() - 1, used);
    }
    (rows, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| r.iter().map(|(c, _)| c).collect())
            .collect()
    }

    #[test]
    fn text_wraps_at_spaces_with_an_indent() {
        let spans = [("• the quick brown fox".to_string(), Style::default())];
        assert_eq!(
            text(&wrap(&spans, 2, 10, None)),
            ["• the", "  quick", "  brown", "  fox"]
        );
        let long = [("abcdefghijkl".to_string(), Style::default())];
        assert_eq!(text(&wrap(&long, 0, 5, None)), ["abcde", "fghij", "kl"]);
    }

    #[test]
    fn the_box_wraps_and_finds_its_cursor() {
        let (rows, at) = wrap_input("hello\nworld!", 12, 4);
        assert_eq!(rows, ["hell", "o", "worl", "d!"]);
        assert_eq!(at, (3, 2));
        let (rows, at) = wrap_input("abcd", 4, 4);
        assert_eq!(rows, ["abcd", ""]);
        assert_eq!(at, (1, 0));
        assert_eq!(wrap_input("ab", 1, 4).1, (0, 1));
    }

    #[test]
    fn the_version_is_left_off_the_placeholder() {
        assert_eq!(short_name("Claude Agent 0.107.0"), "Claude Agent");
        assert_eq!(short_name("gemini"), "gemini");
    }
}
