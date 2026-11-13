//! The full-screen git view (`editor/git_gui.rs`): the changed files,
//! branches and commits on the left, the diff on the right, and the keys
//! for what has focus along the bottom.
//!
//! ```text
//! ╭ [1] Files ──────────╮╭ [4] src/app.rs (unstaged) ───────────╮
//! │ STAGED (1)          ││ diff --git a/src/app.rs b/src/app.rs  │
//! │  M src/main.rs      ││ @@ -40,6 +40,7 @@                    │
//! │ CHANGES (1)         ││    let x = 1;                        │
//! │  M src/app.rs       ││ -  let y = 2;                        │
//! ╰─────────────────────╯│ +  let y = 3;                        │
//! ╭ [2] Branches ───────╮│                                      │
//! │ * main              ││                                      │
//! ╰─────────────────────╯│                                      │
//! ╭ [3] Commits ────────╮│                                      │
//! │ 3765c6f Show git c… ││                                      │
//! ╰─────────────────────╯╰──────────────────────────────────────╯
//!  s stage  ⏎ hunks  a stage all  x discard  c commit  Tab  q close
//! ```

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    symbols::border,
    text::Span,
    widgets::{Block, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use super::text::truncate_end;
use crate::config::Theme;
use crate::editor::git_gui::{DiffOf, GitPanel};
use crate::editor::{Editor, GitRow};
use crate::git::{DiffLineKind, SideKind};
use crate::syntax::Highlights;

/// Where the view's parts are, for drawing and the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitGuiMap {
    /// Each panel's area (border included).
    pub panels: Vec<(Rect, GitPanel)>,
    /// Each panel's rows, and the index of the item on the first.
    pub lists: Vec<(Rect, GitPanel, usize)>,
    /// The blame's rows and the line on the first, while it's shown.
    pub blame: Option<(Rect, usize)>,
    /// The side-by-side view's rows (both halves) and the row on the first.
    pub side: Option<(Rect, usize)>,
}

pub struct GitGuiView<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
    /// The blame's code (the first), or each side's, highlighted.
    highlights: [Option<Highlights>; 2],
}

/// The panels' areas in `area`.
fn layout(area: Rect) -> (Vec<(Rect, GitPanel)>, Rect) {
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let hint = Rect::new(area.x, body.bottom(), area.width, 1.min(area.height));
    let left_width = (body.width * 35 / 100)
        .clamp(body.width.min(28), body.width.min(64))
        .min(body.width);
    let files_height = (body.height * 45 / 100).max(body.height.min(4));
    let branches_height = (body.height * 20 / 100)
        .max(body.height.min(4))
        .min(body.height.saturating_sub(files_height));
    let commits_height = body.height.saturating_sub(files_height + branches_height);
    let x = body.x;
    let mut y = body.y;
    let mut panels = Vec::new();
    for (height, panel) in [
        (files_height, GitPanel::Files),
        (branches_height, GitPanel::Branches),
        (commits_height, GitPanel::Commits),
    ] {
        panels.push((Rect::new(x, y, left_width, height), panel));
        y += height;
    }
    panels.push((
        Rect::new(x + left_width, body.y, body.width - left_width, body.height),
        GitPanel::Diff,
    ));
    (panels, hint)
}

impl<'a> GitGuiView<'a> {
    pub fn new(editor: &'a Editor, theme: &'a Theme, highlights: [Option<Highlights>; 2]) -> Self {
        Self {
            editor,
            theme,
            highlights,
        }
    }

    /// Where things go in `area`, with each list's first item as it'll be
    /// drawn (so call it after drawing, or it scrolls the lists itself).
    pub fn map(editor: &Editor, area: Rect) -> GitGuiMap {
        let Some(gui) = editor.git_gui() else {
            return GitGuiMap::default();
        };
        let (panels, hint) = layout(area);
        if let Some(blame) = &gui.blame {
            let body = Rect::new(area.x, area.y, area.width, hint.y - area.y);
            return GitGuiMap {
                blame: Some((Block::bordered().inner(body), blame.scroll.get())),
                ..GitGuiMap::default()
            };
        }
        if let Some(side) = &gui.side {
            let body = Rect::new(area.x, area.y, area.width, hint.y - area.y);
            let rows = Block::bordered().inner(side_halves(body)[0]);
            return GitGuiMap {
                side: Some((
                    Rect::new(body.x, rows.y, body.width, rows.height),
                    side.scroll.get(),
                )),
                ..GitGuiMap::default()
            };
        }
        let lists = panels
            .iter()
            .map(|&(rect, panel)| {
                let inner = Block::bordered().inner(rect);
                let first = match panel {
                    GitPanel::Files => editor.git_scroll(),
                    GitPanel::Branches => gui.branch_scroll.get(),
                    GitPanel::Commits => gui.commit_scroll.get(),
                    GitPanel::Diff => gui.diff_scroll,
                };
                (inner, panel, first)
            })
            .collect();
        GitGuiMap {
            panels,
            lists,
            blame: None,
            side: None,
        }
    }

