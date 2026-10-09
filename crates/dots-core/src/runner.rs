use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use serde_json::json;

use crate::approvals::grant_key;
use crate::engine::{Engine, EngineEvent, RunContext};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    ApprovalStatus, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use crate::prompt::build_prompt;
use crate::store::Store;
use crate::util::{ct_eq, new_token};
use crate::workspace::{Prepared, WorkspaceManager};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub max_concurrent: usize,
    pub mcp_url: String,
    pub worktrees_dir: PathBuf,
    pub cancel_grace: Duration,
}

struct Active {
    dot_id: String,
    cancel: CancellationToken,
    cancelled_by_user: Arc<AtomicBool>,
}

pub struct Runner {
    store: Store,
    bus: Bus,
    engines: HashMap<EngineKind, Arc<dyn Engine>>,
    workspaces: WorkspaceManager,
    cfg: RunnerConfig,
    active: Mutex<HashMap<String, Active>>,
    secrets: Mutex<HashMap<String, String>>,
    wake: Notify,
    resume_lock: tokio::sync::Mutex<()>,
}

impl Runner {
    pub fn new(
        store: Store,
        bus: Bus,
        engines: HashMap<EngineKind, Arc<dyn Engine>>,
        cfg: RunnerConfig,
    ) -> Arc<Runner> {
        Arc::new(Runner {
            workspaces: WorkspaceManager::new(cfg.worktrees_dir.clone()),
            store,
            bus,
            engines,
            cfg,
            active: Mutex::new(HashMap::new()),
            secrets: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            resume_lock: tokio::sync::Mutex::new(()),
        })
    }

