mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{
    Dot, EngineKind, NewRun, PendingRetry, Run, RunStatus, ScheduleChange, ScheduleState,
    ScheduleStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::{Clock, Scheduler};
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

/// The fake clock starts here. It is years ahead, so a `Once` or a stop date written relative
/// to it is in the future when the store checks it on save.
const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// `secs` seconds after `T0`.
fn after(secs: i64) -> DateTime<Utc> {
    t0() + TimeDelta::seconds(secs)
}

/// `after(secs)` as the store writes it.
fn ts(secs: i64) -> String {
    timestamp(after(secs))
}

/// `after(secs)` as a local date and time (`Once.at`, `Until::Date`); `secs` is whole minutes.
fn local(secs: i64) -> String {
    after(secs)
        .with_timezone(&Local)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

/// The payload of a run that serves the slot at `after(secs)`.
fn slot_payload(secs: i64, caught_up: bool) -> Value {
    let mut payload = json!({ "scheduled_for": after(secs).with_timezone(&Local).to_rfc3339() });
    if caught_up {
        payload["caught_up"] = json!(true);
    }
    payload
}

fn schedule(v: Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

/// Every `minutes`, with slots at `after(offset)` and every `minutes` from there.
fn every(minutes: u32, offset: i64, catch_up: bool) -> Schedule {
    schedule(json!({
        "rule": {"kind": "every", "minutes": minutes, "anchor": ts(offset)},
        "catch_up": catch_up,
        "until": null
    }))
}

/// Runner without a dispatcher: scheduled runs stay queued, which makes coalescing visible.
fn idle_runner(store: &Store, dir: &TempDir, bus: Bus) -> Arc<Runner> {
    Runner::new(
        store.clone(),
        bus,
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: String::new(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
        },
    )
}

/// What the scheduler announced: the dots named by `ScheduleUpdated`, the runs announced by
/// `RunUpdated`, and the `ScheduleAttention` notices as (dot, run, error).
#[derive(Default)]
struct Seen {
    schedules: Vec<String>,
    runs: Vec<String>,
    notices: Vec<(String, String, Option<String>)>,
}

/// The error of the runs the tests fail.
const DOWN: &str = "model server down";

/// A scheduler on a fake clock at `T0`, not started yet.
struct Harness {
    dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    bus: Bus,
    runner: Arc<Runner>,
    sched: Arc<Scheduler>,
    events: Receiver<RuntimeEvent>,
    seen: Seen,
    shutdown: CancellationToken,
    sleeps: u64,
}

impl Harness {
    async fn new() -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let sched = Scheduler::new(store.clone(), runner.clone(), bus.clone(), clock.clone());
        Harness {
            events: bus.subscribe(),
            dir,
            store,
            clock,
            bus,
            runner,
            sched,
            seen: Seen::default(),
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
    }

    /// Stops the scheduler and starts a new one on the same database and clock, as a restart
    /// of Dots does.
    fn restart(&mut self) {
        self.stop();
        self.sched = Scheduler::new(
            self.store.clone(),
            self.runner.clone(),
            self.bus.clone(),
            self.clock.clone(),
        );
        self.start();
    }

    /// Stops the scheduler (Dots is closed).
    fn stop(&mut self) {
        self.shutdown.cancel();
        self.shutdown = CancellationToken::new();
    }

    /// A dot with `schedule` whose row was made at `after(made)`, as the runtime makes it.
    async fn dot(&self, name: &str, schedule: Option<Schedule>, made: i64) -> Dot {
        let mut s = spec(name);
        s.schedule = schedule;
        let dot = self.store.create_dot(&s).await.unwrap();
        self.store
            .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, after(made))
            .await
            .unwrap();
        dot
    }

    fn start(&self) {
        self.sched.spawn(self.shutdown.clone());
    }

    /// Waits until the loop sleeps again after its pass, and returns when it means to wake,
    /// in seconds after `T0`.
    async fn settle(&mut self) -> i64 {
        self.sleeps += 1;
        (self.clock.nth_sleep(self.sleeps).await - t0()).num_seconds()
    }

    /// Moves the clock to `after(secs)`, which must reach the loop's wake time, and waits for
    /// the pass that follows.
    async fn at(&mut self, secs: i64) -> i64 {
        self.clock.set(after(secs));
        self.settle().await
    }

    async fn row(&self, dot: &Dot) -> ScheduleState {
        self.store
            .get_schedule_state(&dot.id)
            .await
            .unwrap()
            .unwrap()
    }

    async fn runs(&self, dot: &Dot) -> Vec<Run> {
        self.store.list_runs(Some(&dot.id), 10).await.unwrap()
    }

    /// The run the row watches.
    async fn watched(&self, dot: &Dot) -> Run {
        let id = self.row(dot).await.watch_run_id.expect("a watched run");
        self.store.get_run(&id).await.unwrap()
    }

    fn drain(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                RuntimeEvent::ScheduleUpdated { dot_id } => self.seen.schedules.push(dot_id),
                RuntimeEvent::RunUpdated { run } => self.seen.runs.push(run.id),
                RuntimeEvent::ScheduleAttention {
                    dot_id,
                    run_id,
                    error,
                } => self.seen.notices.push((dot_id, run_id, error)),
                _ => {}
            }
        }
    }

    /// The dots named by `ScheduleUpdated` and the runs announced by `RunUpdated` since the
    /// last call.
    fn events(&mut self) -> (Vec<String>, Vec<String>) {
        self.drain();
        (
            std::mem::take(&mut self.seen.schedules),
            std::mem::take(&mut self.seen.runs),
        )
    }

    /// The `ScheduleAttention` notices since the last call, as (dot, run, error).
    fn notices(&mut self) -> Vec<(String, String, Option<String>)> {
        self.drain();
        std::mem::take(&mut self.seen.notices)
    }

    /// Ends `run` with `status` at the clock's time, as the runner does, but without
    /// announcing it. A failed run gets the error `DOWN`.
    async fn end(&self, run: &str, status: RunStatus) {
        let error = (status == RunStatus::Failed).then_some(DOWN);
        self.store
            .finish_run(run, status, None, error)
            .await
            .unwrap();
        self.exec(
            "UPDATE runs SET ended_at = ? WHERE id = ?",
            &[&timestamp(self.clock.now()), run],
        )
        .await;
    }

    /// Ends the watched `run` with `status`, then reloads the scheduler and waits for the pass
    /// that judges it; returns when the loop means to wake next.
    async fn judge(&mut self, run: &str, status: RunStatus) -> i64 {
        self.end(run, status).await;
        self.sched.reload();
        self.settle().await
    }

    /// The run called a tool (an engine event of kind `tool_call`).
    async fn tool_call(&self, run: &str) {
        let call = json!({ "type": "tool_call", "id": "c1", "tool": "Bash", "input": {} });
        self.store
            .append_event(run, "tool_call", &call)
            .await
            .unwrap();
    }

    /// Runs `sql` behind the store's back, with `args` bound in order.
    async fn exec(&self, sql: &str, args: &[&str]) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(self.dir.path().join("dots.db"))
            .foreign_keys(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        let mut query = sqlx::query(sql);
        for arg in args {
            query = query.bind(*arg);
        }
        query.execute(&pool).await.unwrap();
        pool.close().await;
    }

    /// Writes the row behind the store's back, for a state the scheduler makes only later.
    async fn raw(&self, sql: &str, dot: &Dot) {
        self.exec(sql, &[&dot.id]).await;
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn a_disabled_dot_and_a_dot_without_a_row_are_left_alone() {
    let mut h = Harness::new().await;
    // Disabled, with a row from an hour ago and missed slots.
    let mut off = spec("off");
    off.schedule = Some(every(10, 0, true));
    off.enabled = false;
    let off = h.store.create_dot(&off).await.unwrap();
    h.store
        .sync_schedule_state(&off.id, off.spec.schedule.as_ref(), false, after(-3600))
        .await
        .unwrap();
    let off_row = h.row(&off).await;
    // A schedule without a row: the runtime makes the rows (at start and on each change).
    let mut s = spec("no-row");
    s.schedule = Some(every(10, -3600, true));
    let no_row = h.store.create_dot(&s).await.unwrap();

    h.start();
    assert_eq!(
        h.settle().await,
        60,
        "nothing is due: the loop sleeps 60 s at most"
    );
    assert_eq!(h.row(&off).await, off_row);
    assert!(h
        .store
        .get_schedule_state(&no_row.id)
        .await
        .unwrap()
        .is_none());
    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
    assert_eq!(h.events(), (vec![], vec![]));
}

#[tokio::test]
async fn a_slot_fires_on_time_and_a_later_one_joins_the_queued_run() {
    let mut h = Harness::new().await;
    let dot = h.dot("tick", Some(every(10, 30, true)), 0).await;
    h.start();
    assert_eq!(h.settle().await, 30, "the loop sleeps until the slot");
    h.events();

    assert_eq!(
        h.at(30).await,
        90,
        "the next slot is 10 min away: 60 s at most"
    );
    let runs = h.runs(&dot).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].trigger, TriggerKind::Schedule);
    assert_eq!(runs[0].payload, Some(slot_payload(30, false)));
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(30));
    assert_eq!(row.fired_count, 1);
    assert_eq!(row.watch_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(
        h.events(),
        (vec![dot.id.clone()], vec![runs[0].id.clone()]),
        "the row change and the new run are announced"
    );

    // The runner is idle, so the run is still queued at the next slot: the slot joins it.
    assert_eq!(h.at(630).await, 690);
    assert_eq!(h.runs(&dot).await.len(), 1);
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(630));
    assert_eq!(row.fired_count, 2);
    assert_eq!(row.watch_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(h.events(), (vec![dot.id.clone()], vec![]));
}

