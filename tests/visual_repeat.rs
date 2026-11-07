//! `.` after an operator on a visual selection acts on the same amount of
//! text from the cursor, as in Vim, rather than replaying the motion that
//! made the selection. Visual `>` and `<` take a count. End to end.

mod common;

use common::{app_with_config, cursor, keys, text};
use ferra::App;
use ferra::editor::Mode;

/// `content` typed without auto-indent, cursor at the start.
fn with_text(content: &str) -> App {
    let mut app = app_with_config("[editor]\nauto-indent = false");
    keys(&mut app, &format!("i{content}<Esc>gg0"));
    app
}

fn register(app: &App, name: char) -> Option<String> {
    app.editor()
        .registers
        .get(Some(name))
        .map(|r| r.text.clone())
}

#[test]
fn characters_on_one_line_repeat_as_the_same_number() {
    let mut app = with_text("ab cdefg");
    keys(&mut app, "ved");
    assert_eq!(text(&app), " cdefg");
    keys(&mut app, "l.");
    assert_eq!(text(&app), " efg", "two characters, not to the word's end");
    assert_eq!(app.editor().mode(), Mode::Normal);
    keys(&mut app, "u");
    assert_eq!(text(&app), " cdefg", "one undo step");
}

#[test]
fn lines_repeat_as_the_same_number_of_lines() {
    let mut app = with_text("a<CR>b<CR><CR>c<CR>d<CR>e");
    keys(&mut app, "Vipd");
    assert_eq!(text(&app), "\nc\nd\ne");
    keys(&mut app, "j.");
    assert_eq!(text(&app), "\ne", "two lines, not the whole paragraph");
}

#[test]
fn several_lines_of_characters_end_at_the_same_column() {
    let mut app = with_text("abc<CR>defgh<CR>ijk<CR>lm nop");
    keys(&mut app, "vjed");
    assert_eq!(text(&app), "\nijk\nlm nop");
    keys(&mut app, "j0.");
    assert_eq!(text(&app), "\np", "to column 4 of the next line");
}

#[test]
fn a_block_repeats_with_the_same_rows_and_width() {
    let mut app = with_text("ab cd<CR>ab cd<CR>xyzw v<CR>xyzw v");
    keys(&mut app, "<C-v>jed");
    assert_eq!(text(&app), " cd\n cd\nxyzw v\nxyzw v");
    keys(&mut app, "jj0.");
    assert_eq!(text(&app), " cd\n cd\nzw v\nzw v");
}

#[test]
fn a_block_to_each_rows_end_repeats_to_each_end() {
    let mut app = with_text("xa<CR>xbb<CR>xccc<CR>xdddd");
    keys(&mut app, "l<C-v>j$d");
    assert_eq!(text(&app), "x\nx\nxccc\nxdddd");
    keys(&mut app, "jjl.");
    assert_eq!(text(&app), "x\nx\nx\nx");
}

#[test]
fn a_block_keeps_its_width_when_the_last_row_is_short() {
    let mut app = with_text("abcd<CR>abcd<CR>abcd<CR>ab");
    keys(&mut app, "<C-v>jlld");
    assert_eq!(text(&app), "d\nd\nabcd\nab");
    keys(&mut app, "jj.");
    assert_eq!(text(&app), "d\nd\nd\n", "the short row loses what it has");
}

#[test]
fn a_change_repeats_on_the_same_number_of_characters() {
    let mut app = with_text("ab cdef");
    keys(&mut app, "vecX<Esc>");
    assert_eq!(text(&app), "X cdef");
    keys(&mut app, "w.");
    assert_eq!(text(&app), "X Xef");
}

#[test]
fn a_block_insert_repeats_on_as_many_rows() {
    let mut app = with_text("a<CR>b<CR>c<CR>d<CR>e");
    keys(&mut app, "<C-v>jI-<Esc>jj.");
    assert_eq!(text(&app), "-a\n-b\n-c\n-d\ne");
}

#[test]
fn the_register_is_used_again() {
    let mut app = with_text("abcdef");
    keys(&mut app, "vl\"ad");
    assert_eq!(register(&app, 'a').as_deref(), Some("ab"));
    keys(&mut app, ".");
    assert_eq!(text(&app), "ef");
    assert_eq!(register(&app, 'a').as_deref(), Some("cd"));
}

#[test]
fn a_count_on_dot_is_ignored() {
    let mut app = with_text("abcdefghij");
    keys(&mut app, "vlld5.");
    assert_eq!(text(&app), "ghij", "three characters again, not five");
}

#[test]
fn a_selection_cut_short_by_the_line_end() {
    let mut app = with_text("abcdef<CR>ab");
    keys(&mut app, "vlllldj0.");
    assert_eq!(text(&app), "f\n", "only what the line has");
    assert_eq!(cursor(&app), (1, 0));
}

#[test]
fn after_a_macro_dot_repeats_its_visual_change() {
    let mut app = with_text("abcdefghijkl");
    keys(&mut app, "qavlldq");
    assert_eq!(text(&app), "defghijkl");
    keys(&mut app, "@a");
    assert_eq!(text(&app), "ghijkl");
    keys(&mut app, ".");
    assert_eq!(text(&app), "jkl");
}

#[test]
fn a_normal_change_after_a_visual_one_repeats_normally() {
    let mut app = with_text("abcdefgh");
    keys(&mut app, "vld");
    keys(&mut app, "x.");
    assert_eq!(text(&app), "efgh", "x then x again, not a selection");
}

#[test]
fn visual_shifts_take_a_count() {
    let mut app = with_text("a<CR>b<CR>c");
    keys(&mut app, "Vj2>");
    assert_eq!(text(&app), "        a\n        b\nc");
    keys(&mut app, "j.");
    assert_eq!(text(&app), "        a\n                b\n        c");
    keys(&mut app, "u");
    assert_eq!(text(&app), "        a\n        b\nc", "each one undo step");

    let mut app = with_text("ab<CR>cd");
    keys(&mut app, "l<C-v>j2>");
    assert_eq!(text(&app), "a        b\nc        d");
    keys(&mut app, "<C-v>j3<");
    assert_eq!(text(&app), "ab\ncd");
}
