mod common;

use chrono::{DateTime, Utc};
use common::{spec, temp_store};
use dots_core::model::{
    Dot, NewRun, PendingRetry, RunStatus, ScheduleChange, ScheduleState, ScheduleStatus, SlotStart,
    TriggerKind,
};
use dots_core::schedule::Schedule;
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;

const T0: &str = "2026-10-14T09:00:00.000Z";
const T1: &str = "2026-10-14T10:00:00.000Z";
const T2: &str = "2026-10-14T11:00:00.000Z";
const T3: &str = "2026-10-14T12:00:00.000Z";

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn daily_at_nine() -> Schedule {
    Schedule::cron("0 0 9 * * *")
}

/// A dot with a schedule and its row, made at `T0`.
async fn scheduled(store: &Store, name: &str) -> Dot {
    let mut s = spec(name);
    s.schedule = Some(daily_at_nine());
    let dot = store.create_dot(&s).await.unwrap();
    assert!(store
        .sync_schedule_state(&dot.id, dot.spec.schedule.as_ref(), false, t(T0))
        .await
        .unwrap());
    dot
}

async fn state(store: &Store, dot: &Dot) -> ScheduleState {
    store.get_schedule_state(&dot.id).await.unwrap().unwrap()
}

/// A row as a reset leaves it.
fn fresh(dot: &Dot, schedule: &Schedule, at: &str) -> ScheduleState {
    ScheduleState {
        dot_id: dot.id.clone(),
        rule_key: serde_json::to_string(schedule).unwrap(),
        status: ScheduleStatus::Active,
        last_slot_at: at.into(),
        fired_count: 0,
        watch_run_id: None,
        retry: None,
        attention_run_id: None,
        updated_at: at.into(),
    }
}

fn retry(attempt: u32) -> PendingRetry {
    PendingRetry {
        slot_at: T1.into(),
        attempt,
        at: T2.into(),
    }
}

async fn failed_run(store: &Store, dot: &Dot) -> String {
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    store
        .finish_run(&run.id, RunStatus::Failed, None, Some("boom"))
        .await
        .unwrap();
    run.id
}

/// Watches `run_id`, with a retry pending and attention set: a row in the middle of things.
async fn busy(store: &Store, dot: &Dot, run_id: &str) {
    let change = ScheduleChange {
        watch_run_id: Some(Some(run_id.into())),
        retry: Some(Some(retry(2))),
        attention_run_id: Some(Some(run_id.into())),
        ..Default::default()
    };
    store
        .update_schedule_state(&dot.id, &change, t(T0))
        .await
        .unwrap()
        .unwrap();
}

fn scheduled_run(dot: &Dot, at: &str) -> NewRun {
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.payload = Some(json!({ "scheduled_for": at }));
    new
}

#[test]
fn the_rule_key_is_the_schedules_json() {
    let schedule = daily_at_nine();
    assert_eq!(
        schedule.rule_key(),
        r#"{"rule":{"kind":"cron","expr":"0 0 9 * * *"},"catch_up":false,"until":null}"#
    );
}

#[tokio::test]
async fn a_row_is_created_reset_and_deleted_with_the_schedule() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let schedule = dot.spec.schedule.clone().unwrap();
    assert_eq!(state(&store, &dot).await, fresh(&dot, &schedule, T0));

    // The same schedule: nothing to do.
    let run = failed_run(&store, &dot).await;
    busy(&store, &dot, &run).await;
    let before = state(&store, &dot).await;
    assert!(!store
        .sync_schedule_state(&dot.id, Some(&schedule), false, t(T1))
        .await
        .unwrap());
    assert_eq!(state(&store, &dot).await, before);

    // A reset (the dot was enabled) forgets the watch, the retry, the attention and the count.
    assert!(store
        .sync_schedule_state(&dot.id, Some(&schedule), true, t(T1))
        .await
        .unwrap());
    assert_eq!(state(&store, &dot).await, fresh(&dot, &schedule, T1));

    // A different schedule resets the row too.
    busy(&store, &dot, &run).await;
    let other = Schedule::cron("0 0 10 * * *");
    assert!(store
        .sync_schedule_state(&dot.id, Some(&other), false, t(T2))
        .await
        .unwrap());
    assert_eq!(state(&store, &dot).await, fresh(&dot, &other, T2));

    // No schedule, no row.
    assert!(store
        .sync_schedule_state(&dot.id, None, false, t(T3))
        .await
        .unwrap());
    assert_eq!(store.get_schedule_state(&dot.id).await.unwrap(), None);
    assert!(!store
        .sync_schedule_state(&dot.id, None, false, t(T3))
        .await
        .unwrap());
}

