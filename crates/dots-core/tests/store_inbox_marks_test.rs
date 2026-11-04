//! The Inbox marks (snooze and done spec section 4.2): snooze, done and their undo, the wake of
//! due snoozes, pruning, and the counts that leave hidden items out. Ids and times are given
//! explicitly, and no runner runs.

mod common;

use std::collections::HashSet;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::apps::propose::AppProposal;
use dots_core::inbox::{InboxKey, InboxKind, MarkState};
use dots_core::model::{
    Dot, FolderConfig, NewRun, PausedReason, Run, RunStatus, TriggerKind, WatcherConfig,
};
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tempfile::TempDir;

const T: &str = "2026-11-04T10:00:00.000Z";

fn t() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T).unwrap().with_timezone(&Utc)
}

fn at(minutes: i64) -> String {
    timestamp(t() + TimeDelta::minutes(minutes))
}

fn key(kind: InboxKind, id: &str) -> InboxKey {
    InboxKey::new(kind, id)
}

async fn raw_pool(dir: &TempDir) -> SqlitePool {
    let opts = SqliteConnectOptions::new().filename(dir.path().join("dots.db"));
    SqlitePool::connect_with(opts).await.unwrap()
}

async fn dot(store: &Store, name: &str) -> Dot {
    store
        .create_dot(&spec(name), None, "default")
        .await
        .unwrap()
}

async fn run(store: &Store, dot: &Dot, trigger: TriggerKind) -> Run {
    store
        .create_run(&NewRun::new(&dot.id, trigger))
        .await
        .unwrap()
}

#[tokio::test]
async fn snooze_then_done_then_undo() {
    let (_dir, store) = temp_store().await;
    let k = key(InboxKind::Message, "m1");
    store
        .snooze_marks(std::slice::from_ref(&k), &at(60), T)
        .await
        .unwrap();
    let marks = store.list_marks().await.unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].key(), k);
    assert_eq!(marks[0].snoozed_until.as_deref(), Some(at(60).as_str()));
    assert_eq!(
        (marks[0].done_at.as_ref(), marks[0].back_at.as_ref()),
        (None, None)
    );

    store.done_mark(&k, &at(1)).await.unwrap();
    let marks = store.list_marks().await.unwrap();
    assert_eq!(marks[0].done_at.as_deref(), Some(at(1).as_str()));
    assert_eq!(marks[0].snoozed_until, None);

    assert!(!store.delete_mark_if(&k, MarkState::Snoozed).await.unwrap());
    assert!(store.delete_mark_if(&k, MarkState::Done).await.unwrap());
    assert_eq!(store.count_marks().await.unwrap(), 0);
}

#[tokio::test]
async fn a_mark_is_in_one_state() {
    let (dir, store) = temp_store().await;
    let pool = raw_pool(&dir).await;
    let both = sqlx::query(
        "INSERT INTO inbox_marks (kind, item_id, snoozed_until, done_at, marked_at) \
         VALUES ('message', 'm1', ?, ?, ?)",
    )
    .bind(at(60))
    .bind(T)
    .bind(T)
    .execute(&pool)
    .await;
    assert!(both.is_err(), "two states are refused");
    let none = sqlx::query(
        "INSERT INTO inbox_marks (kind, item_id, marked_at) VALUES ('message', 'm2', ?)",
    )
    .bind(T)
    .execute(&pool)
    .await;
    assert!(none.is_err(), "no state is refused");
    assert_eq!(store.count_marks().await.unwrap(), 0);
}

#[tokio::test]
async fn wake_due_wakes_only_what_is_due() {
    let (_dir, store) = temp_store().await;
    let first = key(InboxKind::Message, "m1");
    let second = key(InboxKind::Review, "r1");
    store
        .snooze_marks(std::slice::from_ref(&first), &at(1), T)
        .await
        .unwrap();
    store
        .snooze_marks(std::slice::from_ref(&second), &at(2), T)
        .await
        .unwrap();

    assert_eq!(store.wake_due(&at(1)).await.unwrap(), vec![first.clone()]);
    let marks = store.list_marks().await.unwrap();
    let woken = marks.iter().find(|m| m.key() == first).unwrap();
    assert_eq!(woken.back_at.as_deref(), Some(at(1).as_str()));
    assert_eq!(woken.snoozed_until, None);
    let waiting = marks.iter().find(|m| m.key() == second).unwrap();
    assert_eq!(waiting.snoozed_until.as_deref(), Some(at(2).as_str()));

    assert!(store.wake_due(&at(1)).await.unwrap().is_empty());
}

