use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::engine::{PermissionGate, PermissionOutcome};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{Approval, ApprovalStatus};
use crate::policy::Action;
use crate::store::Store;
use crate::util::json_hash;
use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub approved: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecideEffect {
    /// Delivered to the waiting engine; the run continues on its own.
    Live(Approval),
    /// The engine was already told the call is queued; the run must be resumed.
    Parked(Approval),
}

pub fn parked_message(id: &str) -> String {
    format!(
        "Queued for human approval #{id}. Do not retry this action. \
         Finish any other work and end with your summary."
    )
}

/// Key for one-shot grants. Only the shell tools (`Bash`, `PowerShell`) are keyed by the command
/// alone, because the model rewrites free-text fields such as `description` when it retries.
/// Every other tool is keyed by its full input.
pub fn grant_key(tool: &str, input: &Value) -> String {
    let command = matches!(tool, "Bash" | "PowerShell")
        .then(|| input.get("command").and_then(Value::as_str))
        .flatten();
    match command {
        Some(cmd) => json_hash(&json!({ "tool": tool, "command": cmd })),
        None => json_hash(&json!({ "tool": tool, "input": input })),
    }
}

/// Removes a waiter entry when dropped, so an aborted `check` future cannot leak it.
struct WaiterGuard<'a> {
    waiters: &'a Mutex<HashMap<String, oneshot::Sender<Decision>>>,
    id: String,
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.waiters.lock().unwrap().remove(&self.id);
    }
}

fn outcome_of(approved: bool, note: Option<&str>, input: Value) -> PermissionOutcome {
    if approved {
        return PermissionOutcome::Allow { input };
    }
    let message = match note.map(str::trim) {
        Some(n) if !n.is_empty() => format!("Denied by the reviewer: {n}"),
        _ => "Denied by the reviewer.".to_string(),
    };
    PermissionOutcome::Deny { message }
}

pub struct ApprovalHub {
    store: Store,
    bus: Bus,
    waiters: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
}

impl ApprovalHub {
    pub fn new(store: Store, bus: Bus) -> Self {
        Self {
            store,
            bus,
            waiters: Mutex::new(HashMap::new()),
        }
    }

    #[doc(hidden)]
    pub fn waiter_count(&self) -> usize {
        self.waiters.lock().unwrap().len()
    }

    pub async fn decide(
        &self,
        id: &str,
        approved: bool,
        note: Option<String>,
    ) -> Result<DecideEffect> {
        let approval = self
            .store
            .decide_approval(id, approved, note.as_deref())
            .await?;
        let _ = self.bus.send(RuntimeEvent::ApprovalDecided {
            approval: approval.clone(),
        });
        let waiter = self.waiters.lock().unwrap().remove(id);
        if let Some(tx) = waiter {
            if tx.send(Decision { approved, note }).is_ok() {
                return Ok(DecideEffect::Live(approval));
            }
        }
        self.store.mark_parked(id).await?;
        Ok(DecideEffect::Parked(self.store.get_approval(id).await?))
    }

    async fn ask(
        &self,
        run_id: &str,
        tool: &str,
        input: Value,
        wait: Duration,
    ) -> Result<PermissionOutcome> {
        let approval = self.store.create_approval(run_id, tool, &input).await?;
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().unwrap().insert(approval.id.clone(), tx);
        let _guard = WaiterGuard {
            waiters: &self.waiters,
            id: approval.id.clone(),
        };
        let _ = self.bus.send(RuntimeEvent::ApprovalRequested {
            approval: approval.clone(),
        });
        // A decision may have landed between create_approval and the waiter insert, when
        // `decide` found no waiter. Re-read now that the waiter is registered: any later
        // decision reaches the waiter, any earlier one is visible here.
        let current = self.store.get_approval(&approval.id).await?;
        if current.status != ApprovalStatus::Pending {
            return Ok(outcome_of(
                current.status == ApprovalStatus::Approved,
                current.note.as_deref(),
                input,
            ));
        }
        match tokio::time::timeout(wait, rx).await {
            Ok(Ok(d)) => Ok(outcome_of(d.approved, d.note.as_deref(), input)),
            _ => {
                self.store.mark_parked(&approval.id).await?;
                Ok(PermissionOutcome::Deny {
                    message: parked_message(&approval.id),
                })
            }
        }
    }
}

#[async_trait]
impl PermissionGate for ApprovalHub {
    async fn check(&self, run_id: &str, tool: &str, input: Value) -> Result<PermissionOutcome> {
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        if self
            .store
            .take_grant(&run.root_run_id, tool, &grant_key(tool, &input))
            .await?
        {
            return Ok(PermissionOutcome::Allow { input });
        }
        let workspace = run.workspace_path.as_deref().map(Path::new);
        match dot.spec.policy.resolve_in(tool, &input, workspace) {
            Action::Allow => Ok(PermissionOutcome::Allow { input }),
            Action::Deny => Ok(PermissionOutcome::Deny {
                message: format!(
                    "Denied by the policy of dot '{}': {tool} is not permitted.",
                    dot.spec.name
                ),
            }),
            Action::Ask => {
                let wait = Duration::from_secs(dot.spec.approval_wait_secs);
                self.ask(run_id, tool, input, wait).await
            }
        }
    }
}
