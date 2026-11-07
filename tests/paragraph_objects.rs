//! The paragraph text objects `ip` and `ap`, end to end.

mod common;

use common::{app_with_config, keys, text};
use ferra::App;
use ferra::editor::Mode;

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

#[test]
fn dap_deletes_a_paragraph_and_the_blank_after_it() {
    let mut app = with_text("a<CR>b<CR><CR>c");
    keys(&mut app, "dap");
    assert_eq!(text(&app), "c");
}

#[test]
fn dip_keeps_the_blank_lines() {
    let mut app = with_text("a<CR>b<CR><CR>c");
    keys(&mut app, "jdip");
    assert_eq!(text(&app), "\nc");
}

#[test]
fn yank_and_visual_select_whole_lines() {
    let mut app = with_text("a<CR>b<CR><CR>c");
    keys(&mut app, "yip");
    let yanked = app.editor().registers.get(None).unwrap().clone();
    assert_eq!((yanked.text.as_str(), yanked.linewise), ("a\nb", true));

    keys(&mut app, "Gvap");
    assert_eq!(app.editor().mode(), Mode::Visual);
    keys(&mut app, "d");
    assert_eq!(
        text(&app),
        "a\nb",
        "the last paragraph takes the blank before it"
    );
}
