use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use crate::accounts::edit::RosterEdit;
use crate::accounts::{Access, Permission, Role};
use crate::approvals::{ApprovalHub, DecideEffect};
use crate::checkins::CheckIns;
use crate::engine::claude::args::resolve_claude;
use crate::engine::claude::{cli_version, ClaudeEngine};
use crate::engine::local::client::DEFAULT_RETRY_DELAYS;
use crate::engine::local::{self, LocalEngine};
use crate::engine::{Engine, UnavailableEngine};
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::followups::FollowUps;
use crate::library_service::LibraryService;
use crate::model::{
    Approval, AttentionRun, ClaudeStatus, Dot, DotOverview, DotSpec, EngineKind, Inbox,
    MessageKind, NewRun, PendingApproval, ReviewRun, Run, RunEventRecord, RunStatus,
    ScheduleChange, ScheduleView, SettingsSaved, Status, TriggerKind, WorktreeDiff,
};
use crate::policy::{allow_candidates, Action, Rule, RuleCandidate};
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{upcoming, Schedule};
use crate::scheduler::{Clock, Scheduler, SystemClock};
use crate::server::{self, ServerState};
use crate::settings::Settings;
use crate::skill_creator::SkillDrafts;
use crate::store::Store;
use crate::vault::org::{
    http_client, os_protector, CreatedInvite, EditOutcome, JoinPreview, MembersView, OrgStatus,
    OrgSummary, OrgTimers, Orgs, RequestPreview, SourceCreds, OFFLINE_DIR,
};
use crate::vault::{os_secrets, GrantStatus, MemorySecrets, SecretStore, Vault};
use crate::workspace::{Prepared, WorkspaceManager};
use crate::{Error, Result};

/// The data folder's folder of the skills the skill creator saves (skill creator spec §6).
pub const SKILLS_DIR: &str = "skills";

/// The data folder's folder where each skill draft's CLI starts (skill creator spec §5).
pub const DRAFTS_DIR: &str = "drafts";

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
    /// How long a skill draft may take (skill creator spec §5). A test shortens it.
    pub draft_limit: Duration,
    /// Where secret values are kept (env vault spec §5.2). `None`: the signed-in user's
    /// Credential Manager under this data folder's install id, which is what the app uses.
    /// `Config::new` keeps them in memory, so a test never writes to Credential Manager.
    pub secrets: Option<Arc<dyn SecretStore>>,
    /// Where email and GitHub watchers read (watchers spec §4). Tests point them at fakes.
    pub watcher_sources: crate::watchers::Sources,
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
            draft_limit: crate::skill_creator::DRAFT_LIMIT,
            secrets: Some(Arc::new(MemorySecrets::new())),
            watcher_sources: crate::watchers::Sources::default(),
        }
    }

    /// The start configuration the app derives from stored settings.
    pub fn from_settings(data_dir: impl Into<PathBuf>, settings: &Settings) -> Self {
        let mut cfg = Self::new(data_dir);
        cfg.port = settings.port;
        cfg.max_concurrent_runs = settings.max_concurrent_runs;
        cfg.claude_path = settings.claude_path.as_ref().map(PathBuf::from);
        cfg.secrets = None;
        cfg
    }
}

pub struct Runtime {
    store: Store,
    bus: Bus,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    scheduler: Arc<Scheduler>,
    watchers: Arc<crate::watchers::Watchers>,
    library: LibraryService,
    vault: Vault,
    skill_drafts: SkillDrafts,
    clock: Arc<dyn Clock>,
    port: u16,
    server_error: Option<String>,
    claude_program: Option<PathBuf>,
    claude_version: Option<String>,
    data_dir: PathBuf,
    /// The stored settings when the runtime started, and now.
    started: Settings,
    settings: Mutex<Settings>,
    /// Makes `chat` check a thread and add to it in one step. It is the follow-ups' thread lock
    /// (`FollowUps::thread_lock`), which `reply` and the follow-up loop hold too, so no two of
    /// them give one run two children.
    chat_lock: Arc<Mutex<()>>,
    checkins: CheckIns,
    follow_ups: Arc<FollowUps>,
    /// The dots' browsers (browser spec section 2).
    browsers: Arc<crate::browser::Browsers>,
    /// The dot whose conversation the person has open in the focused window (needs-you spec
    /// section 3).
    viewing: Viewing,
    /// The installed apps (app library spec).
    apps: crate::apps::Apps,
    /// Teams, their projects and workflows (teams spec section 11).
    teams: crate::teams::Teams,
    /// The workflow runs and their loop (teams spec sections 5 to 8).
    workflows: Arc<crate::workflows::Workflows>,
    /// The orgs joined on this PC (env vault spec §6), the vault's, and their timers.
    orgs: Orgs,
    org_timers: OrgTimers,
    /// Claude's usage limits (usage and limits spec section 4).
    limits: Arc<crate::limits::Limits>,
    shutdown: CancellationToken,
}

