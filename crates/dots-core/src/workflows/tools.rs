//! The workflow tools of an agent step's run (teams spec section 7), as `Workflows` performs
//! them: `start_member` starts a member on part of a delegate step's work, `report_outcome`
//! records the step's outcome, `send_back` the send-back it asks for. Each call is made under
//! the loop's lock, so it cannot cross a step that ends: a call whose step has closed records
//! nothing. The last call of `report_outcome` and of `send_back` wins.

use serde_json::{json, Value};

use super::delegate::member_running;
use super::{budget_reached, start_branch, Workflows, MAX_TASK};
use crate::model::{NewRun, StepRunStatus, TriggerKind};
use crate::team_tools::{quoted, DelegateTools, StartableMember, StepTools, Tool};
use crate::{Error, Result};

/// Why a call of a step's tool records nothing once the step has ended (a Stop meanwhile).
pub const STEP_CLOSED: &str = "This step of the workflow has ended, so nothing was recorded.";

/// The trimmed text of the argument `key` of `input`; empty when it is missing.
fn arg(input: &Value, key: &str) -> String {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// The declared outcome `given` names: the one equal to it, else the only one equal to it
/// without case. A branch matches the declared label exactly, so that is what is stored.
fn outcome_of<'a>(outcomes: &'a [String], given: &str) -> Option<&'a str> {
    if let Some(o) = outcomes.iter().find(|o| *o == given) {
        return Some(o);
    }
    match outcomes
        .iter()
        .filter(|o| o.eq_ignore_ascii_case(given))
        .collect::<Vec<_>>()
        .as_slice()
    {
        [one] => Some(one),
        _ => None,
    }
}

impl Workflows {
    /// Whether the step run of `step` is still open; `Err` is the tool's answer. Called under
    /// the lock.
    async fn step_open(&self, run_id: &str, step: &StepTools) -> std::result::Result<(), String> {
        match self.store.get_step_run(&step.step_run_id).await {
            Ok(s) if matches!(s.status, StepRunStatus::Running | StepRunStatus::Waiting) => Ok(()),
            Ok(_) => Err(STEP_CLOSED.to_string()),
            Err(e) => {
                tracing::warn!(run = %run_id, "reading the step of a workflow tool failed: {e}");
                Err(format!("Not recorded: {e}."))
            }
        }
    }

    /// Announces the workflow run of `step` after a call changed its step run.
    async fn announce(&self, step: &StepTools) {
        let Ok(s) = self.store.get_step_run(&step.step_run_id).await else {
            return;
        };
        if let Ok(wr) = self.store.get_workflow_run(&s.workflow_run_id).await {
            self.updated(&wr);
        }
    }

    /// Performs an allowed `start_member` call of the run `run_id`, which works for the
    /// delegate step of `step` (section 5.3): `member` names one of the members it may start,
    /// without case, and `task` is 1 to 8,000 characters. Refused past the step's `total`, while
    /// `at_once` of its members run, and past a limit of the budget (section 9). The member's
    /// run is queued in a thread of its own, from the step's own start. `Ok` is the text the
    /// model gets; `Err` a tool error, in one sentence, and the run goes on.
    pub async fn call_start_member(
        &self,
        run_id: &str,
        step: &StepTools,
        input: &Value,
    ) -> std::result::Result<String, String> {
        let _one = self.lock.lock().await;
        self.step_open(run_id, step).await?;
        let Some(delegate) = &step.delegate else {
            return Err(format!(
                "{} is not available in this run",
                Tool::StartMember.name()
            ));
        };
        let (name, task) = (arg(input, "member"), arg(input, "task"));
        let Some(member) = delegate
            .may_start
            .iter()
            .find(|m| m.name.trim().eq_ignore_ascii_case(&name))
        else {
            return Err(format!(
                "\"{name}\" is not a member you may start; you may start: {}.",
                quoted(delegate.may_start.iter().map(|m| m.name.as_str()))
            ));
        };
        if task.is_empty() || task.chars().count() > MAX_TASK {
            return Err(
                "Give a task of 1 to 8,000 characters that says what the member should do.".into(),
            );
        }
        match self.start_member(step, delegate, member, &task).await {
            Ok(answer) => answer,
            Err(e) => {
                tracing::warn!(run = %run_id, "starting a member failed: {e}");
                Err(format!("Not started: {e}."))
            }
        }
    }

