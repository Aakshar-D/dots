//! Workflow runs (teams and workflows spec sections 5 to 8). `Workflows` starts them and moves
//! each one along its steps: an agent step queues a run of its member and ends when that run's
//! thread does; a check waits for the person; a branch takes one of its paths at once. A run's
//! place is the id of its current step, anywhere in the definition's tree. Every move is made
//! by a pass that reads the workflow runs in flight from the store and does what each one's
//! state calls for, so a restart or a lost event never loses a step. The loop passes when a run
//! of a step changes, when a question is posted or answered, when it is woken, and every
//! `TICK`. Slow work a step needs (an agent step's hand-off, a deploy) runs as a job outside the
//! loop's lock (`jobs`). A delegate step's run may start other members, whose results continue
//! it (`delegate`). A check or an agent step may send the work back to an earlier agent step,
//! which goes on where it left off (`send_back`). A deploy step ships the branches its check
//! approved (`deploy_step`), at the tips the person was shown (`check`). "Open folder" finds a
//! project's folder and a ready branch's worktree by id (`folder`).

mod check;
mod delegate;
mod deploy_step;
mod folder;
mod jobs;
mod send_back;
mod tools;

pub use check::{branches_changed, CheckTip, CheckView, ShownBranch, CHECK_CHANGED};
pub use deploy_step::{INTERRUPTED, NO_APPROVAL, NO_DEFAULT_BRANCH};
pub use tools::STEP_CLOSED;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::accounts::Permission;
use crate::deploy::Deployer;
use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    BranchRef, Handoff, NewRun, NewWorkflowRun, Run, RunStatus, StartableWorkflow, StepDotRun,
    StepMember, StepRun, StepRunChange, StepRunStatus, TriggerKind, WaitingOn, WorkflowRun,
    WorkflowRunChange, WorkflowRunFilter, WorkflowRunStatus, WorkflowTrigger,
};
use crate::runner::Runner;
use crate::scheduler::Clock;
use crate::store::Store;
use crate::team_tools::StepTools;
use crate::util::timestamp;
use crate::vault::org::Orgs;
use crate::workflow::{BranchOn, BranchPath, Budget, Step, StepKind};
use crate::workspace::git;
use crate::{Error, Result};

/// The longest task, in characters after trimming.
pub const MAX_TASK: usize = 8_000;
/// The longest summary of an earlier step a prompt carries, in characters.
pub const MAX_HANDOFF_SUMMARY: usize = 4_000;
/// Why a run may not continue the thread of a workflow step that has ended (section 8): it
/// would work in the project's worktree outside the workflow.
pub const ENDED_STEP_THREAD: &str =
    "this chat was a step of a workflow, and the step has ended; start a new chat";
/// How often the loop passes while nothing wakes it.
pub const TICK: Duration = Duration::from_secs(30);
/// The most one tick counts into a run's `active_ms`. The loop passes at least every `TICK`, so
/// a longer gap (the app was closed, the machine slept) is time nobody was counting. A pass that
/// really took longer than about `TICK` drops the excess: an undercount, never an overcount.
const MAX_TICK_GAIN: Duration = Duration::from_secs(2 * TICK.as_secs());

/// What starts a workflow run.
#[derive(Debug, Clone, PartialEq)]
pub struct StartRequest {
    pub workflow_id: String,
    pub project_id: String,
    pub task: String,
    pub trigger: WorkflowTrigger,
    /// The chat run that started it (`start_workflow`).
    pub origin_run_id: Option<String>,
    /// A test run: its deploy steps push nothing (section 5.1).
    pub test: bool,
}

pub struct Workflows {
    store: Store,
    bus: Bus,
    runner: Arc<Runner>,
    clock: Arc<dyn Clock>,
    wake: Notify,
    /// Held by each pass and each of the person's actions, so no two move one run at once.
    lock: Mutex<()>,
    /// Held by `call_start` from its check that the chat has no such run in flight to the start,
    /// so two calls made at once cannot both pass it.
    start_lock: Mutex<()>,
    /// The orgs a team's account role is read from (members spec §6.3); without them, an
    /// account's team acts as `legacy`.
    orgs: OnceLock<Orgs>,
    /// These workflows, for a job to act on from its own task (`jobs`).
    this: Weak<Self>,
    /// The step runs with a job in flight: a pass starts no second one.
    jobs: std::sync::Mutex<HashSet<String>>,
    /// For tests: the gate jobs wait on before their slow part (`hold_jobs`).
    gate: std::sync::Mutex<Option<Arc<Semaphore>>>,
    /// What deploy steps push and open pull requests with (`set_deployer`); unset, `gh` from
    /// the PATH.
    deployer: OnceLock<Deployer>,
}

/// The newest step run of `step_id`.
fn latest<'a>(wr: &'a WorkflowRun, step_id: &str) -> Option<&'a StepRun> {
    wr.steps.iter().rev().find(|s| s.step_id == step_id)
}

/// The newest succeeded step run of `step_id`.
fn succeeded<'a>(wr: &'a WorkflowRun, step_id: &str) -> Option<&'a StepRun> {
    wr.steps
        .iter()
        .rev()
        .find(|s| s.step_id == step_id && s.status == StepRunStatus::Succeeded)
}

/// The attempt a new step run of `step_id` is: one more than the runs it has had.
fn next_attempt(wr: &WorkflowRun, step_id: &str) -> u32 {
    wr.steps.iter().filter(|s| s.step_id == step_id).count() as u32 + 1
}

/// Whether `id` is `step` or a step inside it, at any depth.
fn holds(step: &Step, id: &str) -> bool {
    step.id == id
        || match &step.kind {
            StepKind::Branch {
                paths, otherwise, ..
            } => paths
                .iter()
                .flat_map(|p| &p.steps)
                .chain(otherwise)
                .any(|s| holds(s, id)),
            _ => false,
        }
}

/// The steps of the path the branch step `branch` took in this run: the path its newest
/// succeeded step run chose (`decision.path`), `otherwise` when it chose none. Empty for a
/// branch that has not run and for the other kinds.
fn taken<'a>(wr: &WorkflowRun, branch: &'a Step) -> &'a [Step] {
    let StepKind::Branch {
        paths, otherwise, ..
    } = &branch.kind
    else {
        return &[];
    };
    let Some(run) = succeeded(wr, &branch.id) else {
        return &[];
    };
    let label = run
        .decision
        .as_ref()
        .and_then(|d| d.get("path"))
        .and_then(serde_json::Value::as_str);
    match label {
        Some(label) => paths
            .iter()
            .find(|p| p.label.trim() == label)
            .map_or(&[], |p| p.steps.as_slice()),
        None => otherwise,
    }
}

