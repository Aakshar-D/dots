//! Delivery tracking in the runner (delivery tracking spec sections 2 and 3): a run is
//! delivered once its engine starts on it, and a waiting run says why it waits.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store, wait_status};
use dots_core::delivery::{wait_reason, WaitFacts};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent, RunContext, UnavailableEngine};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::limits::{HoldShares, LimitReading, Limits};
use dots_core::model::{
    Delivery, Dot, EngineKind, NewRun, Run, RunStatus, TriggerKind, WaitReason, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T12:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

const SHARES: HoldShares = HoldShares {
    enabled: true,
    five_hour: 60,
    weekly: 70,
};

fn five(u: f64) -> LimitReading {
    LimitReading {
        window: "five_hour".into(),
        utilization: u,
        resets_at: timestamp(t0() + TimeDelta::hours(1)),
        status: "allowed".into(),
        read_at: timestamp(t0()),
    }
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

struct Env {
    dir: TempDir,
    store: Store,
    bus: Bus,
    runner: Arc<Runner>,
}

fn runner(
    store: &Store,
    bus: &Bus,
    dir: &TempDir,
    engine: Arc<dyn Engine>,
    max: usize,
) -> Arc<Runner> {
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine);
    Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: max,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    )
}

/// A runner with no dispatcher: no queued run ever starts.
async fn still(max: usize) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![finished()])),
        max,
    );
    Env {
        dir,
        store,
        bus,
        runner,
    }
}

/// A runner with a dispatcher, its engine `engine`.
async fn moving(engine: Arc<dyn Engine>) -> Env {
    moving_max(engine, 4).await
}

/// A runner with a dispatcher running at most `max` runs at once, its engine `engine`.
async fn moving_max(engine: Arc<dyn Engine>, max: usize) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner(&store, &bus, &dir, engine, max);
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        dir,
        store,
        bus,
        runner,
    }
}

impl Env {
    async fn dot(&self, name: &str) -> Dot {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    async fn queue(&self, dot: &Dot, trigger: TriggerKind) -> Run {
        let mut new = NewRun::new(&dot.id, trigger);
        new.payload = Some(json!({"text": "hello"}));
        self.store.create_run(&new).await.unwrap()
    }

    async fn reason(&self, id: &str) -> Option<WaitReason> {
        let mut runs = vec![self.store.get_run(id).await.unwrap()];
        self.runner.fill_wait_reasons(&mut runs).await.unwrap();
        runs[0].wait_reason
    }
}

#[test]
fn wait_reason_follows_its_precedence() {
    let base = WaitFacts {
        usage_held: false,
        spend_capped: false,
        runner_held: false,
        dot_has_active_run: false,
        running: 0,
        max_concurrent: 2,
    };
    let cases = [
        (base.clone(), WaitReason::Queued),
        (
            WaitFacts {
                running: 2,
                ..base.clone()
            },
            WaitReason::Busy,
        ),
        (
            WaitFacts {
                running: 2,
                dot_has_active_run: true,
                ..base.clone()
            },
            WaitReason::DotBusy,
        ),
        (
            WaitFacts {
                running: 2,
                dot_has_active_run: true,
                runner_held: true,
                ..base.clone()
            },
            WaitReason::Updating,
        ),
        (
            WaitFacts {
                running: 2,
                dot_has_active_run: true,
                runner_held: true,
                spend_capped: true,
                ..base.clone()
            },
            WaitReason::SpendCap,
        ),
        (
            WaitFacts {
                running: 2,
                dot_has_active_run: true,
                runner_held: true,
                spend_capped: true,
                usage_held: true,
                ..base.clone()
            },
            WaitReason::UsageHold,
        ),
    ];
    for (facts, want) in cases {
        assert_eq!(wait_reason(&facts), want, "{facts:?}");
    }
    assert_eq!(WaitReason::Busy.text(), "other runs are busy");
    assert_eq!(WaitReason::UsageHold.text(), "held for the usage limit");
    assert_eq!(WaitReason::Updating.text(), "Sage is updating");
    assert_eq!(
        WaitReason::DotBusy.text(),
        "this sol is busy with another run"
    );
    assert_eq!(WaitReason::Queued.text(), "next in line");
}

#[tokio::test]
async fn next_in_line() {
    let e = still(4).await;
    let dot = e.dot("a").await;
    let run = e.queue(&dot, TriggerKind::Chat).await;
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::Queued));
}

