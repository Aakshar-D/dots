use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::Uses;
use crate::policy::{Policy, Preset};
use crate::schedule::{next_slot, Schedule, Until};
use crate::vault::Grants;
use crate::{Error, Result};

str_enum!(EngineKind { Claude => "claude", Local => "local" });
str_enum!(WorkspaceMode { Worktree => "worktree", Folder => "folder" });
str_enum!(TriggerKind {
    Schedule => "schedule",
    Webhook => "webhook",
    Manual => "manual",
    Chat => "chat",
    Resume => "resume",
    FollowUp => "follow_up",
    App => "app",
    Workflow => "workflow",
    Watcher => "watcher",
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
// A call that Dots performs itself after a late approval: `queued -> applying -> applied |
// failed | unknown`, or `queued -> skipped`. Nothing leaves the last four.
str_enum!(ApplyState {
    Queued => "queued",
    Applying => "applying",
    Applied => "applied",
    Failed => "failed",
    Unknown => "unknown",
    Skipped => "skipped",
});

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Local-engine endpoints are plain `http://` URLs (a local server); https is not supported.
pub fn check_endpoint_url(url: &str) -> Result<()> {
    let url = url.trim();
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        Error::Invalid(format!(
            "endpoint_url must start with http:// (https is not supported): {url}"
        ))
    })?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(Error::Invalid(format!("endpoint_url has no host: {url}")));
    }
    Ok(())
}

fn yes() -> bool {
    true
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
    pub schedule: Option<Schedule>,
    pub policy: Policy,
    /// The Library items the dot uses (MCP/skills spec section 2).
    #[serde(default)]
    pub uses: Uses,
    /// The variables each of its consumers may resolve (env vault spec §7).
    #[serde(default)]
    pub variables: Grants,
    /// The dot has a browser: a visible Microsoft Edge with its own profile, driven through
    /// Playwright MCP behind Dots' gate (browser spec section 2.4). A spec written before it
    /// existed reads as off.
    #[serde(default)]
    pub browser: bool,
    /// The first prompt starts with the global instructions of Settings (context library spec
    /// section 2).
    #[serde(default = "yes")]
    pub use_global_instructions: bool,
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
            uses: Uses::default(),
            variables: Grants::default(),
            browser: false,
            use_global_instructions: true,
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
        if self.engine == EngineKind::Local {
            match self.endpoint_url.as_deref() {
                Some(url) if !url.trim().is_empty() => check_endpoint_url(url)?,
                _ => return invalid("the local engine requires endpoint_url"),
            }
        }
        if let Some(schedule) = &self.schedule {
            schedule.validate()?;
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
        // Whether the Library has each item is checked by the store when the dot is saved.
        self.uses.validate()?;
        self.variables.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dot {
    pub id: String,
    /// Whether the dot has a webhook token. The token is shown once, when it is made; Dots
    /// keeps only its hash (env vault spec §11).
    pub has_webhook_token: bool,
    pub created_at: String,
    pub updated_at: String,
    /// When the person last looked at the dot's conversation, RFC 3339 UTC; `None` before the
    /// first time (needs-you spec section 2).
    #[serde(default)]
    pub seen_at: Option<String>,
    /// The org the dot belongs to, fixed when it is created; `None` is Personal (env vault spec
    /// §5.2). The org may have been removed since, and the dot keeps its id (§6.4).
    #[serde(default)]
    pub org_id: Option<String>,
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
    /// Set when the user marked the run reviewed or discarded its worktree.
    pub reviewed_at: Option<String>,
    /// The failed scheduled run that this run retries.
    pub retry_of: Option<String>,
    /// What the run would cost on the API, as the `claude` CLI reported it; `None` for a local
    /// run and for a Claude run whose CLI gave none (usage and limits spec section 5).
    pub cost_usd: Option<f64>,
    /// The person released this run from the hold with **Run anyway** (spec section 4).
    pub hold_released: bool,
    /// The workflow step this run works for (teams spec section 4); a resume or a chat that
    /// continues the run's thread keeps it.
    pub workflow_step_run_id: Option<String>,
    /// The part of `tokens_in` read from the prompt cache (teams spec section 9).
    pub tokens_cache_read: i64,
}

/// Input for `Store::create_run`. `root_run_id: None` makes the run its own root.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRun {
    /// The run's id when the caller made it beforehand: a watcher stages files under it, and an
    /// app's start returned it already (app library spec section 7). `None`: the store makes one.
    pub id: Option<String>,
    pub dot_id: String,
    pub trigger: TriggerKind,
    pub payload: Option<Value>,
    pub parent_run_id: Option<String>,
    pub root_run_id: Option<String>,
    pub session_id: Option<String>,
    pub workspace_path: Option<String>,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    pub retry_of: Option<String>,
    /// `None` keeps the parent's (`parent_run_id`), if it has one.
    pub workflow_step_run_id: Option<String>,
}

impl NewRun {
    pub fn new(dot_id: &str, trigger: TriggerKind) -> Self {
        Self {
            id: None,
            dot_id: dot_id.to_string(),
            trigger,
            payload: None,
            parent_run_id: None,
            root_run_id: None,
            session_id: None,
            workspace_path: None,
            branch: None,
            base_commit: None,
            retry_of: None,
            workflow_step_run_id: None,
        }
    }
}

str_enum!(ScheduleStatus {
    Active => "active",
    Completed => "completed",
    Expired => "expired",
});

/// A retry that waits (Scheduler v2 spec §6). Times are RFC 3339 UTC (`util::timestamp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRetry {
    /// The slot whose occurrence is retried.
    pub slot_at: String,
    /// The attempt the retry is, 2 to 6.
    pub attempt: u32,
    /// When it starts.
    pub at: String,
}

