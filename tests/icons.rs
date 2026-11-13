//! File and folder icons with a Nerd Font (`icons = "nerd"`): in the file
//! tree, in place of the folder markers, and before files in the picker.

mod common;

use common::{app_with_config, keys, render};

const RUST: char = '\u{e68b}';
const MARKDOWN: char = '\u{e609}';
const FOLDER: char = '\u{e5ff}';
const FOLDER_OPEN: char = '\u{e5fe}';

// The tree and the picker list the folder the tests run in: this
// repository.

#[test]
fn the_tree_shows_folder_and_file_icons() {
    let mut app = app_with_config("[editor]\nicons = \"nerd\"\n");
    keys(&mut app, "<Space>e");
    let rows = render(&mut app, 60, 30).rows;
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.contains(&format!(" {name} ")) || r.trim_end().ends_with(name))
            .unwrap_or_else(|| panic!("{name}: {rows:#?}"))
            .clone()
    };
    assert!(row("src").contains(FOLDER), "{}", row("src"));
    assert!(row("README.md").contains(MARKDOWN), "{}", row("README.md"));
    assert!(
        row("Cargo.toml").contains(RUST),
        "by name: {}",
        row("Cargo.toml")
    );
}

#[test]
fn an_open_folder_has_the_open_icon_and_names_keep_their_column() {
    let mut plain = app_with_config("");
    let mut nerd = app_with_config("[editor]\nicons = \"nerd\"\n");
    for app in [&mut plain, &mut nerd] {
        keys(app, "<Space>e");
        // Down to src and open it.
        let rows = render(app, 60, 30).rows;
        let src = rows.iter().position(|r| r.contains("src")).unwrap();
        let tree_top = rows.iter().position(|r| r.contains("FERRA")).unwrap();
        for _ in 0..(src - tree_top - 1) {
            keys(app, "j");
        }
        keys(app, "l");
    }
    let plain_rows = render(&mut plain, 60, 30).rows;
    let nerd_rows = render(&mut nerd, 60, 30).rows;
    let src = nerd_rows.iter().find(|r| r.contains("src")).unwrap();
    assert!(src.contains(FOLDER_OPEN), "{src}");
    // The icon takes the marker's place: names start in the same column.
    let column = |rows: &[String], name: &str| {
        let row = rows.iter().find(|r| r.contains(name)).unwrap();
        row[..row.find(name).unwrap()].chars().count()
    };
    assert_eq!(
        column(&plain_rows, "main.rs"),
        column(&nerd_rows, "main.rs")
    );
    assert!(
        nerd_rows
            .iter()
            .any(|r| r.contains(&format!("{RUST} main.rs"))),
        "{nerd_rows:#?}"
    );
}

#[test]
fn the_picker_shows_files_icons() {
    let mut app = app_with_config("[editor]\nicons = \"nerd\"\n");
    keys(&mut app, "<Space>f");
    // The files are listed in the background.
    app.wait_for_background();
    keys(&mut app, "src/main.rs");
    let rows = render(&mut app, 80, 20).rows;
    assert!(
        rows.iter()
            .any(|r| r.contains(&format!("{RUST} src/main.rs"))),
        "{rows:#?}"
    );
}

#[test]
fn plain_icons_draw_no_file_icons() {
    let mut app = app_with_config("");
    keys(&mut app, "<Space>e");
    let rows = render(&mut app, 60, 30).rows;
    assert!(rows.iter().any(|r| r.contains("▸ src")), "{rows:#?}");
    assert!(!rows.iter().any(|r| r.contains(RUST) || r.contains(FOLDER)));
}
