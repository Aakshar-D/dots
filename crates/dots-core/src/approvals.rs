use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::engine::{PermissionGate, PermissionOutcome};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{ApplyState, Approval, ApprovalStatus, Dot, EngineKind};
use crate::policy::{Action, McpTrust, Rule};
use crate::store::Store;
use crate::util::json_hash;
use crate::{Error, Result};

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

/// Why an approved MCP call is not performed after its server's accepted tool list changed.
pub const TOOL_CHANGED: &str = "the tool changed since it was approved";

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
    /// The engines that perform an approved call themselves after a late decision.
    self_apply: Mutex<HashSet<EngineKind>>,
}

impl ApprovalHub {
    pub fn new(store: Store, bus: Bus) -> Self {
        Self {
            store,
            bus,
            waiters: Mutex::new(HashMap::new()),
            self_apply: Mutex::new(HashSet::new()),
        }
    }

    /// Names the engines whose dots get approvals with `self_apply` from now on
    /// (`Engine::applies_approved_calls`). Approvals that exist keep what they recorded.
    pub fn set_self_apply(&self, kinds: impl IntoIterator<Item = EngineKind>) {
        *self.self_apply.lock().unwrap() = kinds.into_iter().collect();
    }

    /// Announces an approval whose `apply_state` just changed.
    async fn announce_update(&self, id: &str) -> Result<()> {
        let approval = self.store.get_approval(id).await?;
        let _ = self.bus.send(RuntimeEvent::ApprovalUpdated { approval });
        Ok(())
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
        self.decide_with_rule(id, approved, note, None).await
    }

    /// Like `decide`. With `rule`, an approval also adds that rule to the policy of the
    /// approval's dot, in one transaction with the decision: `check` reads the policy on every
    /// call, so the rule holds from the moment the decision does. The caller chooses the rule;
    /// this only refuses one that comes with a denial.
    pub async fn decide_with_rule(
        &self,
        id: &str,
        approved: bool,
        note: Option<String>,
        rule: Option<Rule>,
    ) -> Result<DecideEffect> {
        if rule.is_some() && !approved {
            return Err(Error::Invalid(
                "a rule can only be saved with an approval".into(),
            ));
        }
        // Take the waiter first so the decision and its `parked` flag are written together:
        // a decided row is never left looking live while no engine can receive it.
        let waiter = self.waiters.lock().unwrap().remove(id);
        let parked = waiter.is_none();
        let decided = match &rule {
            Some(rule) => {
                self.store
                    .approve_with_rule(id, note.as_deref(), parked, rule)
                    .await
            }
            None => {
                self.store
                    .decide_approval_with_parked(id, approved, note.as_deref(), parked)
                    .await
            }
        };
        let mut approval = match decided {
            Ok(a) => a,
            Err(e) => {
                // Nothing was written; hand the waiter back so a retry can still go live.
                if let Some(tx) = waiter.filter(|tx| !tx.is_closed()) {
                    self.waiters
                        .lock()
                        .unwrap()
                        .entry(id.to_string())
                        .or_insert(tx);
                }
                return Err(e);
            }
        };
        let live = match waiter {
            Some(tx) => tx.send(Decision { approved, note }).is_ok(),
            None => false,
        };
        if !live && !approval.parked {
            // The engine stopped waiting between the update and the send.
            self.store.mark_parked(id).await?;
            approval = self.store.get_approval(id).await?;
        }
        let _ = self.bus.send(RuntimeEvent::ApprovalDecided {
            approval: approval.clone(),
        });
        Ok(if live {
            DecideEffect::Live(approval)
        } else {
            DecideEffect::Parked(approval)
        })
    }

