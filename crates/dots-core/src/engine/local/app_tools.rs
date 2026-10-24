//! The local engine's app tools (app library spec section 6): each tool of the run's apps is a
//! built-in function named `<app>__<tool>`, shown and checked as `App` with its policy input,
//! as an approval names it, and called as the run's dot.

use serde_json::Value;

use super::client::ToolCallRequest;
use super::mcp_tools::arguments;
use super::tools::ToolOutput;
use super::Session;
use crate::apps::APP_POLICY;
use crate::engine::{EngineEvent, PermissionOutcome};

impl Session {
    /// Whether the model's call `name` is one of the run's app tools.
    pub(super) fn app_tool(&self, name: &str) -> bool {
        self.ctx
            .tools
            .apps()
            .is_some_and(|apps| apps.resolve(name).is_some())
    }

    /// Shows, checks and performs a model's call of an app tool. `None` when the run must stop.
    pub(super) async fn app_output(&self, call: &ToolCallRequest) -> Option<ToolOutput> {
        let parsed = arguments(&call.arguments);
        let shown = parsed.clone().unwrap_or_else(|_| call.arguments.clone());
        let checked = self
            .ctx
            .tools
            .apps()
            .and_then(|apps| apps.policy_input(&call.name, &shown));
        let Some(checked) = checked else {
            return Some(ToolOutput::err(format!("unknown tool '{}'", call.name)));
        };
        let event = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool: APP_POLICY.to_string(),
            input: checked.clone(),
        };
        if !self.emit(event).await {
            return None;
        }
        if let Err(e) = parsed {
            return Some(ToolOutput::err(format!("{}: {e}", call.name)));
        }
        let decided = tokio::select! {
            r = self.gate.check(&self.ctx.run_id, APP_POLICY, checked) => r,
            _ = self.ctx.cancel.cancelled() => return None,
        };
        Some(match decided {
            Ok(PermissionOutcome::Allow { input }) => self.perform_app(&input).await,
            Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
            Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
        })
    }

    /// Performs an allowed or approved app call; `checked` is its policy input.
    pub(super) async fn perform_app(&self, checked: &Value) -> ToolOutput {
        let Some(apps) = self.ctx.tools.apps() else {
            return ToolOutput::err("this run has no apps");
        };
        match apps.call(checked).await {
            Ok(text) => ToolOutput::ok(text),
            Err(text) => ToolOutput::err(text),
        }
    }
}
