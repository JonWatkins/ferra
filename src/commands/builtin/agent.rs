//! The coding agent (`editor/agent.rs`, `agent/`): `:agent` shows its
//! sidebar, `:agent_ask` sends it a message; the sidebar's box is edited
//! with `[keys.agent]`.

use crate::commands::CommandRegistry;
use crate::editor::text_field::TextField;

/// An edit to the box.
type FieldEdit = fn(&mut TextField);

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "agent",
        "Show the coding agent's sidebar, focused (starting the agent: the editor option agent says which)",
        |cx| {
            cx.editor.agent_open();
            Ok(())
        },
    );
    r.register(
        "agent_sidebar",
        "Show or hide the coding agent's sidebar",
        |cx| {
            cx.editor.agent_toggle();
            Ok(())
        },
    );
    r.register(
        "agent_ask",
        "Send the coding agent a message (with none given, its box is focused)",
        |cx| {
            if cx.args.is_empty() {
                cx.editor.agent_open();
                return Ok(());
            }
            let text = cx.args.join(" ");
            cx.editor.agent_prompt(&text)
        },
    );
    r.register("agent_send", "Agent sidebar: send what's typed", |cx| {
        cx.editor.agent_send()
    });
    r.register(
        "agent_leave",
        "Agent sidebar: focus back to the text (it stays shown)",
        |cx| {
            cx.editor.agent_leave();
            Ok(())
        },
    );
    r.register("agent_hide", "Hide the coding agent's sidebar", |cx| {
        cx.editor.agent_close();
        Ok(())
    });
    r.register(
        "agent_cancel",
        "Stop what the coding agent is doing",
        |cx| {
            cx.editor.agent_cancel();
            Ok(())
        },
    );
    r.register("agent_stop", "Stop the coding agent's program", |cx| {
        cx.editor.agent_stop();
        Ok(())
    });
    r.register(
        "agent_mode",
        "Change the coding agent's mode (a list of its modes)",
        |cx| cx.editor.agent_mode_picker(),
    );
    r.register(
        "agent_next_mode",
        "Switch the coding agent to its next mode",
        |cx| cx.editor.agent_next_mode(),
    );
    r.register(
        "agent_set_mode",
        "Switch the coding agent to a mode (its id)",
        |cx| match cx.args.first() {
            Some(id) => cx.editor.agent_set_mode(id),
            None => cx.editor.agent_mode_picker(),
        },
    );
    r.register(
        "agent_choose",
        "Choose which coding agent to talk to (from [agents])",
        |cx| cx.editor.agent_picker(),
    );
    r.register(
        "agent_use",
        "Talk to another coding agent (its name in [agents])",
        |cx| match cx.args.first() {
            Some(name) => cx.editor.agent_use(name),
            None => cx.editor.agent_picker(),
        },
    );
    r.register(
        "agent_expand",
        "Agent sidebar: show every tool call's output and changes, or hide them all",
        |cx| {
            cx.editor.agent_toggle_all_tools();
            Ok(())
        },
    );
    r.register(
        "agent_mention",
        "Agent sidebar: mention a file (@, then pick it); it's sent with the message",
        |cx| {
            cx.editor.agent_mention();
            Ok(())
        },
    );
    r.register(
        "agent_newline",
        "Agent sidebar: a new line in the box",
        |cx| {
            cx.editor.agent_input_edit(|f| f.push('\n'));
            Ok(())
        },
    );
    let scrolls: [(&str, &str, isize); 4] = [
        ("agent_scroll_up", "Agent sidebar: scroll the chat up", 3),
        (
            "agent_scroll_down",
            "Agent sidebar: scroll the chat down",
            -3,
        ),
        (
            "agent_page_up",
            "Agent sidebar: scroll the chat up a page",
            20,
        ),
        (
            "agent_page_down",
            "Agent sidebar: scroll the chat down a page",
            -20,
        ),
    ];
    for (name, description, rows) in scrolls {
        r.register(name, description, move |cx| {
            cx.editor.agent_scroll(rows);
            Ok(())
        });
    }
    let edits: [(&str, &str, FieldEdit); 8] = [
        (
            "agent_backspace",
            "Agent sidebar: delete the char before the cursor",
            |f| f.backspace(),
        ),
        (
            "agent_delete",
            "Agent sidebar: delete the char under the cursor",
            |f| f.delete(),
        ),
        (
            "agent_delete_word",
            "Agent sidebar: delete the word before the cursor",
            |f| f.delete_word(),
        ),
        (
            "agent_delete_to_start",
            "Agent sidebar: delete to the start",
            |f| f.delete_to_start(),
        ),
        ("agent_left", "Agent sidebar: cursor left", |f| f.left()),
        ("agent_right", "Agent sidebar: cursor right", |f| f.right()),
        ("agent_home", "Agent sidebar: to the start", |f| f.home()),
        ("agent_end", "Agent sidebar: to the end", |f| f.end()),
    ];
    for (name, description, edit) in edits {
        r.register(name, description, move |cx| {
            cx.editor.agent_input_edit(edit);
            Ok(())
        });
    }
    for n in 1..=4usize {
        r.register(
            format!("agent_option_{n}"),
            "Answer the coding agent's question with that option",
            move |cx| cx.editor.agent_choose(n),
        );
    }
}