impl Runtime {
    pub async fn start(cfg: Config) -> Result<Runtime> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let store = Store::open(&cfg.data_dir.join(crate::store::DB_FILE)).await?;
        let settings = store.get_settings().await?;
        let secrets = match &cfg.secrets {
            Some(secrets) => secrets.clone(),
            None => os_secrets(&store.install_id().await?),
        };
        // The Dots ID of this install, made on first start (accounts design §5), before the orgs
        // load: their member lists are read against it (members spec §6.4). A failure is logged
        // and the app starts anyway; the next start tries again.
        if let Err(e) = crate::accounts::ensure_identity(&store, secrets.as_ref()).await {
            tracing::warn!("making the Dots ID failed: {e}");
        }
        // The orgs joined on this PC, each with its offline copy if it has one, and every org
        // secret in the redactor (env vault spec §6.4). They are fetched in the background
        // once the runtime is up.
        let orgs = Orgs::new(
            store.clone(),
            secrets.clone(),
            os_protector(),
            cfg.data_dir.join(OFFLINE_DIR),
            http_client(),
            cfg.clock.clone(),
        );
        orgs.load().await?;
        // Every Personal secret is in the redactor before any run or Library call (env vault
        // spec §10).
        let vault = Vault::open(store.clone(), secrets.clone())
            .await?
            .with_orgs(orgs.clone());
        // A setup from before the vault: its stored secret values move into Personal (env
        // vault spec §11). A failure is logged and the app starts anyway. The old-backups
        // cut-off is set once the move has run, even when a server failed: that server is
        // logged by name and tried again at the next start.
        match vault.move_stored_literals().await {
            Ok(n) => {
                if n > 0 {
                    tracing::info!("moved the secret values of {n} MCP server(s) to the vault");
                }
                vault.mark_vault_since().await?;
            }
            Err(e) => tracing::error!("moving stored secret values to the vault failed: {e}"),
        }
        // Migration 0013, here and not in `Store::open`: it needs the vault open, since an
        // origin is computed from Personal's values, which live in the secret store. (The move
        // above rewrites only env and header values, never a URL.) Each http server whose URL
        // reads a variable keeps working where it reaches now (env vault spec §8). A failure is
        // logged, and the next start tries again.
        if let Err(e) = store.finish_0013(&vault).await {
            tracing::error!("confirming the origins of the MCP servers failed: {e}");
        }
        let bus = new_bus();
        let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
        // Check-ins and follow-ups (check-ins spec): made before the engines, which offer the
        // check-in tools, and before the runner, which the follow-up loop starts runs on.
        let follow_ups = FollowUps::new(store.clone(), bus.clone(), cfg.clock.clone());
        let checkins = CheckIns::new(store.clone(), bus.clone(), follow_ups.clone());
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
        // No run is running yet: plugin folders of runs are what a crash left behind.
        crate::run_tools::sweep_plugin_folders(&cfg.data_dir.join("plugins"));
        // No skill is being saved yet: hidden folders are saves a crash cut short.
        crate::skill_creator::sweep_unfinished(&cfg.data_dir.join(SKILLS_DIR));
        // Nor is a draft being written: the drafts' folders are what a crash left behind.
        let _ = std::fs::remove_dir_all(cfg.data_dir.join(DRAFTS_DIR));
        // Every schedule has its row before the scheduler's first pass: a dot upgraded by
        // migration 0005 gets one now, and a row a failed change left stale is reset.
        for dot in store.list_dots().await? {
            store
                .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, cfg.clock.now())
                .await?;
        }

        // The installed apps (app library spec), which runs call as their dots and propose.
        let apps = crate::apps::Apps::new(
            store.clone(),
            bus.clone(),
            cfg.data_dir.join(crate::apps::APPS_DIR),
        )
        .with_clock(cfg.clock.clone())
        .with_orgs(orgs.clone());
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
            Arc::new(
                LocalEngine::with_retry_delays(
                    store.clone(),
                    hub.clone(),
                    cfg.local_retry_delays.clone(),
                )
                .with_checkins(checkins.clone())
                .with_apps(apps.clone()),
            ),
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
                plugins_dir: cfg.data_dir.join("plugins"),
            },
        );
        // Dots' own `dots` MCP server serves the check-in tools to Claude runs.
        runner.set_checkins(checkins.clone());
        runner.set_vault(vault.clone());
        // What watchers staged for their runs; a crash's leftovers go (watchers spec §4.2).
        let watcher_files =
            crate::watchers::files::WatcherFiles::new(cfg.data_dir.join("watchers"));
        watcher_files.sweep(&store).await;
        runner.set_watcher_files(watcher_files);
        // Which dots are acting, for the folder watchers (watchers spec section 4.1).
        let acting = crate::watchers::ActingTracker::new(cfg.clock.clone());
        runner.set_acting(acting.clone());
        // The dots' browsers: Edge on each dot's own profile, Playwright MCP per run (browser
        // spec section 2).
        let browsers = crate::browser::Browsers::new(
            store.clone(),
            Arc::new(crate::browser::host::EdgeHost::new(
                cfg.data_dir.join("browser"),
            )),
            crate::browser::PlaywrightCommand::default(),
            port,
        );
        runner.set_browsers(browsers.clone());
        // Claude's usage limits: the readings kept before, and the person's shares (usage and
        // limits spec section 4). The dispatcher asks them before it starts a queued run.
        let limits = crate::limits::Limits::load(
            store.clone(),
            bus.clone(),
            cfg.clock.clone(),
            (&settings).into(),
        )
        .await?;
        runner.set_limits(limits.clone());
        runner.set_apps(apps.clone());
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
        // Every org is fetched once now, without holding the start up, and then every
        // `refresh_minutes` of its policy (env vault spec §6.4).
        let org_timers = OrgTimers::new(
            orgs.clone(),
            cfg.clock.clone(),
            bus.clone(),
            shutdown.clone(),
        );
        org_timers.start_up();
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
        // The runs apps start are queued as the runner's, and apps hear when a run of a topic
        // they started ends (app library spec section 7).
        let starting = Arc::downgrade(&runner);
        apps.set_starter(move |run| {
            if let Some(runner) = starting.upgrade() {
                runner.enqueued(run);
            }
        });
        crate::apps::wake::spawn_run_ends(apps.clone(), &bus, shutdown.clone());
        let scheduler = Scheduler::new(
            store.clone(),
            runner.clone(),
            bus.clone(),
            cfg.clock.clone(),
        );
        scheduler.set_paused(settings.schedules_paused);
        scheduler.spawn(shutdown.clone());
        // Watchers stop with the schedules (watchers spec section 5).
        let watchers = crate::watchers::Watchers::new(crate::watchers::WatcherDeps {
            store: store.clone(),
            bus: bus.clone(),
            runner: runner.clone(),
            vault: vault.clone(),
            clock: cfg.clock.clone(),
            acting,
            data_dir: cfg.data_dir.clone(),
            sources: cfg.watcher_sources.clone(),
        });
        watchers.set_paused(settings.schedules_paused);
        watchers.spawn(shutdown.clone());
        // Follow-ups wait while schedules are paused (check-ins spec section 6).
        follow_ups.set_paused(settings.schedules_paused);
        follow_ups.spawn(runner.clone(), shutdown.clone());
        let viewing = Viewing::default();
        let teams = crate::teams::Teams::new(store.clone(), bus.clone()).with_orgs(orgs.clone());
        // Started after the dispatcher and after interrupted runs were failed, so its first
        // pass sees each workflow's runs as they are.
        let workflows = crate::workflows::Workflows::new(
            store.clone(),
            bus.clone(),
            runner.clone(),
            cfg.clock.clone(),
        );
        workflows.set_orgs(orgs.clone());
        workflows.spawn(shutdown.clone());
        // The workflow tools reach the workflow runs through the check-ins (teams spec
        // section 7).
        checkins.set_workflows(&workflows);
        watch_viewing(
            store.clone(),
            bus.clone(),
            viewing.clone(),
            shutdown.clone(),
        );
        Ok(Runtime {
            library: LibraryService::new(store.clone(), bus.clone())
                .with_skills_dir(cfg.data_dir.join(SKILLS_DIR))
                .with_vault(vault.clone()),
            vault,
            store,
            bus,
            hub,
            runner,
            scheduler,
            watchers,
            clock: cfg.clock.clone(),
            port,
            server_error,
            // Drafts need the CLI, not the local server: they work while its port is taken.
            skill_drafts: SkillDrafts::new(
                claude_program.clone(),
                cfg.claude_env.clone(),
                cfg.data_dir.join(DRAFTS_DIR),
                cfg.draft_limit,
                shutdown.clone(),
            ),
            claude_program,
            claude_version,
            data_dir: cfg.data_dir.clone(),
            started: settings.clone(),
            settings: Mutex::new(settings),
            chat_lock: follow_ups.thread_lock(),
            checkins,
            follow_ups,
            browsers,
            viewing,
            apps,
            teams,
            workflows,
            orgs,
            org_timers,
            limits,
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
        self.watchers.set_paused(next.schedules_paused);
        self.limits.set_shares((&next).into());
        self.runner.nudge();
        self.follow_ups.set_paused(next.schedules_paused);
        *current = next.clone();
        Ok(next)
    }

    pub async fn list_dots(&self) -> Result<Vec<Dot>> {
        self.store.list_dots().await
    }

    pub async fn get_dot(&self, id: &str) -> Result<Dot> {
        self.store.get_dot(id).await
    }

    /// The status of each of the dot's stored grants in its scope (the editor's Variables),
    /// as a run's start would resolve them now, without its fetch of a stale org. It holds no
    /// value (`Vault::grant_statuses`).
    pub async fn dot_variables(&self, id: &str) -> Result<Vec<GrantStatus>> {
        let dot = self.store.get_dot(id).await?;
        self.vault.grant_statuses(&dot).await
    }

    /// Every dot with its active and last run, where its schedule stands, and what it has for
    /// the person (needs-you spec section 3). A dot deleted meanwhile is left out.
    pub async fn dot_overview(&self) -> Result<Vec<DotOverview>> {
        let now = self.clock.now();
        let mut rows = Vec::new();
        for dot in self.store.list_dots().await? {
            let (needs_you, unread) = match self.store.dot_counts(&dot.id).await {
                Ok(counts) => counts,
                Err(Error::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            rows.push(DotOverview {
                active_run: self.store.latest_run(&dot.id, true).await?,
                last_run: self.store.latest_run(&dot.id, false).await?,
                schedule_state: self.schedule_view(&dot, now).await?,
                needs_you,
                unread,
                dot,
            });
        }
        Ok(rows)
    }

    /// The view of the dot's schedule row in local time; `None` without a schedule or a row.
    async fn schedule_view(&self, dot: &Dot, now: DateTime<Utc>) -> Result<Option<ScheduleView>> {
        let Some(schedule) = &dot.spec.schedule else {
            return Ok(None);
        };
        let Some(state) = self.store.get_schedule_state(&dot.id).await? else {
            return Ok(None);
        };
        let attention = match &state.attention_run_id {
            Some(id) => match self.store.get_run(id).await {
                Ok(run) => Some(AttentionRun {
                    run_id: run.id,
                    error: run.error,
                }),
                // Deleted since the row was read: its flag went with it.
                Err(Error::NotFound(_)) => None,
                Err(e) => return Err(e),
            },
            None => None,
        };
        Ok(Some(ScheduleView::new(
            &state,
            schedule,
            dot.spec.enabled,
            attention,
            now,
            &Local,
        )))
    }

    /// The next `count` (1 to 10) slots of a schedule saved now, RFC 3339 in local time, within
    /// its bounds. Refuses what `DotSpec::validate` refuses, and what a create or a change of
    /// the schedule refuses ("this schedule would never run": no slot is left), so the UI
    /// parses no schedule and can say why before a save. An `Every` sent with its anchor keeps
    /// its phase; one without starts now.
    pub fn preview_schedule(schedule: &Schedule, count: usize) -> Result<Vec<String>> {
        schedule.validate()?;
        let now = Local::now();
        let mut schedule = schedule.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        schedule.check_runs_after(&now, &Local)?;
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
    /// with changes, the schedules that need attention, and the MCP servers whose tool list
    /// waits for review.
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
        let app_reviews = self
            .store
            .list_pending_app_proposals()
            .await?
            .into_iter()
            .map(|proposal| crate::model::PendingAppReview {
                dot_name: name(&proposal.dot_id),
                dot_id: proposal.dot_id.clone(),
                proposal,
            })
            .collect();
        Ok(Inbox {
            approvals,
            review_runs,
            attention: self.store.list_schedule_attention().await?,
            tools_changed: self.library.tools_changed().await?,
            watchers: self.store.paused_watchers().await?,
            held: self.held_runs(&names).await?,
            app_reviews,
            team_scope: self.store.team_scope_issues().await?,
        })
    }

    pub async fn inbox_count(&self) -> Result<usize> {
        let inbox = self.inbox().await?;
        // Open questions and unread notes wait too (check-ins spec section 9).
        Ok(inbox.approvals.len()
            + inbox.review_runs.len()
            + inbox.attention.len()
            + inbox.tools_changed.len()
            + inbox.watchers.len()
            // Held runs wait as one line, however many (usage and limits spec section 5).
            + usize::from(!inbox.held.is_empty())
            // A waiting review card counts like an approval (app library spec section 10).
            + inbox.app_reviews.len()
            // A team with a member outside its account or workspace (accounts design §6.3).
            + inbox.team_scope.len()
            + self.store.count_waiting_messages().await?)
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
        self.require_for_run(run_id, Permission::RunDots).await?;
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

    /// The Library: what the `library_*` commands call (MCP/skills spec section 9).
    pub fn library(&self) -> &LibraryService {
        &self.library
    }

    /// The installed apps (app library spec).
    pub fn apps(&self) -> &crate::apps::Apps {
        &self.apps
    }

    /// The personal variables of the Env tab (env vault spec §5).
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// The Dots ID of this install.
    pub async fn identity(&self) -> Result<crate::accounts::Identity> {
        crate::accounts::identity(&self.store).await
    }

    /// Sets the name and email of the Dots ID.
    pub async fn set_profile(&self, name: &str, email: &str) -> Result<crate::accounts::Identity> {
        crate::accounts::set_profile(&self.store, name, email).await
    }

    /// The account and workspace in view; personal when none was chosen or its org is gone.
    pub async fn current_scope(&self) -> Result<crate::accounts::CurrentScope> {
        crate::accounts::current_scope(&self.store, self.orgs()).await
    }

    /// Chooses the account (`personal` or a joined org) and workspace in view.
    pub async fn set_current_scope(
        &self,
        account: &str,
        workspace: &str,
    ) -> Result<crate::accounts::CurrentScope> {
        crate::accounts::set_current_scope(&self.store, self.orgs(), account, workspace).await
    }

    /// The update backups made before this data folder first ran with the vault, until the
    /// offer is answered (env vault spec §11).
    pub async fn old_backups(&self) -> Result<Vec<String>> {
        self.vault.old_backups(&self.data_dir).await
    }

    /// Deletes the old backups when `delete`, and answers the offer. Returns the file names of
    /// the backups that could not be deleted; then the offer stays.
    pub async fn answer_old_backups(&self, delete: bool) -> Result<Vec<String>> {
        self.vault.answer_old_backups(&self.data_dir, delete).await
    }

    /// The skill creator's drafts: what `skill_draft` and `skill_draft_cancel` call (skill
    /// creator spec §5).
    pub fn skill_drafts(&self) -> &SkillDrafts {
        &self.skill_drafts
    }

    /// The dots' notes and questions: what the `message_*` commands call (check-ins spec
    /// section 10). Answering a question is `reply`.
    pub fn checkins(&self) -> &CheckIns {
        &self.checkins
    }

    /// Teams, their projects and workflows.
    pub fn teams(&self) -> &crate::teams::Teams {
        &self.teams
    }

    /// The workflow runs: start, approve a check, retry a step, stop.
    pub fn workflows(&self) -> &Arc<crate::workflows::Workflows> {
        &self.workflows
    }

    /// The follow-ups dots scheduled: what the `follow_up*` commands call.
    pub fn follow_ups(&self) -> &Arc<FollowUps> {
        &self.follow_ups
    }

    /// Answers the open question `message_id` with `text` (check-ins spec section 5): a run
    /// continues the question's thread, or starts afresh when its end has no session.
    /// In an account, the role must allow answering for its dots (members spec §6.3).
    pub async fn reply(&self, message_id: &str, text: &str) -> Result<Run> {
        let question = self.store.get_message(message_id).await?;
        self.require_for_dot(&question.dot_id, Permission::AnswerApprovals)
            .await?;
        self.checkins.reply(&self.runner, message_id, text).await
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

    /// The address `/v1` answers on, for the API keys tab to show: this PC's own port. A
    /// Conflict when the local server is not running, like `webhook_url`.
    pub fn api_base_url(&self) -> Result<String> {
        if let Some(reason) = &self.server_error {
            return Err(Error::Conflict(format!(
                "the API is off because the local server is not running: {reason}"
            )));
        }
        Ok(format!("http://127.0.0.1:{}/v1", self.port))
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

    /// Creates a dot, in the org `org_id` or, with `None`, in Personal, and in its workspace
    /// `workspace_id` or, with `None`, Default. The org must be joined on this PC; it is fixed
    /// from then on (env vault spec §5.2). In an account, the role in that workspace must allow
    /// changing dots, and giving it secrets when it is granted any (members spec §6.3).
    pub async fn create_dot(
        &self,
        spec: DotSpec,
        org_id: Option<String>,
        workspace_id: Option<String>,
    ) -> Result<Dot> {
        if let Some(org) = &org_id {
            if self.store.get_org(org).await?.is_none() {
                return Err(Error::Invalid(format!(
                    "no org '{org}' is joined on this PC"
                )));
            }
        }
        let workspace_id =
            workspace_id.unwrap_or_else(|| crate::accounts::DEFAULT_WORKSPACE.into());
        self.check_workspace(org_id.as_deref(), &workspace_id)
            .await?;
        let workspace_id = workspace_id.as_str();
        self.require_in(org_id.as_deref(), workspace_id, Permission::EditDots)
            .await?;
        if !spec.variables.0.is_empty() {
            self.require_in(org_id.as_deref(), workspace_id, Permission::GrantEntries)
                .await?;
        }
        let dot = self
            .store
            .create_dot(&spec, org_id.as_deref(), workspace_id)
            .await?;
        self.sync_schedule(&dot, false).await?;
        self.scheduler.reload();
        self.watchers.reload();
        Ok(dot)
    }

    /// Refuses a workspace the account (`None` is Personal) doesn't have.
    async fn check_workspace(&self, org_id: Option<&str>, workspace_id: &str) -> Result<()> {
        let scope = org_id.unwrap_or(crate::accounts::PERSONAL);
        if self.store.workspace(scope, workspace_id).await?.is_some() {
            return Ok(());
        }
        Err(Error::NotFound(format!(
            "No workspace '{workspace_id}' in {}.",
            crate::accounts::account_label(Some(&self.orgs), org_id).await
        )))
    }

    /// Moves a dot to another workspace of its account. The role must allow changing dots in
    /// the workspace it leaves and in the one it goes to, and a dot in a team stays with the
    /// team (accounts design §6.3).
    pub async fn move_dot(&self, dot_id: &str, workspace_id: &str) -> Result<Dot> {
        let dot = self.store.get_dot(dot_id).await?;
        self.check_workspace(dot.org_id.as_deref(), workspace_id)
            .await?;
        self.require_dot(&dot, Permission::EditDots).await?;
        self.require_in(dot.org_id.as_deref(), workspace_id, Permission::EditDots)
            .await?;
        if let Some(team_id) = self.store.teams_of_dot(dot_id).await?.first() {
            let team = self.store.get_team(team_id).await?;
            return Err(Error::Invalid(format!(
                "{} is in the team {}. Take it out of the team first.",
                dot.spec.name, team.name
            )));
        }
        self.store.set_dot_workspace(dot_id, workspace_id).await?;
        self.store.get_dot(dot_id).await
    }

    /// The workspaces of `scope` (`personal` or an org id) this PC may see.
    pub async fn workspaces(&self, scope: &str) -> Result<Vec<crate::accounts::WorkspaceView>> {
        crate::accounts::visible_workspaces(&self.store, &self.orgs, scope).await
    }

    /// What this PC may not do with the dot `dot_id`: change it, run it, answer its approvals,
    /// each in the words `require_dot` refuses it with (its account and workspace); `None` is
    /// allowed. The UI switches its controls off by these (members spec §9).
    pub async fn dot_access(&self, dot_id: &str) -> Result<crate::accounts::DotAccess> {
        let dot = self.store.get_dot(dot_id).await?;
        crate::accounts::refusals_in(Some(&self.orgs), dot.org_id.as_deref(), &dot.workspace_id)
            .await
    }

    /// Adds a workspace to Personal; its id is made from the name.
    pub async fn personal_workspace_add(
        &self,
        name: &str,
    ) -> Result<crate::accounts::WorkspaceView> {
        let name = personal_workspace_name(name)?;
        let taken: Vec<String> = self
            .store
            .list_workspaces(crate::accounts::PERSONAL)
            .await?
            .into_iter()
            .map(|w| w.id)
            .collect();
        let id = crate::accounts::workspace_id_for(&name, &taken);
        self.store
            .add_workspace(&crate::store::WorkspaceRow {
                scope: crate::accounts::PERSONAL.into(),
                id: id.clone(),
                name,
                use_rules: None,
                created_at: crate::util::now(),
            })
            .await?;
        self.workspaces(crate::accounts::PERSONAL)
            .await?
            .into_iter()
            .find(|w| w.id == id)
            .ok_or_else(|| Error::NotFound(format!("workspace {id}")))
    }

    /// Renames a Personal workspace; its id stays.
    pub async fn personal_workspace_rename(&self, id: &str, name: &str) -> Result<()> {
        let name = personal_workspace_name(name)?;
        self.store
            .rename_workspace(crate::accounts::PERSONAL, id, &name)
            .await
    }

    /// Deletes a Personal workspace; its dots and teams go to Default.
    pub async fn personal_workspace_delete(&self, id: &str) -> Result<()> {
        self.store
            .delete_workspace(crate::accounts::PERSONAL, id)
            .await
    }

    /// The API keys of the workspaces of `scope` (`personal` or an org id) this PC may see,
    /// newest first. Each says why this PC may not manage it, if it may not. Never carries a
    /// key's text.
    pub async fn api_keys(
        &self,
        scope: &str,
    ) -> Result<Vec<crate::accounts::api_keys::ApiKeyView>> {
        let visible = self.workspaces(scope).await?;
        Ok(self
            .store
            .api_keys(scope)
            .await?
            .into_iter()
            .filter_map(|row| {
                let ws = visible.iter().find(|w| w.id == row.workspace_id)?;
                let (name, refusal) = (ws.name.clone(), ws.refusals.create_api_keys.clone());
                Some(crate::accounts::api_keys::ApiKeyView::new(
                    row, name, refusal,
                ))
            })
            .collect())
    }

    /// Makes a key for the workspace `workspace_id` of `scope` (accounts spec §10). The role
    /// must allow creating API keys there. The key's text is in the answer and nowhere else:
    /// only its prefix and hash are kept.
    pub async fn create_api_key(
        &self,
        scope: &str,
        workspace_id: &str,
        name: &str,
    ) -> Result<crate::accounts::api_keys::CreatedApiKey> {
        use crate::accounts::api_keys::{key_prefix, new_api_key, ApiKeyView, CreatedApiKey};
        let org_id = (scope != crate::accounts::PERSONAL).then_some(scope);
        self.check_workspace(org_id, workspace_id).await?;
        self.require_in(org_id, workspace_id, Permission::CreateApiKeys)
            .await?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(Error::Invalid(
                "Give the key a name (up to 64 characters).".into(),
            ));
        }
        let secret = new_api_key();
        let row = crate::store::ApiKeyRow {
            id: crate::util::new_id(),
            scope: scope.to_string(),
            workspace_id: workspace_id.to_string(),
            name: name.to_string(),
            prefix: key_prefix(&secret),
            hash: crate::util::token_hash(&secret),
            created_at: crate::util::now(),
            last_used_at: None,
            revoked_at: None,
        };
        self.store.add_api_key(&row).await?;
        let key = ApiKeyView::new(row, self.workspace_label(scope, workspace_id).await?, None);
        Ok(CreatedApiKey { key, secret })
    }

    /// Revokes a key: it stops working at once and stays listed as revoked. The role must allow
    /// creating API keys in the key's workspace.
    pub async fn revoke_api_key(&self, id: &str) -> Result<crate::accounts::api_keys::ApiKeyView> {
        let row = self
            .store
            .api_key(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("No API key '{id}'.")))?;
        let org_id = (row.scope != crate::accounts::PERSONAL).then_some(row.scope.as_str());
        self.require_in(org_id, &row.workspace_id, Permission::CreateApiKeys)
            .await?;
        self.store.revoke_api_key(id, &crate::util::now()).await?;
        let row = self
            .store
            .api_key(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("No API key '{id}'.")))?;
        let name = self.workspace_label(&row.scope, &row.workspace_id).await?;
        Ok(crate::accounts::api_keys::ApiKeyView::new(row, name, None))
    }

    /// The workspace's name, or its id when it has none.
    async fn workspace_label(&self, scope: &str, workspace_id: &str) -> Result<String> {
        Ok(self
            .store
            .workspace(scope, workspace_id)
            .await?
            .map_or_else(|| workspace_id.to_string(), |w| w.name))
    }

    /// Saves a dot's spec. A dot whose org is no longer joined on this PC is refused: it can't
    /// run until the org is joined again or the dot is deleted (env vault spec §6.4). In an
    /// account, the role must allow changing dots, and giving secrets to them when the grants
    /// change (members spec §6.3).
    pub async fn update_dot(&self, id: &str, spec: DotSpec) -> Result<Dot> {
        let old = self.store.get_dot(id).await?;
        if let Some(text) = self.vault.org_removed(&old) {
            return Err(Error::Conflict(text));
        }
        self.require_dot(&old, Permission::EditDots).await?;
        if spec.variables != old.spec.variables {
            self.require_dot(&old, Permission::GrantEntries).await?;
        }
        let was_enabled = old.spec.enabled;
        let dot = self.store.update_dot(id, &spec).await?;
        self.sync_schedule(&dot, dot.spec.enabled && !was_enabled)
            .await?;
        self.scheduler.reload();
        self.watchers.reload();
        self.follow_ups.wake();
        Ok(dot)
    }

    /// Deletes the dot; its schedule row goes with it (`ON DELETE CASCADE`). In an account, the
    /// role must allow changing dots (members spec §6.3), or this PC is waiting for or removed
    /// from the member list, and only cleaning up (`require_cleanup`).
    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        let dot = self.store.get_dot(id).await?;
        self.require_cleanup(&dot).await?;
        let had_row = self.store.get_schedule_state(id).await?.is_some();
        let proposals = self.store.list_app_proposals(Some(id)).await?;
        let watchers: Vec<String> = self
            .store
            .list_watchers(id)
            .await?
            .into_iter()
            .map(|w| w.id)
            .collect();
        self.store.delete_dot_if_idle(id).await?;
        // Its proposals' rows went with it; their files go too (app library spec section 6.2).
        for p in &proposals {
            self.apps.drop_proposed(&p.id).await;
        }
        // Its watchers went with it; what they staged and kept goes too.
        self.watchers.forget(&watchers);
        // It can no longer be viewed (needs-you spec section 7).
        {
            let mut viewing = lock(&self.viewing);
            if viewing.as_deref() == Some(id) {
                *viewing = None;
            }
        }
        // Its Edge closes and its profile goes with it (browser spec section 6).
        self.browsers.forget(id).await;
        if had_row {
            self.schedule_updated(id);
        }
        self.scheduler.reload();
        self.watchers.reload();
        self.follow_ups.wake();
        Ok(())
    }

    /// Switches the dot on or off. In an account, the role must allow changing dots (members
    /// spec §6.3); switching one off is cleaning up, which a PC waiting for or removed from the
    /// member list may still do (`require_cleanup`).
    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
        let old = self.store.get_dot(id).await?;
        if enabled {
            self.require_dot(&old, Permission::EditDots).await?;
        } else {
            self.require_cleanup(&old).await?;
        }
        let was_enabled = old.spec.enabled;
        let dot = self.store.set_dot_enabled(id, enabled).await?;
        self.sync_schedule(&dot, enabled && !was_enabled).await?;
        self.scheduler.reload();
        self.watchers.reload();
        self.follow_ups.wake();
        Ok(dot)
    }

    pub async fn regenerate_webhook_token(&self, id: &str) -> Result<String> {
        self.require_for_dot(id, Permission::EditDots).await?;
        let token = self.store.regenerate_webhook_token(id).await?;
        self.scheduler.reload();
        Ok(token)
    }

    pub async fn run_now(&self, dot_id: &str) -> Result<Run> {
        let dot = self.store.get_dot(dot_id).await?;
        self.require_run(&dot).await?;
        self.refuse_at_the_limit(&dot)?;
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
        let dot = self.store.get_dot(dot_id).await?;
        self.require_run(&dot).await?;
        self.refuse_at_the_limit(&dot)?;
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
            if self.follow_ups.deciding(&named.root_run_id) {
                return Err(Error::Conflict(crate::checkins::DECIDING.into()));
            }
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
            // A step's chat ends with the step (teams spec section 8).
            if self.store.in_ended_step_thread(parent).await? {
                return Err(Error::Conflict(crate::workflows::ENDED_STEP_THREAD.into()));
            }
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

    /// The person's answer on an app proposal's card (app library spec section 6.2): "Add to
    /// library" (or "Update") adds the app first, and a failure there leaves the card waiting;
    /// then the proposal is decided and the dot's thread continues with what the person decided,
    /// as an answered question does (`CheckIns::reply`). A thread that has not ended is refused,
    /// as a reply is. Adding runs the app's migrations, which may take long on real data: the
    /// thread lock is let go meanwhile, and the thread is marked as deciding, so no chat, reply
    /// or follow-up continues it and no second decision is made in it.
    pub async fn decide_app_proposal(
        &self,
        id: &str,
        decision: crate::apps::propose::Decision,
    ) -> Result<(crate::apps::propose::AppProposal, Run)> {
        use crate::apps::propose::{Decision, ProposalStatus, DECIDED_ALREADY, DISCARDED};
        // In an account, the role must allow answering for its dots (members spec §6.3).
        let dot_id = self.store.get_app_proposal(id).await?.dot_id;
        self.require_for_dot(&dot_id, Permission::AnswerApprovals)
            .await?;
        let lock = self.follow_ups.thread_lock();
        let mut one_at_a_time = lock.lock().await;
        let p = self.store.get_app_proposal(id).await?;
        if p.status != ProposalStatus::Pending {
            return Err(Error::Conflict(DECIDED_ALREADY.into()));
        }
        let asked = self.store.get_run(&p.run_id).await?;
        if self.follow_ups.deciding(&asked.root_run_id) {
            return Err(Error::Conflict(crate::checkins::ADDING.into()));
        }
        let thread = self.store.list_thread(&asked.root_run_id).await?;
        if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
            return Err(crate::checkins::thread_busy(active.status));
        }
        let end = thread.last().unwrap_or(&asked);
        let (status, answer, said) = match &decision {
            Decision::AskForChanges { text } => {
                let text = text.trim();
                if text.is_empty() {
                    return Err(Error::Invalid("say what to change".into()));
                }
                if text.chars().count() > crate::apps::propose::MAX_NOTE {
                    return Err(Error::Invalid(format!(
                        "say what to change in at most {} characters",
                        crate::apps::propose::MAX_NOTE
                    )));
                }
                let said = format!(
                    "The person asked for changes to the app {}: {text}",
                    p.app_name
                );
                (ProposalStatus::ChangesAsked, Some(text.to_string()), said)
            }
            Decision::Discard => (ProposalStatus::Discarded, None, DISCARDED.to_string()),
            Decision::Add | Decision::AddWithKeptData => {
                let on_kept = decision == Decision::AddWithKeptData;
                let Some(deciding) = self.follow_ups.begin_deciding(&asked.root_run_id) else {
                    return Err(Error::Conflict(crate::checkins::ADDING.into()));
                };
                drop(one_at_a_time);
                let accepted = self.apps.accept(&p, on_kept).await;
                one_at_a_time = lock.lock().await;
                drop(deciding);
                let item = accepted?;
                let version = match &item.config {
                    crate::library::LibraryConfig::App(c) => c.version,
                    _ => 1,
                };
                let said = match p.base_version {
                    Some(_) => format!(
                        "The person accepted your change to the app {}; it is now version {version}.",
                        p.app_name
                    ),
                    None if on_kept => format!(
                        "The person added the app {} to the Library, on the data kept from the \
                         removed one.",
                        p.app_name
                    ),
                    None => format!("The person added the app {} to the Library.", p.app_name),
                };
                (ProposalStatus::Added, None, said)
            }
        };
        let message = if end.session_id.is_some() {
            said
        } else {
            format!(
                "Earlier you sent the app {} for review: \"{}\"\n{said}",
                p.app_name, p.note
            )
        };
        let new = crate::checkins::continuation(&asked, end, message, answer.as_deref());
        let (decided, run) = self
            .store
            .decide_app_proposal(id, status, answer.as_deref(), &new)
            .await?;
        drop(one_at_a_time);
        self.runner.enqueued(&run);
        self.apps.drop_proposed(id).await;
        self.apps.proposal_changed(&decided);
        Ok((decided, run))
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
        let run_id = self.store.get_approval(id).await?.run_id;
        self.require_for_run(&run_id, Permission::AnswerApprovals)
            .await?;
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
        self.require_for_run(run_id, Permission::AnswerApprovals)
            .await?;
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

    /// Holds or releases the run queue (`Runner::set_held`): while an update waits for the
    /// runs that are working, no queued run starts (updates spec §6). Holding returns once no
    /// run can be claimed any more.
    pub async fn hold_runs(&self, held: bool) {
        self.runner.set_held(held).await;
    }

    /// The GitHub token Dots publishes with: the value of the vault entry the settings name
    /// (updates UI spec section 5.1). Never logged or sent in an event.
    pub async fn publish_token(&self) -> Result<Option<String>> {
        let Some(name) = self.settings().await.publish_token else {
            return Ok(None);
        };
        Ok(self
            .vault
            .personal_value(&name)
            .await?
            .map(|v| v.to_string()))
    }

    /// How many runs have not ended (queued, running or waiting for an approval).
    pub async fn unfinished_runs(&self) -> Result<usize> {
        self.store.unfinished_run_count().await
    }

    /// The names of the dots with a run whose status is `running`, sorted: the runs an update
    /// waits for (updates spec §6). Queued runs and runs awaiting an approval survive a
    /// restart, so they are left out. A run is no longer counted once the `RunUpdated` event
    /// of its end is out. A dot deleted between the two reads is left out.
    pub async fn working_dots(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for id in self.store.running_dot_ids().await? {
            match self.store.get_dot(&id).await {
                Ok(dot) => names.push(dot.spec.name),
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        names.sort();
        Ok(names)
    }

    /// Stops the runtime like `shutdown`, then closes the database and waits until every
    /// connection has closed, at most `limit`. True when it closed in time. From then on every
    /// call that reads or writes the database fails (updates spec §7).
    pub async fn close(&self, limit: Duration) -> bool {
        self.shutdown();
        tokio::time::timeout(limit, self.store.close())
            .await
            .is_ok()
    }
}

/// Account roles (members spec §6.3): what this PC may do in Personal or an account.
impl Runtime {
    /// `Ok` when this PC may do `p` in the workspace `workspace_id` of `org_id`, where the
    /// member's role in the workspace narrows the account role (members spec §6.3); otherwise
    /// `Error::NotAllowed` naming the account and the role, or why this PC is not let in. Every
    /// check on a dot or team goes through here (`require_dot`), never the account role alone.
    pub async fn require_in(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
        p: Permission,
    ) -> Result<()> {
        crate::accounts::require_in(Some(&self.orgs), org_id, workspace_id, p).await
    }

    /// `require` on `dot`, in its account and workspace.
    pub async fn require_dot(&self, dot: &Dot, p: Permission) -> Result<()> {
        crate::accounts::require_dot(Some(&self.orgs), dot, p).await
    }

    /// `require_dot` of `EditDots` for cleaning up `dot`: switching it off or deleting it. A PC
    /// waiting for, or removed from, the account's member list may still do that: its dots
    /// can't run, and Leave removes the account. So may a member outside the dot's workspace
    /// (members spec §6.3). Enabling, editing and watcher changes stay refused for them.
    async fn require_cleanup(&self, dot: &Dot) -> Result<()> {
        if matches!(
            crate::accounts::access_dot(Some(&self.orgs), dot),
            Access::Waiting | Access::Removed | Access::OutsideWorkspace { .. }
        ) {
            return Ok(());
        }
        self.require_dot(dot, Permission::EditDots).await
    }

    /// `require_dot` on the dot `dot_id`.
    async fn require_for_dot(&self, dot_id: &str, p: Permission) -> Result<()> {
        let dot = self.store.get_dot(dot_id).await?;
        self.require_dot(&dot, p).await
    }

    /// What a run the person starts of `dot` needs: its org still joined (env vault spec
    /// §6.4), and a role that may run the dots of its account and workspace.
    async fn require_run(&self, dot: &Dot) -> Result<()> {
        if let Some(text) = self.vault.org_removed(dot) {
            return Err(Error::Conflict(text));
        }
        self.require_dot(dot, Permission::RunDots).await
    }

    /// `require_dot` on the dot of the run `run_id`.
    async fn require_for_run(&self, run_id: &str, p: Permission) -> Result<()> {
        let run = self.store.get_run(run_id).await?;
        self.require_for_dot(&run.dot_id, p).await
    }
}

/// Orgs (env vault spec §6.1 and §6.4). The app changes an org through these, not through
/// `Orgs` itself, so that its timer stays in step and the screens hear of it (`OrgUpdated`).
impl Runtime {
    /// The orgs joined on this PC, for what only reads them: the list, a join code's preview,
    /// an org's entries, and a SOPS org's key.
    pub fn orgs(&self) -> &Orgs {
        &self.orgs
    }

    /// The orgs' timers.
    pub fn org_timers(&self) -> &OrgTimers {
        &self.org_timers
    }

    /// What `code` would join, or change for an org joined already. Nothing is fetched.
    pub fn preview_org(&self, code: &str) -> Result<JoinPreview> {
        self.orgs.preview_join(code)
    }

    /// Joins the org of `code` (**Join**), then starts its timer. A SOPS join that waits for
    /// the admin (`Pending`) joins nothing, so it has no timer and no event.
    pub async fn join_org(&self, code: &str, creds: Option<SourceCreds>) -> Result<OrgSummary> {
        let summary = self.orgs.join(code, creds).await?;
        if summary.status != OrgStatus::Pending {
            self.org_timers.start(&summary.id);
            self.org_updated(&summary.id);
        }
        Ok(summary)
    }

    /// The account's member list for the Members tab (members spec section 6).
    pub async fn member_list(&self, org_id: &str) -> Result<MembersView> {
        self.orgs.members(org_id).await
    }

    /// Makes a personal invite for `email` and the join code that carries it.
    pub async fn invite_create(
        &self,
        org_id: &str,
        email: &str,
        role: Role,
    ) -> Result<CreatedInvite> {
        self.orgs.create_invite(org_id, email, role).await
    }

    /// Revokes an invite made on this PC.
    pub async fn invite_revoke(&self, org_id: &str, invite_id: &str) -> Result<()> {
        self.orgs.revoke_invite(org_id, invite_id).await
    }

    /// Makes this waiting PC's request code and keeps it; the org's row changes.
    pub async fn org_request_access(&self, org_id: &str) -> Result<OrgSummary> {
        let summary = self.orgs.request_access(org_id).await?;
        self.org_updated(org_id);
        Ok(summary)
    }

    /// What a request code asks, checked against this PC's invites and the member list.
    pub async fn request_inspect(&self, org_id: &str, code: &str) -> Result<RequestPreview> {
        self.orgs.inspect_request(org_id, code).await
    }

    /// Makes one change to the member list, published or kept as a draft.
    pub async fn member_edit(
        &self,
        org_id: &str,
        base_seq: u64,
        edit: RosterEdit,
    ) -> Result<EditOutcome> {
        let outcome = self.orgs.edit_members(org_id, base_seq, edit).await?;
        self.org_updated(org_id);
        Ok(outcome)
    }

    /// Publishes the org's waiting signed list again.
    pub async fn roster_draft_retry(&self, org_id: &str) -> Result<EditOutcome> {
        let outcome = self.orgs.retry_draft(org_id).await?;
        self.org_updated(org_id);
        Ok(outcome)
    }

    /// Drops the org's waiting signed list.
    pub async fn roster_draft_discard(&self, org_id: &str) -> Result<()> {
        self.orgs.discard_draft(org_id).await?;
        self.org_updated(org_id);
        Ok(())
    }

    /// Replaces a joined org's settings with a new join code's (**Update**). It fetched just
    /// now, so its timer counts from now, with the new policy's `refresh_minutes`.
    pub async fn update_org(
        &self,
        org_id: &str,
        code: &str,
        creds: Option<SourceCreds>,
    ) -> Result<OrgSummary> {
        let summary = self.orgs.update(org_id, code, creds).await?;
        self.org_timers.retime(org_id);
        self.org_updated(org_id);
        Ok(summary)
    }

    /// Fetches the org now (**Refresh**); its timer counts from now.
    pub async fn refresh_org(&self, org_id: &str) -> Result<OrgSummary> {
        let summary = self.orgs.refresh(org_id).await?;
        self.org_timers.retime(org_id);
        self.org_updated(org_id);
        Ok(summary)
    }

    /// Leaves the org (**Remove**): its timer stops first, so no fetch of it starts while its
    /// copy, sign-in details and row go. If the remove fails with the org still joined, its
    /// timer starts again.
    pub async fn remove_org(&self, org_id: &str) -> Result<()> {
        self.org_timers.stop(org_id);
        if let Err(e) = self.orgs.remove(org_id).await {
            if self.orgs.is_joined(org_id) {
                self.org_timers.start(org_id);
            }
            return Err(e);
        }
        self.org_updated(org_id);
        Ok(())
    }

    fn org_updated(&self, org_id: &str) {
        let _ = self.bus.send(RuntimeEvent::OrgUpdated {
            org_id: org_id.to_string(),
        });
    }
}

/// Browser use (browser spec sections 7 and 8).
impl Runtime {
    /// "Open the browser": the dot's Edge, for the person to log in ahead of time. It stays open
    /// until they close it, and the dot's runs use it meanwhile.
    pub async fn browser_open(&self, dot_id: &str) -> Result<()> {
        self.store.get_dot(dot_id).await?;
        self.browsers
            .open_for_person(dot_id)
            .await
            .map_err(|e| Error::Other(e.to_string()))
    }

    /// Where Microsoft Edge and Node.js (`npx`) were found, as a run would start them.
    pub async fn browser_status(&self) -> crate::browser::BrowserStatus {
        self.browsers.status().await
    }

    /// A screenshot the run's browser saved in its workspace's `downloads/`, by the name
    /// Playwright gave it, as a `data:` URL.
    pub async fn browser_screenshot(&self, run_id: &str, name: &str) -> Result<String> {
        let folder = self.run_folder(run_id).await?;
        crate::browser::screenshot(&folder, name)
    }
}

/// What needs the person (needs-you spec section 3).
impl Runtime {
    /// The person looked at the dot's conversation: its `seen_at` is now, and `DotSeen` tells
    /// the app (the tray, the other windows). `NotFound` for an unknown dot.
    pub async fn mark_dot_seen(&self, dot_id: &str) -> Result<()> {
        mark_seen(&self.store, &self.bus, dot_id).await
    }

    /// The dot whose conversation is open in the focused window, or none. Held in memory, not
    /// stored; the last call wins. While a dot is viewed, a run of it that ends and a note it
    /// posts mark it seen.
    pub fn set_viewing(&self, dot_id: Option<&str>) {
        *lock(&self.viewing) = dot_id.map(str::to_string);
    }

    /// The dot being viewed (`set_viewing`).
    pub fn viewing(&self) -> Option<String> {
        lock(&self.viewing).clone()
    }

    /// The sum of every dot's `unread`: the count of the Home link (needs-you spec section 5).
    pub async fn unread_total(&self) -> Result<u32> {
        let mut total = 0;
        for dot in self.store.list_dots().await? {
            match self.store.dot_counts(&dot.id).await {
                Ok((_, unread)) => total += unread,
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(total)
    }
}

/// The dot being viewed (`Runtime::set_viewing`), shared with `watch_viewing`.
type Viewing = Arc<std::sync::Mutex<Option<String>>>;

fn lock(viewing: &Viewing) -> std::sync::MutexGuard<'_, Option<String>> {
    viewing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn mark_seen(store: &Store, bus: &Bus, dot_id: &str) -> Result<()> {
    store.mark_dot_seen(dot_id).await?;
    let _ = bus.send(RuntimeEvent::DotSeen {
        dot_id: dot_id.to_string(),
    });
    Ok(())
}

/// Marks the viewed dot seen when a run of it ends or it posts a note, so that what the person
/// is reading never counts as new. Missed events may have held such a change, so a lag marks
/// the viewed dot seen too.
fn watch_viewing(store: Store, bus: Bus, viewing: Viewing, shutdown: CancellationToken) {
    let mut events = bus.subscribe();
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = shutdown.cancelled() => break,
                event = events.recv() => event,
            };
            let dot_id = match event {
                Ok(RuntimeEvent::RunUpdated { run }) if run.status.is_terminal() => run.dot_id,
                Ok(RuntimeEvent::MessagePosted { message })
                    if message.kind == MessageKind::Note =>
                {
                    message.dot_id
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => match lock(&viewing).clone() {
                    Some(dot_id) => dot_id,
                    None => continue,
                },
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let viewed = lock(&viewing).as_deref() == Some(dot_id.as_str());
            if viewed {
                if let Err(e) = mark_seen(&store, &bus, &dot_id).await {
                    tracing::warn!(dot = %dot_id, "marking the viewed dot seen failed: {e}");
                }
            }
        }
    });
}

/// What each dot remembers, the person's side (dot memory spec section 4). The text rules of
/// section 3 apply; their refusals keep the core's sentences.
impl Runtime {
    /// The dot's memories, pinned first, then newest. `NotFound` for an unknown dot.
    pub async fn memories_list(&self, dot_id: &str) -> Result<Vec<crate::model::Memory>> {
        self.checkins.memories().list(dot_id).await
    }

    /// The person's memory of the dot (source `person`).
    pub async fn memory_add(&self, dot_id: &str, text: &str) -> Result<crate::model::Memory> {
        self.require_for_dot(dot_id, Permission::EditDots).await?;
        self.checkins.memories().add(dot_id, text).await
    }

    /// Changes any memory's text and/or its pin.
    pub async fn memory_update(
        &self,
        id: &str,
        text: Option<&str>,
        pinned: Option<bool>,
    ) -> Result<crate::model::Memory> {
        self.require_for_memory(id).await?;
        self.checkins.memories().update(id, text, pinned).await
    }

    /// Deletes any memory. `NotFound` for an unknown id.
    pub async fn memory_delete(&self, id: &str) -> Result<()> {
        self.require_for_memory(id).await?;
        self.checkins.memories().delete(id).await
    }

    /// Changing a memory of an account's dot needs a role that may change its dots.
    async fn require_for_memory(&self, id: &str) -> Result<()> {
        let dot_id = self.store.get_memory(id).await?.dot_id;
        self.require_for_dot(&dot_id, Permission::EditDots).await
    }
}

/// Watchers, the person's side (watchers spec sections 7 and 8).
impl Runtime {
    pub fn watchers(&self) -> &Arc<crate::watchers::Watchers> {
        &self.watchers
    }

    pub async fn watchers_list(&self, dot_id: &str) -> Result<Vec<crate::model::Watcher>> {
        self.watchers.list(dot_id).await
    }

    pub async fn watcher_add(
        &self,
        dot_id: &str,
        name: &str,
        config: crate::model::WatcherConfig,
    ) -> Result<crate::model::Watcher> {
        self.require_for_dot(dot_id, Permission::EditDots).await?;
        self.watchers.add(dot_id, name, config).await
    }

    pub async fn watcher_update(
        &self,
        id: &str,
        name: &str,
        config: crate::model::WatcherConfig,
    ) -> Result<crate::model::Watcher> {
        self.require_for_watcher(id).await?;
        self.watchers.update(id, name, config).await
    }

    pub async fn watcher_delete(&self, id: &str) -> Result<()> {
        self.require_for_watcher(id).await?;
        self.watchers.delete(id).await
    }

    pub async fn watcher_set_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<crate::model::Watcher> {
        self.require_for_watcher(id).await?;
        self.watchers.set_enabled(id, enabled).await
    }

    pub async fn watcher_resume(&self, id: &str) -> Result<crate::model::Watcher> {
        self.require_for_watcher(id).await?;
        self.watchers.resume(id).await
    }

    /// Changing a watcher of an account's dot needs a role that may change its dots (members
    /// spec §6.3).
    async fn require_for_watcher(&self, id: &str) -> Result<()> {
        let dot_id = self.store.get_watcher(id).await?.dot_id;
        self.require_for_dot(&dot_id, Permission::EditDots).await
    }
}

/// Claude's usage limits (usage and limits spec sections 4 to 6).
impl Runtime {
    /// The limit readings, the shares and what is held now.
    pub async fn limits_get(&self) -> Result<crate::limits::LimitsState> {
        Ok(self.limits.state(
            &self.runner.queued_runs().await?,
            self.runner.running_background_claude(),
        ))
    }

    /// The Usage page: each dot's runs started in the period.
    pub async fn usage_summary(
        &self,
        period: crate::model::UsagePeriod,
    ) -> Result<crate::model::UsageSummary> {
        let since = crate::util::timestamp(self.clock.now() - period.reach());
        Ok(crate::model::UsageSummary {
            period,
            rows: self.store.usage_rows(&since).await?,
            since,
        })
    }

    /// **Run anyway**: releases one held background Claude run from the hold.
    pub async fn run_release_hold(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        // A run waiting for a role is not Run anyway's to release.
        if run.status == RunStatus::Queued {
            if let Some((account, _)) = self.runner.role_hold(&run).await? {
                return Err(Error::NotAllowed(format!(
                    "this run waits for {account} to let you run dots again"
                )));
            }
        }
        if run.status != RunStatus::Queued
            || run.hold_released
            || dot.spec.engine != crate::model::EngineKind::Claude
            || !crate::limits::is_background(run.trigger)
        {
            return Err(Error::Invalid(crate::limits::NOT_HELD.into()));
        }
        let run = self.store.set_hold_released(run_id).await?;
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run: run.clone() });
        self.runner.nudge();
        Ok(run)
    }

    /// Refuses the person's own Claude run while Claude's limit is reached: the CLI would
    /// refuse it anyway (usage and limits spec section 4).
    fn refuse_at_the_limit(&self, dot: &crate::model::Dot) -> Result<()> {
        if dot.spec.engine != crate::model::EngineKind::Claude {
            return Ok(());
        }
        let until = self
            .limits
            .rejected_until()
            .and_then(|t| DateTime::parse_from_rfc3339(&t).ok());
        match until {
            Some(until) => Err(Error::Invalid(crate::limits::limit_reached(
                until.with_timezone(&Utc),
                self.clock.now(),
            ))),
            None => Ok(()),
        }
    }

    /// The Inbox's held runs, oldest first.
    async fn held_runs(
        &self,
        names: &HashMap<String, String>,
    ) -> Result<Vec<crate::model::HeldRun>> {
        let state = self.limits_get().await?;
        let starts_after = if state.holding {
            state.hold_until.clone()
        } else {
            None
        };
        let mut held = Vec::new();
        for id in &state.held_run_ids {
            let run = self.store.get_run(id).await?;
            held.push(crate::model::HeldRun {
                dot_name: names.get(&run.dot_id).cloned().unwrap_or_default(),
                dot_id: run.dot_id,
                run_id: run.id,
                trigger: run.trigger,
                queued_at: run.queued_at,
                starts_after: starts_after.clone(),
                reason: crate::model::HoldReason::Usage,
            });
        }
        // Runs waiting for a role, which the limits do not hold.
        for (run, _) in self.store.queued_with_engine(false).await? {
            if let Some((account, text)) = self.runner.role_hold(&run).await? {
                held.push(crate::model::HeldRun {
                    dot_name: names.get(&run.dot_id).cloned().unwrap_or_default(),
                    dot_id: run.dot_id,
                    run_id: run.id,
                    trigger: run.trigger,
                    queued_at: run.queued_at,
                    starts_after: None,
                    reason: crate::model::HoldReason::Role { account, text },
                });
            }
        }
        held.sort_by(|a, b| a.queued_at.cmp(&b.queued_at));
        Ok(held)
    }
}

/// A Personal workspace's name, trimmed; it needs one.
fn personal_workspace_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("A workspace needs a name.".into()));
    }
    Ok(name.to_string())
}
