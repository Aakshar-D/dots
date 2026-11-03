//! Delivery tracking's storage (spec sections 2, 3.9 and 4.1): the columns from migration 0029,
//! the marks a run gets as it ends, the day's counts and the Needs you term.

mod common;

use chrono::{DateTime, Utc};
use common::{spec, temp_store};
use dots_core::model::{Delivery, DeliveryCounts, NewRun, Run, RunStatus, TriggerKind};
use dots_core::policy::{Policy, Preset};
use dots_core::store::Store;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

async fn dot(store: &Store, name: &str) -> String {
    store
        .create_dot(&spec(name), None, "default")
        .await
        .unwrap()
        .id
}

async fn queued(store: &Store, dot_id: &str, trigger: TriggerKind) -> Run {
    store
        .create_run(&NewRun::new(dot_id, trigger))
        .await
        .unwrap()
}

async fn claimed(store: &Store, dot_id: &str, trigger: TriggerKind) -> Run {
    let run = queued(store, dot_id, trigger).await;
    assert!(store.claim_run(&run.id).await.unwrap());
    store.get_run(&run.id).await.unwrap()
}

#[tokio::test]
async fn a_new_run_is_waiting() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "new").await;
    let run = queued(&store, &d, TriggerKind::Chat).await;
    assert_eq!(run.delivery, Some(Delivery::Waiting));
    assert_eq!(run.delivered_at, None);
    assert_eq!(run.undelivered_at, None);
    assert_eq!(run.undelivered_reason, None);
    assert_eq!(run.dismissed_at, None);
    assert_eq!(run.wait_reason, None);
}

#[tokio::test]
async fn mark_delivered_sets_once() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "once").await;
    let run = claimed(&store, &d, TriggerKind::Chat).await;
    let first = store
        .mark_delivered(&run.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap()
        .expect("first call marks");
    assert_eq!(first.delivery, Some(Delivery::Delivered));
    let stamp = first.delivered_at.clone().unwrap();
    assert!(store
        .mark_delivered(&run.id, at("2026-11-03T11:00:00Z"))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_run(&run.id).await.unwrap().delivered_at,
        Some(stamp)
    );
}

#[tokio::test]
async fn delivery_clears_an_earlier_mark() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "clears").await;
    let run = queued(&store, &d, TriggerKind::Chat).await;
    let marked = store
        .mark_undelivered(&run.id, "Dots is updating", at("2026-11-03T10:00:00Z"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(marked.delivery, Some(Delivery::NotDelivered));
    assert_eq!(
        marked.undelivered_reason.as_deref(),
        Some("Dots is updating")
    );
    // A second mark changes nothing.
    assert!(store
        .mark_undelivered(&run.id, "other", at("2026-11-03T10:05:00Z"))
        .await
        .unwrap()
        .is_none());
    let delivered = store
        .mark_delivered(&run.id, at("2026-11-03T10:31:00Z"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.delivery, Some(Delivery::Delivered));
    assert_eq!(delivered.undelivered_at, None);
    assert_eq!(delivered.undelivered_reason, None);
}

#[tokio::test]
async fn mark_undelivered_skips_delivered_and_dismissed_runs() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "skips").await;
    let delivered = claimed(&store, &d, TriggerKind::Chat).await;
    store
        .mark_delivered(&delivered.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    assert!(store
        .mark_undelivered(&delivered.id, "r", at("2026-11-03T10:30:00Z"))
        .await
        .unwrap()
        .is_none());

    let dismissed = queued(&store, &d, TriggerKind::Chat).await;
    store
        .dismiss_run(&dismissed.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    assert!(store
        .mark_undelivered(&dismissed.id, "r", at("2026-11-03T10:30:00Z"))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_run(&dismissed.id).await.unwrap().undelivered_at,
        None
    );
}

/// The sweeper reads its overdue runs, then marks them: a run cancelled or claimed between the
/// read and the mark is not marked (spec section 3.9.1: a cancelled run is never "not
/// delivered").
#[tokio::test]
async fn mark_undelivered_skips_a_run_cancelled_or_claimed_after_the_sweep_read_it() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "race").await;
    let cancelled = queued(&store, &d, TriggerKind::Chat).await;
    let claimed_run = queued(&store, &d, TriggerKind::Chat).await;
    for id in [&cancelled.id, &claimed_run.id] {
        store
            .set_run_queued_at(id, "2026-11-03T09:00:00.000Z")
            .await
            .unwrap();
    }
    let overdue = store
        .overdue_undelivered("2026-11-03T09:30:00.000Z")
        .await
        .unwrap();
    assert_eq!(overdue.len(), 2);

    store
        .cancel_if_inactive(&cancelled.id)
        .await
        .unwrap()
        .unwrap();
    assert!(store.claim_run(&claimed_run.id).await.unwrap());
    for run in &overdue {
        assert!(store
            .mark_undelivered(&run.id, "next in line", at("2026-11-03T09:31:00Z"))
            .await
            .unwrap()
            .is_none());
        assert_eq!(store.get_run(&run.id).await.unwrap().undelivered_at, None);
    }
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 0);
}

