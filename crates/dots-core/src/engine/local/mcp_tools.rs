//! The local engine's MCP tools (MCP/skills spec section 5). The tools of the run's open
//! sessions are offered as functions named `mcp__<server>__<tool>`, a name too long for a
//! function (or holding a character one cannot have) shortened by `model_name` and mapped back
//! for the run. A call is checked by `PermissionGate::check_mcp` with what the run knows of
//! the server (`RunServer::trust`), sent to the server, and its result reported as text.
//!
//! An approved call that the resume run performs goes through only while the server's accepted
//! fingerprint is the one stored with the approval (spec section 4).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::client::ToolCallRequest;
use super::tools::{self, cap_head, ToolOutput};
use super::Session;
use crate::engine::{EngineEvent, PermissionOutcome};
use crate::mcp::CALL_TIMEOUT;
use crate::model::Approval;
use crate::run_tools::RunServer;

/// The longest function name an OpenAI-compatible endpoint takes.
pub const MAX_NAME: usize = 64;
/// How much of a name that is too long is kept, before `_` and 8 hex characters.
const KEPT: usize = 55;

/// What the model is told about an approved call whose server changed its accepted list.
pub(super) use crate::approvals::TOOL_CHANGED as CHANGED;

/// The function name of the MCP tool `qualified` (`mcp__<server>__<tool>`): the name itself
/// when it fits `^[A-Za-z0-9_-]{1,64}$`; otherwise its first 55 characters (each other
/// character as `_`), `_` and the first 8 hex characters of the SHA-256 of the whole name.
pub fn model_name(qualified: &str) -> String {
    let fits = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if qualified.len() <= MAX_NAME && qualified.chars().all(fits) {
        return qualified.to_string();
    }
    let kept: String = qualified
        .chars()
        .map(|c| if fits(c) { c } else { '_' })
        .take(KEPT)
        .collect();
    let hash: String = Sha256::digest(qualified.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{kept}_{hash}")
}

/// A `tools/call` result as the model gets it: the `text` parts joined by new lines, any other
/// part as `[<type> omitted]`, capped at 30 KB. With no part, the `structuredContent` stands
/// in. `isError` makes it an error result.
pub fn format_result(result: &Value) -> ToolOutput {
    let parts: Vec<String> = result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .map(|part| match part["type"].as_str() {
                    Some("text") => part["text"].as_str().unwrap_or_default().to_string(),
                    Some(kind) => format!("[{kind} omitted]"),
                    None => "[content omitted]".to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let text = match result.get("structuredContent") {
        _ if !parts.is_empty() => parts.join("\n"),
        Some(structured) if !structured.is_null() => structured.to_string(),
        _ => "(no output)".to_string(),
    };
    if result["isError"] == true {
        ToolOutput::err(cap_head(text))
    } else {
        ToolOutput::ok(cap_head(text))
    }
}

/// The run's map from function names back to MCP tools. It only grows: a name the model saw
/// on an earlier turn still finds its tool.
#[derive(Default)]
pub(super) struct ToolNames(HashMap<String, String>);

impl ToolNames {
    /// The MCP tool a model's call names: a mapped name, or a full `mcp__` name as it is.
    fn qualified(&self, name: &str) -> Option<String> {
        self.0
            .get(name)
            .cloned()
            .or_else(|| name.starts_with("mcp__").then(|| name.to_string()))
    }
}

/// The arguments of an MCP call: a JSON object, sent as one or as a JSON string.
fn arguments(raw: &Value) -> Result<Value, String> {
    match raw {
        Value::Object(_) => Ok(raw.clone()),
        Value::Null => Ok(json!({})),
        Value::String(s) if s.trim().is_empty() => Ok(json!({})),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(object @ Value::Object(_)) => Ok(object),
            Ok(_) => Err("the arguments must be a JSON object".into()),
            Err(e) => Err(format!(
                "the arguments are not valid JSON ({e}); send a JSON object"
            )),
        },
        _ => Err("the arguments must be a JSON object".into()),
    }
}

/// The error for a call of `qualified` when the run has no such server open.
fn not_open(qualified: &str) -> String {
    match qualified
        .strip_prefix("mcp__")
        .and_then(|r| r.split_once("__"))
    {
        Some((server, _)) => {
            format!("{qualified}: no MCP server named '{server}' is open in this run")
        }
        None => format!("unknown tool '{qualified}'"),
    }
}

/// Sends a call the check allowed to its server. A cancelled call reads "cancelled"; the
/// caller sees the run's cancel.
async fn send(
    server: &RunServer,
    tool: &str,
    input: Value,
    cancel: &CancellationToken,
) -> ToolOutput {
    match server
        .session()
        .call_tool(tool, input, CALL_TIMEOUT, cancel)
        .await
    {
        Ok(result) => format_result(&result),
        Err(e) => ToolOutput::err(cap_head(e.to_string())),
    }
}

impl Session {
    /// The functions offered this turn: the built-in tools, `skill` when the run has skills,
    /// `context` when it has context collections,
    /// and the tools the run's servers list now. A list change a server announced is listed
    /// first, so a new tool is offered on the next turn.
    pub(super) async fn tool_schemas(&mut self) -> Vec<Value> {
        let mut out = match tools::schemas() {
            Value::Array(builtin) => builtin,
            _ => Vec::new(),
        };
        if !self.ctx.tools.skills().is_empty() {
            out.push(tools::skill::schema());
        }
        if !self.ctx.tools.contexts().is_empty() {
            out.push(tools::context::schema());
        }
        if self.checkins.is_some() {
            out.extend(tools::checkins::schemas());
        }
        // The built-in browser's tools (browser spec section 2.2); their names fit as they are,
        // so `ToolNames` finds them by their `mcp__` name.
        if self.ctx.tools.browser().is_some() {
            out.extend(crate::browser::function_schemas());
        }
        let mut given = HashSet::new();
        for server in self.ctx.tools.servers() {
            // Not settled in time: the list may be the old one; each call is checked anyway.
            let _ = server.settled().await;
            for tool in server.tools() {
                let qualified = format!("mcp__{}__{}", server.name(), tool.name);
                let name = model_name(&qualified);
                // A tool the server lists twice is offered once.
                if !given.insert(name.clone()) {
                    continue;
                }
                self.mcp_names.0.insert(name.clone(), qualified);
                let description = tool.description.or(tool.title).unwrap_or_default();
                out.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": tool.input_schema
                    }
                }));
            }
        }
        out
    }

    /// The MCP tool a model's call names, if it names one.
    pub(super) fn mcp_tool(&self, name: &str) -> Option<String> {
        self.mcp_names.qualified(name)
    }

    /// Shows, checks and performs a model's call of the MCP tool `qualified`. `None` when the
    /// run must stop (cancelled, or the runner stopped listening).
    pub(super) async fn mcp_output(
        &self,
        call: &ToolCallRequest,
        qualified: &str,
    ) -> Option<ToolOutput> {
        let parsed = arguments(&call.arguments);
        let shown = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool: qualified.to_string(),
            input: parsed.clone().unwrap_or_else(|_| call.arguments.clone()),
        };
        if !self.emit(shown).await {
            return None;
        }
        let input = match parsed {
            Ok(input) => input,
            Err(e) => return Some(ToolOutput::err(format!("{qualified}: {e}"))),
        };
        // A call of the built-in browser goes to its gate (browser spec section 3).
        if let (Some(browser), Some(tool)) = (
            self.ctx.tools.browser(),
            qualified.strip_prefix(crate::browser::PREFIX),
        ) {
            let result = tokio::select! {
                r = browser.call(self.gate.as_ref(), tool, input, CALL_TIMEOUT, &self.ctx.cancel) => r,
                _ = self.ctx.cancel.cancelled() => return None,
            };
            return Some(format_result(&result));
        }
        let Some((server, tool)) = self.ctx.tools.resolve(qualified) else {
            return Some(ToolOutput::err(not_open(qualified)));
        };
        // A list change announced with an earlier result holds the server before this check; one
        // not listed in time holds it for this call.
        let trust = tokio::select! {
            trust = server.settled_trust(tool) => trust,
            _ = self.ctx.cancel.cancelled() => return None,
        };
        let checked = tokio::select! {
            r = self.gate.check_mcp(
                &self.ctx.run_id,
                qualified,
                input,
                trust,
                server.accepted_fingerprint(),
            ) => r,
            _ = self.ctx.cancel.cancelled() => return None,
        };
        Some(match checked {
            Ok(PermissionOutcome::Allow { input }) => {
                send(server, tool, input, &self.ctx.cancel).await
            }
            Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
            Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
        })
    }

    /// Whether `approval` is an MCP call whose server's accepted list is no longer the one it
    /// was approved under: it must not be performed.
    pub(super) fn changed_since(&self, approval: &Approval) -> bool {
        self.ctx
            .tools
            .resolve(&approval.tool)
            .is_some_and(|(server, _)| server.accepted_fingerprint() != approval.mcp_fingerprint)
    }

    /// Performs an approved call of the MCP tool `qualified` exactly as it was approved.
    pub(super) async fn perform_mcp(&self, qualified: &str, input: &Value) -> ToolOutput {
        match self.ctx.tools.resolve(qualified) {
            Some((server, tool)) => send(server, tool, input.clone(), &self.ctx.cancel).await,
            None => ToolOutput::err(not_open(qualified)),
        }
    }
}
