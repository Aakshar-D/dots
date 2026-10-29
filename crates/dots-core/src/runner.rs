use std::borrow::Cow;
use std::collections::HashMap;
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
use crate::engine::{Engine, EngineEvent, RunContext};
use crate::events::{Bus, RuntimeEvent};
use crate::limits::{is_background, Admit, Limits, QueuedRun, STOPPED_BY_LIMIT};
use crate::model::{
    ApprovalStatus, Dot, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use crate::prompt::{build_prompt_full, context_tool_name, PromptParts};
use crate::run_tools::{CloseOnDrop, OpenRequest, RunTools};
use crate::store::Store;
use crate::util::{ct_eq, new_token};
use crate::vault::{DotScope, Vault};
use crate::workspace::{default_branch_ref, Prepared, WorkspaceManager};
use crate::{Error, Result};

/// The name the run's model knows a policy tool by: the `Context` tool is `mcp__dots__context`
/// on the Claude engine, `context` on the local one, and the check-in tools likewise
/// (`checkins::Tool::model_name`); a `Memory` call is the memory tool its `input` is for
/// (`memory::Tool::for_input`) and an `App` call the app tool its input names
/// (`apps::run::model_name`); every other tool keeps its name.
fn model_tool_name<'a>(tool: &'a str, input: &Value, engine: EngineKind) -> Cow<'a, str> {
    if tool == "Context" {
        Cow::Borrowed(context_tool_name(engine))
    } else if let Some(check_in) = CheckInTool::from_policy_name(tool) {
        Cow::Borrowed(check_in.model_name(engine))
    } else if tool == crate::memory::POLICY_NAME {
        Cow::Borrowed(crate::memory::Tool::for_input(input).model_name(engine))
    } else if tool == crate::team_tools::POLICY_NAME {
        Cow::Borrowed(crate::team_tools::Tool::for_input(input).model_name(engine))
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

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub max_concurrent: usize,
    pub mcp_url: String,
    pub worktrees_dir: PathBuf,
    pub cancel_grace: Duration,
    /// Where a Claude-engine run's plugin folders go (`<data_dir>/plugins`).
    pub plugins_dir: PathBuf,
}