#[tokio::test]
async fn a_failed_run_never_delivered_is_marked_in_the_same_write() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "failed").await;

    let run = claimed(&store, &d, TriggerKind::Chat).await;
    let ended = store
        .finish_run(
            &run.id,
            RunStatus::Failed,
            None,
            Some("claude CLI not found\nsecond line"),
        )
        .await
        .unwrap();
    assert_eq!(ended.delivery, Some(Delivery::NotDelivered));
    assert!(ended.undelivered_at.is_some());
    assert_eq!(
        ended.undelivered_reason.as_deref(),
        Some("claude CLI not found")
    );

    // No error: the reason is `failed`.
    let bare = claimed(&store, &d, TriggerKind::Chat).await;
    let bare = store
        .finish_run(&bare.id, RunStatus::Failed, None, None)
        .await
        .unwrap();
    assert_eq!(bare.undelivered_reason.as_deref(), Some("failed"));

    // A delivered run that fails stays delivered.
    let ok = claimed(&store, &d, TriggerKind::Chat).await;
    store
        .mark_delivered(&ok.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    let ok = store
        .finish_run(&ok.id, RunStatus::Failed, None, Some("boom"))
        .await
        .unwrap();
    assert_eq!(ok.delivery, Some(Delivery::Delivered));
    assert_eq!(ok.undelivered_at, None);
    assert_eq!(ok.undelivered_reason, None);

    // `fail_if_running` marks the same way.
    let panicked = claimed(&store, &d, TriggerKind::Chat).await;
    let panicked = store
        .fail_if_running(&panicked.id, "internal error: engine task panicked")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        panicked.undelivered_reason.as_deref(),
        Some("internal error: engine task panicked")
    );
    assert_eq!(panicked.delivery, Some(Delivery::NotDelivered));
}

#[tokio::test]
async fn a_cancelled_run_is_not_marked() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "cancel").await;

    let q = queued(&store, &d, TriggerKind::Chat).await;
    let q = store.cancel_if_inactive(&q.id).await.unwrap().unwrap();
    assert_eq!(q.status, RunStatus::Cancelled);
    assert_eq!(q.undelivered_at, None);
    assert_eq!(q.delivery, None);

    let c = claimed(&store, &d, TriggerKind::Chat).await;
    let c = store
        .finish_run(&c.id, RunStatus::Cancelled, None, Some("cancelled"))
        .await
        .unwrap();
    assert_eq!(c.undelivered_at, None);
    assert_eq!(c.delivery, None);

    // Marked first, then cancelled: the mark stays and the run is dismissed.
    let m = queued(&store, &d, TriggerKind::Chat).await;
    store
        .mark_undelivered(&m.id, "waited", at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    let m = store.cancel_if_inactive(&m.id).await.unwrap().unwrap();
    assert!(m.undelivered_at.is_some());
    assert!(m.dismissed_at.is_some());

    // Marked while queued, then claimed and cancelled: the mark stays and the run is dismissed.
    let m2 = queued(&store, &d, TriggerKind::Chat).await;
    store
        .mark_undelivered(&m2.id, "waited", at("2026-11-03T10:00:00Z"))
        .await
        .unwrap()
        .unwrap();
    assert!(store.claim_run(&m2.id).await.unwrap());
    let m2 = store
        .finish_run(&m2.id, RunStatus::Cancelled, None, None)
        .await
        .unwrap();
    assert!(m2.undelivered_at.is_some());
    assert!(m2.dismissed_at.is_some());
}

