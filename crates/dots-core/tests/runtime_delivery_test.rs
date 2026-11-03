//! Retry, Discard and today's counts (delivery tracking spec sections 3.7, 3.9 and 4.4-4.5).
//! The queue is held before any run is created, so no dispatcher claims one mid-test.

mod common;

use std::path::PathBuf;

use chrono::{Local, TimeDelta, TimeZone, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::model::{
    DeliveryCounts, Dot, DotSpec, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::util::timestamp;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

/// Queue times for the runs built by hand, one second apart, long before any run the tests
/// send through the chat path (those are queued at the real time).
const T1: &str = "2026-11-01T10:00:01.000Z";
const T2: &str = "2026-11-01T10:00:02.000Z";
const T3: &str = "2026-11-01T10:00:03.000Z";

async fn start(dir: &TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    // No dispatcher may claim the runs these tests create.
    rt.hold_runs(true).await;
    rt
}

async fn dot(rt: &Runtime, dir: &TempDir) -> Dot {
    let mut s: DotSpec = spec("courier");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    rt.create_dot(s, None, None).await.unwrap()
}

fn chat_run(dot: &Dot, text: &str, parent: Option<&Run>) -> NewRun {
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({ "message": text, "typed": true }));
    if let Some(p) = parent {
        new.parent_run_id = Some(p.id.clone());
        new.root_run_id = Some(p.root_run_id.clone());
        new.session_id = p.session_id.clone();
    }
    new
}

/// A run created, queued at `at`, and still queued.
async fn queued(rt: &Runtime, new: NewRun, at: Option<&str>) -> Run {
    let run = rt.store().create_run(&new).await.unwrap();
    if let Some(at) = at {
        rt.store().set_run_queued_at(&run.id, at).await.unwrap();
    }
    rt.store().get_run(&run.id).await.unwrap()
}

/// A run that failed before its engine started: marked not delivered.
async fn failed_undelivered(rt: &Runtime, new: NewRun, at: &str) -> Run {
    let run = queued(rt, new, Some(at)).await;
    assert!(rt.store().claim_run(&run.id).await.unwrap());
    let run = rt
        .store()
        .finish_run(
            &run.id,
            RunStatus::Failed,
            None,
            Some("claude CLI not found"),
        )
        .await
        .unwrap();
    assert!(run.undelivered_at.is_some());
    run
}

/// A run its engine started on that succeeded with `session`.
async fn succeeded(rt: &Runtime, new: NewRun, at: Option<&str>, session: &str) -> Run {
    let run = queued(rt, new, at).await;
    assert!(rt.store().claim_run(&run.id).await.unwrap());
    rt.store()
        .mark_delivered(&run.id, Utc::now())
        .await
        .unwrap();
    rt.store().set_session(&run.id, session).await.unwrap();
    rt.store()
        .finish_run(&run.id, RunStatus::Succeeded, Some("done"), None)
        .await
        .unwrap()
}

async fn needs_you(rt: &Runtime, dot: &Dot) -> u32 {
    rt.store().dot_counts(&dot.id).await.unwrap().0
}

#[tokio::test]
async fn retry_resends_a_first_message_as_a_new_first_message() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let old = failed_undelivered(&rt, chat_run(&dot, "Ship it", None), T1).await;
    assert_eq!(needs_you(&rt, &dot).await, 1);

    let new = rt.retry_delivery(&old.id).await.unwrap();
    assert_ne!(new.id, old.id);
    assert_eq!(new.trigger, TriggerKind::Chat);
    assert_eq!(new.payload.as_ref().unwrap()["message"], "Ship it");
    assert_eq!(new.parent_run_id, None);
    assert_eq!(new.status, RunStatus::Queued);

    let old = rt.get_run(&old.id).await.unwrap();
    assert!(old.dismissed_at.is_some());
    assert_eq!(needs_you(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn retrying_a_reply_continues_the_same_thread() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = succeeded(&rt, chat_run(&dot, "Hello", None), Some(T1), "s1").await;
    let old = failed_undelivered(&rt, chat_run(&dot, "And then?", Some(&p)), T2).await;
    assert_eq!(old.parent_run_id.as_deref(), Some(p.id.as_str()));

    let new = rt.retry_delivery(&old.id).await.unwrap();
    assert_eq!(new.parent_run_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(new.root_run_id, p.id);
    assert_eq!(new.session_id.as_deref(), Some("s1"));
    assert_eq!(new.payload.as_ref().unwrap()["message"], "And then?");

    // The retried message is now the end of the thread: a reply to it goes through.
    assert!(rt.store().claim_run(&new.id).await.unwrap());
    rt.store().set_session(&new.id, "s2").await.unwrap();
    rt.store()
        .finish_run(&new.id, RunStatus::Succeeded, Some("done"), None)
        .await
        .unwrap();
    let next = rt.chat(&dot.id, "next", Some(&new.id)).await.unwrap();
    assert_eq!(next.parent_run_id.as_deref(), Some(new.id.as_str()));
    assert_eq!(next.root_run_id, p.id);
    rt.shutdown();
}

#[tokio::test]
async fn retrying_twice_sends_once() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let old = failed_undelivered(&rt, chat_run(&dot, "Ship it", None), T1).await;

    rt.retry_delivery(&old.id).await.unwrap();
    let again = rt.retry_delivery(&old.id).await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");

    let runs = rt.list_runs(Some(&dot.id), 100).await.unwrap();
    let sent: Vec<&Run> = runs
        .iter()
        .filter(|r| r.trigger == TriggerKind::Chat && r.id != old.id)
        .collect();
    assert_eq!(sent.len(), 1);
    rt.shutdown();
}

#[tokio::test]
async fn a_refused_retry_leaves_the_message_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = succeeded(&rt, chat_run(&dot, "Hello", None), Some(T1), "s1").await;
    let old = failed_undelivered(&rt, chat_run(&dot, "And then?", Some(&p)), T2).await;
    // A run of the thread is still queued, so the chat path refuses a follow-up. It keeps its
    // real queue time, so the 30-minute sweeper cannot mark it.
    queued(&rt, chat_run(&dot, "Meanwhile", Some(&p)), None).await;

    let refused = rt.retry_delivery(&old.id).await;
    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    let old = rt.get_run(&old.id).await.unwrap();
    assert_eq!(old.dismissed_at, None);
    assert_eq!(needs_you(&rt, &dot).await, 1);
    rt.shutdown();
}

#[tokio::test]
async fn a_message_that_has_just_reached_the_dot_is_neither_retried_nor_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = queued(&rt, chat_run(&dot, "Ship it", None), None).await;
    rt.store()
        .mark_undelivered(&run.id, "Dots is updating", Utc::now())
        .await
        .unwrap()
        .unwrap();
    // The dispatcher claims it; its engine has not started yet.
    assert!(rt.store().claim_run(&run.id).await.unwrap());

    let r = rt.retry_delivery(&run.id).await;
    assert!(matches!(r, Err(Error::Conflict(_))), "{r:?}");
    let d = rt.discard_delivery(&run.id).await;
    assert!(matches!(d, Err(Error::Conflict(_))), "{d:?}");

    let run = rt.get_run(&run.id).await.unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.dismissed_at, None);
    assert_eq!(rt.list_runs(Some(&dot.id), 100).await.unwrap().len(), 1);
    rt.shutdown();
}

