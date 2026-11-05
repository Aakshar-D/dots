//! Delegate steps (teams spec section 5.3): the step's own run starts other members of the team
//! with `start_member` (`tools`), each in a thread of its own that works for the same step run.
//! A member's thread is the one whose root run has a `workflow_step_members` row; the step's
//! own thread is the rest of its runs. The step ends when its own thread and every member's
//! have ended and every member has been reported: once the own thread has ended well and the
//! members it is waiting for have all ended, Dots continues the own thread with their results,
//! as an answer to `ask` continues a thread.

use std::path::Path;

use super::{
    branch_exists, branch_ref, budget_reached, cut, merged_into, place, Workflows,
    MAX_HANDOFF_SUMMARY,
};
use crate::model::{
    BranchRef, EngineKind, MemberHandoff, Run, RunStatus, StepMember, StepRun, WorkflowRun,
};
use crate::team_tools::Tool;
use crate::workflow::{Step, StepKind};
use crate::{Error, Result};

/// The most characters of a member's task the delegating run is reminded of, per member.
const MAX_TASK_LINE: usize = 80;

/// The runs of the step's own thread among `runs`, oldest first: those in no member's thread.
pub(super) fn own_runs<'a>(runs: &'a [Run], members: &[StepMember]) -> Vec<&'a Run> {
    runs.iter()
        .filter(|r| !members.iter().any(|m| m.run_id == r.root_run_id))
        .collect()
}

/// The runs of `member`'s thread among `runs`, oldest first.
fn thread_of<'a>(runs: &'a [Run], member: &StepMember) -> Vec<&'a Run> {
    runs.iter()
        .filter(|r| r.root_run_id == member.run_id)
        .collect()
}

/// The branches a step hands on, so what a check after it shows and a deploy after it ships
/// (section 6.2, with the person's ancestor rule of 2026-11-04). `lead` is the step's own
/// branch, for a delegate step only when it has commits; `members` are the branches of the
/// members it started that still exist, in the order started. With a lead branch: that branch,
/// then each member branch whose tip is not an ancestor of the lead's tip (work the lead did not
/// merge). Without one: every member branch. A step with no members hands on its own branch.
pub(super) async fn handed_branches(
    repo: &Path,
    lead: Option<BranchRef>,
    members: Vec<BranchRef>,
) -> Vec<BranchRef> {
    let Some(lead) = lead else {
        return members;
    };
    let mut out = vec![lead];
    for member in members {
        if !merged_into(repo, &member.branch, &out[0].branch).await {
            out.push(member);
        }
    }
    out
}

/// Whether the thread of `member` has a run that has not ended (queued ones included).
pub(super) fn member_running(runs: &[Run], member: &StepMember) -> bool {
    thread_of(runs, member)
        .iter()
        .any(|r| !r.status.is_terminal())
}

impl Workflows {
    /// The `## Workflow` section of the prompt of a member that the delegating run of `step`
    /// started on `task` (section 5.3): the team and its role, the workflow's task, its part
    /// of it, the project, and who started it. It names no workflow tools: a member has none.
    pub(super) async fn member_section(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        member: &str,
        task: &str,
    ) -> Result<String> {
        let (team, you, workflow) = self.names(wr, member).await?;
        let starter = match &step.kind {
            StepKind::Agent { member, .. } => self.store.get_dot(member).await.ok(),
            _ => None,
        }
        .map_or_else(|| "a member".to_string(), |d| d.spec.name);
        let mut out = String::from("## Workflow\n\n");
        out.push_str(&format!(
            "You are {you} of the team \"{team}\". {starter} started you on part of one step of \
             the workflow \"{workflow}\".\n\n"
        ));
        out.push_str(&place(wr, step));
        out.push_str("### Task\n\n");
        out.push_str(wr.task.trim());
        out.push_str("\n\n### Your part\n\n");
        out.push_str(task.trim());
        out.push_str(&format!(
            "\n\nWhen you are done, end with a summary of what you did: {starter} gets it.\n"
        ));
        Ok(out)
    }

    /// Cancels the runs of the members' threads that have not ended: the delegating thread
    /// they work for failed. A cancel that fails is logged.
    pub(super) async fn cancel_members(&self, runs: &[Run], members: &[StepMember]) {
        for member in members {
            for run in thread_of(runs, member) {
                if run.status.is_terminal() {
                    continue;
                }
                if let Err(e) = self.runner.cancel(&run.id).await {
                    tracing::warn!(run = %run.id, "cancelling a member's run failed: {e}");
                }
            }
        }
    }