/// Pushes the steps that ran inside `step` in this run, in run order: for a branch, the steps
/// of the path it took and, recursively, of the branches on that path.
fn ran_inside<'a>(wr: &WorkflowRun, step: &'a Step, out: &mut Vec<&'a Step>) {
    for s in taken(wr, step) {
        out.push(s);
        ran_inside(wr, s, out);
    }
}

/// The steps that ran before the step `id` in this run, oldest first: `Definition::before`,
/// with the steps of the path each earlier branch took after that branch. A path a branch did
/// not take is never in it. The branches `id` is inside are not opened: the steps of its own
/// path before it are already in `before`.
fn ran_before<'a>(wr: &'a WorkflowRun, id: &str) -> Vec<&'a Step> {
    let mut out = Vec::new();
    for step in wr.definition.before(id) {
        out.push(step);
        if !holds(step, id) {
            ran_inside(wr, step, &mut out);
        }
    }
    out
}

/// The step to begin after the step `id` succeeded: for a branch, the first step of the path
/// it took, else (an empty path) the step after the branch; for the others the step after it.
/// `None` past the last step: the workflow run has succeeded.
fn next_of<'a>(wr: &'a WorkflowRun, id: &str) -> Option<&'a Step> {
    let step = wr.definition.step(id)?;
    taken(wr, step)
        .first()
        .or_else(|| wr.definition.next_after(id))
}

/// The path a branch step takes (section 5.6): the first whose label matches, `None` for
/// `otherwise`. On `outcome` the label is compared with the outcome the newest succeeded run
/// of the nearest agent step before it reported, as the definition declares it (after
/// trimming); on `project` with the workflow run's project name, without case.
fn choose<'a>(
    wr: &WorkflowRun,
    id: &str,
    on: BranchOn,
    paths: &'a [BranchPath],
) -> Option<&'a BranchPath> {
    match on {
        BranchOn::Outcome => {
            let source = wr
                .definition
                .before(id)
                .into_iter()
                .rev()
                .find(|s| matches!(s.kind, StepKind::Agent { .. }))?;
            let outcome = succeeded(wr, &source.id)?.outcome.as_deref()?.trim();
            paths.iter().find(|p| p.label.trim() == outcome)
        }
        BranchOn::Project => {
            let project = wr.project.name.trim();
            paths
                .iter()
                .find(|p| p.label.trim().eq_ignore_ascii_case(project))
        }
    }
}

/// The hand-offs of the steps a member worked on that ran before the step `id` (`ran_before`):
/// agent steps and `member` deploys, oldest first, each step's newest succeeded run.
fn handoffs_before<'a>(wr: &'a WorkflowRun, id: &str) -> Vec<(&'a Step, &'a Handoff)> {
    ran_before(wr, id)
        .into_iter()
        .filter(|s| deploy_step::worker(wr, s).is_some())
        .filter_map(|s| Some((s, succeeded(wr, &s.id)?.handoff.as_ref()?)))
        .collect()
}

/// The newest of `handoffs` that has branches. A step that changed nothing hands on none (its
/// worktree went with its branch), so the work it saw is still the branches before it.
fn newest_with_branches<'a>(handoffs: &[(&'a Step, &'a Handoff)]) -> Option<&'a Handoff> {
    handoffs
        .iter()
        .rev()
        .map(|(_, h)| *h)
        .find(|h| !h.branches.is_empty())
}

/// The branch the step `id` starts from (section 6): the newest earlier hand-off that has
/// branches, when it has exactly one, as its full ref (a tag of the same name must not stand
/// in for it). `None` starts from the project's default branch.
fn start_branch(wr: &WorkflowRun, id: &str) -> Option<String> {
    let newest = newest_with_branches(&handoffs_before(wr, id))?;
    match newest.branches.as_slice() {
        [only] => Some(format!("refs/heads/{}", only.branch)),
        _ => None,
    }
}

/// The hand-off the check `id` shows (sections 5.4 and 6.2): the newest earlier one, with the
/// branches of the newest earlier one that has branches, as `start_branch` finds them. A step
/// that changed nothing (a reviewer that only read) so leaves the branch it was shown to the
/// check, and a deploy after it ships that branch.
fn shown_at(wr: &WorkflowRun, id: &str) -> Handoff {
    let before = handoffs_before(wr, id);
    let mut shown = before.last().map(|(_, h)| (*h).clone()).unwrap_or_default();
    if shown.branches.is_empty() {
        if let Some(h) = newest_with_branches(&before) {
            shown.branches = h.branches.clone();
        }
    }
    shown
}

/// The limit the workflow run has reached, if any (section 9), as the person reads it: a new
/// dot run of it may start only while none is reached.
pub fn budget_reached(wr: &WorkflowRun) -> Option<String> {
    let b = &wr.budget;
    if wr.spent_runs >= b.max_runs {
        return Some(format!("runs, {} of {}", wr.spent_runs, b.max_runs));
    }
    if wr.spent_tokens >= b.max_tokens {
        return Some(format!("tokens, {} of {}", wr.spent_tokens, b.max_tokens));
    }
    let minutes = wr.active_ms / 60_000;
    if wr.active_ms >= u64::from(b.max_minutes) * 60_000 {
        return Some(format!("minutes, {minutes} of {}", b.max_minutes));
    }
    None
}

/// `budget` with each limit `wr` has reached raised by that limit's default amount (section 9,
/// "Add budget").
fn raised(wr: &WorkflowRun) -> Budget {
    let mut b = wr.budget;
    let more = Budget::default();
    if wr.spent_runs >= b.max_runs {
        b.max_runs = b.max_runs.saturating_add(more.max_runs);
    }
    if wr.spent_tokens >= b.max_tokens {
        b.max_tokens = b.max_tokens.saturating_add(more.max_tokens);
    }
    if wr.active_ms >= u64::from(b.max_minutes) * 60_000 {
        b.max_minutes = b.max_minutes.saturating_add(more.max_minutes);
    }
    b
}

/// `text` in at most `max` characters: a longer one is cut short and ends with " [cut]".
fn cut(text: &str, max: usize) -> String {
    const MARK: &str = " [cut]";
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(MARK.chars().count());
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(MARK);
    out
}

/// Where a step's prompt places it: the project, and the step with its number among all the
/// steps of the tree, branches and the paths not taken included.
fn place(wr: &WorkflowRun, step: &Step) -> String {
    format!(
        "- Project: {} ({})\n- Step: {} ({} of {})\n\n",
        wr.project.name,
        wr.project.repo_path,
        step.name,
        wr.definition.position(&step.id).unwrap_or_default() + 1,
        wr.definition.all().len()
    )
}

/// Whether the branch `branch` exists in `repo`, as a branch (a tag of that name is not it).
async fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok()
}

