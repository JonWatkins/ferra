//! The coding agent (`editor/agent.rs`, `agent/`): `:agent` shows the
//! chat, `:agent_ask` sends it a message.

use crate::commands::CommandRegistry;
use crate::editor::input_box::InputBox;

pub fn register(r: &mut CommandRegistry) {
    r.register(
        "agent",
        "Show the coding agent's chat (starting the agent: the editor option agent says which)",
        |cx| {
            cx.editor.agent_open();
            Ok(())
        },
    );
    r.register(
        "agent_ask",
        "Send the coding agent a message (asked for, with none given)",
        |cx| {
            if cx.args.is_empty() {
                let name = cx.editor.options.agent.clone();
                let mut input = InputBox::new(&format!("Ask {name}"), "", "agent_ask");
                input.placeholder = "What should it do?".into();
                cx.editor.open_input_box(input);
                return Ok(());
            }
            let text = cx.args.join(" ");
            cx.editor.agent_prompt(&text)
        },
    );
    r.register(
        "agent_cancel",
        "Stop what the coding agent is doing",
        |cx| {
            cx.editor.agent_cancel();
            Ok(())
        },
    );
    r.register(
        "agent_stop",
        "Stop the coding agent's program",
        |cx| {
            cx.editor.agent_stop();
            Ok(())
        },
    );
    r.register(
        "agent_hide",
        "The agent's chat popup closed",
        |cx| {
            cx.editor.agent_hide();
            Ok(())
        },
    );
    for n in 1..=4usize {
        r.register(
            format!("agent_option_{n}"),
            "Answer the coding agent's question with that option",
            move |cx| cx.editor.agent_choose(n),
        );
    }
}
