use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use crate::accounts::edit::RosterEdit;
use crate::accounts::{Permission, Role};
use crate::approvals::{ApprovalHub, DecideEffect};
use crate::checkins::CheckIns;
use crate::checkpoint::{ChangesPreview, Checkpoints, RunChanges, UndoOutcome};
use crate::dot_admin::{DotAdmin, DotAdminParts};
use crate::engine::claude::args::resolve_claude;
use crate::engine::claude::{cli_version, ClaudeEngine};
use crate::engine::local::client::DEFAULT_RETRY_DELAYS;
use crate::engine::local::{self, LocalEngine};
use crate::engine::{Engine, UnavailableEngine};
use crate::events::{new_bus, Bus, RuntimeEvent};
use crate::followups::FollowUps;
use crate::inbox::{self, InboxEntry, InboxKey, InboxKind, InboxMark, MarkState};
use crate::library::LibraryKind;
use crate::library_service::LibraryService;
use crate::making::service::Makers;
use crate::model::{
    Approval, AttentionRun, ClaudeStatus, ConversationFile, DeliveryCounts, Dot, DotOverview,
    DotSpec, EngineKind, FileSource, Inbox, MessageKind, NewRun, PendingApproval, ReviewRun, Run,
    RunEventRecord, RunStatus, ScheduleChange, ScheduleSkip, ScheduleView, ScheduledMessage,
    ScheduledStatus, ScheduledWait, SentLater, SettingsSaved, StagedAttachment, Status,
    TriggerKind, UndeliveredMessage, WorkspaceMode, WorktreeDiff,
};
use crate::policy::{
    allow_candidates, pattern_touches_private, touches_private, Action, ApprovalGuard, Rule,
    RuleCandidate, PRIVATE_REFUSAL,
};
use crate::runner::{Runner, RunnerConfig};
use crate::schedule::{upcoming, Schedule};
use crate::scheduler::{Clock, Scheduler, SystemClock};
use crate::send_later::{ChatExtra, SendTarget};
use crate::server::{self, ServerState};
use crate::settings::Settings;
use crate::setup::{
    check_proposal, setup_policy, SetupAnswer, SetupApplied, SetupContext, SetupDrafts,
    SetupLibrary, SetupProposal, SetupRequest,
};
use crate::skill_creator::SkillDrafts;
use crate::store::Store;
use crate::util::timestamp;
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

/// Retry's and Discard's refusal for a message the dispatcher has just claimed.
/// Why a result check Dots stopped in the middle of has no verdict.
const CHECK_INTERRUPTED: &str = "Dots stopped before the check ended.";
const JUST_REACHED: &str = "this message has just reached the dot";

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
    /// Install and update the apps that come with Dots at start (app library spec section 8).
    /// `Config::new` leaves them out, so a test starts with no app unless it asks for them;
    /// the app's configuration has them.
    pub bundled_apps: bool,
    /// The home folder whose `.claude` holds the Claude Code agents and plugins an import reads
    /// (import agents spec section 4). `Config::new` has none, so a test never reads the real
    /// one; the app's configuration has the person's.
    pub claude_home: Option<PathBuf>,
    /// Where a dot created without a folder gets one: `<new_dot_root>\<folder name>`. The app
    /// uses `%USERPROFILE%\Dots`; `Config::new` keeps it inside the data folder, so a test
    /// never touches the home folder.
    pub new_dot_root: PathBuf,
    /// The version of Dots the usage counts name (usage counts spec). The app gives its own.
    pub app_version: String,
    /// What posts the usage counts. `Config::new` posts over HTTP, as the app does; nothing is
    /// sent until the person turns the counts on.
    pub usage_sender: Arc<dyn crate::usage_counts::CountsSender>,
    /// What posts audit records to each account's endpoint (env vault spec §10). `Config::new`
    /// posts over HTTP, as the app does; an account whose policy has no `audit` sends nothing.
    pub audit_transport: Arc<dyn crate::vault::audit::AuditTransport>,
    /// Where Claude Code's transcripts are read for the Usage page's All Claude use (usage tab
    /// spec section 3). `Config::new` has none, so a test never reads the real one; the app's
    /// configuration has `CLAUDE_CONFIG_DIR` or the home's `.claude`, then `projects`.
    pub claude_projects: Option<PathBuf>,
}

impl Config {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            new_dot_root: data_dir.join("new-dots"),
            data_dir,
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
            bundled_apps: false,
            claude_home: None,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            usage_sender: Arc::new(crate::usage_counts::HttpSender::default()),
            audit_transport: Arc::new(crate::vault::audit::HttpTransport::default()),
            claude_projects: None,
        }
    }

    /// The start configuration the app derives from stored settings.
    pub fn from_settings(data_dir: impl Into<PathBuf>, settings: &Settings) -> Self {
        let mut cfg = Self::new(data_dir);
        cfg.port = settings.port;
        cfg.max_concurrent_runs = settings.max_concurrent_runs;
        cfg.claude_path = settings.claude_path.as_ref().map(PathBuf::from);
        cfg.secrets = None;
        cfg.bundled_apps = true;
        cfg.claude_home = ["USERPROFILE", "HOME"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
            .map(PathBuf::from);
        cfg.claude_projects = crate::claude_usage::projects_dir(
            std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref(),
            cfg.claude_home.as_deref(),
        );
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
        if let Some(home) = home.filter(|h| !h.is_empty()) {
            cfg.new_dot_root = PathBuf::from(home).join("Dots");
        }
        cfg
    }
}

/// Why a dot cannot use worktree mode on a folder that is not a git repository.
pub const NOT_A_REPO: &str =
    "This folder is not a git repository. Choose 'The folder itself' or pick a git folder.";

/// True when `path` is a folder inside a git work tree.
pub async fn is_git_repo(path: &Path) -> bool {
    path.is_dir()
        && crate::workspace::git(path, &["rev-parse", "--is-inside-work-tree"])
            .await
            .unwrap_or_default()
            == "true"
}