#[tokio::test]
async fn a_slot_up_to_59_seconds_late_is_on_time_and_60_seconds_is_missed() {
    let mut h = Harness::new().await;
    let dot = h.dot("strict", Some(every(10, 0, false)), 0).await;
    h.start();
    h.settle().await;

    assert_eq!(h.at(659).await, 719);
    let runs = h.runs(&dot).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].payload, Some(slot_payload(600, false)));

    // 60 s late without catch-up: skipped, but handled.
    h.at(1260).await;
    assert_eq!(h.runs(&dot).await.len(), 1);
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(1200));
    assert_eq!(row.fired_count, 1);
}

#[tokio::test]
async fn only_the_newest_missed_slot_is_caught_up_once() {
    let mut h = Harness::new().await;
    let dot = h.dot("hourly", Some(every(60, 0, true)), 0).await;
    // Dots was closed from T0 until 5 h 10 min later: five hourly slots were missed.
    h.clock.set(after(5 * 3600 + 600));
    h.start();
    h.settle().await;

    let runs = h.runs(&dot).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].payload, Some(slot_payload(5 * 3600, true)));
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(5 * 3600));
    assert_eq!(row.fired_count, 1);
}

#[tokio::test]
async fn a_missed_slot_is_skipped_without_catch_up_or_when_a_day_old() {
    let mut h = Harness::new().await;
    let no_catch_up = h.dot("no-catch-up", Some(every(60, 0, false)), 0).await;
    let once = schedule(json!({
        "rule": {"kind": "once", "at": local(3600)},
        "catch_up": true,
        "until": null
    }));
    let once = h.dot("once", Some(once), 0).await;
    // The once slot is 24 h and 60 s old when Dots starts.
    h.clock.set(after(25 * 3600 + 60));
    h.start();
    h.settle().await;

    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
    let row = h.row(&no_catch_up).await;
    assert_eq!(row.last_slot_at, ts(25 * 3600));
    assert_eq!((row.status, row.fired_count), (ScheduleStatus::Active, 0));
    let row = h.row(&once).await;
    assert_eq!(row.last_slot_at, ts(3600));
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 0),
        "completed (missed): no run"
    );
}

