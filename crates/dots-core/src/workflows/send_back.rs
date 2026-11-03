//! Send-backs (teams spec sections 5.4 and 9): work goes back to an earlier agent step, from
//! the person at a check (`send_back_check`) or from an agent step that asked for it with
//! `send_back` (performed once that step has succeeded). The target gets a new step run whose
//! first run continues the last run of its earlier one, as an answer to `ask` does (the same
//! session, worktree and branch), with the note. The steps after it then run again in order:
//! an agent step is continued in the same way with the newest hand-off before it, a check waits
//! again and a branch chooses again. Past `max_send_backs` nothing is performed: the workflow
//! run waits until the person allows one more, or stops it.
//!
//! The writes are ordered so a pass finishes a send-back cut off half way (Dots stopped):
//! a sender that is still the current step with its step run succeeded has not performed the
//! send-back it asks for (`move_on` performs it); the target's new step run is made before the
//! workflow run moves to it, and a pass that performs again reuses it; and the first run of a
//! step run that has none is the one `queue_first` makes, whoever starts it.

use std::path::Path;

use serde_json::json;

use super::delegate::own_runs;
use super::{
    branch_exists, handoffs_before, latest, next_attempt, next_of, ran_before, start_branch,
    Workflows, MAX_TASK,
};
use crate::accounts::Permission;
use crate::model::{
    SendBack, SentBy, StepRun, StepRunChange, StepRunStatus, WaitingOn, WorkflowRun,
    WorkflowRunChange, WorkflowRunStatus,
};
use crate::team_tools::quoted;
use crate::workflow::{Step, StepKind};
use crate::workspace::git;
use crate::{Error, Result};

/// Why a step that went back started afresh, after where it started from.
const NOT_RESUMED: &str = "the earlier session could not be resumed.";

/// How a fresh run of a step that went back starts, when its earlier session cannot be
/// resumed: from `start` (a full ref; `None` as the step starts the first time), with `message`
/// in its `## Workflow` section.
pub(super) struct Again {
    pub(super) start: Option<String>,
    pub(super) message: String,
}

fn is_agent(step: &&Step) -> bool {
    matches!(step.kind, StepKind::Agent { .. })
}

/// The agent step a send-back from the check `check` goes to: its `send_back_to`, else the
/// nearest agent step that ran before it on its path, whose hand-off the check showed.
pub(super) fn check_target<'a>(wr: &'a WorkflowRun, check: &Step) -> Option<&'a Step> {
    let StepKind::Check { send_back_to } = &check.kind else {
        return None;
    };
    match send_back_to {
        Some(id) => wr.definition.step(id).filter(is_agent),
        None => ran_before(wr, &check.id).into_iter().rev().find(is_agent),
    }
}

/// The send-back the succeeded step run `step_run` of `step` asks for, if any: a check's
/// decision to send the work back, or what its agent asked for with `send_back`
/// (`{"to", "note"}`). While `step` is still the current step it has not been performed.
fn asked(wr: &WorkflowRun, step: &Step, step_run: &StepRun) -> Option<SendBack> {
    let (to, note, by) = match &step.kind {
        StepKind::Check { .. } => {
            let decision = step_run.decision.as_ref()?;
            if decision.get("decision")?.as_str()? != "sent_back" {
                return None;
            }
            let note = decision.get("note")?.as_str()?;
            (check_target(wr, step)?.id.clone(), note, SentBy::Person)
        }
        StepKind::Agent { .. } => {
            let asked = step_run.send_back.as_ref()?;
            let to = asked.get("to")?.as_str()?.to_string();
            (to, asked.get("note")?.as_str()?, SentBy::Agent)
        }
        _ => return None,
    };
    Some(SendBack {
        from: step.id.clone(),
        to,
        note: note.to_string(),
        by,
        held: false,
    })
}

/// What the target of `send` is told (section 5.4): who sent the work back, and the note.
fn sent_message(wr: &WorkflowRun, send: &SendBack) -> String {
    match send.by {
        SentBy::Person => format!("The person sent this back: {}", send.note),
        SentBy::Agent => {
            let from = wr
                .definition
                .step(&send.from)
                .map_or(send.from.as_str(), |s| s.name.as_str());
            format!("The step \"{from}\" sent this back: {}", send.note)
        }
    }
}

