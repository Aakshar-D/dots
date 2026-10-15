//! The scheduler (Scheduler v2 spec §5): one task that runs a pass over the schedule rows of
//! the enabled dots, then sleeps until the next slot, a waiting retry or 60 s, whichever is
//! first. `reload()` and a pause or resume wake it at once.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Local, SubsecRound, TimeDelta, Utc};
use serde_json::json;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    Dot, NewRun, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart, TriggerKind,
};
use crate::runner::Runner;
use crate::schedule::{due_slot, next_slot, Schedule, Until};
use crate::store::Store;
use crate::{Error, Result};

/// A slot handled less than this after its time is on time; later it is missed.
pub const ON_TIME: TimeDelta = TimeDelta::seconds(60);
/// A missed slot younger than this is caught up (when the schedule says so).
pub const CATCH_UP_WINDOW: TimeDelta = TimeDelta::hours(24);
/// The longest sleep between passes. Timers do not follow the wall clock across system sleep
/// or a clock change, so the loop looks at the clock again at least this often.
pub const MAX_SLEEP: TimeDelta = TimeDelta::seconds(60);
/// The wait before a pass that failed is tried again.
pub const ERROR_WAIT: TimeDelta = TimeDelta::seconds(30);

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
pub fn parse_cron(expr: &str) -> Result<cron::Schedule> {
    let fields = expr.split_whitespace().count();
    if !(6..=7).contains(&fields) {
        return Err(Error::Invalid(format!(
            "cron {expr:?}: expected 6 fields (sec min hour day month weekday) or 7 with year"
        )));
    }
    cron::Schedule::from_str(expr).map_err(|e| Error::Invalid(format!("cron {expr:?}: {e}")))
}

/// Where the scheduler and the runtime get the time. The runtime uses `SystemClock`; tests
/// use a clock they move by hand, so a day of slots passes without waiting.
#[async_trait]
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The time now, to the millisecond (the store's precision).
    fn now(&self) -> DateTime<Utc>;
    /// Returns once `now()` has reached `deadline`. The scheduler wakes it early by dropping
    /// it (on `reload()`, a pause or a resume, and at shutdown).
    async fn sleep_until(&self, deadline: DateTime<Utc>);
}

/// The PC's clock.
#[derive(Debug, Default)]
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(3)
    }

    async fn sleep_until(&self, deadline: DateTime<Utc>) {
        let wait = (deadline - Utc::now()).to_std().unwrap_or_default();
        tokio::time::sleep(wait).await;
    }
}

pub struct Scheduler {
    store: Store,
    runner: Arc<Runner>,
    bus: Bus,
    clock: Arc<dyn Clock>,
    reload: Notify,
    paused: AtomicBool,
    /// Set by a resume, cleared by the pass that follows it.
    resumed: AtomicBool,
}

impl Scheduler {
    pub fn new(
        store: Store,
        runner: Arc<Runner>,
        bus: Bus,
        clock: Arc<dyn Clock>,
    ) -> Arc<Scheduler> {
        Arc::new(Scheduler {
            store,
            runner,
            bus,
            clock,
            reload: Notify::new(),
            paused: AtomicBool::new(false),
            resumed: AtomicBool::new(false),
        })
    }