    fn panel_block(&self, title: String, focused: bool) -> Block<'static> {
        let theme = self.theme;
        let (border_style, title_style) = if focused {
            (
                theme.get("ui.git.border.focused"),
                theme.get("ui.git.title.focused"),
            )
        } else {
            (theme.get("ui.git.border"), theme.get("ui.git.title"))
        };
        Block::bordered()
            .border_set(border::ROUNDED)
            .border_style(border_style)
            .title(Span::styled(title, title_style))
    }

    /// The selected row's style in a list: bright with focus, faint without.
    fn selected_style(&self, focused: bool) -> Style {
        if focused {
            self.theme.get("ui.git.selected")
        } else {
            self.theme.get("ui.git.selected.unfocused")
        }
    }

    fn draw_files(&self, inner: Rect, focused: bool, buf: &mut Buffer) {
        let theme = self.theme;
        let rows = self.editor.git_rows();
        if rows.is_empty() {
            buf.set_string(inner.x + 1, inner.y, "No changes", theme.get("ui.git.dim"));
            return;
        }
        let first = self.editor.git_first_shown(inner.height as usize);
        let width = inner.width as usize;
        for (offset, (index, row)) in rows
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + offset as u16;
            let mut base = theme.get("ui.git");
            if index == self.editor.git_selected() {
                base = base.patch(self.selected_style(focused));
                buf.set_style(Rect::new(inner.x, y, inner.width, 1), base);
            }
            match row {
                GitRow::Header(text) => {
                    let text = truncate_end(&format!(" {}", text.to_uppercase()), width);
                    buf.set_string(
                        inner.x,
                        y,
                        text,
                        base.patch(theme.get("ui.sidebar.git.header")),
                    );
                }
                GitRow::File { file, staged } => {
                    let letter = if *staged { file.staged } else { file.unstaged }.unwrap_or(' ');
                    buf.set_stringn(
                        inner.x + 2,
                        y,
                        letter.to_string(),
                        width.saturating_sub(2),
                        base.patch(theme.get(super::sidebar::git_scope(letter))),
                    );
                    let room = width.saturating_sub(4);
                    buf.set_stringn(inner.x + 4, y, truncate_end(&file.path, room), room, base);
                }
            }
        }
    }

    fn draw_branches(&self, inner: Rect, focused: bool, buf: &mut Buffer) {
        let theme = self.theme;
        let Some(gui) = self.editor.git_gui() else {
            return;
        };
        let first = keep_in_view(
            &gui.branch_scroll,
            gui.branch,
            inner.height as usize,
            gui.branches.len(),
        );
        let width = inner.width as usize;
        for (offset, (index, branch)) in gui
            .branches
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + offset as u16;
            let mut base = theme.get("ui.git");
            if index == gui.branch {
                base = base.patch(self.selected_style(focused));
                buf.set_style(Rect::new(inner.x, y, inner.width, 1), base);
            }
            let (mark, name_style) = if branch.current {
                ("* ", base.patch(theme.get("ui.git.current")))
            } else {
                ("  ", base)
            };
            let x = buf.set_stringn(inner.x + 1, y, mark, width, base).0;
            let room = width.saturating_sub(3);
            let x = buf
                .set_stringn(x, y, truncate_end(&branch.name, room), room, name_style)
                .0;
            let extra = match (branch.upstream.as_str(), branch.track.as_str()) {
                ("", _) => String::new(),
                (up, "") => format!("  {up}"),
                (up, track) => format!("  {up} {track}"),
            };
            let used = (x - inner.x) as usize;
            if !extra.is_empty() && used + 3 < width {
                let room = width - used;
                buf.set_stringn(
                    x,
                    y,
                    truncate_end(&extra, room),
                    room,
                    base.patch(theme.get("ui.git.dim")),
                );
            }
        }
    }

    fn draw_commits(&self, inner: Rect, focused: bool, buf: &mut Buffer) {
        let theme = self.theme;
        let Some(gui) = self.editor.git_gui() else {
            return;
        };
        if gui.commits.is_empty() {
            let text = if gui.history.is_some() {
                "No commits changed it"
            } else {
                "No commits yet"
            };
            buf.set_string(inner.x + 1, inner.y, text, theme.get("ui.git.dim"));
            return;
        }
        let first = keep_in_view(
            &gui.commit_scroll,
            gui.commit,
            inner.height as usize,
            gui.commits.len(),
        );
        let width = inner.width as usize;
        for (offset, (index, commit)) in gui
            .commits
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + offset as u16;
            let mut base = theme.get("ui.git");
            if index == gui.commit {
                base = base.patch(self.selected_style(focused));
                buf.set_style(Rect::new(inner.x, y, inner.width, 1), base);
            }
            let x = buf
                .set_stringn(
                    inner.x + 1,
                    y,
                    &commit.short,
                    width.saturating_sub(1),
                    base.patch(theme.get("ui.git.hash")),
                )
                .0;
            let room = width.saturating_sub((x - inner.x) as usize + 1);
            buf.set_stringn(x + 1, y, truncate_end(&commit.subject, room), room, base);
        }
    }

    fn draw_diff(&self, inner: Rect, focused: bool, buf: &mut Buffer) {
        let theme = self.theme;
        let Some(gui) = self.editor.git_gui() else {
            return;
        };
        gui.diff_rows.set(inner.height as usize);
        let selected_hunk = (focused && matches!(gui.diff_of, DiffOf::File { .. }))
            .then(|| gui.diff.hunks.get(gui.hunk))
            .flatten();
        let width = inner.width as usize;
        if gui.diff.lines.is_empty() {
            let text = match gui.diff_of {
                DiffOf::Nothing => "Nothing to show",
                _ => "No differences",
            };
            buf.set_string(inner.x + 1, inner.y, text, theme.get("ui.git.dim"));
            return;
        }
        for (offset, (index, (kind, text))) in gui
            .diff
            .lines
            .iter()
            .enumerate()
            .skip(gui.diff_scroll)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + offset as u16;
            let mut style = theme.get("ui.git");
            if selected_hunk.is_some_and(|h| (h.start..h.end).contains(&index)) {
                style = style.patch(theme.get("ui.git.diff.selected"));
                buf.set_style(Rect::new(inner.x, y, inner.width, 1), style);
            }
            let scope = match kind {
                DiffLineKind::Added => Some("ui.git.diff.added"),
                DiffLineKind::Removed => Some("ui.git.diff.removed"),
                DiffLineKind::HunkHeader => Some("ui.git.diff.hunk"),
                DiffLineKind::Header
                    if text.starts_with("diff ") || text.starts_with("commit ") =>
                {
                    Some("ui.git.diff.header")
                }
                DiffLineKind::Header => None,
                DiffLineKind::Context => None,
            };
            if let Some(scope) = scope {
                style = style.patch(theme.get(scope));
            }
            let text = text.replace('\t', "    ");
            let room = width.saturating_sub(1);
            buf.set_stringn(inner.x + 1, y, truncate_end(&text, room), room, style);
        }
    }

    /// A file's blame over the whole view. Each run of lines from one
    /// commit is headed by its summary, author and age, with a bar down
    /// the run coloured by how recent it is; the selected line's commit is
    /// picked out, its hash and date along the bottom, and the code is
    /// highlighted.
    fn draw_blame(&self, body: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let Some(blame) = self.editor.git_gui().and_then(|g| g.blame.as_ref()) else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let selected = blame.lines.get(blame.row);
        let footer = match selected {
            Some(line) if line.uncommitted() => " Not committed yet ".to_string(),
            Some(line) => format!(
                " {} • {}, {} • {} ",
                line.short(),
                line.author,
                crate::git::time_ago(now - line.time),
                line.summary
            ),
            None => String::new(),
        };
        let room = body.width.saturating_sub(12) as usize;
        let block = self
            .panel_block(
                format!(" Blame: {} ", truncate_end(&blame.path, room)),
                true,
            )
            .title_bottom(Span::styled(
                truncate_end(&footer, body.width.saturating_sub(4) as usize),
                theme.get("ui.git.title"),
            ));
        let inner = block.inner(body);
        block.render(body, buf);
        let first = keep_in_view(
            &blame.scroll,
            blame.row,
            inner.height as usize,
            blame.lines.len(),
        );
        let number_width = blame.lines.len().to_string().len();
        // The bar, the header (summary, author and age), then a rule.
        let header_width = (inner.width as usize * 2 / 5).clamp(20.min(inner.width as usize), 56);
        let selected_hash = selected.filter(|l| !l.uncommitted()).map(|l| &l.hash);
        for (offset, (index, line)) in blame
            .lines
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + offset as u16;
            let mut base = theme.get("ui.git");
            if index == blame.row {
                base = base.patch(self.selected_style(true));
            } else if selected_hash == Some(&line.hash) {
                base = base.patch(theme.get("ui.git.diff.selected"));
            }
            buf.set_style(Rect::new(inner.x, y, inner.width, 1), base);
            let starts_run = index == first
                || blame.lines.get(index.wrapping_sub(1)).map(|l| &l.hash) != Some(&line.hash);
            let dim = base.patch(theme.get("ui.git.dim"));

            let (bar, bar_scope) = if line.uncommitted() {
                ("┊", "ui.git.blame.uncommitted".to_string())
            } else {
                let heat = blame.heat.get(index).copied().unwrap_or(3);
                (
                    if heat == 0 { "┃" } else { "│" },
                    format!("ui.git.blame.heat{heat}"),
                )
            };
            buf.set_string(inner.x + 1, y, bar, base.patch(theme.get(&bar_scope)));

            // The header, on a run's first line: the summary, then the
            // author and age at its right.
            let header = Rect::new(inner.x + 3, y, (header_width as u16).saturating_sub(4), 1)
                .intersection(inner);
            if starts_run && header.width > 0 {
                let room = header.width as usize;
                if line.uncommitted() {
                    buf.set_stringn(header.x, y, "Not committed", room, dim);
                } else {
                    let who = format!("{} {}", line.author, crate::git::short_age(now - line.time));
                    let who = truncate_end(&who, room / 2);
                    let who_width = who.width();
                    let summary_room = room.saturating_sub(who_width + 1);
                    buf.set_stringn(
                        header.x,
                        y,
                        truncate_end(&line.summary, summary_room),
                        summary_room,
                        base.patch(theme.get("ui.git.blame.summary")),
                    );
                    buf.set_stringn(header.right() - who_width as u16, y, &who, who_width, dim);
                }
            }

            // The line number and the code.
            let mut x = inner.x + header_width as u16;
            if x >= inner.right() {
                continue;
            }
            buf.set_string(x, y, "│", dim);
            x += 2;
            let number = format!("{:>number_width$} ", index + 1);
            buf.set_stringn(x, y, &number, inner.right().saturating_sub(x) as usize, dim);
            x += number.len() as u16;
            let spans = self.highlights[0]
                .as_ref()
                .map_or(&[][..], |h| h.line(index));
            for (col, c) in line.text.chars().enumerate() {
                let style = spans
                    .iter()
                    .filter(|s| s.start <= col && col < s.end)
                    .fold(base, |style, s| style.patch(theme.get(s.name())));
                let (text, w) = match c {
                    '\t' => ("    ".to_string(), 4),
                    c => (
                        c.to_string(),
                        unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16,
                    ),
                };
                if w == 0 {
                    continue;
                }
                if x + w > inner.right() {
                    break;
                }
                buf.set_string(x, y, text, style);
                x += w;
            }
        }
    }

    /// A file's change side by side: the old version on the left, the new
    /// on the right, unchanged lines level and filler where a side has
    /// nothing, the selected row across both.
    fn draw_side(&self, body: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let Some(side) = self.editor.git_gui().and_then(|g| g.side.as_ref()) else {
            return;
        };
        let halves = side_halves(body);
        let changes = side.changes();
        let at = changes.iter().filter(|&&c| c <= side.row).count();
        let mut inners = [Rect::default(); 2];
        for (i, half) in halves.iter().enumerate() {
            let room = half.width.saturating_sub(6) as usize;
            let mut block =
                self.panel_block(format!(" {} ", truncate_end(&side.titles[i], room)), true);
            if i == 1 {
                let count = format!(" change {at} of {} ", changes.len());
                block = block.title_bottom(Span::styled(count, theme.get("ui.git.title")));
            }
            inners[i] = block.inner(*half);
            block.render(*half, buf);
        }
        let height = inners[0].height as usize;
        let first = keep_in_view(&side.scroll, side.row, height, side.rows.len());
        let number_width = side
            .documents
            .iter()
            .map(|d| d.buffer().line_count().to_string().len())
            .max()
            .unwrap_or(1);
        for (offset, (index, row)) in side
            .rows
            .iter()
            .enumerate()
            .skip(first)
            .take(height)
            .enumerate()
        {
            let line_of = [row.left, row.right];
            for (i, inner) in inners.iter().enumerate() {
                let y = inner.y + offset as u16;
                let mut base = theme.get("ui.git");
                if row.kind == SideKind::Changed && line_of[i].is_some() {
                    base = base.patch(theme.get(if i == 0 {
                        "ui.git.side.removed"
                    } else {
                        "ui.git.side.added"
                    }));
                }
                if index == side.row {
                    base = base.patch(self.selected_style(true));
                }
                buf.set_style(Rect::new(inner.x, y, inner.width, 1), base);
                let Some(line) = line_of[i] else {
                    let filler = "╱".repeat(inner.width as usize);
                    buf.set_stringn(
                        inner.x,
                        y,
                        filler,
                        inner.width as usize,
                        base.patch(theme.get("ui.git.side.filler")),
                    );
                    continue;
                };
                let dim = base.patch(theme.get("ui.git.dim"));
                let number = format!(" {:>number_width$} ", line + 1);
                let mut x = inner.x;
                buf.set_stringn(x, y, &number, inner.width as usize, dim);
                x += number.len() as u16;
                let text = side.documents[i].buffer().line(line);
                let spans = self.highlights[i]
                    .as_ref()
                    .map_or(&[][..], |h| h.line(line));
                for (col, c) in text.chars().enumerate() {
                    let style = spans
                        .iter()
                        .filter(|s| s.start <= col && col < s.end)
                        .fold(base, |style, s| style.patch(theme.get(s.name())));
                    let (shown, w) = match c {
                        '\t' => ("    ".to_string(), 4),
                        c => (
                            c.to_string(),
                            unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16,
                        ),
                    };
                    if w == 0 {
                        continue;
                    }
                    if x + w > inner.right() {
                        break;
                    }
                    buf.set_string(x, y, shown, style);
                    x += w;
                }
            }
        }
    }

    fn hint(&self) -> &'static str {
        let Some(gui) = self.editor.git_gui() else {
            return "";
        };
        if gui.side.is_some() {
            return " j/k lines  ]c/[c changes  ⏎ open the file there  ⌃d/⌃u page  Esc back  q close";
        }
        if gui.blame.is_some() {
            return " j/k lines  ⏎ open the commit  H history  ⌃d/⌃u page  Esc back  q close";
        }
        let merging = self.editor.git_merging().is_some();
        match (gui.panel, &gui.diff_of) {
            (GitPanel::Files, _) if merging => {
                " ⏎ open at the conflict  s stage when resolved  c commit the merge  M abort the merge  q close"
            }
            (GitPanel::Files, _) => {
                " s stage/unstage  ⏎ hunks  H history  a stage all  x discard  c commit  Tab panels  q close"
            }
            (GitPanel::Diff, DiffOf::File { staged: false, .. }) => {
                " j/k hunks  s stage the hunk  ⌃d/⌃u scroll  Esc back  q close"
            }
            (GitPanel::Diff, DiffOf::File { staged: true, .. }) => {
                " j/k hunks  u unstage the hunk  ⌃d/⌃u scroll  Esc back  q close"
            }
            (GitPanel::Commits, _) if gui.history.is_some() => {
                " j/k commits  ⏎ into the diff  ⌃d/⌃u scroll  H/Esc whole log  q close"
            }
            (GitPanel::Branches, _) => {
                " ⏎ switch  n new  M merge in  d delete  f fetch  p pull  P push  Tab panels  q close"
            }
            _ => " j/k move  ⌃d/⌃u scroll the diff  f fetch  p pull  P push  c commit  q close",
        }
    }
}

