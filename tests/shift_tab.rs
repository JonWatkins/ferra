//! Shift+Tab takes a level of indentation off: in insert mode the cursor's
//! line (with Vim's <C-d>, and <C-t> to add one), in normal mode [count]
//! lines like <<, and in visual mode the selection (with <Tab> to indent).

mod common;

use common::{app, cursor, keys, text};

#[test]
fn shift_tab_in_insert_mode_outdents_the_line_keeping_the_cursor_on_its_text() {
    let mut app = app();
    keys(&mut app, "i        let x<S-Tab>");
    assert_eq!(text(&app), "    let x");
    assert_eq!(cursor(&app), (0, 9), "still after `x`");
    keys(&mut app, " = 1<S-Tab><S-Tab>");
    assert_eq!(text(&app), "let x = 1", "and on to none");
    assert_eq!(cursor(&app), (0, 9));
    keys(&mut app, "<Esc>u");
    assert_eq!(text(&app), "", "part of the insert, undone with it");
}

#[test]
fn ctrl_t_and_ctrl_d_indent_and_outdent_in_insert_mode() {
    let mut app = app();
    keys(&mut app, "ifoo<C-t><C-t>");
    assert_eq!(text(&app), "        foo");
    assert_eq!(cursor(&app), (0, 11));
    keys(&mut app, "<C-d>");
    assert_eq!(text(&app), "    foo");
    assert_eq!(cursor(&app), (0, 7));
}

#[test]
fn a_cursor_in_the_indentation_stays_within_what_is_left() {
    let mut app = app();
    keys(&mut app, "i        x<Esc>0llli<S-Tab>");
    assert_eq!(text(&app), "    x");
    assert_eq!(cursor(&app), (0, 3), "it was at column 3, still inside");
    keys(&mut app, "<Esc>0wi<S-Tab>");
    assert_eq!(
        cursor(&app),
        (0, 0),
        "at the text, which is now at the start"
    );
}

#[test]
fn shift_tab_in_normal_mode_outdents_count_lines() {
    let mut app = app();
    // Auto-indent carries the indent on to the next lines.
    keys(&mut app, "i    a<CR>b<CR>c<Esc>gg");
    assert_eq!(text(&app), "    a\n    b\n    c");
    keys(&mut app, "2<S-Tab>");
    assert_eq!(text(&app), "a\nb\n    c");
}

#[test]
fn tab_and_shift_tab_shift_a_visual_selection() {
    let mut app = app();
    keys(&mut app, "ia<CR>b<CR>c<Esc>ggVj<Tab>");
    assert_eq!(text(&app), "    a\n    b\nc");
    assert_eq!(app.editor().mode(), ferra::editor::Mode::Normal);
    keys(&mut app, "Vj<S-Tab>");
    assert_eq!(text(&app), "a\nb\nc");
}

#[test]
fn tabs_are_used_where_the_buffer_indents_with_them() {
    let mut app = app();
    keys(&mut app, ":setlocal noexpand-tab<CR>i\t\tx<S-Tab>");
    assert_eq!(text(&app), "\tx");
    assert_eq!(cursor(&app), (0, 2));
}
