//! Jobs: the slow work of a step done outside `Workflows::lock` (teams plan B, "Off-lock
//! work"), so a pass, a Stop or the person's actions never wait on it. Building an agent step's
//! hand-off runs Dots' commit, up to two minutes; a deploy pushes and calls gh.
//!
//! A job is keyed by its step run. A pass starts it when it finds the step's work ended and no
//! job of that step run in flight. The job does its slow part unlocked, then takes the lock,
//! reads the workflow run and the step run again and writes its result only if the step run is
//! still the open current step (a Stop meanwhile wins), moves the workflow run on and wakes the
//! loop. It leaves the set of jobs in flight however it ends. A job lost on the way (Dots
//! stopped) leaves its step run open with no job, which a later pass finds: a hand-off is
//! built again (Dots' commit is safe to make twice); a step whose slow part must not run twice
//! without the person (a deploy) is failed instead.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::Semaphore;

use super::{next_of, Workflows};
use crate::model::{
    Handoff, Run, StepRun, StepRunChange, StepRunStatus, WorkflowRun, WorkflowRunStatus,
};
use crate::{Error, Result};

/// Takes a job out of the set of jobs in flight when it ends, however it ends (an error, a
/// panic, a lost gate).
struct InFlight {
    workflows: Arc<Workflows>,
    step_run_id: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.workflows
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.step_run_id);
    }
}

impl Workflows {
    /// Whether a job of the step run `step_run_id` is in flight.
    pub fn has_job(&self, step_run_id: &str) -> bool {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(step_run_id)
    }

    /// For tests: makes every job wait on `gate` before its slow part, one permit per job
    /// (`None` lets them through). Closing the gate drops the jobs waiting on it, and the ones
    /// that reach it, without a write, as Dots stopping there would. A test holds a job in its
    /// slow part this way, with no hook in the work itself.
    #[doc(hidden)]
    pub fn hold_jobs(&self, gate: Option<Arc<Semaphore>>) {
        *self.gate.lock().unwrap_or_else(|e| e.into_inner()) = gate;
    }

    /// Starts `work` as the job of the step run `step_run_id`, unless one is in flight. `work`
    /// gets the workflows to act on; it ends `Ok` when it has moved the workflow run on or
    /// found nothing to do, which wakes the loop. An error is logged and wakes nothing: the
    /// next pass (an event, the tick) starts the job again.
    pub(super) fn start_job<F, Fut>(&self, step_run_id: &str, work: F)
    where
        F: FnOnce(Arc<Self>) -> Fut,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        // Gone only while the workflows are dropped: nothing would read the result.
        let Some(this) = self.this.upgrade() else {
            return;
        };
        if !self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(step_run_id.to_string())
        {
            return;
        }
        let in_flight = InFlight {
            workflows: this.clone(),
            step_run_id: step_run_id.to_string(),
        };
        let job = work(this.clone());
        let id = step_run_id.to_string();
        tokio::spawn(async move {
            let result = job.await;
            // Out of the set before the wake, so the pass it makes may start the job again.
            drop(in_flight);
            match result {
                Ok(()) => this.wake(),
                Err(e) => tracing::warn!(step_run = %id, "a workflow step's job failed: {e}"),
            }
        });
    }

    /// Waits on the test gate, if one is set. `false`: the gate was closed, and the job ends
    /// here without a write.
    async fn through_gate(&self) -> bool {
        let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match gate {
            Some(gate) => match gate.acquire().await {
                Ok(permit) => {
                    permit.forget();
                    true
                }
                Err(_) => false,
            },
            None => true,
        }
    }

    /// The workflow run `id` and its step run `step_run_id`, read under the lock by a job about
    /// to write: `None` unless the workflow run is running with the step run as its current
    /// step and the step run is still `Running` (stopped, failed or moved on meanwhile, the job
    /// writes nothing).
    pub(super) async fn still_open(
        &self,
        id: &str,
        step_run_id: &str,
    ) -> Result<Option<(WorkflowRun, StepRun)>> {
        let wr = self.store.get_workflow_run(id).await?;
        let Some(step_run) = wr.steps.iter().find(|s| s.id == step_run_id).cloned() else {
            return Ok(None);
        };
        let open = wr.status == WorkflowRunStatus::Running
            && wr.current_step.as_deref() == Some(step_run.step_id.as_str())
            && step_run.status == StepRunStatus::Running;
        Ok(open.then_some((wr, step_run)))
    }

    /// Starts the job that ends the agent step run `step_run`, whose runs `runs` have all
    /// ended with the newest succeeded: it builds the step's hand-off (`handoff_of`, Dots'
    /// commit included), then succeeds the step run with it and begins the step after it.
    pub(super) fn start_hand_off(&self, wr: &WorkflowRun, step_run: &StepRun, runs: Vec<Run>) {
        let wr = wr.clone();
        let step_run_id = step_run.id.clone();
        self.start_job(&step_run.id, move |this| async move {
            this.hand_off(wr, step_run_id, runs).await
        });
    }

    /// The hand-off job (`start_hand_off`).
    async fn hand_off(&self, wr: WorkflowRun, step_run_id: String, runs: Vec<Run>) -> Result<()> {
        if !self.through_gate().await {
            return Ok(());
        }
        let built = self.handoff_of(&wr, &runs).await;
        let _one = self.lock.lock().await;
        let Some((wr, step_run)) = self.still_open(&wr.id, &step_run_id).await? else {
            return Ok(());
        };
        // A run that joined the thread meanwhile (a follow-up, the person's message) has work
        // this hand-off lacks: the pass that sees it end starts the job again.
        let now = self.store.runs_of_step_run(&step_run_id).await?;
        let same = now.len() == runs.len()
            && now
                .iter()
                .zip(&runs)
                .all(|(a, b)| a.id == b.id && a.status == b.status);
        if !same {
            return Ok(());
        }
        let Some(step) = wr.definition.step(&step_run.step_id) else {
            return Err(Error::Other(format!(
                "step {} is not in the workflow",
                step_run.step_id
            )));
        };
        // The step run read under the lock carries the outcome its runs reported.
        let handoff = Handoff {
            outcome: step_run.outcome.clone(),
            ..built
        };
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
        self.step_ended(&wr, &step.id);
        let wr = self.store.get_workflow_run(&wr.id).await?;
        self.begin_or_finish(&wr, next_of(&wr, &step.id)).await
    }
}
