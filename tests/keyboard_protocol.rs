//! Keys the kitty keyboard protocol reports apart from the ones older
//! terminals send for them: `<C-[>`, `<C-i>`, `<C-m>`.

mod common;

use common::{app, app_with_config, cursor, keys, text};
use ferra::editor::Mode;

#[test]
fn ctrl_bracket_is_escape() {
    let mut app = app();
    keys(&mut app, "ihello<C-[>");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "d<C-[>x");
    assert_eq!(text(&app), "hell", "cancels a pending operator");
}

#[test]
fn unbound_ctrl_i_and_ctrl_m_type_a_tab_and_a_line_break() {
    let mut app = app();
    keys(&mut app, "ia<C-m><C-i>b<Esc>");
    assert_eq!(text(&app), "a\n    b");
}

#[test]
fn binding_ctrl_i_in_a_mode_takes_it_apart_from_tab_there() {
    let mut app = app_with_config("[keys.normal]\n\"<C-i>\" = \"move_down\"");
    keys(&mut app, "ia<CR>b<CR>c<Esc>gg");
    keys(&mut app, "<C-i>");
    assert_eq!(cursor(&app), (1, 0), "the binding runs");
    keys(&mut app, "A<C-i>x<Esc>");
    assert_eq!(text(&app), "a\nb    x\nc", "still Tab in insert mode");
}
