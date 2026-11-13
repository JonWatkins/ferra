//! The find box (`editor/find.rs`), at the top right of the active window:
//!
//! ```text
//! ╭──────────────────────────────────────────────────────────╮
//! │ Find      needle█                             2 of 7     │
//! │                                                          │
//! │ Replace   pin                                            │
//! │                                                          │
//! │  Aa  Match case A-c    W  Whole word A-w    .*  Regex A-r│
//! ╰──── ⏎ replace one  A-a replace all  Tab find  Esc close ─╯
//! ```
//!
//! The fields have their own background, a row apart, and the one with
//! focus has its label highlighted. The toggles are chips, highlighted
//! when on, each with what it does and its key. Text put in for you is
//! shown selected (typing replaces it).

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Clear, Widget},
};
use unicode_width::UnicodeWidthStr;

use crate::config::Theme;
use crate::editor::Mode;
use crate::editor::find::{FindCount, FindField, FindState};
use crate::editor::text_field::TextField;
use crate::input::{KeyCode, KeyPress, Keymap};

/// The keys the box shows, as bound in `[keys.find]` (so rebinding them
/// shows), labelled for the platform.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindKeys {
    pub case: Option<String>,
    pub word: Option<String>,
    pub regex: Option<String>,
    pub enter: Option<String>,
    pub prev: Option<String>,
    pub switch: Option<String>,
    pub close: Option<String>,
    pub replace_all: Option<String>,
}

impl FindKeys {
    pub fn from_keymap(keymap: &Keymap) -> Self {
        let label = |command| {
            keymap
                .hint_keys(Mode::Find, command)
                .map(|keys| key_label(&keys, cfg!(target_os = "macos")))
        };
        Self {
            case: label("find_toggle_case"),
            word: label("find_toggle_word"),
            regex: label("find_toggle_regex"),
            enter: label("find_enter"),
            prev: label("find_prev"),
            switch: label("find_switch_field"),
            close: label("find_close"),
            replace_all: label("find_replace_all"),
        }
    }
}