/// Whether the branch `branch` has commits since `base`, the commit its worktree started on;
/// without a base, whether it exists.
async fn has_commits(repo: &Path, branch: &str, base: Option<&str>) -> bool {
    let Some(base) = base else {
        return branch_exists(repo, branch).await;
    };
    let range = format!("{base}..refs/heads/{branch}");
    git(repo, &["rev-list", "--count", "--end-of-options", &range])
        .await
        .is_ok_and(|n| n.trim().parse::<u64>().is_ok_and(|n| n > 0))
}

/// Whether the tip of the branch `branch` is an ancestor of the tip of the branch `of`, or the
/// same commit: all the work of `branch` is in `of`. `false` when git cannot tell (a branch
/// gone), so the branch is shown rather than hidden.
async fn merged_into(repo: &Path, branch: &str, of: &str) -> bool {
    let branch = format!("refs/heads/{branch}");
    let of = format!("refs/heads/{of}");
    let args = [
        "merge-base",
        "--is-ancestor",
        "--end-of-options",
        &branch,
        &of,
    ];
    git(repo, &args).await.is_ok()
}

/// The hand-off's reference to `branch`, the branch of `run`.
fn branch_ref(run: &Run, branch: String) -> BranchRef {
    BranchRef {
        run_id: run.id.clone(),
        branch,
        worktree_path: run.workspace_path.clone().unwrap_or_default(),
        base_commit: run.base_commit.clone(),
    }
}

