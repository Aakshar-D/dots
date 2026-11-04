//! Deploy steps (teams spec sections 5.5 and 6.2): Dots ships exactly the branches the check
//! before the step approved, at the commits the person saw. A check's approval records each
//! branch it showed with its tip (`tips`); the deploy step reads them back (`approved`) and
//! deploys them as a job outside the loop's lock (`crate::deploy::Deployer`), since a push or
//! gh may take minutes. A test run, and mode `branch`, push nothing and list the branches as
//! ready. Mode `member` (in a real run) checks the tips the same way and then runs as an agent
//! step of that member with the step's job: its hand-off is the step's. A deploy is never run
//! twice without the person: one Dots stopped during fails `interrupted`, and Retry runs it
//! again.

use std::path::Path;

use serde_json::{json, Value};

use super::{ran_before, succeeded, Workflows};
use crate::deploy::{
    check_tips, ApprovedBranch, DeployFailure, DeployResult, Deployer, BRANCH_CHANGED,
};
use crate::model::{
    Handoff, ProjectSpec, StepRun, StepRunChange, StepRunStatus, WorkflowRun, WorkflowRunChange,
};
use crate::workflow::{DeployMode, Step, StepKind};
use crate::workspace::git;
use crate::{Error, Result};

/// Why a deploy step fails when the check before it recorded no branches it approved: a check
/// approved before Dots recorded them, or one that sent the work back.
pub const NO_APPROVAL: &str = "the check before it recorded no approved branches";
/// Why a `pr` or `pr_merge` deploy fails when the project's default branch cannot be
/// found (a `push` deploy needs none): the project names none, the remote's `HEAD` names none,
/// and the repo is on no branch.
pub const NO_DEFAULT_BRANCH: &str =
    "the project's default branch could not be found; set it on the project";
/// Why a deploy step Dots stopped during fails, as plan A words an interrupted run.
pub const INTERRUPTED: &str = "interrupted";

/// The member and job of a step a member works on: an agent step, or in a real run a deploy of
/// mode `member` (a test run's deploys push nothing and start no one: they act as `branch`).
pub(super) fn worker<'a>(wr: &WorkflowRun, step: &'a Step) -> Option<(&'a str, &'a str)> {
    match &step.kind {
        StepKind::Agent { member, job, .. } => Some((member, job)),
        StepKind::Deploy {
            mode: DeployMode::Member { member, job },
        } if !wr.test => Some((member, job)),
        _ => None,
    }
}

/// The branches a check's approval records (section 6.2): each branch of the hand-off it
/// showed, with the commit at its tip as the person approved, `null` for one already gone.
pub(super) async fn tips(repo: &Path, shown: &Handoff) -> Vec<Value> {
    let mut out = Vec::new();
    for b in &shown.branches {
        let commit = format!("refs/heads/{}^{{commit}}", b.branch);
        let args = [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ];
        let tip = git(repo, &args).await.ok();
        out.push(json!({"branch": b.branch, "commit": tip}));
    }
    out
}

/// What the deploy step `id` acts on: the branches the newest check that ran before it on its
/// path approved, at the commits it recorded, each with the commit its work started from (the
/// hand-off's `base_commit`, for the secret scan); and the summary that check showed, for a
/// pull request's body.
pub(super) fn approved(
    wr: &WorkflowRun,
    id: &str,
) -> std::result::Result<(Vec<ApprovedBranch>, String), String> {
    let check = ran_before(wr, id)
        .into_iter()
        .rev()
        .filter(|s| matches!(s.kind, StepKind::Check { .. }))
        .find_map(|s| succeeded(wr, &s.id))
        .ok_or_else(|| NO_APPROVAL.to_string())?;
    let shown = check.handoff.clone().unwrap_or_default();
    let recorded = check
        .decision
        .as_ref()
        .filter(|d| d.get("decision").and_then(Value::as_str) == Some("approved"))
        .and_then(|d| d.get("branches"))
        .and_then(Value::as_array)
        .ok_or_else(|| NO_APPROVAL.to_string())?;
    let mut out = Vec::new();
    for entry in recorded {
        let branch = entry
            .get("branch")
            .and_then(Value::as_str)
            .ok_or_else(|| NO_APPROVAL.to_string())?;
        // A branch that was gone when the person approved has no commit they saw.
        let commit = entry
            .get("commit")
            .and_then(Value::as_str)
            .ok_or_else(|| BRANCH_CHANGED.to_string())?;
        let base = shown
            .branches
            .iter()
            .find(|b| b.branch == branch)
            .and_then(|b| b.base_commit.clone());
        out.push(ApprovedBranch {
            branch: branch.to_string(),
            commit: commit.to_string(),
            base,
        });
    }
    Ok((out, shown.summary))
}

