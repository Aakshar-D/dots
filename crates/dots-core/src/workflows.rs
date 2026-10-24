//! Workflow runs (teams and workflows spec sections 5 to 8). `Workflows` starts them and moves
//! each one along its steps: an agent step queues a run of its member and ends when that run's
//! thread does; a check waits for the person. Every move is made by a pass that reads the
//! workflow runs in flight from the store and does what each one's state calls for, so a
//! restart or a lost event never loses a step. The loop passes when a run of a step changes,
//! when a question is posted or answered, when it is woken, and every `TICK`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    BranchRef, Handoff, NewRun, NewWorkflowRun, Run, RunStatus, StepRun, StepRunChange,
    StepRunStatus, TriggerKind, WaitingOn, WorkflowRun, WorkflowRunChange, WorkflowRunStatus,
    WorkflowTrigger,
};
use crate::runner::Runner;
use crate::scheduler::Clock;
use crate::store::Store;
use crate::util::timestamp;
use crate::workflow::{Budget, Step, StepKind};
use crate::workspace::git;
use crate::{Error, Result};

/// The longest task, in characters after trimming.
pub const MAX_TASK: usize = 8_000;
/// The longest summary of an earlier step a prompt carries, in characters.
pub const MAX_HANDOFF_SUMMARY: usize = 4_000;
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
}

/// The newest step run of `step_id`.
fn latest<'a>(wr: &'a WorkflowRun, step_id: &str) -> Option<&'a StepRun> {
    wr.steps.iter().rev().find(|s| s.step_id == step_id)
}

/// The hand-offs of the agent steps before `index`, oldest first: each step's newest
/// succeeded run.
fn handoffs_before(wr: &WorkflowRun, index: usize) -> Vec<(&Step, &Handoff)> {
    wr.definition.steps[..index]
        .iter()
        .filter(|s| matches!(s.kind, StepKind::Agent { .. }))
        .filter_map(|s| {
            let run = wr
                .steps
                .iter()
                .rev()
                .find(|r| r.step_id == s.id && r.status == StepRunStatus::Succeeded)?;
            Some((s, run.handoff.as_ref()?))
        })
        .collect()
}

/// The branch a step at `index` starts from (section 6): the newest earlier hand-off that has
/// branches, when it has exactly one. `None` starts from the project's default branch.
fn start_branch(wr: &WorkflowRun, index: usize) -> Option<String> {
    let newest = handoffs_before(wr, index)
        .into_iter()
        .rev()
        .find(|(_, h)| !h.branches.is_empty())?;
    match newest.1.branches.as_slice() {
        [only] => Some(only.branch.clone()),
        _ => None,
    }
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

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str(" [cut]");
    out
}

