mod common;

use chrono::{DateTime, Duration, Utc};
use common::{spec, temp_store};
use dots_core::model::{
    Dot, NewRun, PendingRetry, RunStatus, ScheduleChange, SlotStart, TriggerKind,
};
use dots_core::schedule::Schedule;
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::json;

const T0: &str = "2026-10-14T09:00:00.000Z";
const T1: &str = "2026-10-14T10:00:00.000Z";
const T2: &str = "2026-10-14T11:00:00.000Z";
const T3: &str = "2026-10-14T12:00:00.000Z";

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

async fn scheduled(store: &Store, name: &str) -> Dot {
    let mut s = spec(name);
    s.schedule = Some(Schedule::cron("0 0 9 * * *"));
    let dot = store.create_dot(&s, None, "default").await.unwrap();
    store
        .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, t(T0))
        .await
        .unwrap();
    dot
}

fn slot_run(dot: &Dot, at: &str) -> NewRun {
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.payload = Some(json!({ "scheduled_for": at }));
    new
}

/// A run of the dot in the given status (no dispatcher is involved at store level).
async fn run_in(store: &Store, dot: &Dot, status: RunStatus) -> String {
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.set_status(&run.id, status).await.unwrap();
    run.id
}

#[tokio::test]
async fn a_dot_is_busy_only_while_a_run_runs_or_waits() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let other = scheduled(&store, "b").await;
    assert!(!store.dot_is_busy(&dot.id).await.unwrap());

    run_in(&store, &dot, RunStatus::Queued).await;
    assert!(!store.dot_is_busy(&dot.id).await.unwrap());

    let running = run_in(&store, &dot, RunStatus::Running).await;
    assert!(store.dot_is_busy(&dot.id).await.unwrap());
    assert!(!store.dot_is_busy(&other.id).await.unwrap());

    store
        .set_status(&running, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    assert!(store.dot_is_busy(&dot.id).await.unwrap());

    store
        .set_status(&running, RunStatus::Succeeded)
        .await
        .unwrap();
    assert!(!store.dot_is_busy(&dot.id).await.unwrap());
}

#[tokio::test]
async fn a_busy_slot_is_recorded_not_run() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let running = run_in(&store, &dot, RunStatus::Running).await;
    let runs_before = store.list_runs(Some(&dot.id), 100).await.unwrap().len();
    let fired_before = store
        .get_schedule_state(&dot.id)
        .await
        .unwrap()
        .unwrap()
        .fired_count;

    let start = store
        .record_slot(&dot.id, t(T1), Some(&slot_run(&dot, T1)), true, t(T2))
        .await
        .unwrap();
    let SlotStart::Busy(skip) = start else {
        panic!("expected Busy, got {start:?}");
    };
    assert_eq!(skip.reason, "busy");
    assert_eq!(skip.run_id, Some(running));
    assert_eq!(skip.slot_at, T1);
    assert_eq!(skip.dot_id, dot.id);

    let state = store.get_schedule_state(&dot.id).await.unwrap().unwrap();
    assert_eq!(state.last_slot_at, T1);
    assert_eq!(state.fired_count, fired_before);
    assert_eq!(
        store.list_runs(Some(&dot.id), 100).await.unwrap().len(),
        runs_before
    );
    assert_eq!(store.schedule_skips(&dot.id, 10).await.unwrap(), vec![skip]);
}