/// Where a dot's schedule stands: its `schedule_state` row (Scheduler v2 spec §4). Times are
/// RFC 3339 UTC (`util::timestamp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleState {
    pub dot_id: String,
    /// The schedule's JSON when the row was made or reset (`Schedule::rule_key`).
    pub rule_key: String,
    pub status: ScheduleStatus,
    /// The newest slot handled: fired, coalesced or skipped.
    pub last_slot_at: String,
    /// Slots that started a run or joined a queued one.
    pub fired_count: u32,
    /// The run whose outcome decides the current occurrence.
    pub watch_run_id: Option<String>,
    pub retry: Option<PendingRetry>,
    /// The last failed run of an occurrence that failed in the end ("Needs attention").
    pub attention_run_id: Option<String>,
    pub updated_at: String,
}

/// Input for `Store::update_schedule_state`: the fields to write, in one transaction. `None`
/// leaves a field as it is; `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScheduleChange {
    pub status: Option<ScheduleStatus>,
    pub watch_run_id: Option<Option<String>>,
    pub retry: Option<Option<PendingRetry>>,
    pub attention_run_id: Option<Option<String>>,
    /// Also marks this run reviewed, in the same transaction, when the row is written: the
    /// scheduler judged the failure of a scheduled run that made no tool call into a retry or
    /// into "Needs attention", so it is not a run to review as well.
    pub reviewed_run: Option<String>,
    /// Writes only while the row watches this run: the change follows from judging it.
    pub if_watching: Option<String>,
    /// Writes only while `last_slot_at` is this: a reset of the row meanwhile moved it.
    pub if_last_slot_at: Option<String>,
}

/// What `Store::record_slot` and `Store::start_retry` did with a slot or a retry.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotStart {
    /// The slot was skipped: no run.
    Skipped,
    /// A new run was queued. It is announced and the dispatcher woken by the caller.
    Enqueued(Run),
    /// The dot's queued run serves it.
    Coalesced(Run),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEventRecord {
    pub run_id: String,
    pub seq: i64,
    pub ts: String,
    pub kind: String,
    pub data: Value,
}

/// The installed `claude` CLI as seen at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeStatus {
    pub program: Option<String>,
    pub version: Option<String>,
    pub tested_version: String,
    /// A version was detected and it differs from `tested_version`.
    pub mismatch: bool,
}