#[tokio::test]
async fn an_interrupted_run_is_not_delivered_only_if_it_never_started() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "interrupted").await;
    let never = claimed(&store, &d, TriggerKind::Chat).await;
    let started = claimed(&store, &d, TriggerKind::Chat).await;
    store
        .mark_delivered(&started.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    let ids = store.recover_interrupted().await.unwrap();
    assert_eq!(ids.len(), 2);

    let never = store.get_run(&never.id).await.unwrap();
    assert_eq!(never.status, RunStatus::Failed);
    assert_eq!(never.delivery, Some(Delivery::NotDelivered));
    assert_eq!(never.undelivered_reason.as_deref(), Some("interrupted"));
    let started = store.get_run(&started.id).await.unwrap();
    assert_eq!(started.delivery, Some(Delivery::Delivered));
    assert_eq!(started.undelivered_at, None);
}

#[tokio::test]
async fn dismiss_run_happens_once() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "dismiss").await;
    let run = queued(&store, &d, TriggerKind::Chat).await;
    let first = store
        .dismiss_run(&run.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap()
        .unwrap();
    let stamp = first.dismissed_at.clone().unwrap();
    assert!(store
        .dismiss_run(&run.id, at("2026-11-03T11:00:00Z"))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_run(&run.id).await.unwrap().dismissed_at,
        Some(stamp)
    );
}

#[tokio::test]
async fn undismiss_run_clears_it() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "undismiss").await;
    let run = queued(&store, &d, TriggerKind::Chat).await;
    store
        .dismiss_run(&run.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(
        store.undismiss_run(&run.id).await.unwrap().dismissed_at,
        None
    );
    assert!(store
        .dismiss_run(&run.id, at("2026-11-03T10:01:00Z"))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn overdue_undelivered_lists_only_queued_unmarked_runs_queued_before_the_cut() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "overdue").await;
    let old_b = queued(&store, &d, TriggerKind::Chat).await;
    let old_a = queued(&store, &d, TriggerKind::Schedule).await;
    let at_cut = queued(&store, &d, TriggerKind::Chat).await;
    let fresh = queued(&store, &d, TriggerKind::Chat).await;
    let marked = queued(&store, &d, TriggerKind::Chat).await;
    let dismissed = queued(&store, &d, TriggerKind::Chat).await;
    let started = claimed(&store, &d, TriggerKind::Chat).await;
    let delivered = queued(&store, &d, TriggerKind::Chat).await;

    store
        .set_run_queued_at(&old_a.id, "2026-11-03T09:00:00.000Z")
        .await
        .unwrap();
    store
        .set_run_queued_at(&old_b.id, "2026-11-03T09:30:00.000Z")
        .await
        .unwrap();
    store
        .set_run_queued_at(&at_cut.id, "2026-11-03T10:00:00.000Z")
        .await
        .unwrap();
    store
        .set_run_queued_at(&fresh.id, "2026-11-03T10:00:00.001Z")
        .await
        .unwrap();
    for id in [&marked.id, &dismissed.id, &started.id, &delivered.id] {
        store
            .set_run_queued_at(id, "2026-11-03T08:00:00.000Z")
            .await
            .unwrap();
    }
    store
        .mark_undelivered(&marked.id, "r", at("2026-11-03T09:40:00Z"))
        .await
        .unwrap();
    store
        .dismiss_run(&dismissed.id, at("2026-11-03T09:40:00Z"))
        .await
        .unwrap();
    store
        .mark_delivered(&delivered.id, at("2026-11-03T09:40:00Z"))
        .await
        .unwrap();

    let ids: Vec<String> = store
        .overdue_undelivered("2026-11-03T10:00:00.000Z")
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![old_a.id, old_b.id, at_cut.id]);
}

#[tokio::test]
async fn needs_you_counts_undelivered_chat_runs_only() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "needs").await;
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 0);

    let chat = claimed(&store, &d, TriggerKind::Chat).await;
    store
        .finish_run(&chat.id, RunStatus::Failed, None, Some("no engine"))
        .await
        .unwrap();
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 1);

    let watcher = claimed(&store, &d, TriggerKind::Watcher).await;
    store
        .finish_run(&watcher.id, RunStatus::Failed, None, Some("no engine"))
        .await
        .unwrap();
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 1);

    store
        .dismiss_run(&chat.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 0);

    // Marked and then delivered: not counted.
    let late = queued(&store, &d, TriggerKind::Chat).await;
    store
        .mark_undelivered(&late.id, "waited", at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 1);
    store
        .mark_delivered(&late.id, at("2026-11-03T10:40:00Z"))
        .await
        .unwrap();
    assert_eq!(store.dot_counts(&d).await.unwrap().0, 0);
}

