//! `propose_instructions` on the local engine (policy name `Instructions`): built in, as the
//! memory tools are (conversational chat spec section 4). What it does is
//! `crate::instructions::Proposals`, which the Claude engine's `dots` MCP server also uses.

use serde_json::Value;

use super::{tool, ToolOutput};
use crate::instructions::{Proposals, DESCRIPTION, REQUIRED, TOOL};

/// The function definition of `propose_instructions`, offered to a run of a chat the person
/// started.
pub fn schema() -> Value {
    tool(
        TOOL,
        DESCRIPTION,
        crate::instructions::properties(),
        &REQUIRED,
    )
}

/// Runs an allowed `propose_instructions` call (its input as `prepare` left it) for the run
/// `run_id`.
pub async fn run(proposals: &Proposals, run_id: &str, input: &Value) -> ToolOutput {
    let arg = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default();
    match proposals
        .propose(run_id, arg("instructions"), arg("reason"))
        .await
    {
        Ok(text) => ToolOutput::ok(text),
        Err(text) => ToolOutput::err(text),
    }
}
