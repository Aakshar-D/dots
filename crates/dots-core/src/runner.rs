use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use serde_json::{json, Value};

use crate::approvals::grant_key;
use crate::checkins::{CheckIns, Tool as CheckInTool};
use crate::checkpoint::Checkpoints;
use crate::context_rules::{self, ContextRules, SeedSection};
use crate::delivery::{wait_reason, WaitFacts};
use crate::engine::claude::once::OnceUsage;
use crate::engine::{Engine, EngineEvent, PassUsage, RunContext};
use crate::events::{Bus, RuntimeEvent};
use crate::limits::{run_is_background, Admit, Limits, QueuedRun, STOPPED_BY_LIMIT};
use crate::model::{
    effective_quiet_limit, ApprovalStatus, Delivery, Dot, EngineKind, NewRun, Run, RunStatus,
    TriggerKind, WorkspaceMode,
};
use crate::prompt::{build_prompt_full, context_tool_name, MakerPart, PromptParts};
use crate::result_check::{CheckEvent, ResultCheck, ResultChecker};
use crate::run_tools::{CloseOnDrop, OpenRequest, RunTools};
use crate::spend::STOPPED_BY_CAP;
use crate::store::{NewContextEvent, Store};
use crate::util::{ct_eq, new_token};
use crate::vault::{DotScope, Vault};
use crate::workspace::{default_branch_ref, Prepared, WorkspaceManager};
use crate::{Error, Result};

/// The project graph a run of `dot` working in `prepared` asks (project graph spec sections
/// 5 and 8): for a linked worktree its own graph, seeded from the main checkout's; else the
/// checkout's. A first build starts in the background; the run never waits for one. `None`
/// when the dot's folder is missing.
async fn run_graph(
    graphs: &crate::graph::Graphs,
    dot: &Dot,
    prepared: &Prepared,
) -> Option<crate::run_tools::RunGraph> {
    use crate::graph::{Project, Scope};
    let workdir = std::path::Path::new(&dot.spec.workdir);
    let base = Project::resolve(workdir).await?;
    let prefix = base.relative(workdir).unwrap_or_default();
    graphs.ensure(&base);
    let own = (dot.spec.workspace_mode == WorkspaceMode::Worktree && prepared.branch.is_some())
        .then(|| Project::worktree(&prepared.path))
        .flatten();
    let project = match own {
        Some(wt) => {
            let (g, b, w) = (graphs.clone(), base.clone(), wt.clone());
            let _ = tokio::task::spawn_blocking(move || g.seed_worktree(&b, &w)).await;
            graphs.refresh(&wt);
            wt
        }
        None => base,
    };
    // Only a policy with a `Read` deny rule hides files; without one the stored map serves.
    let denies_reads = dot.spec.policy.rules.iter().any(|r| {
        r.action == crate::policy::Action::Deny && r.tool.trim_start().starts_with("Read")
    });
    let scope = Scope {
        prefix,
        policy: denies_reads.then(|| dot.spec.policy.clone()),
        root: project.root.clone(),
        workspace: prepared.path.clone(),
    };
    Some(crate::run_tools::RunGraph {
        graphs: graphs.clone(),
        project,
        scope,
    })
}

/// The name the run's model knows a policy tool by: the `Context` tool is `mcp__dots__context`
/// on the Claude engine, `context` on the local one, and the check-in tools likewise
/// (`checkins::Tool::model_name`); a `Memory` call is the memory tool its `input` is for
/// (`memory::Tool::for_input`), an `App` call the app tool its input names
/// (`apps::run::model_name`) and a `MakeDots` call the tool for making dots its input names
/// (`making::tools::Tool::for_input`); every other tool keeps its name.
fn model_tool_name<'a>(tool: &'a str, input: &Value, engine: EngineKind) -> Cow<'a, str> {
    if tool == "Context" {
        Cow::Borrowed(context_tool_name(engine))
    } else if tool == crate::graph::POLICY_NAME {
        Cow::Borrowed(crate::graph::model_name(engine))
    } else if let Some(check_in) = CheckInTool::from_policy_name(tool) {
        Cow::Borrowed(check_in.model_name(engine))
    } else if tool == crate::memory::POLICY_NAME {
        Cow::Borrowed(crate::memory::Tool::for_input(input).model_name(engine))
    } else if tool == crate::team_tools::POLICY_NAME {
        Cow::Borrowed(crate::team_tools::Tool::for_input(input).model_name(engine))
    } else if tool == crate::instructions::POLICY_NAME {
        Cow::Borrowed(crate::instructions::model_name(engine))
    } else if tool == crate::making::tools::POLICY_NAME {
        // The input names which of the tools for making dots it is.
        match crate::making::tools::Tool::for_input(input) {
            Some(making) => Cow::Borrowed(making.model_name(engine)),
            None => Cow::Borrowed(tool),
        }
    } else if tool == crate::apps::PROPOSE_POLICY {
        // `copy_app` names an app; `propose_app` does not.
        let name = if input.get("app").is_some() {
            crate::apps::propose::COPY_TOOL
        } else {
            crate::apps::propose::TOOL
        };
        Cow::Owned(match engine {
            EngineKind::Claude => format!("mcp__dots__{name}"),
            EngineKind::Local => name.to_string(),
        })
    } else if tool == crate::apps::APP_POLICY {
        let part = |key: &str| input[key].as_str().unwrap_or_default().to_string();
        let key = crate::apps::manifest::tool_key(&part("app"), &part("tool"));
        Cow::Owned(crate::apps::run::model_name(&key, engine))
    } else {
        Cow::Borrowed(tool)
    }
}

/// How long after the usage report that reached a spending cap the run is stopped: the
/// engine's own end, reported with that usage, comes first.
const CAP_STOP_AFTER: Duration = Duration::from_millis(500);

/// How every resume prompt ends.
const CONTINUE: &str = "Continue the task, then end with your summary.";

/// The longest summary line a commit message of Dots takes, in characters.
const COMMIT_LINE: usize = 72;

/// What a run of a workflow step takes from its workflow run (teams spec sections 6.1 and 8):
/// the project's repo, where its worktree is made; the project's default branch, if it names
/// one, and its remote; and the step's name, for the commit Dots makes.
struct StepContext {
    repo: String,
    default_branch: Option<String>,
    remote: String,
    step_name: String,
}

/// The dot as its run's workspace sees it: a run of a workflow step works in the project's
/// repo, not in its dot's folder.
fn workspace_dot(dot: &Dot, step: Option<&StepContext>) -> Dot {
    let mut dot = dot.clone();
    if let Some(step) = step {
        dot.spec.workdir = step.repo.clone();
    }
    dot
}

/// The commit message of a step's leftover work: `<step name>: <first line of the summary>`.
fn commit_message(step: &StepContext, summary: Option<&str>) -> String {
    let line = summary
        .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("work in progress");
    let line: String = line.chars().take(COMMIT_LINE).collect();
    format!("{}: {line}", step.step_name)
}

/// How long the first attempt of a run Auto starts again may take to end (`Runner::fall_back`).
const FALLBACK_DRAIN: Duration = Duration::from_secs(15);

/// Where Auto goes when the CLI cannot use its pick: the model it tried first, and the tier and
/// model it tries next, for the same reason.
struct Fallback {
    first: String,
    tier: crate::model::ModelTier,
    model: String,
    reason: String,
}

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub max_concurrent: usize,
    pub mcp_url: String,
    pub worktrees_dir: PathBuf,
    pub cancel_grace: Duration,
    /// Where a Claude-engine run's plugin folders go (`<data_dir>/plugins`).
    pub plugins_dir: PathBuf,
    /// How long one second of a quiet limit lasts: `Duration::from_secs(1)`; tests shorten it.
    pub quiet_unit: Duration,
}

struct Active {
    dot_id: String,
    /// A run of a workflow step: it does not keep its dot's other runs waiting, nor waits for
    /// them (teams spec section 8).
    workflow: bool,
    cancel: CancellationToken,
    cancelled_by_user: Arc<AtomicBool>,
    /// Tells the run's loop the person chose Keep waiting (stuck-run watchdog spec section 4.2).
    keep_waiting: Arc<Notify>,
    /// A background run on the Claude engine: while held runs drain, one such at a time.
    background_claude: bool,
}

