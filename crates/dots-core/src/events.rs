use serde::Serialize;
use tokio::sync::broadcast;

use crate::model::{Approval, Message, Run, RunEventRecord};

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
}

pub type Bus = broadcast::Sender<RuntimeEvent>;

pub fn new_bus() -> Bus {
    broadcast::channel(1024).0
}