impl Workflows {
    pub fn new(store: Store, bus: Bus, runner: Arc<Runner>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            store,
            bus,
            runner,
            clock,
            wake: Notify::new(),
            lock: Mutex::new(()),
        })
    }

    fn updated(&self, wr: &WorkflowRun) {
        let _ = self
            .bus
            .send(RuntimeEvent::WorkflowRunUpdated { run: wr.clone() });
    }

    /// Makes the loop pass now.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub async fn get(&self, id: &str) -> Result<WorkflowRun> {
        self.store.get_workflow_run(id).await
    }

    /// The newest workflow runs, of one team or of all.
    pub async fn list(&self, team_id: Option<&str>, limit: i64) -> Result<Vec<WorkflowRun>> {
        self.store.list_workflow_runs(team_id, limit).await
    }

    /// Starts a run of a workflow on one of its team's projects. Refused for a workflow that is
    /// turned off or has a `problem`, a project of another team or whose folder is no longer a
    /// git repo, and a task that is blank or longer than `MAX_TASK`.
    pub async fn start(&self, req: StartRequest) -> Result<WorkflowRun> {
        let task = req.task.trim();
        if task.is_empty() || task.chars().count() > MAX_TASK {
            return Err(Error::Invalid(
                "the task must be 1 to 8,000 characters".into(),
            ));
        }
        let workflow = self.store.get_workflow(&req.workflow_id).await?;
        let name = &workflow.spec.name;
        if !workflow.spec.enabled {
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
        let wr = self.tick(&self.store.get_workflow_run(id).await?).await?;
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
            failed
                .then(|| running.definition.step(current).map(|(i, _)| i))
                .flatten()
        });
        match held_retry {
            Some(index) => self.begin_step(&running, index).await?,
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

    /// Approves the check the workflow run waits on (section 5.4): the check passes the hand-off
    /// it was shown on, and the workflow goes on.
    pub async fn approve_check(&self, id: &str, note: Option<&str>) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
        let wr = self.tick(&self.store.get_workflow_run(id).await?).await?;
        let step_run = match (&wr.status, &wr.waiting_on, &wr.current_step) {
            (WorkflowRunStatus::Waiting, Some(WaitingOn::Check), Some(current)) => {
                latest(&wr, current).cloned()
            }
            _ => None,
        };
        let Some(step_run) = step_run else {
            return Err(Error::Conflict(format!(
                "workflow run {id} is not waiting for a check"
            )));
        };
        let index = wr.definition.step(&step_run.step_id).map_or(0, |(i, _)| i);
        let shown = handoffs_before(&wr, index)
            .last()
            .map(|(_, h)| (*h).clone())
            .unwrap_or_default();
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Succeeded),
                    handoff: Some(Some(shown)),
                    decision: Some(Some(json!({"decision": "approved", "note": note}))),
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
        let wr = self.tick(&self.store.get_workflow_run(id).await?).await?;
        let index = match (&wr.status, &wr.waiting_on, &wr.current_step) {
            (WorkflowRunStatus::Waiting, Some(WaitingOn::StepFailed), Some(current)) => {
                wr.definition.step(current).map(|(i, _)| i)
            }
            _ => None,
        };
        let Some(index) = index else {
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
        self.begin_step(&wr, index).await?;
        let wr = self.store.get_workflow_run(id).await?;
        self.updated(&wr);
        Ok(wr)
    }

    /// Stops the workflow run (section 8): its queued and running dot runs are cancelled, its
    /// open steps and a waiting check are closed as cancelled.
    pub async fn stop(&self, id: &str) -> Result<WorkflowRun> {
        let _one = self.lock.lock().await;
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
            for run in self.store.runs_of_step_run(&step_run.id).await? {
                if !run.status.is_terminal() {
                    if let Err(e) = self.runner.cancel(&run.id).await {
                        tracing::warn!(run = %run.id, "cancelling a workflow's run failed: {e}");
                    }
                }
            }
            self.store
                .set_step_run(
                    &step_run.id,
                    &StepRunChange {
                        status: Some(StepRunStatus::Cancelled),
                        ..Default::default()
                    },
                )
                .await?;
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

    /// Stops the team's workflow runs in flight, then deletes the team (section 4).
    pub async fn delete_team(&self, team_id: &str) -> Result<()> {
        let _one = self.lock.lock().await;
        for wr in self
            .store
            .list_workflow_runs(Some(team_id), i64::MAX)
            .await?
        {
            if !wr.status.is_terminal() {
                self.stop_locked(&wr.id).await?;
            }
        }
        self.store.delete_team(team_id).await?;
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
            return self.begin_step(&wr, 0).await;
        };
        let Some((index, _)) = wr.definition.step(&current) else {
            return Err(Error::Other(format!(
                "step {current} is not in the workflow"
            )));
        };
        match latest(&wr, &current).map(|s| (s.status, s.clone())) {
            None => self.begin_step(&wr, index).await,
            Some((StepRunStatus::Running, step_run)) => {
                self.watch_agent(&wr, index, &step_run).await
            }
            Some((StepRunStatus::Succeeded, _)) => self.begin_step(&wr, index + 1).await,
            // A move cut off between its two writes (a crash, a store error) leaves the run
            // `Running` behind a step that is failed or waiting for the person: put the run
            // where the person can act on it.
            Some((StepRunStatus::Failed, step_run)) => {
                let error = step_run
                    .error
                    .clone()
                    .unwrap_or_else(|| "the step failed".to_string());
                let name = wr.definition.steps[index].name.clone();
                let waiting = self
                    .store
                    .set_workflow_run(
                        &wr.id,
                        &WorkflowRunChange {
                            status: Some(WorkflowRunStatus::Waiting),
                            waiting_on: Some(Some(WaitingOn::StepFailed)),
                            error: Some(Some(format!("{name}: {error}"))),
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

    /// Starts the step at `index`: an agent step queues its member's run, a check waits for the
    /// person. Past the last step the workflow run has succeeded.
    async fn begin_step(&self, wr: &WorkflowRun, index: usize) -> Result<()> {
        let Some(step) = wr.definition.steps.get(index) else {
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
            return Ok(());
        };
        let attempt = wr.steps.iter().filter(|s| s.step_id == step.id).count() as u32 + 1;
        match &step.kind {
            StepKind::Check {} => {
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
                    return Ok(());
                }
                let refusal = match self.store.get_dot(member).await {
                    Ok(dot) if dot.spec.enabled => None,
                    Ok(dot) => Some(format!("{} is disabled", dot.spec.name)),
                    Err(Error::NotFound(_)) => Some("its member is no longer a dot".to_string()),
                    Err(e) => return Err(e),
                };
                let status = if refusal.is_some() {
                    StepRunStatus::Failed
                } else {
                    StepRunStatus::Running
                };
                let step_run = self
                    .store
                    .create_step_run(&wr.id, &step.id, attempt, status)
                    .await?;
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
                        let wr = self.store.get_workflow_run(&wr.id).await?;
                        self.queue_agent_run(&wr, index, &step_run).await?;
                        self.updated(&self.store.get_workflow_run(&wr.id).await?);
                    }
                }
            }
        }
        Ok(())
    }

    /// Queues the run of an agent step's member, with the `## Workflow` section and the branch
    /// it starts from in its payload (sections 5.2 and 6).
    async fn queue_agent_run(
        &self,
        wr: &WorkflowRun,
        index: usize,
        step_run: &StepRun,
    ) -> Result<Run> {
        let step = &wr.definition.steps[index];
        let StepKind::Agent { member, .. } = &step.kind else {
            return Err(Error::Other(format!(
                "step {} is not an agent step",
                step.id
            )));
        };
        let section = self.section(wr, index).await?;
        let mut new = NewRun::new(member, TriggerKind::Workflow);
        new.workflow_step_run_id = Some(step_run.id.clone());
        new.payload = Some(json!({
            "workflow": {
                "workflow_run_id": wr.id,
                "step_id": step.id,
                "start": start_branch(wr, index),
                "section": section,
            }
        }));
        self.runner.enqueue(new).await
    }

    /// The `## Workflow` section of the prompt of the step at `index` (section 5.2).
    async fn section(&self, wr: &WorkflowRun, index: usize) -> Result<String> {
        let step = &wr.definition.steps[index];
        let StepKind::Agent { member, job } = &step.kind else {
            return Ok(String::new());
        };
        let team = self.store.get_team(&wr.team_id).await?;
        let role = team
            .members
            .iter()
            .find(|m| &m.dot_id == member)
            .map(|m| m.role.trim().to_string())
            .filter(|r| !r.is_empty());
        let workflow = match &wr.workflow_id {
            Some(id) => self.store.get_workflow(id).await.ok().map(|w| w.spec.name),
            None => None,
        }
        .unwrap_or_else(|| "this workflow".to_string());
        let mut out = String::from("## Workflow\n\n");
        out.push_str(&format!(
            "You are {} of the team \"{}\", doing one step of the workflow \"{}\".\n\n",
            role.map_or("a member".to_string(), |r| format!("the {r}")),
            team.name,
            workflow
        ));
        out.push_str(&format!(
            "- Project: {} ({})\n",
            wr.project.name, wr.project.repo_path
        ));
        out.push_str(&format!(
            "- Step: {} ({} of {})\n\n",
            step.name,
            index + 1,
            wr.definition.steps.len()
        ));
        out.push_str("### Task\n\n");
        out.push_str(wr.task.trim());
        out.push_str("\n\n### Your job in this step\n\n");
        out.push_str(job.trim());
        out.push_str("\n\n");
        let earlier = handoffs_before(wr, index);
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
        out.push_str(
            "When you are done, end with a summary of what you did: the next step gets it.\n",
        );
        Ok(out)
    }

    /// Watches an agent step's runs: it ends when its newest run's thread has ended and no
    /// question of it waits for the person. A step whose run is gone gets one again.
    async fn watch_agent(&self, wr: &WorkflowRun, index: usize, step_run: &StepRun) -> Result<()> {
        // The question first: an answer closes it and stores the run that continues the thread
        // in one transaction, so once it reads as closed, the runs read next include that run.
        if self.store.step_has_open_question(&step_run.id).await? {
            return Ok(());
        }
        let runs = self.store.runs_of_step_run(&step_run.id).await?;
        let Some(last) = runs.last() else {
            self.queue_agent_run(wr, index, step_run).await?;
            return Ok(());
        };
        if runs.iter().any(|r| !r.status.is_terminal()) {
            return Ok(());
        }
        let step = &wr.definition.steps[index];
        if last.status != RunStatus::Succeeded {
            let error = last.error.clone().unwrap_or_else(|| match last.status {
                RunStatus::Cancelled => "the run was cancelled".to_string(),
                _ => "the run failed".to_string(),
            });
            return self.fail_step(wr, step, step_run, &error).await;
        }
        let handoff = self.handoff_of(wr, &runs).await;
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Succeeded),
                    handoff: Some(Some(handoff)),
                    ..Default::default()
                },
            )
            .await?;
        let wr = self.store.get_workflow_run(&wr.id).await?;
        self.begin_step(&wr, index + 1).await
    }

    /// Fails a step: the workflow run waits for the person to retry it or stop (section 8).
    async fn fail_step(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
        error: &str,
    ) -> Result<()> {
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Failed),
                    error: Some(Some(error.to_string())),
                    ..Default::default()
                },
            )
            .await?;
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

    /// What a finished agent step hands on (section 6): its newest run's summary, and the
    /// branch of its thread when the branch still exists (a worktree with no change was
    /// removed with its branch).
    async fn handoff_of(&self, wr: &WorkflowRun, runs: &[Run]) -> Handoff {
        let last = runs.last();
        let summary = last.and_then(|r| r.summary.clone()).unwrap_or_default();
        let mut branches = Vec::new();
        if let Some(run) = runs.iter().rev().find(|r| r.branch.is_some()) {
            let branch = run.branch.clone().unwrap_or_default();
            let exists = git(
                Path::new(&wr.project.repo_path),
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .await
            .is_ok();
            if exists {
                branches.push(BranchRef {
                    run_id: run.id.clone(),
                    branch,
                    worktree_path: run.workspace_path.clone().unwrap_or_default(),
                    base_commit: run.base_commit.clone(),
                });
            }
        }
        Handoff {
            summary: cut(&summary, MAX_HANDOFF_SUMMARY),
            branches,
            outcome: None,
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