/// What the app shows in its banners and Settings (spec §12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub port: u16,
    /// Set when the local server could not start; webhooks are off and Claude runs fail.
    pub server_error: Option<String>,
    pub claude: ClaudeStatus,
    pub schedules_paused: bool,
    /// Port, concurrency or `claude` path changed since start.
    pub restart_required: bool,
    pub data_dir: String,
}

str_enum!(FileStatus {
    Added => "added",
    Modified => "modified",
    Deleted => "deleted",
    Renamed => "renamed",
});

/// One changed file of a worktree, as a unified diff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub status: FileStatus,
    /// Empty for binary files and for files past the size cap.
    pub patch: String,
    pub binary: bool,
}

/// A worktree's changes against the run's base commit (spec §12, review actions).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorktreeDiff {
    pub files: Vec<FileDiff>,
    /// Some patches were left out to stay under `workspace::MAX_DIFF_BYTES`.
    pub truncated: bool,
    /// The worktree folder is still on disk; false once it was removed (it had no changes, or
    /// it was discarded), and `files` is then empty.
    pub exists: bool,
}

/// The run that left a schedule needing attention, with its error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionRun {
    pub run_id: String,
    pub error: Option<String>,
}

/// Where a dot's schedule stands, as the Dots screen shows it (Scheduler v2 spec §8): its
/// `schedule_state` row, with the times in local time and what comes next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleView {
    pub status: ScheduleStatus,
    pub fired_count: u32,
    /// The first slot after now, RFC 3339; `None` when nothing more fires: the dot is
    /// disabled, the schedule is not active, or it has no slot or no count left.
    pub next_fire: Option<String>,
    /// When the waiting retry starts, RFC 3339, and the attempt it is; `None` unless a retry
    /// waits on an enabled dot.
    pub retry_at: Option<String>,
    pub retry_attempt: Option<u32>,
    pub attention: Option<AttentionRun>,
}

impl ScheduleView {
    /// The view of `state`, the row of a dot with `schedule`, at `now`, with times in `tz`.
    /// `attention` is the run the row's `attention_run_id` names.
    pub fn new<Tz>(
        state: &ScheduleState,
        schedule: &Schedule,
        enabled: bool,
        attention: Option<AttentionRun>,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> ScheduleView
    where
        Tz: TimeZone + 'static,
        Tz::Offset: std::fmt::Display,
    {
        let parse = |t: &str| DateTime::parse_from_rfc3339(t).ok();
        let counted = match schedule.until {
            Some(Until::Count(n)) => state.fired_count >= n,
            _ => false,
        };
        let live = enabled && state.status == ScheduleStatus::Active;
        // A slot the pass has not handled yet is past: the next fire time is after now.
        let after =
            parse(&state.last_slot_at).map_or(now, |last| now.max(last.with_timezone(&Utc)));
        let next_fire = (live && !counted)
            .then(|| next_slot(schedule, &after, tz).ok().flatten())
            .flatten()
            .map(|t| t.to_rfc3339());
        let retry = state.retry.as_ref().filter(|_| live);
        ScheduleView {
            status: state.status,
            fired_count: state.fired_count,
            next_fire,
            retry_at: retry.map(|r| {
                parse(&r.at).map_or_else(|| r.at.clone(), |at| at.with_timezone(tz).to_rfc3339())
            }),
            retry_attempt: retry.map(|r| r.attempt),
            attention,
        }
    }
}

/// One row of the Dots screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotOverview {
    pub dot: Dot,
    /// The newest queued, running or awaiting run.
    pub active_run: Option<Run>,
    pub last_run: Option<Run>,
    /// `None` without a schedule.
    pub schedule_state: Option<ScheduleView>,
    /// Its open questions and pending approvals (needs-you spec section 2).
    pub needs_you: u32,
    /// `needs_you`, plus its runs that ended and its notes posted after `dot.seen_at`.
    pub unread: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingApproval {
    #[serde(flatten)]
    pub approval: Approval,
    pub dot_id: String,
    pub dot_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRun {
    #[serde(flatten)]
    pub run: Run,
    pub dot_name: String,
    /// The run's worktree folder is still on disk, so there is something to discard; false
    /// for a folder-mode run and for a worktree that was removed.
    pub worktree_exists: bool,
}

/// A schedule whose occurrence failed in the end (Scheduler v2 spec §6, "Needs attention").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedsAttention {
    pub dot_id: String,
    pub dot_name: String,
    /// The last failed run of the occurrence.
    pub run_id: String,
    pub error: Option<String>,
    /// When that run ended, RFC 3339 UTC.
    pub failed_at: String,
}

/// A dot named in a list: its id and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotRef {
    pub id: String,
    pub name: String,
}

