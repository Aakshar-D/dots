//! The memory tools of the local engine (`remember`, `forget`, `recall`; policy name `Memory`):
//! built in, as the check-in tools are (dot memory spec section 3). What they do is
//! `crate::memory`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::memory::{Memories, Tool};

/// The function definitions, offered to every run of an engine that has check-ins.
pub fn schemas() -> Vec<Value> {
    Tool::ALL
        .iter()
        .map(|t| tool(t.name(), t.description(), t.properties(), t.required()))
        .collect()
}

/// Runs an allowed call of the memory tool `name` (its input as `prepare` left it) for the run
/// `run_id`. A broken rule is an error result; the run goes on.
pub async fn run(memories: &Memories, run_id: &str, name: &str, input: &Value) -> ToolOutput {
    let Some(memory_tool) = Tool::from_name(name) else {
        return ToolOutput::err(format!("unknown tool '{name}'"));
    };
    match memories.call(run_id, memory_tool, input).await {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