#[tokio::test]
async fn deleting_the_dot_deletes_its_row() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    store.delete_dot(&dot.id).await.unwrap();
    assert_eq!(store.get_schedule_state(&dot.id).await.unwrap(), None);
}

#[tokio::test]
async fn a_pass_works_on_the_rows_of_enabled_dots() {
    let (_d, store) = temp_store().await;
    let on = scheduled(&store, "on").await;
    let off = scheduled(&store, "off").await;
    store.set_dot_enabled(&off.id, false).await.unwrap();
    // A dot without a schedule has no row.
    store.create_dot(&spec("plain")).await.unwrap();

    let rows = store.list_schedule_states().await.unwrap();
    assert_eq!(rows, [state(&store, &on).await]);
    // The disabled dot's row is left as it is.
    assert!(store.get_schedule_state(&off.id).await.unwrap().is_some());
}

#[tokio::test]
async fn a_skipped_slot_moves_last_slot_at_only() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let before = state(&store, &dot).await;

    let start = store
        .record_slot(&dot.id, t(T1), None, t(T2))
        .await
        .unwrap();
    assert_eq!(start, SlotStart::Skipped);
    assert_eq!(
        state(&store, &dot).await,
        ScheduleState {
            last_slot_at: T1.into(),
            updated_at: T2.into(),
            ..before
        }
    );
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_fired_slot_enqueues_counts_watches_and_drops_the_retry() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    busy(&store, &dot, &failed).await;

    let new = scheduled_run(&dot, "2026-10-14T12:00:00+02:00");
    let SlotStart::Enqueued(run) = store
        .record_slot(&dot.id, t(T1), Some(&new), t(T2))
        .await
        .unwrap()
    else {
        panic!("expected a new run")
    };
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.trigger, TriggerKind::Schedule);
    assert_eq!(run.payload, new.payload);
    assert_eq!(store.get_run(&run.id).await.unwrap(), run);

    let row = state(&store, &dot).await;
    assert_eq!(row.last_slot_at, T1);
    assert_eq!(row.fired_count, 1);
    assert_eq!(row.watch_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(row.retry, None);
    // Attention stays until a scheduled run succeeds (judged later).
    assert_eq!(row.attention_run_id.as_deref(), Some(failed.as_str()));
    assert_eq!(row.updated_at, T2);
}

#[tokio::test]
async fn a_slot_joins_the_queued_run() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let queued = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();

    let start = store
        .record_slot(&dot.id, t(T1), Some(&scheduled_run(&dot, T1)), t(T1))
        .await
        .unwrap();
    assert_eq!(start, SlotStart::Coalesced(queued.clone()));
    assert_eq!(
        store.list_runs(None, 10).await.unwrap(),
        vec![queued.clone()]
    );
    let row = state(&store, &dot).await;
    assert_eq!(row.fired_count, 1);
    assert_eq!(row.watch_run_id, Some(queued.id));
}