    /// While paused no pass runs: nothing fires and no retry starts. A resume does not catch
    /// up: the first pass after it moves every active row's `last_slot_at` to now.
    pub fn set_paused(&self, paused: bool) {
        if self.paused.swap(paused, Ordering::SeqCst) && !paused {
            self.resumed.store(true, Ordering::SeqCst);
        }
        self.reload();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Call after any dot create/update/delete/enable change.
    pub fn reload(&self) {
        self.reload.notify_one();
    }

    /// Runs the loop until `shutdown`. The first pass runs at once; the runtime has recovered
    /// interrupted runs before it starts the scheduler.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let wake = match this.step().await {
                    Ok(wake) => wake,
                    Err(e) => {
                        tracing::error!("scheduler error: {e}");
                        Some(this.clock.now() + ERROR_WAIT)
                    }
                };
                let sleep = async {
                    match wake {
                        Some(at) => this.clock.sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                // A reload that came during the pass is taken before the clock is asked to
                // sleep.
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = this.reload.notified() => {}
                    _ = sleep => {}
                }
            }
        })
    }

    /// One turn of the loop: `None` while paused (only a reload wakes it), else a pass and the
    /// time to wake for the next one.
    async fn step(&self) -> Result<Option<DateTime<Utc>>> {
        if self.is_paused() {
            return Ok(None);
        }
        let now = self.clock.now();
        if self.resumed.swap(false, Ordering::SeqCst) {
            for dot_id in self.store.resume_schedules(now).await? {
                self.updated(&dot_id);
            }
        }
        self.pass(now).await.map(Some)
    }

    /// A pass (spec §5) over the active rows of the enabled dots; returns when to wake.
    async fn pass(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let mut dots: std::collections::HashMap<String, Dot> = self
            .store
            .list_dots()
            .await?
            .into_iter()
            .map(|dot| (dot.id.clone(), dot))
            .collect();
        let mut wake = now + MAX_SLEEP;
        for state in self.store.list_schedule_states().await? {
            if state.status != ScheduleStatus::Active {
                continue;
            }
            let Some(dot) = dots.remove(&state.dot_id) else {
                continue;
            };
            let Some(schedule) = &dot.spec.schedule else {
                continue;
            };
            if let Some(next) = self.handle(&dot, schedule, state, now).await? {
                wake = wake.min(next);
            }
        }
        Ok(wake)
    }

    /// Steps 1, 2 and 5 of a pass for one row: the due slot is fired, caught up or skipped,
    /// then the status is updated. Returns the row's next wake time, `None` when it has none.
    async fn handle(
        &self,
        dot: &Dot,
        schedule: &Schedule,
        state: ScheduleState,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut last = parse_time(&state.last_slot_at)?;
        let mut fired = state.fired_count;
        let mut retry = state.retry;
        let limit = match schedule.until {
            Some(Until::Count(n)) => Some(n),
            _ => None,
        };
        let due = match due_slot(schedule, &last, &now, &Local) {
            Ok(due) => due.map(|t| t.with_timezone(&Utc)),
            Err(e) => return self.expire(dot, &e).await,
        };
        if let Some(due) = due {
            let late = now - due;
            let counted = limit.is_some_and(|n| fired >= n);
            let fire =
                !counted && (late < ON_TIME || (schedule.catch_up && late < CATCH_UP_WINDOW));
            let new = fire.then(|| scheduled_run(dot, due, late >= ON_TIME));
            match self
                .store
                .record_slot(&dot.id, due, new.as_ref(), now)
                .await
            {
                Ok(start) => {
                    last = due;
                    match start {
                        SlotStart::Skipped => {
                            tracing::info!(dot = %dot.spec.name, slot = %due, "missed slot skipped");
                        }
                        SlotStart::Enqueued(run) => {
                            fired += 1;
                            retry = None;
                            self.runner.enqueued(&run);
                        }
                        SlotStart::Coalesced(run) => {
                            fired += 1;
                            retry = None;
                            tracing::info!(
                                dot = %dot.spec.name, run = %run.id,
                                "schedule slot coalesced into the queued run"
                            );
                        }
                    }
                    self.updated(&dot.id);
                }
                // The row was reset or removed after the pass read it; the reload that
                // follows such a change brings the next pass.
                Err(Error::Conflict(_)) => return Ok(None),
                Err(e) => return Err(e),
            }
        }

        // Step 3, a due retry, and step 4, judging the watched run, are Task 5's (spec §6).

        let next = if limit.is_some_and(|n| fired >= n) {
            None
        } else {
            match next_slot(schedule, &last, &Local) {
                Ok(next) => next.map(|t| t.with_timezone(&Utc)),
                Err(e) => return self.expire(dot, &e).await,
            }
        };
        let retry_at = retry.as_ref().map(|r| parse_time(&r.at)).transpose()?;
        if next.is_none() && retry_at.is_none() {
            // No slot is left and no retry can follow: the count is reached, a `Once` is
            // handled, or the stop date has passed.
            self.set_status(dot, ScheduleStatus::Completed, now).await?;
            tracing::info!(dot = %dot.spec.name, "schedule completed");
            return Ok(None);
        }
        // A retry that is due already waits for Task 5's step 3; it is no reason to wake.
        let retry_at = retry_at.filter(|at| *at > now);
        Ok([next, retry_at].into_iter().flatten().min())
    }

    /// The schedule no longer parses (spec §10): the row is `expired` and fires nothing.
    async fn expire(&self, dot: &Dot, e: &Error) -> Result<Option<DateTime<Utc>>> {
        tracing::warn!(dot = %dot.spec.name, "schedule expired: {e}");
        self.set_status(dot, ScheduleStatus::Expired, self.clock.now())
            .await?;
        Ok(None)
    }

    async fn set_status(
        &self,
        dot: &Dot,
        status: ScheduleStatus,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let change = ScheduleChange {
            status: Some(status),
            ..Default::default()
        };
        if self
            .store
            .update_schedule_state(&dot.id, &change, now)
            .await?
            .is_some()
        {
            self.updated(&dot.id);
        }
        Ok(())
    }

    fn updated(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::ScheduleUpdated {
            dot_id: dot_id.to_string(),
        });
    }
}

/// The run that serves the slot at `at`; `caught_up` when the slot was missed.
fn scheduled_run(dot: &Dot, at: DateTime<Utc>, caught_up: bool) -> NewRun {
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    let mut payload = json!({ "scheduled_for": at.with_timezone(&Local).to_rfc3339() });
    if caught_up {
        payload["caught_up"] = json!(true);
    }
    new.payload = Some(payload);
    new
}

/// A time the store wrote (`util::timestamp`).
fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Invalid(format!("stored time {s:?}: {e}")))
}