/// An MCP server whose tool list changed since it was accepted, or was never accepted: its
/// tools ask until the person reviews and accepts the list (MCP/skills spec section 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsChanged {
    pub server_id: String,
    pub name: String,
    /// The scope whose list changed: `personal` or an org id (env vault spec §8). Accepting
    /// it accepts that scope's list.
    pub scope: String,
    /// The dots of that scope that use it (through its plugin, for a plugin's server).
    pub dots: Vec<DotRef>,
}

/// Pending approvals, finished runs awaiting review, schedules that need attention and MCP
/// servers whose tools changed (spec §12 Inbox; Scheduler v2 spec §8; MCP/skills spec §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inbox {
    pub approvals: Vec<PendingApproval>,
    pub review_runs: Vec<ReviewRun>,
    /// Newest failure first.
    pub attention: Vec<NeedsAttention>,
    /// By server name.
    pub tools_changed: Vec<ToolsChanged>,
    /// Watchers Dots paused (too many runs, or an error), newest first (watchers spec §7).
    pub watchers: Vec<PausedWatcher>,
    /// Background runs held for Claude usage, oldest first (usage and limits spec section 5).
    pub held: Vec<HeldRun>,
    /// Instructions dots proposed that wait for the person, newest first (conversational chat
    /// spec section 4).
    pub instructions: Vec<crate::instructions::PendingInstructions>,
    /// Apps dots sent for review that wait for the person, newest first (app library spec
    /// section 10: a waiting card counts like an approval).
    pub app_reviews: Vec<PendingAppReview>,
}

/// An app proposal that waits for the person, as the Inbox lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAppReview {
    pub dot_id: String,
    pub dot_name: String,
    pub proposal: crate::apps::propose::AppProposal,
}

/// A queued run held for Claude usage, as the Inbox lists it (usage and limits spec section 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldRun {
    pub run_id: String,
    pub dot_id: String,
    pub dot_name: String,
    pub trigger: TriggerKind,
    pub queued_at: String,
    /// When the window that holds it resets; `None` while held runs start one at a time.
    pub starts_after: Option<String>,
    pub reason: HoldReason,
}

/// Why a queued run is held (members spec section 6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HoldReason {
    /// Claude usage holds it (usage and limits spec section 5).
    Usage,
    /// This PC's role in the dot's account no longer lets it run dots, or this PC is no longer
    /// on the account's member list. `text` says which. The run stays queued and goes when
    /// the access returns.
    Role { account: String, text: String },
}

str_enum!(UsagePeriod { FiveHours => "five_hours", Week => "week", Month => "month" });

impl UsagePeriod {
    /// How far back the period reaches from now.
    pub fn reach(self) -> chrono::TimeDelta {
        match self {
            Self::FiveHours => chrono::TimeDelta::hours(5),
            Self::Week => chrono::TimeDelta::days(7),
            Self::Month => chrono::TimeDelta::days(30),
        }
    }
}

/// The Usage page (usage and limits spec section 5): the dots' runs started since `since`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSummary {
    pub period: UsagePeriod,
    pub since: String,
    pub rows: Vec<UsageRow>,
}

