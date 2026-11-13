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
use crate::making::tools::{checked_input, policy_input, Tool, POLICY_NAME};
use crate::model::Making;

/// What a call is answered when the run cannot perform the tools for making dots.
const NOT_AVAILABLE: &str = "the tools for making dots are not available in this run";

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
    /// The maker's limits, when the run has the tools for making dots (`RunTools::making`) and
    /// the engine has the makers' service that performs them. A run whose engine could not
    /// perform them is neither shown them nor asks the person about one.
    pub(super) fn making_limits(&self) -> Option<&Making> {
        self.ctx
            .tools
            .making()
            .filter(|_| self.checkins.as_ref().and_then(CheckIns::makers).is_some())
    }

    /// The tool for making dots a model's call `name` is, when the run has them
    /// (`making_limits`). For every other run the name is no tool at all.
    pub(super) fn making_tool(&self, name: &str) -> Option<Tool> {
        self.making_limits().and(Tool::from_name(name))
    }

    /// Shows, checks and performs a model's call of the tool for making dots `tool`, in the
    /// order Dots' own MCP server does: arguments the tool cannot read, and a name that finds no
    /// dot the maker made, are refused before the policy is asked, then the policy is asked
    /// about the tool by name with its arguments, and the made dot it names, under `MakeDots`
    /// (`checked_input`), then the makers' service performs it. `None` when the run must stop.
    pub(super) async fn making_output(
        &self,
        call: &ToolCallRequest,
        tool: Tool,
    ) -> Option<ToolOutput> {
        let parsed = arguments(&call.arguments);
        let shown = parsed.clone().unwrap_or_else(|_| call.arguments.clone());
        let event = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool: POLICY_NAME.to_string(),
            input: policy_input(tool, &shown),
            parent: None,
        };
        if !self.emit(event).await {
            return None;
        }
        let Some(makers) = self.checkins.as_ref().and_then(CheckIns::makers) else {
            return Some(ToolOutput::err(NOT_AVAILABLE));
        };
        let arguments = match parsed {
            Ok(arguments) => arguments,
            Err(e) => return Some(ToolOutput::err(format!("{}: {e}", call.name))),
        };
        let checked = match checked_input(&makers, &self.ctx.run_id, tool, &arguments).await {
            Ok(checked) => checked,
            Err(e) => return Some(ToolOutput::err(e)),
        };
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
            return ToolOutput::err(NOT_AVAILABLE);
        };
        match crate::making::tools::call(&makers, &self.ctx.run_id, checked).await {
            Ok(text) => ToolOutput::ok(text),
            Err(text) => ToolOutput::err(text),
        }
    }
}
