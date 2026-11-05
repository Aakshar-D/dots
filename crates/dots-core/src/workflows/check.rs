//! What a waiting check shows the person (teams spec sections 5.4 and 6.2), and the diff of a
//! branch it shows. The check page reads `check` before the person decides: the branches of the
//! hand-off the check was shown on (`shown_at`, which already holds the ancestor rule), each
//! with the commit at its tip. Approve sends those tips back (`CheckTip`), and the approval is
//! refused when git no longer agrees with them (`changed`), so the person never approves work
//! they did not see. `branch_diff` shows a branch's committed work from the project's default
//! branch, for a branch the run's check shows or its deploy names only: the page never names
//! an arbitrary ref.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::deploy_step::{base_branch, tip};
use super::{latest, shown_at, Workflows, NO_DEFAULT_BRANCH};
use crate::model::{StepRun, WaitingOn, WorkflowRun, WorkflowRunStatus, WorktreeDiff};
use crate::workspace::{git, range_diff};
use crate::{Error, Result};

/// A branch a waiting check shows: the run whose branch it is, and the commit at its tip now
/// (`None` for a branch that is gone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownBranch {
    pub branch: String,
    pub run_id: String,
    pub commit: Option<String>,
}

/// What the check a workflow run waits on shows (section 5.4): the check step, the summary of
/// the hand-off it was shown on, and that hand-off's branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckView {
    pub step_id: String,
    pub summary: String,
    pub branches: Vec<ShownBranch>,
}

/// A branch as the check page showed it, with the commit it showed (`None` for a branch that
/// was gone): what an approval names, and what it records (section 6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckTip {
    pub branch: String,
    pub commit: Option<String>,
}

impl CheckView {
    /// The tips this view shows, as an approval of it names them.
    pub fn tips(&self) -> Vec<CheckTip> {
        self.branches
            .iter()
            .map(|b| CheckTip {
                branch: b.branch.clone(),
                commit: b.commit.clone(),
            })
            .collect()
    }
}

/// Why an approval is refused when what the page showed is not what git has now: `names` are
/// the branches that differ.
pub fn branches_changed(names: &[String]) -> String {
    format!(
        "The branches changed since the check was shown: {}. Look at the check again.",
        names.join(", ")
    )
}

/// The branches whose tips differ between `now` (git's, in the order the check shows them) and
/// `seen` (the page's): a branch shown with another commit or not sent at all, then a branch
/// sent that the check does not show or sent twice. Empty when the page saw exactly `now`.
pub(super) fn changed(now: &[CheckTip], seen: &[CheckTip]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tip in now {
        let same = seen.iter().find(|s| s.branch == tip.branch);
        if same.is_none_or(|s| s.commit != tip.commit) {
            out.push(tip.branch.clone());
        }
    }
    for s in seen {
        let shown = now.iter().any(|t| t.branch == s.branch);
        let twice = seen.iter().filter(|o| o.branch == s.branch).count() > 1;
        if (!shown || twice) && !out.contains(&s.branch) {
            out.push(s.branch.clone());
        }
    }
    out
}

/// The step run of the check the workflow run waits on, if it waits on one.
pub(super) fn waiting_check(wr: &WorkflowRun) -> Option<&StepRun> {
    match (&wr.status, &wr.waiting_on, &wr.current_step) {
        (WorkflowRunStatus::Waiting, Some(WaitingOn::Check), Some(current)) => latest(wr, current),
        _ => None,
    }
}