#[tokio::test]
async fn retry_and_discard_refuse_what_they_do_not_apply_to() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;

    let unknown = rt.retry_delivery("no-such-run").await;
    assert!(matches!(unknown, Err(Error::NotFound(_))), "{unknown:?}");
    let unknown = rt.discard_delivery("no-such-run").await;
    assert!(matches!(unknown, Err(Error::NotFound(_))), "{unknown:?}");

    let schedule = failed_undelivered(&rt, NewRun::new(&dot.id, TriggerKind::Schedule), T1).await;
    let delivered = succeeded(&rt, chat_run(&dot, "Hi", None), Some(T2), "s1").await;
    // Left at the real queue time, so the 30-minute sweeper does not mark it.
    let waiting = queued(&rt, chat_run(&dot, "Hold on", None), None).await;
    let dismissed = failed_undelivered(&rt, chat_run(&dot, "Gone", None), T3).await;
    rt.store()
        .dismiss_run(&dismissed.id, Utc::now())
        .await
        .unwrap()
        .unwrap();

    for run in [&schedule, &delivered, &waiting, &dismissed] {
        let r = rt.retry_delivery(&run.id).await;
        assert!(
            matches!(r, Err(Error::Conflict(_))),
            "retry {}: {r:?}",
            run.id
        );
        let d = rt.discard_delivery(&run.id).await;
        assert!(
            matches!(d, Err(Error::Conflict(_))),
            "discard {}: {d:?}",
            run.id
        );
    }
    // Nothing was sent, and the waiting run still waits.
    assert_eq!(rt.list_runs(Some(&dot.id), 100).await.unwrap().len(), 4);
    assert_eq!(
        rt.get_run(&waiting.id).await.unwrap().status,
        RunStatus::Queued
    );
    rt.shutdown();
}