#[tokio::test]
async fn a_slot_that_is_not_newer_than_the_last_one_is_refused() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    store
        .record_slot(&dot.id, t(T1), None, t(T1))
        .await
        .unwrap();
    let before = state(&store, &dot).await;

    // The same slot again, an older one (the row was reset after the pass read it), and a dot
    // without a row: nothing is written and no run is made.
    for (dot_id, at) in [
        (&dot.id, T1),
        (&dot.id, T0),
        (&"no-such-dot".to_string(), T2),
    ] {
        let err = store
            .record_slot(dot_id, t(at), Some(&scheduled_run(&dot, at)), t(T3))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    }
    assert_eq!(state(&store, &dot).await, before);
    assert!(store.list_runs(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_retry_starts_a_run_without_counting() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    store
        .record_slot(&dot.id, t(T1), None, t(T1))
        .await
        .unwrap();
    busy(&store, &dot, &failed).await;
    let before = state(&store, &dot).await;

    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.retry_of = Some(failed.clone());
    new.payload = Some(json!({ "scheduled_for": T1, "attempt": 2 }));
    let SlotStart::Enqueued(run) = store.start_retry(&dot.id, &new, t(T2)).await.unwrap() else {
        panic!("expected a new run")
    };
    assert_eq!(run.retry_of.as_deref(), Some(failed.as_str()));
    assert_eq!(run.payload, new.payload);
    assert_eq!(
        state(&store, &dot).await,
        ScheduleState {
            watch_run_id: Some(run.id.clone()),
            retry: None,
            updated_at: T2.into(),
            ..before
        }
    );

    // No retry is pending any more.
    let err = store.start_retry(&dot.id, &new, t(T3)).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(store.list_runs(None, 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_retry_joins_the_queued_run() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    busy(&store, &dot, &failed).await;
    let queued = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();

    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.retry_of = Some(failed);
    let start = store.start_retry(&dot.id, &new, t(T2)).await.unwrap();
    assert_eq!(start, SlotStart::Coalesced(queued.clone()));
    let row = state(&store, &dot).await;
    assert_eq!(row.watch_run_id, Some(queued.id));
    assert_eq!((row.retry, row.fired_count), (None, 0));
}

#[tokio::test]
async fn an_update_writes_only_the_fields_it_names() {
    let (_d, store) = temp_store().await;
    let dot = scheduled(&store, "a").await;
    let failed = failed_run(&store, &dot).await;
    busy(&store, &dot, &failed).await;
    let before = state(&store, &dot).await;

    let done = ScheduleChange {
        status: Some(ScheduleStatus::Completed),
        ..Default::default()
    };
    let row = store
        .update_schedule_state(&dot.id, &done, t(T1))
        .await
        .unwrap();
    let expected = ScheduleState {
        status: ScheduleStatus::Completed,
        updated_at: T1.into(),
        ..before
    };
    assert_eq!(row.as_ref(), Some(&expected));
    assert_eq!(state(&store, &dot).await, expected);

    // `Some(None)` clears; a condition on another watched run writes nothing.
    let clear = ScheduleChange {
        watch_run_id: Some(None),
        retry: Some(None),
        attention_run_id: Some(None),
        if_watching: Some("another-run".into()),
        ..Default::default()
    };
    assert_eq!(
        store
            .update_schedule_state(&dot.id, &clear, t(T2))
            .await
            .unwrap(),
        None
    );
    assert_eq!(state(&store, &dot).await, expected);
    let clear = ScheduleChange {
        if_watching: Some(failed.clone()),
        ..clear
    };
    let row = store
        .update_schedule_state(&dot.id, &clear, t(T2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.watch_run_id, row.retry, row.attention_run_id),
        (None, None, None)
    );

    // A pending retry is written whole.
    let later = ScheduleChange {
        retry: Some(Some(retry(6))),
        ..Default::default()
    };
    let row = store
        .update_schedule_state(&dot.id, &later, t(T3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.retry, Some(retry(6)));

    // No row, nothing written.
    assert_eq!(
        store
            .update_schedule_state("no-such-dot", &done, t(T3))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_run_records_the_run_it_retries() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let plain = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(plain.retry_of, None);
    let mut new = NewRun::new(&dot.id, TriggerKind::Schedule);
    new.retry_of = Some(plain.id.clone());
    let retry = store.create_run(&new).await.unwrap();
    assert_eq!(retry.retry_of, Some(plain.id));
}

#[test]
fn timestamps_have_milliseconds_and_a_z() {
    assert_eq!(
        timestamp(t("2026-10-14T11:00:00.1234+02:00")),
        "2026-10-14T09:00:00.123Z"
    );
}