pub struct Runner {
    store: Store,
    bus: Bus,
    engines: HashMap<EngineKind, Arc<dyn Engine>>,
    workspaces: WorkspaceManager,
    cfg: RunnerConfig,
    active: Mutex<HashMap<String, Active>>,
    secrets: Mutex<HashMap<String, String>>,
    /// Each running run's open tools, by run id, for the MCP gateway.
    tools: Mutex<HashMap<String, Arc<RunTools>>>,
    wake: Notify,
    resume_lock: tokio::sync::Mutex<()>,
    /// While set, the dispatcher starts no queued run (`set_held`).
    held: AtomicBool,
    /// Held for each dispatch pass, so `set_held` can wait for a pass in flight.
    pass: tokio::sync::Mutex<()>,
    /// The check-in tools Dots' own `dots` MCP server serves (check-ins spec section 2), set
    /// once by the runtime before any run starts (`set_checkins`).
    checkins: OnceLock<CheckIns>,
    /// Where runs resolve their dots' variables, set once by the runtime (`set_vault`). A
    /// runner without one resolves from an empty vault in memory.
    vault: OnceLock<Vault>,
    /// The dots' browsers (browser spec section 2), set once by the runtime before any run
    /// starts (`set_browsers`).
    browsers: OnceLock<Arc<crate::browser::Browsers>>,
    /// The installed apps (app library spec section 6), set once by the runtime before any run
    /// starts (`set_apps`).
    apps: OnceLock<crate::apps::Apps>,
    /// Which dots are acting, for the watchers (watchers spec section 4.1), set once by the
    /// runtime (`set_acting`). The runner tells it when it takes a run and lets it go.
    acting: OnceLock<Arc<crate::watchers::ActingTracker>>,
    /// Where watchers staged their runs' files (watchers spec section 4.2), set once by the
    /// runtime (`set_watcher_files`).
    watcher_files: OnceLock<crate::watchers::files::WatcherFiles>,
    /// The stored copies of files attached to chat messages (attachments spec section 4.4),
    /// set once by the runtime (`set_chat_files`).
    chat_files: OnceLock<crate::chat_files::ChatFiles>,
    /// Claude's usage limits (usage and limits spec section 4), set once by the runtime
    /// (`set_limits`). Without them no run is held.
    limits: OnceLock<Arc<Limits>>,
    /// What gives a succeeded Claude run's answer its result check (result check spec), set
    /// once by the runtime (`set_checker`). Without one no run is checked.
    checker: OnceLock<Arc<dyn ResultChecker>>,
    /// The projects' code graphs (project graph spec), set once by the runtime
    /// (`set_graphs`). Without them no run gets the `graph` tool.
    graphs: OnceLock<crate::graph::Graphs>,
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
            tools: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            resume_lock: tokio::sync::Mutex::new(()),
            held: AtomicBool::new(false),
            pass: tokio::sync::Mutex::new(()),
            checkins: OnceLock::new(),
            vault: OnceLock::new(),
            browsers: OnceLock::new(),
            apps: OnceLock::new(),
            acting: OnceLock::new(),
            watcher_files: OnceLock::new(),
            chat_files: OnceLock::new(),
            limits: OnceLock::new(),
            checker: OnceLock::new(),
            graphs: OnceLock::new(),
        })
    }

    /// What checks the runs' answers (result check spec). Set once, before any run starts.
    pub fn set_checker(&self, checker: Arc<dyn ResultChecker>) {
        let _ = self.checker.set(checker);
    }

    /// The usage limits the dispatcher asks before it starts a queued run. Set once, before
    /// any run starts.
    pub fn set_limits(&self, limits: Arc<Limits>) {
        let _ = self.limits.set(limits);
    }

    /// Wakes the dispatcher: something the limits depend on changed (a reading, a share, a
    /// reset that passed, a run released from the hold).
    pub fn nudge(&self) {
        self.wake.notify_one();
    }

    /// Every queued run as the limits see it, oldest first.
    /// A run held for a role is not among them: it is no usage hold (members spec section 6.3).
    pub async fn queued_runs(&self) -> Result<Vec<QueuedRun>> {
        let mut queued = Vec::new();
        for (run, engine) in self.store.queued_with_engine(false).await? {
            if self.role_hold(&run).await?.is_none() {
                queued.push(queued_run(&run, engine));
            }
        }
        Ok(queued)
    }

    /// Why a queued background run of an account's dot may not start now: its account and the
    /// words for the Inbox, when this PC's access to the dot's account and workspace does not
    /// let it run dots (members spec §6.3).
    /// Manual and chat runs are refused when the person asks, so none is held here. `None` when
    /// the run may start, or is no run of an account's dot.
    /// A queued run of any trigger is also held while its dot's policy reaches past the policy
    /// ceiling of a Member in its workspace (guardrails spec §4.3), with that text.
    pub async fn role_hold(&self, run: &Run) -> Result<Option<(String, String)>> {
        let dot = match self.store.get_dot(&run.dot_id).await {
            Ok(dot) => dot,
            Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some(org_id) = dot.org_id.as_deref() else {
            return Ok(None);
        };
        let orgs = self.vault.get().and_then(|v| v.orgs());
        let run_dots = crate::accounts::Permission::RunDots;
        let access = crate::accounts::access_dot(orgs, &dot);
        if access.allows(run_dots) || !run_is_background(run) {
            return Ok(self.ceiling_hold(&dot).await);
        }
        let account = match orgs {
            Some(orgs) => orgs.summary(org_id).await.map(|s| s.name),
            None => None,
        }
        .unwrap_or_else(|| org_id.to_string());
        let text = match access {
            // The account role would run it: the role in the dot's workspace does not.
            crate::accounts::Access::Role(_)
                if crate::accounts::access(orgs, Some(org_id)).allows(run_dots) =>
            {
                let workspace = crate::accounts::workspace_name(orgs, org_id, &dot.workspace_id);
                format!(
                    "{account}: your role in the {workspace} workspace doesn't allow running dots"
                )
            }
            crate::accounts::Access::Role(_) => {
                format!("{account}: your role no longer allows running dots")
            }
            // Waiting, changed Dots ID, removed, or outside the workspace: the refusal says
            // which.
            _ => match crate::accounts::require_dot(orgs, &dot, run_dots).await {
                Err(Error::NotAllowed(text)) => text,
                _ => format!("{account}: your role no longer allows running dots"),
            },
        };
        Ok(Some((account, text)))
    }

    /// The account and the hold text when `dot`'s policy reaches past the policy ceiling of a
    /// Member in its workspace (guardrails spec §4.3); `None` otherwise.
    /// Also held: a dot whose workspace's guardrails can't be read on this PC (guardrails
    /// spec §4.4).
    pub async fn ceiling_hold(&self, dot: &Dot) -> Option<(String, String)> {
        let orgs = self.vault.get().and_then(|v| v.orgs());
        dot.org_id.as_ref()?;
        let account = crate::accounts::account_label(orgs, dot.org_id.as_deref()).await;
        if let Some(text) = crate::accounts::unknown_rails(orgs, &account, dot) {
            return Some((account, text));
        }
        let rails = crate::accounts::dot_rails(orgs, dot)?;
        let text = rails.ceiling_hold(&account, &dot.spec.policy, dot.spec.use_user_settings)?;
        Some((account, text))
    }

    /// `Ok` when this PC's access to the account and workspace of `dot` lets it run dots,
    /// otherwise the refusal naming the role, or why this PC is not on the member list or in
    /// the workspace (members spec §6.3).
    /// For a trigger that would queue runs to fire later, such as a webhook: it is refused
    /// when it is called instead.
    pub async fn require_run(&self, dot: &Dot) -> Result<()> {
        let orgs = self.vault.get().and_then(|v| v.orgs());
        crate::accounts::require_dot(orgs, dot, crate::accounts::Permission::RunDots).await
    }

    /// `Ok` when this PC's access to the account `org_id` (None: Personal) and its workspace
    /// `workspace_id` allows `p`, otherwise the refusal in the same words as `require_run`. For
    /// a caller outside the app, such as an API key: checked on every call (accounts spec §10).
    pub async fn require_in(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
        p: crate::accounts::Permission,
    ) -> Result<()> {
        let orgs = self.vault.get().and_then(|v| v.orgs());
        crate::accounts::require_in(orgs, org_id, workspace_id, p).await
    }

    /// Whether the account `org_id` is joined on this PC. False when there is no vault yet.
    pub fn org_joined(&self, org_id: &str) -> bool {
        self.vault
            .get()
            .and_then(|v| v.orgs())
            .is_some_and(|orgs| orgs.is_joined(org_id))
    }

    /// How many background Claude runs the runner holds now.
    pub fn running_background_claude(&self) -> usize {
        self.active
            .lock()
            .unwrap()
            .values()
            .filter(|a| a.background_claude)
            .count()
    }

    /// The ids of the queued runs the usage limits hold now; none without limits.
    pub async fn usage_held_ids(&self) -> Result<HashSet<String>> {
        let Some(limits) = self.limits.get() else {
            return Ok(HashSet::new());
        };
        let state = limits.state(&self.queued_runs().await?, self.running_background_claude());
        Ok(state.held_run_ids.into_iter().collect())
    }

    /// The spending cap that keeps the queued `run` from starting (spend caps design): the
    /// first cap reached for its dot, unless the person let it run once anyway. `None` when it
    /// may start, or is no longer queued.
    pub async fn cap_hold(&self, run: &Run) -> Result<Option<crate::spend::CapHit>> {
        if run.status != RunStatus::Queued || run.cap_released {
            return Ok(None);
        }
        crate::spend::Ledger::load(&self.store)
            .await?
            .hit_for(&self.store, &run.dot_id)
            .await
    }

    /// Sets why each waiting run waits (delivery tracking spec section 3): each `queued` run
    /// not yet delivered gets its reason, every other run `None`. The facts are read once.
    /// The slots in use and the busy dots are the runner's own, exactly what `dispatch_once`
    /// checks: a run parked or waiting for an approval has left them and keeps no dot busy.
    pub async fn fill_wait_reasons(&self, runs: &mut [Run]) -> Result<()> {
        let waiting =
            |r: &Run| r.status == RunStatus::Queued && r.delivery == Some(Delivery::Waiting);
        if !runs.iter().any(waiting) {
            for run in runs.iter_mut() {
                run.wait_reason = None;
                run.wait_detail = None;
            }
            return Ok(());
        }
        let usage_held = self.usage_held_ids().await?;
        // The spending caps, read once for every waiting run (spend caps design).
        let mut ledger = crate::spend::Ledger::load(&self.store).await?;
        let mut capped: HashMap<String, String> = HashMap::new();
        for run in runs.iter().filter(|r| waiting(r) && !r.cap_released) {
            if let Some(hit) = ledger.hit_for(&self.store, &run.dot_id).await? {
                capped.insert(run.id.clone(), hit.text);
            }
        }
        let (running, busy_dots) = {
            let active = self.active.lock().unwrap();
            let busy: HashSet<String> = active
                .values()
                .filter(|a| !a.workflow)
                .map(|a| a.dot_id.clone())
                .collect();
            (active.len(), busy)
        };
        let runner_held = self.is_held();
        for run in runs.iter_mut() {
            run.wait_reason = waiting(run).then(|| {
                wait_reason(&WaitFacts {
                    usage_held: usage_held.contains(&run.id),
                    spend_capped: capped.contains_key(&run.id),
                    runner_held,
                    // A run of a workflow step neither waits for its dot's other runs nor
                    // keeps them waiting (teams spec section 8).
                    dot_has_active_run: run.workflow_step_run_id.is_none()
                        && busy_dots.contains(&run.dot_id),
                    running,
                    max_concurrent: self.cfg.max_concurrent,
                })
            });
            run.wait_detail = match run.wait_reason {
                Some(crate::model::WaitReason::SpendCap) => capped.get(&run.id).cloned(),
                _ => None,
            };
        }
        Ok(())
    }

    /// The vault runs resolve their dots' variables from (env vault spec §8). Set once, before
    /// any run starts.
    pub fn set_vault(&self, vault: Vault) {
        let _ = self.vault.set(vault);
    }

    /// The tracker of acting dots the watchers read. Set once, before any run starts.
    pub fn set_acting(&self, acting: Arc<crate::watchers::ActingTracker>) {
        let _ = self.acting.set(acting);
    }

    /// Where watchers staged their runs' files. Set once, before any run starts.
    pub fn set_watcher_files(&self, files: crate::watchers::files::WatcherFiles) {
        let _ = self.watcher_files.set(files);
    }

    /// Where files attached to chat messages are stored. Set once, before any run starts.
    pub fn set_chat_files(&self, files: crate::chat_files::ChatFiles) {
        let _ = self.chat_files.set(files);
    }

    /// Puts the files of the run's topic that are missing into its workspace (attachments
    /// spec section 4.4). A file of this run's own message that cannot be put there fails the
    /// run before its engine starts; an earlier message's file is a notice on the run.
    async fn copy_chat_files(&self, run: &Run, workspace: &Path) -> Result<()> {
        let Some(files) = self.chat_files.get() else {
            return Ok(());
        };
        let topic = self.store.topic_attachments(&run.root_run_id).await?;
        if topic.is_empty() {
            return Ok(());
        }
        let (files, root, workspace, run_id) = (
            files.clone(),
            run.root_run_id.clone(),
            workspace.to_path_buf(),
            run.id.clone(),
        );
        let listed = topic.clone();
        let failed = tokio::task::spawn_blocking(move || {
            files.copy_into(&root, &listed, &workspace, &run_id)
        })
        .await
        .map_err(|e| Error::Other(format!("copying the attached files failed: {e}")))?;
        // The files now in place are marked, before any engine starts: from then on each is
        // taken as it is there (attachments spec section 4.4). A failed mark only means its
        // name is checked again next time.
        let placed: Vec<String> = topic
            .iter()
            .filter(|f| f.name.is_some() && f.placed_at.is_none())
            .filter(|f| !failed.iter().any(|(id, _)| *id == f.id))
            .map(|f| f.id.clone())
            .collect();
        if !placed.is_empty() {
            let at = crate::util::timestamp(chrono::Utc::now());
            if let Err(e) = self.store.mark_attachments_placed(&placed, &at).await {
                tracing::warn!(run = %run.id, "marking the attached files placed failed: {e}");
            }
        }
        let mut notices = Vec::new();
        for (id, e) in failed {
            let Some(file) = topic.iter().find(|f| f.id == id) else {
                continue;
            };
            if file.run_id.as_deref() == Some(run.id.as_str()) {
                return Err(Error::Other(format!(
                    "{}: {e}",
                    crate::chat_files::COPY_FAILED
                )));
            }
            let name = file.name.clone().unwrap_or_else(|| file.file_name.clone());
            tracing::warn!(run = %run.id, file = %name, "copying an attached file failed: {e}");
            notices.push(format!(
                "Dots could not copy {name} into the workspace: {e}"
            ));
        }
        for text in notices {
            self.record(&run.id, &EngineEvent::Notice { text }).await?;
        }
        Ok(())
    }

    /// Whether the run's topic folder under `attachments/` holds the dot's work: an attachment
    /// it edited or a file it added (`ChatFiles::changed_in`). A failure to tell counts as
    /// changed, so nothing is removed on a guess.
    async fn chat_files_changed(&self, run: &Run, workspace: &Path) -> bool {
        let Some(files) = self.chat_files.get() else {
            return false;
        };
        let topic = match self.store.topic_attachments(&run.root_run_id).await {
            Ok(topic) => topic,
            Err(e) => {
                tracing::warn!(run = %run.id, "reading the topic's files failed: {e}");
                return true;
            }
        };
        let (files, root, workspace) = (
            files.clone(),
            run.root_run_id.clone(),
            workspace.to_path_buf(),
        );
        tokio::task::spawn_blocking(move || files.changed_in(&root, &topic, &workspace))
            .await
            .unwrap_or(true)
    }

    /// Copies a watcher thread's staged attachments into the run's workspace; for the thread's
    /// root, stores and announces the payload with their paths, and answers the stored run.
    async fn copy_attachments(&self, run: &Run, workspace: &Path) -> Option<Run> {
        let files = self.watcher_files.get()?;
        let root = if run.id == run.root_run_id {
            run.clone()
        } else {
            self.store.get_run(&run.root_run_id).await.ok()?
        };
        if root.trigger != TriggerKind::Watcher {
            return None;
        }
        match files.copy_into(&root, workspace) {
            Ok(Some(payload)) if run.id == root.id => {
                match self.store.set_run_payload(&run.id, &payload).await {
                    Ok(stored) => {
                        self.emit_run(&stored);
                        Some(stored)
                    }
                    Err(e) => {
                        tracing::warn!(run = %run.id, "saving attachment paths failed: {e}");
                        None
                    }
                }
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(run = %run.id, "copying attachments failed: {e}");
                None
            }
        }
    }

    /// Removes a watcher topic's attachments from its workspace. A chat topic's files stay
    /// for the rest of the conversation (attachments spec section 4.4).
    async fn remove_attachments(&self, workspace: &Path, root_run_id: &str) {
        if self.watcher_files.get().is_none() {
            return;
        }
        match self.store.get_run(root_run_id).await {
            Ok(root) if root.trigger == TriggerKind::Watcher => {
                crate::watchers::files::WatcherFiles::remove_copy(workspace, root_run_id);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(run = %root_run_id, "reading a topic's first run failed: {e}")
            }
        }
    }

    /// Lets go of a watcher thread's staged files once no run of it is left to come.
    async fn release_attachments(&self, run_id: &str) {
        if let Some(files) = self.watcher_files.get() {
            files.release(&self.store, run_id).await;
        }
    }

    fn acting_begin(&self, dot_id: &str) {
        if let Some(acting) = self.acting.get() {
            acting.begin(dot_id);
        }
    }

    fn acting_end(&self, dot_id: &str) {
        if let Some(acting) = self.acting.get() {
            acting.end(dot_id);
        }
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

    /// Whether `run` may start workflows: it is no step of a workflow, and the person's chat is
    /// at the root of its resumes (a follow-up, scheduled, webhook or manual run is not). A chat
    /// run that carries a maker's job or Dots' wake of a maker is not the person's: its words
    /// were written by a model (dots-make-dots design section 8). So is a job's text the person
    /// sent again after its job ended (`prompt::FROM_MAKER`).
    async fn may_start_workflows(&self, run: &Run) -> bool {
        if run.workflow_step_run_id.is_some() {
            return false;
        }
        let mut cur = run.clone();
        while cur.trigger == TriggerKind::Resume {
            let Some(parent) = cur.parent_run_id.clone() else {
                return false;
            };
            match self.store.get_run(&parent).await {
                Ok(p) => cur = p,
                Err(_) => return false,
            }
        }
        let made_by_a_model = [&cur, run].iter().any(|r| {
            r.payload.as_ref().is_some_and(|p| {
                p.get(crate::making::jobs::JOB_ID).is_some()
                    || p.get(crate::making::wake::WAKE_ID).is_some()
                    || p.get(crate::prompt::FROM_MAKER).is_some()
            })
        });
        cur.trigger == TriggerKind::Chat && !made_by_a_model
    }

    /// The dot's runs a person's chat message names (conversational chat spec section 3.1): for
    /// the first message of a topic, the newest ended runs outside it; for one that resumes the
    /// topic, those that ended after its last run was queued. Nothing for any other run. A list
    /// that cannot be read is logged and left empty; the chat goes on without it.
    async fn recent_runs(&self, run: &Run) -> Vec<Run> {
        if !crate::prompt::is_typed(run) {
            return Vec::new();
        }
        let since = match (&run.session_id, &run.parent_run_id) {
            (None, _) => None,
            (Some(_), Some(parent)) => match self.store.get_run(parent).await {
                Ok(parent) => Some(parent.queued_at),
                Err(e) => {
                    tracing::warn!(run = %run.id, "reading the chat's last run failed: {e}");
                    return Vec::new();
                }
            },
            (Some(_), None) => return Vec::new(),
        };
        match self
            .store
            .ended_runs_outside(
                &run.dot_id,
                &run.root_run_id,
                since.as_deref(),
                crate::prompt::RECENT_RUNS,
            )
            .await
        {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(run = %run.id, "listing the dot's recent runs failed: {e}");
                Vec::new()
            }
        }
    }

    /// For a person's message that resumes its topic: the dot was saved after the session last
    /// learned its instructions, so the prompt names them as they are now. The session learned
    /// them in the nearest earlier run of the topic that told it: the person's message (it had
    /// this section, or nothing had changed), or the run that started the session (it had the
    /// whole prompt; only that run has no parent). The runs Dots makes in between (an answer, a
    /// resume after an approval, a follow-up, a decision) send only their message, so they are
    /// passed over. `Dot::updated_at` moves on every save of the dot, not only of its
    /// instructions; naming them once more is harmless, so a topic that cannot be read counts
    /// as a change (logged). Both are `util::now()` text, so they compare as strings.
    ///
    /// A maker's job that goes on in its made dot's session is told the same way (dots-make-dots
    /// design section 8): a job's prompt names them too, so a job's run tells the session as a
    /// person's message does.
    async fn instructions_changed(&self, run: &Run, dot: &Dot) -> bool {
        let tells =
            |r: &Run| crate::prompt::is_typed(r) || crate::prompt::makers_words(r).is_some();
        if !tells(run) || run.session_id.is_none() {
            return false;
        }
        let thread = match self.store.list_thread(&run.root_run_id).await {
            Ok(thread) => thread,
            Err(e) => {
                tracing::warn!(run = %run.id, "reading the chat's runs failed: {e}");
                return true;
            }
        };
        let mut next = run.parent_run_id.as_deref();
        // At most one step per run of the thread.
        for _ in 0..thread.len() {
            let Some(prev) = next.and_then(|id| thread.iter().find(|r| r.id == id)) else {
                break;
            };
            if prev.parent_run_id.is_none() || tells(prev) {
                return dot.updated_at > prev.queued_at;
            }
            next = prev.parent_run_id.as_deref();
        }
        tracing::warn!(run = %run.id, "the chat's runs do not lead to its first one");
        true
    }

    /// For a run whose topic a move or merge changed (its restart mark): the run without its
    /// session, in hand only, and the topic's runs before it along its line. The mark is taken
    /// only once a run that restarted has used its fresh session (`finish`: it succeeded or
    /// waits for an approval), so a run that fails first, or a crash, leaves it for the next
    /// one; the stored session stays, so the topic can still be replied to. A `resume`
    /// finishes an approved call in its own session and leaves the mark alone.
    async fn restart_if_moved(&self, run: &Run) -> Result<Option<(Run, Vec<Run>)>> {
        if run.trigger == TriggerKind::Resume {
            return Ok(None);
        }
        if !self.store.topic_restart_pending(&run.root_run_id).await? {
            return Ok(None);
        }
        let mut fresh = run.clone();
        fresh.session_id = None;
        let so_far = self
            .store
            .list_thread(&run.root_run_id)
            .await?
            .into_iter()
            .take_while(|r| r.id != run.id)
            .collect();
        tracing::info!(run = %run.id, "the topic changed: a fresh session");
        Ok(Some((fresh, so_far)))
    }

    /// Makes the run's open tools reachable through the MCP gateway (`POST /mcp/s/<server>`,
    /// spec section 5) until the run ends.
    pub fn register_tools(&self, run_id: &str, tools: Arc<RunTools>) {
        self.tools.lock().unwrap().insert(run_id.to_string(), tools);
    }

    /// The open tools of a running run (found by its secret, `run_for_secret`).
    pub fn run_tools(&self, run_id: &str) -> Option<Arc<RunTools>> {
        self.tools.lock().unwrap().get(run_id).cloned()
    }

    /// Gives Dots' own `dots` MCP server the check-in tools. The runtime calls it once, at
    /// start; a second call changes nothing.
    pub fn set_checkins(&self, checkins: CheckIns) {
        let _ = self.checkins.set(checkins);
    }

    /// Gives the runs of dots that use the project graph their graph. The runtime calls it
    /// once, at start; a second call changes nothing.
    pub fn set_graphs(&self, graphs: crate::graph::Graphs) {
        let _ = self.graphs.set(graphs);
    }

    /// The projects' graphs, once the runtime has set them.
    pub fn graphs(&self) -> Option<&crate::graph::Graphs> {
        self.graphs.get()
    }

    /// The check-in tools, once the runtime has set them.
    pub fn checkins(&self) -> Option<&CheckIns> {
        self.checkins.get()
    }

    /// Gives the runs of dots with `browser` on their browser. The runtime calls it once, at
    /// start; a second call changes nothing.
    pub fn set_browsers(&self, browsers: Arc<crate::browser::Browsers>) {
        let _ = self.browsers.set(browsers);
    }

    /// Gives the runs of dots with apps their apps. The runtime calls it once, at start; a
    /// second call changes nothing.
    pub fn set_apps(&self, apps: crate::apps::Apps) {
        let _ = self.apps.set(apps);
    }

    /// The installed apps, once the runtime has set them.
    pub fn apps(&self) -> Option<&crate::apps::Apps> {
        self.apps.get()
    }

    /// The dots' browsers, once the runtime has set them.
    pub fn browsers(&self) -> Option<&Arc<crate::browser::Browsers>> {
        self.browsers.get()
    }

    /// The cancel token of a run the runner holds: a call the gateway sends for the run stops
    /// when it fires.
    pub fn run_cancel(&self, run_id: &str) -> Option<CancellationToken> {
        self.active
            .lock()
            .unwrap()
            .get(run_id)
            .map(|a| a.cancel.clone())
    }

    /// Forgets what the MCP endpoints know of a run that no longer runs.
    fn forget_run(&self, run_id: &str) {
        self.secrets.lock().unwrap().retain(|_, r| r != run_id);
        self.tools.lock().unwrap().remove(run_id);
    }

    pub async fn enqueue(&self, new: NewRun) -> Result<Run> {
        let run = self.store.create_run(&new).await?;
        self.enqueued(&run);
        Ok(run)
    }

    /// Announces a run queued by someone else (the scheduler queues its runs in the same
    /// transaction as its schedule row) and wakes the dispatcher for it.
    pub fn enqueued(&self, run: &Run) {
        self.emit_run(run);
        self.wake.notify_one();
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

    /// Holds or releases the queue. While held, no queued run starts, whatever its trigger;
    /// new runs are still queued and start once the queue is released (updates spec §6).
    /// When holding, it returns only once no dispatch pass is in flight, so no queued run is
    /// claimed after it returns.
    pub async fn set_held(&self, held: bool) {
        self.held.store(held, Ordering::SeqCst);
        if held {
            drop(self.pass.lock().await);
        } else {
            self.wake.notify_one();
        }
    }

    /// Whether the queue is held (`set_held`).
    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }

    /// How many runs the runner runs at once.
    pub fn max_concurrent(&self) -> usize {
        self.cfg.max_concurrent
    }

    pub async fn dispatch_once(self: &Arc<Self>) -> Result<usize> {
        let _pass = self.pass.lock().await;
        if self.held.load(Ordering::SeqCst) {
            return Ok(0);
        }
        let mut started = 0;
        // The limits hear of the queue first: a hold that ended starts draining before a run
        // is let through (usage and limits spec section 4).
        let limits = self.limits.get().cloned();
        let mut running_background = self.running_background_claude();
        if let Some(limits) = &limits {
            limits.publish(&self.queued_runs().await?, running_background);
        }
        // The spending caps, read once for the pass (spend caps design).
        let mut ledger: Option<crate::spend::Ledger> = None;
        for (run, engine) in self.store.queued_with_engine(true).await? {
            let background_claude = engine == EngineKind::Claude && run_is_background(&run);
            // A role that may not run dots keeps a background run queued, unstarted, until the
            // role returns (members spec section 6.3); the next pass looks again.
            if self.role_hold(&run).await?.is_some() {
                continue;
            }
            // A reached spending cap keeps any run queued, unstarted, until the day rolls over
            // or the cap is raised, unless the person let it run once anyway.
            if !run.cap_released {
                let ledger = match &mut ledger {
                    Some(l) => l,
                    None => ledger.insert(crate::spend::Ledger::load(&self.store).await?),
                };
                if ledger.hit_for(&self.store, &run.dot_id).await?.is_some() {
                    continue;
                }
            }
            if let Some(limits) = &limits {
                if limits.admit(&queued_run(&run, engine), running_background) != Admit::Go {
                    continue;
                }
            }
            // Register as active before claiming, so a run is never `running` in the store
            // without a cancellation handle: a cancel in between is recorded on the token
            // and honoured by `execute` before anything starts.
            let cancel = CancellationToken::new();
            let by_user = Arc::new(AtomicBool::new(false));
            let keep_waiting = Arc::new(Notify::new());
            {
                let mut active = self.active.lock().unwrap();
                if active.len() >= self.cfg.max_concurrent {
                    break;
                }
                let workflow = run.workflow_step_run_id.is_some();
                let dot_busy = !workflow
                    && active
                        .values()
                        .any(|a| a.dot_id == run.dot_id && !a.workflow);
                if active.contains_key(&run.id) || dot_busy {
                    continue;
                }
                active.insert(
                    run.id.clone(),
                    Active {
                        dot_id: run.dot_id.clone(),
                        workflow,
                        cancel: cancel.clone(),
                        cancelled_by_user: by_user.clone(),
                        keep_waiting: keep_waiting.clone(),
                        background_claude,
                    },
                );
            }
            if background_claude {
                running_background += 1;
            }
            self.acting_begin(&run.dot_id);
            match self.store.claim_run(&run.id).await {
                Ok(true) => {}
                Ok(false) => {
                    self.active.lock().unwrap().remove(&run.id);
                    self.acting_end(&run.dot_id);
                    continue;
                }
                Err(e) => {
                    self.active.lock().unwrap().remove(&run.id);
                    self.acting_end(&run.dot_id);
                    return Err(e);
                }
            }
            // Nothing fallible may sit between claiming and handing off: failures
            // after this point go through `execute`'s error path, which frees the slot.
            // A panic skips that path, so a watcher task cleans up instead.
            let run_id = run.id.clone();
            let inner = tokio::spawn(self.clone().execute(
                run_id.clone(),
                cancel.clone(),
                by_user,
                keep_waiting,
            ));
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
        let released = self.active.lock().unwrap().remove(run_id);
        if let Some(active) = released {
            self.acting_end(&active.dot_id);
        }
        self.release_attachments(run_id).await;
        self.forget_run(run_id);
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
        self.close_applies(run_id).await;
        self.wake.notify_one();
    }

    async fn execute(
        self: Arc<Self>,
        run_id: String,
        cancel: CancellationToken,
        by_user: Arc<AtomicBool>,
        keep_waiting: Arc<Notify>,
    ) {
        let result = match self.store.get_run(&run_id).await {
            Ok(run) => {
                self.emit_run(&run);
                self.execute_inner(&run, cancel.clone(), &by_user, &keep_waiting)
                    .await
            }
            Err(e) => Err(e),
        };
        // Whatever happened, never leave an engine running unsupervised.
        cancel.cancel();
        if result.is_ok() {
            self.start_check(&run_id).await;
        }
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
        // An error path may have left a watcher run's attachments in its workspace.
        if let Ok(stored) = self.store.get_run(&run_id).await {
            if let Some(path) = &stored.workspace_path {
                self.remove_attachments(Path::new(path), &stored.root_run_id)
                    .await;
            }
        }
        self.forget_run(&run_id);
        self.close_applies(&run_id).await;
        // A cancel that arrived after the final status was computed (the run parked) is
        // applied now, before the resume check could act on the parked decisions.
        self.apply_late_cancel(&run_id, &by_user).await;
        self.after_finish(&run_id).await;
        self.release_attachments(&run_id).await;
        // Released only after the resume check, so `is_active` going false means the
        // runner is completely done with the run.
        let released = self.active.lock().unwrap().remove(&run_id);
        if let Some(active) = released {
            self.acting_end(&active.dot_id);
        }
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

    /// Closes the approved calls that the resume run `run_id` was to perform, once that run no
    /// longer runs (it ended, parked, or was cancelled before it started): a call it claimed
    /// and gave no outcome is `unknown`, a call it never reached is `skipped`. Nothing performs
    /// them later. Each change is announced; errors are logged, never returned.
    async fn close_applies(&self, run_id: &str) {
        let closed = match self.store.get_run(run_id).await {
            Ok(run) if run.trigger == TriggerKind::Resume => match run.parent_run_id {
                Some(parent) => self.store.close_applies(&parent).await,
                None => return,
            },
            Ok(_) => return,
            Err(e) => Err(e),
        };
        match closed {
            Ok(approvals) => {
                for approval in approvals {
                    let _ = self.bus.send(RuntimeEvent::ApprovalUpdated { approval });
                }
            }
            Err(e) => tracing::warn!(run = %run_id, "closing approved calls failed: {e}"),
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
        let engine = self.store.get_dot(&run.dot_id).await?.spec.engine;
        let mut lines = Vec::new();
        let mut grants = Vec::new();
        let mut queued = Vec::new();
        let mut needs_child = false;
        for a in &decided {
            let note = a.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
            match (a.status, note) {
                // A browser call approved after its run parked is not performed: the page may
                // have moved on (browser spec section 3.6). No grant and no queued call; the
                // dot is told, and a rule chosen with it was saved with the decision.
                (ApprovalStatus::Approved, _)
                    if crate::browser::is_browser_approval(&a.tool, &a.input) =>
                {
                    if self.store.skip_late_browser_call(&a.id).await? {
                        let approval = self.store.get_approval(&a.id).await?;
                        let _ = self.bus.send(RuntimeEvent::ApprovalUpdated { approval });
                    }
                    lines.push(crate::browser::late_line(&a.id));
                    needs_child = true;
                }
                // Dots performs this call itself, as the first thing the child run does: no
                // grant, and nothing to tell the model until the call has a result.
                (ApprovalStatus::Approved, _) if a.self_apply => {
                    queued.push(a.id.clone());
                    needs_child = true;
                }
                (ApprovalStatus::Approved, _) => {
                    grants.push((
                        a.tool.clone(),
                        grant_key(&a.tool, &a.input),
                        a.mcp_fingerprint.clone(),
                    ));
                    lines.push(format!(
                        "- Approval #{} granted: you may now use {} with the same input. Perform that action now.",
                        a.id,
                        model_tool_name(&a.tool, &a.input, engine)
                    ));
                    needs_child = true;
                }
                (_, Some(note)) => {
                    lines.push(format!(
                        "- Approval #{} for {} was denied. Reviewer note: {note}",
                        a.id,
                        model_tool_name(&a.tool, &a.input, engine)
                    ));
                    needs_child = true;
                }
                _ => lines.push(format!(
                    "- Approval #{} for {} was denied. Do not attempt it again.",
                    a.id,
                    model_tool_name(&a.tool, &a.input, engine)
                )),
            }
        }
        let child = needs_child.then(|| {
            // Without a line there is nothing to review: every decision is a call that Dots
            // performs, and the engine reports those itself.
            let message = if lines.is_empty() {
                CONTINUE.to_string()
            } else {
                format!(
                    "Human review of your queued actions:\n{}\n\n{CONTINUE}",
                    lines.join("\n")
                )
            };
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
        // Closed without a follow-up: nothing will use the workspace again. Cleaned up before
        // the parent reads as ended, so a step's hand-off, built once every run of the step has
        // ended, finds Dots' commit of what the run left (teams spec section 6.1).
        if child.is_none() {
            self.cleanup_workspace(&run).await;
        }
        // One transaction: grants, resolved flags, queued calls, child run and closing the
        // parent either all happen or none do, so a failure never loses a decision.
        let resolved: Vec<String> = decided.iter().map(|a| a.id.clone()).collect();
        let child = match self
            .store
            .resume_parked(run_id, &grants, &resolved, &queued, child.as_ref())
            .await
        {
            Ok(child) => child,
            // Cancelled (or otherwise closed) concurrently: nothing to resume.
            Err(Error::Conflict(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        if let Some(c) = &child {
            self.emit_run(c);
            self.wake.notify_one();
        }
        self.emit_run(&self.store.get_run(run_id).await?);
        Ok(child)
    }

    /// Removes the run's worktree if it is unchanged; errors are logged, never returned. A run
    /// of a workflow step first has its leftover work committed (`finish_workspace`).
    async fn cleanup_workspace(&self, run: &Run) {
        let Some(prepared) = Prepared::from_run(run) else {
            return;
        };
        let result = async {
            let dot = self.store.get_dot(&run.dot_id).await?;
            let step = self.step_context(run).await?;
            let ws_dot = workspace_dot(&dot, step.as_ref());
            self.finish_workspace(
                run,
                &ws_dot,
                step.as_ref(),
                &prepared,
                run.summary.as_deref(),
            )
            .await;
            Ok::<_, Error>(())
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
        }
    }

    /// The workflow step a run works for, as its workspace needs it; `None` for an ordinary run.
    async fn step_context(&self, run: &Run) -> Result<Option<StepContext>> {
        let Some(step_run_id) = &run.workflow_step_run_id else {
            return Ok(None);
        };
        let step = self.store.get_step_run(step_run_id).await?;
        let workflow_run = self.store.get_workflow_run(&step.workflow_run_id).await?;
        let step_name = workflow_run
            .definition
            .step(&step.step_id)
            .map(|s| s.name.clone())
            .unwrap_or(step.step_id);
        Ok(Some(StepContext {
            repo: workflow_run.project.repo_path,
            default_branch: workflow_run.project.default_branch,
            remote: workflow_run.project.remote,
            step_name,
        }))
    }

    /// The checkpoints of folder runs, beside the worktrees in Dots' data folder
    /// (`<data>/checkpoints`).
    pub fn checkpoints(&self) -> Checkpoints {
        Checkpoints::new(
            self.cfg
                .worktrees_dir
                .with_file_name(crate::checkpoint::DIR),
        )
    }

    /// What happens to a run's worktree when the run stops for good: a run of a workflow step has
    /// what it left uncommitted committed by Dots (teams spec section 6.1; a failure is a notice
    /// on the run), then an unchanged worktree is removed. A worktree whose chat files the dot
    /// edited, or wrote beside, is kept (attachments spec section 7). Errors are logged, never
    /// returned.
    async fn finish_workspace(
        &self,
        run: &Run,
        ws_dot: &Dot,
        step: Option<&StepContext>,
        prepared: &Prepared,
        summary: Option<&str>,
    ) {
        if let Some(step) = step {
            if let Err(e) = self.commit_step(step, prepared, summary).await {
                tracing::warn!(run = %run.id, "committing the step's work failed: {e}");
                let text = format!("Dots could not commit the changes: {e}");
                if let Err(e) = self.record(&run.id, &EngineEvent::Notice { text }).await {
                    tracing::warn!(run = %run.id, "recording the notice failed: {e}");
                }
            }
        }
        if ws_dot.spec.workspace_mode == WorkspaceMode::Worktree
            && self.chat_files_changed(run, &prepared.path).await
        {
            return;
        }
        if let Err(e) = self.workspaces.cleanup_if_unchanged(ws_dot, prepared).await {
            tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
        }
    }

    /// Dots' commit of what a run of a workflow step left uncommitted in its worktree (teams
    /// spec section 6.1). A worktree that is gone, or has nothing left to commit, is left as
    /// it is.
    async fn commit_step(
        &self,
        step: &StepContext,
        prepared: &Prepared,
        summary: Option<&str>,
    ) -> Result<()> {
        if prepared.branch.is_none() || !prepared.path.is_dir() {
            return Ok(());
        }
        // A path that does not exist, beside the worktrees folder in Dots' data folder
        // (`<data>/worktrees`), so no agent's workspace (`<data>/worktrees/<run>`) has it at
        // `../`: git finds no hook there.
        let no_hooks = self.cfg.worktrees_dir.with_file_name("no-hooks");
        let message = commit_message(step, summary);
        self.workspaces
            .commit_leftovers(prepared, &message, &no_hooks)
            .await?;
        Ok(())
    }

    /// Commits what the run of a workflow step left uncommitted, as the run's end does. A
    /// step's hand-off calls it before it reads the step's branch, so the next step never
    /// starts from a tip without that work, even when the run's end did not get to commit it.
    /// A worktree already clean or gone is left as it is. Errors are logged, never returned:
    /// the run's end has already put its own failure on the run as a notice.
    pub async fn commit_step_work(&self, run: &Run) {
        let Some(prepared) = Prepared::from_run(run) else {
            return;
        };
        let committed = match self.step_context(run).await {
            Ok(Some(step)) => {
                self.commit_step(&step, &prepared, run.summary.as_deref())
                    .await
            }
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        if let Err(e) = committed {
            tracing::warn!(run = %run.id, "committing the step's work failed: {e}");
        }
    }

    async fn record(&self, run_id: &str, ev: &EngineEvent) -> Result<()> {
        match ev {
            // Written to the run, not stored as an event and not announced: a turn is too
            // often for that, and the run's next update carries it (context rules spec
            // section 3).
            EngineEvent::Context { fill, window } => {
                if self.store.set_run_context(run_id, *fill, *window).await? {
                    if let Some(fill) = fill {
                        self.store.set_clear_after(run_id, *fill).await?;
                    }
                }
                return Ok(());
            }
            // Dots' own `/compact` is recorded by `compact_first`; the CLI's own compaction
            // inside the run is recorded here (context rules spec section 7).
            EngineEvent::Compacted {
                trigger,
                before,
                after,
            } => {
                self.store
                    .set_run_context(run_id, Some(*after), None)
                    .await?;
                if trigger != "manual" {
                    let run = self.store.get_run(run_id).await?;
                    self.store
                        .add_context_event(&NewContextEvent {
                            run_id,
                            dot_id: &run.dot_id,
                            root_run_id: &run.root_run_id,
                            kind: context_rules::EventKind::Compact,
                            manual: false,
                            fill_before: *before,
                            fill_after: Some(*after),
                            from_session: run.session_id.as_deref(),
                            tokens: None,
                            ok: true,
                        })
                        .await?;
                }
            }
            EngineEvent::SessionStarted { session_id } => {
                self.store.set_session(run_id, session_id).await?
            }
            EngineEvent::Usage {
                tokens_in,
                tokens_out,
                cache_read,
                cost_usd,
            } => {
                self.store
                    .add_usage(
                        run_id,
                        *tokens_in as i64,
                        *tokens_out as i64,
                        *cache_read as i64,
                        *cost_usd,
                    )
                    .await?
            }
            EngineEvent::Limits {
                status,
                windows,
                resets_at,
            } => {
                if let Some(limits) = self.limits.get() {
                    let reset = resets_at.as_deref();
                    if let Err(e) = limits.record_reset(status, windows.clone(), reset).await {
                        tracing::warn!(run = %run_id, "keeping the limit reading failed: {e}");
                    }
                    self.nudge();
                }
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

    /// The step before `run`'s turn (context rules spec section 4), from its session's last
    /// fill and the topic's request. The request is only read here; `finish_request` takes it
    /// once the step is done, so a run that fails before then leaves it for the next turn.
    /// Dots' own compaction is held back for a few turns after one that failed or did not
    /// help (`Store::compact_backoff`). Nothing for a run the rules do not look at. A failed
    /// read is logged and counts as nothing known.
    async fn context_plan(
        &self,
        run: &Run,
        dot: &Dot,
        rules: &ContextRules,
        restarted: bool,
    ) -> ContextPlan {
        let mut plan = ContextPlan {
            step: context_rules::Step::Nothing,
            manual: false,
            fill: 0,
            window: context_rules::WINDOW_DEFAULT,
            session: run.session_id.clone(),
        };
        let Some(session) = run.session_id.as_deref() else {
            return plan;
        };
        if restarted || !context_rules::applies(run, dot.spec.engine) {
            return plan;
        }
        let last = self.store.session_fill(session).await.unwrap_or_else(|e| {
            tracing::warn!(run = %run.id, "reading the session's context failed: {e}");
            None
        });
        let request = match self.store.thread_state(&run.root_run_id).await {
            Ok(state) => state.request,
            Err(e) => {
                tracing::warn!(run = %run.id, "reading the topic's request failed: {e}");
                None
            }
        };
        let model = last
            .as_ref()
            .and_then(|l| l.model.clone())
            .unwrap_or_else(|| dot.spec.model.clone());
        plan.window = context_rules::window_for(Some(&model), last.as_ref().and_then(|l| l.window));
        if let Some(last) = &last {
            plan.fill = last.fill;
            // Under the clear threshold: the topic's "keep going" is spent.
            if !context_rules::at_or_above(last.fill, plan.window, rules.clear_at) {
                if let Err(e) = self.store.clear_keep_going(&run.root_run_id).await {
                    tracing::warn!(run = %run.id, "clearing the topic's keep going failed: {e}");
                }
            }
        }
        plan.manual = request.is_some();
        plan.step = context_rules::decide(
            rules,
            &context_rules::Turn {
                chat: run.trigger == TriggerKind::Chat,
                fill: last.map(|l| l.fill),
                window: plan.window,
                request,
            },
        );
        if plan.step == context_rules::Step::Compact && !plan.manual {
            let held = self
                .store
                .compact_backoff(session)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(run = %run.id, "reading the compaction backoff failed: {e}");
                    false
                });
            if held {
                plan.step = context_rules::Step::Nothing;
            }
        }
        plan
    }

    /// Takes the topic's request once its step is done (context rules spec section 6).
    async fn finish_request(&self, run: &Run, plan: &ContextPlan) -> Result<()> {
        if plan.manual {
            self.store
                .set_thread_request(&run.root_run_id, None)
                .await?;
        }
        Ok(())
    }

    /// Why an extra CLI call before the turn must not be made: the dot reached a spending cap,
    /// or Claude's limit is reached (context rules spec section 5). A run let past a cap once
    /// was let past for its turn, not for extra calls, so the cap counts here.
    async fn pass_held(&self, run: &Run) -> Option<String> {
        let released = Run {
            cap_released: false,
            ..run.clone()
        };
        if let Some(cap) = self.cap_reached(&released).await {
            return Some(cap);
        }
        let limited = self
            .limits
            .get()
            .is_some_and(|l| l.rejected_until().is_some());
        limited.then(|| "Claude's usage limit is reached".to_string())
    }

    /// Adds what an extra CLI call used to the run (spend caps and the run's numbers count it).
    async fn add_pass_usage(&self, run: &Run, usage: &PassUsage) -> Result<()> {
        if usage.tokens_in + usage.tokens_out == 0 && usage.cost_usd.is_none() {
            return Ok(());
        }
        self.store
            .add_usage(
                &run.id,
                usage.tokens_in as i64,
                usage.tokens_out as i64,
                usage.cache_read as i64,
                usage.cost_usd,
            )
            .await
    }

    /// Compacts the run's session before its turn and records it, what it used included, and
    /// whether it helped. A failure is a notice and a record, and the turn goes ahead; a cancel
    /// is neither (context rules spec section 5).
    async fn compact_first(
        &self,
        run: &Run,
        engine: &Arc<dyn Engine>,
        ctx: &RunContext,
        plan: &ContextPlan,
        rules: &ContextRules,
        deadline: Instant,
    ) -> Result<()> {
        if let Some(why) = self.pass_held(run).await {
            let text = format!("The conversation was not compacted first: {why}.");
            return self.record(&run.id, &EngineEvent::Notice { text }).await;
        }
        let done = match tokio::time::timeout_at(deadline, engine.compact(ctx)).await {
            Ok(done) => done,
            Err(_) => Err(Error::Other("the run's time limit was reached".into())),
        };
        if ctx.cancel.is_cancelled() {
            return Ok(());
        }
        let record = |fill_after, tokens, ok| NewContextEvent {
            run_id: &run.id,
            dot_id: &run.dot_id,
            root_run_id: &run.root_run_id,
            kind: context_rules::EventKind::Compact,
            manual: plan.manual,
            fill_before: plan.fill,
            fill_after,
            from_session: plan.session.as_deref(),
            tokens,
            ok,
        };
        match done {
            Ok(Some(done)) => {
                self.add_pass_usage(run, &done.usage).await?;
                self.record(
                    &run.id,
                    &EngineEvent::Compacted {
                        trigger: "manual".into(),
                        before: done.before,
                        after: done.after,
                    },
                )
                .await?;
                let helped = !context_rules::at_or_above(done.after, plan.window, rules.compact_at);
                let used = done.usage.tokens_in + done.usage.tokens_out;
                self.store
                    .add_context_event(&NewContextEvent {
                        fill_before: done.before,
                        ..record(Some(done.after), Some(used), helped)
                    })
                    .await?;
                if !context_rules::at_or_above(done.after, plan.window, rules.clear_at) {
                    self.store.clear_keep_going(&run.root_run_id).await?;
                }
                if let Ok(row) = self.store.get_run(&run.id).await {
                    self.emit_run(&row);
                }
            }
            Ok(None) => {}
            Err(e) => {
                self.store
                    .add_context_event(&record(None, None, false))
                    .await?;
                let text =
                    format!("Compacting the conversation failed: {e}; it goes on as it was.");
                self.record(&run.id, &EngineEvent::Notice { text }).await?;
            }
        }
        Ok(())
    }

    /// The section a fresh session starts with (context rules spec section 5): the old
    /// session's hand-off note, the topic's last messages and the extra sections. Records the
    /// fresh start. `None` keeps the session: the note could not be asked for (a cap or the
    /// limit), or it failed and the start was the rules' (then the session is compacted
    /// instead). A fresh start the person asked for goes ahead without a note, with a notice.
    #[allow(clippy::too_many_arguments)]
    async fn fresh_seed(
        &self,
        run: &Run,
        engine: &Arc<dyn Engine>,
        ctx: &RunContext,
        plan: &ContextPlan,
        dot: &Dot,
        rules: &ContextRules,
        deadline: Instant,
    ) -> Result<Option<String>> {
        if let Some(why) = self.pass_held(run).await {
            let text = format!("The conversation did not start afresh: {why}.");
            self.record(&run.id, &EngineEvent::Notice { text }).await?;
            return Ok(None);
        }
        let mut ask = ctx.clone();
        ask.session_id = plan.session.clone();
        ask.prompt = context_rules::HANDOFF_ASK.to_string();
        let asked = match tokio::time::timeout_at(deadline, engine.handoff_summary(&ask)).await {
            Ok(asked) => asked,
            Err(_) => Err(Error::Other("the run's time limit was reached".into())),
        };
        if ctx.cancel.is_cancelled() {
            return Ok(None);
        }
        let (summary, tokens) = match asked {
            Ok(Some(handoff)) => {
                self.add_pass_usage(run, &handoff.usage).await?;
                let used = handoff.usage.tokens_in + handoff.usage.tokens_out;
                (Some(handoff.summary), Some(used))
            }
            Ok(None) => (None, None),
            Err(e) if !plan.manual => {
                let text = format!(
                    "The hand-off note could not be written: {e}; the conversation keeps its \
                     session and is compacted instead."
                );
                self.record(&run.id, &EngineEvent::Notice { text }).await?;
                self.compact_first(run, engine, ctx, plan, rules, deadline)
                    .await?;
                return Ok(None);
            }
            Err(e) => {
                let text = format!(
                    "The hand-off note could not be written: {e}; the fresh session starts \
                     without it."
                );
                self.record(&run.id, &EngineEvent::Notice { text }).await?;
                (None, None)
            }
        };
        let thread: Vec<Run> = self
            .store
            .list_thread(&run.root_run_id)
            .await?
            .into_iter()
            .take_while(|r| r.id != run.id)
            .collect();
        let seed = context_rules::handoff_seed(
            summary.as_deref(),
            &thread,
            &self.extra_seed_sections(dot, run).await,
        );
        self.store
            .add_context_event(&NewContextEvent {
                run_id: &run.id,
                dot_id: &run.dot_id,
                root_run_id: &run.root_run_id,
                kind: context_rules::EventKind::Clear,
                manual: plan.manual,
                fill_before: plan.fill,
                fill_after: None,
                from_session: plan.session.as_deref(),
                tokens,
                ok: true,
            })
            .await?;
        self.store.clear_keep_going(&run.root_run_id).await?;
        let text = format!(
            "Started a fresh session with a summary: the last one was {}% full.",
            context_rules::percent(plan.fill, plan.window)
        );
        self.record(&run.id, &EngineEvent::Notice { text }).await?;
        Ok(Some(seed))
    }

    /// Extra sections of a fresh session's prompt (context rules spec section 5): the project
    /// map of the run's graph, as its first prompt has it (project graph spec section 9), for a
    /// dot that uses the graph and a project that has one.
    async fn extra_seed_sections(&self, dot: &Dot, run: &Run) -> Vec<SeedSection> {
        let Some(graph) = self
            .run_tools(&run.id)
            .and_then(|tools| tools.graph().cloned())
        else {
            return Vec::new();
        };
        let tool = crate::graph::model_name(dot.spec.engine);
        tokio::task::spawn_blocking(move || {
            graph
                .graphs
                .seed_section(&graph.project, &graph.scope, tool)
        })
        .await
        .ok()
        .flatten()
        .into_iter()
        .collect()
    }

    /// The spending cap the running `run` has reached, if any: read after each usage report.
    /// A run let past a cap once is never stopped by one. A failed read is logged and stops
    /// nothing.
    async fn cap_reached(&self, run: &Run) -> Option<String> {
        if run.cap_released {
            return None;
        }
        let hit = async {
            crate::spend::Ledger::load(&self.store)
                .await?
                .hit_for(&self.store, &run.dot_id)
                .await
        }
        .await;
        match hit {
            Ok(hit) => hit.map(|h| h.text),
            Err(e) => {
                tracing::warn!(run = %run.id, "reading the spending caps failed: {e}");
                None
            }
        }
    }

    async fn execute_inner(
        &self,
        run: &Run,
        cancel: CancellationToken,
        by_user: &AtomicBool,
        keep_waiting: &Notify,
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
        if let Some(reason) = engine.unavailable() {
            return Err(Error::Other(reason));
        }
        // A topic a move or merge changed starts its next run afresh, told what the topic said
        // so far (topic tools spec section 4.4).
        let restarted = self.restart_if_moved(run).await?;
        let (run, so_far) = match &restarted {
            Some((fresh, so_far)) => (fresh, so_far.as_slice()),
            None => (run, &[][..]),
        };
        // A run of a workflow step works in the project's repo (teams spec section 8). One that
        // continues the thread of a step that has ended works there no more, nor for the
        // workflow: it fails before its workspace is touched.
        if self.store.in_ended_step_thread(run).await? {
            // A Stop closed the run's own step after the run turned `Running`, before its
            // cancel reached the run: it ends cancelled, as the Stop meant, not failed.
            let stopped = match &run.workflow_step_run_id {
                Some(id) => {
                    self.store.get_step_run(id).await?.status
                        == crate::model::StepRunStatus::Cancelled
                }
                None => false,
            };
            if stopped {
                return self.finish_cancelled_before_start(&run.id).await;
            }
            return Err(Error::Other(
                crate::workflows::ENDED_STEP_THREAD.to_string(),
            ));
        }
        // A dot whose org is no longer joined does not start, and a stale org is fetched
        // before its variables resolve (env vault spec §6.4), before any folder is made.
        let vault = self
            .vault
            .get()
            .cloned()
            .unwrap_or_else(|| Vault::in_memory(self.store.clone()));
        vault.before_run(&dot).await?;
        let step = self.step_context(run).await?;
        let ws_dot = workspace_dot(&dot, step.as_ref());
        let prepared = match Prepared::from_run(run) {
            Some(p) if p.path.is_dir() => {
                // A resumed or follow-up run works in the folder its row names, and the
                // policy's `./` rules are anchored there. A row with a branch names a worktree:
                // it must be one of dots' own, or the run fails here, before the engine starts.
                // A folder run has no branch and nothing to compare its folder with.
                if p.branch.is_some() {
                    self.workspaces.check_owned(&p)?;
                }
                p
            }
            // A follow-up of a session whose unchanged worktree was cleaned up: the session
            // remembers that directory, so recreate the worktree at the same path.
            Some(p)
                if run.session_id.is_some()
                    && dot.spec.workspace_mode == WorkspaceMode::Worktree
                    && p.branch.is_some()
                    && p.base_commit.is_some() =>
            {
                self.workspaces.restore(&ws_dot, &p).await?
            }
            _ => {
                // A step's first run starts where the workflow says: an earlier step's branch,
                // else the project's default branch, else the repo's.
                let mut start = run
                    .payload
                    .as_ref()
                    .and_then(|p| p.pointer("/workflow/start"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if start.is_none() {
                    if let Some(s) = &step {
                        if let Some(branch) = &s.default_branch {
                            let repo = std::path::Path::new(&s.repo);
                            start = Some(default_branch_ref(repo, branch, &s.remote).await);
                        }
                    }
                }
                let p = self
                    .workspaces
                    .prepare_from(&ws_dot, &run.id, start.as_deref())
                    .await?;
                self.store
                    .set_workspace(
                        &run.id,
                        &p.path.to_string_lossy(),
                        p.branch.as_deref(),
                        p.base_commit.as_deref(),
                    )
                    .await?;
                // The run now has a folder, a branch and a base commit: say so, or a view of
                // the running run would not know them until it ends. Only an announcement:
                // a failed read must not fail a run whose workspace is made and recorded.
                match self.store.get_run(&run.id).await {
                    Ok(stored) => self.emit_run(&stored),
                    Err(e) => {
                        tracing::warn!(run = %run.id, "announcing the workspace failed: {e}")
                    }
                }
                p
            }
        };
        // A watcher run's attachments, in its workspace for the run (watchers spec §4.2).
        let refreshed = self.copy_attachments(run, &prepared.path).await;
        let run = refreshed.as_ref().unwrap_or(run);
        // The topic's chat files, missing ones copied again (attachments spec section 4.4).
        self.copy_chat_files(run, &prepared.path).await?;
        // The run's MCP sessions, skills and plugin folders (MCP/skills spec section 5), each
        // server with the variables its dot grants it, in the dot's scope (env vault spec §8).
        let (tools, mut notices) = RunTools::open(OpenRequest {
            store: &self.store,
            bus: &self.bus,
            run_id: &run.id,
            uses: &dot.spec.uses,
            workspace: &prepared.path,
            plugins_dir: (dot.spec.engine == EngineKind::Claude)
                .then_some(self.cfg.plugins_dir.as_path()),
            cancel: &cancel,
            vault: &vault,
            scope: &DotScope::of(&dot),
            workspace_id: &dot.workspace_id,
            grants: &dot.spec.variables,
            dot_name: &dot.spec.name,
        })
        .await?;
        // The built-in browser of a dot with `browser` on (browser spec section 2.2).
        let tools = crate::browser::attach(
            self.browsers.get(),
            &dot,
            &run.id,
            &prepared.path,
            tools,
            &mut notices,
        );
        // The run's apps (app library spec section 6).
        let tools =
            crate::apps::run::attach(self.apps.get(), &dot, &run.id, tools, &mut notices).await?;
        // A chat run may start its dot's team workflows (teams spec section 7), unless its
        // policy denies `Workflow` outright: the tool is then neither offered nor listed. A list
        // that cannot be read is logged and left empty; the person's chat goes on without it.
        let workflow_off = dot
            .spec
            .policy
            .resolve(crate::policy::WORKFLOW_TOOL, &Value::Null)
            == crate::policy::Action::Deny;
        let tools = if !workflow_off && self.may_start_workflows(run).await {
            let startable = match self.store.startable_workflows(&dot.id).await {
                Ok(startable) => startable,
                Err(e) => {
                    tracing::warn!(run = %run.id, "listing the workflows it may start failed: {e}");
                    Vec::new()
                }
            };
            tools.with_startable(startable)
        } else {
            tools
        };
        // A run of an open agent step has the step's workflow tools, under the same rule. A step
        // that cannot be read is logged; the run goes on without them.
        let tools = if workflow_off {
            tools
        } else {
            match crate::team_tools::for_run(&self.store, run).await {
                Ok(step) => tools.with_step(step),
                Err(e) => {
                    tracing::warn!(run = %run.id, "reading the step's workflow tools failed: {e}");
                    tools
                }
            }
        };
        // A run of a chat the person started may propose new instructions for its dot
        // (conversational chat spec section 4), unless its policy denies `Instructions`
        // outright: the tool is then neither offered nor named.
        let instructions_off = dot
            .spec
            .policy
            .resolve(crate::policy::INSTRUCTIONS_TOOL, &Value::Null)
            == crate::policy::Action::Deny;
        let proposes = !instructions_off
            && self.checkins().is_some()
            && match crate::instructions::may_propose(&self.store, run).await {
                Ok(may) => may,
                Err(e) => {
                    tracing::warn!(run = %run.id, "reading the run's topic failed: {e}");
                    false
                }
            };
        let tools = tools.with_instructions(proposes);
        // A run of a dot that may make dots has the tools for it (dots-make-dots design section
        // 5), where the makers' service would not refuse the run for what it is: not a workflow
        // step's run. A policy that denies `MakeDots` outright leaves them out, as above.
        let making_off = dot
            .spec
            .policy
            .resolve(crate::policy::MAKE_DOTS_TOOL, &Value::Null)
            == crate::policy::Action::Deny;
        let makes = !making_off
            && self.checkins().is_some_and(|c| c.makers().is_some())
            && crate::making::tools::offered(&dot, run);
        let tools = tools.with_making(makes.then_some(dot.spec.making));
        // The project's code graph (project graph spec section 5): its first build starts now,
        // in the background; the run never waits for it. A policy that denies `Graph` outright
        // leaves the tool out.
        let graph_off = dot
            .spec
            .policy
            .resolve(crate::graph::POLICY_NAME, &Value::Null)
            == crate::policy::Action::Deny;
        let graph = match self.graphs.get() {
            Some(graphs) if dot.spec.use_graph && !graph_off => {
                run_graph(graphs, &dot, &prepared).await
            }
            _ => None,
        };
        let tools = tools.with_graph(graph);
        let tools = Arc::new(tools);
        let _closer = CloseOnDrop(tools.clone());
        if cancel.is_cancelled() {
            tools.close().await;
            self.remove_attachments(&prepared.path, &run.root_run_id)
                .await;
            if let Err(e) = self
                .workspaces
                .cleanup_if_unchanged(&ws_dot, &prepared)
                .await
            {
                tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
            }
            return self.finish_cancelled_before_start(&run.id).await;
        }
        for text in notices {
            self.record(&run.id, &EngineEvent::Notice { text }).await?;
        }
        // A resume run on an engine that performs approved calls itself is handed the calls
        // its parent queued. The engine claims each one before it runs it.
        let approved_calls = match &run.parent_run_id {
            Some(parent)
                if run.trigger == TriggerKind::Resume && engine.applies_approved_calls() =>
            {
                self.store.queued_applies(parent).await?
            }
            _ => Vec::new(),
        };
        let settings = self.store.get_settings().await?;
        let global = &settings.global_instructions;
        // The quiet limit is read once, as the engine starts (stuck-run watchdog spec 4.1).
        let quiet_secs = effective_quiet_limit(&dot.spec, &settings);
        // The step before this turn (context rules spec section 4), decided on the run as it
        // was queued. `queued_as` keeps that run: a requeue puts its session back.
        let rules = dot.spec.context_rules.unwrap_or(settings.context_rules);
        let plan = self
            .context_plan(run, &dot, &rules, restarted.is_some())
            .await;
        let queued_as = run;
        let secret = self.register_secret(&run.id);
        self.register_tools(&run.id, tools.clone());
        // The model the engine starts with: the dot's own, or Auto's pick for a Claude dot set
        // to Auto (auto model spec section 2). A thread keeps the tier its last run had, when
        // Auto picked that one.
        let thread_tier = match &run.parent_run_id {
            Some(parent) => match self.store.get_run(parent).await {
                Ok(parent) => parent.model_tier,
                Err(e) => {
                    tracing::warn!(run = %run.id, "reading the thread's model failed: {e}");
                    None
                }
            },
            None => None,
        };
        let choice = crate::auto_model::choose(&dot, run, thread_tier, &settings);
        self.store
            .set_run_model(
                &run.id,
                &choice.model,
                choice.auto.as_ref().map(|r| (r.tier, r.reason.as_str())),
                None,
            )
            .await?;
        if choice.auto.is_some() {
            self.announce(&run.id).await;
        }
        // Where Auto goes when the CLI cannot use its pick: once, to another tier's model.
        let mut fallback = choice.auto.as_ref().and_then(|route| {
            let tier = crate::auto_model::fallback_tier(route.tier);
            let model = settings.auto_models.model(tier).to_string();
            (model != choice.model).then(|| Fallback {
                first: choice.model.clone(),
                tier,
                model,
                reason: route.reason.clone(),
            })
        });
        let mut engine_dot = dot.clone();
        engine_dot.spec.model = choice.model.clone();
        let mut ctx = RunContext {
            run_id: run.id.clone(),
            dot: engine_dot,
            workspace: prepared.path.clone(),
            prompt: String::new(),
            session_id: run.session_id.clone(),
            mcp_url: self.cfg.mcp_url.clone(),
            mcp_secret: secret,
            cancel: cancel.clone(),
            approved_calls,
            tools: tools.clone(),
            autocompact_pct: (rules.enabled && dot.spec.engine == EngineKind::Claude)
                .then_some(rules.compact_at),
            guardrails: crate::accounts::enforced_rails(
                self.vault.get().and_then(|v| v.orgs()),
                &dot,
            ),
        };
        // The run's time limit starts before any extra call, which counts against it
        // (context rules spec section 5).
        let deadline = Instant::now() + Duration::from_secs(dot.spec.timeout_secs);
        let seed = match plan.step {
            context_rules::Step::Nothing => None,
            context_rules::Step::Compact => {
                self.compact_first(run, &engine, &ctx, &plan, &rules, deadline)
                    .await?;
                None
            }
            context_rules::Step::Fresh => {
                self.fresh_seed(run, &engine, &ctx, &plan, &dot, &rules, deadline)
                    .await?
            }
        };
        // Cancelled during an extra call: the turn never starts.
        if cancel.is_cancelled() || by_user.load(Ordering::SeqCst) {
            tools.close().await;
            self.remove_attachments(&prepared.path, &run.root_run_id)
                .await;
            if let Err(e) = self
                .workspaces
                .cleanup_if_unchanged(&ws_dot, &prepared)
                .await
            {
                tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
            }
            return self.finish_cancelled_before_start(&run.id).await;
        }
        self.finish_request(run, &plan).await?;
        // A fresh start is the run without its session from here on, so its prompt is a first
        // prompt.
        let fresh;
        let run = if seed.is_some() {
            let mut r = run.clone();
            r.session_id = None;
            fresh = r;
            &fresh
        } else {
            run
        };
        ctx.session_id = run.session_id.clone();
        // The memories as they are now: a run's prompt does not change while it runs.
        let memories = self.store.list_memories(&dot.id).await?;
        let recent = self.recent_runs(run).await;
        let instructions_changed = self.instructions_changed(run, &dot).await;
        let startable = crate::team_tools::prompt_section(tools.startable(), dot.spec.engine);
        let handoff = seed.as_deref();
        // A first prompt of a run with the tools for making dots lists the dots its dot made
        // (dots-make-dots design section 11), as they are now.
        let made_dots = match tools.making() {
            Some(_) if run.session_id.is_none() => self.store.list_made_dots(&dot.id).await?,
            _ => Vec::new(),
        };
        let prompt = {
            let mut prompt = build_prompt_full(
                &dot,
                run,
                &prepared.path,
                chrono::Local::now(),
                global,
                PromptParts {
                    contexts: tools.contexts(),
                    memories: &memories,
                    apps: tools.apps().map(|apps| apps.as_ref()),
                    recent: &recent,
                    instructions_tool: tools
                        .instructions()
                        .then(|| crate::instructions::model_name(dot.spec.engine)),
                    instructions_changed,
                    so_far,
                    handoff,
                    making: tools.making().map(|limits| MakerPart {
                        limits: *limits,
                        made: &made_dots,
                    }),
                },
            );
            // A chat's first prompt lists the workflows it may start (teams spec section 7).
            if run.session_id.is_none() && !startable.is_empty() {
                prompt.push_str("\n## Workflows you can start\n\n");
                prompt.push_str(&startable);
            }
            prompt
        };
        // And the project map, once the project has a graph (project graph spec section 9). A
        // fresh start's seed carries it already (`extra_seed_sections`).
        let mut prompt = prompt;
        if let Some(graph) = tools
            .graph()
            .filter(|_| run.session_id.is_none() && seed.is_none())
        {
            let graph = graph.clone();
            let tool = crate::graph::model_name(dot.spec.engine);
            let section = tokio::task::spawn_blocking(move || {
                graph
                    .graphs
                    .prompt_section(&graph.project, &graph.scope, tool)
            })
            .await
            .ok()
            .flatten();
            if let Some(section) = section {
                prompt.push('\n');
                prompt.push_str(&section);
            }
        }
        ctx.prompt = prompt;
        self.store.set_run_quiet_limit(&run.id, quiet_secs).await?;
        let retry_ctx = fallback.as_ref().map(|_| ctx.clone());
        // A folder run gets a checkpoint before its engine starts, so what it changes can be
        // listed and undone (undo spec section 2). Worktree runs have their branch instead.
        let checkpoint = (ws_dot.spec.workspace_mode == WorkspaceMode::Folder
            && prepared.branch.is_none())
        .then(|| self.checkpoints());
        if let Some(checkpoints) = &checkpoint {
            checkpoints.begin(&run.id, &prepared.path).await;
        }
        let mut rx = match engine.start(ctx).await {
            Ok(rx) => rx,
            Err(e) => {
                if let Some(checkpoints) = &checkpoint {
                    checkpoints.finish(&run.id).await;
                }
                return Err(e);
            }
        };
        // The engine has the message: the run reached its dot (delivery tracking spec
        // section 2). Recorded once; a requeued run keeps its first delivery. A failed write
        // fails the run; a failed read for the announcement only is logged, as for the
        // workspace above.
        if self
            .store
            .set_delivered(&run.id, chrono::Utc::now())
            .await?
        {
            match self.store.get_run(&run.id).await {
                Ok(row) => self.emit_run(&row),
                Err(e) => tracing::warn!(run = %run.id, "announcing the delivery failed: {e}"),
            }
        }

        let mut grace: Option<Instant> = None;
        let mut timed_out = false;
        // The stuck-run watchdog (stuck-run watchdog spec section 4.2): warn after `quiet` of no
        // output, stop after another `quiet` still quiet. No limit, no timer.
        let quiet = quiet_secs.map(|secs| {
            self.cfg
                .quiet_unit
                .saturating_mul(u32::try_from(secs).unwrap_or(u32::MAX))
        });
        let mut last_output = Instant::now();
        let mut quiet_at: Option<Instant> = None;
        let mut quiet_stopped = false;
        // A spending cap the run's own usage reached while it ran (spend caps design).
        let mut cap_stopped: Option<String> = None;
        let mut cap_stop_at: Option<Instant> = None;
        let mut cap_fired = false;
        let mut summary: Option<String> = None;
        let mut error: Option<String> = None;
        // Claude's `rejected` ending a background run (usage and limits spec section 4).
        let mut saw_tool_call = false;
        let mut saw_rejected = false;
        loop {
            // A reached cap stops the run a moment after the usage that reached it, so the end
            // the engine reports with that usage is not cut off.
            let limit = grace.unwrap_or(cap_stop_at.map_or(deadline, |c| c.min(deadline)));
            // Warn a limit after the last output; once quiet, stop a limit after the warning.
            // Off once the run is being stopped.
            let quiet_timer = quiet
                .filter(|_| grace.is_none())
                .map(|q| quiet_at.unwrap_or(last_output) + q);
            // Biased, in this order: the time limit and a cancel come first, so a flood of
            // engine events can never starve them. Output comes before the quiet timer, so
            // output ready at the same poll as the warn deadline wins and a run that just
            // spoke is never announced as gone quiet.
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(limit) => {
                    if grace.is_some() {
                        tracing::warn!(run = %run.id, "engine ignored cancellation; abandoning it");
                        break;
                    }
                    cap_fired = cap_stop_at.is_some();
                    timed_out = !cap_fired;
                    cancel.cancel();
                    grace = Some(Instant::now() + self.cfg.cancel_grace);
                }
                _ = cancel.cancelled(), if grace.is_none() => {
                    grace = Some(Instant::now() + self.cfg.cancel_grace);
                }
                ev = rx.recv() => {
                    let Some(ev) = ev else { break };
                    // The CLI could not use Auto's pick and did nothing yet: started again,
                    // once, with another tier's model (auto model spec section 4).
                    if let (EngineEvent::Failed { error: e }, Some(ctx)) = (&ev, &retry_ctx) {
                        if grace.is_none()
                            && !saw_tool_call
                            && fallback.is_some()
                            && crate::auto_model::model_unavailable(e)
                        {
                            let next = fallback.take().expect("checked above");
                            if let Some(again) = self
                                .fall_back(run, &engine, ctx, &mut rx, &cancel, &next, e)
                                .await?
                            {
                                rx = again;
                                last_output = Instant::now();
                                continue;
                            }
                        }
                    }
                    self.record(&run.id, &ev).await?;
                    last_output = Instant::now();
                    if quiet_at.take().is_some() {
                        self.set_quiet(run, None, quiet_secs).await;
                    }
                    if matches!(ev, EngineEvent::Usage { .. })
                        && cap_stopped.is_none()
                        && grace.is_none()
                    {
                        if let Some(text) = self.cap_reached(run).await {
                            cap_stopped = Some(text);
                            cap_stop_at = Some(Instant::now() + CAP_STOP_AFTER);
                        }
                    }
                    match ev {
                        EngineEvent::Finished { summary: s } => summary = Some(s),
                        EngineEvent::Failed { error: e } => error = Some(e),
                        EngineEvent::ToolCall { .. } => saw_tool_call = true,
                        EngineEvent::Limits { status, .. } if status == "rejected" => {
                            saw_rejected = true;
                        }
                        _ => {}
                    }
                }
                _ = tokio::time::sleep_until(quiet_timer.unwrap_or(deadline)),
                    if quiet_timer.is_some() =>
                {
                    // A store error must not end the run: it counts as waiting, and the
                    // clock starts again.
                    let pending = self
                        .store
                        .has_pending_for_run(&run.id)
                        .await
                        .unwrap_or_else(|e| {
                            tracing::warn!(run = %run.id, "checking pending approvals failed: {e}");
                            true
                        });
                    if pending {
                        // Waiting on the person is not quiet: the clock starts again.
                        last_output = Instant::now();
                        if quiet_at.take().is_some() {
                            self.set_quiet(run, None, quiet_secs).await;
                        }
                    } else if quiet_at.is_none() {
                        quiet_at = Some(Instant::now());
                        let since = crate::util::now();
                        self.set_quiet(run, Some(&since), quiet_secs).await;
                    } else {
                        // Still quiet a limit after the warning: stopped as the time limit
                        // stops a run.
                        quiet_stopped = true;
                        cancel.cancel();
                        grace = Some(Instant::now() + self.cfg.cancel_grace);
                    }
                }
                _ = keep_waiting.notified() => {
                    last_output = Instant::now();
                    if quiet_at.take().is_some() {
                        self.set_quiet(run, None, quiet_secs).await;
                    }
                }
            }
        }
        tools.close().await;
        // Gone before the worktree's "unchanged" check: they are never a run's changes.
        self.remove_attachments(&prepared.path, &run.root_run_id)
            .await;
        // The run's closing checkpoint: what it changed in its folder.
        if let Some(checkpoints) = &checkpoint {
            checkpoints.finish(&run.id).await;
        }

        let user_cancelled = by_user.load(Ordering::SeqCst);
        // A cap stopped the run, unless it had finished by then: the usage that reached the cap
        // usually comes with the run's own end.
        let cap_failed = cap_fired && (summary.is_none() || error.is_some());
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
        } else if let (true, Some(text)) = (cap_failed, &cap_stopped) {
            (RunStatus::Failed, Some(format!("{STOPPED_BY_CAP}{text}")))
        } else if quiet_stopped {
            let minutes = 2 * quiet_secs.unwrap_or_default() / 60;
            (
                RunStatus::Failed,
                Some(format!("stopped: no output for {minutes} min")),
            )
        } else if waiting {
            (RunStatus::AwaitingApproval, error)
        } else if let Some(e) = error {
            let stopped_by_limit =
                saw_rejected && dot.spec.engine == EngineKind::Claude && run_is_background(run);
            // Back to the queue only while Dots knows when the limit resets: without that the
            // run would start again at once, and be refused again.
            let resets_known = self
                .limits
                .get()
                .is_some_and(|l| l.rejected_until().is_some());
            if stopped_by_limit && !saw_tool_call && resets_known {
                // It did nothing yet: back to the queue, held until the limit resets, with its
                // trigger and payload, so a watcher's event is not lost. A folder it was queued
                // with (a follow-up's thread) stays; one this attempt made goes.
                if run.workspace_path.is_none() {
                    if let Err(e) = self.workspaces.cleanup_if_unchanged(&dot, &prepared).await {
                        tracing::warn!(run = %run.id, "workspace cleanup failed: {e}");
                    }
                }
                let queued = self.store.requeue_run(&run.id, queued_as).await?;
                self.emit_run(&queued);
                return Ok(());
            }
            if stopped_by_limit {
                (RunStatus::Failed, Some(format!("{STOPPED_BY_LIMIT}: {e}")))
            } else {
                (RunStatus::Failed, Some(e))
            }
        } else if summary.is_some() {
            (RunStatus::Succeeded, None)
        } else {
            (
                RunStatus::Failed,
                Some("engine exited without a result".to_string()),
            )
        };
        // A run that reached a cap and ended by itself says so: the next run waits.
        if let (false, Some(text)) = (cap_failed || user_cancelled || timed_out, &cap_stopped) {
            let text = format!("{text}: the next run waits");
            self.record(&run.id, &EngineEvent::Notice { text }).await?;
        }
        // The watchdog names the ending only when nothing before it in the list above did.
        let quiet_stopped = quiet_stopped && !user_cancelled && !timed_out && !cap_failed;
        let cap_failed = cap_failed && !user_cancelled && !timed_out;
        if user_cancelled || timed_out || quiet_stopped || cap_failed {
            self.store.expire_pending_for_run(&run.id).await?;
        }
        // Clean up before publishing the terminal status so observers that see
        // `succeeded`/`failed` never find a leftover unchanged worktree, nor a step's work left
        // uncommitted.
        if status != RunStatus::AwaitingApproval {
            self.finish_workspace(run, &ws_dot, step.as_ref(), &prepared, summary.as_deref())
                .await;
        }
        // A succeeded answer that gets a result check is stored as being checked before it is
        // announced as ended, so whoever waits for the check (the notification) knows from the
        // first announcement.
        if status == RunStatus::Succeeded {
            if let Some(model) = self.check_model(&dot, run, summary.as_deref()).await {
                let checking = ResultCheck::Checking { model };
                if let Err(e) = self.store.set_result_check(&run.id, Some(&checking)).await {
                    tracing::warn!(run = %run.id, "marking the result check failed: {e}");
                }
            }
        }
        let finished = self
            .store
            .finish_run_with(
                &run.id,
                status,
                summary.as_deref(),
                err_text.as_deref(),
                quiet_stopped,
            )
            .await?;
        // The run may have changed the project: its graph catches up in the background. The
        // graphs of worktrees that are gone go.
        if let Some(graph) = tools.graph() {
            graph.graphs.refresh(&graph.project);
            let (graphs, dir) = (graph.graphs.clone(), self.cfg.worktrees_dir.clone());
            tokio::task::spawn_blocking(move || graphs.sweep_worktrees(&dir));
        }
        // A run that restarted its topic's session and got to use it spends the mark.
        if restarted.is_some()
            && matches!(status, RunStatus::Succeeded | RunStatus::AwaitingApproval)
        {
            if let Err(e) = self.store.take_topic_restart(&run.root_run_id).await {
                tracing::warn!(run = %run.id, "clearing the topic's restart mark failed: {e}");
            }
        }
        self.emit_run(&finished);
        Ok(())
    }

    /// The model a result check of this run's answer uses, `None` when it gets no check
    /// (`result_check::should_check`, or no checker, or the settings cannot be read).
    async fn check_model(&self, dot: &Dot, run: &Run, summary: Option<&str>) -> Option<String> {
        self.checker.get()?;
        let settings = match self.store.get_settings().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(run = %run.id, "reading the settings for a result check failed: {e}");
                return None;
            }
        };
        crate::result_check::should_check(
            settings.check_results,
            dot,
            run,
            RunStatus::Succeeded,
            summary,
        )
        .ok()?;
        Some(settings.checker_model)
    }

    /// Starts the result check of the run, when it ended succeeded and is marked as being
    /// checked. The check runs on its own: the runner does not wait for it.
    async fn start_check(self: &Arc<Self>, run_id: &str) {
        let Some(checker) = self.checker.get().cloned() else {
            return;
        };
        let run = match self.store.get_run(run_id).await {
            Ok(run) => run,
            Err(e) => {
                tracing::warn!(run = %run_id, "reading the run to check failed: {e}");
                return;
            }
        };
        let Some(ResultCheck::Checking { model }) = run.result_check.clone() else {
            return;
        };
        let this = self.clone();
        tokio::spawn(async move {
            this.check_result(checker, run, model).await;
        });
    }

    /// Checks the run's answer and stores the verdict, or why there is none (result check spec
    /// section 3). What the checker used is added to the run's usage and recorded as a
    /// `result_check` event. A flagged answer leaves an Inbox note. Nothing here changes how the
    /// run ended; every failure is logged and leaves the check skipped.
    async fn check_result(&self, checker: Arc<dyn ResultChecker>, run: Run, model: String) {
        let (check, usage) = self.ask_checker(checker, &run, &model).await;
        let usage = usage.unwrap_or_default();
        let (verdict, skipped) = match &check {
            ResultCheck::Done { verdict, .. } => (Some(*verdict), None),
            ResultCheck::Skipped { reason } => (None, Some(reason.clone())),
            ResultCheck::Checking { .. } => (None, None),
        };
        if usage.tokens_in > 0 || usage.tokens_out > 0 || usage.cost_usd.is_some() {
            let added = self
                .store
                .add_usage(
                    &run.id,
                    usage.tokens_in as i64,
                    usage.tokens_out as i64,
                    usage.cache_read as i64,
                    usage.cost_usd,
                )
                .await;
            if let Err(e) = added {
                tracing::warn!(run = %run.id, "adding the result check's usage failed: {e}");
            }
        }
        let event = CheckEvent {
            kind: crate::result_check::EVENT_KIND.to_string(),
            model: model.clone(),
            verdict,
            skipped,
            tokens_in: usage.tokens_in,
            tokens_out: usage.tokens_out,
            cache_read: usage.cache_read,
            cost_usd: usage.cost_usd,
        };
        let recorded = match serde_json::to_value(&event) {
            Ok(data) => {
                self.store
                    .append_event(&run.id, crate::result_check::EVENT_KIND, &data)
                    .await
            }
            Err(e) => Err(e.into()),
        };
        match recorded {
            Ok(rec) => {
                let _ = self.bus.send(RuntimeEvent::RunEvent { event: rec });
            }
            Err(e) => tracing::warn!(run = %run.id, "recording the result check failed: {e}"),
        }
        // The note first: whoever sees the verdict finds the note there too.
        if let Some(text) = crate::result_check::note_text(&check) {
            match self.store.add_note(&run.dot_id, Some(&run.id), &text).await {
                Ok(message) => {
                    let _ = self.bus.send(RuntimeEvent::MessagePosted { message });
                }
                Err(e) => tracing::warn!(run = %run.id, "leaving the check's note failed: {e}"),
            }
        }
        match self.store.set_result_check(&run.id, Some(&check)).await {
            Ok(row) => self.emit_run(&row),
            Err(e) => {
                tracing::warn!(run = %run.id, "storing the result check failed: {e}");
            }
        }
    }

    /// Asks `checker` about the run's answer: the verdict, or why there is none, and what the
    /// checker used. Never longer than `CHECK_LIMIT` (and a little to end the CLI).
    async fn ask_checker(
        &self,
        checker: Arc<dyn ResultChecker>,
        run: &Run,
        model: &str,
    ) -> (ResultCheck, Option<OnceUsage>) {
        let skipped = |reason: &str| ResultCheck::Skipped {
            reason: reason.to_string(),
        };
        let answer = run.summary.clone().unwrap_or_default();
        let dot = match self.store.get_dot(&run.dot_id).await {
            Ok(dot) => dot,
            Err(e) => return (skipped(&format!("The dot could not be read: {e}")), None),
        };
        let task = crate::result_check::task_text(&dot, run);
        let diff = self.worktree_changes(run).await;
        let id = crate::util::new_id();
        let nonce = &id[id.len().saturating_sub(10)..];
        let prompt = crate::result_check::build_prompt(&task, &answer, diff.as_deref(), nonce);
        let limit = crate::result_check::CHECK_LIMIT + Duration::from_secs(5);
        // On a task of its own, so a checker that panics leaves a skipped check, not one that
        // never ends.
        let owned_model = model.to_string();
        let mut asking = tokio::spawn(async move {
            checker
                .ask(
                    &owned_model,
                    crate::result_check::CHECKER_INSTRUCTIONS,
                    &prompt,
                )
                .await
        });
        let asked = tokio::time::timeout(limit, &mut asking).await;
        if asked.is_err() {
            asking.abort();
        }
        match asked {
            Err(_) => (skipped("The check took too long."), None),
            Ok(Err(_)) => (skipped("The checker stopped unexpectedly."), None),
            Ok(Ok(Err(reason))) => (skipped(&reason), None),
            Ok(Ok(Ok(reply))) => match crate::result_check::parse_verdict(&reply.text) {
                Some(parsed) => (
                    ResultCheck::Done {
                        model: model.to_string(),
                        verdict: parsed.verdict,
                        concerns: parsed.concerns,
                        confidence: parsed.confidence,
                    },
                    reply.usage,
                ),
                None => (
                    skipped("The checker's answer could not be read."),
                    reply.usage,
                ),
            },
        }
    }

    /// A summary of what a worktree run changed, for the checker; `None` for a folder run, a
    /// worktree that is gone or unchanged, or one whose changes cannot be read.
    async fn worktree_changes(&self, run: &Run) -> Option<String> {
        let prepared = Prepared::from_run(run).filter(|p| p.branch.is_some())?;
        let base = prepared.base_commit.clone()?;
        self.workspaces.check_owned(&prepared).ok()?;
        if !prepared.path.is_dir() {
            return None;
        }
        match crate::workspace::diff(&prepared.path, &base).await {
            Ok(diff) => crate::result_check::diff_summary(&diff),
            Err(e) => {
                tracing::warn!(run = %run.id, "reading the changes to check failed: {e}");
                None
            }
        }
    }

    /// Announces the run's row as it is now. Only an announcement: a failed read is logged and
    /// the run goes on.
    async fn announce(&self, run_id: &str) {
        match self.store.get_run(run_id).await {
            Ok(row) => self.emit_run(&row),
            Err(e) => tracing::warn!(run = %run_id, "announcing the run's model failed: {e}"),
        }
    }

    /// Starts `run` again with `next`'s model after the CLI said it could not use the one Auto
    /// picked (auto model spec section 4). The first attempt is let go first: its channel is
    /// read to its end, so its process and files are gone before the second starts with the
    /// same run id. `None` when it does not end in time or the run was cancelled meanwhile:
    /// the run then ends with the first attempt's error.
    #[allow(clippy::too_many_arguments)]
    async fn fall_back(
        &self,
        run: &Run,
        engine: &Arc<dyn Engine>,
        ctx: &RunContext,
        rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>,
        cancel: &CancellationToken,
        next: &Fallback,
        error: &str,
    ) -> Result<Option<tokio::sync::mpsc::Receiver<EngineEvent>>> {
        let drained =
            tokio::time::timeout(FALLBACK_DRAIN, async { while rx.recv().await.is_some() {} })
                .await;
        if drained.is_err() || cancel.is_cancelled() {
            return Ok(None);
        }
        let first = crate::auto_model::model_label(&next.first);
        let then = crate::auto_model::model_label(&next.model);
        let said: String = error
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("unavailable")
            .chars()
            .take(200)
            .collect();
        let text = format!("{first} could not be used ({said}). Trying again with {then}.");
        self.record(&run.id, &EngineEvent::Notice { text }).await?;
        self.store
            .set_run_model(
                &run.id,
                &next.model,
                Some((next.tier, next.reason.as_str())),
                Some(&next.first),
            )
            .await?;
        self.announce(&run.id).await;
        let mut again = ctx.clone();
        again.dot.spec.model = next.model.clone();
        Ok(Some(engine.start(again).await?))
    }

    /// Sets (`since`) or clears the run's quiet mark and announces it: a `RunQuiet` and the
    /// run's row. Only an announcement of the watchdog's state: a failed write is logged and
    /// the run goes on.
    async fn set_quiet(&self, run: &Run, since: Option<&str>, limit: Option<u64>) {
        match self.store.set_run_quiet(&run.id, since).await {
            Ok(row) => {
                let _ = self.bus.send(RuntimeEvent::RunQuiet {
                    run_id: row.id.clone(),
                    dot_id: row.dot_id.clone(),
                    quiet_since: row.quiet_since.clone(),
                    quiet_limit_secs: limit,
                });
                self.emit_run(&row);
            }
            Err(e) => tracing::warn!(run = %run.id, "updating the quiet mark failed: {e}"),
        }
    }

    /// Keep waiting on a quiet run (stuck-run watchdog spec section 4.2): its loop clears the
    /// mark and starts the quiet clock again. `Conflict` if the runner does not hold the run.
    pub async fn keep_waiting(&self, run_id: &str) -> Result<()> {
        let keep = self
            .active
            .lock()
            .unwrap()
            .get(run_id)
            .map(|a| a.keep_waiting.clone());
        match keep {
            Some(keep) => {
                keep.notify_one();
                Ok(())
            }
            None => Err(already(&self.store.get_run(run_id).await?)),
        }
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

    /// Cancels the run only if it is still `queued` and the runner does not hold it, as
    /// `cancel` does for such a run: its approvals expire, its attachments go, and the change
    /// is announced. `None` when it was not (for example, the dispatcher claimed it a moment
    /// earlier); a run awaiting an approval reached its dot and is left alone.
    pub async fn cancel_if_queued(&self, run_id: &str) -> Result<Option<Run>> {
        if self.active.lock().unwrap().contains_key(run_id) {
            return Ok(None);
        }
        self.cancel_inactive_with(run_id, true).await
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
        self.cancel_inactive_with(run_id, false).await
    }

    /// `cancel_inactive`, for a `queued` run only when `queued_only`.
    async fn cancel_inactive_with(&self, run_id: &str, queued_only: bool) -> Result<Option<Run>> {
        // Serialised with `maybe_resume` so a cancel never interleaves with a resume.
        let (prior, cancelled) = {
            let _guard = self.resume_lock.lock().await;
            let prior = self.store.get_run(run_id).await?;
            let cancelled = if queued_only {
                self.store.cancel_if_queued(run_id).await?
            } else {
                self.store.cancel_if_inactive(run_id).await?
            };
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
        // A resume run cancelled before it started performs none of its parent's calls.
        self.close_applies(run_id).await;
        // Clean up before publishing, as `execute_inner` does: whoever sees `cancelled` must
        // not find a leftover unchanged worktree.
        if prior.status == RunStatus::AwaitingApproval {
            self.cleanup_workspace(&run).await;
        }
        self.release_attachments(run_id).await;
        self.emit_run(&run);
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

/// A queued run as the limits see it.
/// The step before a turn and what it was decided on (context rules spec section 4).
struct ContextPlan {
    step: context_rules::Step,
    /// The person asked for it.
    manual: bool,
    /// The session's last fill (0 when none is known) and window.
    fill: u64,
    window: u64,
    /// The session the run continues.
    session: Option<String>,
}

fn queued_run(run: &Run, engine: EngineKind) -> QueuedRun {
    QueuedRun {
        id: run.id.clone(),
        trigger: run.trigger,
        background: run_is_background(run),
        engine,
        hold_released: run.hold_released,
        queued_at: run.queued_at.clone(),
    }
}