#[tokio::test]
async fn the_runner_held() {
    let e = still(4).await;
    e.runner.set_held(true).await;
    let dot = e.dot("a").await;
    let run = e.queue(&dot, TriggerKind::Chat).await;
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::Updating));
}

/// An engine that works for a long time: its run stays active in the runner.
fn working() -> Arc<dyn Engine> {
    Arc::new(ScriptedEngine::new(vec![
        Step::Sleep(Duration::from_secs(60)),
        finished(),
    ]))
}

#[tokio::test]
async fn the_dot_busy() {
    let e = moving_max(working(), 4).await;
    let dot = e.dot("a").await;
    let other = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &other.id, RunStatus::Running, 10).await;
    let run = e.queue(&dot, TriggerKind::Chat).await;
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::DotBusy));
}

#[tokio::test]
async fn busy() {
    let e = moving_max(working(), 1).await;
    let (a, b) = (e.dot("a").await, e.dot("b").await);
    let other = e
        .runner
        .enqueue(NewRun::new(&b.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &other.id, RunStatus::Running, 10).await;
    let run = e.queue(&a, TriggerKind::Chat).await;
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::Busy));
}

/// The waits read the runner's own slots, as the dispatcher does: a run the store calls running
/// or waiting for an approval, but the runner does not hold (parked for an approval), takes no
/// slot and keeps no dot busy.
#[tokio::test]
async fn a_run_the_runner_does_not_hold_keeps_no_dot_busy() {
    let e = still(1).await;
    let dot = e.dot("a").await;
    let parked = e.queue(&dot, TriggerKind::Manual).await;
    let claimed = e.queue(&dot, TriggerKind::Manual).await;
    let run = e.queue(&dot, TriggerKind::Chat).await;
    assert!(e.store.claim_run(&parked.id).await.unwrap());
    e.store
        .set_status(&parked.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    assert!(e.store.claim_run(&claimed.id).await.unwrap());
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::Queued));
}

#[tokio::test]
async fn a_disabled_dot_alone_is_next_in_line() {
    let e = still(4).await;
    let dot = e.dot("a").await;
    e.store.set_dot_enabled(&dot.id, false).await.unwrap();
    let run = e.queue(&dot, TriggerKind::Chat).await;
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::Queued));
}

#[tokio::test]
async fn usage_hold() {
    let e = still(4).await;
    let clock = FakeClock::new(t0());
    let limits = Limits::load(e.store.clone(), e.bus.clone(), clock.clone(), SHARES)
        .await
        .unwrap();
    limits.record("allowed", vec![five(0.7)]).await.unwrap();
    e.runner.set_limits(limits);
    let dot = e.dot("a").await;
    let run = e.queue(&dot, TriggerKind::Schedule).await;
    assert_eq!(e.reason(&run.id).await, Some(WaitReason::UsageHold));
    assert!(e.runner.usage_held_ids().await.unwrap().contains(&run.id));
}