    fn emit_run(&self, run: &Run) {
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run: run.clone() });
    }

    /// Creates the per-run secret the MCP endpoint uses to identify a run.
    pub fn register_secret(&self, run_id: &str) -> String {
        let secret = new_token();
        self.secrets
            .lock()
            .unwrap()
            .insert(secret.clone(), run_id.to_string());
        secret
    }

    pub fn run_for_secret(&self, secret: &str) -> Option<String> {
        let secrets = self.secrets.lock().unwrap();
        secrets
            .iter()
            .find(|(s, _)| ct_eq(s, secret))
            .map(|(_, run)| run.clone())
    }

    pub async fn enqueue(&self, new: NewRun) -> Result<Run> {
        let run = self.store.create_run(&new).await?;
        self.emit_run(&run);
        self.wake.notify_one();
        Ok(run)
    }

    pub async fn recover(&self) -> Result<Vec<String>> {
        self.store.recover_interrupted().await
    }

    pub fn spawn_dispatcher(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = this.dispatch_once().await {
                    tracing::error!("dispatch failed: {e}");
                }
                tokio::select! {
                    _ = this.wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                    _ = shutdown.cancelled() => break,
                }
            }
        })
    }

    /// Whether the runner still holds `run_id` (claimed, executing, or finishing up).
    #[doc(hidden)]
    pub fn is_active(&self, run_id: &str) -> bool {
        self.active.lock().unwrap().contains_key(run_id)
    }

    pub async fn dispatch_once(self: &Arc<Self>) -> Result<usize> {
        let mut started = 0;
        for run in self.store.dispatchable_runs().await? {
            // Register as active before claiming, so a run is never `running` in the store
            // without a cancellation handle: a cancel in between is recorded on the token
            // and honoured by `execute` before anything starts.
            let cancel = CancellationToken::new();
            let by_user = Arc::new(AtomicBool::new(false));
            {
                let mut active = self.active.lock().unwrap();
                if active.len() >= self.cfg.max_concurrent {
                    break;
                }
                if active.contains_key(&run.id) || active.values().any(|a| a.dot_id == run.dot_id) {
                    continue;
                }
                active.insert(
                    run.id.clone(),
                    Active {
                        dot_id: run.dot_id.clone(),
                        cancel: cancel.clone(),
                        cancelled_by_user: by_user.clone(),
                    },
                );
            }
            match self.store.claim_run(&run.id).await {
                Ok(true) => {}
                Ok(false) => {
                    self.active.lock().unwrap().remove(&run.id);
                    continue;
                }
                Err(e) => {
                    self.active.lock().unwrap().remove(&run.id);
                    return Err(e);
                }
            }
            // Nothing fallible may sit between claiming and handing off: failures
            // after this point go through `execute`'s error path, which frees the slot.
            // A panic skips that path, so a watcher task cleans up instead.
            let run_id = run.id.clone();
            let inner = tokio::spawn(
                self.clone()
                    .execute(run_id.clone(), cancel.clone(), by_user),
            );
            let this = self.clone();
            tokio::spawn(async move {
                if let Err(e) = inner.await {
                    if e.is_panic() {
                        this.recover_panicked(&run_id, &cancel).await;
                    }
                }
            });
            started += 1;
        }
        Ok(started)
    }

    /// Cleans up after an `execute` task that panicked: frees the slot and secrets, fails
    /// the run if it was still running, and expires its pending approvals.
    async fn recover_panicked(&self, run_id: &str, cancel: &CancellationToken) {
        tracing::error!(run = %run_id, "engine task panicked");
        cancel.cancel();
        self.active.lock().unwrap().remove(run_id);
        self.secrets.lock().unwrap().retain(|_, r| r != run_id);
        match self
            .store
            .fail_if_running(run_id, "internal error: engine task panicked")
            .await
        {
            Ok(Some(run)) => {
                if let Err(e) = self.store.expire_pending_for_run(run_id).await {
                    tracing::warn!(run = %run_id, "expiring approvals failed: {e}");
                }
                self.emit_run(&run);
            }
            Ok(None) => {}
            Err(e) => tracing::error!(run = %run_id, "failing panicked run failed: {e}"),
        }
        self.wake.notify_one();
    }

    async fn execute(
        self: Arc<Self>,
        run_id: String,
        cancel: CancellationToken,
        by_user: Arc<AtomicBool>,
    ) {
        let result = match self.store.get_run(&run_id).await {
            Ok(run) => {
                self.emit_run(&run);
                self.execute_inner(&run, cancel.clone(), &by_user).await
            }
            Err(e) => Err(e),
        };
        // Whatever happened, never leave an engine running unsupervised.
        cancel.cancel();
        if let Err(e) = result {
            tracing::warn!(run = %run_id, "run failed: {e}");
            if let Err(e2) = self.store.expire_pending_for_run(&run_id).await {
                tracing::warn!(run = %run_id, "expiring approvals failed: {e2}");
            }
            if let Ok(r) = self
                .store
                .finish_run(&run_id, RunStatus::Failed, None, Some(&e.to_string()))
                .await
            {
                self.emit_run(&r);
            }
        }
        self.secrets.lock().unwrap().retain(|_, r| r != &run_id);
        // A cancel that arrived after the final status was computed (the run parked) is
        // applied now, before the resume check could act on the parked decisions.
        self.apply_late_cancel(&run_id, &by_user).await;
        self.after_finish(&run_id).await;
        // Released only after the resume check, so `is_active` going false means the
        // runner is completely done with the run.
        self.active.lock().unwrap().remove(&run_id);
        self.apply_late_cancel(&run_id, &by_user).await;
        self.wake.notify_one();
    }

    async fn apply_late_cancel(&self, run_id: &str, by_user: &AtomicBool) {
        if !by_user.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.cancel_inactive(run_id).await {
            tracing::warn!(run = %run_id, "late cancel failed: {e}");
        }
    }

    async fn after_finish(&self, run_id: &str) {
        if let Err(e) = self.maybe_resume(run_id).await {
            tracing::error!(run = %run_id, "resume check failed: {e}");
        }
    }

    /// Turns decided, parked approvals of a finished run into a resume child run.
    pub async fn maybe_resume(&self, run_id: &str) -> Result<Option<Run>> {
        let _guard = self.resume_lock.lock().await;
        let run = self.store.get_run(run_id).await?;
        if run.status != RunStatus::AwaitingApproval
            || self.store.has_pending_for_run(run_id).await?
        {
            return Ok(None);
        }
        let decided = self.store.unresolved_parked_decisions(run_id).await?;
        let mut lines = Vec::new();
        let mut grants = Vec::new();
        let mut needs_child = false;
        for a in &decided {
            let note = a.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
            match (a.status, note) {
                (ApprovalStatus::Approved, _) => {
                    grants.push((a.tool.clone(), grant_key(&a.tool, &a.input)));
                    lines.push(format!(
                        "- Approval #{} granted: you may now use {} with the same input. Perform that action now.",
                        a.id, a.tool
                    ));
                    needs_child = true;
                }
                (_, Some(note)) => {
                    lines.push(format!(
                        "- Approval #{} for {} was denied. Reviewer note: {note}",
                        a.id, a.tool
                    ));
                    needs_child = true;
                }
                _ => lines.push(format!(
                    "- Approval #{} for {} was denied. Do not attempt it again.",
                    a.id, a.tool
                )),
            }
        }
        let child = needs_child.then(|| {
            let message = format!(
                "Human review of your queued actions:\n{}\n\nContinue the task, then end with your summary.",
                lines.join("\n")
            );
            let ids: Vec<String> = decided.iter().map(|a| a.id.clone()).collect();
            let mut new = NewRun::new(&run.dot_id, TriggerKind::Resume);
            new.payload = Some(json!({ "message": message, "approvals": ids }));
            new.parent_run_id = Some(run.id.clone());
            new.root_run_id = Some(run.root_run_id.clone());
            new.session_id = run.session_id.clone();
            new.workspace_path = run.workspace_path.clone();
            new.branch = run.branch.clone();
            new.base_commit = run.base_commit.clone();
            new
        });
        // One transaction: grants, resolved flags, child run and closing the parent
        // either all happen or none do, so a failure never loses a decision.
        let resolved: Vec<String> = decided.iter().map(|a| a.id.clone()).collect();
        let child = match self
            .store
            .resume_parked(run_id, &grants, &resolved, child.as_ref())
            .await
        {
            Ok(child) => child,
            // Cancelled (or otherwise closed) concurrently: nothing to resume.
            Err(Error::Conflict(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        match &child {
            Some(c) => {
                self.emit_run(c);
                self.wake.notify_one();
            }
            // Closed without a follow-up: nothing will use the workspace again.
            None => self.cleanup_workspace(&run).await,
        }
        self.emit_run(&self.store.get_run(run_id).await?);
        Ok(child)
    }

    /// Removes the run's worktree if it is unchanged; errors are logged, never returned.
    async fn cleanup_workspace(&self, run: &Run) {
        let Some(prepared) = Prepared::from_run(run) else {
            return;
        };
        let result = match self.store.get_dot(&run.dot_id).await {
            Ok(dot) => self
                .workspaces
                .cleanup_if_unchanged(&dot, &prepared)
                .await
                .map(|_| ()),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
        }
    }

    async fn record(&self, run_id: &str, ev: &EngineEvent) -> Result<()> {
        match ev {
            EngineEvent::SessionStarted { session_id } => {
                self.store.set_session(run_id, session_id).await?
            }
            EngineEvent::Usage {
                tokens_in,
                tokens_out,
            } => {
                self.store
                    .add_usage(run_id, *tokens_in as i64, *tokens_out as i64)
                    .await?
            }
            _ => {}
        }
        let rec = self
            .store
            .append_event(run_id, ev.kind(), &serde_json::to_value(ev)?)
            .await?;
        let _ = self.bus.send(RuntimeEvent::RunEvent { event: rec });
        Ok(())
    }

    async fn execute_inner(
        &self,
        run: &Run,
        cancel: CancellationToken,
        by_user: &AtomicBool,
    ) -> Result<()> {
        // Cancelled between claim and start: never prepare a workspace or start an engine.
        if cancel.is_cancelled() || by_user.load(Ordering::SeqCst) {
            return self.finish_cancelled_before_start(&run.id).await;
        }
        let dot = self.store.get_dot(&run.dot_id).await?;
        // Resolve the engine first so a missing one never creates a workspace.
        let engine =
            self.engines
                .get(&dot.spec.engine)
                .cloned()
                .ok_or_else(|| match dot.spec.engine {
                    EngineKind::Claude => Error::Other(
                        "claude CLI not found: install Claude Code or set Config.claude_path"
                            .into(),
                    ),
                    other => {
                        Error::Invalid(format!("engine '{}' is not available", other.as_str()))
                    }
                })?;
        let prepared = match Prepared::from_run(run) {
            Some(p) if p.path.is_dir() => p,
            // A follow-up of a session whose unchanged worktree was cleaned up: the session
            // remembers that directory, so recreate the worktree at the same path.
            Some(p)
                if run.session_id.is_some()
                    && dot.spec.workspace_mode == WorkspaceMode::Worktree
                    && p.branch.is_some()
                    && p.base_commit.is_some() =>
            {
                self.workspaces.restore(&dot, &p).await?
            }
            _ => {
                let p = self.workspaces.prepare(&dot, &run.id).await?;
                self.store
                    .set_workspace(
                        &run.id,
                        &p.path.to_string_lossy(),
                        p.branch.as_deref(),
                        p.base_commit.as_deref(),
                    )
                    .await?;
                p
            }
        };
        if cancel.is_cancelled() {
            if let Err(e) = self.workspaces.cleanup_if_unchanged(&dot, &prepared).await {
                tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
            }
            return self.finish_cancelled_before_start(&run.id).await;
        }
        let secret = self.register_secret(&run.id);
        let ctx = RunContext {
            run_id: run.id.clone(),
            dot: dot.clone(),
            workspace: prepared.path.clone(),
            prompt: build_prompt(&dot, run, &prepared.path, chrono::Local::now()),
            session_id: run.session_id.clone(),
            mcp_url: self.cfg.mcp_url.clone(),
            mcp_secret: secret,
            cancel: cancel.clone(),
        };
        let mut rx = engine.start(ctx).await?;

        let deadline = Instant::now() + Duration::from_secs(dot.spec.timeout_secs);
        let mut grace: Option<Instant> = None;
        let mut timed_out = false;
        let mut summary: Option<String> = None;
        let mut error: Option<String> = None;
        loop {
            let limit = grace.unwrap_or(deadline);
            tokio::select! {
                ev = rx.recv() => {
                    let Some(ev) = ev else { break };
                    self.record(&run.id, &ev).await?;
                    match ev {
                        EngineEvent::Finished { summary: s } => summary = Some(s),
                        EngineEvent::Failed { error: e } => error = Some(e),
                        _ => {}
                    }
                }
                _ = tokio::time::sleep_until(limit) => {
                    if grace.is_some() {
                        tracing::warn!(run = %run.id, "engine ignored cancellation; abandoning it");
                        break;
                    }
                    timed_out = true;
                    cancel.cancel();
                    grace = Some(Instant::now() + self.cfg.cancel_grace);
                }
                _ = cancel.cancelled(), if grace.is_none() => {
                    grace = Some(Instant::now() + self.cfg.cancel_grace);
                }
            }
        }

        let user_cancelled = by_user.load(Ordering::SeqCst);
        let waiting = self.store.has_pending_for_run(&run.id).await?
            || !self
                .store
                .unresolved_parked_decisions(&run.id)
                .await?
                .is_empty();
        let (status, err_text) = if user_cancelled {
            (RunStatus::Cancelled, None)
        } else if timed_out {
            (
                RunStatus::Failed,
                Some(format!("timeout after {}s", dot.spec.timeout_secs)),
            )
        } else if waiting {
            (RunStatus::AwaitingApproval, error)
        } else if let Some(e) = error {
            (RunStatus::Failed, Some(e))
        } else if summary.is_some() {
            (RunStatus::Succeeded, None)
        } else {
            (
                RunStatus::Failed,
                Some("engine exited without a result".to_string()),
            )
        };
        if user_cancelled || timed_out {
            self.store.expire_pending_for_run(&run.id).await?;
        }
        // Clean up before publishing the terminal status so observers that see
        // `succeeded`/`failed` never find a leftover unchanged worktree.
        if status != RunStatus::AwaitingApproval {
            if let Err(e) = self.workspaces.cleanup_if_unchanged(&dot, &prepared).await {
                tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
            }
        }
        let finished = self
            .store
            .finish_run(&run.id, status, summary.as_deref(), err_text.as_deref())
            .await?;
        self.emit_run(&finished);
        Ok(())
    }

    pub async fn cancel(&self, run_id: &str) -> Result<Run> {
        // A run only moves forward (queued -> held and running -> released and parked or
        // finished), and it is held before it is claimed, so retrying the two checks
        // catches every transition that lands between them.
        for _ in 0..3 {
            if let Some(first) = self.signal_cancel(run_id) {
                // Still held by the runner. If it parked, `execute` applies the cancel once
                // it lets go; if it already finished, there is nothing left to cancel.
                // A run that is `cancelled` right after our first signal was cancelled by it
                // (e.g. before its engine started).
                let run = self.store.get_run(run_id).await?;
                if run.status.is_terminal() && !(first && run.status == RunStatus::Cancelled) {
                    return Err(already(&run));
                }
                return Ok(run);
            }
            if let Some(r) = self.cancel_inactive(run_id).await? {
                return Ok(r);
            }
            let run = self.store.get_run(run_id).await?;
            if run.status.is_terminal() {
                return Err(already(&run));
            }
        }
        Err(already(&self.store.get_run(run_id).await?))
    }

    /// Flags a held run as cancelled by the user. `None` if the runner does not hold it,
    /// otherwise whether this was the first cancel request for the run.
    fn signal_cancel(&self, run_id: &str) -> Option<bool> {
        let active = self.active.lock().unwrap();
        match active.get(run_id) {
            Some(a) => {
                let first = !a.cancelled_by_user.swap(true, Ordering::SeqCst);
                a.cancel.cancel();
                Some(first)
            }
            None => None,
        }
    }

    /// Cancels a `queued` or `awaiting_approval` run that the runner does not hold, expiring
    /// its pending approvals and removing an unchanged worktree of a parked run.
    async fn cancel_inactive(&self, run_id: &str) -> Result<Option<Run>> {
        // Serialised with `maybe_resume` so a cancel never interleaves with a resume.
        let (prior, cancelled) = {
            let _guard = self.resume_lock.lock().await;
            let prior = self.store.get_run(run_id).await?;
            let cancelled = self.store.cancel_if_inactive(run_id).await?;
            if cancelled.is_some() {
                if let Err(e) = self.store.expire_pending_for_run(run_id).await {
                    tracing::warn!(run = %run_id, "expiring approvals failed: {e}");
                }
            }
            (prior, cancelled)
        };
        let Some(run) = cancelled else {
            return Ok(None);
        };
        self.emit_run(&run);
        if prior.status == RunStatus::AwaitingApproval {
            self.cleanup_workspace(&run).await;
        }
        Ok(Some(run))
    }

    /// Finishes a run that was cancelled before its engine started.
    async fn finish_cancelled_before_start(&self, run_id: &str) -> Result<()> {
        self.store.expire_pending_for_run(run_id).await?;
        let run = self
            .store
            .finish_run(run_id, RunStatus::Cancelled, None, None)
            .await?;
        self.emit_run(&run);
        Ok(())
    }
}

fn already(run: &Run) -> Error {
    Error::Conflict(format!("run {} is already {}", run.id, run.status.as_str()))
}
