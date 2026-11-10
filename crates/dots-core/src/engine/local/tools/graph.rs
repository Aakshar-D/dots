//! The `graph` tool (policy name `Graph`) of the local engine: it asks the code graph of the
//! dot's project, never the workspace (project graph spec section 8). The answering itself is
//! `crate::graph`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{cap_head, tool, ToolOutput};
use crate::graph::query::{properties, DESCRIPTION};
use crate::run_tools::RunGraph;

/// The function definition, offered to a run whose dot uses the project graph.
pub fn schema() -> Value {
    tool(crate::graph::TOOL, DESCRIPTION, properties(), &["action"])
}

/// Runs an allowed `graph` call (its input as `prepare` left it) against the run's graph.
pub async fn run(graph: Option<&RunGraph>, input: &Value) -> ToolOutput {
    let Some(graph) = graph else {
        return ToolOutput::err("graph is not available in this run");
    };
    match graph.graphs.call(&graph.project, &graph.scope, input).await {
        Ok(text) => ToolOutput::ok(cap_head(text)),
        Err(e) => ToolOutput::err(cap_head(e)),
    }
}