/// Keys as people read them: `⌃A`, `⌥⏎`, `⇧Tab` on macOS; `C-a`, `A-⏎`
/// elsewhere.
pub fn key_label(keys: &[KeyPress], mac: bool) -> String {
    keys.iter()
        .map(|key| {
            let m = key.modifiers;
            let modified = m.ctrl || m.alt;
            let name = match key.code {
                KeyCode::Char(' ') => "Space".to_string(),
                KeyCode::Char(c) if mac && modified => c.to_uppercase().to_string(),
                KeyCode::Char(c) => c.to_string(),
                KeyCode::Enter => "⏎".into(),
                KeyCode::Esc => "Esc".into(),
                KeyCode::Backspace => "⌫".into(),
                KeyCode::Delete => "Del".into(),
                KeyCode::Tab => "Tab".into(),
                KeyCode::Left => "←".into(),
                KeyCode::Right => "→".into(),
                KeyCode::Up => "↑".into(),
                KeyCode::Down => "↓".into(),
                KeyCode::Home => "Home".into(),
                KeyCode::End => "End".into(),
                KeyCode::PageUp => "PgUp".into(),
                KeyCode::PageDown => "PgDn".into(),
                KeyCode::F(n) => format!("F{n}"),
                KeyCode::PasteStart | KeyCode::PasteEnd => String::new(),
            };
            let mut label = String::new();
            for (on, symbol, prefix) in [
                (m.ctrl, "⌃", "C-"),
                (m.alt, "⌥", "A-"),
                (m.shift, "⇧", "S-"),
            ] {
                if on {
                    label.push_str(if mac { symbol } else { prefix });
                }
            }
            label + &name
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const WIDTH: u16 = 62;
/// The labels' column ("Replace" and a gap).
const LABEL: u16 = 10;
/// Room for the count after the find field ("10000 of 10000+" is longer,
/// and cut).
const COUNT: u16 = 13;

/// The toggles: the chip, what it does (and shorter).
const TOGGLES: [(&str, &str, &str); 3] = [
    ("Aa", "Match case", "Case"),
    ("W", "Whole word", "Word"),
    (".*", "Regex", "Regex"),
];

/// How much of each toggle is shown: its name (long or short), its key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Names {
    Long,
    Short,
    None,
}

pub struct FindView<'a> {
    state: &'a FindState,
    count: FindCount,
    keys: &'a FindKeys,
    theme: &'a Theme,
}

/// Where the parts go inside the border.
struct Parts {
    inner: Rect,
    /// The fields' background starts here; their text a column in.
    field_x: u16,
    field_width: u16,
    replace_y: u16,
    toggles_y: u16,
}

impl Parts {
    fn new(state: &FindState, area: Rect) -> Self {
        let inner = Block::bordered().inner(area);
        let field_width = inner.width.saturating_sub(1 + LABEL + 1 + COUNT).max(3);
        let replace_y = inner.y + 2;
        let toggles_y = if state.replacing {
            inner.y + 4
        } else {
            inner.y + 2
        };
        Self {
            inner,
            field_x: inner.x + 1 + LABEL,
            field_width,
            replace_y,
            toggles_y,
        }
    }

    /// The columns a field's text gets (inside its padding).
    fn text_width(&self) -> u16 {
        self.field_width.saturating_sub(2).max(1)
    }
}

/// The first char shown of a field `width` wide, so its cursor is in view.
pub(super) fn field_scroll(field: &TextField, width: u16) -> usize {
    field
        .cursor()
        .saturating_sub(width.saturating_sub(1) as usize)
}

impl<'a> FindView<'a> {
    pub fn new(
        state: &'a FindState,
        count: FindCount,
        keys: &'a FindKeys,
        theme: &'a Theme,
    ) -> Self {
        Self {
            state,
            count,
            keys,
            theme,
        }
    }

    /// The bottom edge's hints for the field with focus, longest first.
    fn hints(&self) -> Vec<String> {
        let k = self.keys;
        let pair =
            |key: &Option<String>, what: &str| key.as_ref().map(|key| format!("{key} {what}"));
        let parts: Vec<Option<String>> = match self.state.focus {
            FindField::Find => vec![
                pair(&k.enter, "next"),
                pair(&k.prev, "prev"),
                pair(&k.switch, "replace"),
                pair(&k.close, "close"),
            ],
            FindField::Replace => vec![
                pair(&k.enter, "replace one"),
                pair(&k.replace_all, "replace all"),
                pair(&k.switch, "find"),
                pair(&k.close, "close"),
            ],
        };
        let parts: Vec<String> = parts.into_iter().flatten().collect();
        // All of them, then fewer from the end, keeping "close".
        let mut hints = Vec::new();
        for n in (1..=parts.len()).rev() {
            let mut shown: Vec<&str> = parts[..n.saturating_sub(1)]
                .iter()
                .map(String::as_str)
                .collect();
            if let Some(last) = parts.last() {
                shown.push(last);
            }
            hints.push(format!(" {} ", shown.join("  ")));
        }
        hints
    }

    /// The box's area: the top right of `window`, a column in from its edge.
    pub fn area(state: &FindState, window: Rect) -> Rect {
        // Borders, the rows, and a blank row between each.
        let height = if state.replacing { 7 } else { 5 };
        let width = WIDTH
            .min(window.width.saturating_sub(2))
            .max(window.width.min(28));
        let x = window.right().saturating_sub(width + 1).max(window.x);
        Rect::new(x, window.y, width, height.min(window.height))
    }

    /// Where the terminal cursor goes: in the field with focus.
    pub fn cursor_position(state: &FindState, area: Rect) -> Option<Position> {
        let parts = Parts::new(state, area);
        let (field, y) = match state.focus {
            FindField::Find => (&state.find, parts.inner.y),
            FindField::Replace => (&state.replace, parts.replace_y),
        };
        if y >= parts.inner.bottom() {
            return None;
        }
        let col = field.cursor() - field_scroll(field, parts.text_width());
        Some(Position::new(parts.field_x + 1 + col as u16, y))
    }

    fn draw_field(
        &self,
        field: &TextField,
        label: &str,
        focused: bool,
        y: u16,
        parts: &Parts,
        buf: &mut Buffer,
    ) {
        let theme = self.theme;
        let label_style = if focused {
            theme.get("ui.find.label.focused")
        } else {
            theme.get("ui.find.label")
        };
        buf.set_string(parts.inner.x + 1, y, label, label_style);
        let style = if focused {
            theme.get("ui.find.field.focused")
        } else {
            theme.get("ui.find.field")
        };
        buf.set_style(Rect::new(parts.field_x, y, parts.field_width, 1), style);
        let width = parts.text_width();
        let shown: String = field
            .text()
            .chars()
            .skip(field_scroll(field, width))
            .take(width as usize)
            .collect();
        let text_style = if field.is_selected() {
            style.patch(theme.get("ui.find.field.selected"))
        } else {
            style
        };
        buf.set_stringn(parts.field_x + 1, y, shown, width as usize, text_style);
    }

    fn count_span(&self) -> Span<'static> {
        let theme = self.theme;
        match self.count {
            FindCount::Empty => Span::raw(""),
            FindCount::Invalid => Span::styled("Bad regex", theme.get("ui.find.none")),
            FindCount::Found { total: 0, .. } => {
                Span::styled("No results", theme.get("ui.find.none"))
            }
            FindCount::Found {
                current,
                total,
                more,
            } => {
                let current = current.map_or("?".to_string(), |n| n.to_string());
                let plus = if more { "+" } else { "" };
                Span::styled(
                    format!("{current} of {total}{plus}"),
                    theme.get("ui.find.count"),
                )
            }
        }
    }

    /// The toggles' row: as much as fits of each chip, its name and key.
    fn draw_toggles(&self, x: u16, y: u16, right: u16, buf: &mut Buffer) {
        let theme = self.theme;
        let state = self.state;
        let on = [state.match_case, state.whole_word, state.regex];
        let keys = toggle_keys(self.keys);
        let (names, show_keys) = toggle_fit(self.keys, x, right);
        let mut x = x;
        for (((chip, long, short), on), key) in TOGGLES.into_iter().zip(on).zip(keys) {
            let chip_style = if on {
                theme.get("ui.find.toggle.on")
            } else {
                theme.get("ui.find.toggle")
            };
            let mut spans = vec![Span::styled(format!(" {chip} "), chip_style)];
            if let Some(name) = name_of(names, long, short) {
                spans.push(Span::styled(
                    format!(" {name}"),
                    theme.get("ui.find.toggle.label"),
                ));
            }
            if let Some(key) = key.as_ref().filter(|_| show_keys) {
                spans.push(Span::styled(format!(" {key}"), theme.get("ui.find.hint")));
            }
            for span in spans {
                if x >= right {
                    return;
                }
                let width = (right - x).min(span.width() as u16);
                x = buf.set_span(x, y, &span, width).0;
            }
            x += 3;
        }
    }

    /// Where the box's parts are, for the mouse: the fields, and each
    /// toggle (chip, name and key) with which it is (0 case, 1 word, 2
    /// regex).
    pub fn hits(state: &FindState, keys: &FindKeys, area: Rect) -> FindHits {
        let parts = Parts::new(state, area);
        let inner = parts.inner;
        let field = |y| Rect::new(parts.field_x, y, parts.field_width, 1);
        let x0 = inner.x + 1;
        let right = inner.right();
        let (names, show_keys) = toggle_fit(keys, x0, right);
        let mut x = x0;
        let mut toggles = Vec::new();
        for (i, ((chip, long, short), key)) in
            TOGGLES.into_iter().zip(toggle_keys(keys)).enumerate()
        {
            let width = chip.width() as u16
                + 2
                + name_of(names, long, short).map_or(0, |n| 1 + n.width() as u16)
                + key
                    .as_ref()
                    .filter(|_| show_keys)
                    .map_or(0, |k| 1 + k.width() as u16);
            if x >= right || parts.toggles_y >= inner.bottom() {
                break;
            }
            toggles.push((Rect::new(x, parts.toggles_y, width.min(right - x), 1), i));
            x += width + 3;
        }
        FindHits {
            area,
            find: field(inner.y),
            replace: (state.replacing && parts.replace_y < inner.bottom())
                .then(|| field(parts.replace_y)),
            toggles,
        }
    }

    /// The field's column under `x` (for a click in it).
    pub fn field_col(field: &TextField, field_area: Rect, x: u16) -> usize {
        let width = field_area.width.saturating_sub(2).max(1);
        let col = field_scroll(field, width) + x.saturating_sub(field_area.x + 1) as usize;
        col.min(field.text().chars().count())
    }
}

