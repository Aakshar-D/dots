use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::engine::{Engine, EngineEvent, RunContext};
use crate::events::{Bus, RuntimeEvent};
use crate::model::{EngineKind, NewRun, Run, RunStatus};
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
    #[allow(dead_code)] // used by automatic resume (Task 9)
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

    pub async fn dispatch_once(self: &Arc<Self>) -> Result<usize> {
        let mut started = 0;
        for run in self.store.dispatchable_runs().await? {
            {
                let active = self.active.lock().unwrap();
                if active.len() >= self.cfg.max_concurrent {
                    break;
                }
                if active.values().any(|a| a.dot_id == run.dot_id) {
                    continue;
                }
            }
            if !self.store.claim_run(&run.id).await? {
                continue;
            }
            let cancel = CancellationToken::new();
            let by_user = Arc::new(AtomicBool::new(false));
            self.active.lock().unwrap().insert(
                run.id.clone(),
                Active {
                    dot_id: run.dot_id.clone(),
                    cancel: cancel.clone(),
                    cancelled_by_user: by_user.clone(),
                },
            );
            // Nothing fallible may sit between claiming and handing off: failures
            // after this point go through `execute`'s error path, which frees the slot.
            let this = self.clone();
            let run_id = run.id.clone();
            tokio::spawn(async move { this.execute(run_id, cancel, by_user).await });
            started += 1;
        }
        Ok(started)
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
        self.active.lock().unwrap().remove(&run_id);
        self.secrets.lock().unwrap().retain(|_, r| r != &run_id);
        self.after_finish(&run_id).await;
        self.wake.notify_one();
    }

    /// Hook for post-run work; Task 9 adds automatic resume here.
    async fn after_finish(&self, _run_id: &str) {}

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
        let dot = self.store.get_dot(&run.dot_id).await?;
        // Resolve the engine first so a missing one never creates a workspace.
        let engine = self.engines.get(&dot.spec.engine).cloned().ok_or_else(|| {
            Error::Invalid(format!(
                "engine '{}' is not available",
                dot.spec.engine.as_str()
            ))
        })?;
        let prepared = match Prepared::from_run(run) {
            Some(p) if p.path.is_dir() => p,
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
        let secret = new_token();
        self.secrets
            .lock()
            .unwrap()
            .insert(secret.clone(), run.id.clone());
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
        let signalled = {
            let active = self.active.lock().unwrap();
            match active.get(run_id) {
                Some(a) => {
                    a.cancelled_by_user.store(true, Ordering::SeqCst);
                    a.cancel.cancel();
                    true
                }
                None => false,
            }
        };
        if signalled {
            return self.store.get_run(run_id).await;
        }
        if let Some(r) = self.store.cancel_if_inactive(run_id).await? {
            if let Err(e) = self.store.expire_pending_for_run(run_id).await {
                tracing::warn!(run = %run_id, "expiring approvals failed: {e}");
            }
            self.emit_run(&r);
            return Ok(r);
        }
        let run = self.store.get_run(run_id).await?;
        match run.status {
            RunStatus::Running => Err(Error::Conflict(format!(
                "run {run_id} is starting; try again"
            ))),
            other => Err(Error::Conflict(format!(
                "run {run_id} is already {}",
                other.as_str()
            ))),
        }
    }
}