/// The branches with the commits the workflow run recorded for them: each approval's tips and
/// each deploy's branches.
fn recorded(wr: &WorkflowRun) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in &wr.steps {
        let approved = s
            .decision
            .as_ref()
            .filter(|d| d.get("decision").and_then(Value::as_str) == Some("approved"))
            .and_then(|d| d.get("branches"))
            .and_then(Value::as_array);
        for entry in approved.into_iter().flatten() {
            let branch = entry.get("branch").and_then(Value::as_str);
            let commit = entry.get("commit").and_then(Value::as_str);
            if let (Some(branch), Some(commit)) = (branch, commit) {
                out.push((branch.to_string(), commit.to_string()));
            }
        }
        for b in s.deploy_result.iter().flat_map(|r| &r.branches) {
            out.push((b.branch.clone(), b.commit.clone()));
        }
    }
    out
}

/// The commit at the tip of the project's default branch (`base_branch`, as deploys find it):
/// the local branch of that name, else the remote's. `None` when there is neither.
async fn default_tip(repo: &Path, wr: &WorkflowRun) -> Option<String> {
    let name = base_branch(repo, &wr.project).await?;
    let remote = format!("refs/remotes/{}/{name}", wr.project.remote.trim());
    for full in [format!("refs/heads/{name}"), remote] {
        let commit = format!("{full}^{{commit}}");
        let args = [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ];
        if let Ok(tip) = git(repo, &args).await {
            return Some(tip);
        }
    }
    None
}

impl Workflows {
    /// What the check the workflow run `id` waits on shows (sections 5.4 and 6.2), each branch
    /// with its tip as git has it now. Read without the loop's lock: it changes nothing.
    /// `Conflict` when the run is not waiting on a check.
    pub async fn check(&self, id: &str) -> Result<CheckView> {
        let wr = self.store.get_workflow_run(id).await?;
        let Some(step_run) = waiting_check(&wr) else {
            return Err(Error::Conflict(format!(
                "workflow run {id} is not waiting for a check"
            )));
        };
        let shown = shown_at(&wr, &step_run.step_id);
        let repo = Path::new(&wr.project.repo_path);
        let mut branches = Vec::new();
        for b in &shown.branches {
            branches.push(ShownBranch {
                branch: b.branch.clone(),
                run_id: b.run_id.clone(),
                commit: tip(repo, &b.branch).await,
            });
        }
        Ok(CheckView {
            step_id: step_run.step_id.clone(),
            summary: shown.summary,
            branches,
        })
    }

    /// The committed changes of `branch` at `commit` from where it left the project's default
    /// branch (`<merge-base>..<commit>`, spec section 5.4), capped as a worktree's diff is
    /// (`workspace::MAX_DIFF_BYTES`). Only a branch the workflow run shows may be diffed: one
    /// its waiting check shows, at its tip now, or one an approval or a deploy of it recorded,
    /// at the commit recorded. `Invalid` for a branch it does not show, `Conflict` for a branch
    /// it shows at another commit, and when the default branch cannot be found.
    pub async fn branch_diff(&self, id: &str, branch: &str, commit: &str) -> Result<WorktreeDiff> {
        let wr = self.store.get_workflow_run(id).await?;
        let repo = Path::new(&wr.project.repo_path);
        let mut named = false;
        let mut at = false;
        if let Some(step_run) = waiting_check(&wr) {
            let shown = shown_at(&wr, &step_run.step_id);
            if shown.branches.iter().any(|b| b.branch == branch) {
                named = true;
                at |= tip(repo, branch).await.as_deref() == Some(commit);
            }
        }
        for (b, c) in recorded(&wr) {
            if b == branch {
                named = true;
                at |= c == commit;
            }
        }
        if !named {
            return Err(Error::Invalid(format!(
                "workflow run {id} shows no branch {branch}"
            )));
        }
        if !at {
            return Err(Error::Conflict(branches_changed(&[branch.to_string()])));
        }
        // From here on `commit` is a commit git or the run recorded, never a ref the page made.
        let Some(default) = default_tip(repo, &wr).await else {
            return Err(Error::Conflict(NO_DEFAULT_BRANCH.into()));
        };
        let base = git(repo, &["merge-base", "--end-of-options", &default, commit]).await?;
        range_diff(repo, &base, commit).await
    }
}
