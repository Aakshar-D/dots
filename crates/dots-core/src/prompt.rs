use std::path::Path;

use chrono::{DateTime, Local};
use serde_json::Value;

use crate::context::{self, Collection};
use crate::memory;
use crate::model::{Dot, EngineKind, Memory, Run, TriggerKind};
use crate::policy::Action;

/// The name a run's model calls the `context` tool by: Dots' own MCP server's tool on the
/// Claude engine, a built-in tool on the local engine.
pub fn context_tool_name(engine: EngineKind) -> &'static str {
    match engine {
        EngineKind::Claude => "mcp__dots__context",
        EngineKind::Local => "context",
    }
}

/// The prompt of `run` for a dot without memories (`build_prompt_with_memories`).
pub fn build_prompt(
    dot: &Dot,
    run: &Run,
    workspace: &Path,
    now: DateTime<Local>,
    global: &str,
    contexts: &[Collection],
) -> String {
    build_prompt_with_memories(dot, run, workspace, now, global, contexts, &[])
}

/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it, has the dot's `memories` after its
/// instructions (`memory::prompt_section`, dot memory spec section 3) unless its policy denies
/// `Memory` (the person turned memory off), and ends with the run's
/// context collections; resumes, chat follow-ups and follow-up runs that resume a session send
/// only their message.
pub fn build_prompt_with_memories(
    dot: &Dot,
    run: &Run,
    workspace: &Path,
    now: DateTime<Local>,
    global: &str,
    contexts: &[Collection],
    memories: &[Memory],
) -> String {
    let message = run
        .payload
        .as_ref()
        .and_then(|p| p.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    match run.trigger {
        TriggerKind::Resume => return message.to_string(),
        TriggerKind::Chat | TriggerKind::FollowUp if run.session_id.is_some() => {
            return message.to_string()
        }
        _ => {}
    }

    let mut out = String::new();
    let global = global.trim();
    if dot.spec.use_global_instructions && !global.is_empty() {
        out.push_str("## Instructions for every dot\n\n");
        out.push_str(global);
        out.push_str("\n\n## Instructions for this dot\n\n");
    }
    out.push_str(dot.spec.instructions.trim());
    let memory_off = dot.spec.policy.resolve(memory::POLICY_NAME, &Value::Null) == Action::Deny;
    let remembered = if memory_off {
        String::new()
    } else {
        memory::prompt_section(memories)
    };
    if !remembered.is_empty() {
        out.push_str("\n\n");
        out.push_str(remembered.trim_end());
    }
    out.push_str("\n\n---\n\n");
    if run.trigger == TriggerKind::Chat {
        out.push_str("## Message from the user\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else if run.trigger == TriggerKind::FollowUp {
        // A follow-up whose thread could not be resumed (check-ins spec section 6).
        out.push_str("## Follow-up\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else {
        out.push_str("## Trigger\n\n");
        out.push_str(&format!("- kind: {}\n", run.trigger.as_str()));
        out.push_str(&format!(
            "- time: {}\n",
            now.format("%Y-%m-%d %H:%M:%S %:z")
        ));
        match &run.payload {
            Some(p) => out.push_str(&format!(
                "- payload:\n\n```json\n{}\n```\n\n",
                serde_json::to_string_pretty(p).unwrap_or_default()
            )),
            None => out.push_str("- payload: none\n\n"),
        }
    }
    out.push_str("## Standing rules\n\n");
    out.push_str(&format!(
        "- Work only inside this directory: {}\n",
        workspace.display()
    ));
    out.push_str(
        "- If a tool call is queued for human approval, do not retry it; finish the remaining work.\n",
    );
    out.push_str(
        "- End with a short summary: what you did, what changed, and what needs human review.\n",
    );
    // A dot with a browser is told how to use it (browser spec section 4).
    if dot.spec.browser {
        out.push('\n');
        out.push_str(crate::browser::PROMPT);
    }
    let section = context::prompt_section(contexts, context_tool_name(dot.spec.engine));
    if !section.is_empty() {
        out.push_str("\n## Shared context\n\n");
        out.push_str(&section);
        out.push('\n');
    }
    out
}