/// Where the find box's parts were drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindHits {
    pub area: Rect,
    pub find: Rect,
    pub replace: Option<Rect>,
    pub toggles: Vec<(Rect, usize)>,
}

fn name_of(names: Names, long: &'static str, short: &'static str) -> Option<&'static str> {
    match names {
        Names::Long => Some(long),
        Names::Short => Some(short),
        Names::None => None,
    }
}

fn toggle_keys(keys: &FindKeys) -> [&Option<String>; 3] {
    [&keys.case, &keys.word, &keys.regex]
}

/// How much of the toggles fits between `x` and `right`, keeping the keys
/// longest: long names and keys, short names and keys, long names, short
/// names, the chips.
fn toggle_fit(keys: &FindKeys, x: u16, right: u16) -> (Names, bool) {
    let keys = toggle_keys(keys);
    let fits = |names: Names, show_keys: bool| {
        let width: usize = TOGGLES
            .iter()
            .zip(keys)
            .map(|((chip, long, short), key)| {
                chip.width()
                    + 2
                    + name_of(names, long, short).map_or(0, |n| 1 + n.width())
                    + key
                        .as_ref()
                        .filter(|_| show_keys)
                        .map_or(0, |k| 1 + k.width())
                    + 3
            })
            .sum();
        x as usize + width <= right as usize + 3
    };
    [
        (Names::Long, true),
        (Names::Short, true),
        (Names::Long, false),
        (Names::Short, false),
    ]
    .into_iter()
    .find(|&(names, show_keys)| fits(names, show_keys))
    .unwrap_or((Names::None, false))
}