#[tokio::test]
async fn prune_marks_keeps_only_given_keys() {
    let (_dir, store) = temp_store().await;
    let a = key(InboxKind::Message, "a");
    let b = key(InboxKind::Approval, "b");
    let c = key(InboxKind::Watcher, "c@x");
    store
        .snooze_marks(&[a.clone(), b.clone()], &at(60), T)
        .await
        .unwrap();
    store.done_mark(&c, T).await.unwrap();
    let keep: HashSet<InboxKey> = [a.clone(), c.clone()].into_iter().collect();
    assert_eq!(store.prune_marks(&keep).await.unwrap(), 1);
    let left: HashSet<InboxKey> = store
        .list_marks()
        .await
        .unwrap()
        .iter()
        .map(|m| m.key())
        .collect();
    assert_eq!(left, keep);
}

fn proposal(dot: &Dot, run: &Run, id: &str) -> AppProposal {
    serde_json::from_value(json!({
        "id": id, "dot_id": dot.id, "run_id": run.id, "root_run_id": run.root_run_id,
        "app_name": "notes", "app_id": null, "base_version": null, "note": "Built it.",
        "summary": {
            "title": "Notes", "description": "", "tools": [], "permissions": [],
            "tests": {"results": []}, "files": 0, "bytes": 0, "migrations": [],
            "update_from": null
        },
        "status": "pending", "answer": null, "answer_run_id": null,
        "created_at": T, "decided_at": null
    }))
    .unwrap()
}

#[tokio::test]
async fn dot_counts_leave_out_hidden_items() {
    let (dir, store) = temp_store().await;
    let d = dot(&store, "courier").await;
    let asking = run(&store, &d, TriggerKind::Manual).await;
    let question = store
        .add_question(&d.id, &asking.id, "Ship it?", None)
        .await
        .unwrap()
        .unwrap();
    let waiting = run(&store, &d, TriggerKind::Manual).await;
    let approval = store
        .create_approval(&waiting.id, "Bash", &json!({"command": "ls"}))
        .await
        .unwrap();
    let folder = dir.path().to_string_lossy().to_string();
    let watcher = store
        .add_watcher(
            &d.id,
            "inbox",
            &WatcherConfig::Folder(FolderConfig::new(&folder)),
            None,
        )
        .await
        .unwrap();
    let paused_at = at(0);
    store
        .pause_watcher(&watcher.id, PausedReason::Error, Some("gone"), &paused_at)
        .await
        .unwrap()
        .unwrap();
    let proposing = run(&store, &d, TriggerKind::Manual).await;
    store
        .add_app_proposal(&proposal(&d, &proposing, "p1"))
        .await
        .unwrap();
    let chat = run(&store, &d, TriggerKind::Chat).await;
    store
        .mark_undelivered(&chat.id, "other runs are busy", t())
        .await
        .unwrap()
        .unwrap();

    // Seen long ago: what follows is new.
    sqlx::query("UPDATE dots SET seen_at = '2000-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(&d.id)
        .execute(&raw_pool(&dir).await)
        .await
        .unwrap();
    let note = store
        .add_note(&d.id, None, "Half the tests pass.")
        .await
        .unwrap();
    let failing = run(&store, &d, TriggerKind::Manual).await;
    assert!(store.claim_run(&failing.id).await.unwrap());
    store
        .finish_run(&failing.id, RunStatus::Failed, None, Some("boom"))
        .await
        .unwrap();

    let start = store.dot_counts(&d.id).await.unwrap();
    assert_eq!(start, (5, 7));

    let watcher_key = key(InboxKind::Watcher, &format!("{}@{paused_at}", watcher.id));
    let needs_you = [
        key(InboxKind::Message, &question.id),
        key(InboxKind::Approval, &approval.id),
        watcher_key.clone(),
        key(InboxKind::AppReview, "p1"),
        key(InboxKind::NotDelivered, &chat.id),
    ];
    let mut expected = start;
    for k in &needs_you {
        store
            .snooze_marks(std::slice::from_ref(k), &at(60), T)
            .await
            .unwrap();
        expected = (expected.0 - 1, expected.1 - 1);
        assert_eq!(store.dot_counts(&d.id).await.unwrap(), expected, "{k:?}");
    }
    store
        .done_mark(&key(InboxKind::Message, &note.id), T)
        .await
        .unwrap();
    expected.1 -= 1;
    assert_eq!(store.dot_counts(&d.id).await.unwrap(), expected);
    store
        .done_mark(&key(InboxKind::Review, &failing.id), T)
        .await
        .unwrap();
    expected.1 -= 1;
    assert_eq!(store.dot_counts(&d.id).await.unwrap(), expected);
    assert_eq!(expected, (0, 0));

    // Back from snooze: counted again. The done marks stay.
    assert_eq!(store.wake_due(&at(60)).await.unwrap().len(), 5);
    assert_eq!(store.dot_counts(&d.id).await.unwrap(), (5, 5));

    // A mark of an old pause does not hide the same watcher paused again.
    store
        .snooze_marks(std::slice::from_ref(&watcher_key), &at(120), T)
        .await
        .unwrap();
    assert_eq!(store.dot_counts(&d.id).await.unwrap(), (4, 4));
    store.resume_watcher(&watcher.id).await.unwrap();
    store
        .pause_watcher(&watcher.id, PausedReason::Error, Some("gone"), &at(5))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.dot_counts(&d.id).await.unwrap(), (5, 5));
}

