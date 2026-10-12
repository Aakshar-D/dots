use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use crate::approvals::{ApprovalHub, DecideEffect};
use crate::engine::claude::args::resolve_claude;
use crate::engine::claude::{cli_version, ClaudeEngine};
use crate::engine::local::client::DEFAULT_RETRY_DELAYS;
use crate::engine::local::{self, LocalEngine};
use crate::engine::{Engine, UnavailableEngine};
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::model::{
    Approval, ClaudeStatus, Dot, DotSpec, EngineKind, NewRun, Run, RunStatus, SettingsSaved,
    Status, TriggerKind,
};
use crate::runner::{Runner, RunnerConfig};
use crate::scheduler::Scheduler;
use crate::server::{self, ServerState};
use crate::settings::Settings;
use crate::store::Store;
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub port: u16,
    pub max_concurrent_runs: usize,
    pub claude_path: Option<PathBuf>,
    pub claude_env: Vec<(String, String)>,
    /// Delays between attempts when a local endpoint is unreachable or answers 5xx.
    pub local_retry_delays: Vec<Duration>,
}

impl Config {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            port: 47321,
            max_concurrent_runs: 2,
            claude_path: None,
            claude_env: Vec::new(),
            local_retry_delays: DEFAULT_RETRY_DELAYS.to_vec(),
        }
    }

    /// The start configuration the app derives from stored settings.
    pub fn from_settings(data_dir: impl Into<PathBuf>, settings: &Settings) -> Self {
        let mut cfg = Self::new(data_dir);
        cfg.port = settings.port;
        cfg.max_concurrent_runs = settings.max_concurrent_runs;
        cfg.claude_path = settings.claude_path.as_ref().map(PathBuf::from);
        cfg
    }
}

pub struct Runtime {
    store: Store,
    bus: Bus,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    port: u16,
    server_error: Option<String>,
    claude_program: Option<PathBuf>,
    claude_version: Option<String>,
    data_dir: PathBuf,
    /// The stored settings when the runtime started, and now.
    started: Settings,
    settings: Mutex<Settings>,
    shutdown: CancellationToken,
}

impl Runtime {
    pub async fn start(cfg: Config) -> Result<Runtime> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let store = Store::open(&cfg.data_dir.join(crate::store::DB_FILE)).await?;
        let settings = store.get_settings().await?;
        let bus = new_bus();
        let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
        // A taken port must not keep the app from starting (spec §8): run without the server.
        let (listener, server_error) = match server::bind(cfg.port).await {
            Ok(l) => (Some(l), None),
            Err(e) => {
                let msg = format!("could not listen on 127.0.0.1:{}: {e}", cfg.port);
                tracing::error!("{msg}; webhooks are off and Claude dots cannot run");
                (None, Some(msg))
            }
        };
        let port = match &listener {
            Some(l) => l.local_addr()?.port(),
            None => cfg.port,
        };
        let recovered = store.recover_interrupted().await?;
        if !recovered.is_empty() {
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }

        let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
        let claude_program = resolve_claude(cfg.claude_path.as_deref());
        let claude_version = match &claude_program {
            Some(program) => cli_version(program, &cfg.claude_env).await,
            None => None,
        };
        let claude: Arc<dyn Engine> = match (&claude_program, &server_error) {
            // Claude runs need the MCP permission endpoint of the local server.
            (_, Some(_)) => Arc::new(UnavailableEngine(format!(
                "local server is not running: port {port} is in use. Change the port in \
                 Settings and restart Dots."
            ))),
            (None, None) => {
                tracing::warn!(
                    "claude CLI not found; Claude dots will fail until it is configured"
                );
                Arc::new(UnavailableEngine(
                    "claude CLI not found: install Claude Code or set its path in Settings".into(),
                ))
            }
            (Some(program), None) => {
                let mut engine = ClaudeEngine::new(program.clone(), cfg.data_dir.join("mcp"));
                for (k, v) in &cfg.claude_env {
                    engine = engine.with_env(k, v);
                }
                Arc::new(engine)
            }
        };
        engines.insert(EngineKind::Claude, claude);
        engines.insert(
            EngineKind::Local,
            Arc::new(LocalEngine::with_retry_delays(
                store.clone(),
                hub.clone(),
                cfg.local_retry_delays.clone(),
            )),
        );

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
        // Decisions parked before a restart (decided, but the resume never happened) are
        // turned into resume runs now; failures are logged and never block startup.
        match store.list_runs_by_status(RunStatus::AwaitingApproval).await {
            Ok(waiting) => {
                for run in waiting {
                    if let Err(e) = runner.maybe_resume(&run.id).await {
                        tracing::error!(run = %run.id, "startup resume check failed: {e}");
                    }
                }
            }
            Err(e) => tracing::error!("listing awaiting runs failed: {e}"),
        }
        let shutdown = CancellationToken::new();
        if let Some(listener) = listener {
            server::serve(
                listener,
                ServerState {
                    store: store.clone(),
                    runner: runner.clone(),
                    hub: hub.clone(),
                },
                shutdown.clone(),
            );
        }
        runner.spawn_dispatcher(shutdown.clone());
        let scheduler = Scheduler::new(store.clone(), runner.clone());
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
        Ok(Runtime {
            store,
            bus,
            hub,
            runner,
            scheduler,
            port,
            server_error,
            claude_program,
            claude_version,
            data_dir: cfg.data_dir.clone(),
            started: settings.clone(),
            settings: Mutex::new(settings),
            shutdown,
        })
    }

    pub async fn status(&self) -> Status {
        let settings = self.settings.lock().await.clone();
        Status {
            port: self.port,
            server_error: self.server_error.clone(),
            claude: ClaudeStatus {
                program: self
                    .claude_program
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                version: self.claude_version.clone(),
                mismatch: self
                    .claude_version
                    .as_ref()
                    .is_some_and(|v| *v != settings.tested_claude_version),
                tested_version: settings.tested_claude_version.clone(),
            },
            schedules_paused: self.scheduler.is_paused(),
            restart_required: self.started.restart_required(&settings),
            data_dir: self.data_dir.to_string_lossy().into_owned(),
        }
    }

    /// Records the detected `claude` version as tested, which clears the mismatch warning.
    pub async fn mark_claude_tested(&self) -> Result<Status> {
        let Some(version) = self.claude_version.clone() else {
            return Err(Error::Invalid("no claude CLI version was detected".into()));
        };
        let mut next = self.settings().await;
        next.tested_claude_version = version;
        self.update_settings(next).await?;
        Ok(self.status().await)
    }

    pub async fn settings(&self) -> Settings {
        self.settings.lock().await.clone()
    }

    /// Validates and stores `next`. `schedules_paused` applies now; port, concurrency and the
    /// `claude` path apply after a restart (`restart_required`). Autostart is the app's job.
    pub async fn update_settings(&self, next: Settings) -> Result<SettingsSaved> {
        let mut current = self.settings.lock().await;
        self.store.put_settings(&next).await?;
        self.scheduler.set_paused(next.schedules_paused);
        *current = next.clone();
        Ok(SettingsSaved {
            restart_required: self.started.restart_required(&next),
            settings: next,
        })
    }

    pub async fn set_schedules_paused(&self, paused: bool) -> Result<Status> {
        let mut next = self.settings().await;
        next.schedules_paused = paused;
        self.update_settings(next).await?;
        Ok(self.status().await)
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
        self.store.delete_dot_if_idle(id).await?;
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
            if !parent.status.is_terminal() || parent.session_id.is_none() {
                return Err(Error::Conflict(
                    "parent run has no finished session to continue".into(),
                ));
            }
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
        self.runner.cancel(run_id).await
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
                if let Err(e) = self.runner.maybe_resume(&a.run_id).await {
                    tracing::error!(run = %a.run_id, "resume after approval failed: {e}");
                }
                Ok(a)
            }
        }
    }

    /// Checks that `model` at the OpenAI-compatible `endpoint` answers with a well-formed tool
    /// call (the Settings "Test endpoint" button). Returns a short success message.
    pub async fn test_endpoint(&self, endpoint: &str, model: &str) -> Result<String> {
        local::probe(endpoint, model).await
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}