#[tokio::test]
async fn runs_not_waiting_have_no_reason() {
    let e = still(4).await;
    let (a, b) = (e.dot("a").await, e.dot("b").await);
    let running = e.queue(&a, TriggerKind::Chat).await;
    assert!(e.store.claim_run(&running.id).await.unwrap());
    let requeued = e.queue(&b, TriggerKind::Chat).await;
    assert!(e.store.claim_run(&requeued.id).await.unwrap());
    e.store
        .mark_delivered(&requeued.id, Utc::now())
        .await
        .unwrap();
    e.store.requeue_run(&requeued.id, &requeued).await.unwrap();

    let mut runs = vec![
        e.store.get_run(&running.id).await.unwrap(),
        e.store.get_run(&requeued.id).await.unwrap(),
    ];
    assert_eq!(runs[1].status, RunStatus::Queued);
    assert_eq!(runs[1].delivery, Some(Delivery::Delivered));
    runs[0].wait_reason = Some(WaitReason::Queued);
    runs[1].wait_reason = Some(WaitReason::Queued);
    e.runner.fill_wait_reasons(&mut runs).await.unwrap();
    assert_eq!(runs[0].wait_reason, None);
    assert_eq!(runs[1].wait_reason, None);
}

/// Waits (at most 5 s) for a `RunUpdated` of `id` that `pred` accepts.
async fn updated(
    rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>,
    id: &str,
    pred: impl Fn(&Run) -> bool,
) -> Run {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let RuntimeEvent::RunUpdated { run } = rx.recv().await.unwrap() {
                if run.id == id && pred(&run) {
                    return run;
                }
            }
        }
    })
    .await
    .expect("no such RunUpdated")
}

#[tokio::test]
async fn a_run_is_delivered_when_its_engine_starts() {
    let engine = Arc::new(ScriptedEngine::new(vec![
        Step::Sleep(Duration::from_millis(300)),
        finished(),
    ]));
    let e = moving(engine).await;
    let mut rx = e.bus.subscribe();
    let dot = e.dot("a").await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let seen = updated(&mut rx, &run.id, |r| {
        r.delivery == Some(Delivery::Delivered)
    })
    .await;
    assert_eq!(seen.status, RunStatus::Running);
    assert!(seen.delivered_at.is_some());
    let done = wait_status(&e.store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(done.delivered_at, seen.delivered_at);
    assert_eq!(done.delivery, Some(Delivery::Delivered));
}

/// An engine whose start fails, without saying it is unavailable.
struct NoStart;

#[async_trait]
impl Engine for NoStart {
    async fn start(&self, _ctx: RunContext) -> dots_core::Result<mpsc::Receiver<EngineEvent>> {
        Err(Error::Other("no start".into()))
    }
}

#[tokio::test]
async fn an_engine_that_cannot_start_leaves_it_undelivered() {
    let e = moving(Arc::new(NoStart)).await;
    let dot = e.dot("a").await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    let done = wait_status(&e.store, &run.id, RunStatus::Failed, 10).await;
    assert_eq!(done.undelivered_reason.as_deref(), Some("no start"));
    assert_eq!(done.delivered_at, None);
    assert_eq!(done.delivery, Some(Delivery::NotDelivered));
}

#[tokio::test]
async fn an_unavailable_engine_marks_it_too() {
    let why = "claude CLI not found: install Claude Code";
    let e = moving(Arc::new(UnavailableEngine(why.into()))).await;
    let dot = e.dot("a").await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    let done = wait_status(&e.store, &run.id, RunStatus::Failed, 10).await;
    assert_eq!(done.undelivered_reason.as_deref(), Some(why));
    assert_eq!(done.delivered_at, None);
}

#[tokio::test]
async fn a_resumed_run_keeps_delivered_at() {
    let e = moving(Arc::new(ScriptedEngine::new(vec![finished()]))).await;
    e.runner.set_held(true).await;
    let dot = e.dot("a").await;
    let queued = e.queue(&dot, TriggerKind::Schedule).await;
    assert!(e.store.claim_run(&queued.id).await.unwrap());
    let first = t0() - TimeDelta::hours(2);
    e.store.mark_delivered(&queued.id, first).await.unwrap();
    e.store.requeue_run(&queued.id, &queued).await.unwrap();
    e.runner.set_held(false).await;
    let done = wait_status(&e.store, &queued.id, RunStatus::Succeeded, 10).await;
    assert_eq!(done.delivered_at, Some(timestamp(first)));
    assert_eq!(done.delivery, Some(Delivery::Delivered));
}
