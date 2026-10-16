use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::library::LibraryItem;
use crate::model::{ApplyState, Approval, Dot};
use crate::Result;

pub mod claude;
pub mod local;
pub mod scripted;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineEvent {
    SessionStarted {
        session_id: String,
    },
    AssistantText {
        text: String,
    },
    ToolCall {
        id: String,
        tool: String,
        input: Value,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    Usage {
        tokens_in: u64,
        tokens_out: u64,
    },
    Raw {
        line: String,
    },
    /// One chat message of a local-engine conversation (system, user, assistant or tool), in
    /// OpenAI chat format. Persisted so a resume or follow-up can rebuild the conversation.
    Message {
        message: Value,
    },
    Finished {
        summary: String,
    },
    Failed {
        error: String,
    },
}

impl EngineEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SessionStarted { .. } => "session_started",
            Self::AssistantText { .. } => "assistant_text",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolResult { .. } => "tool_result",
            Self::Usage { .. } => "usage",
            Self::Raw { .. } => "raw",
            Self::Message { .. } => "message",
            Self::Finished { .. } => "finished",
            Self::Failed { .. } => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunContext {
    pub run_id: String,
    pub dot: Dot,
    pub workspace: PathBuf,
    pub prompt: String,
    pub session_id: Option<String>,
    pub mcp_url: String,
    pub mcp_secret: String,
    pub cancel: CancellationToken,
    /// Approved calls of the parked parent run that this run performs before anything else, in
    /// the order they were asked. Only a `resume` run on an engine that
    /// `applies_approved_calls` gets any.
    pub approved_calls: Vec<Approval>,
    /// The Library's MCP servers the dot uses, its plugins' included, as stored: `${VAR}` not
    /// expanded (`Store::used_mcp_servers`).
    pub mcp_servers: Vec<LibraryItem>,
}

#[async_trait]
pub trait Engine: Send + Sync {
    /// Starts a run. Events arrive on the returned channel, which closes when the engine is
    /// done. The engine must stop promptly once `ctx.cancel` is cancelled.
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>>;

    /// Why this engine cannot run anything now. `Some` fails a run before its workspace is
    /// prepared.
    fn unavailable(&self) -> Option<String> {
        None
    }

    /// Whether this engine performs an approved call itself when the approval is decided after
    /// the engine stopped waiting (`RunContext::approved_calls`). When false, such a decision
    /// becomes a one-shot grant and the model is asked to issue the call again.
    fn applies_approved_calls(&self) -> bool {
        false
    }
}

/// Stands in for an engine that cannot run (no `claude` CLI, or no local server for its MCP
/// endpoint): every run fails with the reason.
pub struct UnavailableEngine(pub String);

#[async_trait]
impl Engine for UnavailableEngine {
    async fn start(&self, _ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>> {
        Err(crate::Error::Other(self.0.clone()))
    }

    fn unavailable(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionOutcome {
    Allow { input: Value },
    Deny { message: String },
}

#[async_trait]
pub trait PermissionGate: Send + Sync {
    async fn check(&self, run_id: &str, tool: &str, input: Value) -> Result<PermissionOutcome>;

    /// Claims an approved call that waits for Dots to perform it. False means the call is not
    /// the caller's to perform, and it must not be run. A gate without approvals claims nothing.
    async fn claim_apply(&self, _approval_id: &str) -> Result<bool> {
        Ok(false)
    }

    /// Records how a claimed call ended: `ApplyState::Applied` or `ApplyState::Failed`.
    async fn settle_apply(&self, _approval_id: &str, _state: ApplyState) -> Result<()> {
        Ok(())
    }

    /// Gives up a call that waits to be performed. True when it was waiting.
    async fn skip_apply(&self, _approval_id: &str) -> Result<bool> {
        Ok(false)
    }
}
