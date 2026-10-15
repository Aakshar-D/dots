use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, Utc};
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
    Approval, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox, NewRun, PendingApproval,
    ReviewRun, Run, RunEventRecord, RunStatus, ScheduleChange, SettingsSaved, Status, TriggerKind,
    WorktreeDiff,
};
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{next_slot, upcoming, Schedule};
use crate::scheduler::{Clock, Scheduler, SystemClock};
use crate::server::{self, ServerState};
use crate::settings::Settings;
use crate::store::Store;
use crate::workspace::{Prepared, WorkspaceManager};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub port: u16,
    /// How long `Runtime::start` keeps trying a port that is in use before it starts without
    /// the server (`server::bind_retrying`). A test that holds the port on purpose shortens it.
    pub bind_wait: Duration,
    pub max_concurrent_runs: usize,
    pub claude_path: Option<PathBuf>,
    pub claude_env: Vec<(String, String)>,
    /// Delays between attempts when a local endpoint is unreachable or answers 5xx.
    pub local_retry_delays: Vec<Duration>,
    /// Where the scheduler and the schedule rows get the time. Tests move a fake one by hand.
    pub clock: Arc<dyn Clock>,
}

impl Config {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            port: 47321,
            bind_wait: server::BIND_WAIT,
            max_concurrent_runs: 2,
            claude_path: None,
            claude_env: Vec::new(),
            local_retry_delays: DEFAULT_RETRY_DELAYS.to_vec(),
            clock: Arc::new(SystemClock),
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
    clock: Arc<dyn Clock>,
    port: u16,
    server_error: Option<String>,
    claude_program: Option<PathBuf>,
    claude_version: Option<String>,
    data_dir: PathBuf,
    /// The stored settings when the runtime started, and now.
    started: Settings,
    settings: Mutex<Settings>,
    /// Makes `chat` check a thread and add to it in one step.
    chat_lock: Mutex<()>,
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
        // The bind waits a moment for a port in use, which after "Restart Dots" is the old
        // process on its way out.
        let (listener, bind_error) = match server::bind_retrying(cfg.port, cfg.bind_wait).await {
            Ok(l) => (Some(l), None),
            Err(e) => (None, Some(e)),
        };
        let server_error = bind_error
            .as_ref()
            .map(|e| format!("could not listen on 127.0.0.1:{}: {e}", cfg.port));
        if let Some(msg) = &server_error {
            tracing::error!("{msg}; webhooks are off and Claude dots cannot run");
        }
        let port = match &listener {
            Some(l) => l.local_addr()?.port(),
            None => cfg.port,
        };
        let recovered = store.recover_interrupted().await?;
        if !recovered.is_empty() {
            tracing::warn!("marked {} interrupted run(s) as failed", recovered.len());
        }
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
        // migration 0005 gets one now, and a row a failed change left stale is reset.
        for dot in store.list_dots().await? {
            store
                .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, cfg.clock.now())
                .await?;
        }

        let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
        let claude_program = resolve_claude(cfg.claude_path.as_deref());
        let claude_version = match &claude_program {
            Some(program) => cli_version(program, &cfg.claude_env).await,
            None => None,
        };
        let claude: Arc<dyn Engine> = match (&claude_program, &bind_error) {
            // Claude runs need the MCP permission endpoint of the local server.
            (_, Some(e)) => Arc::new(UnavailableEngine(server::no_server_text(port, e))),
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

        // From here on a new approval records whether Dots performs its call after a late
        // decision. The hub exists before the engines (the local engine asks it), so it is
        // told now which of them do.
        hub.set_self_apply(
            engines
                .iter()
                .filter(|(_, engine)| engine.applies_approved_calls())
                .map(|(kind, _)| *kind),
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
        let scheduler = Scheduler::new(
            store.clone(),
            runner.clone(),
            bus.clone(),
            cfg.clock.clone(),
        );
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
        Ok(Runtime {
            store,
            bus,
            hub,
            runner,
            scheduler,
            clock: cfg.clock.clone(),
            port,
            server_error,
            claude_program,
            claude_version,
            data_dir: cfg.data_dir.clone(),
            started: settings.clone(),
            settings: Mutex::new(settings),
            chat_lock: Mutex::new(()),
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
        self.change_settings(|s| s.tested_claude_version = version)
            .await?;
        Ok(self.status().await)
    }

    pub async fn settings(&self) -> Settings {
        self.settings.lock().await.clone()
    }

    /// Validates and stores `next`. Port, concurrency and the `claude` path apply after a
    /// restart (`restart_required`); autostart is the app's job. The stored `schedules_paused`
    /// and `tested_claude_version` are kept whatever `next` says: only `set_schedules_paused`
    /// and `mark_claude_tested` change them, so a settings form holding an old copy cannot undo
    /// a pause or a "mark as tested".
    pub async fn update_settings(&self, next: Settings) -> Result<SettingsSaved> {
        let settings = self
            .change_settings(|s| {
                *s = Settings {
                    schedules_paused: s.schedules_paused,
                    tested_claude_version: std::mem::take(&mut s.tested_claude_version),
                    ..next
                };
            })
            .await?;
        Ok(SettingsSaved {
            restart_required: self.started.restart_required(&settings),
            settings,
        })
    }

    pub async fn set_schedules_paused(&self, paused: bool) -> Result<Status> {
        self.change_settings(|s| s.schedules_paused = paused)
            .await?;
        Ok(self.status().await)
    }

    /// One read-modify-write of the settings under their lock, so two changes cannot overwrite
    /// each other. Nothing is stored when the result does not validate.
    async fn change_settings(&self, change: impl FnOnce(&mut Settings) + Send) -> Result<Settings> {
        let mut current = self.settings.lock().await;
        let mut next = current.clone();
        change(&mut next);
        self.store.put_settings(&next).await?;
        self.scheduler.set_paused(next.schedules_paused);
        *current = next.clone();
        Ok(next)
    }

    pub async fn list_dots(&self) -> Result<Vec<Dot>> {
        self.store.list_dots().await
    }

    pub async fn get_dot(&self, id: &str) -> Result<Dot> {
        self.store.get_dot(id).await
    }

    /// Every dot with its active and last run and its next scheduled fire time.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = self.clock.now().with_timezone(&Local);
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
            let next_fire = match (&dot.spec.schedule, dot.spec.enabled) {
                (Some(schedule), true) => next_slot(schedule, &now, &Local)
                    .ok()
                    .flatten()
                    .map(|t| t.to_rfc3339()),
                _ => None,
            };
            rows.push(DotOverview {
                active_run: self.store.latest_run(&dot.id, true).await?,
                last_run: self.store.latest_run(&dot.id, false).await?,
                next_fire,
                dot,
            });
        }
        Ok(rows)
    }

    /// The next `count` (1 to 10) slots of a schedule saved now, RFC 3339 in local time, within
    /// its bounds; empty when it has none. Refuses what `DotSpec::validate` refuses, so the UI
    /// parses no schedule.
    pub fn preview_schedule(schedule: &Schedule, count: usize) -> Result<Vec<String>> {
        schedule.validate()?;
        let now = Local::now();
        let mut schedule = schedule.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        Ok(upcoming(&schedule, &now, &Local)?
            .take(count.clamp(1, 10))
            .map(|t| t.to_rfc3339())
            .collect())
    }

    /// Newest first; `limit` is clamped to 1..=500.
    pub async fn list_runs(&self, dot_id: Option<&str>, limit: i64) -> Result<Vec<Run>> {
        self.store.list_runs(dot_id, limit.clamp(1, 500)).await
    }

    pub async fn get_run(&self, id: &str) -> Result<Run> {
        self.store.get_run(id).await
    }

    /// The whole lineage (root, resumes, chat follow-ups) `run_id` belongs to, oldest first.
    pub async fn run_thread(&self, run_id: &str) -> Result<Vec<Run>> {
        let run = self.store.get_run(run_id).await?;
        self.store.list_thread(&run.root_run_id).await
    }

    pub async fn list_run_events(
        &self,
        run_id: &str,
        after_seq: i64,
    ) -> Result<Vec<RunEventRecord>> {
        self.store.get_run(run_id).await?;
        self.store.list_events(run_id, after_seq).await
    }

    pub async fn approvals_for_run(&self, run_id: &str) -> Result<Vec<Approval>> {
        self.store.get_run(run_id).await?;
        self.store.approvals_for_run(run_id).await
    }

    /// Pending approvals, the finished, unreviewed thread ends that failed or kept a worktree
    /// with changes, and the schedules that need attention.
    pub async fn inbox(&self) -> Result<Inbox> {
        let mut names = std::collections::HashMap::new();
        for dot in self.store.list_dots().await? {
            names.insert(dot.id, dot.spec.name);
        }
        let name = |dot_id: &str| names.get(dot_id).cloned().unwrap_or_default();
        let mut approvals = Vec::new();
        for approval in self.store.list_pending_approvals().await? {
            let run = self.store.get_run(&approval.run_id).await?;
            approvals.push(PendingApproval {
                dot_name: name(&run.dot_id),
                dot_id: run.dot_id,
                approval,
            });
        }
        let review_runs = self
            .store
            .unreviewed_thread_ends()
            .await?
            .into_iter()
            .filter_map(|run| {
                let worktree_exists = run.branch.is_some()
                    && run
                        .workspace_path
                        .as_deref()
                        .is_some_and(|p| Path::new(p).is_dir());
                (run.status == RunStatus::Failed || worktree_exists).then(|| ReviewRun {
                    dot_name: name(&run.dot_id),
                    run,
                    worktree_exists,
                })
            })
            .collect();
        Ok(Inbox {
            approvals,
            review_runs,
            attention: self.store.list_schedule_attention().await?,
        })
    }

    pub async fn inbox_count(&self) -> Result<usize> {
        let inbox = self.inbox().await?;
        Ok(inbox.approvals.len() + inbox.review_runs.len() + inbox.attention.len())
    }

    /// Clears the dot's "Needs attention" (Scheduler v2 spec §8); the schedule is not
    /// otherwise changed. `NotFound` when the dot has no schedule row; nothing happens when
    /// the flag is not set.
    pub async fn dismiss_schedule_attention(&self, dot_id: &str) -> Result<()> {
        let Some(state) = self.store.get_schedule_state(dot_id).await? else {
            return Err(Error::NotFound(format!("schedule of dot {dot_id}")));
        };
        if state.attention_run_id.is_none() {
            return Ok(());
        }
        let change = ScheduleChange {
            attention_run_id: Some(None),
            ..Default::default()
        };
        if self
            .store
            .update_schedule_state(dot_id, &change, self.clock.now())
            .await?
            .is_some()
        {
            self.schedule_updated(dot_id);
        }
        Ok(())
    }

    /// Takes the thread of `run_id` out of the Inbox. Refused while the thread is active.
    pub async fn mark_reviewed(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        self.ensure_thread_idle(&run.root_run_id).await?;
        for changed in self.store.mark_thread_reviewed(&run.root_run_id).await? {
            let _ = self.bus.send(RuntimeEvent::RunUpdated { run: changed });
        }
        self.store.get_run(run_id).await
    }

    /// The worktree a run row describes, for the review actions. The row is the only thing
    /// between the database and git commands in the user's repository, so before any of them
    /// it must name a worktree dots made (`WorkspaceManager::check_owned`: path, branch and
    /// base commit). `Invalid` for a folder run and for a row that fails the check.
    fn review_worktree(&self, run: &Run) -> Result<(WorkspaceManager, Prepared)> {
        let Some(prepared) = Prepared::from_run(run).filter(|p| p.branch.is_some()) else {
            return Err(Error::Invalid(format!("run {} has no worktree", run.id)));
        };
        let workspaces = WorkspaceManager::new(self.data_dir.join("worktrees"));
        workspaces.check_owned(&prepared)?;
        Ok((workspaces, prepared))
    }

    /// The changes the run's thread made in its worktree. Empty, with `exists` false, once the
    /// worktree is removed. Refused for folder runs and for any row that does not name a
    /// worktree of dots, whether or not its folder exists.
    pub async fn worktree_diff(&self, run_id: &str) -> Result<WorktreeDiff> {
        let run = self.store.get_run(run_id).await?;
        let (_, prepared) = self.review_worktree(&run)?;
        let Some(base) = &prepared.base_commit else {
            return Err(Error::Invalid(format!("run {run_id} has no worktree")));
        };
        if !prepared.path.is_dir() {
            return Ok(WorktreeDiff::default());
        }
        crate::workspace::diff(&prepared.path, base).await
    }

    /// Removes the thread's worktree and branch and marks the thread reviewed. Refused while
    /// the thread is active, for folder runs, and for any row that does not name a worktree of
    /// dots.
    pub async fn discard_worktree(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        self.ensure_thread_idle(&run.root_run_id).await?;
        let (workspaces, prepared) = self.review_worktree(&run)?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        workspaces
            .discard(Path::new(&dot.spec.workdir), &prepared)
            .await?;
        self.mark_reviewed(run_id).await
    }

    /// The folder a run worked in, for "Open folder".
    pub async fn run_folder(&self, run_id: &str) -> Result<PathBuf> {
        let run = self.store.get_run(run_id).await?;
        let Some(path) = run.workspace_path.map(PathBuf::from) else {
            return Err(Error::Invalid(format!("run {run_id} has not started yet")));
        };
        if !path.is_dir() {
            return Err(Error::NotFound(format!(
                "folder {} no longer exists",
                path.display()
            )));
        }
        Ok(path)
    }

    async fn ensure_thread_idle(&self, root_run_id: &str) -> Result<()> {
        if let Some(r) = self
            .store
            .list_thread(root_run_id)
            .await?
            .into_iter()
            .find(|r| !r.status.is_terminal())
        {
            return Err(Error::Conflict(format!(
                "run {} of this thread is still {}",
                r.id,
                r.status.as_str()
            )));
        }
        Ok(())
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

    /// The dot's trigger URL. `Conflict` while the local server is not running: the port then
    /// belongs to another program, and a webhook sent there would hand it the dot's token.
    pub fn webhook_url(&self, dot: &Dot) -> Result<String> {
        if let Some(reason) = &self.server_error {
            return Err(Error::Conflict(format!(
                "webhooks are off because the local server is not running: {reason}"
            )));
        }
        Ok(format!(
            "http://127.0.0.1:{}/dots/{}/trigger",
            self.port, dot.id
        ))
    }

    /// Brings the dot's schedule row in line with the dot after a change of it (Scheduler v2
    /// spec §4): made for a new schedule, reset for a changed one or when `enabled_now` (the
    /// dot was disabled and is enabled now), deleted without a schedule. A change of the row
    /// is published as `ScheduleUpdated`.
    async fn sync_schedule(&self, dot: &Dot, enabled_now: bool) -> Result<()> {
        let schedule = dot.spec.schedule.as_ref();
        if self
            .store
            .sync_schedule_state(&dot.id, schedule, enabled_now, self.clock.now())
            .await?
        {
            self.schedule_updated(&dot.id);
        }
        Ok(())
    }

    fn schedule_updated(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::ScheduleUpdated {
            dot_id: dot_id.to_string(),
        });
    }

    pub async fn create_dot(&self, spec: DotSpec) -> Result<Dot> {
        let dot = self.store.create_dot(&spec).await?;
        self.sync_schedule(&dot, false).await?;
        self.scheduler.reload();
        Ok(dot)
    }

    pub async fn update_dot(&self, id: &str, spec: DotSpec) -> Result<Dot> {
        let was_enabled = self.store.get_dot(id).await?.spec.enabled;
        let dot = self.store.update_dot(id, &spec).await?;
        self.sync_schedule(&dot, dot.spec.enabled && !was_enabled)
            .await?;
        self.scheduler.reload();
        Ok(dot)
    }

    /// Deletes the dot; its schedule row goes with it (`ON DELETE CASCADE`).
    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        let had_row = self.store.get_schedule_state(id).await?.is_some();
        self.store.delete_dot_if_idle(id).await?;
        if had_row {
            self.schedule_updated(id);
        }
        self.scheduler.reload();
        Ok(())
    }

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
        let was_enabled = self.store.get_dot(id).await?.spec.enabled;
        let dot = self.store.set_dot_enabled(id, enabled).await?;
        self.sync_schedule(&dot, enabled && !was_enabled).await?;
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
        // Held until the new run is stored: of two follow-ups sent at once, the second must
        // find the first one's run in the thread.
        let _one_at_a_time = self.chat_lock.lock().await;
        if let Some(pid) = parent_run_id {
            let named = self.store.get_run(pid).await?;
            if named.dot_id != dot_id {
                return Err(Error::Invalid(format!(
                    "run {pid} belongs to a different dot"
                )));
            }
            // A thread is one line of runs: a follow-up continues from its end, and only once
            // every run of it has ended. Anything else would give one run two children, each
            // resuming the same session in the same folder. The UI keeps its composer locked
            // until then, so these refusals answer a stale screen or a second window; their
            // text is shown to the user as it is.
            let thread = self.store.list_thread(&named.root_run_id).await?;
            if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
                return Err(Error::Conflict(
                    match active.status {
                        RunStatus::AwaitingApproval => {
                            "this chat is waiting for an approval; answer it, then send your \
                             message again"
                        }
                        _ => {
                            "the dot is still working on this chat; wait until it has \
                             finished, then send your message again"
                        }
                    }
                    .into(),
                ));
            }
            // The parent is taken from this reading of the thread, in which it has ended.
            let Some(parent) = thread.last().filter(|r| r.id == pid) else {
                return Err(Error::Conflict(
                    "this chat has a newer message than the one you replied to; open the chat \
                     again, then send your message"
                        .into(),
                ));
            };
            if parent.session_id.is_none() {
                return Err(Error::Conflict(
                    "this chat cannot be continued because its last run ended before the dot \
                     started working; start a new chat"
                        .into(),
                ));
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

    /// The allow rules a person may save while approving this call, narrowest first. Empty when
    /// the call can only be approved one at a time.
    pub async fn approval_rule_candidates(&self, approval_id: &str) -> Result<Vec<RuleCandidate>> {
        let approval = self.store.get_approval(approval_id).await?;
        let run = self.store.get_run(&approval.run_id).await?;
        let workspace = run.workspace_path.as_deref().map(Path::new);
        Ok(allow_candidates(&approval.tool, &approval.input, workspace))
    }

    /// Decides an approval. With `allow_rule`, an approval also saves that allow rule to the
    /// dot's policy. The pattern must be one that `approval_rule_candidates` offers for this
    /// approval: whoever calls this cannot widen the dot's policy beyond what Dots derived
    /// from the call a person is looking at. Anything else is `Invalid` and nothing is decided.
    pub async fn decide_approval(
        &self,
        id: &str,
        approved: bool,
        note: Option<String>,
        allow_rule: Option<String>,
    ) -> Result<Approval> {
        let rule = match allow_rule {
            None => None,
            Some(_) if !approved => {
                return Err(Error::Invalid(
                    "a rule can only be saved with an approval".into(),
                ))
            }
            Some(pattern) => {
                let offered = self.approval_rule_candidates(id).await?;
                if !offered.iter().any(|c| c.pattern == pattern) {
                    return Err(Error::Invalid(format!(
                        "{pattern:?} is not one of the rules offered for approval {id}"
                    )));
                }
                Some(Rule::new(&pattern, Action::Allow))
            }
        };
        match self.hub.decide_with_rule(id, approved, note, rule).await? {
            DecideEffect::Live(a) => Ok(a),
            DecideEffect::Parked(a) => {
                if let Err(e) = self.runner.maybe_resume(&a.run_id).await {
                    tracing::error!(run = %a.run_id, "resume after approval failed: {e}");
                }
                Ok(a)
            }
        }
    }

    /// Gives every pending approval of one run the same answer, in the order they were asked,
    /// and resumes the run once. An approval that is no longer pending when its turn comes is
    /// left out of the result. No rule can be saved this way.
    pub async fn decide_run_approvals(
        &self,
        run_id: &str,
        approved: bool,
        note: Option<String>,
    ) -> Result<Vec<Approval>> {
        self.store.get_run(run_id).await?;
        let mut decided = Vec::new();
        let mut parked = false;
        let mut failure = None;
        for pending in self.store.pending_for_run(run_id).await? {
            match self.hub.decide(&pending.id, approved, note.clone()).await {
                Ok(DecideEffect::Live(a)) => decided.push(a),
                Ok(DecideEffect::Parked(a)) => {
                    parked = true;
                    decided.push(a);
                }
                Err(Error::Conflict(_)) => {}
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        // What was decided stands, also when a later decision failed: the run must not wait
        // for a resume that nobody triggers.
        if parked {
            if let Err(e) = self.runner.maybe_resume(run_id).await {
                tracing::error!(run = %run_id, "resume after approvals failed: {e}");
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(decided),
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
