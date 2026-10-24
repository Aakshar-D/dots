//! The workflow tools of the local engine (`start_workflow`; policy name `Workflow`): built in,
//! as the check-in tools are (teams spec section 7). What they do is `crate::workflows`, which
//! the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::model::StartableWorkflow;
use crate::team_tools::Tool;
use crate::workflows::Workflows;

/// The function definition of `start_workflow`, offered to a run that may start a workflow.
pub fn start_schema() -> Value {
    let t = Tool::StartWorkflow;
    tool(t.name(), t.description(), t.properties(), t.required())
}

/// Runs an allowed `start_workflow` call (its input as `prepare` left it) for the run `run_id`.
pub async fn run(
    workflows: &Workflows,
    run_id: &str,
    startable: &[StartableWorkflow],
    input: &Value,
) -> ToolOutput {
    match workflows.call_start(run_id, startable, input).await {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
