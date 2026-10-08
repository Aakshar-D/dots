use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::model::Dot;
use crate::Result;

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
}

#[async_trait]
pub trait Engine: Send + Sync {
    /// Starts a run. Events arrive on the returned channel, which closes when the engine is
    /// done. The engine must stop promptly once `ctx.cancel` is cancelled.
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum PermissionOutcome {
    Allow { input: Value },
    Deny { message: String },
}

#[async_trait]
pub trait PermissionGate: Send + Sync {
    async fn check(&self, run_id: &str, tool: &str, input: Value) -> Result<PermissionOutcome>;
}
