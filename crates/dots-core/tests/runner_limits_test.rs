//! The runner and Claude's usage limits (usage and limits spec section 4): held background runs
//! stay queued while the person's runs start; after a hold they start one at a time, oldest
//! first; a background run that Claude's `rejected` ended before it did anything goes back to
//! the queue, one that had acted fails with a note, and the person's own run is left alone.

mod common;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store, wait_status};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent, RunContext};
use dots_core::events::new_bus;
use dots_core::limits::{HoldShares, LimitReading, Limits, STOPPED_BY_LIMIT};
use dots_core::model::{Dot, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::util::timestamp;
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

fn five(u: f64, status: &str) -> LimitReading {
    LimitReading {
        window: "five_hour".into(),
        utilization: u,
        resets_at: timestamp(t0() + TimeDelta::hours(1)),
        status: status.into(),
        read_at: timestamp(t0()),
    }
}

fn rejected() -> EngineEvent {
    EngineEvent::Limits {
        status: "rejected".into(),
        windows: vec![five(1.0, "rejected")],
        resets_at: Some(timestamp(t0() + TimeDelta::hours(1))),
    }
}

fn finished() -> EngineEvent {
    EngineEvent::Finished {
        summary: "done".into(),
    }
}

/// An engine that plays its scripts in turn, one per run it starts, then `default` for every
/// run after them.
struct Turns {
    scripts: Mutex<VecDeque<ScriptedEngine>>,
    default: ScriptedEngine,
}

impl Turns {
    fn new(scripts: Vec<Vec<Step>>, default: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into_iter().map(ScriptedEngine::new).collect()),
            default: ScriptedEngine::new(default),
        })
    }
}

#[async_trait]
impl Engine for Turns {
    async fn start(&self, ctx: RunContext) -> dots_core::Result<mpsc::Receiver<EngineEvent>> {
        let next = self.scripts.lock().unwrap().pop_front();
        match next {
            Some(engine) => engine.start(ctx).await,
            None => self.default.start(ctx).await,
        }
    }
}

struct Env {
    dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    limits: Arc<Limits>,
    runner: Arc<Runner>,
}

async fn env(engine: Arc<dyn Engine>, max: usize) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let clock = FakeClock::new(t0());
    let limits = Limits::load(store.clone(), bus.clone(), clock.clone(), SHARES)
        .await
        .unwrap();
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine);
    let runner = Runner::new(
        store.clone(),
        bus,
        engines,
        RunnerConfig {
            max_concurrent: max,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    runner.set_limits(limits.clone());
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        dir,
        store,
        clock,
        limits,
        runner,
    }
}

