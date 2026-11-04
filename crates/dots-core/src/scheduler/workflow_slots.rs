//! The scheduled workflows (teams spec section 5.1). Each pass of the scheduler also fires
//! every workflow that is turned on, has no `problem` and whose trigger is a schedule. Its slots
//! are a dot's (`schedule::due_slot`, `Until`), kept in its own row (`workflow_schedule_state`),
//! but a workflow schedule never catches up and never retries:
//!
//! - A due slot handled `ON_TIME` or later after its time is skipped. So after a gap (Dots was
//!   closed, or the PC asleep), the first pass marks the newest missed slot handled and starts
//!   nothing; none of the missed slots runs, and the next slot starts on time.
//! - A slot that comes due while a run of the workflow is in flight (running or waiting, a check
//!   included) is skipped, with a log line.
//! - A start that is refused (the project or its repo is gone, the workflow was turned off, the
//!   role may not run dots) is logged, and the slot is used up.
//!
//! The slot is taken before its run is started, so a slot starts at most one run. Only a
//! started run counts towards `Until::Count`. A row is made at the pass's time, and reset when
//! its rule key is not the schedule's, so a new or changed rule starts no slot from before. A
//! workflow that no longer fires on a schedule (turned off, a problem, another trigger) loses
//! its row, so turning it back on starts afresh, as enabling a dot does.

use std::collections::HashMap;

use chrono::{DateTime, Local, Utc};

use super::{parse_time, Scheduler, ON_TIME};
use crate::model::{
    ScheduleStatus, Team, Workflow, WorkflowRun, WorkflowScheduleState, WorkflowTrigger,
};
use crate::schedule::{due_slot, next_slot, Schedule, Until};
use crate::util::timestamp;
use crate::workflow::Trigger;
use crate::workflows::{StartRequest, Workflows};
use crate::{Error, Result};

/// What a workflow's schedule trigger asks for: its schedule, its task and the name of its
/// project.
struct Scheduled<'a> {
    team: &'a Team,
    workflow: &'a Workflow,
    schedule: &'a Schedule,
    task: &'a str,
    project: &'a str,
}

impl Scheduler {
    /// The scheduled workflows' part of a pass. Returns the earliest next slot among them;
    /// `None` when none has one, or when no workflows are set. A workflow whose slot could not
    /// be handled is logged and tried again on the next pass.
    pub(super) async fn workflow_pass(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
        let Some(workflows) = self.workflows.get() else {
            return Ok(None);
        };
        let mut rows: HashMap<String, WorkflowScheduleState> = self
            .store
            .list_workflow_schedule_states()
            .await?
            .into_iter()
            .map(|row| (row.workflow_id.clone(), row))
            .collect();
        let mut wake: Option<DateTime<Utc>> = None;
        for team in self.store.list_teams().await? {
            for workflow in self.store.list_workflows(&team.id).await? {
                let Trigger::Schedule {
                    schedule,
                    task,
                    project,
                } = &workflow.spec.definition.trigger
                else {
                    continue;
                };
                if !workflow.spec.enabled || workflow.problem.is_some() {
                    continue;
                }
                let row = rows.remove(&workflow.id);
                let scheduled = Scheduled {
                    team: &team,
                    workflow: &workflow,
                    schedule,
                    task,
                    project,
                };
                match self.handle_workflow(workflows, &scheduled, row, now).await {
                    Ok(next) => wake = wake.into_iter().chain(next).min(),
                    Err(e) => tracing::warn!(
                        workflow = %workflow.spec.name,
                        "the workflow's schedule could not be handled: {e}"
                    ),
                }
            }
        }
        // The rows left belong to workflows that no longer fire on a schedule.
        for workflow_id in rows.into_keys() {
            self.store
                .delete_workflow_schedule_state(&workflow_id)
                .await?;
        }
        Ok(wake)
    }

