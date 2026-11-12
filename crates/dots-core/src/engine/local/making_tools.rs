//! The local engine's tools for making dots (dots-make-dots design sections 5 and 7 to 10):
//! `make_dot`, `change_dot`, `delete_dot`, `give_job` and `list_made_dots` are built-in
//! functions of a maker's run, shown and checked as `MakeDots` with their policy input, as an
//! approval names them, and performed by the makers' service, which the Claude engine's `dots`
//! MCP server also uses. Their names, words and arguments are `making::tools`, so both engines
//! show a model the same.
//!
//! They do not go through `tools::prepare`: their arguments are objects and lists that it does
//! not know, and an argument a tool does not have is refused in a sentence, not dropped.

use serde_json::Value;

use super::client::ToolCallRequest;
use super::mcp_tools::arguments;
use super::tools::{self, ToolOutput};
use super::Session;
use crate::checkins::CheckIns;
use crate::engine::{EngineEvent, PermissionOutcome};
use crate::making::tools::{check_arguments, policy_input, Tool, POLICY_NAME};
use crate::model::Making;

/// The function definitions of the tools for making dots, for a maker with `limits`.
pub(super) fn schemas(limits: &Making) -> Vec<Value> {
    Tool::ALL
        .into_iter()
        .map(|tool| {
            tools::tool(
                tool.name(),
                &tool.description(limits),
                tool.properties(),
                tool.required(),
            )
        })
        .collect()
}

impl Session {
    /// The tool for making dots a model's call `name` is, when the run has them
    /// (`RunTools::making`). For every other run the name is no tool at all.
    pub(super) fn making_tool(&self, name: &str) -> Option<Tool> {
        self.ctx.tools.making().and(Tool::from_name(name))
    }

    /// Shows, checks and performs a model's call of the tool for making dots `tool`, in the
    /// order Dots' own MCP server does: arguments the tool cannot read are refused before the
    /// policy is asked, then the policy is asked about the tool by name with its arguments
    /// under `MakeDots`, then the makers' service performs it. `None` when the run must stop.
    pub(super) async fn making_output(
        &self,
        call: &ToolCallRequest,
        tool: Tool,
    ) -> Option<ToolOutput> {
        let parsed = arguments(&call.arguments);
        let shown = parsed.clone().unwrap_or_else(|_| call.arguments.clone());
        let checked = policy_input(tool, &shown);
        let event = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool: POLICY_NAME.to_string(),
            input: checked.clone(),
            parent: None,
        };
        if !self.emit(event).await {
            return None;
        }
        let read = parsed
            .map_err(|e| format!("{}: {e}", call.name))
            .and_then(|arguments| check_arguments(tool, &arguments));
        if let Err(e) = read {
            return Some(ToolOutput::err(e));
        }
        let decided = tokio::select! {
            r = self.gate.check(&self.ctx.run_id, POLICY_NAME, checked) => r,
            _ = self.ctx.cancel.cancelled() => return None,
        };
        Some(match decided {
            Ok(PermissionOutcome::Allow { input }) => self.perform_making(&input).await,
            Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
            Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
        })
    }

    /// Performs an allowed or approved call of a tool for making dots, as this run; `checked`
    /// is its policy input. What the service refuses is an error result with its sentence. A
    /// run that does not have the tools performs nothing: an approval is not a way around the
    /// switch.
    pub(super) async fn perform_making(&self, checked: &Value) -> ToolOutput {
        let makers = self
            .checkins
            .as_ref()
            .filter(|_| self.ctx.tools.making().is_some())
            .and_then(CheckIns::makers);
        let Some(makers) = makers else {
            return ToolOutput::err("the tools for making dots are not available in this run");
        };
        match crate::making::tools::call(&makers, &self.ctx.run_id, checked).await {
            Ok(text) => ToolOutput::ok(text),
            Err(text) => ToolOutput::err(text),
        }
    }
}
