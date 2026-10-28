use std::path::Path;

use chrono::{DateTime, Local};
use serde_json::Value;

use crate::apps::RunApps;
use crate::context::{self, Collection};
use crate::memory;
use crate::model::{Dot, EngineKind, Memory, Run, TriggerKind};
use crate::policy::Action;

/// The key `Runtime::chat` sets in a chat run's payload: the person typed the message in a chat
/// (conversational chat spec section 3). Chat runs Dots makes itself carry no such key.
pub const TYPED: &str = "typed";

/// What a person's chat message's first prompt tells the dot (conversational chat spec section
/// 3): a chat is a conversation, not a run of its instructions.
pub const CONVERSATION: &str = "## Chat with the person\n\n\
    This is a conversation with the person who set you up, not a run of the instructions above.\n\n\
    - Answer questions directly, also ones that have nothing to do with your instructions.\n\
    - Run the process your instructions describe only when the person asks you to run it.\n\
    - When asked to change something in your work, change only that, without redoing the rest.\n\
    - Files a past run wrote are in that run's own copy, not in your workspace. To change them, \
    tell the person to use Reply on that run's card.\n";

/// The standing rule on summaries of a person's chat: only work ends with one.
pub const CHAT_SUMMARY_RULE: &str = "- When you ran your instructions or changed files, end with \
    a short summary: what you did, what changed, and what needs human review. Otherwise your \
    answer is your reply.\n";

/// The run carries a message the person typed in a chat, not one Dots wrote for them.
pub fn is_typed(run: &Run) -> bool {
    run.trigger == TriggerKind::Chat
        && run
            .payload
            .as_ref()
            .and_then(|p| p.get(TYPED))
            .and_then(Value::as_bool)
            == Some(true)
}

/// The name a run's model calls the `context` tool by: Dots' own MCP server's tool on the
/// Claude engine, a built-in tool on the local engine.
pub fn context_tool_name(engine: EngineKind) -> &'static str {
    match engine {
        EngineKind::Claude => "mcp__dots__context",
        EngineKind::Local => "context",
    }
}

/// What a run's prompt names beyond the dot and the run: its context collections, the dot's
/// memories and the run's apps.
#[derive(Default, Clone, Copy)]
pub struct PromptParts<'a> {
    pub contexts: &'a [Collection],
    pub memories: &'a [Memory],
    pub apps: Option<&'a RunApps>,
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

/// The prompt of `run` for a dot without apps (`build_prompt_full`).
pub fn build_prompt_with_memories(
    dot: &Dot,
    run: &Run,
    workspace: &Path,
    now: DateTime<Local>,
    global: &str,
    contexts: &[Collection],
    memories: &[Memory],
) -> String {
    let parts = PromptParts {
        contexts,
        memories,
        apps: None,
    };
    build_prompt_full(dot, run, workspace, now, global, parts)
}

/// The prompt of `run`. A thread's first prompt starts with `global` (the global instructions of
/// Settings) when it is not blank and the dot uses it, has the dot's memories after its
/// instructions (`memory::prompt_section`, dot memory spec section 3) unless its policy denies
/// `Memory` (the person turned memory off), names the run's apps (app library spec section 6)
/// and ends with the run's context collections; resumes, chat follow-ups and follow-up runs
/// that resume a session send only their message.
pub fn build_prompt_full(
    dot: &Dot,
    run: &Run,
    workspace: &Path,
    now: DateTime<Local>,
    global: &str,
    parts: PromptParts<'_>,
) -> String {
    let PromptParts {
        contexts,
        memories,
        apps,
    } = parts;
    let message = run
        .payload
        .as_ref()
        .and_then(|p| p.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    match run.trigger {
        TriggerKind::Resume => return message.to_string(),
        TriggerKind::Chat | TriggerKind::FollowUp | TriggerKind::Workflow
            if run.session_id.is_some() =>
        {
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
    let typed = is_typed(run);
    if typed {
        out.push_str(CONVERSATION);
        out.push('\n');
    }
    if run.trigger == TriggerKind::Chat {
        out.push_str("## Message from the user\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else if run.trigger == TriggerKind::App {
        // A run an app started (app library spec section 7): the app's message, by its title.
        let app = run
            .payload
            .as_ref()
            .and_then(|p| p.get("app_title"))
            .and_then(Value::as_str)
            .unwrap_or("an app");
        out.push_str(&format!("## Message from the app {app}\n\n"));
        // Quoted: the text an app builds (from task titles a dot may have written) cannot open
        // a section of its own, such as "## Message from the user".
        for line in message.lines() {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    } else if let Some(section) = workflow_section(run) {
        // A workflow step (teams spec section 5.2): the workflow, the task and the hand-offs,
        // written by the workflow run.
        out.push_str(section.trim());
        out.push_str("\n\n");
    } else if run.trigger == TriggerKind::FollowUp {
        // A follow-up whose thread could not be resumed (check-ins spec section 6).
        out.push_str("## Follow-up\n\n");
        out.push_str(message);
        out.push_str("\n\n");
    } else if run.trigger == TriggerKind::Watcher {
        // What woke the dot, and the outside text apart from it (watchers spec section 5).
        out.push_str(&crate::watchers::describe::section(
            run.payload.as_ref().unwrap_or(&Value::Null),
            now,
        ));
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
    if typed {
        out.push_str(CHAT_SUMMARY_RULE);
    } else {
        out.push_str(
            "- End with a short summary: what you did, what changed, and what needs human review.\n",
        );
    }
    // A dot with a browser is told how to use it (browser spec section 4).
    if dot.spec.browser {
        out.push('\n');
        out.push_str(crate::browser::PROMPT);
    }
    if let Some(apps) = apps {
        out.push_str("\n## Apps\n\n");
        out.push_str(&apps.prompt_section(dot.spec.engine));
    }
    let section = context::prompt_section(contexts, context_tool_name(dot.spec.engine));
    if !section.is_empty() {
        out.push_str("\n## Shared context\n\n");
        out.push_str(&section);
        out.push('\n');
    }
    out
}

/// The `## Workflow` section a workflow run wrote into the payload of a step's run.
fn workflow_section(run: &Run) -> Option<&str> {
    if run.trigger != TriggerKind::Workflow {
        return None;
    }
    run.payload
        .as_ref()?
        .pointer("/workflow/section")?
        .as_str()
        .filter(|s| !s.trim().is_empty())
}