/// What the member of a real-run `member` deploy starts from: the one branch the check
/// approved, at the commit approved (never a later worker's hand-off). `None` when the check
/// approved none or several: the project's default branch.
pub(super) fn member_start(wr: &WorkflowRun, step: &Step) -> Option<String> {
    let (branches, _) = approved(wr, &step.id).ok()?;
    match branches.as_slice() {
        [only] => Some(only.commit.clone()),
        _ => None,
    }
}

/// The project's default branch by name, for a pull request's base and the scan of a branch
/// with no base commit: its `default_branch`, else as worktrees find it: the branch the remote's
/// `HEAD` names, else the branch the repo is on. `None` when none of them names one.
async fn base_branch(repo: &Path, project: &ProjectSpec) -> Option<String> {
    let set = project.default_branch.as_deref().map(str::trim);
    if let Some(branch) = set.filter(|b| !b.is_empty()) {
        return Some(branch.to_string());
    }
    let remote = project.remote.trim();
    let head = format!("refs/remotes/{remote}/HEAD");
    let named = git(repo, &["symbolic-ref", "--quiet", "--short", &head]).await;
    if let Some(branch) = named
        .ok()
        .and_then(|r| r.strip_prefix(&format!("{remote}/")).map(str::to_string))
        .filter(|b| !b.is_empty())
    {
        return Some(branch);
    }
    git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .ok()
        .filter(|b| !b.is_empty())
}

impl Workflows {
    /// Sets the deployer deploy steps use, once (by default `gh` from the PATH); a test gives a
    /// fake `gh` this way.
    pub fn set_deployer(&self, deployer: Deployer) {
        let _ = self.deployer.set(deployer);
    }

    fn deployer(&self) -> Deployer {
        self.deployer.get().cloned().unwrap_or_default()
    }

