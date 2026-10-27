use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::model::{ApplyState, Approval, Dot};
use crate::policy::McpTrust;
use crate::run_tools::RunTools;
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
        /// The part of `tokens_in` read from the prompt cache (teams spec section 9). An event
        /// recorded before it existed reads as 0.
        #[serde(default)]
        cache_read: u64,
        /// What the run so far would cost on the API (the `claude` CLI's `total_cost_usd`);
        /// `None` when the engine gives none (usage and limits spec section 5).
        #[serde(default)]
        cost_usd: Option<f64>,
    },
    /// The `claude` CLI's reading of the account's usage limits (usage and limits spec section
    /// 3): the status (`allowed`, `allowed_warning`, `rejected`) and each window it named.
    Limits {
        status: String,
        windows: Vec<crate::store::LimitReading>,
        /// When the window that limits now resets (the line's own `resetsAt`), RFC 3339 UTC.
        #[serde(default)]
        resets_at: Option<String>,
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
    /// Something the person should know about the run that is not the model's doing, such as
    /// a Library item left out of it ("skill pdf: folder not found") or an MCP server's
    /// problem ("salesforce: could not start: SF_TOKEN is not set").
    Notice {
        text: String,
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
            Self::Limits { .. } => "limits",
            Self::Raw { .. } => "raw",
            Self::Message { .. } => "message",
            Self::Finished { .. } => "finished",
            Self::Failed { .. } => "failed",
            Self::Notice { .. } => "notice",
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
    /// The run's MCP sessions, skills and plugin folders, opened by the runner before the
    /// engine starts and closed when the run ends (`RunTools`). The local engine calls the
    /// sessions itself; the Claude CLI reaches them through the gateway.
    pub tools: Arc<RunTools>,
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

    /// Checks a call of the MCP tool `tool` (`mcp__<server>__<tool>`) with what the run knows of
    /// its server (`Policy::resolve_mcp`). An approval it asks for stores `fingerprint`, the
    /// server's accepted one. A gate that does not check MCP calls denies them.
    async fn check_mcp(
        &self,
        _run_id: &str,
        tool: &str,
        _input: Value,
        _trust: McpTrust,
        _fingerprint: Option<String>,
    ) -> Result<PermissionOutcome> {
        Ok(PermissionOutcome::Deny {
            message: format!("{tool} cannot be checked, so it is not permitted."),
        })
    }

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
