//! The delivery sweeper (delivery tracking spec section 3): a run that waited 30 minutes
//! without reaching its dot is marked not delivered, with the reason it waits; never one held
//! for the usage limit.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::tz::TestTz;
use common::{spec, temp_store};
use dots_core::delivery::{local_day_bounds, DeliverySweeper};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::limits::{HoldShares, LimitReading, Limits};
use dots_core::model::{Delivery, Dot, EngineKind, NewRun, Run, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::util::timestamp;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T12:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

struct Env {
    dir: TempDir,
    store: Store,
    bus: Bus,
    clock: Arc<FakeClock>,
    runner: Arc<Runner>,
    sweeper: Arc<DeliverySweeper>,
}

/// A runner with no dispatcher, held as during an update, and a sweeper on a fake clock at t0.
async fn env() -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let clock = FakeClock::new(t0());
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(
        EngineKind::Claude,
        Arc::new(ScriptedEngine::new(vec![Step::Emit(
            EngineEvent::Finished {
                summary: "done".into(),
            },
        )])),
    );
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 4,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    );
    runner.set_held(true).await;
    let sweeper = Arc::new(DeliverySweeper::new(
        store.clone(),
        runner.clone(),
        bus.clone(),
        clock.clone(),
    ));
    Env {
        dir,
        store,
        bus,
        clock,
        runner,
        sweeper,
    }
}

impl Env {
    async fn dot(&self, name: &str) -> Dot {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    /// A run queued `ago` before t0.
    async fn queued(&self, dot: &Dot, trigger: TriggerKind, ago: TimeDelta) -> Run {
        let run = self
            .store
            .create_run(&NewRun::new(&dot.id, trigger))
            .await
            .unwrap();
        self.store
            .set_run_queued_at(&run.id, &timestamp(t0() - ago))
            .await
            .unwrap();
        self.store.get_run(&run.id).await.unwrap()
    }
}

#[tokio::test]
async fn the_sweeper_marks_a_message_waiting_30_minutes() {
    let e = env().await;
    let dot = e.dot("a").await;
    let old = e
        .queued(&dot, TriggerKind::Chat, TimeDelta::minutes(31))
        .await;
    let young = e
        .queued(&dot, TriggerKind::Chat, TimeDelta::minutes(29))
        .await;

    let marked = e.sweeper.sweep().await.unwrap();
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].id, old.id);
    assert_eq!(
        marked[0].undelivered_reason.as_deref(),
        Some("Dots is updating")
    );
    assert_eq!(marked[0].undelivered_at, Some(timestamp(t0())));
    assert_eq!(marked[0].delivery, Some(Delivery::NotDelivered));
    let young = e.store.get_run(&young.id).await.unwrap();
    assert_eq!(young.undelivered_at, None);

    assert!(e.sweeper.sweep().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_sweeper_never_marks_a_usage_hold() {
    let e = env().await;
    let limits = Limits::load(
        e.store.clone(),
        e.bus.clone(),
        e.clock.clone(),
        HoldShares {
            enabled: true,
            five_hour: 60,
            weekly: 70,
        },
    )
    .await
    .unwrap();
    limits
        .record(
            "allowed",
            vec![LimitReading {
                window: "five_hour".into(),
                utilization: 0.7,
                resets_at: timestamp(t0() + TimeDelta::hours(1)),
                status: "allowed".into(),
                read_at: timestamp(t0()),
            }],
        )
        .await
        .unwrap();
    e.runner.set_limits(limits);
    let dot = e.dot("a").await;
    let held = e
        .queued(&dot, TriggerKind::Schedule, TimeDelta::hours(5))
        .await;

    assert!(e.sweeper.sweep().await.unwrap().is_empty());
    let held = e.store.get_run(&held.id).await.unwrap();
    assert_eq!(held.undelivered_at, None);
    assert_eq!(held.delivery, Some(Delivery::Waiting));
}

#[tokio::test]
async fn the_sweeper_skips_delivered_and_dismissed_runs() {
    let e = env().await;
    let (a, b) = (e.dot("a").await, e.dot("b").await);
    let delivered = e.queued(&a, TriggerKind::Chat, TimeDelta::hours(1)).await;
    assert!(e.store.claim_run(&delivered.id).await.unwrap());
    e.store
        .mark_delivered(&delivered.id, t0() - TimeDelta::minutes(50))
        .await
        .unwrap();
    e.store
        .requeue_run(&delivered.id, &delivered)
        .await
        .unwrap();
    let dismissed = e.queued(&b, TriggerKind::Chat, TimeDelta::hours(1)).await;
    e.store
        .dismiss_run(&dismissed.id, t0() - TimeDelta::minutes(40))
        .await
        .unwrap();

    assert!(e.sweeper.sweep().await.unwrap().is_empty());
    for id in [&delivered.id, &dismissed.id] {
        assert_eq!(e.store.get_run(id).await.unwrap().undelivered_at, None);
    }
}

#[tokio::test]
async fn the_sweeper_runs_on_the_clock() {
    let e = env().await;
    let dot = e.dot("a").await;
    let old = e
        .queued(&dot, TriggerKind::Chat, TimeDelta::minutes(31))
        .await;
    let mut rx = e.bus.subscribe();
    let shutdown = CancellationToken::new();
    let task = e.sweeper.clone().spawn(shutdown.clone());

    assert_eq!(e.clock.nth_sleep(1).await, t0() + TimeDelta::seconds(30));
    e.clock.set(t0() + TimeDelta::seconds(30));
    let run = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let RuntimeEvent::RunUpdated { run } = rx.recv().await.unwrap() {
                if run.id == old.id {
                    return run;
                }
            }
        }
    })
    .await
    .expect("the sweeper did not mark the run");
    assert_eq!(run.undelivered_reason.as_deref(), Some("Dots is updating"));

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the sweeper did not stop")
        .unwrap();
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn local_day_bounds_follow_local_midnight() {
    // TestTz is UTC+1 in November: local midnight is 23:00 UTC.
    let (start, end) = local_day_bounds(utc("2026-11-03T22:59:59Z"), &TestTz);
    assert_eq!(start, utc("2026-11-02T23:00:00Z"));
    assert_eq!(end, utc("2026-11-03T23:00:00Z"));

    let (start, end) = local_day_bounds(utc("2026-11-03T23:00:00Z"), &TestTz);
    assert_eq!(start, utc("2026-11-03T23:00:00Z"));
    assert_eq!(end, utc("2026-11-04T23:00:00Z"));

    // Spring forward: the clocks go from 02:00 to 03:00, so the day is 23 hours long.
    let (start, end) = local_day_bounds(utc("2026-03-29T12:00:00Z"), &TestTz);
    assert_eq!(start, utc("2026-03-28T23:00:00Z"));
    assert_eq!(end, utc("2026-03-29T22:00:00Z"));
    assert_eq!(end - start, TimeDelta::hours(23));
}

