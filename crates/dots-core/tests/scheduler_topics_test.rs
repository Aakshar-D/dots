mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::engine::Engine;
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::model::{EngineKind, NewRun, RunStatus, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::Scheduler;
use dots_core::util::timestamp;
use tokio_util::sync::CancellationToken;

const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn the_scheduler_s_pass_archives_idle_topics_hourly() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let mut events = bus.subscribe();
    let clock = FakeClock::new(t0());
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: String::new(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    );
    let dot = store
        .create_dot(&spec("idle"), None, "default")
        .await
        .unwrap();
    // Two chat topics 40 days old: the older is idle, the newer is the current one.
    let mut roots = Vec::new();
    for (i, days) in [41, 40].into_iter().enumerate() {
        let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
        new.payload = Some(serde_json::json!({ "message": format!("m{i}"), "typed": true }));
        let run = store.create_run(&new).await.unwrap();
        let at = timestamp(t0() - TimeDelta::days(days));
        store.set_run_queued_at(&run.id, &at).await.unwrap();
        store
            .set_status(&run.id, RunStatus::Succeeded)
            .await
            .unwrap();
        store.set_run_times(&run.id, &at, Some(&at)).await.unwrap();
        roots.push(run.id);
    }
    let sched = Scheduler::new(store.clone(), runner, bus.clone(), clock.clone());
    let shutdown = CancellationToken::new();
    sched.spawn(shutdown.clone());
    clock.nth_sleep(1).await;

    let info = store.topic_info(&roots[0]).await.unwrap().unwrap();
    assert_eq!(info.archived_at, Some(timestamp(t0())));
    assert!(store.topic_info(&roots[1]).await.unwrap().is_none());
    let mut announced = false;
    while let Ok(ev) = events.try_recv() {
        if matches!(&ev, RuntimeEvent::TopicsChanged { dot_id } if *dot_id == dot.id) {
            announced = true;
        }
    }
    assert!(announced);

    // Unarchived by hand, and idle again only in 30 days: the next passes leave it.
    store
        .set_topic_archived(&roots[0], false, t0())
        .await
        .unwrap();
    clock.set(t0() + TimeDelta::hours(2));
    clock.nth_sleep(2).await;
    assert!(store
        .topic_info(&roots[0])
        .await
        .unwrap()
        .unwrap()
        .archived_at
        .is_none());

    // Off in the settings: nothing is archived, however idle.
    let mut settings = store.get_settings().await.unwrap();
    settings.auto_archive_days = 0;
    store.put_settings(&settings).await.unwrap();
    clock.set(t0() + TimeDelta::days(60));
    clock.nth_sleep(3).await;
    assert!(store
        .topic_info(&roots[0])
        .await
        .unwrap()
        .unwrap()
        .archived_at
        .is_none());
    shutdown.cancel();
}
