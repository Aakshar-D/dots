//! The scheduler (Scheduler v2 spec §5): one task that runs a pass over the schedule rows of
//! the enabled dots, then sleeps until the next slot, a waiting retry or 60 s, whichever is
//! first. `reload()`, a pause or resume, and the end of a watched run wake it at once. The
//! pass also judges the outcome of each occurrence and retries it or flags it (spec §6).

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Local, SubsecRound, TimeDelta, Utc};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{
    Dot, NewRun, PendingRetry, Run, RunStatus, ScheduleChange, ScheduleState, ScheduleStatus,
    SlotStart, TriggerKind,
};
use crate::runner::Runner;
use crate::schedule::{due_slot, next_slot, Schedule, Until};
use crate::store::Store;
use crate::util::timestamp;
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
/// The attempts an occurrence has at most: the run of its slot and five retries (spec §6).
pub const MAX_ATTEMPTS: u32 = 6;

/// How long after attempt `attempt` (1 to 5) ended the next one starts: 1, 2, 4, 8 and 16
/// minutes.
pub fn retry_delay(attempt: u32) -> TimeDelta {
    TimeDelta::minutes(1 << (attempt.clamp(1, MAX_ATTEMPTS - 1) - 1))
}

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
    /// The runs the rows watched at the end of the last pass: the end of one wakes the loop.
    watched: Mutex<HashSet<String>>,
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
            watched: Mutex::new(HashSet::new()),
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
        // Subscribed before the first pass, so no end of a run it watches is missed.
        let mut events = self.bus.subscribe();
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
                    _ = this.watched_run_ended(&mut events) => {}
                    _ = sleep => {}
                }
            }
        })
    }

    /// Returns when a run the rows watch has ended (its `RunUpdated`), or when events were
    /// lost and one may have.
    async fn watched_run_ended(&self, events: &mut Receiver<RuntimeEvent>) {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::RunUpdated { run })
                    if run.status.is_terminal()
                        && self.watched.lock().unwrap().contains(&run.id) =>
                {
                    return
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => return,
                Err(RecvError::Closed) => std::future::pending().await,
            }
        }
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
        let mut dots: HashMap<String, Dot> = self
            .store
            .list_dots()
            .await?
            .into_iter()
            .map(|dot| (dot.id.clone(), dot))
            .collect();
        let mut wake = now + MAX_SLEEP;
        let mut watched = HashSet::new();
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
            if let Some(next) = self
                .handle(&dot, schedule, state, now, &mut watched)
                .await?
            {
                wake = wake.min(next);
            }
        }
        *self.watched.lock().unwrap() = watched;
        Ok(wake)
    }

    /// A pass for one row (spec §5): the due slot is fired, caught up or skipped, a due retry
    /// starts, the watched run is judged once it has ended, then the status is updated. Adds
    /// the run the row watches to `watched`. Returns the row's next wake time, `None` when it
    /// has none.
    async fn handle(
        &self,
        dot: &Dot,
        schedule: &Schedule,
        state: ScheduleState,
        now: DateTime<Utc>,
        watched: &mut HashSet<String>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut last = parse_time(&state.last_slot_at)?;
        let mut fired = state.fired_count;
        let mut watch = state.watch_run_id.clone();
        let mut retry = state.retry.clone();
        let mut attention = state.attention_run_id.clone();
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
                            watch = Some(run.id);
                        }
                        SlotStart::Coalesced(run) => {
                            fired += 1;
                            retry = None;
                            tracing::info!(
                                dot = %dot.spec.name, run = %run.id,
                                "schedule slot coalesced into the queued run"
                            );
                            watch = Some(run.id);
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

        // Step 3: a retry that is due starts. While it waited, the row watched the failed run
        // it repeats.
        if let Some(due) = &retry {
            if parse_time(&due.at)? <= now {
                let new = retry_run(dot, due, watch.clone())?;
                match self.store.start_retry(&dot.id, &new, now).await {
                    Ok(start) => {
                        match start {
                            SlotStart::Enqueued(run) => {
                                tracing::info!(
                                    dot = %dot.spec.name, run = %run.id, attempt = due.attempt,
                                    "scheduled run retried"
                                );
                                self.runner.enqueued(&run);
                                watch = Some(run.id);
                            }
                            SlotStart::Coalesced(run) => {
                                tracing::info!(
                                    dot = %dot.spec.name, run = %run.id, attempt = due.attempt,
                                    "retry coalesced into the queued run"
                                );
                                watch = Some(run.id);
                            }
                            SlotStart::Skipped => {}
                        }
                        retry = None;
                        self.updated(&dot.id);
                    }
                    Err(Error::Conflict(_)) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
        }

        // Step 4: the watched run is judged once it has ended. A failed run that a retry
        // repeats was judged already.
        if retry.is_none() {
            if let Some(id) = &watch {
                if let Some(row) = self
                    .judge(dot, id, &state.last_slot_at, attention.is_some(), now)
                    .await?
                {
                    watch = row.watch_run_id;
                    retry = row.retry;
                    attention = row.attention_run_id;
                }
            }
        }

        // "Needs attention" also clears when a later manual or scheduled run of the dot
        // succeeds: the Inbox offers "Run now" next to the flag.
        if let Some(flagged) = &attention {
            if self.store.succeeded_after(&dot.id, flagged).await? {
                let change = ScheduleChange {
                    attention_run_id: Some(None),
                    ..Default::default()
                };
                if self
                    .store
                    .update_schedule_state(&dot.id, &change, now)
                    .await?
                    .is_some()
                {
                    tracing::info!(dot = %dot.spec.name, "a later run succeeded: attention cleared");
                    self.updated(&dot.id);
                }
            }
        }

        let next = if limit.is_some_and(|n| fired >= n) {
            None
        } else {
            match next_slot(schedule, &last, &Local) {
                Ok(next) => next.map(|t| t.with_timezone(&Utc)),
                Err(e) => return self.expire(dot, &e).await,
            }
        };
        let retry_at = retry.as_ref().map(|r| parse_time(&r.at)).transpose()?;
        if next.is_none() && retry_at.is_none() && watch.is_none() {
            // No slot is left, the last occurrence is judged and no retry can follow: the
            // count is reached, a `Once` is handled, or the stop date has passed.
            self.set_status(dot, ScheduleStatus::Completed, now).await?;
            tracing::info!(dot = %dot.spec.name, "schedule completed");
            return Ok(None);
        }
        watched.extend(watch);
        // A retry that is due already (its run ended while Dots was closed) starts on the next
        // pass, at once.
        Ok([next, retry_at].into_iter().flatten().min())
    }

    /// Step 4 (spec §6): judges the watched run `id` if it has ended, with a write guarded by
    /// the watch. A parked run closed by its decision hands the watch to its resume run, which
    /// is judged in turn. `attention_set`: the flag was set before. Returns the row as last
    /// written, `None` when nothing was.
    async fn judge(
        &self,
        dot: &Dot,
        id: &str,
        last_slot_at: &str,
        attention_set: bool,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduleState>> {
        let mut id = id.to_string();
        let mut written = None;
        loop {
            let run = match self.store.get_run(&id).await {
                Ok(run) => run,
                // Deleted meanwhile: the row lost its watch with it (ON DELETE SET NULL), and
                // the occurrence counts as done.
                Err(Error::NotFound(_)) => return Ok(written),
                Err(e) => return Err(e),
            };
            let mut change = match run.status {
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval => {
                    return Ok(written)
                }
                RunStatus::Succeeded => match self.store.resume_child(&id).await? {
                    Some(child) => {
                        let change = ScheduleChange {
                            watch_run_id: Some(Some(child.clone())),
                            if_watching: Some(id.clone()),
                            ..Default::default()
                        };
                        match self
                            .store
                            .update_schedule_state(&dot.id, &change, now)
                            .await?
                        {
                            Some(row) => {
                                self.updated(&dot.id);
                                written = Some(row);
                                id = child;
                                continue;
                            }
                            None => return Ok(written),
                        }
                    }
                    // The occurrence is done.
                    None => ScheduleChange {
                        watch_run_id: Some(None),
                        attention_run_id: Some(None),
                        ..Default::default()
                    },
                },
                // The person stopped it: done, without a retry or attention.
                RunStatus::Cancelled => ScheduleChange {
                    watch_run_id: Some(None),
                    ..Default::default()
                },
                RunStatus::Failed => self.failed(&run, last_slot_at, now).await?,
            };
            change.if_watching = Some(id);
            let Some(row) = self
                .store
                .update_schedule_state(&dot.id, &change, now)
                .await?
            else {
                return Ok(written);
            };
            self.updated(&dot.id);
            if !attention_set && row.attention_run_id.as_deref() == Some(run.id.as_str()) {
                tracing::warn!(
                    dot = %dot.spec.name, run = %run.id,
                    "scheduled run failed in the end: the schedule needs attention"
                );
                let _ = self.bus.send(RuntimeEvent::ScheduleAttention {
                    dot_id: dot.id.clone(),
                    run_id: run.id.clone(),
                    error: run.error.clone(),
                });
            }
            return Ok(Some(row));
        }
    }

    /// What a failed watched run leads to (spec §6). Failed before any tool call, with
    /// attempts left: the next attempt, `retry_delay` after it ended; the row keeps watching
    /// it, the run the retry repeats. Otherwise "Needs attention".
    async fn failed(
        &self,
        run: &Run,
        last_slot_at: &str,
        now: DateTime<Utc>,
    ) -> Result<ScheduleChange> {
        let (slot, attempt) = self.occurrence(run, last_slot_at).await?;
        if attempt < MAX_ATTEMPTS && !self.store.called_a_tool(&run.id).await? {
            let ended = match &run.ended_at {
                Some(t) => parse_time(t)?,
                None => now,
            };
            let retry = PendingRetry {
                slot_at: timestamp(slot),
                attempt: attempt + 1,
                at: timestamp(ended + retry_delay(attempt)),
            };
            tracing::info!(
                run = %run.id, attempt = retry.attempt, at = %retry.at,
                "scheduled run failed before any tool call: retry"
            );
            return Ok(ScheduleChange {
                retry: Some(Some(retry)),
                ..Default::default()
            });
        }
        Ok(ScheduleChange {
            watch_run_id: Some(None),
            attention_run_id: Some(Some(run.id.clone())),
            ..Default::default()
        })
    }

    /// The slot whose occurrence `run` serves, and which attempt it is. A resume run speaks
    /// for the run it resumes; a scheduled run names both in its payload (`scheduled_for`,
    /// and `attempt` for a retry). A run of another trigger that a slot or retry joined is
    /// taken as attempt 1 of the newest handled slot.
    async fn occurrence(&self, run: &Run, last_slot_at: &str) -> Result<(DateTime<Utc>, u32)> {
        let mut head = run.clone();
        while head.trigger == TriggerKind::Resume {
            let Some(parent) = head.parent_run_id.clone() else {
                break;
            };
            head = self.store.get_run(&parent).await?;
        }
        let payload = head
            .payload
            .as_ref()
            .filter(|_| head.trigger == TriggerKind::Schedule);
        let field = |key: &str| payload.and_then(|p| p.get(key));
        let slot = field("scheduled_for")
            .and_then(Value::as_str)
            .and_then(|s| parse_time(s).ok());
        let attempt = field("attempt")
            .and_then(Value::as_u64)
            .map_or(1, |a| a as u32);
        let slot = match slot {
            Some(slot) => slot,
            None => parse_time(last_slot_at)?,
        };
        Ok((slot, attempt))
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

/// The run that retries the failed run `failed` for the pending retry `retry` (spec §6).
fn retry_run(dot: &Dot, retry: &PendingRetry, failed: Option<String>) -> Result<NewRun> {
    let slot = parse_time(&retry.slot_at)?;
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.payload = Some(json!({
        "scheduled_for": slot.with_timezone(&Local).to_rfc3339(),
        "attempt": retry.attempt,
    }));
    new.retry_of = failed;
    Ok(new)
}

/// A time the store wrote (`util::timestamp`).
fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Invalid(format!("stored time {s:?}: {e}")))
}