/// One dot's runs in the period, by tokens (in + out), heaviest dot first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub dot_id: String,
    pub dot_name: String,
    pub engine: EngineKind,
    pub runs: u32,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// The ended runs' time, start to end.
    pub run_secs: i64,
    /// `None` when no run of the dot reported a cost (a local model's).
    pub cost_usd: Option<f64>,
    /// The dot's heaviest runs in the period, by tokens, at most `limits::HEAVIEST`.
    pub heaviest: Vec<UsageRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRun {
    pub run_id: String,
    pub trigger: TriggerKind,
    pub started_at: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// `None` while the run has not ended.
    pub run_secs: Option<i64>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSaved {
    pub settings: crate::settings::Settings,
    pub restart_required: bool,
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
    /// Dots performs this call itself when it is approved after the engine stopped waiting.
    pub self_apply: bool,
    /// Where a call that Dots performs stands; `None` for every other approval.
    pub apply_state: Option<ApplyState>,
    /// When the call ended `applied` or `failed`.
    pub applied_at: Option<String>,
    /// For an MCP tool (`mcp__<server>__<tool>`): the server's accepted fingerprint when the
    /// call asked, `None` when it had no accepted list. A decided call goes through later only
    /// while the server's accepted fingerprint is still this one (MCP/skills spec section 4).
    pub mcp_fingerprint: Option<String>,
}

str_enum!(MessageKind { Note => "note", Question => "question" });
str_enum!(FollowUpStatus {
    Pending => "pending",
    Fired => "fired",
    Cancelled => "cancelled",
    Skipped => "skipped",
});

/// What `reply` answers about a question that was answered already (check-ins spec section 5).
pub const QUESTION_ANSWERED: &str = "this question has already been answered";
/// What `reply` answers about a question the person dismissed.
pub const QUESTION_DISMISSED: &str = "this question was dismissed";

/// A dot's check-in (check-ins spec sections 5 and 7): a note that needs no answer, or a
/// question the person answers or dismisses. Times are RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub dot_id: String,
    /// The dot's name, for the Inbox and the notification.
    pub dot_name: String,
    /// The run that posted it; `None` for a note of Dots' own (a skipped follow-up).
    pub run_id: Option<String>,
    pub kind: MessageKind,
    pub text: String,
    /// The answers a question offers as buttons.
    pub choices: Option<Vec<String>>,
    pub created_at: String,
    pub read_at: Option<String>,
    pub answered_at: Option<String>,
    /// The run the answer started.
    pub answer_run_id: Option<String>,
    pub dismissed_at: Option<String>,
}

impl Message {
    /// It waits for the person: a question neither answered nor dismissed, or a note not read
    /// yet. The Inbox and its count show these.
    pub fn waits(&self) -> bool {
        match self.kind {
            MessageKind::Question => self.answered_at.is_none() && self.dismissed_at.is_none(),
            MessageKind::Note => self.read_at.is_none(),
        }
    }

    /// Why the message cannot be answered: `None` for an open question.
    pub fn refusal(&self) -> Option<Error> {
        match self.kind {
            MessageKind::Note => Some(Error::Invalid("a note cannot be answered".into())),
            MessageKind::Question if self.dismissed_at.is_some() => {
                Some(Error::Conflict(QUESTION_DISMISSED.into()))
            }
            MessageKind::Question if self.answered_at.is_some() => {
                Some(Error::Conflict(QUESTION_ANSWERED.into()))
            }
            MessageKind::Question => None,
        }
    }
}

/// A run a dot scheduled for itself (check-ins spec sections 6 and 7). Times are RFC 3339 UTC
/// (`util::timestamp`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUp {
    pub id: String,
    pub dot_id: String,
    /// The run that called `follow_up`; the follow-up continues its thread.
    pub from_run_id: String,
    pub due_at: String,
    /// What the dot told itself to do then.
    pub note: String,
    pub status: FollowUpStatus,
    pub fired_run_id: Option<String>,
    /// Starts that failed.
    pub attempts: u32,
    /// When a start that failed is tried again.
    pub retry_at: Option<String>,
    pub created_at: String,
}

str_enum!(MemorySource { Dot => "dot", Person => "person" });

/// One thing a dot remembers across runs (dot memory spec section 2): one fact, 1 to 500
/// characters. Times are RFC 3339 UTC (`util::now`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub dot_id: String,
    pub text: String,
    /// Shown first, in the prompt and the Memory tab.
    pub pinned: bool,
    /// Who wrote it: a run of the dot (`remember`) or the person (the Memory tab).
    pub source: MemorySource,
    /// The run that saved it; `None` for the person's own.
    pub run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The longest team, project or workflow name, in characters after trimming.