    /// Begins the deploy step `step` as its next attempt: a step run that stays `Running` while
    /// a job deploys. In a real run, a deploy of mode `member` checks that the approved branches
    /// are unchanged and then runs as an agent step of that member (`start_agent`).
    pub(super) async fn begin_deploy(&self, wr: &WorkflowRun, step: &Step) -> Result<()> {
        if !matches!(step.kind, StepKind::Deploy { .. }) {
            return Err(Error::Other(format!("step {} is not a deploy", step.id)));
        }
        let member = worker(wr, step).is_some();
        // A step run a begin cut off before the move below left running is used, not doubled.
        let step_run = self
            .step_run_to_begin(wr, step, StepRunStatus::Running)
            .await?;
        let wr = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    current_step: Some(Some(step.id.clone())),
                    ..Default::default()
                },
            )
            .await?;
        if !member {
            // In the locked section that made the step run: no pass finds it without its job.
            self.start_deploy(&wr, &step_run);
            self.updated(&wr);
            return Ok(());
        }
        // The approved tips are checked in `start_agent`, which every start of the member's
        // run goes through (this one, a wait on the budget, a restart).
        self.start_agent(&wr, step, &step_run).await?;
        self.updated(&self.store.get_workflow_run(&wr.id).await?);
        Ok(())
    }

    /// For a deploy step a member works on: whether the branches its check approved are still at
    /// the commits approved (`Err` with the words to fail the step with, if not).
    pub(super) async fn check_approved(
        &self,
        wr: &WorkflowRun,
        step: &Step,
    ) -> std::result::Result<(), String> {
        let (branches, _) = approved(wr, &step.id)?;
        check_tips(Path::new(&wr.project.repo_path), &branches).await
    }

    /// Watches a deploy step run that runs with no member: its job is in flight, or Dots
    /// stopped during it, which fails it `interrupted`. A push may have gone through, so it
    /// is not run again without the person: Retry runs it again.
    pub(super) async fn watch_deploy(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
    ) -> Result<()> {
        if self.has_job(&step_run.id) {
            return Ok(());
        }
        self.fail_step(wr, step, step_run, INTERRUPTED).await
    }

    /// Starts the job that deploys the step run `step_run` (`deploy`).
    fn start_deploy(&self, wr: &WorkflowRun, step_run: &StepRun) {
        let id = wr.id.clone();
        let step_run_id = step_run.id.clone();
        self.start_job(&step_run.id, move |this| async move {
            this.deploy(id, step_run_id).await
        });
    }

    /// The deploy job (`start_deploy`): reads what to deploy under the lock (a Stop while the
    /// job waited wins, and nothing is pushed), deploys unlocked, then, if the step run is still
    /// open, succeeds it with what was done and moves on, or fails it with the deploy's error:
    /// the run waits for the person to retry it or stop.
    async fn deploy(&self, id: String, step_run_id: String) -> Result<()> {
        if !self.through_gate().await {
            return Ok(());
        }
        let wr = {
            let _one = self.lock.lock().await;
            match self.still_open(&id, &step_run_id).await? {
                Some((wr, _)) => wr,
                None => return Ok(()),
            }
        };
        let step_id = wr
            .steps
            .iter()
            .find(|s| s.id == step_run_id)
            .map(|s| s.step_id.clone())
            .unwrap_or_default();
        let done = self.deploy_now(&wr, &step_id).await;
        let _one = self.lock.lock().await;
        let Some((wr, step_run)) = self.still_open(&id, &step_run_id).await? else {
            return Ok(());
        };
        let Some(step) = wr.definition.step(&step_run.step_id) else {
            return Err(Error::Other(format!(
                "step {} is not in the workflow",
                step_run.step_id
            )));
        };
        match done {
            Ok(result) => {
                self.store
                    .set_step_run(
                        &step_run.id,
                        &StepRunChange {
                            status: Some(StepRunStatus::Succeeded),
                            deploy_result: Some(Some(result)),
                            ..Default::default()
                        },
                    )
                    .await?;
                self.step_ended(&wr, &step.id);
                let wr = self.store.get_workflow_run(&wr.id).await?;
                self.move_on(&wr, step).await
            }
            // What it did before the error is kept with it, in the write that fails the step:
            // a branch already pushed, a pull request already open.
            Err((error, done)) => {
                let result = DeployResult {
                    error: Some(error.clone()),
                    ..done
                };
                self.fail_step_with(&wr, step, &step_run, &error, Some(result))
                    .await
            }
        }
    }

    /// Deploys the step `step_id` of `wr` as its mode says (`branch` in a test run), on the
    /// project's repo and remote, titled with the task and with the approved summary as the
    /// body. `Err` is the error with what was done before it (no branches when the approved
    /// ones cannot be read). Slow: run without the lock.
    async fn deploy_now(
        &self,
        wr: &WorkflowRun,
        step_id: &str,
    ) -> std::result::Result<DeployResult, DeployFailure> {
        let unread = |error: String| (error, DeployResult::not_pushed(&[]));
        let mode = match wr.definition.step(step_id).map(|s| &s.kind) {
            Some(StepKind::Deploy { .. }) if wr.test => DeployMode::Branch,
            Some(StepKind::Deploy { mode }) => mode.clone(),
            _ => return Err(unread(format!("step {step_id} is not a deploy"))),
        };
        let (branches, summary) = approved(wr, step_id).map_err(unread)?;
        let repo = Path::new(&wr.project.repo_path);
        let base = match mode {
            DeployMode::Pr | DeployMode::PrMerge { .. } => {
                match base_branch(repo, &wr.project).await {
                    Some(base) => base,
                    None => {
                        let none = DeployResult::not_pushed(&branches);
                        return Err((NO_DEFAULT_BRANCH.to_string(), none));
                    }
                }
            }
            _ => String::new(),
        };
        self.deployer()
            .deploy(
                repo,
                &wr.project.remote,
                &base,
                &mode,
                &branches,
                &wr.task,
                &summary,
            )
            .await
    }
}