/// A zone whose clocks skip midnight: on 2026-11-03 they go from 00:00 to 01:00 (UTC+1 to
/// UTC+2 at 2026-11-02T23:00Z).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MidnightGapTz;

impl chrono::TimeZone for MidnightGapTz {
    type Offset = chrono::FixedOffset;

    fn from_offset(_: &chrono::FixedOffset) -> Self {
        MidnightGapTz
    }

    fn offset_from_local_date(
        &self,
        local: &chrono::NaiveDate,
    ) -> chrono::LocalResult<chrono::FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(
        &self,
        local: &chrono::NaiveDateTime,
    ) -> chrono::LocalResult<chrono::FixedOffset> {
        let fits: Vec<chrono::FixedOffset> = [7200, 3600]
            .into_iter()
            .map(|s| chrono::FixedOffset::east_opt(s).unwrap())
            .filter(|o| {
                let at = *local - TimeDelta::seconds(o.local_minus_utc() as i64);
                self.offset_from_utc_datetime(&at) == *o
            })
            .collect();
        match fits[..] {
            [o] => chrono::LocalResult::Single(o),
            [a, b] => chrono::LocalResult::Ambiguous(a, b),
            _ => chrono::LocalResult::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &chrono::NaiveDate) -> chrono::FixedOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, at: &chrono::NaiveDateTime) -> chrono::FixedOffset {
        let change = utc("2026-11-02T23:00:00Z").naive_utc();
        chrono::FixedOffset::east_opt(if *at >= change { 7200 } else { 3600 }).unwrap()
    }
}

#[test]
fn a_day_whose_midnight_is_skipped_starts_at_the_first_valid_instant() {
    let (start, end) = local_day_bounds(utc("2026-11-03T12:00:00Z"), &MidnightGapTz);
    // Local 00:00 on 3 November does not exist; the day starts at 01:00 local, the change.
    assert_eq!(start, utc("2026-11-02T23:00:00Z"));
    assert_eq!(end, utc("2026-11-03T22:00:00Z"));
}

#[tokio::test]
async fn today_counts_a_run_queued_at_local_midnight_and_one_just_before_the_next() {
    let e = env().await;
    let dot = e.dot("a").await;
    let (start, end) = local_day_bounds(utc("2026-11-03T12:00:00Z"), &TestTz);
    // Local 00:00:00.000 and 23:59:59.999 on 3 November count; the instants just outside do not.
    for at in [
        "2026-11-02T22:59:59.999Z",
        "2026-11-02T23:00:00.000Z",
        "2026-11-03T22:59:59.999Z",
        "2026-11-03T23:00:00.000Z",
    ] {
        let run = e
            .store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
            .await
            .unwrap();
        e.store.set_run_queued_at(&run.id, at).await.unwrap();
    }
    let counts = e
        .store
        .delivery_counts(&dot.id, &timestamp(start), &timestamp(end))
        .await
        .unwrap();
    assert_eq!(counts.sent, 2);
}