    /// One scheduled workflow's part of a pass (module docs): its row made or reset, the due
    /// slot started or skipped, the status updated. Returns its next slot, `None` when it has
    /// none.
    async fn handle_workflow(
        &self,
        workflows: &Workflows,
        s: &Scheduled<'_>,
        row: Option<WorkflowScheduleState>,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>> {
        let id = &s.workflow.id;
        let name = &s.workflow.spec.name;
        let row = match row {
            Some(row) if row.rule_key == s.schedule.rule_key() => row,
            _ => {
                self.store
                    .sync_workflow_schedule_state(id, s.schedule, now)
                    .await?
            }
        };
        if row.status != ScheduleStatus::Active {
            return Ok(None);
        }
        let mut last = parse_time(&row.last_slot_at)?;
        let mut row_slot = row.last_slot_at.clone();
        let mut fired = row.fired_count;
        let limit = match s.schedule.until {
            Some(Until::Count(n)) => Some(n),
            _ => None,
        };
        let due = match due_slot(s.schedule, &last, &now, &Local) {
            Ok(due) => due.map(|t| t.with_timezone(&Utc)),
            Err(e) => return self.expire_workflow(s.workflow, &e, &row_slot, now).await,
        };
        if let Some(due) = due {
            if !self.store.record_workflow_slot(id, due, now).await? {
                // Taken since the row was read: nothing to do.
                return Ok(None);
            }
            last = due;
            row_slot = timestamp(due);
            if limit.is_some_and(|n| fired >= n) {
                // The count was reached: the status below completes the row.
            } else if now - due >= ON_TIME {
                tracing::info!(
                    workflow = %name, slot = %due,
                    "missed slot skipped: a workflow schedule never catches up"
                );
            } else if self.store.workflow_has_runs_in_flight(id).await? {
                tracing::info!(
                    workflow = %name, slot = %due,
                    "slot skipped: a run of the workflow is still in flight"
                );
            } else {
                match start(workflows, s).await {
                    Ok(wr) => {
                        self.store.count_workflow_slot(id, now).await?;
                        fired += 1;
                        tracing::info!(
                            workflow = %name, workflow_run = %wr.id, slot = %due,
                            "scheduled workflow started"
                        );
                    }
                    Err(e) => tracing::warn!(
                        workflow = %name, slot = %due,
                        "scheduled workflow not started, the slot is used up: {e}"
                    ),
                }
            }
        }
        let next = if limit.is_some_and(|n| fired >= n) {
            None
        } else {
            match next_slot(s.schedule, &last, &Local) {
                Ok(next) => next.map(|t| t.with_timezone(&Utc)),
                Err(e) => return self.expire_workflow(s.workflow, &e, &row_slot, now).await,
            }
        };
        if next.is_none() {
            // No slot is left: the count is reached, a `Once` is handled, or the stop date has
            // passed.
            if self
                .store
                .set_workflow_schedule_status(id, ScheduleStatus::Completed, &row_slot, now)
                .await?
            {
                tracing::info!(workflow = %name, "workflow schedule completed");
            }
        }
        Ok(next)
    }

    /// The schedule no longer gives slots (an `Every` without an anchor, a cron expression
    /// that no longer parses): the row is `expired` and fires nothing until its rule changes.
    async fn expire_workflow(
        &self,
        workflow: &Workflow,
        e: &Error,
        last_slot_at: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>> {
        if self
            .store
            .set_workflow_schedule_status(&workflow.id, ScheduleStatus::Expired, last_slot_at, now)
            .await?
        {
            tracing::warn!(workflow = %workflow.spec.name, "workflow schedule expired: {e}");
        }
        Ok(None)
    }
}

/// Starts the scheduled workflow on the project its trigger names, found by name without case
/// among its team's projects, with the trigger's task. `Workflows::start` refuses what it
/// refuses for any start.
async fn start(workflows: &Workflows, s: &Scheduled<'_>) -> Result<WorkflowRun> {
    let wanted = s.project.trim();
    let Some(project) = s
        .team
        .projects
        .iter()
        .find(|p| p.spec.name.trim().eq_ignore_ascii_case(wanted))
    else {
        return Err(Error::NotFound(format!(
            "the team has no project named \"{wanted}\""
        )));
    };
    workflows
        .start(StartRequest {
            workflow_id: s.workflow.id.clone(),
            project_id: project.id.clone(),
            task: s.task.to_string(),
            trigger: WorkflowTrigger::Schedule,
            origin_run_id: None,
            test: false,
        })
        .await
}