#[tokio::test]
async fn a_free_or_once_slot_still_fires() {
    let (_d, store) = temp_store().await;

    // Busy, but the schedule does not skip (a one-time schedule).
    let busy = scheduled(&store, "busy").await;
    run_in(&store, &busy, RunStatus::Running).await;
    let start = store
        .record_slot(&busy.id, t(T1), Some(&slot_run(&busy, T1)), false, t(T2))
        .await
        .unwrap();
    assert!(matches!(start, SlotStart::Enqueued(_)), "{start:?}");
    assert!(store.schedule_skips(&busy.id, 10).await.unwrap().is_empty());

    // Free, skipping on.
    let free = scheduled(&store, "free").await;
    let start = store
        .record_slot(&free.id, t(T1), Some(&slot_run(&free, T1)), true, t(T2))
        .await
        .unwrap();
    assert!(matches!(start, SlotStart::Enqueued(_)), "{start:?}");

    // Only a queued run: the slot joins it.
    let queued = scheduled(&store, "queued").await;
    run_in(&store, &queued, RunStatus::Queued).await;
    let start = store
        .record_slot(&queued.id, t(T1), Some(&slot_run(&queued, T1)), true, t(T2))
        .await
        .unwrap();
    assert!(matches!(start, SlotStart::Coalesced(_)), "{start:?}");
}

#[tokio::test]
async fn a_busy_skip_keeps_a_pending_retry() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let running = run_in(&store, &dot, RunStatus::Running).await;
    let change = ScheduleChange {
        watch_run_id: Some(Some(running.clone())),
        retry: Some(Some(PendingRetry {
            slot_at: T0.into(),
            attempt: 2,
            at: T3.into(),
        })),
        ..Default::default()
    };
    store
        .update_schedule_state(&dot.id, &change, t(T0))
        .await
        .unwrap()
        .unwrap();
    let before = store.get_schedule_state(&dot.id).await.unwrap().unwrap();

    let start = store
        .record_slot(&dot.id, t(T1), Some(&slot_run(&dot, T1)), true, t(T2))
        .await
        .unwrap();
    assert!(matches!(start, SlotStart::Busy(_)), "{start:?}");

    let after = store.get_schedule_state(&dot.id).await.unwrap().unwrap();
    assert_eq!(after.retry, before.retry);
    assert!(after.retry.is_some());
    assert_eq!(after.watch_run_id, before.watch_run_id);
    assert_eq!(after.fired_count, before.fired_count);
}

#[tokio::test]
async fn only_the_newest_50_skips_are_kept() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    run_in(&store, &dot, RunStatus::Running).await;
    let mut last = String::new();
    for i in 1..=55 {
        let at = t(T1) + Duration::minutes(i);
        last = timestamp(at);
        let start = store
            .record_slot(&dot.id, at, Some(&slot_run(&dot, &last)), true, at)
            .await
            .unwrap();
        assert!(matches!(start, SlotStart::Busy(_)), "{start:?}");
    }
    let skips = store.schedule_skips(&dot.id, 100).await.unwrap();
    assert_eq!(skips.len(), 50);
    assert_eq!(skips[0].slot_at, last);
    assert!(skips.windows(2).all(|w| w[0].slot_at > w[1].slot_at));
    // The 5 oldest are gone.
    assert_eq!(
        skips.last().unwrap().slot_at,
        timestamp(t(T1) + Duration::minutes(6))
    );
    assert_eq!(store.schedule_skips(&dot.id, 3).await.unwrap().len(), 3);
}

#[tokio::test]
async fn skips_go_with_their_dot() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    run_in(&store, &dot, RunStatus::Running).await;
    store
        .record_slot(&dot.id, t(T1), Some(&slot_run(&dot, T1)), true, t(T2))
        .await
        .unwrap();
    assert_eq!(store.schedule_skips(&dot.id, 10).await.unwrap().len(), 1);
    store.delete_dot(&dot.id).await.unwrap();
    assert!(store.schedule_skips(&dot.id, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn skips_since_counts_after_a_time() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    run_in(&store, &dot, RunStatus::Running).await;
    for at in [T1, T2, T3] {
        let start = store
            .record_slot(&dot.id, t(at), Some(&slot_run(&dot, at)), true, t(at))
            .await
            .unwrap();
        assert!(matches!(start, SlotStart::Busy(_)), "{start:?}");
    }
    assert_eq!(store.skips_since(&dot.id, Some(T1)).await.unwrap(), 2);
    assert_eq!(store.skips_since(&dot.id, None).await.unwrap(), 3);
    assert_eq!(store.skips_since(&dot.id, Some(T3)).await.unwrap(), 0);
}