/// True when a dot may work in a worktree of `path`: the folder is the top level of a git
/// repository (`git rev-parse --show-toplevel` names it) and `HEAD` names a commit. A folder that
/// only sits inside a repository would get a checkout of the whole outer repository, and a
/// repository with no commit has nothing to check out.
pub async fn worktree_ready(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let Ok(top) = crate::workspace::git(path, &["rev-parse", "--show-toplevel"]).await else {
        return false;
    };
    let same = match (std::fs::canonicalize(&top), std::fs::canonicalize(path)) {
        (Ok(top), Ok(path)) => top == path,
        _ => false,
    };
    same && crate::workspace::git(path, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .await
        .is_ok()
}

/// Claims and returns the first free `<root>/<folder_name>[-N]`, missing parents created. A
/// candidate is skipped when a stored dot works in it (`used`); otherwise it is claimed by
/// making it, so two creates at once never get the same folder, or, when it is there already,
/// taken only if empty. The flag is true when this call made the folder, false when it was
/// there already, empty: only a folder it made may be removed again.
pub async fn default_folder(root: &Path, name: &str, used: &[PathBuf]) -> Result<(PathBuf, bool)> {
    let base = crate::model::folder_name(name);
    tokio::fs::create_dir_all(root).await?;
    let same = |a: &Path, b: &Path| {
        a == b
            || matches!(
                (std::fs::canonicalize(a), std::fs::canonicalize(b)),
                (Ok(a), Ok(b)) if a == b
            )
    };
    for n in 1u32..10_000 {
        let dir = if n == 1 {
            root.join(&base)
        } else {
            root.join(format!("{base}-{n}"))
        };
        if used.iter().any(|u| same(u, &dir)) {
            continue;
        }
        match tokio::fs::create_dir(&dir).await {
            Ok(()) => return Ok((dir, true)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let empty = match tokio::fs::read_dir(&dir).await {
                    Ok(mut entries) => entries.next_entry().await?.is_none(),
                    Err(_) => false,
                };
                if empty {
                    return Ok((dir, false));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Other(format!(
        "no free folder for '{name}' in {}",
        root.display()
    )))
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
    /// The setup agent's requests (setup agent spec §9).
    setup_drafts: SetupDrafts,
    /// Makes `setup_apply` check a name and create its dot in one step, so a second press of
    /// Run setup meets the first one's dot instead of making a second.
    setup_lock: Mutex<()>,
    clock: Arc<dyn Clock>,
    port: u16,
    server_error: Option<String>,
    claude_program: Option<PathBuf>,
    claude_version: Option<String>,
    data_dir: PathBuf,
    new_dot_root: PathBuf,
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
    /// What Claude Code's own transcripts say (usage tab spec).
    claude_usage: Arc<crate::claude_usage::ClaudeUsageService>,
    /// Where Claude Code's agents are looked for (`Config::claude_home`).
    claude_home: Option<PathBuf>,
    /// Makes agent imports run one at a time (`import_claude_agents`).
    import_lock: Mutex<()>,
    /// Wakes the send later loop: a message was scheduled or edited (send later spec 4.5).
    send_later_wake: Arc<tokio::sync::Notify>,
    /// The stored copies of files attached to chat messages (attachments spec section 4.2).
    chat_files: crate::chat_files::ChatFiles,
    /// The hourly usage counts, off until turned on (usage counts spec).
    usage_counts: Arc<crate::usage_counts::UsageCounts>,
    /// Sends each account's audit records (env vault spec §10).
    audit_sender: Arc<crate::vault::audit::AuditSender>,
    /// Creates, changes and deletes dots, for the person and for a run's tools alike
    /// (dots-make-dots design section 12).
    dot_admin: DotAdmin,
    /// What a maker's run does (dots-make-dots design sections 7 to 10).
    makers: Makers,
    shutdown: CancellationToken,
}

impl Runtime {
    pub async fn start(cfg: Config) -> Result<Runtime> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        // The vault folder is a private place: no rule allows a call into it (approval guards
        // spec section 3.1).
        if let Ok(data) = std::path::absolute(&cfg.data_dir) {
            crate::policy::register_vault(&data.join(OFFLINE_DIR));
        }
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
        // A result check Dots stopped in the middle of gets no verdict (result check spec
        // section 3); its leftover folders go.
        match store.skip_unfinished_checks(CHECK_INTERRUPTED).await {
            Ok(0) => {}
            Ok(n) => tracing::warn!("marked {n} unfinished result check(s) as skipped"),
            Err(e) => tracing::warn!("marking unfinished result checks failed: {e}"),
        }
        let _ = std::fs::remove_dir_all(cfg.data_dir.join(crate::result_check::CHECKS_DIR));
        // A scheduled message a crash left being sent: sent if its run was stored, else
        // scheduled again (send later spec section 4.5). Never sent twice, never lost.
        let recovered = store.recover_sending_messages(cfg.clock.now()).await?;
        if !recovered.is_empty() {
            tracing::warn!(
                "recovered {} scheduled message(s) a crash left being sent",
                recovered.len()
            );
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
        // Projects comes with Dots (spec section 8). One that cannot be installed or updated
        // leaves Dots running without the change; the next start tries again.
        if cfg.bundled_apps {
            if let Err(e) = apps.sync_bundled().await {
                tracing::error!(
                    "the apps that come with Dots could not be brought up to date: {e}"
                );
            }
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
                quiet_unit: Duration::from_secs(1),
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
        // Files attached to chat messages; staged ones and a crash's leftovers go (attachments
        // spec section 4.7).
        let chat_files =
            crate::chat_files::ChatFiles::new(cfg.data_dir.join(crate::chat_files::DIR));
        chat_files.sweep(&store).await;
        runner.set_chat_files(chat_files.clone());
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
        // The projects' code graphs (project graph spec), each in its own file.
        let graphs =
            crate::graph::Graphs::new(cfg.data_dir.join(crate::graph::DIR), Some(bus.clone()));
        // The graphs of worktrees removed while Dots was not running go now.
        {
            let (graphs, dir) = (graphs.clone(), cfg.data_dir.join("worktrees"));
            tokio::task::spawn_blocking(move || graphs.sweep_worktrees(&dir));
        }
        runner.set_graphs(graphs);
        let shutdown = CancellationToken::new();
        // All Claude use: Claude Code's transcripts, read in the background (usage tab spec
        // section 5).
        let claude_usage = crate::claude_usage::ClaudeUsageService::new(
            store.clone(),
            cfg.claude_projects.clone(),
            &cfg.data_dir,
            cfg.clock.clone(),
        );
        claude_usage.spawn(shutdown.clone());
        // The result check asks the CLI with no tools; it needs no local server.
        if let Some(program) = &claude_program {
            runner.set_checker(Arc::new(crate::result_check::CliChecker::new(
                program.clone(),
                cfg.claude_env.clone(),
                cfg.data_dir.join(crate::result_check::CHECKS_DIR),
                shutdown.clone(),
            )));
        }
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
        // A run that waits 30 minutes without reaching its dot is marked not delivered
        // (delivery tracking spec section 3).
        Arc::new(crate::delivery::DeliverySweeper::new(
            store.clone(),
            runner.clone(),
            bus.clone(),
            cfg.clock.clone(),
        ))
        .spawn(shutdown.clone());
        // Usage counts are sent only once the person turns them on (usage counts spec).
        let usage_counts = crate::usage_counts::UsageCounts::new(
            store.clone(),
            cfg.clock.clone(),
            cfg.usage_sender.clone(),
            cfg.app_version.clone(),
        );
        usage_counts.clone().spawn(shutdown.clone());
        // What each account queued goes to its audit endpoint, now and as it is written.
        let audit_sender =
            crate::vault::audit::AuditSender::new(orgs.clone(), cfg.audit_transport.clone());
        audit_sender.clone().spawn(shutdown.clone());
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
            zone: Arc::new(chrono::Local),
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
        // Scheduled workflows start from the scheduler's pass (teams spec section 5.1).
        scheduler.set_workflows(workflows.clone());
        // The workflow tools reach the workflow runs through the check-ins (teams spec
        // section 7).
        checkins.set_workflows(&workflows);
        watch_viewing(
            store.clone(),
            bus.clone(),
            viewing.clone(),
            shutdown.clone(),
        );
        // A dot is created, changed and deleted through one path, the person's and a maker's
        // alike. The tools for making dots reach it through the check-ins, on both engines
        // (dots-make-dots design section 12).
        let dot_admin = DotAdmin::new(DotAdminParts {
            store: store.clone(),
            bus: bus.clone(),
            clock: cfg.clock.clone(),
            new_dot_root: cfg.new_dot_root.clone(),
            orgs: orgs.clone(),
            vault: vault.clone(),
            scheduler: scheduler.clone(),
            watchers: watchers.clone(),
            follow_ups: follow_ups.clone(),
            runner: runner.clone(),
            teams: teams.clone(),
            apps: apps.clone(),
            chat_files: chat_files.clone(),
            browsers: browsers.clone(),
            viewing: viewing.clone(),
        });
        checkins.set_dot_admin(&dot_admin);
        // What a maker's run does. The check-ins hold it weakly, as they hold the dot admin:
        // the runtime keeps it alive (dots-make-dots design section 12).
        let makers = Makers::new(
            store.clone(),
            bus.clone(),
            dot_admin.clone(),
            vault.clone(),
            cfg.data_dir.join("worktrees"),
            follow_ups.clone(),
            runner.clone(),
        );
        checkins.set_makers(&makers);
        // Started after interrupted runs were failed, so its first pass sees each topic as it
        // is and delivers the wakes that waited across the restart (dots-make-dots design
        // section 8).
        makers.spawn_wakes(shutdown.clone());
        Ok(Runtime {
            dot_admin,
            makers,
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
            setup_drafts: SetupDrafts::new(
                claude_program.clone(),
                cfg.claude_env.clone(),
                cfg.data_dir.join(DRAFTS_DIR),
                cfg.draft_limit,
                shutdown.clone(),
            ),
            setup_lock: Mutex::new(()),
            claude_program,
            claude_version,
            data_dir: cfg.data_dir.clone(),
            new_dot_root: cfg.new_dot_root.clone(),
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
            claude_usage,
            claude_home: cfg.claude_home.clone(),
            import_lock: Mutex::new(()),
            send_later_wake: Arc::new(tokio::sync::Notify::new()),
            chat_files,
            usage_counts,
            audit_sender,
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

    /// A dot with the core's defaults and the model of Settings (model switcher spec section
    /// 2.2): what every dot made from the UI starts from.
    pub async fn new_dot_spec(&self, name: &str, instructions: &str, workdir: &str) -> DotSpec {
        let settings = self.settings.lock().await;
        crate::models::new_dot_spec(&settings, name, instructions, workdir)
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
        // As the store keeps it, so the live copy reads the same as the stored one.
        next.default_model = next.default_model.trim().to_string();
        next.checker_model = next.checker_model.trim().to_string();
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
        let mut view =
            ScheduleView::new(&state, schedule, dot.spec.enabled, attention, now, &Local);
        view.last_skipped_at = self
            .store
            .schedule_skips(&dot.id, 1)
            .await?
            .into_iter()
            .next()
            .map(|skip| skip.slot_at);
        let last_run = self.store.last_schedule_run_queued_at(&dot.id).await?;
        view.skipped_since_run = self.store.skips_since(&dot.id, last_run.as_deref()).await?;
        Ok(Some(view))
    }

    /// The dot's kept schedule skips, newest slot first: `limit` of them (default and most 50).
    /// `NotFound` for an unknown dot.
    pub async fn schedule_skips(
        &self,
        dot_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<ScheduleSkip>> {
        self.store.get_dot(dot_id).await?;
        self.store
            .schedule_skips(dot_id, limit.unwrap_or(50).min(50))
            .await
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

    /// Sets why each waiting run waits (`Runner::fill_wait_reasons`). The reason only explains
    /// a wait, so failing to read it fails nothing: it is logged and every reason left `None`.
    async fn fill_wait_reasons(&self, runs: &mut [Run]) {
        if let Err(e) = self.runner.fill_wait_reasons(runs).await {
            tracing::warn!("reading why runs wait failed: {e}");
            for run in runs.iter_mut() {
                run.wait_reason = None;
            }
        }
    }

    /// Newest first; `limit` is clamped to 1..=500.
    pub async fn list_runs(&self, dot_id: Option<&str>, limit: i64) -> Result<Vec<Run>> {
        let mut runs = self.store.list_runs(dot_id, limit.clamp(1, 500)).await?;
        self.fill_wait_reasons(&mut runs).await;
        Ok(runs)
    }

    pub async fn get_run(&self, id: &str) -> Result<Run> {
        let mut run = self.store.get_run(id).await?;
        self.fill_wait_reasons(std::slice::from_mut(&mut run)).await;
        Ok(run)
    }

    /// Why `run` is still waiting to reach its dot; `None` unless it is queued and not yet
    /// delivered (delivery tracking spec section 3).
    pub async fn wait_reason(&self, run: &Run) -> Result<Option<crate::model::WaitReason>> {
        let mut run = run.clone();
        self.runner
            .fill_wait_reasons(std::slice::from_mut(&mut run))
            .await?;
        Ok(run.wait_reason)
    }

    /// The whole lineage (root, resumes, chat follow-ups) `run_id` belongs to, oldest first.
    pub async fn run_thread(&self, run_id: &str) -> Result<Vec<Run>> {
        let run = self.store.get_run(run_id).await?;
        let mut runs = self.store.list_thread(&run.root_run_id).await?;
        self.fill_wait_reasons(&mut runs).await;
        Ok(runs)
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

    /// The whole Inbox, hidden items included, and the messages that wait: pending approvals,
    /// the finished, unreviewed thread ends that failed or kept a worktree with changes, the
    /// chat messages not delivered (not listed as runs to review too), the schedules that need
    /// attention, the MCP servers whose tool list waits for review, the workflow runs that wait
    /// for the person, and the rest.
    async fn full_inbox(&self) -> Result<(Inbox, Vec<crate::model::Message>)> {
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
        let not_delivered: Vec<UndeliveredMessage> = self
            .store
            .undelivered_chat_runs()
            .await?
            .into_iter()
            .map(|run| UndeliveredMessage {
                dot_name: name(&run.dot_id),
                run,
            })
            .collect();
        // A chat message that did not reach its dot is listed under Not delivered until it is
        // retried or discarded, and then it was dealt with: never as a run to review.
        let review_runs = self
            .store
            .unreviewed_thread_ends()
            .await?
            .into_iter()
            .filter(|run| {
                !(run.trigger == TriggerKind::Chat
                    && run.undelivered_at.is_some()
                    && run.delivered_at.is_none())
            })
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
        let instructions = self
            .store
            .pending_instruction_proposals()
            .await?
            .into_iter()
            .map(|proposal| crate::instructions::PendingInstructions {
                dot_name: name(&proposal.dot_id),
                proposal,
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
        let inbox = Inbox {
            approvals,
            review_runs,
            attention: self.store.list_schedule_attention().await?,
            tools_changed: self.library.tools_changed().await?,
            watchers: self.store.paused_watchers().await?,
            held: self.held_runs(&names).await?,
            instructions,
            app_reviews,
            team_scope: self.store.team_scope_issues().await?,
            not_delivered,
            workflows: self.store.waiting_workflow_runs().await?,
            back: Vec::new(),
            snoozed: Vec::new(),
            done: Vec::new(),
            hidden_messages: Vec::new(),
        };
        Ok((inbox, self.store.waiting_messages().await?))
    }

    /// The Inbox as the person sees it (snooze and done spec section 4.3): every section
    /// without the items snoozed or marked done, and the marks of the items still listed in
    /// `back`, `snoozed` and `done` (the Done list only for 7 days).
    pub async fn inbox(&self) -> Result<Inbox> {
        let (mut inbox, messages) = self.full_inbox().await?;
        let entries: HashMap<InboxKey, InboxEntry> = inbox::entries(&inbox, &messages)
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();
        let listed_since = timestamp(self.clock.now() - inbox::DONE_LISTED_FOR);
        let mut hidden = HashSet::new();
        for mut mark in self.store.list_marks().await? {
            let Some(entry) = entries.get(&mark.key()) else {
                continue;
            };
            mark.title = entry.title.clone();
            mark.dot_id = entry.dot_id.clone();
            if mark.back_at.is_some() {
                inbox.back.push(mark);
                continue;
            }
            hidden.insert(mark.key());
            if mark.snoozed_until.is_some() {
                inbox.snoozed.push(mark);
            } else if mark.done_at.as_deref() >= Some(listed_since.as_str()) {
                inbox.done.push(mark);
            }
        }
        // Newest back and done first, soonest snooze first; ties by key, so the order holds.
        inbox
            .back
            .sort_by(|a, b| b.back_at.cmp(&a.back_at).then(a.id.cmp(&b.id)));
        inbox
            .snoozed
            .sort_by(|a, b| a.snoozed_until.cmp(&b.snoozed_until).then(a.id.cmp(&b.id)));
        inbox
            .done
            .sort_by(|a, b| b.done_at.cmp(&a.done_at).then(a.id.cmp(&b.id)));
        let shown = |kind: InboxKind, id: &str| !hidden.contains(&InboxKey::new(kind, id));
        inbox
            .approvals
            .retain(|a| shown(InboxKind::Approval, &a.approval.id));
        inbox
            .not_delivered
            .retain(|u| shown(InboxKind::NotDelivered, &u.run.id));
        inbox
            .review_runs
            .retain(|r| shown(InboxKind::Review, &r.run.id));
        inbox
            .attention
            .retain(|a| shown(InboxKind::Attention, &a.run_id));
        inbox
            .tools_changed
            .retain(|t| shown(InboxKind::Tools, &format!("{}/{}", t.server_id, t.scope)));
        inbox
            .watchers
            .retain(|w| shown(InboxKind::Watcher, &format!("{}@{}", w.watcher_id, w.at)));
        inbox.held.retain(|h| shown(InboxKind::Held, &h.run_id));
        inbox
            .instructions
            .retain(|i| shown(InboxKind::Instructions, &i.proposal.id));
        inbox
            .app_reviews
            .retain(|r| shown(InboxKind::AppReview, &r.proposal.id));
        inbox
            .team_scope
            .retain(|t| shown(InboxKind::TeamScope, &format!("{}/{}", t.team_id, t.dot_id)));
        inbox.hidden_messages = messages
            .iter()
            .filter(|m| !shown(InboxKind::Message, &m.id))
            .map(|m| m.id.clone())
            .collect();
        Ok(inbox)
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
            + inbox.instructions.len()
            // A waiting review card counts like an approval (app library spec section 10).
            + inbox.app_reviews.len()
            // A team with a member outside its account or workspace (accounts design §6.3).
            + inbox.team_scope.len()
            // A workflow run waiting on the person (teams spec section 10).
            + inbox.workflows.len()
            // A chat message that did not reach its dot (snooze and done spec section 3.9.7).
            + inbox.not_delivered.len()
            + self.store.count_waiting_messages().await?)
    }

    /// The keys of everything in the Inbox now, hidden items included.
    async fn inbox_entries(&self) -> Result<Vec<InboxEntry>> {
        let (inbox, messages) = self.full_inbox().await?;
        Ok(inbox::entries(&inbox, &messages))
    }

    /// `NotFound` unless every one of `items` is in the Inbox now.
    async fn ensure_in_inbox(&self, items: &[InboxKey]) -> Result<()> {
        let keys: HashSet<InboxKey> = self
            .inbox_entries()
            .await?
            .into_iter()
            .map(|e| e.key)
            .collect();
        if items.iter().all(|k| keys.contains(k)) {
            Ok(())
        } else {
            Err(Error::NotFound("That is no longer in the Inbox.".into()))
        }
    }

    /// Hides `items` until `until` (snooze and done spec section 4.4): later than now and at
    /// most 365 days ahead. All of them or none.
    pub async fn inbox_snooze(&self, items: &[InboxKey], until: DateTime<Utc>) -> Result<()> {
        if items.is_empty() {
            return Err(Error::Invalid("Pick something to snooze.".into()));
        }
        let now = self.clock.now();
        if until <= now || until > now + inbox::SNOOZE_REACH {
            return Err(Error::Invalid("Pick a time in the future.".into()));
        }
        self.ensure_in_inbox(items).await?;
        self.store
            .snooze_marks(items, &timestamp(until), &timestamp(now))
            .await?;
        let _ = self.bus.send(RuntimeEvent::InboxUpdated);
        Ok(())
    }

    /// Marks the item done: hidden from the Inbox and the counts, with no time. An approval is
    /// decided, not marked done.
    pub async fn inbox_done(&self, item: &InboxKey) -> Result<()> {
        if item.kind == InboxKind::Approval {
            return Err(Error::Conflict(
                "An approval is decided, not marked done.".into(),
            ));
        }
        self.ensure_in_inbox(std::slice::from_ref(item)).await?;
        self.store
            .done_mark(item, &timestamp(self.clock.now()))
            .await?;
        let _ = self.bus.send(RuntimeEvent::InboxUpdated);
        Ok(())
    }

    /// **Unsnooze**: puts a snoozed item back as it was, with no tag. A back mark is taken too,
    /// so Unsnooze pressed just as the pass wakes the item is not an error.
    pub async fn inbox_unsnooze(&self, item: &InboxKey) -> Result<()> {
        if !self.store.delete_mark_if(item, MarkState::Snoozed).await? {
            return Err(Error::NotFound("That is not snoozed.".into()));
        }
        let _ = self.bus.send(RuntimeEvent::InboxUpdated);
        Ok(())
    }

    /// **Undo** on a done item: puts it back as it was.
    pub async fn inbox_undo_done(&self, item: &InboxKey) -> Result<()> {
        if !self.store.delete_mark_if(item, MarkState::Done).await? {
            return Err(Error::NotFound("That is not done.".into()));
        }
        let _ = self.bus.send(RuntimeEvent::InboxUpdated);
        Ok(())
    }

    /// One wake pass (snooze and done spec section 4.5): deletes the marks of items that left
    /// the Inbox on their own, then brings back every snooze whose time came, also one that
    /// passed while Dots was closed. Sends `InboxUpdated` when anything changed, and one
    /// `SnoozeEnded` for everything that woke. Returns the marks that woke, in Inbox order.
    pub async fn wake_snoozed(&self) -> Result<Vec<InboxMark>> {
        if self.store.count_marks().await? == 0 {
            return Ok(Vec::new());
        }
        // A mark made from here on is newer than the snapshot, which may not hold its item yet.
        let now = timestamp(self.clock.now());
        let mut entries = self.inbox_entries().await?;
        let keys: HashSet<InboxKey> = entries.iter().map(|e| e.key.clone()).collect();
        let pruned = self.store.prune_marks(&keys, &now).await?;
        let woke: HashSet<InboxKey> = self.store.wake_due(&now).await?.into_iter().collect();
        if !woke.is_empty() {
            // An item that left since the snapshot is not announced; its mark goes next pass.
            // The woken marks are already back, so a failed re-read keeps the first snapshot
            // rather than losing their notice.
            match self.inbox_entries().await {
                Ok(fresh) => entries = fresh,
                Err(e) => tracing::warn!("re-reading the Inbox after a wake failed: {e}"),
            }
        }
        let items: Vec<InboxMark> = entries
            .into_iter()
            .filter(|e| woke.contains(&e.key))
            .map(|e| InboxMark {
                kind: e.key.kind,
                id: e.key.id,
                title: e.title,
                dot_id: e.dot_id,
                snoozed_until: None,
                done_at: None,
                back_at: Some(now.clone()),
            })
            .collect();
        if pruned > 0 || !woke.is_empty() {
            let _ = self.bus.send(RuntimeEvent::InboxUpdated);
        }
        if !items.is_empty() {
            let _ = self.bus.send(RuntimeEvent::SnoozeEnded {
                items: items.clone(),
            });
        }
        Ok(items)
    }

    /// Runs a wake pass at once, then one every 30 seconds of the clock, until shutdown or
    /// until the runtime is dropped (it is held weakly). The app starts it once its event
    /// forwarder listens, so the first pass's notice is heard. Each wait is measured from the
    /// start of the pass before it.
    pub fn spawn_snooze_waker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let runtime = Arc::downgrade(self);
        let clock = self.clock.clone();
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            loop {
                let next = clock.now() + inbox::WAKE_EVERY;
                let Some(rt) = runtime.upgrade() else {
                    return;
                };
                if let Err(e) = rt.wake_snoozed().await {
                    tracing::warn!("waking snoozed items failed: {e}");
                }
                drop(rt);
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = clock.sleep_until(next) => {}
                }
            }
        })
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

    fn checkpoints(&self) -> Checkpoints {
        Checkpoints::new(self.data_dir.join(crate::checkpoint::DIR))
    }

    /// What a folder run changed in its folder (undo spec section 3), from its checkpoint.
    /// `None` for a run with none: a worktree run (its diff is `worktree_diff`), a run from
    /// before checkpoints, or one that has not started.
    pub async fn run_changes(&self, run_id: &str) -> Result<Option<RunChanges>> {
        self.store.get_run(run_id).await?;
        self.checkpoints().changes(run_id)
    }

    /// Each file of a folder run's changes with what undo (or redo, once undone) would do to
    /// it now, the run's diff, and the later runs in the folder that changed the same files.
    pub async fn run_changes_preview(&self, run_id: &str) -> Result<ChangesPreview> {
        self.store.get_run(run_id).await?;
        self.checkpoints().preview(run_id).await
    }

    /// **Undo these changes** (undo spec section 4): puts the files the run changed back as
    /// they were before it. A file changed since is skipped unless `overwrite`.
    pub async fn undo_run(&self, run_id: &str, overwrite: bool) -> Result<UndoOutcome> {
        self.apply_checkpoint(run_id, false, overwrite).await
    }

    /// **Redo**: puts an undone run's changes back as the run left them.
    pub async fn redo_run(&self, run_id: &str, overwrite: bool) -> Result<UndoOutcome> {
        self.apply_checkpoint(run_id, true, overwrite).await
    }

    /// Undo or redo, refused while the run, or any run in its folder, is working. Recorded on
    /// the run as a notice.
    async fn apply_checkpoint(
        &self,
        run_id: &str,
        redo: bool,
        overwrite: bool,
    ) -> Result<UndoOutcome> {
        self.require_for_run(run_id, Permission::RunDots).await?;
        let run = self.store.get_run(run_id).await?;
        if !run.status.is_terminal() {
            return Err(Error::Conflict(
                "this run is still working; undo its changes once it ends".into(),
            ));
        }
        if let Some(path) = &run.workspace_path {
            if !self.store.running_in_folder(path).await?.is_empty() {
                return Err(Error::Conflict(
                    "a run is working in this folder now; try again once it ends".into(),
                ));
            }
        }
        let outcome = self.checkpoints().apply(run_id, redo, overwrite).await?;
        let verb = if redo { "Redid" } else { "Undid" };
        let files = |n: usize| {
            if n == 1 {
                "1 file".to_string()
            } else {
                format!("{n} files")
            }
        };
        let mut text = format!(
            "{verb} this run's changes: put back {}.",
            files(outcome.restored.len())
        );
        if !outcome.skipped.is_empty() {
            text.push_str(&format!(
                " Left {} as {} {}.",
                files(outcome.skipped.len()),
                if outcome.skipped.len() == 1 {
                    "it"
                } else {
                    "they"
                },
                if outcome.skipped.len() == 1 {
                    "is"
                } else {
                    "are"
                },
            ));
        }
        let notice = crate::engine::EngineEvent::Notice { text };
        match self
            .store
            .append_event(run_id, notice.kind(), &serde_json::to_value(&notice)?)
            .await
        {
            Ok(event) => {
                let _ = self.bus.send(RuntimeEvent::RunEvent { event });
            }
            Err(e) => tracing::warn!(run = %run_id, "recording the undo failed: {e}"),
        }
        Ok(outcome)
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

    /// The Files panel's rows for the topics `roots` of the dot (attachments spec section 4.5):
    /// the files attached in them and the files the dot wrote or edited, inside each run's
    /// workspace only, merged by path and newest first. A root of another dot is left out.
    pub async fn conversation_files(
        &self,
        dot_id: &str,
        roots: &[String],
    ) -> Result<Vec<ConversationFile>> {
        self.store.get_dot(dot_id).await?;
        let mut mine: Vec<String> = Vec::new();
        for root in roots {
            match self.store.get_run(root).await {
                Ok(run) if run.dot_id == dot_id && !mine.contains(&run.root_run_id) => {
                    mine.push(run.root_run_id)
                }
                Ok(_) | Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let mut found: Vec<(PathBuf, FileSource, String, String)> = Vec::new();
        for root in &mine {
            let thread = self.store.list_thread(root).await?;
            // The topic's workspace: its newest run that has one.
            let workspace = thread
                .iter()
                .rev()
                .find_map(|r| r.workspace_path.as_deref().map(PathBuf::from));
            for a in self.store.topic_attachments(root).await? {
                let Some(run) = thread.iter().find(|r| a.run_id.as_ref() == Some(&r.id)) else {
                    continue;
                };
                let copy = match (&workspace, &a.name) {
                    (Some(ws), Some(name)) => {
                        Some(crate::chat_files::topic_dir(ws, root).join(name))
                    }
                    _ => None,
                };
                let path = match copy {
                    Some(copy) if copy.is_file() => copy,
                    _ => self.chat_files.stored_path(&a),
                };
                found.push((
                    path,
                    FileSource::Attached,
                    run.id.clone(),
                    run.queued_at.clone(),
                ));
            }
        }
        for w in self.store.written_files(dot_id, &mine).await? {
            let Some(workspace) = w.workspace_path.as_deref().map(PathBuf::from) else {
                continue;
            };
            let named = PathBuf::from(&w.path);
            let joined = if named.is_absolute() {
                named
            } else {
                workspace.join(named)
            };
            let path = crate::chat_files::normalize(&joined);
            // A name Windows would read as another file (`run.cmd.`, `a.pdf:x.exe`) is left
            // out: the list names files as they really are (attachments spec section 4.5.1).
            let plain = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(crate::chat_files::plain_name);
            if !plain || !crate::chat_files::is_inside(&path, &workspace) {
                continue;
            }
            found.push((path, FileSource::Written, w.run_id, w.at));
        }
        // One row per path (ignoring case on Windows): the newest time, Attached if any is.
        let mut rows: Vec<(String, ConversationFile)> = Vec::new();
        for (path, source, run_id, at) in found {
            let key = crate::chat_files::path_key(&path);
            match rows.iter_mut().find(|(k, _)| *k == key) {
                Some((_, row)) => {
                    if at > row.at {
                        row.at = at;
                        row.run_id = run_id;
                    }
                    if source == FileSource::Attached && row.source != FileSource::Attached {
                        row.source = FileSource::Attached;
                        row.path = path.to_string_lossy().to_string();
                    }
                }
                None => rows.push((
                    key,
                    ConversationFile {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        path: path.to_string_lossy().to_string(),
                        source,
                        run_id,
                        at,
                        exists: false,
                        can_open: false,
                    },
                )),
            }
        }
        let mut files: Vec<ConversationFile> = rows
            .into_iter()
            .map(|(_, mut row)| {
                row.exists = Path::new(&row.path).is_file();
                row.can_open = row.exists && crate::chat_files::can_open_file(Path::new(&row.path));
                row
            })
            .collect();
        files.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.path.cmp(&b.path)));
        Ok(files)
    }

    /// The file of the Files panel at `path`, to open (`open`) or to show in its folder: only a
    /// path the list names and that is a file now. Only a document, image or media file is
    /// opened (`can_open_file`).
    pub async fn conversation_file_target(
        &self,
        dot_id: &str,
        roots: &[String],
        path: &str,
        open: bool,
    ) -> Result<PathBuf> {
        let key = crate::chat_files::path_key(Path::new(path));
        let listed = self
            .conversation_files(dot_id, roots)
            .await?
            .into_iter()
            .find(|f| crate::chat_files::path_key(Path::new(&f.path)) == key);
        let Some(file) = listed.filter(|f| f.exists) else {
            return Err(Error::NotFound(crate::chat_files::NOT_THERE.into()));
        };
        if open && !file.can_open {
            return Err(Error::Invalid(crate::chat_files::PROGRAM.into()));
        }
        Ok(PathBuf::from(file.path))
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

    /// The hourly usage counts: their state, turning them on and off, the preview and Reset
    /// ID (usage counts spec).
    pub fn usage_counts(&self) -> &Arc<crate::usage_counts::UsageCounts> {
        &self.usage_counts
    }

    /// The audit sender: how sending stands for each account, and a pass on demand.
    pub fn audit_sender(&self) -> &Arc<crate::vault::audit::AuditSender> {
        &self.audit_sender
    }

    /// The joined account `org_id`'s History and how sending its records stands (env vault
    /// spec §10). Never a value.
    pub async fn org_audit(&self, org_id: &str) -> Result<crate::vault::audit::AuditView> {
        if !self.orgs.is_joined(org_id) {
            return Err(Error::NotFound(format!(
                "no org '{org_id}' is joined on this PC"
            )));
        }
        let audit = self.orgs.audit();
        // The last accepted target, also while no policy is in memory (no saved copy after a
        // restart): such an account still sends, so History must not say it has no endpoint.
        let endpoint = self.orgs.audit_target(org_id).map(|a| a.endpoint);
        Ok(crate::vault::audit::AuditView::new(
            endpoint,
            self.store.audit_queued(org_id).await?,
            self.audit_sender.state(org_id),
            self.store
                .audit_history_as_of(org_id, audit.clock().now(), &audit.limits())
                .await?,
        ))
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

    /// What the setup agent is told besides the request (setup agent spec §6): today, the
    /// local UTC offset, the home folder, where new dots get their folder, the Library (without
    /// the MCP servers a plugin brings, as the editor lists it) and the existing dots' names.
    pub async fn setup_context(&self) -> Result<SetupContext> {
        let now = Local::now();
        let home = ["USERPROFILE", "HOME"]
            .iter()
            .filter_map(std::env::var_os)
            .find(|h| !h.is_empty())
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut library = SetupLibrary::default();
        for entry in self.library.list().await? {
            let item = &entry.item;
            if item.owner_id.is_some() {
                continue;
            }
            let name = item.name.clone();
            match item.kind() {
                LibraryKind::Skill => {
                    let description = entry
                        .skill
                        .as_ref()
                        .map(|s| s.description.clone())
                        .unwrap_or_default();
                    library.skills.push((name, description));
                }
                LibraryKind::McpServer => library.mcp_servers.push(name),
                LibraryKind::Plugin => library.plugins.push(name),
                LibraryKind::Context => library.context.push(name),
                LibraryKind::App => {}
            }
        }
        let dot_names = self
            .store
            .list_dots()
            .await?
            .into_iter()
            .map(|d| d.spec.name)
            .collect();
        Ok(SetupContext {
            today: now.format("%Y-%m-%d (%A)").to_string(),
            utc_offset: now.format("%:z").to_string(),
            home,
            new_dot_root: self.new_dot_root.to_string_lossy().into_owned(),
            library,
            dot_names,
        })
    }

    /// Asks the setup agent for the setup `id` (`setup_ask`, setup agent spec §6): its
    /// questions, or a checked proposal. A newer request of the same id cancels the one before;
    /// other ids are left alone. It is the person's own action: never queued, never held.
    pub async fn setup_ask(&self, id: &str, req: SetupRequest) -> Result<SetupAnswer> {
        let ctx = self.setup_context().await?;
        self.setup_drafts.ask(id, &ctx, &req).await
    }

    /// Cancels the setup's request, if one is being answered (`setup_cancel`).
    pub fn setup_cancel(&self, id: &str) {
        self.setup_drafts.cancel(id);
    }

    /// Creates the proposed dot and starts its first chat (`setup_apply`, setup agent spec
    /// §9). The proposal is checked again, with a name clash a `Conflict`; a folder it names
    /// must be a folder on this PC (`Invalid`), and no folder gets a new default one, which a
    /// failed create removes again. The dot works in a worktree when its folder is the top level
    /// of a git repository with a commit, and in the folder itself otherwise. When the first chat cannot start, the dot stays and `run_error`
    /// says why.
    pub async fn setup_apply(
        &self,
        p: SetupProposal,
        org: Option<String>,
        workspace: Option<String>,
    ) -> Result<SetupApplied> {
        // Held until the dot is stored: a second Run setup meets its name.
        let one_at_a_time = self.setup_lock.lock().await;
        let ctx = self.setup_context().await?;
        let named = p
            .folder
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_string);
        let before = p.notes.clone();
        let p = check_proposal(p, &ctx, false)?;
        // Only what this check added: the card already showed the proposal's own notes.
        let notes = p
            .notes
            .strip_prefix(before.as_str())
            .unwrap_or(&p.notes)
            .trim()
            .to_string();
        let workdir = match (&named, p.folder.as_deref()) {
            (None, _) => String::new(),
            (Some(_), Some(checked)) if Path::new(checked).is_dir() => checked.to_string(),
            (Some(folder), _) => {
                return Err(Error::Invalid(format!(
                    "{folder} is not a folder on this PC."
                )))
            }
        };
        // Worktree mode, which `create_dot` turns into folder mode unless the folder is a
        // repository's top level with a commit, and always for a folder it makes.
        let mut spec = self.new_dot_spec(&p.name, &p.instructions, &workdir).await;
        spec.workspace_mode = WorkspaceMode::Worktree;
        spec.schedule = p.schedule.clone();
        spec.policy = setup_policy(p.preset, &p.extras);
        spec.uses = p.uses.clone();
        // A blank workdir gets the default folder in `create_dot`, which removes it again
        // when the create fails.
        let dot = self.create_dot(spec, org, workspace).await?;
        drop(one_at_a_time);
        // A blank first message, cleared on the card, starts no chat.
        let (run_id, run_error) = if p.first_message.trim().is_empty() {
            (None, None)
        } else {
            match self.chat(&dot.id, &p.first_message, None).await {
                Ok(run) => (Some(run.id), None),
                Err(e) => (None, Some(e.to_string())),
            }
        };
        Ok(SetupApplied {
            dot,
            run_id,
            run_error,
            notes,
        })
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

    /// What creates, changes and deletes dots: `create_dot`, `update_dot` and `delete_dot` go
    /// through it, and so do the tools a maker's run has (dots-make-dots design section 12).
    pub fn dot_admin(&self) -> &DotAdmin {
        &self.dot_admin
    }

    /// What a maker's run does: making a dot within its reach and its limits (dots-make-dots
    /// design section 7.1). The tools of both engines reach the same service through the
    /// check-ins.
    pub fn makers(&self) -> &Makers {
        &self.makers
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
        self.webhooks_on()?;
        Ok(format!(
            "http://127.0.0.1:{}/dots/{}/trigger",
            self.port, dot.id
        ))
    }

    /// The workflow's trigger URL (teams spec section 5.1), refused as `webhook_url` is while
    /// the local server is not running. `NotFound` for an unknown workflow.
    pub async fn workflow_webhook_url(&self, id: &str) -> Result<String> {
        self.webhooks_on()?;
        let workflow = self.store.get_workflow(id).await?;
        Ok(format!(
            "http://127.0.0.1:{}/workflows/{}/trigger",
            self.port, workflow.id
        ))
    }

    /// `Conflict` while the local server is not running, for the webhook URLs.
    fn webhooks_on(&self) -> Result<()> {
        match &self.server_error {
            Some(reason) => Err(Error::Conflict(format!(
                "webhooks are off because the local server is not running: {reason}"
            ))),
            None => Ok(()),
        }
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
        self.dot_admin.sync_schedule(dot, enabled_now).await
    }

    fn schedule_updated(&self, dot_id: &str) {
        self.dot_admin.schedule_updated(dot_id);
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
        self.dot_admin
            .create(spec, org_id, workspace_id, None)
            .await
    }

    /// The folders an agent scan reads: the home folder's agents, the workdir of each dot in the
    /// account and workspace in view (`scope`), and each installed Claude Code plugin (import
    /// agents spec section 3).
    fn agent_sources(
        &self,
        dots: &[Dot],
        scope: &crate::accounts::CurrentScope,
    ) -> crate::claude_agents::AgentSources {
        let Some(home) = &self.claude_home else {
            return Default::default();
        };
        let mut projects: Vec<(PathBuf, crate::model::WorkspaceMode)> = Vec::new();
        let in_view = |dot: &&Dot| {
            dot.org_id.as_deref().unwrap_or(crate::accounts::PERSONAL) == scope.account
                && dot.workspace_id == scope.workspace
        };
        for dot in dots.iter().filter(in_view) {
            let dir = PathBuf::from(&dot.spec.workdir);
            if dot.spec.workdir.trim().is_empty() || projects.iter().any(|(p, _)| *p == dir) {
                continue;
            }
            projects.push((dir, dot.spec.workspace_mode));
        }
        let home_text = home.to_string_lossy().into_owned();
        let env = |key: &str| match key {
            "USERPROFILE" | "HOME" => Some(home_text.clone()),
            other => crate::library_service::process_env(other),
        };
        let plugins = crate::kit::installed_plugins(&env)
            .into_iter()
            .filter(|p| p.exists)
            .map(|p| (p.name, p.install_path))
            .collect();
        crate::claude_agents::AgentSources {
            user: Some(home.join(".claude").join("agents")),
            projects,
            plugins,
        }
    }

    async fn agent_scan(&self) -> Result<(Vec<crate::claude_agents::Found>, Vec<String>)> {
        let dots = self.store.list_dots().await?;
        let names: Vec<String> = dots.iter().map(|d| d.spec.name.clone()).collect();
        let scope = self.current_scope().await?;
        let sources = self.agent_sources(&dots, &scope);
        let found =
            tokio::task::spawn_blocking(move || crate::claude_agents::scan(&sources, &names))
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        Ok((found, dots.into_iter().map(|d| d.spec.name).collect()))
    }

    /// The Claude Code agents on this PC, each with the dot it would become.
    pub async fn scan_claude_agents(&self) -> Result<Vec<crate::claude_agents::ScannedAgent>> {
        Ok(self
            .agent_scan()
            .await?
            .0
            .into_iter()
            .map(|f| f.listed)
            .collect())
    }

    /// Makes a dot of each agent file in `paths`, in the account `org_id` and workspace
    /// `workspace_id` as `create_dot` does. Only a file a fresh scan finds and can read is
    /// imported, as it reads now, and a path given twice once; a taken name gets a number. An
    /// agent that fails is named in `failed` and the others go on. Imports run one at a time
    /// and read the dots' names when they start, so two never pick the same name; a dot made
    /// meanwhile under that name (the store refuses a second) moves the import on to the next
    /// number. An agent that is not a project's gets a new folder as a dot made without one.
    pub async fn import_claude_agents(
        &self,
        paths: Vec<String>,
        org_id: Option<String>,
        workspace_id: Option<String>,
    ) -> Result<crate::claude_agents::AgentImport> {
        use crate::claude_agents::{agent_spec, free_name, AgentImport, ImportFailure};
        let workspace = workspace_id
            .clone()
            .unwrap_or_else(|| crate::accounts::DEFAULT_WORKSPACE.into());
        self.check_workspace(org_id.as_deref(), &workspace).await?;
        self.require_in(org_id.as_deref(), &workspace, Permission::EditDots)
            .await?;
        let _one_at_a_time = self.import_lock.lock().await;
        let (found, names) = self.agent_scan().await?;
        let mut taken: std::collections::BTreeSet<String> =
            names.iter().map(|n| n.to_lowercase()).collect();
        let mut done = AgentImport {
            created: Vec::new(),
            failed: Vec::new(),
        };
        let mut given = std::collections::BTreeSet::new();
        let default_model = self.settings.lock().await.default_model.clone();
        for path in paths {
            if !given.insert(path.clone()) {
                continue;
            }
            let fail = |reason: String| ImportFailure {
                path: path.clone(),
                reason,
            };
            let Some(item) = found.iter().find(|f| f.listed.path == path) else {
                done.failed.push(fail(
                    "it was not found by a scan of the Claude Code agent folders".into(),
                ));
                continue;
            };
            let Some(agent) = &item.agent else {
                let reason = item.listed.error.clone().unwrap_or_default();
                done.failed.push(fail(reason));
                continue;
            };
            // A few names in turn, for a dot made under the picked one meanwhile.
            let mut outcome = Err(Error::Other("no name was free".into()));
            for _ in 0..5 {
                let name = free_name(&item.listed.name, &mut taken);
                // A project agent works in its project; any other gets a folder of its own from
                // `create_dot` (a blank workdir), which removes it again if the create fails.
                let (workdir, mode) = match &item.project {
                    Some((dir, mode)) => (dir.to_string_lossy().into_owned(), *mode),
                    None => (String::new(), crate::model::WorkspaceMode::Folder),
                };
                let mut spec = agent_spec(agent, &name, &workdir, mode);
                // An agent that names no model starts with the default one.
                if crate::kit::agent_model(agent.model.as_deref()).is_none() {
                    spec.model = default_model.clone();
                }
                outcome = self
                    .create_dot(spec, org_id.clone(), workspace_id.clone())
                    .await;
                if !matches!(outcome, Err(Error::Conflict(_))) {
                    break;
                }
            }
            match outcome {
                Ok(dot) => done.created.push(dot),
                Err(e) => done.failed.push(fail(e.to_string())),
            }
        }
        Ok(done)
    }

    /// Refuses a workspace the account (`None` is Personal) doesn't have.
    async fn check_workspace(&self, org_id: Option<&str>, workspace_id: &str) -> Result<()> {
        self.dot_admin.check_workspace(org_id, workspace_id).await
    }

    async fn no_workspace(&self, org_id: Option<&str>, workspace_id: &str) -> Error {
        self.dot_admin.no_workspace(org_id, workspace_id).await
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
    /// newest first. Keys of a workspace this PC can no longer see are left out (they stay
    /// refused at request time). Never carries a key's text; whether keys may be created in a
    /// workspace is in its `WorkspaceView`.
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
                Some(crate::accounts::api_keys::ApiKeyView::new(
                    row,
                    ws.name.clone(),
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
        // The workspace may have been deleted since it was checked.
        match self.store.add_workspace_api_key(&row).await {
            Err(Error::NotFound(_)) => return Err(self.no_workspace(org_id, workspace_id).await),
            added => added?,
        }
        let key = ApiKeyView::new(row, self.workspace_label(scope, workspace_id).await?);
        Ok(CreatedApiKey { key, secret })
    }

    /// Revokes a key: it stops working at once and stays listed as revoked. No role is needed:
    /// revoking only reduces access, so any key stored on this PC may be revoked.
    pub async fn revoke_api_key(&self, id: &str) -> Result<crate::accounts::api_keys::ApiKeyView> {
        let gone = || Error::NotFound("That API key isn't on this PC any more.".into());
        match self.store.revoke_api_key(id, &crate::util::now()).await {
            Err(Error::NotFound(_)) => return Err(gone()),
            revoked => revoked?,
        }
        let row = self.store.api_key(id).await?.ok_or_else(gone)?;
        let name = self.workspace_label(&row.scope, &row.workspace_id).await?;
        Ok(crate::accounts::api_keys::ApiKeyView::new(row, name))
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
        self.dot_admin.update(id, spec, false).await
    }

    /// Deletes the dot; its schedule row goes with it (`ON DELETE CASCADE`). In an account, the
    /// role must allow changing dots (members spec §6.3), or this PC is waiting for or removed
    /// from the member list, and only cleaning up (`require_cleanup`).
    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        self.dot_admin.delete(id).await
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

    /// Stages the file at `path` (picked or dropped) for a chat message to the dot
    /// (attachments spec section 4.2). Refused with a reason for a link, a folder, a file over
    /// 25 MB or one that cannot be read.
    pub async fn add_attachment_path(&self, dot_id: &str, path: &Path) -> Result<StagedAttachment> {
        let dot = self.store.get_dot(dot_id).await?;
        self.require_run(&dot).await?;
        let id = crate::util::new_id();
        let files = self.chat_files.clone();
        let from = path.to_path_buf();
        let staged_id = id.clone();
        let (name, size) = tokio::task::spawn_blocking(move || files.stage_path(&staged_id, &from))
            .await
            .map_err(|e| Error::Other(format!("staging a file failed: {e}")))??;
        self.record_staged(&id, dot_id, name, size).await
    }

    /// Stages `bytes` (a pasted image) named `name` for a chat message to the dot.
    pub async fn add_attachment_bytes(
        &self,
        dot_id: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<StagedAttachment> {
        let dot = self.store.get_dot(dot_id).await?;
        self.require_run(&dot).await?;
        let id = crate::util::new_id();
        let (name, size) = self.chat_files.stage_bytes(&id, name, bytes)?;
        self.record_staged(&id, dot_id, name, size).await
    }

    /// Records a stored copy's row; a failed insert removes the copy.
    async fn record_staged(
        &self,
        id: &str,
        dot_id: &str,
        name: String,
        size: u64,
    ) -> Result<StagedAttachment> {
        match self
            .store
            .insert_staged_attachment(id, dot_id, &name, size)
            .await
        {
            Ok(_) => Ok(StagedAttachment {
                id: id.to_string(),
                name,
                size,
            }),
            Err(e) => {
                self.chat_files.remove(id);
                Err(e)
            }
        }
    }

    /// Removes a staged file (its chip's ×, or the chat box going away unsent). `Conflict`
    /// once a message took it; `NotFound` for an unknown id.
    pub async fn remove_attachment(&self, id: &str) -> Result<()> {
        let attachment = self.store.get_attachment(id).await?;
        if !self.store.remove_unsent_attachment(id).await? {
            return Err(Error::Conflict(
                "this file was already sent with a message".into(),
            ));
        }
        self.chat_files.remove(&attachment.id);
        Ok(())
    }

    pub async fn chat(
        &self,
        dot_id: &str,
        message: &str,
        parent_run_id: Option<&str>,
    ) -> Result<Run> {
        self.chat_with(dot_id, message, parent_run_id, ChatExtra::default())
            .await
    }

    /// "Ask the dot to fix this" (result check spec section 4): the concerns the run's result
    /// check raised go to the dot as the person's next message in the run's conversation, after
    /// its last message, as `chat` sends it (and refuses it). `Invalid` when the check flagged
    /// nothing. Never sent by itself: only the person asks.
    pub async fn ask_to_fix(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        let text = run
            .result_check
            .as_ref()
            .and_then(crate::result_check::fix_request)
            .ok_or_else(|| Error::Invalid("This answer's check found nothing to fix.".into()))?;
        let thread = self.store.list_thread(&run.root_run_id).await?;
        let end = crate::send_later::thread_end(&thread)
            .map(|r| r.id.clone())
            .unwrap_or(run.id.clone());
        self.chat(&run.dot_id, &text, Some(&end)).await
    }

    /// The chat path with files (attachments spec section 4.3): `attachment_ids` are staged
    /// files of this dot. The message may be files only.
    pub async fn chat_with_files(
        &self,
        dot_id: &str,
        message: &str,
        parent_run_id: Option<&str>,
        attachment_ids: &[String],
    ) -> Result<Run> {
        let extra = ChatExtra {
            attachment_ids: attachment_ids.to_vec(),
            ..ChatExtra::default()
        };
        self.chat_with(dot_id, message, parent_run_id, extra).await
    }

    /// The names a new file of the topic `root` may not take: its chat files', its root's
    /// files when a watcher started it (they share the folder), and every entry in its folder
    /// in `workspace` now, such as a file the dot made there (attachments spec section 3.1.5).
    async fn taken_names(&self, root: &str, workspace: Option<&str>) -> Result<Vec<String>> {
        let mut names = self.store.topic_attachment_names(root).await?;
        let first = self.store.get_run(root).await?;
        if first.trigger == TriggerKind::Watcher {
            let listed = first
                .payload
                .as_ref()
                .and_then(|p| p.get("attachments"))
                .and_then(|a| a.as_array());
            for a in listed.into_iter().flatten() {
                if let Some(name) = a.get("name").and_then(|n| n.as_str()) {
                    names.push(name.to_string());
                }
            }
        }
        if let Some(ws) = workspace {
            let dir = crate::chat_files::topic_dir(Path::new(ws), root);
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    names.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        }
        Ok(names)
    }

    /// Checks the files a message names before anything is written (attachments spec section
    /// 4.3): each is a file of this dot that no message took (or that `scheduled_id` holds),
    /// and its stored copy is there. Answers them in order.
    async fn files_to_send(
        &self,
        dot_id: &str,
        ids: &[String],
        scheduled_id: Option<&str>,
    ) -> Result<Vec<crate::model::ChatAttachment>> {
        if ids.len() > crate::chat_files::MAX_FILES {
            return Err(Error::Invalid(crate::chat_files::TOO_MANY.into()));
        }
        let gone = || Error::Conflict(crate::chat_files::GONE.into());
        let mut files = Vec::with_capacity(ids.len());
        for id in ids {
            if files
                .iter()
                .any(|f: &crate::model::ChatAttachment| &f.id == id)
            {
                return Err(gone());
            }
            let a = match self.store.get_attachment(id).await {
                Ok(a) => a,
                Err(Error::NotFound(_)) => return Err(gone()),
                Err(e) => return Err(e),
            };
            let held_here = match (&a.scheduled_id, scheduled_id) {
                (None, _) => true,
                (Some(held), Some(by)) => held == by,
                (Some(_), None) => false,
            };
            if a.dot_id != dot_id
                || a.run_id.is_some()
                || !held_here
                || !self.chat_files.stored_path(&a).is_file()
            {
                return Err(gone());
            }
            files.push(a);
        }
        Ok(files)
    }

    /// The chat path (`chat`), with what the run carries besides its text (send later spec
    /// section 4.3): a scheduled message's note goes into the payload as `send_later`.
    pub async fn chat_with(
        &self,
        dot_id: &str,
        message: &str,
        parent_run_id: Option<&str>,
        extra: ChatExtra,
    ) -> Result<Run> {
        // Retry: the files of the message sent again go with it, keeping their names.
        let carried = match &extra.carry_from {
            Some(old) => self.store.run_attachments(old).await?,
            None => Vec::new(),
        };
        // A message may be files only (attachments spec section 3.1).
        if message.trim().is_empty() && extra.attachment_ids.is_empty() && carried.is_empty() {
            return Err(Error::Invalid("message must not be empty".into()));
        }
        let files = self
            .files_to_send(dot_id, &extra.attachment_ids, extra.scheduled_id.as_deref())
            .await?;
        let dot = self.store.get_dot(dot_id).await?;
        self.require_run(&dot).await?;
        self.refuse_at_the_limit(&dot)?;
        let mut new = NewRun::new(dot_id, TriggerKind::Chat);
        new.id = Some(crate::util::new_id());
        // The person typed it (conversational chat spec section 3).
        let mut payload = json!({ "message": message, crate::prompt::TYPED: true });
        if let Some(note) = &extra.send_later {
            payload["send_later"] = serde_json::to_value(note)?;
        }
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
            // The parent is taken from this reading of the thread, in which it has ended. A
            // message that never reached the dot and was retried or discarded (delivery
            // tracking spec section 3.9) is not part of the conversation: the end is the last
            // run that is not one of those.
            let live = crate::send_later::thread_end(&thread);
            let Some(parent) = live.filter(|r| r.id == pid) else {
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
        // Names are fixed now, unique in the topic ignoring case, with the names it already
        // has, a watcher root's files, whatever is in its folder now and `.gitignore` taken
        // (attachments spec section 3.1).
        let mut links = Vec::with_capacity(files.len());
        if !files.is_empty() {
            let mut all = match &new.root_run_id {
                Some(root) => {
                    self.taken_names(root, new.workspace_path.as_deref())
                        .await?
                }
                None => Vec::new(),
            };
            all.push(crate::chat_files::IGNORE_FILE.to_string());
            let mut seen = HashSet::new();
            all.retain(|n| seen.insert(n.to_lowercase()));
            let taken = all.len();
            all.extend(files.iter().map(|f| f.file_name.clone()));
            let unique = crate::watchers::files::unique_names(&all);
            for (f, name) in files.iter().zip(unique.into_iter().skip(taken)) {
                links.push((f.id.clone(), name));
            }
        }
        if !carried.is_empty() || !links.is_empty() {
            let mut listed: Vec<serde_json::Value> = carried
                .iter()
                .map(|a| json!({ "id": a.id, "name": a.name, "size": a.size }))
                .collect();
            listed.extend(
                files
                    .iter()
                    .zip(&links)
                    .map(|(f, (id, name))| json!({ "id": id, "name": name, "size": f.size })),
            );
            payload["attachments"] = serde_json::Value::Array(listed);
        }
        new.payload = Some(payload);
        let run = self
            .store
            .create_chat_run(
                &new,
                &links,
                extra.scheduled_id.as_deref(),
                extra.carry_from.as_deref(),
            )
            .await?;
        self.runner.enqueued(&run);
        Ok(run)
    }

    /// **Send again** (crash resend spec section 3.6): queues the failed chat run `run_id`'s
    /// message once more, in its topic, with `retry_of` = `run_id`. Checked as a reply is: the
    /// person may run the dot, the usage limit is not reached, and the run is the last of its
    /// topic, which has nothing working. Refusals are `Conflict` with text the UI shows.
    pub async fn resend(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        if run.trigger != TriggerKind::Chat || run.status != RunStatus::Failed {
            return Err(Error::Conflict(
                "only a message that failed can be sent again".into(),
            ));
        }
        // A message that never reached the dot has its own Retry (delivery tracking spec
        // section 3.9), which also clears it from Needs you.
        if run.delivered_at.is_none() && run.undelivered_at.is_some() {
            return Err(Error::Conflict(
                "this message did not reach the dot; use Retry on it".into(),
            ));
        }
        let dot = self.store.get_dot(&run.dot_id).await?;
        self.require_run(&dot).await?;
        self.refuse_at_the_limit(&dot)?;
        let _one_at_a_time = self.chat_lock.lock().await;
        if self.follow_ups.deciding(&run.root_run_id) {
            return Err(Error::Conflict(crate::checkins::DECIDING.into()));
        }
        let thread = self.store.list_thread(&run.root_run_id).await?;
        if thread.iter().any(|r| !r.status.is_terminal())
            || thread.last().map(|r| r.id.as_str()) != Some(run_id)
        {
            return Err(Error::Conflict(
                "this chat has moved on since this message; send a new message instead".into(),
            ));
        }
        if self.store.in_ended_step_thread(&run).await? {
            return Err(Error::Conflict(crate::workflows::ENDED_STEP_THREAD.into()));
        }
        let parent = match &run.parent_run_id {
            Some(pid) => Some(self.store.get_run(pid).await?),
            None => None,
        };
        let mut new = NewRun::resend_of(&run, parent.as_ref());
        // Sent by the person: no note that Dots resent it.
        if let Some(serde_json::Value::Object(payload)) = &mut new.payload {
            payload.remove(crate::model::RESENT);
        }
        // Its files go with it, as a Retry's do (attachments spec section 4.3).
        let run = self
            .store
            .create_chat_run(&new, &[], None, Some(run_id))
            .await?;
        self.runner.enqueued(&run);
        Ok(run)
    }

    fn topics_changed(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::TopicsChanged {
            dot_id: dot_id.to_string(),
        });
    }

    /// The rows of the dot's topics: titles and archive marks (topic tools spec section 4.3).
    pub async fn list_topics(&self, dot_id: &str) -> Result<Vec<crate::model::TopicInfo>> {
        self.store.get_dot(dot_id).await?;
        self.store.list_topics(dot_id).await
    }

    /// Names the topic whose root is `root` (`None` or blank clears the name).
    pub async fn rename_topic(
        &self,
        root: &str,
        title: Option<&str>,
    ) -> Result<crate::model::TopicInfo> {
        self.require_for_run(root, Permission::RunDots).await?;
        let info = self.store.set_topic_title(root, title).await?;
        self.topics_changed(&info.dot_id);
        Ok(info)
    }

    /// Archives or unarchives the topic whose root is `root`.
    pub async fn set_topic_archived(
        &self,
        root: &str,
        archived: bool,
    ) -> Result<crate::model::TopicInfo> {
        self.require_for_run(root, Permission::RunDots).await?;
        let info = self
            .store
            .set_topic_archived(root, archived, Utc::now())
            .await?;
        self.topics_changed(&info.dot_id);
        Ok(info)
    }

    /// Moves the run `run_id` and its replies to the topic `to`, or to a new topic (`None`)
    /// (topic tools spec section 3.4). Returns the root of the topic the run is now in.
    pub async fn move_run(&self, run_id: &str, to: Option<&str>) -> Result<String> {
        let run = self.store.get_run(run_id).await?;
        self.require_for_dot(&run.dot_id, Permission::RunDots)
            .await?;
        // Under the lock a message to a topic takes, so none is sent into the line being cut.
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        for root in std::iter::once(run.root_run_id.as_str()).chain(to) {
            if self.follow_ups.deciding(root) {
                return Err(Error::Conflict(crate::checkins::DECIDING.into()));
            }
        }
        let root = self.store.move_runs(run_id, to).await?;
        self.topics_changed(&run.dot_id);
        Ok(root)
    }

    /// Merges the topic `from` into the topic `into` of the same dot (topic tools spec section
    /// 3.5).
    pub async fn merge_topics(&self, from: &str, into: &str) -> Result<()> {
        let run = self.store.get_run(into).await?;
        self.require_for_dot(&run.dot_id, Permission::RunDots)
            .await?;
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        if self.follow_ups.deciding(from) || self.follow_ups.deciding(into) {
            return Err(Error::Conflict(crate::checkins::DECIDING.into()));
        }
        self.store.merge_topics(from, into).await?;
        self.topics_changed(&run.dot_id);
        Ok(())
    }

    pub async fn cancel_run(&self, run_id: &str) -> Result<Run> {
        self.runner.cancel(run_id).await
    }

    /// Sends a chat message that did not get there again (delivery tracking spec sections 3.9
    /// and 4.4): the old run is dismissed (cancelled first if it still waits), then the same
    /// text goes through the chat path at the old run's place in the thread. A send the chat
    /// path refuses leaves the old run as it was, so the person can try again. Returns the new
    /// run. `NotFound` for an unknown run; `Conflict` unless it is a chat message marked not
    /// delivered and not yet retried or discarded.
    pub async fn retry_delivery(&self, run_id: &str) -> Result<Run> {
        let run = self.dismiss_undelivered(run_id).await?;
        let text = ["answer", "message"].iter().find_map(|k| {
            run.payload
                .as_ref()
                .and_then(|p| p.get(*k))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });
        let Some(text) = text else {
            let old = self.store.undismiss_run(run_id).await?;
            self.emit_run_updated(old);
            return Err(Error::Invalid(
                "this message has no text to send again".into(),
            ));
        };
        // Its files move to the new message (attachments spec section 4.3).
        let extra = ChatExtra {
            carry_from: Some(run.id.clone()),
            ..ChatExtra::default()
        };
        match self
            .chat_with(&run.dot_id, &text, run.parent_run_id.as_deref(), extra)
            .await
        {
            Ok(new) => {
                self.emit_run_updated(run);
                Ok(new)
            }
            Err(e) => {
                match self.store.undismiss_run(run_id).await {
                    Ok(old) => self.emit_run_updated(old),
                    Err(u) => tracing::warn!(run = %run_id, "undoing a refused retry failed: {u}"),
                }
                Err(e)
            }
        }
    }

    /// Puts away a chat message that did not get there (delivery tracking spec section 4.4):
    /// the run is dismissed, and cancelled first if it still waits so it never starts later.
    /// Returns the dismissed run. Same refusals as `retry_delivery`.
    pub async fn discard_delivery(&self, run_id: &str) -> Result<Run> {
        let run = self.dismiss_undelivered(run_id).await?;
        self.emit_run_updated(run.clone());
        Ok(run)
    }

    /// Retry's and Discard's first steps: checks the run and dismisses it, cancelling it if it
    /// is still queued (the cancel's write dismisses a marked run). Returns the dismissed run.
    async fn dismiss_undelivered(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        if run.trigger != TriggerKind::Chat {
            return Err(Error::Conflict(
                "only a chat message can be retried or discarded".into(),
            ));
        }
        if run.undelivered_at.is_none() || run.delivered_at.is_some() {
            return Err(Error::Conflict(
                "this message is not marked as not delivered".into(),
            ));
        }
        if run.dismissed_at.is_some() {
            return Err(Error::Conflict(
                "this message was already retried or discarded".into(),
            ));
        }
        if run.status == RunStatus::Queued {
            return match self.runner.cancel_if_queued(run_id).await? {
                Some(run) => Ok(run),
                None => Err(Error::Conflict(JUST_REACHED.into())),
            };
        }
        // Claimed by the dispatcher: its engine is starting on it, so it stays.
        if !run.status.is_terminal() {
            return Err(Error::Conflict(JUST_REACHED.into()));
        }
        match self.store.dismiss_run(run_id, self.clock.now()).await? {
            Some(run) => Ok(run),
            None => Err(Error::Conflict(
                "this message was already retried or discarded".into(),
            )),
        }
    }

    fn emit_run_updated(&self, run: Run) {
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run });
    }

    /// The dot's sends today, local time (delivery tracking spec sections 3.8 and 4.5).
    /// `NotFound` for an unknown dot.
    pub async fn delivery_today(&self, dot_id: &str) -> Result<DeliveryCounts> {
        self.store.get_dot(dot_id).await?;
        let (start, end) = crate::delivery::local_day_bounds(self.clock.now(), &Local);
        self.store
            .delivery_counts(
                dot_id,
                &crate::util::timestamp(start),
                &crate::util::timestamp(end),
            )
            .await
    }

    /// Keep waiting on a quiet run (stuck-run watchdog spec section 4.4): the mark goes and the
    /// quiet clock starts again. `NotFound` for an unknown run, `Conflict` unless it is running
    /// and quiet.
    pub async fn keep_waiting(&self, run_id: &str) -> Result<()> {
        let run = self.store.get_run(run_id).await?;
        if run.status != RunStatus::Running || run.quiet_since.is_none() {
            return Err(Error::Conflict(format!("run {run_id} is not quiet")));
        }
        self.runner.keep_waiting(run_id).await
    }

    /// The allow rules a person may save while approving this call, narrowest first. Empty when
    /// the call can only be approved one at a time.
    pub async fn approval_rule_candidates(&self, approval_id: &str) -> Result<Vec<RuleCandidate>> {
        let approval = self.store.get_approval(approval_id).await?;
        let run = self.store.get_run(&approval.run_id).await?;
        let workspace = run.workspace_path.as_deref().map(Path::new);
        // A call the send-and-delete guard asks about asks while the switch is on, whatever
        // rule is saved: offering one would promise what it cannot do.
        if self
            .approval_guard(approval_id)
            .await?
            .is_some_and(|g| g.always_asks)
        {
            return Ok(Vec::new());
        }
        Ok(allow_candidates(&approval.tool, &approval.input, workspace))
    }

    /// What the send-and-delete guard finds in the call of this approval (it sends or deletes,
    /// and how, for the card), and whether the dot's "Always ask before sending or deleting"
    /// is on, so that it always asks. `None` when the call neither sends nor deletes as far as
    /// Dots can tell.
    pub async fn approval_guard(&self, approval_id: &str) -> Result<Option<ApprovalGuard>> {
        let approval = self.store.get_approval(approval_id).await?;
        let Some(found) = crate::policy::guard::classify(&approval.tool, &approval.input) else {
            return Ok(None);
        };
        let run = self.store.get_run(&approval.run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        Ok(Some(ApprovalGuard {
            kind: found.kind,
            summary: found.summary,
            always_asks: dot.spec.policy.ask_send_delete,
        }))
    }

    /// True when the call of this approval touches a private folder (approval guards spec
    /// section 3): it always asks, and no rule can be saved with its approval.
    pub async fn approval_touches_private(&self, approval_id: &str) -> Result<bool> {
        let approval = self.store.get_approval(approval_id).await?;
        let run = self.store.get_run(&approval.run_id).await?;
        let workspace = run.workspace_path.as_deref().map(Path::new);
        Ok(touches_private(&approval.tool, &approval.input, workspace))
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

    /// The person's answer on a card of instructions a dot proposed (conversational chat spec
    /// section 4). Approving saves them as the editor does (`update_dot`), unless the dot's
    /// instructions changed since it proposed: that is a `Conflict` and the card waits; if they
    /// changed to the proposal itself, they are kept as they are and the approval goes on. Then
    /// the proposal is decided and the dot's chat continues with what the person decided, as
    /// for an app proposal (`decide_app_proposal`). A chat that has not ended is refused.
    pub async fn decide_instruction_proposal(
        &self,
        id: &str,
        approve: bool,
    ) -> Result<(crate::instructions::InstructionProposal, Run)> {
        use crate::instructions::{InstructionStatus, APPROVED, DECIDED_ALREADY, REJECTED, STALE};
        let lock = self.follow_ups.thread_lock();
        let _one_at_a_time = lock.lock().await;
        let p = self.store.get_instruction_proposal(id).await?;
        if p.status != InstructionStatus::Pending {
            return Err(Error::Conflict(DECIDED_ALREADY.into()));
        }
        let asked = self.store.get_run(&p.run_id).await?;
        // A chat adding an app the person accepted is not continued (`decide_app_proposal`).
        if self.follow_ups.deciding(&asked.root_run_id) {
            return Err(Error::Conflict(crate::checkins::DECIDING.into()));
        }
        let thread = self.store.list_thread(&asked.root_run_id).await?;
        if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
            return Err(crate::checkins::thread_busy(active.status));
        }
        let end = thread.last().unwrap_or(&asked);
        let (status, said) = if approve {
            let dot = self.store.get_dot(&p.dot_id).await?;
            let current = dot.spec.instructions.trim();
            // Instructions that already say the proposal (the person typed it in the editor)
            // count as applied: nothing is saved again, and the approval goes on.
            if current != p.instructions {
                if current != p.base {
                    return Err(Error::Conflict(STALE.into()));
                }
                let mut spec = dot.spec;
                spec.instructions = p.instructions.clone();
                self.update_dot(&p.dot_id, spec).await?;
            }
            (InstructionStatus::Approved, APPROVED)
        } else {
            (InstructionStatus::Rejected, REJECTED)
        };
        let message = if end.session_id.is_some() {
            said.to_string()
        } else {
            format!(
                "Earlier you proposed new instructions: \"{}\"\n{said}",
                p.reason
            )
        };
        let new = crate::checkins::continuation(&asked, end, message, None);
        let (decided, run) = self
            .store
            .decide_instruction_proposal(id, status, &new)
            .await?;
        self.runner.enqueued(&run);
        self.checkins
            .instructions()
            .changed(&decided.id, &decided.dot_id);
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
                // Said first, so the refusal names the reason rather than "not offered".
                if self.approval_touches_private(id).await? || pattern_touches_private(&pattern) {
                    return Err(Error::Invalid(PRIVATE_REFUSAL.into()));
                }
                if self
                    .approval_guard(id)
                    .await?
                    .is_some_and(|g| g.always_asks)
                {
                    return Err(Error::Invalid(crate::policy::GUARD_REFUSAL.into()));
                }
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

    /// Records where the decided approval `id` was decided ("phone: <name>", phone view spec
    /// section 7) and announces it. An approval still pending, or one recorded already, is
    /// left as it is.
    pub async fn note_decided_via(&self, id: &str, via: &str) -> Result<()> {
        if self.store.set_decided_via(id, via).await? {
            let approval = self.store.get_approval(id).await?;
            let _ = self.bus.send(RuntimeEvent::ApprovalUpdated { approval });
        }
        Ok(())
    }

    /// Gives pending approvals of one run the same answer, in the order they were asked, and
    /// resumes the run once. With `only`, just the approvals of the run whose ids it lists
    /// (what the person was shown: a snoozed approval of the same run is not decided with
    /// them); without it, every pending approval of the run. An approval that is no longer
    /// pending when its turn comes is left out of the result. No rule can be saved this way.
    pub async fn decide_run_approvals(
        &self,
        run_id: &str,
        only: Option<&[String]>,
        approved: bool,
        note: Option<String>,
    ) -> Result<Vec<Approval>> {
        self.require_for_run(run_id, Permission::AnswerApprovals)
            .await?;
        let mut decided = Vec::new();
        let mut parked = false;
        let mut failure = None;
        let pending = self.store.pending_for_run(run_id).await?;
        let shown = pending
            .into_iter()
            .filter(|a| only.is_none_or(|ids| ids.contains(&a.id)));
        for pending in shown {
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
        self.dot_admin.require_cleanup(dot).await
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
            // A Remove that failed after the values were gone leaves the org listed as damaged;
            // the screens read the list again to show it.
            self.org_updated(org_id);
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
pub(crate) type Viewing = Arc<std::sync::Mutex<Option<String>>>;

pub(crate) fn lock(viewing: &Viewing) -> std::sync::MutexGuard<'_, Option<String>> {
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

/// Search (search spec 2026-11-06-search-design.md).
impl Runtime {
    /// The hits of `query` among the dots and goals of the workspace `workspace_id` of `scope`
    /// (`personal` or an org id), best first; refused for a role that may not view it.
    pub async fn search(
        &self,
        scope: &str,
        workspace_id: &str,
        query: &crate::search::SearchQuery,
    ) -> Result<Vec<crate::search::SearchHit>> {
        let org_id = org_of_scope(scope);
        self.check_workspace(org_id, workspace_id).await?;
        self.require_in(org_id, workspace_id, Permission::View)
            .await?;
        self.store.search(org_id, workspace_id, query).await
    }
}

/// The person's goals (Intents). A goal lives in the account and workspace it was added in;
/// changing one needs a role that may change goals there (`Permission::EditGoals`).
impl Runtime {
    /// The goals of the workspace `workspace_id` of `scope` (`personal` or an org id), oldest
    /// first, and why this PC may not change them.
    pub async fn intents_list(
        &self,
        scope: &str,
        workspace_id: &str,
    ) -> Result<crate::intents::IntentsView> {
        let org_id = org_of_scope(scope);
        self.check_workspace(org_id, workspace_id).await?;
        self.require_in(org_id, workspace_id, Permission::View)
            .await?;
        let refusal = match self
            .require_in(org_id, workspace_id, Permission::EditGoals)
            .await
        {
            Err(Error::NotAllowed(text)) => Some(text),
            Err(e) => return Err(e),
            Ok(()) => None,
        };
        Ok(crate::intents::IntentsView {
            intents: self.store.list_intents(org_id, workspace_id).await?,
            refusal,
        })
    }

    /// Adds a goal to the workspace `workspace_id` of `scope`.
    pub async fn intent_create(
        &self,
        scope: &str,
        workspace_id: &str,
        fields: crate::intents::IntentFields,
    ) -> Result<crate::model::Intent> {
        let org_id = org_of_scope(scope);
        self.check_workspace(org_id, workspace_id).await?;
        self.require_in(org_id, workspace_id, Permission::EditGoals)
            .await?;
        let fields = crate::intents::clean(fields)?;
        self.check_intent_dot(org_id, workspace_id, fields.dot_id.as_deref())
            .await?;
        let intent = self.store.add_intent(org_id, workspace_id, &fields).await?;
        self.intents_updated(&intent);
        Ok(intent)
    }

    /// Replaces the goal's text, due date, urgency and dot.
    pub async fn intent_update(
        &self,
        id: &str,
        fields: crate::intents::IntentFields,
    ) -> Result<crate::model::Intent> {
        let old = self.require_for_intent(id).await?;
        let fields = crate::intents::clean(fields)?;
        self.check_intent_dot(
            old.org_id.as_deref(),
            &old.workspace_id,
            fields.dot_id.as_deref(),
        )
        .await?;
        let intent = self.store.set_intent_fields(id, &fields).await?;
        self.intents_updated(&intent);
        Ok(intent)
    }

    /// Marks the goal done, or open again.
    pub async fn intent_set_done(&self, id: &str, done: bool) -> Result<crate::model::Intent> {
        self.require_for_intent(id).await?;
        let intent = self.store.set_intent_done(id, done).await?;
        self.intents_updated(&intent);
        Ok(intent)
    }

    /// Deletes the goal.
    pub async fn intent_delete(&self, id: &str) -> Result<()> {
        let old = self.require_for_intent(id).await?;
        self.store.delete_intent(id).await?;
        self.intents_updated(&old);
        Ok(())
    }

    /// The goal `id`, once this PC may change goals in its workspace.
    async fn require_for_intent(&self, id: &str) -> Result<crate::model::Intent> {
        let intent = self.store.get_intent(id).await?;
        self.require_in(
            intent.org_id.as_deref(),
            &intent.workspace_id,
            Permission::EditGoals,
        )
        .await?;
        Ok(intent)
    }

    /// A goal may be handed only to a dot of its own account and workspace.
    async fn check_intent_dot(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
        dot_id: Option<&str>,
    ) -> Result<()> {
        let Some(dot_id) = dot_id else {
            return Ok(());
        };
        let dot = match self.store.get_dot(dot_id).await {
            Ok(dot) => dot,
            Err(Error::NotFound(_)) => {
                return Err(Error::Invalid(crate::intents::NO_DOT.into()));
            }
            Err(e) => return Err(e),
        };
        if dot.org_id.as_deref() != org_id || dot.workspace_id != workspace_id {
            return Err(Error::Invalid(crate::intents::OTHER_WORKSPACE.into()));
        }
        Ok(())
    }

    fn intents_updated(&self, intent: &crate::model::Intent) {
        let _ = self.bus.send(RuntimeEvent::IntentsUpdated {
            org_id: intent.org_id.clone(),
            workspace_id: intent.workspace_id.clone(),
        });
    }
}

/// The org of a scope (`personal` or an org id): `None` for Personal.
fn org_of_scope(scope: &str) -> Option<&str> {
    (scope != crate::accounts::PERSONAL).then_some(scope)
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
        hours: Option<crate::model::WatchHours>,
    ) -> Result<crate::model::Watcher> {
        self.require_for_dot(dot_id, Permission::EditDots).await?;
        self.watchers.add(dot_id, name, config, hours).await
    }

    pub async fn watcher_update(
        &self,
        id: &str,
        name: &str,
        config: crate::model::WatcherConfig,
        hours: Option<crate::model::WatchHours>,
    ) -> Result<crate::model::Watcher> {
        self.require_for_watcher(id).await?;
        self.watchers.update(id, name, config, hours).await
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

    /// The Usage page's All Claude use for the period (usage tab spec section 6).
    pub async fn claude_usage(
        &self,
        period: crate::model::UsagePeriod,
    ) -> Result<crate::claude_usage::ClaudeUsage> {
        self.claude_usage.summary(period).await
    }

    /// Reads what Claude Code wrote since the last scan, unless that was within half a minute.
    pub async fn claude_usage_scan(&self) -> Result<crate::claude_usage::ClaudeScan> {
        self.claude_usage.scan(crate::claude_usage::MIN_GAP).await
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
            || !crate::limits::run_is_background(&run)
        {
            return Err(Error::Invalid(crate::limits::NOT_HELD.into()));
        }
        let run = self.store.set_hold_released(run_id).await?;
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run: run.clone() });
        self.runner.nudge();
        Ok(run)
    }

    /// **Run once anyway** (spend caps design): lets one of the person's own runs (a chat, a
    /// manual run, a resume) start past the spending cap that holds it. The run is marked, so no
    /// cap stops it either, and the release is noted on it. Background runs wait for the cap.
    pub async fn run_release_cap(&self, run_id: &str) -> Result<Run> {
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        self.require_dot(&dot, Permission::RunDots).await?;
        if crate::limits::run_is_background(&run) {
            return Err(Error::Invalid(crate::spend::BACKGROUND_WAITS.into()));
        }
        let Some(hit) = self.runner.cap_hold(&run).await? else {
            return Err(Error::Invalid(crate::spend::NOT_CAPPED.into()));
        };
        let run = self.store.set_cap_released(run_id).await?;
        let text = format!("Run once past the cap: {}", hit.text);
        let ev = crate::engine::EngineEvent::Notice { text };
        let rec = self
            .store
            .append_event(run_id, ev.kind(), &serde_json::to_value(&ev)?)
            .await?;
        let _ = self.bus.send(RuntimeEvent::RunEvent { event: rec });
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run: run.clone() });
        self.runner.nudge();
        Ok(run)
    }

    /// A topic's context meter and its question (context rules spec sections 3 and 6).
    pub async fn thread_context(
        &self,
        root_run_id: &str,
    ) -> Result<crate::context_rules::ThreadContext> {
        use crate::context_rules as cr;
        let root = self.store.get_run(root_run_id).await?;
        let dot = self.store.get_dot(&root.dot_id).await?;
        let rules = dot
            .spec
            .context_rules
            .unwrap_or(self.store.get_settings().await?.context_rules);
        let last = self.store.thread_fill(&root.root_run_id).await?;
        let state = self.store.thread_state(&root.root_run_id).await?;
        let fill = last.as_ref().map(|l| l.fill);
        let model = last
            .as_ref()
            .and_then(|l| l.model.clone())
            .unwrap_or_else(|| dot.spec.model.clone());
        let window = cr::window_for(Some(&model), last.as_ref().and_then(|l| l.window));
        Ok(cr::ThreadContext {
            root_run_id: root.root_run_id.clone(),
            fill,
            window,
            rules,
            keep_going: state.keep_going,
            request: state.request,
            asks_fresh: dot.spec.engine == crate::model::EngineKind::Claude
                && cr::asks_fresh(&rules, fill, window, state.keep_going, state.request),
        })
    }

    /// The person's answer for a topic (context rules spec section 6): compact or start fresh
    /// on its next turn, or keep going. Only a Claude dot's topic takes one.
    pub async fn thread_context_choose(
        &self,
        root_run_id: &str,
        choice: crate::context_rules::Choice,
    ) -> Result<crate::context_rules::ThreadContext> {
        use crate::context_rules::{Choice, Request};
        let root = self.store.get_run(root_run_id).await?;
        let dot = self.store.get_dot(&root.dot_id).await?;
        self.require_dot(&dot, Permission::RunDots).await?;
        if dot.spec.engine != crate::model::EngineKind::Claude {
            return Err(Error::Invalid(
                "Only a Claude dot's conversation can be compacted or started fresh.".into(),
            ));
        }
        let root_id = &root.root_run_id;
        match choice {
            Choice::Compact => {
                self.store
                    .set_thread_request(root_id, Some(Request::Compact))
                    .await?
            }
            Choice::Fresh => {
                self.store
                    .set_thread_request(root_id, Some(Request::Fresh))
                    .await?
            }
            Choice::KeepGoing => self.store.set_keep_going(root_id, true).await?,
        }
        self.thread_context(root_id).await
    }

    /// A run's compactions and fresh starts, with its dot's totals (context rules spec
    /// section 7).
    pub async fn run_context_report(
        &self,
        run_id: &str,
    ) -> Result<crate::context_rules::RunContextReport> {
        let run = self.store.get_run(run_id).await?;
        let dot = self.store.get_dot(&run.dot_id).await?;
        let rules = dot
            .spec
            .context_rules
            .unwrap_or(self.store.get_settings().await?.context_rules);
        let (dot_compactions, dot_clears) = self.store.context_counts(&dot.id).await?;
        Ok(crate::context_rules::RunContextReport {
            events: self.store.list_context_events(run_id).await?,
            dot_compactions,
            dot_clears,
            rules,
        })
    }

    /// Today's spend against the spending caps (spend caps design).
    pub async fn spend_today(&self) -> Result<crate::spend::SpendToday> {
        crate::spend::today(&self.store, chrono::Utc::now()).await
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

    /// Claude's usage limits. Tests record a reading through it.
    #[doc(hidden)]
    pub fn limits(&self) -> &Arc<crate::limits::Limits> {
        &self.limits
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
        // Runs waiting for a role, which the limits do not hold, and runs waiting for a
        // spending cap, read once for them all (spend caps design).
        let usage_held: HashSet<String> = held.iter().map(|h| h.run_id.clone()).collect();
        let mut ledger = crate::spend::Ledger::load(&self.store).await?;
        for (run, _) in self.store.queued_with_engine(false).await? {
            if !run.cap_released && !usage_held.contains(&run.id) {
                if let Some(hit) = ledger.hit_for(&self.store, &run.dot_id).await? {
                    held.push(crate::model::HeldRun {
                        dot_name: names.get(&run.dot_id).cloned().unwrap_or_default(),
                        dot_id: run.dot_id,
                        run_id: run.id,
                        trigger: run.trigger,
                        queued_at: run.queued_at,
                        starts_after: None,
                        reason: crate::model::HoldReason::SpendCap {
                            text: hit.text,
                            scope: hit.scope,
                        },
                    });
                    continue;
                }
            }
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

/// Send later (send later spec section 4): scheduling a chat message, Send now, Edit, Cancel,
/// and the sender that sends what is due.
impl Runtime {
    /// Schedules `text` to the dot at `due_at` (RFC 3339, a minute to a year ahead), to the
    /// topic of `root_run_id` or, with `None`, as a new topic. Wakes the sender.
    pub async fn schedule_message(
        &self,
        dot_id: &str,
        text: &str,
        root_run_id: Option<&str>,
        due_at: &str,
    ) -> Result<ScheduledMessage> {
        self.schedule_message_with_files(dot_id, text, root_run_id, due_at, &[])
            .await
    }

    /// `schedule_message` with staged files (attachments spec section 4.6): they are held for
    /// the message until it is sent, and their names are fixed then. It may be files only.
    pub async fn schedule_message_with_files(
        &self,
        dot_id: &str,
        text: &str,
        root_run_id: Option<&str>,
        due_at: &str,
        attachment_ids: &[String],
    ) -> Result<ScheduledMessage> {
        if attachment_ids.len() > crate::chat_files::MAX_FILES {
            return Err(Error::Invalid(crate::chat_files::TOO_MANY.into()));
        }
        if text.trim().is_empty() && attachment_ids.is_empty() {
            return Err(Error::Invalid("message must not be empty".into()));
        }
        let files = self.files_to_send(dot_id, attachment_ids, None).await?;
        let dot = self.store.get_dot(dot_id).await?;
        self.require_run(&dot).await?;
        let now = self.clock.now();
        let due_at = crate::send_later::check_due(due_at, now)?;
        let root = match root_run_id {
            Some(id) => {
                let run = self.store.get_run(id).await?;
                if run.dot_id != dot_id {
                    return Err(Error::Invalid(format!(
                        "run {id} belongs to a different dot"
                    )));
                }
                Some(run.root_run_id)
            }
            None => None,
        };
        let mut message = self
            .store
            .insert_scheduled_with_files(
                dot_id,
                root.as_deref(),
                text,
                &due_at,
                now,
                attachment_ids,
            )
            .await?;
        message.attachments = files
            .iter()
            .map(|f| StagedAttachment {
                id: f.id.clone(),
                name: f.file_name.clone(),
                size: f.size,
            })
            .collect();
        self.scheduled_updated(&message);
        self.send_later_wake.notify_one();
        Ok(message)
    }

    /// The dot's pending scheduled messages (scheduled, sending or failed), in due order, each
    /// with the files it waits with. A read, so it needs only `View`, as the conversation does:
    /// a Viewer sees the cards too.
    pub async fn scheduled_messages(&self, dot_id: &str) -> Result<Vec<ScheduledMessage>> {
        self.require_for_dot(dot_id, Permission::View).await?;
        let mut messages = self.store.pending_scheduled_messages(dot_id).await?;
        for m in &mut messages {
            m.attachments = self
                .store
                .scheduled_attachments(&m.id)
                .await?
                .into_iter()
                .map(|f| StagedAttachment {
                    id: f.id,
                    name: f.file_name,
                    size: f.size,
                })
                .collect();
        }
        Ok(messages)
    }

    /// Edit: new text and time for a message that is scheduled or failed; it is scheduled
    /// again. `Conflict` once it is being sent, sent or cancelled.
    pub async fn edit_scheduled_message(
        &self,
        id: &str,
        text: &str,
        due_at: &str,
    ) -> Result<ScheduledMessage> {
        // Edit changes the text only: a message with files may have none.
        if text.trim().is_empty() && self.store.scheduled_attachments(id).await?.is_empty() {
            return Err(Error::Invalid("message must not be empty".into()));
        }
        let current = self.store.get_scheduled_message(id).await?;
        self.require_for_dot(&current.dot_id, Permission::RunDots)
            .await?;
        let now = self.clock.now();
        let due_at = crate::send_later::check_due(due_at, now)?;
        let Some(message) = self
            .store
            .edit_scheduled_message(id, text, &due_at, now)
            .await?
        else {
            return Err(self.not_changeable(id).await);
        };
        self.scheduled_updated(&message);
        self.send_later_wake.notify_one();
        Ok(message)
    }

    /// Cancel: a message that is scheduled or failed is cancelled at once (the row is kept).
    /// `Conflict` once it is being sent, sent or cancelled.
    pub async fn cancel_scheduled_message(&self, id: &str) -> Result<ScheduledMessage> {
        let current = self.store.get_scheduled_message(id).await?;
        self.require_for_dot(&current.dot_id, Permission::RunDots)
            .await?;
        let Some(message) = self
            .store
            .cancel_scheduled_message(id, self.clock.now())
            .await?
        else {
            return Err(self.not_changeable(id).await);
        };
        self.release_staged(id).await;
        self.scheduled_updated(&message);
        Ok(message)
    }

    /// Send now: a message that is scheduled or failed is due now (so never late) and sent at
    /// once. Returns it sent, still scheduled with a waiting reason (it goes when its topic is
    /// free), or failed. `Conflict` once it is being sent, sent or cancelled.
    pub async fn send_scheduled_now(&self, id: &str) -> Result<ScheduledMessage> {
        let current = self.store.get_scheduled_message(id).await?;
        let dot = self.store.get_dot(&current.dot_id).await?;
        self.require_run(&dot).await?;
        let Some(message) = self.store.reset_for_send_now(id, self.clock.now()).await? else {
            return Err(self.not_changeable(id).await);
        };
        self.scheduled_updated(&message);
        self.send_scheduled(message, By::SendNow).await
    }

    /// One pass of the sender: every scheduled message that is due, in due order. A message
    /// that fails to be handled is logged and the pass goes on. Returns the rows that changed.
    pub async fn send_due(&self) -> Result<Vec<ScheduledMessage>> {
        // A send whose run was stored but whose row was not marked sent (a failed write) is
        // sent: the row follows its run now, not only at the next start.
        let mut changed = self
            .store
            .settle_sending_with_runs(self.clock.now())
            .await?;
        for message in &changed {
            self.scheduled_updated(message);
        }
        let now = timestamp(self.clock.now());
        for message in self.store.due_scheduled_messages(&now).await? {
            match self.send_scheduled(message.clone(), By::Sender).await {
                Ok(row) if row != message => changed.push(row),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(id = %message.id, "sending a scheduled message failed: {e}")
                }
            }
        }
        Ok(changed)
    }

    /// Sends one due message (send later spec section 4.2): refused for good, it fails; else
    /// its target is read from its topic, and it is claimed and sent through the chat path.
    async fn send_scheduled(&self, message: ScheduledMessage, by: By) -> Result<ScheduledMessage> {
        let dot = self.store.get_dot(&message.dot_id).await?;
        if let Err(e) = self.require_run(&dot).await {
            let failed = self
                .store
                .mark_scheduled_failed(
                    &message.id,
                    &crate::send_later::first_line(&e),
                    self.clock.now(),
                )
                .await?;
            return self.settled(&message.id, failed).await;
        }
        let target = self.send_target(&dot, &message).await?;
        self.send_scheduled_by(message, target, by).await
    }

    /// Steps 4 to 6 of sending (send later spec section 4.2), for a target already chosen: a
    /// wait is recorded; otherwise the message is claimed and goes through the chat path. A
    /// `Conflict` there (the thread moved since the target was read) means wait, not fail.
    /// Tests call it with a stale target.
    #[doc(hidden)]
    pub async fn send_scheduled_to(
        &self,
        message: ScheduledMessage,
        target: SendTarget,
    ) -> Result<ScheduledMessage> {
        self.send_scheduled_by(message, target, By::Sender).await
    }

    /// `send_scheduled_to`, for the sender or for Send now.
    async fn send_scheduled_by(
        &self,
        message: ScheduledMessage,
        target: SendTarget,
        by: By,
    ) -> Result<ScheduledMessage> {
        let now = self.clock.now();
        let (parent, fallback) = match target {
            SendTarget::Wait(wait) => {
                let waiting = self
                    .store
                    .set_scheduled_waiting(&message.id, wait, now)
                    .await?;
                return self.settled(&message.id, waiting).await;
            }
            SendTarget::Reply { parent_run_id } => (Some(parent_run_id), false),
            SendTarget::New { fallback } => (None, fallback),
        };
        // Lost to an Edit, a Cancel or another send meanwhile: nothing more to do. A Send now
        // that lost to another send or a Cancel is refused, as one that came later would be
        // (send later spec section 4.4).
        let Some(claimed) = self.store.claim_scheduled_message(&message.id, now).await? else {
            let current = self.store.get_scheduled_message(&message.id).await?;
            return match (by, current.status) {
                (
                    By::SendNow,
                    ScheduledStatus::Sending | ScheduledStatus::Sent | ScheduledStatus::Cancelled,
                ) => Err(crate::send_later::not_changeable(current.status)),
                _ => Ok(current),
            };
        };
        let extra = self.chat_extra_for(&claimed, now, fallback).await?;
        let sent = self
            .chat_with(&claimed.dot_id, &claimed.text, parent.as_deref(), extra)
            .await;
        let changed = match sent {
            Ok(run) => {
                self.store
                    .mark_scheduled_sent(&claimed.id, &run.id, now)
                    .await?
            }
            // A held file is gone: waiting would not bring it back.
            Err(Error::Conflict(m)) if m == crate::chat_files::GONE => {
                self.store
                    .mark_scheduled_failed(&claimed.id, &m, now)
                    .await?
            }
            Err(Error::Conflict(_)) => {
                self.store
                    .set_scheduled_waiting(&claimed.id, ScheduledWait::Busy, now)
                    .await?
            }
            // Claude's limit was reached after the target was read: wait for it to reset.
            Err(Error::Invalid(m)) if crate::limits::is_limit_reached(&m) => {
                self.store
                    .set_scheduled_waiting(&claimed.id, ScheduledWait::UsageLimit, now)
                    .await?
            }
            Err(e) => {
                self.store
                    .mark_scheduled_failed(&claimed.id, &crate::send_later::first_line(&e), now)
                    .await?
            }
        };
        self.settled(&claimed.id, changed).await
    }

    /// Where a due message goes now (send later spec section 4.2, steps 2 and 3). Read without
    /// the chat lock: the chat path checks again under it.
    async fn send_target(&self, dot: &Dot, message: &ScheduledMessage) -> Result<SendTarget> {
        let target = match &message.root_run_id {
            None => SendTarget::New { fallback: false },
            Some(root) => match self.store.get_run(root).await {
                Err(Error::NotFound(_)) => SendTarget::New { fallback: true },
                Err(e) => return Err(e),
                Ok(run) if run.dot_id != dot.id => SendTarget::New { fallback: true },
                Ok(run) => {
                    let thread = self.store.list_thread(&run.root_run_id).await?;
                    if let Some(active) = thread.iter().find(|r| !r.status.is_terminal()) {
                        return Ok(SendTarget::Wait(match active.status {
                            RunStatus::AwaitingApproval => ScheduledWait::Approval,
                            _ => ScheduledWait::Busy,
                        }));
                    }
                    if self.follow_ups.deciding(&run.root_run_id) {
                        return Ok(SendTarget::Wait(ScheduledWait::Busy));
                    }
                    match crate::send_later::thread_end(&thread) {
                        Some(end)
                            if end.session_id.is_some()
                                && !self.store.in_ended_step_thread(end).await? =>
                        {
                            SendTarget::Reply {
                                parent_run_id: end.id.clone(),
                            }
                        }
                        _ => SendTarget::New { fallback: true },
                    }
                }
            },
        };
        if self.at_the_limit(&dot.id).await? {
            return Ok(SendTarget::Wait(ScheduledWait::UsageLimit));
        }
        Ok(target)
    }

    /// Whether the dot is a Claude dot and Claude's limit is reached: the chat path would
    /// refuse its message (`refuse_at_the_limit`).
    async fn at_the_limit(&self, dot_id: &str) -> Result<bool> {
        let dot = self.store.get_dot(dot_id).await?;
        Ok(dot.spec.engine == EngineKind::Claude && self.limits.rejected_until().is_some())
    }

    /// What a scheduled message's chat run carries besides its text (send later spec section
    /// 4.3): the one place they are built, which attachments extends with its files.
    async fn chat_extra_for(
        &self,
        message: &ScheduledMessage,
        sent_at: DateTime<Utc>,
        new_topic: bool,
    ) -> Result<ChatExtra> {
        // The files it waits with go with it (attachments spec section 4.6).
        let attachment_ids = self
            .store
            .scheduled_attachments(&message.id)
            .await?
            .into_iter()
            .map(|a| a.id)
            .collect();
        Ok(ChatExtra {
            send_later: Some(SentLater {
                id: message.id.clone(),
                due_at: message.due_at.clone(),
                late: crate::send_later::is_late(&message.due_at, sent_at),
                new_topic,
            }),
            attachment_ids,
            scheduled_id: Some(message.id.clone()),
            carry_from: None,
        })
    }

    /// Frees the files a cancelled message held: their rows and stored copies (attachments
    /// spec section 4.6). A failure is logged; the startup sweep is the backstop.
    async fn release_staged(&self, id: &str) {
        match self.store.delete_scheduled_attachments(id).await {
            Ok(ids) => {
                for a in ids {
                    self.chat_files.remove(&a);
                }
            }
            Err(e) => {
                tracing::warn!(scheduled = %id, "removing a cancelled message's files failed: {e}")
            }
        }
    }

    /// The refusal for a message no longer scheduled or failed, by what it is now.
    async fn not_changeable(&self, id: &str) -> Error {
        match self.store.get_scheduled_message(id).await {
            Ok(now) => crate::send_later::not_changeable(now.status),
            Err(e) => e,
        }
    }

    /// The row after a change: the changed row, announced, or the row as it stands when
    /// nothing changed.
    async fn settled(
        &self,
        id: &str,
        changed: Option<ScheduledMessage>,
    ) -> Result<ScheduledMessage> {
        match changed {
            Some(message) => {
                self.scheduled_updated(&message);
                Ok(message)
            }
            None => self.store.get_scheduled_message(id).await,
        }
    }

    fn scheduled_updated(&self, message: &ScheduledMessage) {
        let _ = self.bus.send(RuntimeEvent::ScheduledMessageUpdated {
            message: message.clone(),
        });
    }

    /// Starts the sender (send later spec section 4.5): a pass at once, then one at the
    /// earliest future due time or after 30 seconds of the clock, whichever is sooner, or when
    /// a message is scheduled or edited. Stops at shutdown or when the runtime is dropped (it
    /// is held weakly). The app starts it once the runtime is in an `Arc`.
    pub fn spawn_send_later(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let runtime = Arc::downgrade(self);
        let clock = self.clock.clone();
        let shutdown = self.shutdown.clone();
        let wake = self.send_later_wake.clone();
        tokio::spawn(async move {
            loop {
                let Some(rt) = runtime.upgrade() else {
                    return;
                };
                if let Err(e) = rt.send_due().await {
                    tracing::warn!("sending scheduled messages failed: {e}");
                }
                let now = clock.now();
                let mut next = now + crate::send_later::SEND_EVERY;
                match rt.store.next_scheduled_due(&timestamp(now)).await {
                    Ok(Some(due)) => {
                        if let Ok(due) = DateTime::parse_from_rfc3339(&due) {
                            next = next.min(due.with_timezone(&Utc));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("reading the next scheduled message failed: {e}"),
                }
                drop(rt);
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return,
                    _ = wake.notified() => {}
                    _ = clock.sleep_until(next) => {}
                }
            }
        })
    }
}

/// Who sends a scheduled message: the sender's pass, or the person's Send now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    Sender,
    SendNow,
}

/// A Personal workspace's name, trimmed; it needs one.
fn personal_workspace_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("A workspace needs a name.".into()));
    }
    Ok(name.to_string())
}

/// The project graph of a dot, for the Graph screen (project graph spec section 10).
impl Runtime {
    /// The graphs, and the project of the dot `dot_id`. `NotFound` when the dot's folder is
    /// missing.
    async fn graph_project(
        &self,
        dot_id: &str,
    ) -> Result<(crate::graph::Graphs, crate::graph::Project)> {
        let dot = self.store.get_dot(dot_id).await?;
        let graphs = self
            .runner
            .graphs()
            .cloned()
            .ok_or_else(|| Error::Other("the project graphs are not running".into()))?;
        let project = crate::graph::Project::resolve(Path::new(&dot.spec.workdir))
            .await
            .ok_or_else(|| {
                Error::NotFound(format!("folder {} no longer exists", dot.spec.workdir))
            })?;
        Ok((graphs, project))
    }

    /// The state of the dot's project graph.
    pub async fn graph_status(&self, dot_id: &str) -> Result<crate::graph::GraphStatus> {
        let (graphs, project) = self.graph_project(dot_id).await?;
        blocking(move || graphs.status(&project)).await
    }

    /// What the Graph screen draws: the whole graph, one community of it, or one node per
    /// community when it is large.
    pub async fn graph_view(
        &self,
        dot_id: &str,
        community: Option<u32>,
    ) -> Result<crate::graph::GraphView> {
        let (graphs, project) = self.graph_project(dot_id).await?;
        blocking(move || graphs.view(&project, community)).await
    }

    /// One node and its neighbours; `NotFound` for a node the graph no longer has.
    pub async fn graph_node(&self, dot_id: &str, id: &str) -> Result<crate::graph::NodeDetail> {
        let (graphs, project) = self.graph_project(dot_id).await?;
        let id = id.to_string();
        blocking(move || graphs.node(&project, &id))
            .await?
            .ok_or_else(|| Error::NotFound("the graph no longer has this node".into()))
    }

    /// The nodes whose name or path matches `query`, best first.
    pub async fn graph_search(
        &self,
        dot_id: &str,
        query: &str,
    ) -> Result<Vec<crate::graph::ViewNode>> {
        let (graphs, project) = self.graph_project(dot_id).await?;
        let query = query.to_string();
        blocking(move || graphs.search(&project, &query)).await
    }

    /// Starts a build of the dot's project graph in the background.
    pub async fn graph_rebuild(&self, dot_id: &str) -> Result<crate::graph::GraphStatus> {
        let (graphs, project) = self.graph_project(dot_id).await?;
        graphs.rebuild(&project);
        blocking(move || graphs.status(&project)).await
    }

    /// Writes graphify's `graph.json` into the project's `graphify-out` folder, and returns
    /// its path.
    pub async fn graph_export(&self, dot_id: &str) -> Result<PathBuf> {
        self.require_for_dot(dot_id, Permission::EditDots).await?;
        let (graphs, project) = self.graph_project(dot_id).await?;
        blocking(move || graphs.export(&project))
            .await?
            .map_err(Error::Invalid)
    }

    /// The file of a node, inside the project, for Show in folder.
    pub async fn graph_file(&self, dot_id: &str, id: &str) -> Result<PathBuf> {
        let (graphs, project) = self.graph_project(dot_id).await?;
        let id = id.to_string();
        blocking(move || graphs.file_path(&project, &id))
            .await?
            .ok_or_else(|| Error::NotFound("this node's file no longer exists".into()))
    }
}

/// Runs `f` on the blocking pool.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("the graph task stopped: {e}")))
}
