//! The workflow tools of the local engine (`start_workflow`, `report_outcome`, `send_back`;
//! policy name `Workflow`): built in, as the check-in tools are (teams spec section 7). What
//! they do is `crate::workflows`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::run_tools::RunTools;
use crate::team_tools::Tool;
use crate::workflows::Workflows;

/// The function definition of the workflow tool `t`.
pub fn schema(t: Tool) -> Value {
    tool(t.name(), t.description(), t.properties(), t.required())
}

/// The function definitions of the workflow tools the run has (`RunTools::offers`).
pub fn schemas(tools: &RunTools) -> Vec<Value> {
    Tool::ALL
        .into_iter()
        .filter(|t| tools.offers(*t))
        .map(schema)
        .collect()
}

/// Runs an allowed call of the workflow tool `t` (its input as `prepare` left it) for the run
/// `run_id`, which has the tools `tools`. A tool the run does not have is not available.
pub async fn run(
    workflows: &Workflows,
    run_id: &str,
    tools: &RunTools,
    t: Tool,
    input: &Value,
) -> ToolOutput {
    let done = match (t, tools.step()) {
        _ if !tools.offers(t) => {
            return ToolOutput::err(format!("{} is not available in this run", t.name()))
        }
        (Tool::StartWorkflow, _) => workflows.call_start(run_id, tools.startable(), input).await,
        (Tool::ReportOutcome, Some(step)) => {
            workflows.call_report_outcome(run_id, step, input).await
        }
        (Tool::SendBack, Some(step)) => workflows.call_send_back(run_id, step, input).await,
        _ => return ToolOutput::err(format!("{} is not available in this run", t.name())),
    };
    match done {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
