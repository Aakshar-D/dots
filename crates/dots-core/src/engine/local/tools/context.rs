//! The `context` tool (policy name `Context`) of the local engine: it reads the run's context
//! collections, never the workspace (context library spec section 4). The reading itself is
//! `crate::context`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::context::{self, Collection, DESCRIPTION};

/// The function definition, offered to a run that has context collections.
pub fn schema() -> Value {
    tool(
        "context",
        DESCRIPTION,
        context::properties(),
        &["collection"],
    )
}

/// Runs an allowed `context` call (its input as `prepare` left it) against the run's
/// collections.
pub async fn run(collections: &[Collection], input: &Value) -> ToolOutput {
    let (collections, input) = (collections.to_vec(), input.clone());
    match tokio::task::spawn_blocking(move || context::call(&collections, &input)).await {
        Ok(Ok(text)) => ToolOutput::ok(text),
        Ok(Err(e)) => ToolOutput::err(e),
        Err(e) => ToolOutput::err(format!("reading the collection failed: {e}")),
    }
}