/// The two halves of the side-by-side view.
fn side_halves(body: Rect) -> [Rect; 2] {
    let left = body.width / 2;
    [
        Rect::new(body.x, body.y, left, body.height),
        Rect::new(body.x + left, body.y, body.width - left, body.height),
    ]
}

/// The first row to show of a list `len` long in `height` rows, keeping
/// `selected` in view (remembered in `scroll`).
fn keep_in_view(
    scroll: &std::cell::Cell<usize>,
    selected: usize,
    height: usize,
    len: usize,
) -> usize {
    let mut top = scroll.get();
    if selected < top {
        top = selected;
    } else if height > 0 && selected >= top + height {
        top = selected + 1 - height;
    }
    top = top.min(len.saturating_sub(height.max(1)));
    scroll.set(top);
    top
}

impl Widget for GitGuiView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(gui) = self.editor.git_gui() else {
            return;
        };
        if area.width < 20 || area.height < 8 {
            return;
        }
        Clear.render(area, buf);
        buf.set_style(area, self.theme.get("ui.git"));
        let (panels, hint) = layout(area);
        if gui.blame.is_some() || gui.side.is_some() {
            let body = Rect::new(area.x, area.y, area.width, hint.y - area.y);
            if gui.side.is_some() {
                self.draw_side(body, buf);
            } else {
                self.draw_blame(body, buf);
            }
            let text = truncate_end(self.hint(), hint.width as usize);
            buf.set_string(hint.x, hint.y, text, self.theme.get("ui.git.hint"));
            return;
        }
        for (rect, panel) in panels {
            let focused = gui.panel == panel;
            let title = match panel {
                GitPanel::Files => match self.editor.git_merging() {
                    Some(_) => " [1] Files (merging) ".to_string(),
                    None => " [1] Files ".to_string(),
                },
                GitPanel::Branches => format!(" [2] Branches ({}) ", gui.branches.len()),
                GitPanel::Commits => match &gui.history {
                    Some(path) => {
                        let room = rect.width.saturating_sub(16) as usize;
                        format!(" [3] History: {} ", truncate_end(path, room))
                    }
                    None => " [3] Commits ".to_string(),
                },
                GitPanel::Diff => {
                    let room = rect.width.saturating_sub(10) as usize;
                    format!(" [4] {} ", truncate_end(&gui.diff_title, room))
                }
            };
            let block = self.panel_block(title, focused);
            let inner = block.inner(rect);
            block.render(rect, buf);
            match panel {
                GitPanel::Files => self.draw_files(inner, focused, buf),
                GitPanel::Branches => self.draw_branches(inner, focused, buf),
                GitPanel::Commits => self.draw_commits(inner, focused, buf),
                GitPanel::Diff => self.draw_diff(inner, focused, buf),
            }
        }
        let text = truncate_end(self.hint(), hint.width as usize);
        if hint.width as usize > text.width() {
            buf.set_string(hint.x, hint.y, text, self.theme.get("ui.git.hint"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_share_the_screen() {
        let (panels, hint) = layout(Rect::new(0, 0, 100, 30));
        let left: u16 = panels[..3].iter().map(|(r, _)| r.height).sum();
        assert_eq!(left, 29, "the left panels fill the height");
        assert_eq!(panels[3].0, Rect::new(35, 0, 65, 29));
        assert_eq!(hint, Rect::new(0, 29, 100, 1));
    }

    #[test]
    fn the_blames_code_is_highlighted() {
        let Some(dir) = crate::git::tests_support::repo() else {
            return;
        };
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let path = root.join("main.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        crate::git::stage(&root, &["main.rs"]).unwrap();
        crate::git::commit(&root, "main").unwrap();
        let mut editor = Editor::default();
        editor.open(&path).unwrap();
        editor.git_blame_view().unwrap();
        let blame = editor.git_gui().unwrap().blame.as_ref().unwrap();
        assert_eq!(blame.language.as_deref(), Some("rust"));

        let theme = crate::config::Config::default().theme;
        let mut engine = crate::syntax::SyntaxEngine::default();
        let highlights = engine.highlight_as(
            &blame.document,
            blame.language.as_deref(),
            &editor.languages,
            &editor.syntax_trees,
            0..1,
        );
        assert!(highlights.is_some());
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        GitGuiView::new(&editor, &theme, [highlights.cloned(), None]).render(area, &mut buf);
        let row: String = (0..80).map(|x| buf[(x, 1)].symbol()).collect();
        let x = row.find("fn main").expect("the code");
        let x = row[..x].chars().count() as u16;
        assert_eq!(
            buf[(x, 1)].fg,
            theme.get("keyword").fg.unwrap(),
            "`fn` as a keyword: {row}"
        );
        assert!(row.contains("main"), "the summary: {row}");
    }
}