impl Env {
    async fn dot(&self, name: &str) -> Dot {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None).await.unwrap()
    }

    async fn queue(&self, dot: &Dot, trigger: TriggerKind) -> String {
        let mut new = NewRun::new(&dot.id, trigger);
        new.payload = Some(json!({"n": dot.spec.name}));
        let id = self.runner.enqueue(new).await.unwrap().id;
        // Queue times are kept to the millisecond: the next run must come after this one.
        tokio::time::sleep(Duration::from_millis(5)).await;
        id
    }

    async fn status(&self, id: &str) -> RunStatus {
        self.store.get_run(id).await.unwrap().status
    }

    /// A reading as a run would bring it, and the dispatcher told.
    async fn read(&self, u: f64) {
        self.limits
            .record("allowed", vec![five(u, "allowed")])
            .await
            .unwrap();
        self.runner.nudge();
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn slow(steps: Vec<Step>) -> Vec<Step> {
    let mut all = vec![Step::Sleep(Duration::from_millis(400))];
    all.extend(steps);
    all
}

#[tokio::test]
async fn held_runs_stay_queued_while_others_start() {
    let e = env(Turns::new(vec![], vec![Step::Emit(finished())]), 2).await;
    e.read(0.7).await;
    let (a, b) = (e.dot("a").await, e.dot("b").await);
    let held = e.queue(&a, TriggerKind::Schedule).await;
    let mine = e.queue(&b, TriggerKind::Manual).await;
    wait_status(&e.store, &mine, RunStatus::Succeeded, 5).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(e.status(&held).await, RunStatus::Queued);
}

async fn three_held(e: &Env) -> [String; 3] {
    e.read(0.7).await;
    let mut ids = Vec::new();
    for name in ["w1", "w2", "w3"] {
        let dot = e.dot(name).await;
        ids.push(e.queue(&dot, TriggerKind::Watcher).await);
    }
    e.runner.nudge();
    tokio::time::sleep(Duration::from_millis(200)).await;
    for id in &ids {
        assert_eq!(e.status(id).await, RunStatus::Queued);
    }
    ids.try_into().unwrap()
}

#[tokio::test]
async fn held_runs_start_one_at_a_time_oldest_first() {
    let reading = EngineEvent::Limits {
        status: "allowed".into(),
        windows: vec![five(0.1, "allowed")],
        resets_at: None,
    };
    let e = env(
        Turns::new(
            vec![],
            slow(vec![Step::Emit(reading), Step::Emit(finished())]),
        ),
        2,
    )
    .await;
    let [w1, w2, w3] = three_held(&e).await;
    e.read(0.1).await;
    assert_eq!(e.status(&w1).await, RunStatus::Running);
    assert_eq!(e.status(&w2).await, RunStatus::Queued);
    assert_eq!(e.status(&w3).await, RunStatus::Queued);
    wait_status(&e.store, &w1, RunStatus::Succeeded, 5).await;
    wait_status(&e.store, &w2, RunStatus::Running, 5).await;
    assert_eq!(e.status(&w3).await, RunStatus::Queued);
    wait_status(&e.store, &w3, RunStatus::Succeeded, 10).await;
}

#[tokio::test]
async fn a_run_that_ends_without_a_reading_lets_the_next_start() {
    let e = env(Turns::new(vec![], slow(vec![Step::Emit(finished())])), 2).await;
    let [w1, w2, w3] = three_held(&e).await;
    e.read(0.1).await;
    assert_eq!(e.status(&w1).await, RunStatus::Running);
    assert_eq!(e.status(&w2).await, RunStatus::Queued);
    wait_status(&e.store, &w1, RunStatus::Succeeded, 5).await;
    wait_status(&e.store, &w2, RunStatus::Running, 5).await;
    assert_eq!(e.status(&w3).await, RunStatus::Queued);
}

#[tokio::test]
async fn a_background_run_rejected_before_a_tool_call_is_queued_again() {
    let first = vec![
        Step::Emit(rejected()),
        Step::Emit(EngineEvent::Failed {
            error: "You've hit your limit".into(),
        }),
    ];
    let e = env(Turns::new(vec![first], vec![Step::Emit(finished())]), 2).await;
    let dot = e.dot("w").await;
    let id = e.queue(&dot, TriggerKind::Watcher).await;
    // It ran, was stopped by the limit, and is back in the queue as it was.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while e.limits.rejected_until().is_none() || e.status(&id).await != RunStatus::Queued {
        assert!(std::time::Instant::now() < deadline, "not queued again");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let run = e.store.get_run(&id).await.unwrap();
    assert_eq!(run.payload, Some(json!({"n": "w"})));
    assert_eq!(run.error, None);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(e.status(&id).await, RunStatus::Queued);

    e.clock
        .set(t0() + TimeDelta::hours(1) + TimeDelta::seconds(1));
    e.runner.nudge();
    wait_status(&e.store, &id, RunStatus::Succeeded, 5).await;
}

/// Final review I1: a follow-up continues its thread; sent back to the queue it keeps the
/// thread's session and folder, so after the reset it resumes the conversation.
#[tokio::test]
async fn a_follow_up_sent_back_to_the_queue_keeps_its_thread() {
    let first = vec![
        Step::Emit(rejected()),
        Step::Emit(EngineEvent::Failed {
            error: "You've hit your limit".into(),
        }),
    ];
    let e = env(Turns::new(vec![first], vec![Step::Emit(finished())]), 2).await;
    let dot = e.dot("f").await;
    let folder = dot.spec.workdir.clone();
    let mut new = NewRun::new(&dot.id, TriggerKind::FollowUp);
    new.session_id = Some("thread-session".into());
    new.workspace_path = Some(folder.clone());
    let id = e.runner.enqueue(new).await.unwrap().id;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while e.limits.rejected_until().is_none() || e.status(&id).await != RunStatus::Queued {
        assert!(std::time::Instant::now() < deadline, "not queued again");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let run = e.store.get_run(&id).await.unwrap();
    assert_eq!(run.session_id.as_deref(), Some("thread-session"));
    assert_eq!(run.workspace_path.as_deref(), Some(folder.as_str()));
}

/// Final review I2: a `rejected` line Dots cannot read a reset from does not send the run
/// back to the queue, where it would start again at once; it fails with the note.
#[tokio::test]
async fn a_rejected_run_without_a_reset_fails_instead_of_starting_again() {
    let first = vec![
        Step::Emit(EngineEvent::Limits {
            status: "rejected".into(),
            windows: vec![],
            resets_at: None,
        }),
        Step::Emit(EngineEvent::Failed {
            error: "You've hit your limit".into(),
        }),
    ];
    let e = env(Turns::new(vec![first], vec![Step::Emit(finished())]), 2).await;
    let dot = e.dot("w").await;
    let id = e.queue(&dot, TriggerKind::Watcher).await;
    let run = wait_status(&e.store, &id, RunStatus::Failed, 5).await;
    assert_eq!(
        run.error.as_deref(),
        Some(format!("{STOPPED_BY_LIMIT}: You've hit your limit").as_str())
    );
}

#[tokio::test]
async fn a_background_run_rejected_after_a_tool_call_fails_with_the_note() {
    let first = vec![
        Step::Emit(EngineEvent::ToolCall {
            id: "t1".into(),
            tool: "Read".into(),
            input: json!({"file_path": "a.txt"}),
        }),
        Step::Emit(rejected()),
        Step::Emit(EngineEvent::Failed {
            error: "You've hit your limit".into(),
        }),
    ];
    let e = env(Turns::new(vec![first], vec![Step::Emit(finished())]), 2).await;
    let dot = e.dot("w").await;
    let id = e.queue(&dot, TriggerKind::Schedule).await;
    let run = wait_status(&e.store, &id, RunStatus::Failed, 5).await;
    assert_eq!(
        run.error.as_deref(),
        Some(format!("{STOPPED_BY_LIMIT}: You've hit your limit").as_str())
    );
}

#[tokio::test]
async fn a_running_chat_is_not_touched_by_a_rejected_reading() {
    let first = vec![
        Step::Emit(rejected()),
        Step::Emit(EngineEvent::Failed {
            error: "You've hit your limit".into(),
        }),
    ];
    let e = env(Turns::new(vec![first], vec![Step::Emit(finished())]), 2).await;
    let dot = e.dot("c").await;
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "hello"}));
    let id = e.runner.enqueue(new).await.unwrap().id;
    let run = wait_status(&e.store, &id, RunStatus::Failed, 5).await;
    assert_eq!(run.error.as_deref(), Some("You've hit your limit"));
}
