use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::policy::{Policy, Preset};
use crate::{Error, Result};

str_enum!(EngineKind { Claude => "claude", Local => "local" });
str_enum!(WorkspaceMode { Worktree => "worktree", Folder => "folder" });
str_enum!(TriggerKind {
    Schedule => "schedule",
    Webhook => "webhook",
    Manual => "manual",
    Chat => "chat",
    Resume => "resume",
});
str_enum!(RunStatus {
    Queued => "queued",
    Running => "running",
    AwaitingApproval => "awaiting_approval",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});
str_enum!(ApprovalStatus {
    Pending => "pending",
    Approved => "approved",
    Denied => "denied",
    Expired => "expired",
});

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

pub const MAX_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;
pub const MAX_APPROVAL_WAIT_SECS: u64 = 280;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotSpec {
    pub name: String,
    pub instructions: String,
    pub engine: EngineKind,
    pub model: String,
    pub endpoint_url: Option<String>,
    pub workdir: String,
    pub workspace_mode: WorkspaceMode,
    pub schedule: Option<String>,
    pub policy: Policy,
    pub mcp_servers: Option<Value>,
    pub use_user_settings: bool,
    pub max_turns: u32,
    pub timeout_secs: u64,
    pub approval_wait_secs: u64,
    pub enabled: bool,
}

impl DotSpec {
    pub fn new(name: &str, instructions: &str, workdir: &str) -> Self {
        Self {
            name: name.to_string(),
            instructions: instructions.to_string(),
            engine: EngineKind::Claude,
            model: "sonnet".to_string(),
            endpoint_url: None,
            workdir: workdir.to_string(),
            workspace_mode: WorkspaceMode::Worktree,
            schedule: None,
            policy: Policy::preset(Preset::Sandboxed),
            mcp_servers: None,
            use_user_settings: false,
            max_turns: 40,
            timeout_secs: 1800,
            approval_wait_secs: 240,
            enabled: true,
        }
    }

    pub fn validate(&self) -> Result<()> {
        fn invalid(m: &str) -> Result<()> {
            Err(Error::Invalid(m.to_string()))
        }
        let name_ok = !self.name.is_empty()
            && self.name.len() <= 64
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !name_ok {
            return invalid("name must be 1-64 characters of A-Z, a-z, 0-9, '-' or '_'");
        }
        if self.instructions.trim().is_empty() {
            return invalid("instructions must not be empty");
        }
        if self.model.trim().is_empty() {
            return invalid("model must not be empty");
        }
        if self.workdir.trim().is_empty() {
            return invalid("workdir must not be empty");
        }
        if self.engine == EngineKind::Local
            && self
                .endpoint_url
                .as_deref()
                .is_none_or(|u| u.trim().is_empty())
        {
            return invalid("the local engine requires endpoint_url");
        }
        if let Some(expr) = &self.schedule {
            crate::scheduler::parse_cron(expr)?;
        }
        if self.max_turns == 0 {
            return invalid("max_turns must be at least 1");
        }
        if self.timeout_secs == 0 {
            return invalid("timeout_secs must be at least 1");
        }
        if self.timeout_secs > MAX_TIMEOUT_SECS {
            return invalid("timeout_secs must be at most 604800 (7 days)");
        }
        // The CLI aborts a pending permission-prompt call at about 300 s.
        if self.approval_wait_secs > MAX_APPROVAL_WAIT_SECS {
            return invalid("approval_wait_secs must be at most 280");
        }
        self.policy.validate()?;
        if let Some(servers) = &self.mcp_servers {
            let Some(obj) = servers.as_object() else {
                return invalid("mcp_servers must be a JSON object of name -> server config");
            };
            if obj.contains_key("dots") {
                return invalid("mcp server name 'dots' is reserved");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dot {
    pub id: String,
    pub webhook_token: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(flatten)]
    pub spec: DotSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub dot_id: String,
    pub root_run_id: String,
    pub parent_run_id: Option<String>,
    pub trigger: TriggerKind,
    pub payload: Option<Value>,
    pub status: RunStatus,
    pub session_id: Option<String>,
    pub workspace_path: Option<String>,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

/// Input for `Store::create_run`. `root_run_id: None` makes the run its own root.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRun {
    pub dot_id: String,
    pub trigger: TriggerKind,
    pub payload: Option<Value>,
    pub parent_run_id: Option<String>,
    pub root_run_id: Option<String>,
    pub session_id: Option<String>,
    pub workspace_path: Option<String>,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
}

impl NewRun {
    pub fn new(dot_id: &str, trigger: TriggerKind) -> Self {
        Self {
            dot_id: dot_id.to_string(),
            trigger,
            payload: None,
            parent_run_id: None,
            root_run_id: None,
            session_id: None,
            workspace_path: None,
            branch: None,
            base_commit: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEventRecord {
    pub run_id: String,
    pub seq: i64,
    pub ts: String,
    pub kind: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub run_id: String,
    pub tool: String,
    pub input: Value,
    pub input_hash: String,
    pub status: ApprovalStatus,
    pub note: Option<String>,
    pub parked: bool,
    pub resolved: bool,
    pub created_at: String,
    pub decided_at: Option<String>,
}
