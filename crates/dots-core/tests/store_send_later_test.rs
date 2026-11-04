//! The `scheduled_messages` rows (send later spec section 4.1): stored and listed in due order,
//! and every change of state made by one conditional update, so the sender, Send now, Edit and
//! Cancel never both win; and what a crash left `sending` recovered at start.

mod common;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::model::{Dot, NewRun, ScheduledStatus, ScheduledWait, TriggerKind};
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::json;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-04T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn at(offset: TimeDelta) -> String {
    timestamp(t0() + offset)
}

/// A time every message of these tests is due by.
fn later() -> DateTime<Utc> {
    t0() + TimeDelta::days(2)
}

async fn dot(store: &Store) -> Dot {
    store
        .create_dot(&spec("courier"), None, "default")
        .await
        .unwrap()
}

async fn message(store: &Store, dot: &Dot, due: TimeDelta) -> String {
    store
        .insert_scheduled_message(&dot.id, None, "Ship it", &at(due), t0())
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn a_scheduled_message_is_stored_and_listed() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let root = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    let later = store
        .insert_scheduled_message(
            &dot.id,
            Some(&root.id),
            "Later",
            &at(TimeDelta::hours(2)),
            t0(),
        )
        .await
        .unwrap();
    let sooner = store
        .insert_scheduled_message(&dot.id, None, "Sooner", &at(TimeDelta::hours(1)), t0())
        .await
        .unwrap();

    let pending = store.pending_scheduled_messages(&dot.id).await.unwrap();
    let ids: Vec<&str> = pending.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, [sooner.id.as_str(), later.id.as_str()]);
    assert!(pending
        .iter()
        .all(|m| m.status == ScheduledStatus::Scheduled && m.waiting.is_none()));
    assert_eq!(pending[0].root_run_id, None);
    assert_eq!(pending[1].root_run_id.as_deref(), Some(root.id.as_str()));
    assert_eq!(pending[1].text, "Later");
    assert_eq!(pending[1].due_at, "2026-11-04T11:00:00.000Z");
}

#[tokio::test]
async fn due_and_next_follow_due_at() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let a = message(&store, &dot, TimeDelta::minutes(-1)).await;
    let b = message(&store, &dot, TimeDelta::zero()).await;
    message(&store, &dot, TimeDelta::minutes(5)).await;
    let cancelled = message(&store, &dot, TimeDelta::minutes(-2)).await;
    store
        .cancel_scheduled_message(&cancelled, t0())
        .await
        .unwrap();
    let sent = message(&store, &dot, TimeDelta::minutes(-3)).await;
    store.claim_scheduled_message(&sent, later()).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    store
        .mark_scheduled_sent(&sent, &run.id, t0())
        .await
        .unwrap();

    let due = store
        .due_scheduled_messages(&timestamp(t0()))
        .await
        .unwrap();
    let ids: Vec<&str> = due.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, [a.as_str(), b.as_str()]);
    assert_eq!(
        store.next_scheduled_due(&timestamp(t0())).await.unwrap(),
        Some(at(TimeDelta::minutes(5)))
    );
    assert_eq!(
        store
            .next_scheduled_due(&at(TimeDelta::minutes(5)))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_claim_happens_once() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::hours(1)).await;

    let claimed = store.claim_scheduled_message(&id, later()).await.unwrap();
    assert_eq!(claimed.unwrap().status, ScheduledStatus::Sending);
    assert_eq!(
        store.claim_scheduled_message(&id, later()).await.unwrap(),
        None
    );
    assert_eq!(
        store
            .edit_scheduled_message(&id, "Other", &at(TimeDelta::hours(2)), t0())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.cancel_scheduled_message(&id, t0()).await.unwrap(),
        None
    );
    assert_eq!(
        store.get_scheduled_message(&id).await.unwrap().text,
        "Ship it"
    );
}

#[tokio::test]
async fn a_message_not_yet_due_is_not_claimed() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::hours(1)).await;
    assert_eq!(
        store.claim_scheduled_message(&id, t0()).await.unwrap(),
        None
    );
    let at_due = t0() + TimeDelta::hours(1);
    assert!(store
        .claim_scheduled_message(&id, at_due)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn edit_and_cancel_only_from_scheduled_or_failed() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::hours(1)).await;
    let edited = store
        .edit_scheduled_message(&id, "Ship it today", &at(TimeDelta::hours(3)), t0())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited.text, "Ship it today");
    assert_eq!(edited.due_at, at(TimeDelta::hours(3)));
    assert_eq!(edited.status, ScheduledStatus::Scheduled);

    store
        .mark_scheduled_failed(&id, "org removed", later())
        .await
        .unwrap()
        .unwrap();
    let edited = store
        .edit_scheduled_message(&id, "Again", &at(TimeDelta::hours(4)), t0())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited.status, ScheduledStatus::Scheduled);
    assert_eq!(edited.error, None);

    store
        .mark_scheduled_failed(&id, "org removed", later())
        .await
        .unwrap()
        .unwrap();
    let cancelled = store.cancel_scheduled_message(&id, t0()).await.unwrap();
    assert_eq!(cancelled.unwrap().status, ScheduledStatus::Cancelled);

    let sent = message(&store, &dot, TimeDelta::hours(1)).await;
    store.claim_scheduled_message(&sent, later()).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    store
        .mark_scheduled_sent(&sent, &run.id, t0())
        .await
        .unwrap();
    assert_eq!(
        store
            .edit_scheduled_message(&sent, "x", &at(TimeDelta::hours(2)), t0())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.cancel_scheduled_message(&sent, t0()).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn waiting_changes_only_when_it_differs() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::zero()).await;

    let busy = store
        .set_scheduled_waiting(&id, ScheduledWait::Busy, t0())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(busy.waiting, Some(ScheduledWait::Busy));
    assert_eq!(
        store
            .set_scheduled_waiting(&id, ScheduledWait::Busy, t0())
            .await
            .unwrap(),
        None
    );
    let approval = store
        .set_scheduled_waiting(&id, ScheduledWait::Approval, t0())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.waiting, Some(ScheduledWait::Approval));

    store.claim_scheduled_message(&id, later()).await.unwrap();
    let back = store
        .set_scheduled_waiting(&id, ScheduledWait::Approval, t0())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.status, ScheduledStatus::Scheduled);
    assert_eq!(back.waiting, Some(ScheduledWait::Approval));
}