#[tokio::test]
async fn once_is_completed_when_its_slot_fired_and_its_run_is_judged() {
    let mut h = Harness::new().await;
    let once = schedule(json!({
        "rule": {"kind": "once", "at": local(60)},
        "catch_up": true,
        "until": null
    }));
    let dot = h.dot("once", Some(once), 0).await;
    h.start();
    assert_eq!(h.settle().await, 60);
    assert_eq!(h.at(60).await, 120, "no slot is left: 60 s at most");
    assert_eq!(h.runs(&dot).await.len(), 1);
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Active, 1),
        "its run may still fail and be retried"
    );
    assert_eq!(h.events().0, vec![dot.id.clone()]);

    let run = h.watched(&dot).await;
    h.judge(&run.id, RunStatus::Succeeded).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count, row.watch_run_id.clone()),
        (ScheduleStatus::Completed, 1, None)
    );
    assert_eq!(h.events().0, vec![dot.id.clone(), dot.id.clone()]);

    // A completed schedule fires nothing.
    h.at(3600).await;
    assert_eq!(h.row(&dot).await, row);
}

#[tokio::test]
async fn a_count_completes_the_schedule_after_the_last_outcome() {
    let mut h = Harness::new().await;
    let mut twice = every(1, 0, true);
    twice.until = Some(dots_core::schedule::Until::Count(2));
    let dot = h.dot("twice", Some(twice), 0).await;
    h.start();
    h.settle().await;

    h.at(60).await;
    let row = h.row(&dot).await;
    assert_eq!((row.status, row.fired_count), (ScheduleStatus::Active, 1));
    h.at(120).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Active, 2),
        "the count is reached, the outcome is not known yet"
    );
    assert_eq!(
        h.runs(&dot).await.len(),
        1,
        "the second slot joined the queued run"
    );

    // The last occurrence fails before any call: its retry still runs.
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 180);
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Active);
    h.at(180).await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    let row = h.row(&dot).await;
    assert_eq!((row.status, row.fired_count), (ScheduleStatus::Active, 2));

    h.judge(&retry.id, RunStatus::Succeeded).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 2)
    );
    h.at(600).await;
    assert_eq!(h.row(&dot).await, row);
    assert_eq!(h.runs(&dot).await.len(), 2);
}

/// The count is reached, but a retry of the last occurrence still waits: the schedule stays
/// active until no retry can follow (Task 5 judges and retries; here the row is written by
/// hand). The loop wakes for the retry.
#[tokio::test]
async fn a_reached_count_waits_for_a_pending_retry() {
    let mut h = Harness::new().await;
    let mut once_more = every(1, 0, true);
    once_more.until = Some(dots_core::schedule::Until::Count(1));
    let dot = h.dot("retrying", Some(once_more), 0).await;
    h.raw(
        &format!(
            "UPDATE schedule_state SET fired_count = 1, retry_slot_at = '{}', \
             retry_attempt = 2, retry_at = '{}' WHERE dot_id = ?",
            ts(0),
            ts(40)
        ),
        &dot,
    )
    .await;
    h.start();
    assert_eq!(h.settle().await, 40, "the loop wakes for the retry");
    let row = h.row(&dot).await;
    assert_eq!(row.status, ScheduleStatus::Active);
    assert_eq!(
        row.retry,
        Some(PendingRetry {
            slot_at: ts(0),
            attempt: 2,
            at: ts(40)
        })
    );

    // No retry left: the schedule is completed.
    let cleared = ScheduleChange {
        retry: Some(None),
        ..Default::default()
    };
    h.store
        .update_schedule_state(&dot.id, &cleared, after(0))
        .await
        .unwrap();
    h.sched.reload();
    h.settle().await;
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Completed);
    assert!(h.runs(&dot).await.is_empty());
}

#[tokio::test]
async fn a_stop_date_completes_the_schedule_after_its_last_slot() {
    let mut h = Harness::new().await;
    let mut until = every(1, 0, true);
    until.until = Some(serde_json::from_value(json!({ "date": local(60) })).unwrap());
    let dot = h.dot("until", Some(until), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Active);
    let run = h.watched(&dot).await;
    h.judge(&run.id, RunStatus::Cancelled).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.fired_count),
        (ScheduleStatus::Completed, 1)
    );
    assert_eq!(h.runs(&dot).await.len(), 1);
}

#[tokio::test]
async fn a_schedule_that_no_longer_parses_expires() {
    let mut h = Harness::new().await;
    let dot = h.dot("old", Some(Schedule::cron("0 0 9 * * *")), 0).await;
    // Only an old cron string a newer crate refuses can get here.
    h.raw(
        r#"UPDATE dots SET schedule = '{"rule":{"kind":"cron","expr":"not cron"},
           "catch_up":false,"until":null}' WHERE id = ?"#,
        &dot,
    )
    .await;
    h.start();
    assert_eq!(h.settle().await, 60);
    assert_eq!(h.row(&dot).await.status, ScheduleStatus::Expired);
    assert!(h.runs(&dot).await.is_empty());
}

#[tokio::test]
async fn a_resume_does_not_catch_up_the_slots_of_the_pause() {
    let mut h = Harness::new().await;
    let dot = h.dot("paused", Some(every(10, 0, true)), 0).await;
    h.start();
    h.settle().await;
    h.events();

    h.sched.set_paused(true);
    // Three slots pass while paused.
    h.clock.set(after(35 * 60));
    h.sched.set_paused(false);
    assert_eq!(h.settle().await, 35 * 60 + 60);

    assert!(h.runs(&dot).await.is_empty());
    let row = h.row(&dot).await;
    assert_eq!(row.last_slot_at, ts(35 * 60), "the resume time");
    assert_eq!(row.fired_count, 0);
    assert_eq!(h.events().0, vec![dot.id.clone()]);

    // The slots after the resume fire as usual.
    h.at(40 * 60).await;
    assert_eq!(h.runs(&dot).await.len(), 1);
}

/// Final review F6: a resume whose first pass fails on a store error is kept. The pass after
/// the error wait moves the rows to its time, so the slots of the pause are not caught up.
#[tokio::test]
async fn a_resume_is_kept_across_a_store_error() {
    let mut h = Harness::new().await;
    let dot = h.dot("paused", Some(every(10, 0, true)), 0).await;
    h.start();
    h.settle().await;

    h.sched.set_paused(true);
    h.clock.set(after(35 * 60));
    h.exec(
        "ALTER TABLE schedule_state RENAME TO schedule_state_away",
        &[],
    )
    .await;
    h.sched.set_paused(false);
    assert_eq!(
        h.settle().await,
        35 * 60 + 30,
        "the pass failed: it is tried again 30 s later"
    );
    h.exec(
        "ALTER TABLE schedule_state_away RENAME TO schedule_state",
        &[],
    )
    .await;

    assert_eq!(h.at(35 * 60 + 30).await, 35 * 60 + 90);
    assert!(
        h.runs(&dot).await.is_empty(),
        "the slots of the pause are not caught up"
    );
    assert_eq!(h.row(&dot).await.last_slot_at, ts(35 * 60 + 30));
}