struct Active {
    dot_id: String,
    /// A run of a workflow step: it does not keep its dot's other runs waiting, nor waits for
    /// them (teams spec section 8).
    workflow: bool,
    cancel: CancellationToken,
    cancelled_by_user: Arc<AtomicBool>,
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
    /// Claude's usage limits (usage and limits spec section 4), set once by the runtime
    /// (`set_limits`). Without them no run is held.
    limits: OnceLock<Arc<Limits>>,
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
            limits: OnceLock::new(),
        })
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
    pub async fn role_hold(&self, run: &Run) -> Result<Option<(String, String)>> {
        if !is_background(run.trigger) {
            return Ok(None);
        }
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
        if access.allows(run_dots) {
            return Ok(None);
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

    /// Removes a watcher run's attachments from its workspace.
    fn remove_attachments(&self, workspace: &Path, root_run_id: &str) {
        if self.watcher_files.get().is_some() {
            crate::watchers::files::WatcherFiles::remove_copy(workspace, root_run_id);
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
    /// at the root of its resumes (a follow-up, scheduled, webhook or manual run is not).
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
        cur.trigger == TriggerKind::Chat
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
        for (run, engine) in self.store.queued_with_engine(true).await? {
            let background_claude = engine == EngineKind::Claude && is_background(run.trigger);
            // A role that may not run dots keeps a background run queued, unstarted, until the
            // role returns (members spec section 6.3); the next pass looks again.
            if self.role_hold(&run).await?.is_some() {
                continue;
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
        // An error path may have left a watcher run's attachments in its workspace.
        if let Ok(stored) = self.store.get_run(&run_id).await {
            if let Some(path) = &stored.workspace_path {
                self.remove_attachments(Path::new(path), &stored.root_run_id);
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
            .map(|(_, s)| s.name.clone())
            .unwrap_or(step.step_id);
        Ok(Some(StepContext {
            repo: workflow_run.project.repo_path,
            default_branch: workflow_run.project.default_branch,
            remote: workflow_run.project.remote,
            step_name,
        }))
    }

    /// What happens to a run's worktree when the run stops for good: a run of a workflow step has
    /// what it left uncommitted committed by Dots (teams spec section 6.1; a failure is a notice
    /// on the run), then an unchanged worktree is removed. Errors are logged, never returned.
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
        if let Some(reason) = engine.unavailable() {
            return Err(Error::Other(reason));
        }
        // A run of a workflow step works in the project's repo (teams spec section 8). One that
        // continues the thread of a step that has ended works there no more, nor for the
        // workflow: it fails before its workspace is touched.
        if self.store.in_ended_step_thread(run).await? {
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
        let tools = Arc::new(tools);
        let _closer = CloseOnDrop(tools.clone());
        if cancel.is_cancelled() {
            tools.close().await;
            self.remove_attachments(&prepared.path, &run.root_run_id);
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
        let global = self.store.get_settings().await?.global_instructions;
        // The memories as they are now: a run's prompt does not change while it runs.
        let memories = self.store.list_memories(&dot.id).await?;
        let secret = self.register_secret(&run.id);
        self.register_tools(&run.id, tools.clone());
        let mut prompt = build_prompt_full(
            &dot,
            run,
            &prepared.path,
            chrono::Local::now(),
            &global,
            PromptParts {
                contexts: tools.contexts(),
                memories: &memories,
                apps: tools.apps().map(|apps| apps.as_ref()),
            },
        );
        // A chat's first prompt lists the workflows it may start (teams spec section 7).
        let startable = crate::team_tools::prompt_section(tools.startable(), dot.spec.engine);
        if run.session_id.is_none() && !startable.is_empty() {
            prompt.push_str("\n## Workflows you can start\n\n");
            prompt.push_str(&startable);
        }
        let ctx = RunContext {
            run_id: run.id.clone(),
            dot: dot.clone(),
            workspace: prepared.path.clone(),
            prompt,
            session_id: run.session_id.clone(),
            mcp_url: self.cfg.mcp_url.clone(),
            mcp_secret: secret,
            cancel: cancel.clone(),
            approved_calls,
            tools: tools.clone(),
        };
        let mut rx = engine.start(ctx).await?;

        let deadline = Instant::now() + Duration::from_secs(dot.spec.timeout_secs);
        let mut grace: Option<Instant> = None;
        let mut timed_out = false;
        let mut summary: Option<String> = None;
        let mut error: Option<String> = None;
        // Claude's `rejected` ending a background run (usage and limits spec section 4).
        let mut saw_tool_call = false;
        let mut saw_rejected = false;
        loop {
            let limit = grace.unwrap_or(deadline);
            tokio::select! {
                ev = rx.recv() => {
                    let Some(ev) = ev else { break };
                    self.record(&run.id, &ev).await?;
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
        tools.close().await;
        // Gone before the worktree's "unchanged" check: they are never a run's changes.
        self.remove_attachments(&prepared.path, &run.root_run_id);

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
            let stopped_by_limit =
                saw_rejected && dot.spec.engine == EngineKind::Claude && is_background(run.trigger);
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
                let queued = self.store.requeue_run(&run.id, run).await?;
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
        if user_cancelled || timed_out {
            self.store.expire_pending_for_run(&run.id).await?;
        }
        // Clean up before publishing the terminal status so observers that see
        // `succeeded`/`failed` never find a leftover unchanged worktree, nor a step's work left
        // uncommitted.
        if status != RunStatus::AwaitingApproval {
            self.finish_workspace(run, &ws_dot, step.as_ref(), &prepared, summary.as_deref())
                .await;
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
fn queued_run(run: &Run, engine: EngineKind) -> QueuedRun {
    QueuedRun {
        id: run.id.clone(),
        trigger: run.trigger,
        engine,
        hold_released: run.hold_released,
        queued_at: run.queued_at.clone(),
    }
}