    /// Starts `member` on `task` for the delegate step of `step`, unless a limit refuses it
    /// (the inner `Err`, the tool's answer). Called under the lock, by `call_start_member`.
    async fn start_member(
        &self,
        step: &StepTools,
        delegate: &DelegateTools,
        member: &StartableMember,
        task: &str,
    ) -> Result<std::result::Result<String, String>> {
        // `total` counts the members the step started in every attempt (section 5.4), and
        // `at_once` those of this one that are running now.
        let started = self.store.step_members_started(&step.step_run_id).await?;
        if started >= delegate.total {
            return Ok(Err(format!(
                "Not started: you have started {} members, the most this step allows.",
                delegate.total
            )));
        }
        let members = self.store.step_members(&step.step_run_id).await?;
        let runs = self.store.runs_of_step_run(&step.step_run_id).await?;
        let running = members.iter().filter(|m| member_running(&runs, m)).count();
        if running as u32 >= delegate.at_once {
            return Ok(Err(format!(
                "Not started: {running} members you started are still running, and at most {} \
                 may run at once. End your turn; you will be told when they finish.",
                delegate.at_once
            )));
        }
        let step_run = self.store.get_step_run(&step.step_run_id).await?;
        let wr = self
            .store
            .get_workflow_run(&step_run.workflow_run_id)
            .await?;
        if let Some(limit) = budget_reached(&wr) {
            return Ok(Err(format!("Not started: Budget reached: {limit}.")));
        }
        if let Some(refusal) = self.member_refusal(&member.dot_id).await? {
            return Ok(Err(format!("Not started: {refusal}.")));
        }
        let Some(definition_step) = wr.definition.step(&step.step_id) else {
            return Err(Error::Other(format!(
                "step {} is not in the workflow",
                step.step_id
            )));
        };
        let section = self
            .member_section(&wr, definition_step, &member.dot_id, task)
            .await?;
        // A thread of its own (no parent, so its own root), working for the step run: it counts
        // against the budget as the step's own runs do, and starts where the step itself did.
        let mut new = NewRun::new(&member.dot_id, TriggerKind::Workflow);
        new.workflow_step_run_id = Some(step.step_run_id.clone());
        new.payload = Some(json!({
            "workflow": {
                "workflow_run_id": wr.id,
                "step_id": step.step_id,
                "start": start_branch(&wr, &step.step_id),
                "section": section,
            }
        }));
        let run = self.store.add_step_member(&new, task).await?;
        self.runner.enqueued(&run);
        self.updated(&self.store.get_workflow_run(&wr.id).await?);
        Ok(Ok(format!("Started \"{}\".", member.name)))
    }

    /// Performs an allowed `report_outcome` call of the run `run_id`, which works for the step
    /// of `step`: the label must be one of the step's outcomes. `Ok` is the text the model
    /// gets; `Err` a tool error, in one sentence, and the run goes on.
    pub async fn call_report_outcome(
        &self,
        run_id: &str,
        step: &StepTools,
        input: &Value,
    ) -> std::result::Result<String, String> {
        let _one = self.lock.lock().await;
        self.step_open(run_id, step).await?;
        let given = arg(input, "outcome");
        let Some(outcome) = outcome_of(&step.outcomes, &given) else {
            return Err(format!(
                "\"{given}\" is not an outcome of this step; use one of: {}.",
                quoted(step.outcomes.iter().map(String::as_str))
            ));
        };
        if let Err(e) = self
            .store
            .set_step_outcome(&step.step_run_id, Some(outcome))
            .await
        {
            tracing::warn!(run = %run_id, "recording an outcome failed: {e}");
            return Err(format!("Not recorded: {e}."));
        }
        self.announce(step).await;
        Ok("Outcome recorded.".to_string())
    }

    /// Performs an allowed `send_back` call of the run `run_id`, which works for the step of
    /// `step`: `to_step` names one of the steps it may send back to, without case, and the
    /// note is 1 to 8,000 characters. It is recorded on the step run, and performed when the
    /// step succeeds. `Ok` is the text the model gets; `Err` a tool error, in one sentence.
    pub async fn call_send_back(
        &self,
        run_id: &str,
        step: &StepTools,
        input: &Value,
    ) -> std::result::Result<String, String> {
        let _one = self.lock.lock().await;
        self.step_open(run_id, step).await?;
        let (to, note) = (arg(input, "to_step"), arg(input, "note"));
        let Some(target) = step
            .send_back_to
            .iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(&to))
        else {
            return Err(format!(
                "You can send work back only to: {}.",
                quoted(step.send_back_to.iter().map(|t| t.name.as_str()))
            ));
        };
        if note.is_empty() || note.chars().count() > MAX_TASK {
            return Err("Give a note of 1 to 8,000 characters that says what to change.".into());
        }
        let send_back = json!({"to": target.step_id, "note": note});
        if let Err(e) = self
            .store
            .set_step_send_back(&step.step_run_id, Some(&send_back))
            .await
        {
            tracing::warn!(run = %run_id, "recording a send-back failed: {e}");
            return Err(format!("Not recorded: {e}."));
        }
        self.announce(step).await;
        Ok(format!(
            "Sent back to \"{}\". End your turn now: the work goes back when you finish.",
            target.name
        ))
    }
}