impl Widget for FindView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 3 || area.height < 3 {
            return;
        }
        let theme = self.theme;
        let state = self.state;
        Clear.render(area, buf);
        buf.set_style(area, theme.get("ui.find"));

        // The keys for what you're doing, shortened to fit.
        let hints = self.hints();
        let mut block = Block::bordered()
            .border_set(border::ROUNDED)
            .border_style(theme.get("ui.find.border"));
        if let Some(hint) = hints
            .into_iter()
            .find(|hint| hint.width() + 4 <= area.width as usize)
        {
            block = block.title_bottom(
                Line::from(Span::styled(hint, theme.get("ui.find.hint"))).right_aligned(),
            );
        }
        let parts = Parts::new(state, area);
        block.render(area, buf);
        let inner = parts.inner;

        self.draw_field(
            &state.find,
            "Find",
            state.focus == FindField::Find,
            inner.y,
            &parts,
            buf,
        );
        if state.error().is_some() {
            let text = Rect::new(parts.field_x + 1, inner.y, parts.text_width(), 1);
            buf.set_style(text, theme.get("ui.find.none"));
        }
        let count = self.count_span();
        let count_x = parts.field_x + parts.field_width + 2;
        if count.width() > 0 && count_x < inner.right() {
            let room = inner.right() - count_x;
            buf.set_span(count_x, inner.y, &count, room);
        }

        if state.replacing && parts.replace_y < inner.bottom() {
            self.draw_field(
                &state.replace,
                "Replace",
                state.focus == FindField::Replace,
                parts.replace_y,
                &parts,
                buf,
            );
        }
        if parts.toggles_y < inner.bottom() {
            self.draw_toggles(inner.x + 1, parts.toggles_y, inner.right(), buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position as At;
    use crate::editor::Editor;
    use crate::input::parse_keys;

    /// `notation` labelled as this platform labels keys.
    fn label(notation: &str) -> String {
        key_label(&parse_keys(notation).unwrap(), cfg!(target_os = "macos"))
    }

    #[test]
    fn keys_are_labelled_with_symbols_on_macos() {
        let both = |notation| {
            let keys = parse_keys(notation).unwrap();
            (key_label(&keys, true), key_label(&keys, false))
        };
        assert_eq!(both("<C-a>"), ("⌃A".into(), "C-a".into()));
        assert_eq!(both("<A-r>"), ("⌥R".into(), "A-r".into()));
        assert_eq!(both("<C-CR>"), ("⌃⏎".into(), "C-⏎".into()));
        assert_eq!(both("<S-Tab>"), ("⇧Tab".into(), "S-Tab".into()));
        assert_eq!(both("<Up>"), ("↑".into(), "↑".into()));
        assert_eq!(both("gg"), ("g g".into(), "g g".into()));
    }

    #[test]
    fn the_hints_show_the_keys_bound() {
        let mut keymap = crate::config::Config::default().keys;
        keymap.bind(
            Mode::Find,
            &parse_keys("<C-t>").unwrap(),
            "find_toggle_case",
        );
        let keys = FindKeys::from_keymap(&keymap);
        assert_eq!(
            keys.case,
            Some(label("<C-a>")),
            "the shorter text wins a tie"
        );
        assert_eq!(keys.prev, Some(label("<Up>")), "fewest modifiers");
        assert_eq!(keys.replace_all, Some(label("<C-CR>")), "not an Alt key");
        assert_eq!(keys.close, Some("Esc".into()));
    }

    fn rows(state: &FindState, count: FindCount, width: u16) -> (Vec<String>, Rect, Buffer) {
        let window = Rect::new(0, 0, width, 10);
        let area = FindView::area(state, window);
        let mut buf = Buffer::empty(window);
        let config = crate::config::Config::default();
        let keys = FindKeys::from_keymap(&config.keys);
        FindView::new(state, count, &keys, &config.theme).render(area, &mut buf);
        let rows = (0..area.height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        (rows, area, buf)
    }

    #[test]
    fn draws_the_field_count_and_labelled_toggles_at_the_top_right() {
        let mut editor = Editor::default();
        editor.document.insert(At::new(0, 0), "needle hay needle");
        editor.document.set_cursor(At::new(0, 0));
        editor.open_find(false);
        let (rows, area, buf) = rows(editor.find_state(), editor.find_count(), 70);
        assert_eq!((area.x, area.width, area.height), (7, 62, 5));
        assert!(rows[1].contains("│ Find       needle "), "{rows:#?}");
        assert!(rows[1].contains("1 of 2"), "{rows:#?}");
        assert!(rows[2].trim_end().ends_with("│"), "a blank row: {rows:#?}");
        assert!(
            rows[3].contains(&format!(
                " Aa  Match case {}    W  Whole word {}    .*  Regex {}",
                label("<C-a>"),
                label("<C-w>"),
                label("<C-r>")
            )),
            "{rows:#?}"
        );
        assert!(rows[4].contains("⏎ next"), "{rows:#?}");
        let theme = crate::config::Config::default().theme;
        assert_eq!(
            buf[(area.x + 1 + 1 + LABEL, 1)].bg,
            theme.get("ui.find.field.focused").bg.unwrap(),
            "the field has its own background"
        );
        assert_ne!(
            theme.get("ui.find.field.focused").bg,
            theme.get("ui.find").bg
        );
        assert_eq!(
            FindView::cursor_position(editor.find_state(), area),
            Some(Position::new(area.x + 1 + 1 + LABEL + 1 + 6, 1)),
            "after the text"
        );
    }

    #[test]
    fn shows_the_replace_field_a_row_apart_and_no_results() {
        let mut editor = Editor::default();
        editor.document.insert(At::new(0, 0), "hay");
        editor.open_find(true);
        editor.find_edit(|f| f.push('z'));
        editor.find_switch_field();
        editor.find_edit(|f| f.push('q'));
        editor.find_toggle(|f| f.regex = true);
        let (rows, area, buf) = rows(editor.find_state(), editor.find_count(), 70);
        assert_eq!(area.height, 7);
        assert!(rows[1].contains("No results"), "{rows:#?}");
        assert!(rows[3].contains("│ Replace    q "), "{rows:#?}");
        assert!(rows[5].contains("Regex"), "{rows:#?}");
        assert!(
            rows[6].contains(&format!("{} replace all", label("<C-CR>"))),
            "{rows:#?}"
        );
        let theme = crate::config::Config::default().theme;
        let regex_chip = rows[5].find(".*").unwrap();
        let chip_x = rows[5][..regex_chip].chars().count() as u16;
        assert_eq!(
            buf[(chip_x, 5)].bg,
            theme.get("ui.find.toggle.on").bg.unwrap(),
            "on"
        );
        assert_eq!(
            FindView::cursor_position(editor.find_state(), area).map(|p| p.y),
            Some(3)
        );
    }

    #[test]
    fn a_narrow_box_shortens_the_toggles() {
        let mut editor = Editor::default();
        editor.open_find(false);
        let toggles = |width| rows_at(editor.find_state(), width).0[3].clone();
        let row = toggles(52);
        let case_key = format!("Case {}", label("<C-a>"));
        assert!(row.contains(&case_key) && !row.contains("Match"), "{row}");
        let row = toggles(46);
        assert!(row.contains("Case") && !row.contains(&case_key), "{row}");
        let row = toggles(30);
        assert!(row.contains("Aa") && !row.contains("Case"), "{row}");
    }

    fn rows_at(state: &FindState, width: u16) -> (Vec<String>, Rect, Buffer) {
        rows(state, FindCount::Empty, width)
    }

    #[test]
    fn a_long_field_scrolls_to_its_cursor() {
        let mut editor = Editor::default();
        editor.open_find(false);
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        editor.find_edit(|f| long.chars().for_each(|c| f.push(c)));
        let (rows, area, _) = rows(editor.find_state(), editor.find_count(), 50);
        assert!(rows[1].contains("789"), "the end shown: {rows:#?}");
        assert!(!rows[1].contains("abc"), "{rows:#?}");
        let cursor = FindView::cursor_position(editor.find_state(), area).unwrap();
        assert!(cursor.x < area.right() - 1);
    }
}
