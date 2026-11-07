//! Esc arriving glued to the next key (seen as Alt+key), and the key
//! timeout for half-typed sequences, end to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use ferra::editor::Mode;

#[test]
fn an_unbound_alt_key_is_esc_then_the_key() {
    let mut app = app_with_config("");
    // Typed fast, `<Esc>x` reaches us as Alt+x.
    keys(&mut app, "iabc<A-x>");
    assert_eq!(text(&app), "ab", "left insert mode, then x deleted the c");
    assert_eq!(app.editor().mode(), Mode::Normal);
}

#[test]
fn esc_then_a_count_and_motion() {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, "i1<CR>2<CR>3<CR>4<CR>5<A-3>G");
    assert_eq!(app.editor().mode(), Mode::Normal);
    assert_eq!(cursor(&app).0, 2, "<A-3>G is <Esc>3G");
}

#[test]
fn a_bound_alt_key_keeps_its_binding() {
    let mut app = app_with_config("[keys.normal]\n\"<A-x>\" = \"move_down\"");
    keys(&mut app, "ione<CR>two<Esc>gg<A-x>");
    assert_eq!(text(&app), "one\ntwo", "nothing deleted");
    assert_eq!(cursor(&app).0, 1);
}

#[test]
fn split_keys_are_recorded_into_macros() {
    let mut app = app_with_config("");
    keys(&mut app, "qaAx<A-0>q");
    keys(&mut app, "@a");
    assert_eq!(text(&app), "xx");
    assert_eq!(cursor(&app), (0, 0), "the replay also ended with <Esc>0");
}

#[test]
fn timeout_types_a_half_finished_insert_binding() {
    let mut app = app_with_config("[keys.insert]\njk = \"normal_mode\"");
    keys(&mut app, "ij");
    assert_eq!(text(&app), "", "waiting to see if k follows");
    app.handle_key_timeout();
    assert_eq!(text(&app), "j");
    keys(&mut app, "k");
    assert_eq!(text(&app), "jk", "too late to count as jk");
    assert_eq!(app.editor().mode(), Mode::Insert);

    keys(&mut app, "jk");
    assert_eq!(app.editor().mode(), Mode::Normal, "jk in time still works");
}

#[test]
fn timeout_makes_a_shadowed_binding_reachable() {
    let mut app =
        app_with_config("[editor]\nauto-indent = false\n[keys.normal]\ng = \"goto_file_end\"");
    keys(&mut app, "i1<CR>2<CR>3<Esc>gg");
    assert_eq!(cursor(&app).0, 0, "gg still wins when typed in time");
    keys(&mut app, "g");
    app.handle_key_timeout();
    assert_eq!(cursor(&app).0, 2, "g alone, after the timeout");
}

#[test]
fn timeout_keeps_waiting_on_a_normal_mode_prefix() {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, "i1<CR>2<CR>3<Esc>g");
    app.handle_key_timeout();
    keys(&mut app, "g");
    assert_eq!(cursor(&app).0, 0, "gg, however slowly typed");
}

#[test]
fn dot_repeats_text_typed_by_a_timeout() {
    let mut app = app_with_config("[keys.insert]\njk = \"normal_mode\"");
    keys(&mut app, "ij");
    app.handle_key_timeout();
    keys(&mut app, "x<Esc>.");
    assert_eq!(text(&app), "jjxx");
}