#[tokio::test]
async fn waiting_messages_are_the_ones_that_wait() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "courier").await;
    let asking = run(&store, &d, TriggerKind::Manual).await;
    let question = store
        .add_question(&d.id, &asking.id, "Ship it?", None)
        .await
        .unwrap()
        .unwrap();
    let note = store.add_note(&d.id, None, "Done.").await.unwrap();
    let read = store.add_note(&d.id, None, "Old news.").await.unwrap();
    store.mark_message_read(&read.id).await.unwrap();
    let other = run(&store, &d, TriggerKind::Manual).await;
    let dismissed = store
        .add_question(&d.id, &other.id, "Still?", None)
        .await
        .unwrap()
        .unwrap();
    store.dismiss_question(&dismissed.id).await.unwrap();

    let ids: HashSet<String> = store
        .waiting_messages()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, [question.id, note.id].into_iter().collect());
}

#[tokio::test]
async fn count_waiting_messages_leaves_out_hidden_ones() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "courier").await;
    let asking = run(&store, &d, TriggerKind::Manual).await;
    let question = store
        .add_question(&d.id, &asking.id, "Ship it?", None)
        .await
        .unwrap()
        .unwrap();
    let note = store.add_note(&d.id, None, "Done.").await.unwrap();
    assert_eq!(store.count_waiting_messages().await.unwrap(), 2);
    store
        .snooze_marks(&[key(InboxKind::Message, &question.id)], &at(60), T)
        .await
        .unwrap();
    assert_eq!(store.count_waiting_messages().await.unwrap(), 1);
    store
        .done_mark(&key(InboxKind::Message, &note.id), T)
        .await
        .unwrap();
    assert_eq!(store.count_waiting_messages().await.unwrap(), 0);
    store.wake_due(&at(60)).await.unwrap();
    assert_eq!(store.count_waiting_messages().await.unwrap(), 1);
}

#[tokio::test]
async fn undelivered_chat_runs_lists_only_live_marks() {
    let (_dir, store) = temp_store().await;
    let d = dot(&store, "courier").await;
    let live = run(&store, &d, TriggerKind::Chat).await;
    let dismissed = run(&store, &d, TriggerKind::Chat).await;
    let delivered = run(&store, &d, TriggerKind::Chat).await;
    let background = run(&store, &d, TriggerKind::Schedule).await;
    for (r, minute) in [
        (&live, 1),
        (&dismissed, 2),
        (&delivered, 3),
        (&background, 4),
    ] {
        store
            .mark_undelivered(
                &r.id,
                "other runs are busy",
                t() + TimeDelta::minutes(minute),
            )
            .await
            .unwrap()
            .unwrap();
    }
    store
        .dismiss_run(&dismissed.id, t() + TimeDelta::minutes(5))
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .set_delivered(&delivered.id, t() + TimeDelta::minutes(5))
        .await
        .unwrap());
    let listed: Vec<String> = store
        .undelivered_chat_runs()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(listed, vec![live.id]);
}

#[tokio::test]
async fn migration_0031_creates_the_table() {
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
        .filter(|m| m.version < 31)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.count_marks().await.unwrap(), 0);
}
