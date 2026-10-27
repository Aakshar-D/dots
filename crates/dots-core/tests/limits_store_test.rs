//! What the usage phase stores (usage and limits spec section 7): the latest reading of each
//! Claude limit window, the API-equivalent cost of a run, a run put back in the queue by
//! Claude's limit, and **Run anyway** on a held run.

mod common;

use common::{spec, temp_store};
use dots_core::model::{NewRun, RunStatus, TriggerKind};
use dots_core::store::LimitReading;
use dots_core::Error;
use serde_json::json;

fn reading(window: &str, utilization: f64) -> LimitReading {
    LimitReading {
        window: window.into(),
        utilization,
        resets_at: "2026-10-26T15:10:00.000Z".into(),
        status: "allowed".into(),
        read_at: "2026-10-26T12:00:00.000Z".into(),
    }
}

#[tokio::test]
async fn readings_are_kept_per_window_and_replaced() {
    let (_d, store) = temp_store().await;
    store
        .save_limit_readings(&[reading("five_hour", 0.42)])
        .await
        .unwrap();
    store
        .save_limit_readings(&[reading("five_hour", 0.64), reading("seven_day", 0.55)])
        .await
        .unwrap();
    let all = store.limit_readings().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0], reading("five_hour", 0.64));
    assert_eq!(all[1], reading("seven_day", 0.55));
}

#[tokio::test]
async fn cost_adds_up_and_stays_null_without_one() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.add_usage(&run.id, 10, 2, None).await.unwrap();
    assert_eq!(store.get_run(&run.id).await.unwrap().cost_usd, None);
    store.add_usage(&run.id, 10, 2, Some(0.5)).await.unwrap();
    store.add_usage(&run.id, 10, 2, Some(0.25)).await.unwrap();
    let run = store.get_run(&run.id).await.unwrap();
    assert_eq!(run.cost_usd, Some(0.75));
    assert_eq!((run.tokens_in, run.tokens_out), (30, 6));
}

#[tokio::test]
async fn requeue_puts_a_running_run_back_with_its_payload() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.payload = Some(json!({"a": 1}));
    let run = store.create_run(&new).await.unwrap();
    assert!(store.claim_run(&run.id).await.unwrap());
    store
        .set_workspace(&run.id, "C:/ws", Some("dots/a/x"), Some("abc123"))
        .await
        .unwrap();
    store.set_session(&run.id, "sess-1").await.unwrap();

    // Queued with nothing of a thread: what this attempt set goes.
    let back = store.requeue_run(&run.id, &run).await.unwrap();
    assert_eq!(back.status, RunStatus::Queued);
    assert_eq!(back.started_at, None);
    assert_eq!(back.session_id, None);
    assert_eq!(back.workspace_path, None);
    assert_eq!(back.branch, None);
    assert_eq!(back.base_commit, None);
    assert_eq!(back.trigger, TriggerKind::Watcher);
    assert_eq!(back.payload, Some(json!({"a": 1})));
    assert!(matches!(
        store.requeue_run(&run.id, &run).await,
        Err(Error::Conflict(_))
    ));
}

/// Final review I1: a run queued with its thread's session and folder gets them back.
#[tokio::test]
async fn requeue_restores_the_thread_a_run_was_queued_with() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::FollowUp);
    new.session_id = Some("thread".into());
    new.workspace_path = Some("C:/ws".into());
    new.branch = Some("dots/a/x".into());
    new.base_commit = Some("abc123".into());
    let queued = store.create_run(&new).await.unwrap();
    assert!(store.claim_run(&queued.id).await.unwrap());
    store.set_session(&queued.id, "other").await.unwrap();
    let back = store.requeue_run(&queued.id, &queued).await.unwrap();
    assert_eq!(back.status, RunStatus::Queued);
    assert_eq!(back.session_id.as_deref(), Some("thread"));
    assert_eq!(back.workspace_path.as_deref(), Some("C:/ws"));
    assert_eq!(back.branch.as_deref(), Some("dots/a/x"));
    assert_eq!(back.base_commit.as_deref(), Some("abc123"));
}

#[tokio::test]
async fn hold_released_is_stored() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    assert!(!run.hold_released);
    let released = store.set_hold_released(&run.id).await.unwrap();
    assert!(released.hold_released);
    assert!(store.get_run(&run.id).await.unwrap().hold_released);
}
