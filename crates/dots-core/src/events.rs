use serde::Serialize;
use tokio::sync::broadcast;

use crate::inbox::InboxMark;
use crate::model::{
    Approval, Message, PausedReason, Run, RunEventRecord, ScheduleSkip, ScheduledMessage,
    WorkflowRun,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    RunUpdated {
        run: Run,
    },
    RunEvent {
        event: RunEventRecord,
    },
    /// The run's quiet mark was set (`quiet_since` is when) or cleared (`None`) by the
    /// stuck-run watchdog (stuck-run watchdog spec section 4.3). `quiet_limit_secs` is the
    /// limit the run started with. A `RunUpdated` with the row goes with it.
    RunQuiet {
        run_id: String,
        dot_id: String,
        quiet_since: Option<String>,
        quiet_limit_secs: Option<u64>,
    },
    ApprovalRequested {
        approval: Approval,
    },
    ApprovalDecided {
        approval: Approval,
    },
    /// A decided approval changed afterwards: Dots claimed, performed or gave up its call.
    ApprovalUpdated {
        approval: Approval,
    },
    /// The dot's schedule row (`schedule_state`) was made, changed or deleted.
    ScheduleUpdated {
        dot_id: String,
    },
    /// A repeating slot came due while the dot was busy with a run and was not run (skip when
    /// busy spec section 4). A `ScheduleUpdated` for the row follows.
    ScheduleSkipped {
        dot_id: String,
        skip: ScheduleSkip,
    },
    /// The dot's topics changed beyond a run (topic tools spec section 4.3): a topic was
    /// named, archived or unarchived, or runs moved between topics. Its runs, threads and topic
    /// rows are read again.
    TopicsChanged {
        dot_id: String,
    },
    /// A scheduled message was scheduled, edited, cancelled, sent or failed, or its waiting
    /// reason changed (send later spec section 4.5). The app notifies on a new failure.
    ScheduledMessageUpdated {
        message: ScheduledMessage,
    },
    /// The dot's schedule needs attention: an occurrence failed in the end (Scheduler v2 spec
    /// §6). Published once, when the flag goes from clear to set, with the failed run and its
    /// error; a `ScheduleUpdated` for the row goes with it. The app notifies on it.
    ScheduleAttention {
        dot_id: String,
        run_id: String,
        error: Option<String>,
    },
    /// A Library item was added, changed or removed, or an MCP server's catalog changed (a
    /// check, an accept, a list that changed during a run). `item_id` names the item; the
    /// Library and the Inbox read it again.
    LibraryUpdated {
        item_id: String,
    },
    /// A dot posted a note or asked a question (check-ins spec section 5). The app notifies.
    MessagePosted {
        message: Message,
    },
    /// A message was read, answered or dismissed.
    MessageUpdated {
        message: Message,
    },
    /// A follow-up of the dot was scheduled, fired, cancelled or skipped.
    FollowUpsUpdated {
        dot_id: String,
    },
    /// The person looked at the dot's conversation, or the core marked the dot being viewed
    /// seen: its `seen_at` moved, and its `unread` with it (needs-you spec section 3).
    DotSeen {
        dot_id: String,
    },
    /// A memory of the dot was saved, changed or deleted, by a run or by the person (dot
    /// memory spec section 3).
    MemoriesUpdated {
        dot_id: String,
    },
    /// A call changed the app's data (app library spec section 5): its screen reads again.
    AppDataChanged {
        app: String,
    },
    /// A dot proposed an app, or the person decided a proposal (app library spec section 6.2):
    /// the review card reads it again.
    AppProposalChanged {
        id: String,
        dot_id: String,
    },
    /// A dot proposed instructions, or a proposal was decided or replaced (conversational chat
    /// spec section 4): the card reads it again.
    InstructionProposalChanged {
        id: String,
        dot_id: String,
    },
    /// A team, its members or its projects were made, changed or deleted (teams spec
    /// section 11).
    TeamUpdated {
        team_id: String,
    },
    /// A workflow of the team was made, changed or deleted.
    WorkflowUpdated {
        team_id: String,
        workflow_id: String,
    },
    /// A workflow run started, moved to another step, waited, or ended (teams spec section
    /// 11): the run page, the chat card, Needs you and the Inbox read it.
    WorkflowRunUpdated {
        run: WorkflowRun,
    },
    /// A watcher of the dot was added, changed, paused, resumed, deleted or had an event
    /// (watchers spec section 7).
    WatcherUpdated {
        dot_id: String,
        watcher_id: String,
    },
    /// Dots paused a watcher (too many runs, or an error it cannot get past). Published once,
    /// when it goes from running to paused; a `WatcherUpdated` goes with it. The app notifies
    /// on it.
    WatcherPaused {
        dot_id: String,
        watcher_id: String,
        name: String,
        reason: PausedReason,
        error: Option<String>,
    },
    /// The Claude limit readings or the hold changed (usage and limits spec section 6): a new
    /// reading, a hold starting or ending, a held run starting or released. `change` says when
    /// a hold started or ended; the app notifies on it.
    LimitsUpdated {
        limits: crate::limits::LimitsState,
        change: Option<crate::limits::HoldChange>,
    },
    /// An org was joined, updated, fetched (by **Refresh**, its timer or the start-up fetch) or
    /// removed (env vault spec §6.4): its status, policy or values may have changed, and so
    /// what Settings, the Env tab and the Library show of it.
    OrgUpdated {
        org_id: String,
    },
    /// An Inbox item was snoozed, unsnoozed, marked done or undone, came back from snooze, or
    /// its mark went because it left (snooze and done spec section 4.4): the Inbox and the
    /// counts read again.
    InboxUpdated,
    /// Snoozed items came back at their time, in one wake pass (spec section 4.5). The app
    /// notifies once for them all. An `InboxUpdated` goes with it.
    SnoozeEnded {
        items: Vec<InboxMark>,
    },
    /// A goal of the workspace `workspace_id` of `org_id` (`None` is Personal) was added,
    /// changed, marked done or undone, or deleted, or a deleted dot left it unlinked: the
    /// Intents screen reads it again.
    IntentsUpdated {
        org_id: Option<String>,
        workspace_id: String,
    },
    /// A project's graph started building, made progress (at most every 250 ms), or finished
    /// or failed (project graph spec section 5). The Graph screen of a dot in that project
    /// (`status.key`) reads it again.
    GraphUpdated {
        status: crate::graph::GraphStatus,
    },
}

pub type Bus = broadcast::Sender<RuntimeEvent>;

pub fn new_bus() -> Bus {
    broadcast::channel(1024).0
}