    async fn ask(
        &self,
        run_id: &str,
        tool: &str,
        input: Value,
        wait: Duration,
        self_apply: bool,
        mcp_fingerprint: Option<&str>,
    ) -> Result<PermissionOutcome> {
        let approval = self
            .store
            .create_mcp_approval(run_id, tool, &input, self_apply, mcp_fingerprint)
            .await?;
        let (tx, mut rx) = oneshot::channel();
        self.waiters.lock().unwrap().insert(approval.id.clone(), tx);
        let _guard = WaiterGuard {
            waiters: &self.waiters,
            id: approval.id.clone(),
        };
        let _ = self.bus.send(RuntimeEvent::ApprovalRequested {
            approval: approval.clone(),
        });
        // A decision may have landed between create_approval and the waiter insert. `decide`
        // found no waiter then, wrote the row as parked and returned `Parked`, so the caller
        // resumes the run with a grant. Executing the action here as well would run it twice,
        // so tell the engine it is queued and leave the outcome to the parked/resume path.
        // Re-reading after the insert is race-free: a later decision reaches the waiter.
        let current = self.store.get_approval(&approval.id).await?;
        if current.status == ApprovalStatus::Pending {
            if let Ok(Ok(d)) = tokio::time::timeout(wait, &mut rx).await {
                return Ok(outcome_of(d.approved, d.note.as_deref(), input));
            }
        }
        // Stop accepting decisions, then take one that was sent before the close: `decide`
        // reports `Live` exactly when its send succeeded, so that decision must be honoured
        // here; any later send fails and `decide` parks the row itself.
        rx.close();
        if let Ok(d) = rx.try_recv() {
            return Ok(outcome_of(d.approved, d.note.as_deref(), input));
        }
        self.store.mark_parked(&approval.id).await?;
        Ok(PermissionOutcome::Deny {
            message: parked_message(&approval.id),
        })
    }

    /// Carries out what the policy resolved for a call of `dot`'s run.
    async fn act(
        &self,
        action: Action,
        run_id: &str,
        dot: &Dot,
        tool: &str,
        input: Value,
        mcp_fingerprint: Option<&str>,
    ) -> Result<PermissionOutcome> {
        match action {
            Action::Allow => Ok(PermissionOutcome::Allow { input }),
            Action::Deny => Ok(PermissionOutcome::Deny {
                message: format!(
                    "Denied by the policy of dot '{}': {tool} is not permitted.",
                    dot.spec.name
                ),
            }),
            Action::Ask => {
                let wait = Duration::from_secs(dot.spec.approval_wait_secs);
                let self_apply = self.self_apply.lock().unwrap().contains(&dot.spec.engine);
                self.ask(run_id, tool, input, wait, self_apply, mcp_fingerprint)
                    .await
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
        // An MCP tool the CLI reached outside the gateway (not Dots' own `mcp__dots__*`) is
        // never decided by `default`: it is an unknown tool, so deny and allow rules apply
        // and anything else falls to `mcp_default` (spec section 4).
        let action = if tool.starts_with("mcp__") && !tool.starts_with("mcp__dots__") {
            let unknown = McpTrust {
                held: false,
                read_only: false,
            };
            dot.spec.policy.resolve_mcp(tool, &input, unknown)
        } else {
            let workspace = run.workspace_path.as_deref().map(Path::new);
            dot.spec.policy.resolve_in(tool, &input, workspace)
        };
        self.act(action, run_id, &dot, tool, input, None).await
    }

    /// A one-shot grant (the Claude engine's retry of a call approved after it parked; the local
    /// engine never gets one) is used up first, and allows the call only while the server's
    /// accepted fingerprint is still the one the call was approved under (spec section 4).
    async fn check_mcp(
        &self,
        run_id: &str,
        tool: &str,
        input: Value,
        trust: McpTrust,
        fingerprint: Option<String>,
    ) -> Result<PermissionOutcome> {
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        let key = grant_key(tool, &input);
        if let Some(granted) = self
            .store
            .take_mcp_grant(&run.root_run_id, tool, &key)
            .await?
        {
            return Ok(if granted == fingerprint {
                PermissionOutcome::Allow { input }
            } else {
                PermissionOutcome::Deny {
                    message: format!("{tool}: not performed, because {TOOL_CHANGED}."),
                }
            });
        }
        let action = dot.spec.policy.resolve_mcp(tool, &input, trust);
        self.act(action, run_id, &dot, tool, input, fingerprint.as_deref())
            .await
    }

    async fn claim_apply(&self, approval_id: &str) -> Result<bool> {
        let claimed = self.store.claim_apply(approval_id).await?;
        if claimed {
            self.announce_update(approval_id).await?;
        }
        Ok(claimed)
    }

    async fn settle_apply(&self, approval_id: &str, state: ApplyState) -> Result<()> {
        if !self.store.settle_apply(approval_id, state).await? {
            return Err(Error::Conflict(format!(
                "approval {approval_id} is not being performed"
            )));
        }
        self.announce_update(approval_id).await
    }

    async fn skip_apply(&self, approval_id: &str) -> Result<bool> {
        let skipped = self.store.skip_apply(approval_id).await?;
        if skipped {
            self.announce_update(approval_id).await?;
        }
        Ok(skipped)
    }
}