impl Workflows {
    pub fn new(store: Store, bus: Bus, runner: Arc<Runner>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            store,
            bus,
            runner,
            clock,
            wake: Notify::new(),
            lock: Mutex::new(()),
            start_lock: Mutex::new(()),
            orgs: OnceLock::new(),
            this: this.clone(),
            jobs: std::sync::Mutex::new(HashSet::new()),
            gate: std::sync::Mutex::new(None),
            deployer: OnceLock::new(),
        })
    }

    /// The orgs whose roles gate what a person does with an account's workflow runs.
    pub fn set_orgs(&self, orgs: Orgs) {
        let _ = self.orgs.set(orgs);
    }

    /// `Ok` when this PC may do `p` in the account and workspace of the team `team_id`
    /// (members spec §6.3).
    async fn require(&self, team_id: &str, p: Permission) -> Result<()> {
        let team = self.store.get_team(team_id).await?;
        crate::accounts::require_team(self.orgs.get(), &team, p).await
    }

    fn updated(&self, wr: &WorkflowRun) {
        let _ = self
            .bus
            .send(RuntimeEvent::WorkflowRunUpdated { run: wr.clone() });
    }

    /// Announces what a step that ended changed beyond its row: the follow-ups its runs
    /// scheduled are cancelled with it (`Store::set_step_run`).
    fn step_ended(&self, wr: &WorkflowRun, step_id: &str) {
        if let Some((member, _)) = wr
            .definition
            .step(step_id)
            .and_then(|step| deploy_step::worker(wr, step))
        {
            let _ = self.bus.send(RuntimeEvent::FollowUpsUpdated {
                dot_id: member.to_string(),
            });
        }
    }

    /// Makes the loop pass now.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub async fn get(&self, id: &str) -> Result<WorkflowRun> {
        self.store.get_workflow_run(id).await
    }

    /// The workflow runs `filter` picks (by team, workflow and status), newest first.
    pub async fn list(&self, filter: &WorkflowRunFilter) -> Result<Vec<WorkflowRun>> {
        self.store.find_workflow_runs(filter).await
    }

    /// The workflow runs the chat runs `run_ids` started (`origin_run_id`), oldest first: a
    /// chat shows each one's card after the turn that started it (section 10.4).
    pub async fn for_origins(&self, run_ids: &[String]) -> Result<Vec<WorkflowRun>> {
        self.store.workflow_runs_from(run_ids).await
    }

    /// The dot runs of the workflow run `id`'s step runs, oldest first: the run page links each
    /// step to its own (section 10.3). `NotFound` for an unknown workflow run.
    pub async fn dot_runs(&self, id: &str) -> Result<Vec<StepDotRun>> {
        self.store.workflow_run_dot_runs(id).await
    }

    /// Starts a run of a workflow on one of its team's projects. Refused for a workflow that is
    /// turned off (except a Test run) or has a `problem`, a project of another team or whose
    /// folder is no longer a git repo, and a task that is blank or longer than `MAX_TASK`.
    pub async fn start(&self, req: StartRequest) -> Result<WorkflowRun> {
        let task = req.task.trim();
        if task.is_empty() || task.chars().count() > MAX_TASK {
            return Err(Error::Invalid(
                "the task must be 1 to 8,000 characters".into(),
            ));
        }
        let workflow = self.store.get_workflow(&req.workflow_id).await?;
        self.require(&workflow.team_id, Permission::RunDots).await?;
        let name = &workflow.spec.name;
        // `enabled` arms the chat, schedule and webhook triggers and Run now. A Test run (the
        // person's manual start with `test`) starts while the workflow is off, so a new
        // workflow can be tried before it is armed.
        let test_run = req.test && matches!(req.trigger, WorkflowTrigger::Manual);
        if !workflow.spec.enabled && !test_run {
            return Err(Error::Conflict(format!("workflow {name} is turned off")));
        }
        if let Some(problem) = &workflow.problem {
            return Err(Error::Invalid(format!(
                "workflow {name} cannot start: {problem}"
            )));
        }
        let project = self.store.get_project(&req.project_id).await?;
        if project.team_id != workflow.team_id {
            return Err(Error::Invalid(format!(
                "{} is not a project of the workflow's team",
                project.spec.name
            )));
        }
        crate::teams::check_repo(&project.spec.repo_path).await?;
        let wr = self
            .store
            .create_workflow_run(&NewWorkflowRun {
                workflow_id: workflow.id.clone(),
                team_id: workflow.team_id.clone(),
                definition: workflow.spec.definition.clone(),
                project_id: project.id.clone(),
                project: project.spec.clone(),
                task: task.to_string(),
                trigger: req.trigger,
                origin_run_id: req.origin_run_id,
                test: req.test,
            })
            .await?;
        // Its clock starts on the clock it is counted with.
        let wr = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    clock_at: Some(Some(timestamp(self.clock.now()))),
                    ..Default::default()
                },
            )
            .await?;
        self.updated(&wr);
        self.wake();
        Ok(wr)
    }

    /// Raises each limit the waiting workflow run has reached by its default amount, once, and
    /// lets it go on (section 9).
    pub async fn add_budget(&self, id: &str) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.store.get_workflow_run(id).await?;
        self.require(&wr.team_id, Permission::RunDots).await?;
        let wr = self.tick(&wr).await?;
        if !(wr.status == WorkflowRunStatus::Waiting && wr.waiting_on == Some(WaitingOn::Budget)) {
            return Err(Error::Conflict(format!(
                "workflow run {id} is not waiting for budget"
            )));
        }
        let running = self
            .store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    waiting_on: Some(None),
                    budget: Some(raised(&wr)),
                    error: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        // A retry the limit held: its step failed, so it starts again here, as `retry_step`
        // does, instead of failing the run back to the failed step.
        let held_retry = running.current_step.as_deref().and_then(|current| {
            let failed =
                latest(&running, current).is_some_and(|s| s.status == StepRunStatus::Failed);
            failed.then(|| running.definition.step(current)).flatten()
        });
        match held_retry {
            Some(step) => self.begin_step(&running, step).await?,
            None => self.advance(id).await?,
        }
        let wr = self.store.get_workflow_run(id).await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Counts the time since the workflow run's clock last counted into `active_ms`, unless it
    /// waited on the person (section 9): a check, a limit, a failed step, or a question or an
    /// approval of one of its runs. Reads the time from the clock the runtime was given.
    async fn tick(&self, wr: &WorkflowRun) -> Result<WorkflowRun> {
        if wr.status.is_terminal() {
            return Ok(wr.clone());
        }
        let now = self.clock.now();
        let last = self
            .store
            .workflow_run_clock(&wr.id)
            .await?
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        let active = wr.status == WorkflowRunStatus::Running
            && !self.store.workflow_waits_on_person(&wr.id).await?;
        let gained = match (active, last) {
            (true, Some(last)) if now > last => {
                let gap = (now - last).num_milliseconds().max(0) as u64;
                gap.min(MAX_TICK_GAIN.as_millis() as u64)
            }
            _ => 0,
        };
        self.store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    active_ms: Some(wr.active_ms + gained),
                    clock_at: Some(Some(timestamp(now))),
                    ..Default::default()
                },
            )
            .await
    }

    /// Performs an allowed `start_workflow` call of the chat run `run_id`, which may start the
    /// workflows `startable` (section 7). `Ok` is the text the model gets; `Err` a tool error,
    /// in one sentence, and the run goes on.
    pub async fn call_start(
        &self,
        run_id: &str,
        startable: &[StartableWorkflow],
        input: &serde_json::Value,
    ) -> std::result::Result<String, String> {
        let arg = |key: &str| {
            input
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string()
        };
        let (name, project, task) = (arg("workflow"), arg("project"), arg("task"));
        if name.is_empty() || project.is_empty() {
            return Err("Give `workflow` and `project` by their names as listed.".into());
        }
        let found: Vec<&StartableWorkflow> = startable
            .iter()
            .filter(|w| {
                w.workflow.eq_ignore_ascii_case(&name)
                    || format!("{}/{}", w.team, w.workflow).eq_ignore_ascii_case(&name)
            })
            .collect();
        let workflow = match found.as_slice() {
            [one] => *one,
            [] => {
                return Err(format!(
                    "There is no workflow named \"{name}\" that you may start."
                ))
            }
            _ => {
                return Err(format!(
                "Several teams have a workflow named \"{name}\"; give it as \"<team>/<workflow>\"."
            ))
            }
        };
        // The list was taken when the run was prepared: the workflow may since have stopped
        // naming the member, or the member left the team.
        let caller = self.store.get_run(run_id).await.ok();
        let still = match &caller {
            Some(run) => self.store.startable_workflows(&run.dot_id).await.ok(),
            None => None,
        };
        let may = still.is_some_and(|l| l.iter().any(|w| w.workflow_id == workflow.workflow_id));
        let Some(caller) = caller.filter(|_| may) else {
            return Err(format!(
                "There is no workflow named \"{name}\" that you may start."
            ));
        };
        let Some(project) = workflow
            .projects
            .iter()
            .find(|p| p.eq_ignore_ascii_case(&project))
        else {
            return Err(format!(
                "The team \"{}\" has no project named \"{project}\".",
                workflow.team
            ));
        };
        let project_id = match self
            .store
            .get_team_project_id(&workflow.workflow_id, project)
            .await
        {
            Ok(id) => id,
            Err(e) => return Err(format!("Not started: {e}.")),
        };
        // One run of a workflow on a project per chat at a time: a model that calls again, in a
        // loop or in parallel, does not start the same work twice and spend its budget twice.
        let _one_start = self.start_lock.lock().await;
        match self
            .store
            .workflow_in_flight_from_thread(&workflow.workflow_id, &project_id, &caller.root_run_id)
            .await
        {
            Ok(false) => {}
            Ok(true) => {
                return Err(format!(
                    "Not started: \"{}\" is already running on \"{project}\" from this chat.",
                    workflow.workflow
                ))
            }
            Err(e) => return Err(format!("Not started: {e}.")),
        }
        let started = self
            .start(StartRequest {
                workflow_id: workflow.workflow_id.clone(),
                project_id,
                task,
                trigger: WorkflowTrigger::Chat,
                origin_run_id: Some(run_id.to_string()),
                test: false,
            })
            .await;
        match started {
            Ok(_) => Ok(format!(
                "Workflow \"{}\" started on \"{project}\".",
                workflow.workflow
            )),
            Err(Error::Invalid(m)) | Err(Error::Conflict(m)) => Err(format!("Not started: {m}.")),
            Err(e) => {
                tracing::warn!(run = %run_id, "starting a workflow failed: {e}");
                Err(format!("Not started: {e}."))
            }
        }
    }

    /// Approves the check the workflow run waits on (section 5.4): the check passes the hand-off
    /// it was shown on, with the tip of each of its branches (section 6.2), and the workflow
    /// goes on. `step_run_id` is the step run the check page showed (`check`): when the run
    /// waits on another one now, the approval is refused (`Conflict`, `CHECK_CHANGED`) and
    /// nothing is recorded. `tips` are the branches and commits the check page showed: when they
    /// are not exactly the shown branches at their tips now, the approval is refused
    /// (`Conflict`, `branches_changed`) and nothing is recorded.
    pub async fn approve_check(
        &self,
        id: &str,
        note: Option<&str>,
        step_run_id: &str,
        tips: &[CheckTip],
    ) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.store.get_workflow_run(id).await?;
        self.require(&wr.team_id, Permission::AnswerApprovals)
            .await?;
        let wr = self.tick(&wr).await?;
        let Some(step_run) = check::waiting_check(&wr).cloned() else {
            return Err(Error::Conflict(format!(
                "workflow run {id} is not waiting for a check"
            )));
        };
        // The check the person saw, read again under the lock: two checks in a row can show the
        // same tips, so only the step run tells the next check from the one approved.
        if step_run.id != step_run_id {
            return Err(Error::Conflict(CHECK_CHANGED.into()));
        }
        let shown = shown_at(&wr, &step_run.step_id);
        // The tips the person approved, read again under the lock: a deploy after the check
        // pushes exactly these, so they must be what the page showed. Equal to the page's tips,
        // and kept in the order the check shows them.
        let branches = deploy_step::tips(Path::new(&wr.project.repo_path), &shown).await;
        let differ = check::changed(&branches, tips);
        if !differ.is_empty() {
            return Err(Error::Conflict(branches_changed(&differ)));
        }
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        let decision = json!({"decision": "approved", "note": note, "branches": branches});
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Succeeded),
                    handoff: Some(Some(shown)),
                    decision: Some(Some(decision)),
                    ..Default::default()
                },
            )
            .await?;
        self.store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    waiting_on: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        self.advance(id).await?;
        let wr = self.store.get_workflow_run(id).await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Runs the failed step again, as a new attempt (sections 8 and 10.3).
    pub async fn retry_step(&self, id: &str) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.store.get_workflow_run(id).await?;
        self.require(&wr.team_id, Permission::RunDots).await?;
        let wr = self.tick(&wr).await?;
        let current = match (&wr.status, &wr.waiting_on, &wr.current_step) {
            (WorkflowRunStatus::Waiting, Some(WaitingOn::StepFailed), Some(current))
                if wr.definition.step(current).is_some() =>
            {
                Some(current.clone())
            }
            _ => None,
        };
        let Some(current) = current else {
            return Err(Error::Conflict(format!(
                "workflow run {id} has no failed step to retry"
            )));
        };
        let wr = self
            .store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    waiting_on: Some(None),
                    error: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        if let Some(step) = wr.definition.step(&current) {
            self.begin_step(&wr, step).await?;
        }
        let wr = self.store.get_workflow_run(id).await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Stops the workflow run (section 8): its queued and running dot runs are cancelled, the
    /// questions they left open are dismissed, its open steps and a waiting check are closed as
    /// cancelled, and with them the follow-ups their runs scheduled. Refused without the
    /// permission to run dots in the team's account.
    pub async fn stop(&self, id: &str) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.store.get_workflow_run(id).await?;
        self.require(&wr.team_id, Permission::RunDots).await?;
        self.stop_locked(id).await
    }

    async fn stop_locked(&self, id: &str) -> Result<WorkflowRun> {
        let wr = self.store.get_workflow_run(id).await?;
        if wr.status.is_terminal() {
            return Err(Error::Conflict(format!(
                "workflow run {id} is already {}",
                wr.status.as_str()
            )));
        }
        for step_run in wr.steps.iter().filter(|s| !s.status.is_terminal()) {
            // The questions first: an answer given before is a run of the step, cancelled
            // below; one given after is refused.
            for message in self.store.dismiss_step_questions(&step_run.id).await? {
                let _ = self.bus.send(RuntimeEvent::MessageUpdated { message });
            }
            // Then the step, which closes it to new runs of its threads (`insert_run`).
            self.store
                .set_step_run(
                    &step_run.id,
                    &StepRunChange {
                        status: Some(StepRunStatus::Cancelled),
                        ..Default::default()
                    },
                )
                .await?;
            self.step_ended(&wr, &step_run.step_id);
            for run in self.store.runs_of_step_run(&step_run.id).await? {
                if !run.status.is_terminal() {
                    if let Err(e) = self.runner.cancel(&run.id).await {
                        tracing::warn!(run = %run.id, "cancelling a workflow's run failed: {e}");
                    }
                }
            }
        }
        let wr = self
            .store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Cancelled),
                    waiting_on: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Stops the team's workflow runs in flight, then deletes the team (section 4), and
    /// announces each workflow it removed and the team.
    pub async fn delete_team(&self, team_id: &str) -> Result<()> {
        self.require(team_id, Permission::EditDots).await?;
        let _one = self.lock.lock().await;
        let in_flight = WorkflowRunFilter {
            team_id: Some(team_id.to_string()),
            statuses: vec![WorkflowRunStatus::Running, WorkflowRunStatus::Waiting],
            ..Default::default()
        };
        for wr in self.store.find_workflow_runs(&in_flight).await? {
            self.stop_locked(&wr.id).await?;
        }
        let workflows = self.store.list_workflows(team_id).await?;
        self.store.delete_team(team_id).await?;
        for workflow in &workflows {
            let _ = self.bus.send(RuntimeEvent::WorkflowUpdated {
                team_id: team_id.to_string(),
                workflow_id: workflow.id.clone(),
            });
        }
        let _ = self.bus.send(RuntimeEvent::TeamUpdated {
            team_id: team_id.to_string(),
        });
        Ok(())
    }

    /// Runs the loop until `shutdown`.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        // Subscribed before the first pass, so no change it should see is missed.
        let mut events = self.bus.subscribe();
        tokio::spawn(async move {
            loop {
                if let Err(e) = this.pass().await {
                    tracing::error!("workflow pass failed: {e}");
                }
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = this.wake.notified() => {}
                    _ = relevant(&mut events) => {}
                    _ = tokio::time::sleep(TICK) => {}
                }
            }
        })
    }

    /// One pass over the workflow runs in flight. A run whose move fails is logged and tried
    /// again on the next pass.
    pub async fn pass(&self) -> Result<()> {
        let _one = self.lock.lock().await;
        for id in self.store.workflow_runs_in_flight().await? {
            if let Err(e) = self.advance(&id).await {
                tracing::warn!(workflow_run = %id, "moving the workflow run failed: {e}");
            }
        }
        Ok(())
    }

    /// Does what the workflow run's state calls for: start its first step, watch the step that
    /// runs, or start the step after one that succeeded. A waiting run waits for the person.
    async fn advance(&self, id: &str) -> Result<()> {
        let wr = self.tick(&self.store.get_workflow_run(id).await?).await?;
        if wr.status != WorkflowRunStatus::Running {
            return Ok(());
        }
        let Some(current) = wr.current_step.clone() else {
            return self.begin_or_finish(&wr, wr.definition.first()).await;
        };
        let Some(step) = wr.definition.step(&current) else {
            return Err(Error::Other(format!(
                "step {current} is not in the workflow"
            )));
        };
        match latest(&wr, &current).map(|s| (s.status, s.clone())) {
            None => self.begin_step(&wr, step).await,
            // A branch cut off between its two writes: it chooses now.
            Some((StepRunStatus::Running, step_run))
                if matches!(step.kind, StepKind::Branch { .. }) =>
            {
                let next = self.take_branch(&wr, step, Some(&step_run)).await?;
                self.begin_or_finish(&wr, next).await
            }
            // A deploy with no member to watch: its job runs, or Dots stopped during it.
            Some((StepRunStatus::Running, step_run))
                if matches!(step.kind, StepKind::Deploy { .. })
                    && deploy_step::worker(&wr, step).is_none() =>
            {
                self.watch_deploy(&wr, step, &step_run).await
            }
            Some((StepRunStatus::Running, step_run)) => {
                self.watch_agent(&wr, step, &step_run).await
            }
            Some((StepRunStatus::Succeeded, _)) => self.move_on(&wr, step).await,
            // A move cut off between its two writes (a crash, a store error) leaves the run
            // `Running` behind a step that is failed or waiting for the person: put the run
            // where the person can act on it.
            Some((StepRunStatus::Failed, step_run)) => {
                let error = step_run
                    .error
                    .clone()
                    .unwrap_or_else(|| "the step failed".to_string());
                let waiting = self
                    .store
                    .set_workflow_run(
                        &wr.id,
                        &WorkflowRunChange {
                            status: Some(WorkflowRunStatus::Waiting),
                            waiting_on: Some(Some(WaitingOn::StepFailed)),
                            error: Some(Some(format!("{}: {error}", step.name))),
                            ..Default::default()
                        },
                    )
                    .await?;
                self.updated(&waiting);
                Ok(())
            }
            Some((StepRunStatus::Waiting, _)) => {
                let waiting = self
                    .store
                    .set_workflow_run(
                        &wr.id,
                        &WorkflowRunChange {
                            status: Some(WorkflowRunStatus::Waiting),
                            waiting_on: Some(Some(WaitingOn::Check)),
                            ..Default::default()
                        },
                    )
                    .await?;
                self.updated(&waiting);
                Ok(())
            }
            Some(_) => Ok(()),
        }
    }

    /// Begins `next`; `None` (past the last step) succeeds the workflow run.
    async fn begin_or_finish(&self, wr: &WorkflowRun, next: Option<&Step>) -> Result<()> {
        match next {
            Some(step) => self.begin_step(wr, step).await,
            None => self.finish(wr).await,
        }
    }

    /// Succeeds the workflow run: it is past its last step.
    async fn finish(&self, wr: &WorkflowRun) -> Result<()> {
        let done = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Succeeded),
                    current_step: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        self.updated(&done);
        Ok(())
    }

    /// Begins `step`, a step of `wr`'s definition: an agent step queues its member's run, a
    /// check waits for the person, a deploy deploys (`deploy_step`), and a branch takes its path
    /// at once and begins the step that path leads to (past the last step, the workflow run has
    /// succeeded).
    async fn begin_step<'a>(&self, wr: &'a WorkflowRun, step: &'a Step) -> Result<()> {
        let mut step = step;
        // A loop, not a call of itself: a branch whose path starts with a branch takes both
        // in one move, and an async fn cannot call itself without a box.
        while matches!(step.kind, StepKind::Branch { .. }) {
            match self.take_branch(wr, step, None).await? {
                Some(next) => step = next,
                None => return self.finish(wr).await,
            }
        }
        let attempt = next_attempt(wr, &step.id);
        match &step.kind {
            StepKind::Check { .. } => {
                self.store
                    .create_step_run(&wr.id, &step.id, attempt, StepRunStatus::Waiting)
                    .await?;
                let waiting = self
                    .store
                    .set_workflow_run(
                        &wr.id,
                        &WorkflowRunChange {
                            status: Some(WorkflowRunStatus::Waiting),
                            waiting_on: Some(Some(WaitingOn::Check)),
                            current_step: Some(Some(step.id.clone())),
                            ..Default::default()
                        },
                    )
                    .await?;
                self.updated(&waiting);
            }
            StepKind::Agent { member, .. } => {
                // No new dot run past a limit: the workflow waits for the person (section 9).
                if let Some(limit) = budget_reached(wr) {
                    return self.wait_on_budget(wr, &limit).await;
                }
                let refusal = self.member_refusal(member).await?;
                let status = if refusal.is_some() {
                    StepRunStatus::Failed
                } else {
                    StepRunStatus::Running
                };
                let step_run = self.step_run_to_begin(wr, step, status).await?;
                self.store
                    .set_workflow_run(
                        &wr.id,
                        &WorkflowRunChange {
                            current_step: Some(Some(step.id.clone())),
                            ..Default::default()
                        },
                    )
                    .await?;
                match refusal {
                    Some(error) => self.fail_step(wr, step, &step_run, &error).await?,
                    None => {
                        // Read again: the hand-offs before the step include the paths the
                        // branches just taken chose.
                        let wr = self.store.get_workflow_run(&wr.id).await?;
                        self.queue_first(&wr, step, &step_run).await?;
                        self.updated(&self.store.get_workflow_run(&wr.id).await?);
                    }
                }
            }
            StepKind::Deploy { .. } => self.begin_deploy(wr, step).await?,
            StepKind::Branch { .. } => {}
        }
        Ok(())
    }

    /// Runs the branch step `step` (section 5.6): chooses its path (`choose`) and records the
    /// choice on a step run that succeeds at once, `decision: {"path": <label> | null}`, with
    /// the branch as the current step. `step_run` is one a move cut off left running. Gives the
    /// step to begin next: the path's first, else the step after the branch.
    async fn take_branch<'a>(
        &self,
        wr: &'a WorkflowRun,
        step: &'a Step,
        step_run: Option<&StepRun>,
    ) -> Result<Option<&'a Step>> {
        let StepKind::Branch {
            on,
            paths,
            otherwise,
        } = &step.kind
        else {
            return Ok(wr.definition.next_after(&step.id));
        };
        let path = choose(wr, &step.id, *on, paths);
        let step_run = match step_run {
            Some(run) => run.clone(),
            None => {
                self.step_run_to_begin(wr, step, StepRunStatus::Running)
                    .await?
            }
        };
        self.store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    current_step: Some(Some(step.id.clone())),
                    ..Default::default()
                },
            )
            .await?;
        // One write: the choice and the end, so a branch that reads as succeeded has chosen.
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Succeeded),
                    decision: Some(Some(json!({"path": path.map(|p| p.label.trim())}))),
                    ..Default::default()
                },
            )
            .await?;
        let first = path
            .map_or(otherwise.as_slice(), |p| p.steps.as_slice())
            .first();
        Ok(first.or_else(|| wr.definition.next_after(&step.id)))
    }

    /// The step run `step` begins on (an agent step, a branch, a deploy), which then becomes
    /// the current step in a second write: the newest step run of `step` when it is `Running`
    /// and has no runs, as a begin cut off between those two writes (Dots stopped) left it, so
    /// it is not left running beside a new attempt; else a new one, its next attempt, with
    /// `status`. Every step run of a step being begun has otherwise ended, as
    /// `perform_send_back` also relies on.
    async fn step_run_to_begin(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        status: StepRunStatus,
    ) -> Result<StepRun> {
        if let Some(cut) = latest(wr, &step.id).filter(|s| s.status == StepRunStatus::Running) {
            if self.store.runs_of_step_run(&cut.id).await?.is_empty() {
                return Ok(cut.clone());
            }
        }
        self.store
            .create_step_run(&wr.id, &step.id, next_attempt(wr, &step.id), status)
            .await
    }

    /// Puts the workflow run in wait for the person to add budget, past `limit` (section 9).
    async fn wait_on_budget(&self, wr: &WorkflowRun, limit: &str) -> Result<()> {
        let waiting = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Waiting),
                    waiting_on: Some(Some(WaitingOn::Budget)),
                    error: Some(Some(format!("Budget reached: {limit}"))),
                    ..Default::default()
                },
            )
            .await?;
        self.updated(&waiting);
        Ok(())
    }

    /// Why the member of an agent step may not get a run, if it may not: it is disabled, or no
    /// longer a dot.
    async fn member_refusal(&self, member: &str) -> Result<Option<String>> {
        match self.store.get_dot(member).await {
            Ok(dot) if dot.spec.enabled => Ok(None),
            Ok(dot) => Ok(Some(format!("{} is disabled", dot.spec.name))),
            Err(Error::NotFound(_)) => Ok(Some("its member is no longer a sol".to_string())),
            Err(e) => Err(e),
        }
    }

    /// Queues the run of an agent step's member, with the `## Workflow` section and the branch
    /// it starts from in its payload (sections 5.2 and 6). `again` starts afresh a step that
    /// went back but whose earlier session cannot be resumed (`send_back`): from the branch it
    /// names, with the send-back's message in the section.
    async fn queue_agent_run(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
        again: Option<&send_back::Again>,
    ) -> Result<Run> {
        let Some((member, _)) = deploy_step::worker(wr, step) else {
            return Err(Error::Other(format!(
                "step {} is not an agent step",
                step.id
            )));
        };
        let section = self
            .section(wr, step, step_run, again.map(|a| a.message.as_str()))
            .await?;
        let start = match again {
            Some(again) => again.start.clone(),
            None if matches!(step.kind, StepKind::Deploy { .. }) => {
                deploy_step::member_start(wr, step)
            }
            None => start_branch(wr, &step.id),
        };
        let mut new = NewRun::new(member, TriggerKind::Workflow);
        new.workflow_step_run_id = Some(step_run.id.clone());
        new.payload = Some(json!({
            "workflow": {
                "workflow_run_id": wr.id,
                "step_id": step.id,
                "start": start,
                "section": section,
            }
        }));
        self.runner.enqueue(new).await
    }

    /// How a prompt of a run of `member` names its place: the team's name, who the member is
    /// in it (`the <role>`, else `a member`) and the workflow's name.
    async fn names(&self, wr: &WorkflowRun, member: &str) -> Result<(String, String, String)> {
        let team = self.store.get_team(&wr.team_id).await?;
        let you = team
            .members
            .iter()
            .find(|m| m.dot_id == member)
            .map(|m| m.role.trim().to_string())
            .filter(|r| !r.is_empty())
            .map_or("a member".to_string(), |r| format!("the {r}"));
        let workflow = match &wr.workflow_id {
            Some(id) => self.store.get_workflow(id).await.ok().map(|w| w.spec.name),
            None => None,
        }
        .unwrap_or_else(|| "this workflow".to_string());
        Ok((team.name, you, workflow))
    }

    /// The `## Workflow` section of the prompt of `step` (section 5.2). Its place counts every
    /// step of the tree in run order, branches and the paths not taken included. It names the
    /// workflow tools the step's runs have (section 7), as the member's model knows them, unless
    /// its policy denies `Workflow` outright (the runner then offers none). `sent` is what a
    /// step that went back and starts afresh is told about the send-back.
    async fn section(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
        sent: Option<&str>,
    ) -> Result<String> {
        let Some((member, job)) = deploy_step::worker(wr, step) else {
            return Ok(String::new());
        };
        let (team, you, workflow) = self.names(wr, member).await?;
        let mut out = String::from("## Workflow\n\n");
        out.push_str(&format!(
            "You are {you} of the team \"{team}\", doing one step of the workflow \"{workflow}\".\n\n"
        ));
        out.push_str(&place(wr, step));
        out.push_str("### Task\n\n");
        out.push_str(wr.task.trim());
        out.push_str("\n\n### Your job in this step\n\n");
        out.push_str(job.trim());
        out.push_str("\n\n");
        // A member deploy is shown only what the check approved, each branch at its commit.
        if matches!(step.kind, StepKind::Deploy { .. }) {
            if let Ok((branches, summary)) = deploy_step::approved(wr, &step.id) {
                out.push_str("### Earlier steps\n\n#### Approved\n\n");
                let summary = summary.trim();
                out.push_str(if summary.is_empty() {
                    "(no summary)"
                } else {
                    summary
                });
                out.push_str("\n\n");
                if !branches.is_empty() {
                    let named: Vec<String> = branches
                        .iter()
                        .map(|b| format!("{} (commit {})", b.branch, b.commit))
                        .collect();
                    out.push_str(&format!("Branches: {}\n\n", named.join(", ")));
                }
            }
        }
        let earlier = if matches!(step.kind, StepKind::Deploy { .. }) {
            Vec::new()
        } else {
            handoffs_before(wr, &step.id)
        };
        if !earlier.is_empty() {
            out.push_str("### Earlier steps\n\n");
            for (s, h) in earlier {
                out.push_str(&format!("#### {}\n\n", s.name));
                let summary = h.summary.trim();
                out.push_str(if summary.is_empty() {
                    "(no summary)"
                } else {
                    summary
                });
                out.push_str("\n\n");
                if !h.branches.is_empty() {
                    let names: Vec<&str> = h.branches.iter().map(|b| b.branch.as_str()).collect();
                    out.push_str(&format!("Branches: {}\n\n", names.join(", ")));
                }
            }
        }
        let dot = self.store.get_dot(member).await.ok().filter(|dot| {
            dot.spec
                .policy
                .resolve(crate::policy::WORKFLOW_TOOL, &serde_json::Value::Null)
                != crate::policy::Action::Deny
        });
        let tools = match dot {
            Some(dot) => StepTools::read(&self.store, &step_run.id, step, &wr.definition)
                .await?
                .map(|tools| tools.prompt_lines(dot.spec.engine))
                .unwrap_or_default(),
            None => String::new(),
        };
        if !tools.is_empty() {
            out.push_str("### Workflow tools\n\n");
            out.push_str(&tools);
            out.push('\n');
        }
        if let Some(sent) = sent {
            out.push_str("### Sent back\n\n");
            out.push_str(sent.trim());
            out.push_str("\n\n");
        }
        out.push_str(
            "When you are done, end with a summary of what you did: the next step gets it.\n",
        );
        Ok(out)
    }

    /// Watches an agent step's runs: it ends when its own thread has ended and no question of
    /// it waits for the person, by the job that builds its hand-off (unless one is in flight).
    /// A delegate step also waits for the threads of the members it started, and continues its
    /// own thread with their results (`delegate`). An own thread that failed fails the step,
    /// and cancels the members still running. A step whose run is gone gets one again, as a
    /// step that begins does: unless its member may not run, which fails it, or a limit is
    /// reached, which waits for the person.
    async fn watch_agent(&self, wr: &WorkflowRun, step: &Step, step_run: &StepRun) -> Result<()> {
        // The question first: an answer closes it and stores the run that continues the thread
        // in one transaction, so once it reads as closed, the runs read next include that run.
        if self.store.step_has_open_question(&step_run.id).await? {
            return Ok(());
        }
        let runs = self.store.runs_of_step_run(&step_run.id).await?;
        let members = self.store.step_members(&step_run.id).await?;
        let own = delegate::own_runs(&runs, &members);
        let Some(last) = own.last().copied() else {
            return self.start_agent(wr, step, step_run).await;
        };
        if own.iter().any(|r| !r.status.is_terminal()) {
            return Ok(());
        }
        if last.status != RunStatus::Succeeded {
            let error = last.error.clone().unwrap_or_else(|| match last.status {
                RunStatus::Cancelled => "the run was cancelled".to_string(),
                _ => "the run failed".to_string(),
            });
            self.fail_step(wr, step, step_run, &error).await?;
            self.cancel_members(&runs, &members).await;
            return Ok(());
        }
        if !members.is_empty()
            && !self
                .members_done(wr, step, step_run, last, &members, &runs)
                .await?
        {
            return Ok(());
        }
        // The hand-off runs Dots' commit, up to two minutes: not under the lock.
        self.start_hand_off(wr, step_run, runs);
        Ok(())
    }

    /// Starts the first run of the agent step run `step_run`, which has none (`queue_first`),
    /// unless its member may not run, which fails it, or a limit is reached, which waits for
    /// the person.
    async fn start_agent(&self, wr: &WorkflowRun, step: &Step, step_run: &StepRun) -> Result<()> {
        if let Some((member, _)) = deploy_step::worker(wr, step) {
            if let Some(error) = self.member_refusal(member).await? {
                return self.fail_step(wr, step, step_run, &error).await;
            }
        }
        // A deploy ships what the check approved: a branch moved since fails it, whenever the
        // member's run is started (first, after a wait on the budget, after a restart).
        if matches!(step.kind, StepKind::Deploy { .. }) {
            if let Err(error) = self.check_approved(wr, step).await {
                return self.fail_step(wr, step, step_run, &error).await;
            }
        }
        if let Some(limit) = budget_reached(wr) {
            return self.wait_on_budget(wr, &limit).await;
        }
        self.queue_first(wr, step, step_run).await
    }

    /// Fails a step: the workflow run waits for the person to retry it or stop (section 8).
    async fn fail_step(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
        error: &str,
    ) -> Result<()> {
        self.fail_step_with(wr, step, step_run, error, None).await
    }

    /// `fail_step`, with what a deploy did before it failed, kept in the same write.
    async fn fail_step_with(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
        error: &str,
        deploy_result: Option<crate::deploy::DeployResult>,
    ) -> Result<()> {
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Failed),
                    error: Some(Some(error.to_string())),
                    deploy_result: deploy_result.map(Some),
                    ..Default::default()
                },
            )
            .await?;
        self.step_ended(wr, &step.id);
        let waiting = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Waiting),
                    waiting_on: Some(Some(WaitingOn::StepFailed)),
                    error: Some(Some(format!("{}: {error}", step.name))),
                    ..Default::default()
                },
            )
            .await?;
        self.updated(&waiting);
        Ok(())
    }

    /// What a finished agent step hands on (section 6): its own thread's newest summary, and
    /// the branch of that thread when the branch still exists (a worktree with no change was
    /// removed with its branch). What the thread left uncommitted is committed first. A
    /// delegate step also hands on what each member it started handed back, and its branches
    /// are those of section 6.2 (`delegate::handed_branches`): its own branch when it has
    /// commits, with each member's branch that exists and that it did not merge; else each
    /// member's branch that exists. A check after the step shows these, and a deploy after it
    /// ships them. The outcome is the caller's: the step run's. Slow, and run by a job without
    /// the lock.
    async fn handoff_of(&self, wr: &WorkflowRun, members: &[StepMember], runs: &[Run]) -> Handoff {
        let own = delegate::own_runs(runs, members);
        let summary = own
            .last()
            .and_then(|r| r.summary.clone())
            .unwrap_or_default();
        let repo = Path::new(&wr.project.repo_path);
        let mut lead = None;
        if let Some(run) = own.iter().rev().find(|r| r.branch.is_some()) {
            // Dots' commit, should the run's end not have made it (section 6.1): the next step
            // starts from this branch's tip.
            self.runner.commit_step_work(run).await;
            let branch = run.branch.clone().unwrap_or_default();
            if branch_exists(repo, &branch).await
                && (members.is_empty()
                    || has_commits(repo, &branch, run.base_commit.as_deref()).await)
            {
                lead = Some(branch_ref(run, branch));
            }
        }
        let mut handed = Vec::new();
        let mut member_branches = Vec::new();
        for member in members {
            let (end, branch) = self.member_end(wr, member, runs, true).await;
            handed.push(end);
            member_branches.extend(branch);
        }
        let branches = delegate::handed_branches(repo, lead, member_branches).await;
        Handoff {
            summary: cut(&summary, MAX_HANDOFF_SUMMARY),
            branches,
            outcome: None,
            members: handed,
        }
    }
}

/// Returns when something a pass should see happened: a run of a workflow step changed, a
/// question was posted or answered, an approval was asked or decided; or events were lost.
async fn relevant(events: &mut Receiver<RuntimeEvent>) {
    loop {
        match events.recv().await {
            Ok(RuntimeEvent::RunUpdated { run }) if run.workflow_step_run_id.is_some() => return,
            Ok(RuntimeEvent::MessagePosted { .. })
            | Ok(RuntimeEvent::MessageUpdated { .. })
            | Ok(RuntimeEvent::ApprovalRequested { .. })
            | Ok(RuntimeEvent::ApprovalDecided { .. }) => return,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return,
            Err(RecvError::Closed) => std::future::pending().await,
        }
    }
}
