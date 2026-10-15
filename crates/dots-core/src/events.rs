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
}

pub type Bus = broadcast::Sender<RuntimeEvent>;

pub fn new_bus() -> Bus {
    broadcast::channel(1024).0
}