pub const MAX_TEAM_NAME: usize = 64;
/// The longest team description, in characters after trimming.
pub const MAX_DESCRIPTION: usize = 2_000;
/// The longest member role, in characters after trimming.
pub const MAX_ROLE: usize = 80;

fn check_name(what: &str, name: &str) -> Result<()> {
    let n = name.trim().chars().count();
    if n == 0 || n > MAX_TEAM_NAME {
        return Err(Error::Invalid(format!(
            "a {what} name must be 1 to {MAX_TEAM_NAME} characters"
        )));
    }
    Ok(())
}

/// A git name Dots passes on a command line (a remote or a branch): no leading `-`, so it can
/// never be read as an option, and only `A-Z a-z 0-9 . _ / -`.
fn check_git_name(what: &str, name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if !ok {
        return Err(Error::Invalid(format!(
            "{what} must be letters, digits, '.', '_', '/' or '-', not starting with '-'"
        )));
    }
    Ok(())
}

/// A member of a team as it is saved: a dot and its role (teams spec section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberSpec {
    pub dot_id: String,
    #[serde(default)]
    pub role: String,
}

/// A team as it is saved: its name, description and members, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub members: Vec<MemberSpec>,
    /// The account the team belongs to; `None` is Personal. Fixed once the team is created.
    #[serde(default)]
    pub org_id: Option<String>,
}

impl TeamSpec {
    pub fn validate(&self) -> Result<()> {
        check_name("team", &self.name)?;
        if self.description.trim().chars().count() > MAX_DESCRIPTION {
            return Err(Error::Invalid(
                "a team description must be at most 2,000 characters".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for m in &self.members {
            if !seen.insert(m.dot_id.as_str()) {
                return Err(Error::Invalid(format!("{} is in the team twice", m.dot_id)));
            }
            if m.role.trim().chars().count() > MAX_ROLE {
                return Err(Error::Invalid(
                    "a role must be at most 80 characters".into(),
                ));
            }
        }
        Ok(())
    }
}

/// A member of a team as it is read: the dot's name comes with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMember {
    pub dot_id: String,
    pub dot_name: String,
    pub role: String,
}

/// A team's project: a local git repo a workflow run works on (teams spec section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSpec {
    pub name: String,
    pub repo_path: String,
    /// `None`: found as worktrees find it (`origin/HEAD`, else `HEAD`).
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default = "origin")]
    pub remote: String,
}

fn origin() -> String {
    "origin".into()
}

impl ProjectSpec {
    /// The rules that need no disk; that `repo_path` is a git repo is checked when it is saved.
    pub fn validate(&self) -> Result<()> {
        check_name("project", &self.name)?;
        if self.repo_path.trim().is_empty() {
            return Err(Error::Invalid("a project needs its repo folder".into()));
        }
        check_git_name("the remote", &self.remote)?;
        if let Some(branch) = &self.default_branch {
            check_git_name("the default branch", branch)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub team_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(flatten)]
    pub spec: ProjectSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub description: String,
    pub members: Vec<TeamMember>,
    pub projects: Vec<Project>,
    /// The account the team belongs to; `None` is Personal.
    pub org_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A workflow as it is saved (teams spec section 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    pub definition: crate::workflow::Definition,
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl WorkflowSpec {
    /// Its name and its definition against the team's `members` (dot ids).
    pub fn validate(&self, members: &[String]) -> Result<()> {
        check_name("workflow", &self.name)?;
        self.definition.validate(members)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub team_id: String,
    pub created_at: String,
    pub updated_at: String,
    /// Why it cannot start now, when the team changed under it (a member it names was deleted
    /// or removed): the refusal its definition would get if it were saved now.
    pub problem: Option<String>,
    #[serde(flatten)]
    pub spec: WorkflowSpec,
}

str_enum!(WorkflowTrigger {
    Chat => "chat",
    Manual => "manual",
    Schedule => "schedule",
    Webhook => "webhook",
});
str_enum!(WorkflowRunStatus {
    Running => "running",
    Waiting => "waiting",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
});
// What a waiting workflow run waits for: the person's check, more budget, a send-back past the
// cap, or a decision about a failed step (retry or stop).
str_enum!(WaitingOn {
    Check => "check",
    Budget => "budget",
    SendBacks => "send_backs",
    StepFailed => "step_failed",
});
str_enum!(StepRunStatus {
    Running => "running",
    Waiting => "waiting",
    Succeeded => "succeeded",
    Failed => "failed",
    Cancelled => "cancelled",
    Skipped => "skipped",
});

impl WorkflowRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

impl StepRunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }
}

/// A branch a step produced (teams spec section 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRef {
    pub run_id: String,
    pub branch: String,
    pub worktree_path: String,
    pub base_commit: Option<String>,
}

/// What a finished step passes to the next ones (teams spec section 6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    pub summary: String,
    pub branches: Vec<BranchRef>,
    pub outcome: Option<String>,
}

