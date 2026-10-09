use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::approvals::{ApprovalHub, DecideEffect};
use crate::engine::claude::args::resolve_claude;
use crate::engine::claude::ClaudeEngine;
use crate::engine::Engine;
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::model::{Approval, Dot, DotSpec, EngineKind, NewRun, Run, RunStatus, TriggerKind};
use crate::runner::{Runner, RunnerConfig};
use crate::scheduler::Scheduler;
use crate::server::{self, ServerState};
use crate::store::Store;
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub port: u16,
    pub max_concurrent_runs: usize,
    pub claude_path: Option<PathBuf>,
    pub claude_env: Vec<(String, String)>,
}

impl Config {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            port: 47321,
            max_concurrent_runs: 2,
            claude_path: None,
            claude_env: Vec::new(),
        }
    }
}

pub struct Runtime {
    store: Store,
    bus: Bus,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    port: u16,
    claude_program: Option<PathBuf>,
    shutdown: CancellationToken,
}

impl Runtime {
    pub async fn start(cfg: Config) -> Result<Runtime> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let store = Store::open(&cfg.data_dir.join("dots.db")).await?;
        let recovered = store.recover_interrupted().await?;
        if !recovered.is_empty() {
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }
        let bus = new_bus();
        let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
        let listener = server::bind(cfg.port).await?;
        let port = listener.local_addr()?.port();

        let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
        let claude_program = resolve_claude(cfg.claude_path.as_deref());
        match &claude_program {
            Some(program) => {
                let mut engine = ClaudeEngine::new(program.clone(), cfg.data_dir.join("mcp"));
                for (k, v) in &cfg.claude_env {
                    engine = engine.with_env(k, v);
                }
                engines.insert(EngineKind::Claude, Arc::new(engine));
            }
            None => {
                tracing::warn!("claude CLI not found; Claude dots will fail until it is configured")
            }
        }

        let runner = Runner::new(
            store.clone(),
            bus.clone(),
            engines,
            RunnerConfig {
                max_concurrent: cfg.max_concurrent_runs,
                mcp_url: format!("http://127.0.0.1:{port}/mcp"),
                worktrees_dir: cfg.data_dir.join("worktrees"),
                cancel_grace: Duration::from_secs(10),
            },
        );
        let shutdown = CancellationToken::new();
        server::serve(
            listener,
            ServerState {
                store: store.clone(),
                runner: runner.clone(),
                hub: hub.clone(),
            },
            shutdown.clone(),
        );
        runner.spawn_dispatcher(shutdown.clone());
        let scheduler = Scheduler::new(store.clone(), runner.clone());
        scheduler.spawn(shutdown.clone());
        Ok(Runtime {
            store,
            bus,
            hub,
            runner,
            scheduler,
            port,
            claude_program,
            shutdown,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.bus.subscribe()
    }

    pub fn claude_program(&self) -> Option<&Path> {
        self.claude_program.as_deref()
    }

    pub fn webhook_url(&self, dot: &Dot) -> String {
        format!("http://127.0.0.1:{}/dots/{}/trigger", self.port, dot.id)
    }

    pub async fn create_dot(&self, spec: DotSpec) -> Result<Dot> {
        let dot = self.store.create_dot(&spec).await?;
        self.scheduler.reload();
        Ok(dot)
    }

    pub async fn update_dot(&self, id: &str, spec: DotSpec) -> Result<Dot> {
        let dot = self.store.update_dot(id, &spec).await?;
        self.scheduler.reload();
        Ok(dot)
    }

    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        let active = self
            .store
            .list_runs(Some(id), 1000)
            .await?
            .into_iter()
            .any(|r| !r.status.is_terminal());
        if active {
            return Err(Error::Conflict(
                "dot has queued, running or awaiting runs; cancel them first".into(),
            ));
        }
        self.store.delete_dot(id).await?;
        self.scheduler.reload();
        Ok(())
    }

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
        let dot = self.store.set_dot_enabled(id, enabled).await?;
        self.scheduler.reload();
        Ok(dot)
    }

    pub async fn regenerate_webhook_token(&self, id: &str) -> Result<String> {
        let token = self.store.regenerate_webhook_token(id).await?;
        self.scheduler.reload();
        Ok(token)
    }

    pub async fn run_now(&self, dot_id: &str) -> Result<Run> {
        self.store.get_dot(dot_id).await?;
        self.runner
            .enqueue(NewRun::new(dot_id, TriggerKind::Manual))
            .await
    }

    pub async fn chat(
        &self,
        dot_id: &str,
        message: &str,
        parent_run_id: Option<&str>,
    ) -> Result<Run> {
        if message.trim().is_empty() {
            return Err(Error::Invalid("message must not be empty".into()));
        }
        self.store.get_dot(dot_id).await?;
        let mut new = NewRun::new(dot_id, TriggerKind::Chat);
        new.payload = Some(json!({ "message": message }));
        if let Some(pid) = parent_run_id {
            let parent = self.store.get_run(pid).await?;
            if parent.dot_id != dot_id {
                return Err(Error::Invalid(format!(
                    "run {pid} belongs to a different dot"
                )));
            }
            new.parent_run_id = Some(parent.id.clone());
            new.root_run_id = Some(parent.root_run_id.clone());
            new.session_id = parent.session_id.clone();
            new.workspace_path = parent.workspace_path.clone();
            new.branch = parent.branch.clone();
            new.base_commit = parent.base_commit.clone();
        }
        self.runner.enqueue(new).await
    }

    pub async fn cancel_run(&self, run_id: &str) -> Result<Run> {
        // The runner reports a Conflict while a claimed run has not registered as
        // active yet; that window is short, so retry briefly before surfacing it.
        let mut attempts = 0;
        loop {
            match self.runner.cancel(run_id).await {
                Err(Error::Conflict(_)) if attempts < 100 => {
                    if self.store.get_run(run_id).await?.status != RunStatus::Running {
                        return self.runner.cancel(run_id).await;
                    }
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => return other,
            }
        }
    }

    pub async fn decide_approval(
        &self,
        id: &str,
        approved: bool,
        note: Option<String>,
    ) -> Result<Approval> {
        match self.hub.decide(id, approved, note).await? {
            DecideEffect::Live(a) => Ok(a),
            DecideEffect::Parked(a) => {
                self.runner.maybe_resume(&a.run_id).await?;
                Ok(a)
            }
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}