#[tokio::test]
async fn a_scheduler_started_paused_does_nothing_until_resumed() {
    let mut h = Harness::new().await;
    let dot = h.dot("hourly", Some(every(60, 0, true)), 0).await;
    h.clock.set(after(3600 + 300));
    h.sched.set_paused(true);
    h.start();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.clock.sleeps(), 0, "no pass while paused");
    assert_eq!(h.row(&dot).await.last_slot_at, T0);

    h.sched.set_paused(false);
    h.settle().await;
    assert!(
        h.runs(&dot).await.is_empty(),
        "the missed slot is not caught up"
    );
    assert_eq!(h.row(&dot).await.last_slot_at, ts(3600 + 300));
}

#[tokio::test]
async fn reload_wakes_the_loop_for_a_new_schedule() {
    let mut h = Harness::new().await;
    h.start();
    assert_eq!(h.settle().await, 60);
    let dot = h.dot("late", Some(every(10, 20, true)), 0).await;
    h.sched.reload();
    assert_eq!(h.settle().await, 20, "the sleep follows the new slot");
    h.at(20).await;
    assert_eq!(h.runs(&dot).await.len(), 1);
}

/// Spec §6: a failure before any tool call is retried 1, 2, 4, 8 and 16 minutes after the
/// failed run ended, by a run that names the run it repeats. The sixth failed attempt sets
/// "Needs attention", announced once, and the schedule goes on.
#[tokio::test]
async fn a_failure_before_any_call_is_retried_up_to_the_sixth_attempt() {
    let mut h = Harness::new().await;
    let dot = h.dot("flaky", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let mut failed = h.watched(&dot).await;
    let mut now = 60;
    for (attempt, delay) in [(2, 60), (3, 120), (4, 240), (5, 480), (6, 960)] {
        assert_eq!(
            h.judge(&failed.id, RunStatus::Failed).await,
            now + delay.min(60),
            "the loop wakes for attempt {attempt}, or after 60 s"
        );
        let row = h.row(&dot).await;
        assert_eq!(
            row.retry,
            Some(PendingRetry {
                slot_at: ts(60),
                attempt,
                at: ts(now + delay)
            })
        );
        assert_eq!(
            row.watch_run_id.as_deref(),
            Some(failed.id.as_str()),
            "the failed run is kept until its retry starts"
        );
        assert_eq!(row.attention_run_id, None);

        now += delay;
        h.at(now).await;
        let retry = h.watched(&dot).await;
        assert_eq!(retry.trigger, TriggerKind::Schedule);
        assert_eq!(retry.retry_of.as_deref(), Some(failed.id.as_str()));
        let mut payload = slot_payload(60, false);
        payload["attempt"] = json!(attempt);
        assert_eq!(retry.payload, Some(payload));
        let row = h.row(&dot).await;
        assert_eq!(
            (row.retry, row.fired_count),
            (None, 1),
            "a retry does not count"
        );
        failed = retry;
    }
    assert!(h.notices().is_empty());

    h.judge(&failed.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.watch_run_id, row.retry),
        (ScheduleStatus::Active, None, None),
        "no seventh attempt"
    );
    assert_eq!(row.attention_run_id.as_deref(), Some(failed.id.as_str()));
    assert_eq!(
        h.notices(),
        [(dot.id.clone(), failed.id.clone(), Some(DOWN.to_string()))]
    );
    assert_eq!(h.runs(&dot).await.len(), 6);
    assert!(
        h.store.unreviewed_thread_ends().await.unwrap().is_empty(),
        "no attempt made a tool call: each is marked reviewed when judged"
    );

    // The schedule goes on: the next slot fires.
    h.at(60 + 86400).await;
    assert_eq!(h.runs(&dot).await.len(), 7);
    assert_eq!(h.row(&dot).await.fired_count, 2);
}

