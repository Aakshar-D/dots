use serde::Serialize;
use tokio::sync::broadcast;

use crate::model::{Approval, Run, RunEventRecord};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    RunUpdated {
        run: Run,
    },
    RunEvent {
        event: RunEventRecord,
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
}

pub type Bus = broadcast::Sender<RuntimeEvent>;

pub fn new_bus() -> Bus {
    broadcast::channel(1024).0
}