    /// For a delegate step whose own thread has ended with `last` succeeded: whether its
    /// members are done, every one ended and reported, so the step may end. Until then it
    /// waits while a member runs; once none runs, it continues the own thread with the results
    /// of those not reported yet, or past a limit waits on budget (section 9) and continues it
    /// when the person adds some.
    pub(super) async fn members_done(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
        last: &Run,
        members: &[StepMember],
        runs: &[Run],
    ) -> Result<bool> {
        if members.iter().any(|m| member_running(runs, m)) {
            return Ok(false);
        }
        let unreported: Vec<&StepMember> = members.iter().filter(|m| !m.reported).collect();
        if unreported.is_empty() {
            return Ok(true);
        }
        if let Some(limit) = budget_reached(wr) {
            self.wait_on_budget(wr, &limit).await?;
            return Ok(false);
        }
        let mut message = String::from("The members you started have finished:\n\n");
        for member in &unreported {
            let (end, _) = self.member_end(wr, member, runs, false).await;
            let name = match self.store.get_dot(&member.dot_id).await {
                Ok(dot) => dot.spec.name,
                Err(_) => member.dot_id.clone(),
            };
            let first = member.task.lines().next().unwrap_or_default().trim();
            message.push_str(&format!(
                "#### {name}: {}\n\nStatus: {}\nBranch: {}\n\n{}\n\n",
                cut(first, MAX_TASK_LINE),
                end.status.as_str(),
                end.branch.as_deref().unwrap_or("none"),
                end.summary
            ));
        }
        let total = match &step.kind {
            StepKind::Agent {
                delegate: Some(d), ..
            } => d.total,
            _ => 0,
        };
        // Counted over every attempt of the step, as `start_member` counts them.
        let started = self.store.step_members_started(&step_run.id).await?;
        let left = total.saturating_sub(started);
        // A lead that is no longer a dot fails its continued run, and with it the step.
        let engine = match self.store.get_dot(&last.dot_id).await {
            Ok(dot) => dot.spec.engine,
            Err(Error::NotFound(_)) => EngineKind::Local,
            Err(e) => return Err(e),
        };
        let tool = Tool::StartMember.model_name(engine);
        message.push_str(&if left > 0 {
            format!(
                "You may start {left} more with `{tool}`, or end with your summary of the step."
            )
        } else {
            "You may start no more members: end with your summary of the step.".to_string()
        });
        // The own thread goes on where it ended: the same session, worktree and branch. It
        // names the step run, as a run whose step has not ended does.
        let mut new = crate::checkins::continuation(last, last, message, None);
        new.workflow_step_run_id = Some(step_run.id.clone());
        let reported: Vec<String> = unreported.iter().map(|m| m.run_id.clone()).collect();
        let run = self.store.report_step_members(&new, &reported).await?;
        self.runner.enqueued(&run);
        self.updated(&self.store.get_workflow_run(&wr.id).await?);
        Ok(false)
    }

    /// What `member`'s thread, which has ended, hands back (section 6): how its newest run
    /// ended, its summary (else its error) cut to the hand-off limit, and its branch while that
    /// exists, with the branch's reference for the step's hand-off. With `commit`, what its run
    /// left uncommitted is committed first, as for the step's own thread (section 6.1); the
    /// run's end has done it already unless Dots stopped in between.
    pub(super) async fn member_end(
        &self,
        wr: &WorkflowRun,
        member: &StepMember,
        runs: &[Run],
        commit: bool,
    ) -> (MemberHandoff, Option<BranchRef>) {
        let thread = thread_of(runs, member);
        let last = thread.last();
        let status = match last.map(|r| r.status) {
            Some(RunStatus::Succeeded) => RunStatus::Succeeded,
            Some(RunStatus::Cancelled) => RunStatus::Cancelled,
            _ => RunStatus::Failed,
        };
        let text = last
            .and_then(|r| r.summary.clone())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| last.and_then(|r| r.error.clone()))
            .unwrap_or_else(|| "(no summary)".to_string());
        let mut branch = None;
        if let Some(run) = thread.iter().rev().find(|r| r.branch.is_some()) {
            if commit {
                self.runner.commit_step_work(run).await;
            }
            let name = run.branch.clone().unwrap_or_default();
            if branch_exists(Path::new(&wr.project.repo_path), &name).await {
                branch = Some(branch_ref(run, name));
            }
        }
        let end = MemberHandoff {
            dot_id: member.dot_id.clone(),
            task: member.task.clone(),
            status,
            summary: cut(&text, MAX_HANDOFF_SUMMARY),
            branch: branch.as_ref().map(|b| b.branch.clone()),
        };
        (end, branch)
    }
}
