//! The menu bar (`editor/menu_bar.rs`).

use crate::commands::{CommandRegistry, Invocation};
use crate::editor::Editor;
use crate::editor::menu_bar::{MENUS, title_spans};

/// Opens menu `index` under its title.
fn open(editor: &mut Editor, index: usize) -> Result<(), String> {
    let x = title_spans(0).get(index).map_or(0, |span| span.0);
    editor.menu_open(index, x)
}

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "menu_open",
        "Open a menu of the menu bar (:menu_open edit; F10, or Alt and its letter)",
        |cx| {
            let index = match cx.args.first() {
                None => 0,
                Some(name) => {
                    let name = name.to_lowercase();
                    MENUS
                        .iter()
                        .position(|m| m.title.to_lowercase() == name)
                        .or_else(|| {
                            let mut chars = name.chars();
                            let letter = chars.next()?;
                            if chars.next().is_some() {
                                return None;
                            }
                            Editor::menu_for_letter(letter)
                        })
                        .ok_or_else(|| {
                            let titles: Vec<_> = MENUS.iter().map(|m| m.title).collect();
                            format!("No menu {name} (there's {})", titles.join(", "))
                        })?
                }
            };
            open(cx.editor, index)
        },
    );
    r.register("menu_run", "In a menu: run the selected item", |cx| {
        let Some((name, args)) = cx.editor.menu_chosen()? else {
            return Ok(());
        };
        let invocation = Invocation {
            args: &args,
            ..Invocation::default()
        };
        cx.registry.execute(&name, cx.editor, invocation)
    });
    // One command per menu, for keys (`<A-f>` = "menu_file").
    for (index, menu) in MENUS.iter().enumerate() {
        r.register(
            format!("menu_{}", menu.title.to_lowercase()),
            format!("Open the {} menu", menu.title),
            move |cx| open(cx.editor, index),
        );
    }
    for (name, delta, description) in [
        ("menu_prev", -1, "In a menu: the menu to its left"),
        ("menu_next", 1, "In a menu: the menu to its right"),
    ] {
        r.register(name, description, move |cx| {
            match cx.editor.menu_beside(delta) {
                Some(index) => open(cx.editor, index),
                None => Ok(()),
            }
        });
    }
}