#[tokio::test]
async fn discard_dismisses_and_cancels_a_waiting_message() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = queued(&rt, chat_run(&dot, "Ship it", None), None).await;
    rt.store()
        .mark_undelivered(&run.id, "Dots is updating", Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(needs_you(&rt, &dot).await, 1);

    let run = rt.discard_delivery(&run.id).await.unwrap();
    assert_eq!(run.status, RunStatus::Cancelled);
    assert!(run.dismissed_at.is_some());
    assert!(run.undelivered_at.is_some());
    assert_eq!(needs_you(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn discard_dismisses_a_message_that_failed() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = failed_undelivered(&rt, chat_run(&dot, "Ship it", None), T1).await;

    let run = rt.discard_delivery(&run.id).await.unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.dismissed_at.is_some());
    assert_eq!(needs_you(&rt, &dot).await, 0);
    assert_eq!(rt.list_runs(Some(&dot.id), 100).await.unwrap().len(), 1);
    rt.shutdown();
}

#[tokio::test]
async fn retry_of_a_waiting_message_cancels_it_and_sends_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let old = queued(&rt, chat_run(&dot, "Ship it", None), None).await;
    rt.store()
        .mark_undelivered(&old.id, "Dots is updating", Utc::now())
        .await
        .unwrap()
        .unwrap();

    let new = rt.retry_delivery(&old.id).await.unwrap();
    assert_ne!(new.id, old.id);
    assert_eq!(new.status, RunStatus::Queued);
    let old = rt.get_run(&old.id).await.unwrap();
    assert_eq!(old.status, RunStatus::Cancelled);
    assert!(old.dismissed_at.is_some());
    assert_eq!(needs_you(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn delivery_today_counts_the_dot() {
    let dir = tempfile::tempdir().unwrap();
    // The runtime's clock stands at local noon of a fixed day, far from either midnight.
    let now = Local
        .with_ymd_and_hms(2026, 11, 3, 12, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.clock = FakeClock::new(now);
    let rt = Runtime::start(cfg).await.unwrap();
    rt.hold_runs(true).await;
    let dot = dot(&rt, &dir).await;
    let one = timestamp(now - TimeDelta::seconds(1));
    let two = timestamp(now - TimeDelta::seconds(2));
    succeeded(&rt, chat_run(&dot, "Hi", None), Some(&two), "s1").await;
    failed_undelivered(&rt, chat_run(&dot, "Hello?", None), &one).await;

    let counts = rt.delivery_today(&dot.id).await.unwrap();
    assert_eq!(
        counts,
        DeliveryCounts {
            sent: 2,
            delivered: 1,
            not_delivered: 1
        }
    );

    let unknown = rt.delivery_today("no-such-dot").await;
    assert!(matches!(unknown, Err(Error::NotFound(_))), "{unknown:?}");
    rt.shutdown();
}

/// Why a run waits only explains the wait: when it cannot be read, the runs are still listed
/// and read, with no reason (here the dots table is gone, so the usage hold cannot be read).
#[tokio::test]
async fn a_wait_reason_that_cannot_be_read_fails_no_read_of_the_runs() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = queued(&rt, chat_run(&dot, "Ship it", None), None).await;
    assert!(rt.get_run(&run.id).await.unwrap().wait_reason.is_some());

    let db = dir.path().join("data").join(dots_core::store::DB_FILE);
    let pool = sqlx::sqlite::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(&db),
    )
    .await
    .unwrap();
    sqlx::query("ALTER TABLE dots RENAME TO dots_gone")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let listed = rt.list_runs(Some(&dot.id), 100).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].wait_reason, None);
    assert_eq!(rt.get_run(&run.id).await.unwrap().wait_reason, None);
    let thread = rt.run_thread(&run.id).await.unwrap();
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0].wait_reason, None);
    rt.shutdown();
}