/// One step's execution inside a workflow run. Times are RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRun {
    pub id: String,
    pub workflow_run_id: String,
    pub step_id: String,
    pub attempt: u32,
    pub status: StepRunStatus,
    pub handoff: Option<Handoff>,
    /// A check's decision: `{"decision": "approved" | "sent_back", "note": ...}`.
    pub decision: Option<Value>,
    pub error: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// One execution of a workflow (teams spec section 4), with its step runs, oldest first, and
/// what it has spent (section 9). Times are RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: Option<String>,
    pub team_id: String,
    pub definition: crate::workflow::Definition,
    pub project_id: Option<String>,
    pub project: ProjectSpec,
    pub task: String,
    pub trigger: WorkflowTrigger,
    pub origin_run_id: Option<String>,
    /// A test run: its deploy steps push nothing (section 5.1).
    pub test: bool,
    pub status: WorkflowRunStatus,
    pub waiting_on: Option<WaitingOn>,
    pub current_step: Option<String>,
    /// The limits now: the definition's, raised by "Add budget".
    pub budget: crate::workflow::Budget,
    pub send_backs: u32,
    /// Dot runs of the workflow run, resume children included.
    pub spent_runs: u32,
    /// New input plus output tokens of those runs: `tokens_in - tokens_cache_read + tokens_out`.
    pub spent_tokens: u64,
    /// Their tokens read from the prompt cache, shown beside the budget, not counted in it.
    pub spent_cache_read: u64,
    /// Time spent not waiting on the person, in milliseconds.
    pub active_ms: u64,
    pub error: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub steps: Vec<StepRun>,
}

/// Input for `Store::create_workflow_run`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewWorkflowRun {
    pub workflow_id: String,
    pub team_id: String,
    pub definition: crate::workflow::Definition,
    pub project_id: String,
    pub project: ProjectSpec,
    pub task: String,
    pub trigger: WorkflowTrigger,
    pub origin_run_id: Option<String>,
    pub test: bool,
}

/// The fields `Store::set_workflow_run` writes: `None` leaves one as it is, `Some(None)` clears
/// it. A terminal status also sets `ended_at`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkflowRunChange {
    pub status: Option<WorkflowRunStatus>,
    pub waiting_on: Option<Option<WaitingOn>>,
    pub current_step: Option<Option<String>>,
    pub budget: Option<crate::workflow::Budget>,
    pub send_backs: Option<u32>,
    pub active_ms: Option<u64>,
    pub clock_at: Option<Option<String>>,
    pub error: Option<Option<String>>,
}

/// The fields `Store::set_step_run` writes, as `WorkflowRunChange`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepRunChange {
    pub status: Option<StepRunStatus>,
    pub handoff: Option<Option<Handoff>>,
    pub decision: Option<Option<Value>>,
    pub error: Option<Option<String>>,
}

/// A workflow a member may start from a chat (`start_workflow`, teams spec section 7): turned
/// on, without a problem, with a chat trigger that names the member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartableWorkflow {
    pub workflow_id: String,
    pub workflow: String,
    pub team: String,
    /// The team's projects, by name.
    pub projects: Vec<String>,
}

