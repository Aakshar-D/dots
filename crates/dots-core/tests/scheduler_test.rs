mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::{EngineKind, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
use tokio_util::sync::CancellationToken;

/// Runner without a dispatcher: scheduled runs stay queued, which makes coalescing visible.
fn idle_runner(store: &Store, dir: &tempfile::TempDir) -> Arc<Runner> {
    Runner::new(
        store.clone(),
        new_bus(),
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: String::new(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
        },
    )
}

#[tokio::test]
async fn every_second_schedule_enqueues_once_and_coalesces() {
    let (dir, store) = temp_store().await;
    let mut s = spec("tick");
    s.schedule = Some("* * * * * *".into());
    let dot = store.create_dot(&s).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(2600)).await;
    let runs = store.list_runs(Some(&dot.id), 10).await.unwrap();
    assert_eq!(
        runs.len(),
        1,
        "later ticks must coalesce into the queued run"
    );
    assert_eq!(runs[0].trigger, TriggerKind::Schedule);
    assert!(runs[0].payload.as_ref().unwrap()["scheduled_for"].is_string());
}

#[tokio::test]
async fn disabled_and_unscheduled_dots_are_ignored() {
    let (dir, store) = temp_store().await;
    let mut off = spec("off");
    off.schedule = Some("* * * * * *".into());
    off.enabled = false;
    store.create_dot(&off).await.unwrap();
    store.create_dot(&spec("manual-only")).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn reload_picks_up_new_schedules() {
    let (dir, store) = temp_store().await;
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    sched.spawn(CancellationToken::new());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut s = spec("late");
    s.schedule = Some("* * * * * *".into());
    let dot = store.create_dot(&s).await.unwrap();
    sched.reload();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while store.list_runs(Some(&dot.id), 10).await.unwrap().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "no scheduled run after reload"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn fire_coalesces_when_a_run_is_already_queued() {
    let (dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("f")).await.unwrap();
    let sched = Scheduler::new(store.clone(), idle_runner(&store, &dir));
    let now = chrono::Local::now();
    assert!(sched.fire(&dot, now).await.unwrap().is_some());
    assert!(sched.fire(&dot, now).await.unwrap().is_none());
}