#[tokio::test]
async fn delivery_counts_cover_the_day_and_leave_out_resumes() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "counts").await;
    let other = dot(&store, "other").await;

    let delivered = claimed(&store, &d, TriggerKind::Chat).await;
    store
        .mark_delivered(&delivered.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    let dismissed = queued(&store, &d, TriggerKind::Chat).await;
    store
        .mark_undelivered(&dismissed.id, "waited", at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    store
        .dismiss_run(&dismissed.id, at("2026-11-03T10:05:00Z"))
        .await
        .unwrap();
    let late = queued(&store, &d, TriggerKind::Schedule).await;
    let outside = queued(&store, &d, TriggerKind::Chat).await;
    let resume = queued(&store, &d, TriggerKind::Resume).await;
    let elsewhere = queued(&store, &other, TriggerKind::Chat).await;

    for (run, when) in [
        (&delivered, "2026-11-03T00:00:00.000Z"),
        (&dismissed, "2026-11-03T12:00:00.000Z"),
        (&late, "2026-11-03T23:59:59.999Z"),
        (&outside, "2026-11-04T00:00:00.000Z"),
        (&resume, "2026-11-03T12:00:00.000Z"),
        (&elsewhere, "2026-11-03T12:00:00.000Z"),
    ] {
        store.set_run_queued_at(&run.id, when).await.unwrap();
    }

    let counts = store
        .delivery_counts(&d, "2026-11-03T00:00:00.000Z", "2026-11-04T00:00:00.000Z")
        .await
        .unwrap();
    assert_eq!(
        counts,
        DeliveryCounts {
            sent: 3,
            delivered: 1,
            not_delivered: 1
        }
    );
    let none = store
        .delivery_counts(&d, "2026-12-01T00:00:00.000Z", "2026-12-02T00:00:00.000Z")
        .await
        .unwrap();
    assert_eq!(
        none,
        DeliveryCounts {
            sent: 0,
            delivered: 0,
            not_delivered: 0
        }
    );
}

#[tokio::test]
async fn a_requeue_is_not_a_second_send() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "requeue").await;
    let queued_run = queued(&store, &d, TriggerKind::Schedule).await;
    assert!(store.claim_run(&queued_run.id).await.unwrap());
    store
        .mark_delivered(&queued_run.id, at("2026-11-03T10:00:00Z"))
        .await
        .unwrap();
    let back = store
        .requeue_run(&queued_run.id, &queued_run)
        .await
        .unwrap();
    assert_eq!(back.status, RunStatus::Queued);
    assert!(back.delivered_at.is_some());
    assert_eq!(back.delivery, Some(Delivery::Delivered));

    let counts = store
        .delivery_counts(&d, "2000-01-01T00:00:00.000Z", "2100-01-01T00:00:00.000Z")
        .await
        .unwrap();
    assert_eq!(counts.sent, 1);
    assert_eq!(counts.delivered, 1);
}

#[tokio::test]
async fn migration_0029_backfills_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 29)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
         webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
         created_at, updated_at) VALUES ('D1', 'old', 'Do it.', 'claude', 'sonnet', \
         'C:/tmp/repo', 'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(serde_json::to_string(&Policy::preset(Preset::Sandboxed)).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    for (id, started, status) in [
        ("R1", Some("2026-10-02T10:00:01.000Z"), "failed"),
        ("R2", None, "cancelled"),
    ] {
        sqlx::query(
            "INSERT INTO runs (id, dot_id, root_run_id, trigger_kind, status, queued_at, \
             started_at, ended_at) VALUES (?, 'D1', ?, 'chat', ?, '2026-10-02T10:00:00.000Z', ?, \
             '2026-10-02T10:05:00.000Z')",
        )
        .bind(id)
        .bind(id)
        .bind(status)
        .bind(started)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let started = store.get_run("R1").await.unwrap();
    assert_eq!(started.delivered_at, started.started_at);
    assert!(started.delivered_at.is_some());
    assert_eq!(started.dismissed_at, None);
    assert_eq!(started.delivery, Some(Delivery::Delivered));
    let unstarted = store.get_run("R2").await.unwrap();
    assert_eq!(unstarted.delivered_at, None);
    assert_eq!(unstarted.dismissed_at, unstarted.ended_at);
    assert!(unstarted.dismissed_at.is_some());
    assert_eq!(unstarted.delivery, None);
    assert_eq!(store.dot_counts("D1").await.unwrap().0, 0);
}