/// What a step after the target of the workflow run's last send-back is told when it runs
/// again: the newest hand-off before it, with its branches. `kept`: the step's own branch
/// still has its earlier commits, so it is told to merge the new work into it.
fn rerun_message(wr: &WorkflowRun, step: &Step, kept: bool) -> String {
    let target = wr
        .send_back
        .as_ref()
        .and_then(|s| wr.definition.step(&s.to))
        .map_or("an earlier step", |s| s.name.as_str());
    let newest = handoffs_before(wr, &step.id).last().map(|(_, h)| *h);
    let summary = newest
        .map(|h| h.summary.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("(no summary)");
    let branches: Vec<&str> = newest
        .map(|h| h.branches.iter().map(|b| b.branch.as_str()).collect())
        .unwrap_or_default();
    let mut out =
        format!("Work was sent back to \"{target}\" and has run again. Its new hand-off:\n\n");
    out.push_str(summary);
    out.push_str("\n\n");
    if !branches.is_empty() {
        out.push_str(&format!("Branches: {}\n\n", branches.join(", ")));
        if kept {
            out.push_str(&format!(
                "Your branch still has your earlier commits: merge {} into it before you go \
                 on.\n\n",
                quoted(branches.iter().copied())
            ));
        }
    }
    out.push_str("Update your work to match, then end with a new summary.");
    out
}

impl Workflows {
    /// Sends the work the check the workflow run waits on was shown back (section 5.4), with
    /// the person's note of 1 to 8,000 characters: the check's step run succeeds with the
    /// decision, and the work goes to the check's `send_back_to`, else to the nearest agent
    /// step before it, within the cap of send-backs (section 9). Refused for a run not waiting
    /// on a check, and for a check with no agent step before it.
    pub async fn send_back_check(&self, id: &str, note: &str) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.store.get_workflow_run(id).await?;
        self.require(&wr.team_id, Permission::AnswerApprovals)
            .await?;
        let wr = self.tick(&wr).await?;
        let step_run = match (&wr.status, &wr.waiting_on, &wr.current_step) {
            (WorkflowRunStatus::Waiting, Some(WaitingOn::Check), Some(current)) => {
                latest(&wr, current).cloned()
            }
            _ => None,
        };
        let check = step_run
            .as_ref()
            .and_then(|s| wr.definition.step(&s.step_id));
        let (Some(step_run), Some(check)) = (step_run.as_ref(), check) else {
            return Err(Error::Conflict(format!(
                "workflow run {id} is not waiting for a check"
            )));
        };
        let note = note.trim();
        if note.is_empty() || note.chars().count() > MAX_TASK {
            return Err(Error::Invalid(
                "the note must be 1 to 8,000 characters".into(),
            ));
        }
        if check_target(&wr, check).is_none() {
            return Err(Error::Invalid(format!(
                "the check \"{}\" has no agent step before it to send the work back to",
                check.name
            )));
        }
        let shown = handoffs_before(&wr, &check.id)
            .last()
            .map(|(_, h)| (*h).clone())
            .unwrap_or_default();
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Succeeded),
                    handoff: Some(Some(shown)),
                    decision: Some(Some(json!({"decision": "sent_back", "note": note}))),
                    ..Default::default()
                },
            )
            .await?;
        let wr = self
            .store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    waiting_on: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        self.move_on(&wr, check).await?;
        let wr = self.store.get_workflow_run(id).await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Performs the send-back the workflow run holds at its cap (section 9): raises its
    /// `max_send_backs` by one, once, and sends the work back.
    pub async fn allow_send_back(&self, id: &str) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.store.get_workflow_run(id).await?;
        self.require(&wr.team_id, Permission::RunDots).await?;
        let wr = self.tick(&wr).await?;
        if !(wr.status == WorkflowRunStatus::Waiting && wr.waiting_on == Some(WaitingOn::SendBacks))
        {
            return Err(Error::Conflict(format!(
                "workflow run {id} is not waiting at its send-back limit"
            )));
        }
        let mut budget = wr.budget;
        budget.max_send_backs = budget.max_send_backs.saturating_add(1);
        self.store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    waiting_on: Some(None),
                    budget: Some(budget),
                    error: Some(None),
                    ..Default::default()
                },
            )
            .await?;
        // The sender is still the current step with its send-back not performed: the move on
        // from it performs it, now within the cap.
        self.advance(id).await?;
        let wr = self.store.get_workflow_run(id).await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Moves the workflow run on from `step`, its current step, whose newest step run has
    /// succeeded: performs the send-back that step run asks for, else begins the step after
    /// it (past the last, the workflow run has succeeded).
    pub(super) async fn move_on(&self, wr: &WorkflowRun, step: &Step) -> Result<()> {
        let send = latest(wr, &step.id)
            .filter(|s| s.status == StepRunStatus::Succeeded)
            .and_then(|s| asked(wr, step, s));
        match send {
            Some(send) => self.perform_send_back(wr, send).await,
            None => self.begin_or_finish(wr, next_of(wr, &step.id)).await,
        }
    }

    /// Performs `send` (section 5.4) within the cap (section 9). At the cap the workflow run
    /// waits on `send_backs` with `send` held. Otherwise the target gets a new step run and
    /// becomes the current step as the send-back is counted, then its first run starts
    /// (`start_agent`: past a limit of the budget, it waits on budget instead).
    async fn perform_send_back(&self, wr: &WorkflowRun, send: SendBack) -> Result<()> {
        let max = wr.budget.max_send_backs;
        if wr.send_backs >= max {
            let waiting = self
                .store
                .set_workflow_run(
                    &wr.id,
                    &WorkflowRunChange {
                        status: Some(WorkflowRunStatus::Waiting),
                        waiting_on: Some(Some(WaitingOn::SendBacks)),
                        error: Some(Some(format!(
                            "Send-back limit reached: {} of {max}",
                            wr.send_backs
                        ))),
                        send_back: Some(Some(SendBack { held: true, ..send })),
                        ..Default::default()
                    },
                )
                .await?;
            self.updated(&waiting);
            return Ok(());
        }
        let Some(target) = wr.definition.step(&send.to).filter(is_agent) else {
            return Err(Error::Other(format!(
                "step {} is not an agent step of the workflow",
                send.to
            )));
        };
        // A target running while a later step is current is the step run of this send-back,
        // made by a move cut off before the workflow run moved to it: every step before the
        // current one has ended otherwise.
        let step_run = match latest(wr, &target.id) {
            Some(s) if s.status == StepRunStatus::Running => s.clone(),
            _ => {
                self.store
                    .create_step_run(
                        &wr.id,
                        &target.id,
                        next_attempt(wr, &target.id),
                        StepRunStatus::Running,
                    )
                    .await?
            }
        };
        // One write: the target becomes the current step as the send-back is counted, so a
        // pass never performs it twice.
        let wr = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    waiting_on: Some(None),
                    error: Some(None),
                    current_step: Some(Some(target.id.clone())),
                    send_backs: Some(wr.send_backs + 1),
                    send_back: Some(Some(send)),
                    ..Default::default()
                },
            )
            .await?;
        self.start_agent(&wr, target, &step_run).await?;
        self.updated(&self.store.get_workflow_run(&wr.id).await?);
        Ok(())
    }

    /// Queues the first run of the agent step run `step_run`. A step that succeeded earlier
    /// in this workflow run runs again only after a send-back: the last run of its newest
    /// succeeded step run's own thread is continued, with the same session, worktree and
    /// branch (for a delegate step, the delegating run). The target of the workflow run's last
    /// send-back is told the note; a step after it gets the newest hand-off before it, and a
    /// worktree it no longer has is made again from that hand-off's tip. A last run without a
    /// session starts afresh, from its old branch while that exists, and the step run's notice
    /// says so. Any other step starts afresh as usual (`queue_agent_run`).
    pub(super) async fn queue_first(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
    ) -> Result<()> {
        // A deploy's member never goes on in an old session or from a live branch: it starts
        // fresh from the commit the check approved (`deploy_step::member_start`).
        if matches!(step.kind, StepKind::Deploy { .. }) {
            self.queue_agent_run(wr, step, step_run, None).await?;
            return Ok(());
        }
        let earlier = wr.steps.iter().rev().find(|s| {
            s.step_id == step.id && s.id != step_run.id && s.status == StepRunStatus::Succeeded
        });
        let Some(earlier) = earlier else {
            self.queue_agent_run(wr, step, step_run, None).await?;
            return Ok(());
        };
        let runs = self.store.runs_of_step_run(&earlier.id).await?;
        let members = self.store.step_members(&earlier.id).await?;
        let last = own_runs(&runs, &members).last().copied();
        let repo = Path::new(&wr.project.repo_path);
        let branch = last.and_then(|r| r.branch.clone());
        let kept = match &branch {
            Some(b) => branch_exists(repo, b).await,
            None => false,
        };
        let target = wr.send_back.as_ref().filter(|s| !s.held && s.to == step.id);
        let message = match target {
            Some(send) => sent_message(wr, send),
            None => rerun_message(wr, step, kept),
        };
        match last.filter(|r| r.session_id.is_some()) {
            Some(last) => {
                let mut new = crate::checkins::continuation(last, last, message, None);
                new.workflow_step_run_id = Some(step_run.id.clone());
                // A step after the target whose branch is gone (it had no changes): the
                // runner makes its worktree again on its branch from the base commit, so the
                // base is the tip it starts from now, which holds the new work.
                if target.is_none() && branch.is_some() && !kept {
                    if let Some(start) = start_branch(wr, &step.id) {
                        let commit = format!("{start}^{{commit}}");
                        let args = ["rev-parse", "--verify", "--end-of-options", &commit];
                        if let Ok(tip) = git(repo, &args).await {
                            new.base_commit = Some(tip);
                        }
                    }
                }
                self.runner.enqueue(new).await?;
            }
            None => {
                let old = branch.filter(|_| kept);
                let (start, notice) = match &old {
                    Some(b) => (
                        Some(format!("refs/heads/{b}")),
                        format!("Started again from \"{b}\": {NOT_RESUMED}"),
                    ),
                    None => (
                        start_branch(wr, &step.id),
                        format!("Started again from the start: {NOT_RESUMED}"),
                    ),
                };
                self.store
                    .set_step_run(
                        &step_run.id,
                        &StepRunChange {
                            notice: Some(Some(notice)),
                            ..Default::default()
                        },
                    )
                    .await?;
                let again = Again { start, message };
                self.queue_agent_run(wr, step, step_run, Some(&again))
                    .await?;
            }
        }
        Ok(())
    }
}
