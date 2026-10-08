use std::path::Path;

use chrono::{DateTime, Local};
use serde_json::Value;

use crate::model::{Dot, Run, TriggerKind};

pub fn build_prompt(dot: &Dot, run: &Run, workspace: &Path, now: DateTime<Local>) -> String {
    let message = run
        .payload
        .as_ref()
        .and_then(|p| p.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    match run.trigger {
        TriggerKind::Resume => return message.to_string(),
        TriggerKind::Chat if run.session_id.is_some() => return message.to_string(),
        _ => {}
    }

    let mut out = String::new();
    out.push_str(dot.spec.instructions.trim());
    out.push_str("\n\n---\n\n");
    if run.trigger == TriggerKind::Chat {
        out.push_str("## Message from the user\n\n");
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
    out
}
