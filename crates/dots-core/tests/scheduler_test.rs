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
    Dot, EngineKind, PendingRetry, Run, ScheduleChange, ScheduleState, ScheduleStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::Schedule;
use dots_core::scheduler::Scheduler;
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
        },
    )
}

/// A scheduler on a fake clock at `T0`, not started yet.
struct Harness {
    dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    sched: Arc<Scheduler>,
    events: Receiver<RuntimeEvent>,
    shutdown: CancellationToken,
    sleeps: u64,
}

impl Harness {
    async fn new() -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let sched = Scheduler::new(store.clone(), runner, bus.clone(), clock.clone());
        Harness {
            events: bus.subscribe(),
            dir,
            store,
            clock,
            sched,
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
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

    /// The dots named by `ScheduleUpdated` and the runs announced by `RunUpdated` since the
    /// last call.
    fn events(&mut self) -> (Vec<String>, Vec<String>) {
        let (mut schedules, mut runs) = (Vec::new(), Vec::new());
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                RuntimeEvent::ScheduleUpdated { dot_id } => schedules.push(dot_id),
                RuntimeEvent::RunUpdated { run } => runs.push(run.id),
                _ => {}
            }
        }
        (schedules, runs)
    }

    /// Writes the row behind the store's back, for a state only Task 5's judging makes.
    async fn raw(&self, sql: &str, dot: &Dot) {
        let pool = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            self.dir.path().join("dots.db").display()
        ))
        .await
        .unwrap();
        sqlx::query(sql).bind(&dot.id).execute(&pool).await.unwrap();
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
async fn once_is_completed_when_its_slot_fired() {
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
        (ScheduleStatus::Completed, 1)
    );
    assert_eq!(h.events().0, vec![dot.id.clone(), dot.id.clone()]);

    // A completed schedule fires nothing.
    h.at(3600).await;
    assert_eq!(h.row(&dot).await, row);
}

#[tokio::test]
async fn a_count_completes_the_schedule_when_it_is_reached() {
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
        (ScheduleStatus::Completed, 2)
    );

    h.at(300).await;
    assert_eq!(h.row(&dot).await, row);
    assert_eq!(
        h.runs(&dot).await.len(),
        1,
        "the second slot joined the queued run"
    );
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
