//! The check-in tools of the local engine (`notify`, `ask`, `follow_up`; policy names `Notify`,
//! `Ask`, `FollowUp`): built in, as `context` is (check-ins spec section 2). What they do is
//! `crate::checkins`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::checkins::{CheckIns, Tool};

/// The function definitions, offered to every run of an engine that has check-ins.
pub fn schemas() -> Vec<Value> {
    Tool::ALL
        .iter()
        .map(|t| tool(t.name(), t.description(), t.properties(), t.required()))
        .collect()
}

/// Runs an allowed call of the check-in tool `name` (its input as `prepare` left it) for the
/// run `run_id`. A broken rule or a call that could not be stored is an error result; the run
/// goes on.
pub async fn run(checkins: &CheckIns, run_id: &str, name: &str, input: &Value) -> ToolOutput {
    let Some(check_in) = Tool::from_name(name) else {
        return ToolOutput::err(format!("unknown tool '{name}'"));
    };
    match checkins.call(run_id, check_in, input).await {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