/// Spec §6: a run that called a tool may have changed something, so its failure is not
/// retried: the schedule needs attention at once, and keeps firing. A second failure moves
/// the flag without a second notice; a scheduled run that succeeds clears it.
#[tokio::test]
async fn a_failure_after_a_tool_call_needs_attention_until_a_scheduled_run_succeeds() {
    let mut h = Harness::new().await;
    let dot = h.dot("careful", Some(every(10, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.tool_call(&first.id).await;
    h.events();
    h.judge(&first.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.retry), (None, None), "not retried");
    assert_eq!(row.attention_run_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(h.events(), (vec![dot.id.clone()], vec![]));
    assert_eq!(
        h.notices(),
        [(dot.id.clone(), first.id.clone(), Some(DOWN.to_string()))]
    );

    h.at(660).await;
    let second = h.watched(&dot).await;
    assert_ne!(second.id, first.id);
    h.tool_call(&second.id).await;
    h.judge(&second.id, RunStatus::Failed).await;
    assert_eq!(
        h.row(&dot).await.attention_run_id.as_deref(),
        Some(second.id.as_str())
    );
    assert!(h.notices().is_empty(), "the flag was set already");

    h.at(1260).await;
    let third = h.watched(&dot).await;
    h.events();
    h.judge(&third.id, RunStatus::Succeeded).await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.attention_run_id), (None, None));
    assert_eq!(h.events().0, vec![dot.id.clone()]);
    assert!(h.notices().is_empty(), "clearing the flag is quiet");
}

/// Final review F1: a run still running when the next slot fires is no longer watched, so the
/// scheduler never judges it. When it then fails before any tool call, it is a run to review.
/// A scheduled run that failed before any tool call and that the scheduler judged into a
/// retry is marked reviewed with that write and is not listed. One that failed after a tool
/// call may have changed something: it needs attention and stays a run to review.
#[tokio::test]
async fn an_overrun_run_that_fails_is_for_review_and_a_judged_one_is_not() {
    let mut h = Harness::new().await;
    let dot = h.dot("slow", Some(every(10, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let overrun = h.watched(&dot).await;
    assert!(h.store.claim_run(&overrun.id).await.unwrap());

    // The next slot comes while the run is still running: a new run is queued and watched.
    h.at(660).await;
    let next = h.watched(&dot).await;
    assert_ne!(next.id, overrun.id);
    h.end(&overrun.id, RunStatus::Failed).await;
    h.sched.reload();
    h.settle().await;
    let review = |h: &Harness| {
        let store = h.store.clone();
        async move {
            let runs = store.unreviewed_thread_ends().await.unwrap();
            runs.into_iter().map(|r| r.id).collect::<Vec<_>>()
        }
    };
    assert_eq!(
        review(&h).await,
        [overrun.id.as_str()],
        "never judged: listed"
    );

    // The watched run fails before any call: judged into a retry, and marked reviewed.
    h.judge(&next.id, RunStatus::Failed).await;
    assert!(h.row(&dot).await.retry.is_some());
    assert!(h
        .store
        .get_run(&next.id)
        .await
        .unwrap()
        .reviewed_at
        .is_some());
    assert_eq!(review(&h).await, [overrun.id.as_str()]);

    // Its retry fails after a tool call: judged into "Needs attention", and still for review.
    h.at(720).await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(next.id.as_str()));
    h.tool_call(&retry.id).await;
    h.judge(&retry.id, RunStatus::Failed).await;
    assert_eq!(
        h.row(&dot).await.attention_run_id.as_deref(),
        Some(retry.id.as_str())
    );
    assert_eq!(
        h.store.get_run(&retry.id).await.unwrap().reviewed_at,
        None,
        "it may have changed something"
    );
    let mut listed = review(&h).await;
    listed.sort();
    let mut want = vec![overrun.id.clone(), retry.id.clone()];
    want.sort();
    assert_eq!(listed, want);
}

/// Final review F1, follow-up: a manual run that a retry joined is judged as the occurrence's
/// run, but it is the person's run: its failure is retried, and it stays a run to review.
#[tokio::test]
async fn a_joined_manual_run_that_fails_stays_for_review() {
    let mut h = Harness::new().await;
    let dot = h.dot("joined", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.judge(&first.id, RunStatus::Failed).await;
    let manual = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    h.at(120).await;
    assert_eq!(
        h.row(&dot).await.watch_run_id.as_deref(),
        Some(manual.id.as_str())
    );

    h.judge(&manual.id, RunStatus::Failed).await;
    assert!(h.row(&dot).await.retry.is_some(), "judged into a retry");
    assert_eq!(h.store.get_run(&manual.id).await.unwrap().reviewed_at, None);
    assert!(
        h.store
            .get_run(&first.id)
            .await
            .unwrap()
            .reviewed_at
            .is_some(),
        "the scheduled run is marked"
    );
    let listed: Vec<String> = h
        .store
        .unreviewed_thread_ends()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(listed, [manual.id.as_str()]);
}

/// Spec §6 and §10: a cancelled watched run, and a watched run that was deleted (its thread
/// discarded), end the occurrence without a retry or "Needs attention".
#[tokio::test]
async fn a_cancelled_or_deleted_watched_run_ends_the_occurrence_quietly() {
    let mut h = Harness::new().await;
    let mut once = every(10, 60, true);
    once.until = Some(dots_core::schedule::Until::Count(1));
    let cancelled = h.dot("cancelled", Some(once.clone()), 0).await;
    let deleted = h.dot("deleted", Some(once), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let run = h.watched(&cancelled).await;
    h.end(&run.id, RunStatus::Cancelled).await;
    let gone = h.watched(&deleted).await;
    h.exec("DELETE FROM runs WHERE id = ?", &[&gone.id]).await;
    h.sched.reload();
    h.settle().await;

    for dot in [&cancelled, &deleted] {
        let row = h.row(dot).await;
        assert_eq!(
            (
                row.status,
                row.watch_run_id,
                row.retry,
                row.attention_run_id
            ),
            (ScheduleStatus::Completed, None, None, None),
            "{}",
            dot.spec.name
        );
    }
    assert!(h.notices().is_empty());
    assert_eq!(h.runs(&cancelled).await.len(), 1);
    assert!(h.runs(&deleted).await.is_empty());
}

/// Spec §6: a run parked on an approval has not ended. When the decision resumes it, the
/// parked run is closed and its resume child decides the occurrence. The parked run called a
/// tool, so the child's failure is not retried even though the child made no call.
#[tokio::test]
async fn the_resume_child_of_a_parked_run_takes_over_the_watch() {
    let mut h = Harness::new().await;
    let dot = h.dot("asks", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let parent = h.watched(&dot).await;
    h.tool_call(&parent.id).await;
    h.store
        .set_status(&parent.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    h.sched.reload();
    h.settle().await;
    assert_eq!(
        h.row(&dot).await.watch_run_id.as_deref(),
        Some(parent.id.as_str()),
        "a parked run has not ended"
    );

    let mut resume = NewRun::new(&dot.id, TriggerKind::Resume);
    resume.parent_run_id = Some(parent.id.clone());
    resume.root_run_id = Some(parent.id.clone());
    let child = h
        .store
        .resume_parked(&parent.id, &[], &[], &[], Some(&resume))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        h.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded,
        "the resume closes the parked run"
    );
    h.sched.reload();
    h.settle().await;
    let row = h.row(&dot).await;
    assert_eq!(row.watch_run_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(row.attention_run_id, None);

    h.judge(&child.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.retry), (None, None));
    assert_eq!(row.attention_run_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(h.notices().len(), 1);
}

/// Spec §6: a retry that comes due while the dot has a queued run joins that run, which is
/// watched from then on.
#[tokio::test]
async fn a_due_retry_joins_the_queued_run() {
    let mut h = Harness::new().await;
    let dot = h.dot("joined", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 120);
    let manual = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    h.events();

    h.at(120).await;
    let row = h.row(&dot).await;
    assert_eq!(row.watch_run_id.as_deref(), Some(manual.id.as_str()));
    assert_eq!((row.retry, row.fired_count), (None, 1));
    assert_eq!(h.runs(&dot).await.len(), 2);
    assert_eq!(
        h.events(),
        (vec![dot.id.clone()], vec![]),
        "no run was added"
    );
}

/// Spec §5 "Fire": a slot that fires while a retry waits drops the retry; the newer
/// occurrence serves.
#[tokio::test]
async fn a_slot_that_fires_drops_a_waiting_retry() {
    let mut h = Harness::new().await;
    let dot = h.dot("busy", Some(every(10, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.at(620).await, 660);
    assert_eq!(
        h.judge(&first.id, RunStatus::Failed).await,
        660,
        "the slot comes before the retry"
    );
    assert_eq!(h.row(&dot).await.retry.unwrap().at, ts(680));

    h.at(660).await;
    let second = h.watched(&dot).await;
    assert_ne!(second.id, first.id);
    let mut payload = slot_payload(660, false);
    payload["attempt"] = json!(2);
    assert_eq!(
        second.payload,
        Some(payload),
        "the slot's run carries the dropped retry's attempt"
    );
    assert_eq!(second.retry_of, None);
    let row = h.row(&dot).await;
    assert_eq!((row.retry, row.fired_count), (None, 2));
    h.at(720).await;
    assert_eq!(
        h.runs(&dot).await.len(),
        2,
        "the dropped retry never starts"
    );
}

/// The Inbox offers "Run now" for a schedule that needs attention (spec §9), and the smoke
/// test expects it to clear the flag (spec §12): a later manual or scheduled run of the dot
/// that succeeds clears it.
#[tokio::test]
async fn a_run_now_that_succeeds_clears_the_attention() {
    let mut h = Harness::new().await;
    let dot = h.dot("fixed", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.tool_call(&first.id).await;
    h.judge(&first.id, RunStatus::Failed).await;
    assert!(h.row(&dot).await.attention_run_id.is_some());
    assert_eq!(h.notices().len(), 1);

    // A chat run that succeeds says nothing about the schedule.
    let chat = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    h.judge(&chat.id, RunStatus::Succeeded).await;
    assert!(h.row(&dot).await.attention_run_id.is_some());

    let manual = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    h.events();
    h.judge(&manual.id, RunStatus::Succeeded).await;
    assert_eq!(h.row(&dot).await.attention_run_id, None);
    assert_eq!(h.events().0, vec![dot.id.clone()]);
    assert!(h.notices().is_empty());
}

/// Final review F2: a successful Run now clears the flag whatever the row's status and
/// whether the dot is enabled: a `Once` whose occurrence ended in attention is completed at
/// once, and the Inbox still lists the flag of a disabled dot.
#[tokio::test]
async fn a_run_now_clears_the_flag_of_a_completed_once_and_of_a_disabled_dot() {
    let mut h = Harness::new().await;
    let once = schedule(json!({
        "rule": {"kind": "once", "at": local(60)},
        "catch_up": true,
        "until": null
    }));
    let once = h.dot("once", Some(once), 0).await;
    let off = h.dot("off", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    for dot in [&once, &off] {
        let run = h.watched(dot).await;
        h.tool_call(&run.id).await;
        h.judge(&run.id, RunStatus::Failed).await;
        assert!(h.row(dot).await.attention_run_id.is_some());
    }
    assert_eq!(h.row(&once).await.status, ScheduleStatus::Completed);
    h.raw("UPDATE dots SET enabled = 0 WHERE id = ?", &off)
        .await;
    h.sched.reload();
    h.settle().await;
    assert_eq!(h.notices().len(), 2);
    h.events();

    for dot in [&once, &off] {
        let manual = h
            .store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        h.judge(&manual.id, RunStatus::Succeeded).await;
        assert_eq!(h.row(dot).await.attention_run_id, None, "{}", dot.spec.name);
        assert_eq!(h.events().0, vec![dot.id.clone()], "{}", dot.spec.name);
    }
    assert_eq!(h.row(&once).await.status, ScheduleStatus::Completed);
    assert!(h.notices().is_empty());
}

/// Final review F2: a Run now that parks on an approval counts once the decision resumes it
/// and its resume run succeeds. A webhook run that succeeds does not clear the flag.
#[tokio::test]
async fn a_run_now_that_parks_clears_the_flag_once_its_resume_succeeds() {
    let mut h = Harness::new().await;
    let dot = h.dot("asks", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let failed = h.watched(&dot).await;
    h.tool_call(&failed.id).await;
    h.judge(&failed.id, RunStatus::Failed).await;
    assert!(h.row(&dot).await.attention_run_id.is_some());

    let hook = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();
    h.judge(&hook.id, RunStatus::Succeeded).await;
    assert!(
        h.row(&dot).await.attention_run_id.is_some(),
        "a webhook run does not clear"
    );

    let manual = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    h.store
        .set_status(&manual.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    let mut resume = NewRun::new(&dot.id, TriggerKind::Resume);
    resume.parent_run_id = Some(manual.id.clone());
    resume.root_run_id = Some(manual.id.clone());
    let child = h
        .store
        .resume_parked(&manual.id, &[], &[], &[], Some(&resume))
        .await
        .unwrap()
        .unwrap();
    h.sched.reload();
    h.settle().await;
    assert!(
        h.row(&dot).await.attention_run_id.is_some(),
        "the parked run closed by its decision has not succeeded"
    );
    h.events();

    h.judge(&child.id, RunStatus::Succeeded).await;
    assert_eq!(h.row(&dot).await.attention_run_id, None);
    assert_eq!(h.events().0, vec![dot.id.clone()]);
}

/// Spec §5 "Pause": no retry starts while schedules are paused; one that came due during the
/// pause starts on the first pass after the resume.
#[tokio::test]
async fn a_retry_that_came_due_during_a_pause_starts_on_resume() {
    let mut h = Harness::new().await;
    let dot = h.dot("paused", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.judge(&first.id, RunStatus::Failed).await;

    h.sched.set_paused(true);
    h.clock.set(after(600));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.runs(&dot).await.len(), 1, "nothing starts while paused");
    h.sched.set_paused(false);
    h.settle().await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    let row = h.row(&dot).await;
    assert_eq!((row.retry, row.last_slot_at), (None, ts(600)));
}

/// Spec §6 "Watching": the row remembers the retry, so a restart while it waits keeps it.
#[tokio::test]
async fn a_restart_while_a_retry_waits_keeps_it() {
    let mut h = Harness::new().await;
    let dot = h.dot("restarted", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 120);

    h.restart();
    assert_eq!(h.settle().await, 120, "the new loop wakes for the retry");
    h.at(120).await;
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    assert_eq!(retry.payload.unwrap()["attempt"], json!(2));
}

/// Final review F4: without catch-up, a retry more than `ON_TIME` overdue (Dots was closed) is
/// dropped, and the occurrence ends as failed: "Needs attention", announced once. A retry less
/// late than that starts.
#[tokio::test]
async fn without_catch_up_a_retry_overdue_by_a_minute_is_dropped_into_attention() {
    let mut h = Harness::new().await;
    let dot = h.dot("strict", Some(every(1440, 60, false)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    assert_eq!(h.judge(&first.id, RunStatus::Failed).await, 120);

    // Closed until 59 s after the retry was due: it starts.
    h.stop();
    h.clock.set(after(179));
    h.restart();
    h.settle().await;
    let second = h.watched(&dot).await;
    assert_eq!(second.retry_of.as_deref(), Some(first.id.as_str()));
    assert_eq!(h.judge(&second.id, RunStatus::Failed).await, 239);
    assert_eq!(h.row(&dot).await.retry.unwrap().at, ts(299));
    h.events();

    // Closed until 60 s after the next retry was due: it is dropped.
    h.stop();
    h.clock.set(after(359));
    h.restart();
    assert_eq!(h.settle().await, 419);
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.watch_run_id, row.retry),
        (ScheduleStatus::Active, None, None)
    );
    assert_eq!(row.attention_run_id.as_deref(), Some(second.id.as_str()));
    assert_eq!(h.runs(&dot).await.len(), 2, "no third attempt");
    assert_eq!(h.events(), (vec![dot.id.clone()], vec![]));
    assert_eq!(
        h.notices(),
        [(dot.id.clone(), second.id.clone(), Some(DOWN.to_string()))]
    );
}

/// Final review F4: a retry whose slot is `CATCH_UP_WINDOW` old or older is dropped, also with
/// catch-up on, and the occurrence ends in "Needs attention".
#[tokio::test]
async fn a_retry_of_a_slot_a_day_old_is_dropped_into_attention() {
    let mut h = Harness::new().await;
    let dot = h.dot("weekly", Some(every(7 * 1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.judge(&first.id, RunStatus::Failed).await;
    h.events();

    h.stop();
    h.clock.set(after(60 + 86400));
    h.restart();
    h.settle().await;
    let row = h.row(&dot).await;
    assert_eq!((row.watch_run_id, row.retry), (None, None));
    assert_eq!(row.attention_run_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(h.runs(&dot).await.len(), 1);
    assert_eq!(h.events(), (vec![dot.id.clone()], vec![]));
    assert_eq!(h.notices().len(), 1);
}

/// Spec §6 "Watching": a watched run that ended while Dots was closed is judged by the first
/// pass after the start; its retry, already due, starts at once.
#[tokio::test]
async fn a_watched_run_that_ended_while_dots_was_closed_is_judged_at_start() {
    let mut h = Harness::new().await;
    let dot = h.dot("closed", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.stop();
    h.clock.set(after(300));
    h.end(&first.id, RunStatus::Failed).await;

    h.clock.set(after(3600));
    h.restart();
    assert_eq!(
        h.settle().await,
        360,
        "the retry was due a minute after the run ended"
    );
    assert_eq!(h.settle().await, 3660);
    let retry = h.watched(&dot).await;
    assert_eq!(retry.retry_of.as_deref(), Some(first.id.as_str()));
    assert_eq!(h.row(&dot).await.retry, None);
}

/// Spec §10: a dot deleted while a retry waits takes its row along; the retry never starts.
#[tokio::test]
async fn a_dot_deleted_while_a_retry_waits_is_never_retried() {
    let mut h = Harness::new().await;
    let dot = h.dot("deleted", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let first = h.watched(&dot).await;
    h.judge(&first.id, RunStatus::Failed).await;

    h.store.delete_dot_if_idle(&dot.id).await.unwrap();
    h.sched.reload();
    h.settle().await;
    assert!(h.store.get_schedule_state(&dot.id).await.unwrap().is_none());
    h.at(120).await;
    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
}

/// Spec §5: the end of a watched run wakes the loop at once, without a reload; the end of
/// another run does not.
#[tokio::test]
async fn the_end_of_a_watched_run_wakes_the_loop() {
    let mut h = Harness::new().await;
    let dot = h.dot("woken", Some(every(1440, 60, true)), 0).await;
    h.start();
    h.settle().await;
    h.at(60).await;
    let watched = h.watched(&dot).await;

    let other = h
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let other = h
        .store
        .finish_run(&other.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    h.bus.send(RuntimeEvent::RunUpdated { run: other }).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.clock.sleeps(), h.sleeps, "not a watched run: no pass");

    h.end(&watched.id, RunStatus::Failed).await;
    let ended = h.store.get_run(&watched.id).await.unwrap();
    h.bus.send(RuntimeEvent::RunUpdated { run: ended }).unwrap();
    assert_eq!(h.settle().await, 120);
    assert_eq!(h.row(&dot).await.retry.unwrap().attempt, 2);
}

/// A clock that runs a hook once, inside its `n`th reading of the time after `arm`: a test
/// changes the database there, between a pass's reads and its writes. The hook runs on a
/// thread of its own and is waited for, so the pass stands still meanwhile.
struct HookClock {
    inner: Arc<FakeClock>,
    /// The reading the hook runs in, counted down, and the hook.
    hook: std::sync::Mutex<Option<(u32, Hook)>>,
}

type Hook = Box<dyn FnOnce() + Send>;

impl std::fmt::Debug for HookClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookClock")
            .field("inner", &self.inner)
            .finish()
    }
}

impl HookClock {
    fn arm(&self, n: u32, hook: impl FnOnce() + Send + 'static) {
        *self.hook.lock().unwrap() = Some((n, Box::new(hook)));
    }
}

#[async_trait::async_trait]
impl Clock for HookClock {
    fn now(&self) -> DateTime<Utc> {
        let hook = {
            let mut armed = self.hook.lock().unwrap();
            match armed.as_mut() {
                Some((n, _)) if *n > 1 => {
                    *n -= 1;
                    None
                }
                Some(_) => armed.take().map(|(_, hook)| hook),
                None => None,
            }
        };
        if let Some(hook) = hook {
            std::thread::spawn(hook).join().unwrap();
        }
        self.inner.now()
    }

    async fn sleep_until(&self, deadline: DateTime<Utc>) {
        self.inner.sleep_until(deadline).await
    }
}

/// Task 4 review: a pass writes a row's status only if the row was not reset after the pass
/// read it. Here the person saves a schedule that parses while a pass is about to mark the
/// old one `expired`: the reset row stays active, and nothing is announced for it.
#[tokio::test]
async fn a_row_reset_during_a_pass_is_not_overwritten() {
    let mut h = Harness::new().await;
    let dot = h.dot("old", Some(Schedule::cron("0 0 9 * * *")), -60).await;
    h.raw(
        r#"UPDATE dots SET schedule = '{"rule":{"kind":"cron","expr":"not cron"},
           "catch_up":false,"until":null}' WHERE id = ?"#,
        &dot,
    )
    .await;
    let clock = Arc::new(HookClock {
        inner: h.clock.clone(),
        hook: std::sync::Mutex::new(None),
    });
    h.sched = Scheduler::new(
        h.store.clone(),
        h.runner.clone(),
        h.bus.clone(),
        clock.clone(),
    );

    // The first reading of the time starts the pass; the second is the expiry's, after the
    // pass read the rows and the dots.
    let fixed = every(1440, 60, true);
    let (db, id, saved) = (h.dir.path().join("dots.db"), dot.id.clone(), fixed.clone());
    clock.arm(2, move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let store = Store::open(&db).await.unwrap();
                let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(&db);
                let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
                sqlx::query("UPDATE dots SET schedule = ? WHERE id = ?")
                    .bind(serde_json::to_string(&saved).unwrap())
                    .bind(&id)
                    .execute(&pool)
                    .await
                    .unwrap();
                pool.close().await;
                assert!(store
                    .sync_schedule_state(&id, Some(&saved), false, after(0))
                    .await
                    .unwrap());
            });
    });
    h.start();
    h.settle().await;
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.rule_key.clone(), row.last_slot_at.clone()),
        (ScheduleStatus::Active, fixed.rule_key(), ts(0)),
        "the reset row is not expired"
    );
    assert_eq!(
        h.events().0,
        Vec::<String>::new(),
        "no write, no announcement"
    );

    // The reload that follows the save brings a pass over the new schedule.
    h.sched.reload();
    assert_eq!(h.settle().await, 60);
    assert_eq!(h.row(&dot).await, row);
    h.at(60).await;
    assert_eq!(h.row(&dot).await.fired_count, 1);
}

/// Spec §6, the carry rule: a slot that drops a pending retry carries its attempt. A schedule
/// that fires more often than its retries come, and keeps failing before any tool call,
/// needs attention after its sixth failed attempt in a row, announced once. A slot with no
/// pending retry, as after a success, is attempt 1 again.
#[tokio::test]
async fn a_frequent_schedule_that_keeps_failing_needs_attention_after_six_attempts() {
    let mut h = Harness::new().await;
    let dot = h.dot("frequent", Some(every(2, 60, true)), 0).await;
    h.start();
    h.settle().await;
    // (time, attempt, wake after the failure): slots every 120 s from 60; the retry after
    // attempt 1 comes before the next slot, every later one is dropped by a slot. The loop
    // wakes at the slot, the retry or after 60 s, whichever is first.
    let attempts = [
        (60, 1, 120),
        (120, 2, 180),
        (180, 3, 240),
        (300, 4, 360),
        (420, 5, 480),
        (540, 6, 600),
    ];
    let mut failed = None;
    for (at, attempt, wake) in attempts {
        h.at(at).await;
        let run = h.watched(&dot).await;
        let carried = run.payload.as_ref().and_then(|p| p.get("attempt"));
        assert_eq!(
            carried.and_then(Value::as_u64),
            (attempt > 1).then_some(attempt),
            "the attempt of the run at {at}"
        );
        assert_eq!(run.retry_of.is_some(), at == 120, "a retry run only at 120");
        assert!(h.notices().is_empty());
        assert_eq!(h.judge(&run.id, RunStatus::Failed).await, wake);
        failed = Some(run);
    }
    let failed = failed.unwrap();
    let row = h.row(&dot).await;
    assert_eq!(
        (row.status, row.watch_run_id, row.retry),
        (ScheduleStatus::Active, None, None),
        "no seventh attempt"
    );
    assert_eq!(row.attention_run_id.as_deref(), Some(failed.id.as_str()));
    assert_eq!(
        h.notices(),
        [(dot.id.clone(), failed.id.clone(), Some(DOWN.to_string()))]
    );

    // No retry waits: the next slot is attempt 1, and its success clears the flag.
    h.at(660).await;
    let ok = h.watched(&dot).await;
    assert_eq!(ok.payload, Some(slot_payload(660, false)));
    h.judge(&ok.id, RunStatus::Succeeded).await;
    assert_eq!(h.row(&dot).await.attention_run_id, None);

    // After the success, a failure starts the count again: attempt 1, retried a minute later.
    h.at(780).await;
    let again = h.watched(&dot).await;
    assert_eq!(again.payload, Some(slot_payload(780, false)));
    h.judge(&again.id, RunStatus::Failed).await;
    let row = h.row(&dot).await;
    assert_eq!(
        row.retry,
        Some(PendingRetry {
            slot_at: ts(780),
            attempt: 2,
            at: ts(840)
        })
    );
    assert!(h.notices().is_empty(), "announced once");
}