/// A pass reads every due row first; an Edit that moved one later meanwhile wins over the
/// pass's stale waiting reason and its stale refusal.
#[tokio::test]
async fn a_message_not_yet_due_neither_waits_nor_fails() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::hours(1)).await;
    assert_eq!(
        store
            .set_scheduled_waiting(&id, ScheduledWait::Busy, t0())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .mark_scheduled_failed(&id, "org removed", t0())
            .await
            .unwrap(),
        None
    );
    let row = store.get_scheduled_message(&id).await.unwrap();
    assert_eq!(row.status, ScheduledStatus::Scheduled);
    assert_eq!((row.waiting, row.error), (None, None));

    // From `sending` (a send that lost its race, or was refused) the due time does not matter.
    let sending = message(&store, &dot, TimeDelta::zero()).await;
    store
        .claim_scheduled_message(&sending, t0())
        .await
        .unwrap()
        .unwrap();
    store
        .edit_scheduled_message(&id, "x", &at(TimeDelta::hours(3)), t0())
        .await
        .unwrap();
    let failed = store
        .mark_scheduled_failed(&sending, "org removed", t0() - TimeDelta::hours(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, ScheduledStatus::Failed);
}

#[tokio::test]
async fn sent_and_failed_record_their_outcome() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::zero()).await;
    store
        .set_scheduled_waiting(&id, ScheduledWait::Busy, t0())
        .await
        .unwrap();
    store.claim_scheduled_message(&id, later()).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    let later = t0() + TimeDelta::minutes(2);
    let sent = store
        .mark_scheduled_sent(&id, &run.id, later)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent.status, ScheduledStatus::Sent);
    assert_eq!(sent.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(sent.sent_at, Some(timestamp(later)));
    assert_eq!(sent.waiting, None);

    let other = message(&store, &dot, TimeDelta::zero()).await;
    let failed = store
        .mark_scheduled_failed(&other, "org removed", t0())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, ScheduledStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("org removed"));
}

#[tokio::test]
async fn reset_for_send_now_moves_due_at_to_now() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::days(1)).await;
    store
        .mark_scheduled_failed(&id, "org removed", later())
        .await
        .unwrap()
        .unwrap();
    let reset = store.reset_for_send_now(&id, t0()).await.unwrap().unwrap();
    assert_eq!(reset.due_at, timestamp(t0()));
    assert_eq!(reset.error, None);
    assert_eq!(reset.status, ScheduledStatus::Scheduled);
}

#[tokio::test]
async fn recovery_finds_the_run_a_crash_left() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let first = message(&store, &dot, TimeDelta::zero()).await;
    let second = message(&store, &dot, TimeDelta::zero()).await;
    store
        .claim_scheduled_message(&first, later())
        .await
        .unwrap();
    store
        .claim_scheduled_message(&second, later())
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({
        "message": "Ship it",
        "typed": true,
        "send_later": { "id": first, "due_at": at(TimeDelta::zero()), "late": false, "new_topic": false },
    }));
    let run = store.create_run(&new).await.unwrap();

    let changed = store.recover_sending_messages(t0()).await.unwrap();
    assert_eq!(changed.len(), 2);
    let first = store.get_scheduled_message(&first).await.unwrap();
    assert_eq!(first.status, ScheduledStatus::Sent);
    assert_eq!(first.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(first.sent_at.as_deref(), Some(run.queued_at.as_str()));
    let second = store.get_scheduled_message(&second).await.unwrap();
    assert_eq!(second.status, ScheduledStatus::Scheduled);
    assert_eq!(second.run_id, None);
}

/// While Dots runs, only a send whose run is stored is settled: a row with no run yet may be
/// on its way through the chat path.
#[tokio::test]
async fn a_send_with_its_run_stored_is_settled_and_one_on_its_way_is_left() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let stored = message(&store, &dot, TimeDelta::zero()).await;
    let on_its_way = message(&store, &dot, TimeDelta::zero()).await;
    for id in [&stored, &on_its_way] {
        store.claim_scheduled_message(id, t0()).await.unwrap();
    }
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({
        "message": "Ship it",
        "typed": true,
        "send_later": { "id": stored, "due_at": at(TimeDelta::zero()), "late": false, "new_topic": false },
    }));
    let run = store.create_run(&new).await.unwrap();

    let changed = store.settle_sending_with_runs(later()).await.unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].id, stored);
    assert_eq!(changed[0].status, ScheduledStatus::Sent);
    assert_eq!(changed[0].run_id.as_deref(), Some(run.id.as_str()));
    let left = store.get_scheduled_message(&on_its_way).await.unwrap();
    assert_eq!(left.status, ScheduledStatus::Sending);
    assert!(store
        .settle_sending_with_runs(later())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn rows_go_with_their_dot() {
    let (_d, store) = temp_store().await;
    let dot = dot(&store).await;
    let id = message(&store, &dot, TimeDelta::hours(1)).await;
    store.delete_dot(&dot.id).await.unwrap();
    assert!(store.get_scheduled_message(&id).await.is_err());
}