str_enum!(WatcherKind { Folder => "folder", Email => "email", Github => "github" });
// Why Dots paused a watcher (watchers spec sections 5 and 7); the person's pause is `enabled`.
str_enum!(PausedReason { Flood => "flood", Error => "error" });
str_enum!(FolderEvent { Created => "created", Changed => "changed", Deleted => "deleted" });
str_enum!(GithubEvent {
    Issue => "issue",
    PullRequest => "pull_request",
    Comment => "comment",
    Push => "push",
    Ci => "ci",
});

fn all_folder_events() -> Vec<FolderEvent> {
    FolderEvent::ALL.to_vec()
}

fn inbox_label() -> String {
    "INBOX".to_string()
}

fn on_by_default() -> bool {
    true
}

/// A folder watcher (watchers spec section 4.1): files matching `pattern` (empty: every file)
/// that are created, changed or deleted in `path`, and in its subfolders when `recursive`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderConfig {
    pub path: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default = "all_folder_events")]
    pub events: Vec<FolderEvent>,
    #[serde(default = "on_by_default")]
    pub recursive: bool,
}

impl FolderConfig {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            pattern: String::new(),
            events: all_folder_events(),
            recursive: true,
        }
    }
}

/// An email watcher (watchers spec section 4.2): Gmail over IMAP, read-only. `secret` names
/// the vault entry holding the app password; the filters that are set must all match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailConfig {
    pub address: String,
    pub secret: String,
    #[serde(default = "inbox_label")]
    pub label: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub has_attachment: Option<bool>,
}

impl EmailConfig {
    pub fn new(address: &str, secret: &str) -> Self {
        Self {
            address: address.to_string(),
            secret: secret.to_string(),
            label: inbox_label(),
            from: None,
            subject: None,
            has_attachment: None,
        }
    }
}

/// A GitHub watcher (watchers spec section 4.3): `repos` polled with the token in the vault
/// entry `secret`; the filters that are set must all match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubConfig {
    pub repos: Vec<String>,
    pub events: Vec<GithubEvent>,
    pub secret: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default = "on_by_default")]
    pub skip_bots: bool,
    #[serde(default)]
    pub branches: Vec<String>,
}

impl GithubConfig {
    pub fn new(repos: &[&str], secret: &str) -> Self {
        Self {
            repos: repos.iter().map(|r| r.to_string()).collect(),
            events: vec![GithubEvent::Issue, GithubEvent::PullRequest],
            secret: secret.to_string(),
            label: None,
            authors: Vec::new(),
            skip_bots: true,
            branches: Vec::new(),
        }
    }
}

/// What a watcher watches, tagged by its kind (watchers spec section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WatcherConfig {
    Folder(FolderConfig),
    Email(EmailConfig),
    Github(GithubConfig),
}

impl WatcherConfig {
    pub fn kind(&self) -> WatcherKind {
        match self {
            WatcherConfig::Folder(_) => WatcherKind::Folder,
            WatcherConfig::Email(_) => WatcherKind::Email,
            WatcherConfig::Github(_) => WatcherKind::Github,
        }
    }

    /// The vault entry the watcher reads its password or token from.
    pub fn secret(&self) -> Option<&str> {
        match self {
            WatcherConfig::Folder(_) => None,
            WatcherConfig::Email(c) => Some(&c.secret),
            WatcherConfig::Github(c) => Some(&c.secret),
        }
    }
}

/// A watcher of a dot (watchers spec section 3). `enabled` is the person's switch;
/// `paused_reason` is set when Dots paused it. Times are RFC 3339 UTC (`util::now`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
    pub dot_id: String,
    pub name: String,
    pub config: WatcherConfig,
    pub enabled: bool,
    pub paused_reason: Option<PausedReason>,
    pub last_error: Option<String>,
    pub last_event_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Watcher {
    pub fn kind(&self) -> WatcherKind {
        self.config.kind()
    }
}

/// A watcher Dots paused, as the Inbox lists it (watchers spec section 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PausedWatcher {
    pub watcher_id: String,
    pub dot_id: String,
    pub dot_name: String,
    pub kind: WatcherKind,
    pub name: String,
    pub reason: PausedReason,
    pub error: Option<String>,
    /// When Dots paused it.
    pub at: String,
}
