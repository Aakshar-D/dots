//! The stuck-run watchdog's columns (spec section 4.3): a dot's quiet limit, a run's quiet mark,
//! the limit it started with and the watchdog's ending, from migration 0026.

mod common;

use common::{spec, temp_store};
use dots_core::model::{NewRun, Run, RunStatus, TriggerKind};
use dots_core::policy::{Policy, Preset};
use dots_core::store::Store;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

async fn running_run(store: &Store, name: &str) -> (String, Run) {
    let dot = store
        .create_dot(&spec(name), None, "default")
        .await
        .unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store.claim_run(&run.id).await.unwrap());
    (dot.id, store.get_run(&run.id).await.unwrap())
}

#[tokio::test]
async fn a_dot_keeps_its_quiet_limit() {
    let (_dir, store) = temp_store().await;
    let mut s = spec("quiet");
    assert_eq!(s.quiet_limit_secs, None);
    s.quiet_limit_secs = Some(900);
    let dot = store.create_dot(&s, None, "default").await.unwrap();
    assert_eq!(dot.spec.quiet_limit_secs, Some(900));
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.quiet_limit_secs,
        Some(900)
    );

    let mut off = dot.spec.clone();
    off.quiet_limit_secs = Some(0);
    let dot = store.update_dot(&dot.id, &off).await.unwrap();
    assert_eq!(dot.spec.quiet_limit_secs, Some(0));
    let mut none = dot.spec.clone();
    none.quiet_limit_secs = None;
    let dot = store.update_dot(&dot.id, &none).await.unwrap();
    assert_eq!(dot.spec.quiet_limit_secs, None);
}

#[tokio::test]
async fn a_dot_from_before_0026_reads_no_quiet_limit() {
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
        .filter(|m| m.version < 26)
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
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let read = store.get_dot("D1").await.unwrap();
    assert_eq!(read.spec.quiet_limit_secs, None);
    let saved = store.update_dot("D1", &read.spec).await.unwrap();
    assert_eq!(saved.spec.quiet_limit_secs, None);
}

#[tokio::test]
async fn the_quiet_mark_is_set_cleared_and_counted() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = running_run(&store, "marked").await;
    assert_eq!(run.quiet_since, None);
    let ts = "2026-11-02T10:00:00.000Z";
    let marked = store.set_run_quiet(&run.id, Some(ts)).await.unwrap();
    assert_eq!(marked.quiet_since.as_deref(), Some(ts));
    assert_eq!(store.dot_counts(&dot).await.unwrap().0, 1);
    let cleared = store.set_run_quiet(&run.id, None).await.unwrap();
    assert_eq!(cleared.quiet_since, None);
    assert_eq!(store.dot_counts(&dot).await.unwrap().0, 0);
}

#[tokio::test]
async fn ending_a_run_clears_its_mark_and_records_the_watchdog() {
    let (_dir, store) = temp_store().await;
    let (_, run) = running_run(&store, "watched").await;
    store
        .set_run_quiet(&run.id, Some("2026-11-02T10:00:00.000Z"))
        .await
        .unwrap();
    let done = store
        .finish_run_with(
            &run.id,
            RunStatus::Failed,
            None,
            Some("stopped: no output for 20 min"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(done.quiet_since, None);
    assert!(done.quiet_stopped);
    assert_eq!(done.status, RunStatus::Failed);
    // A mark is only set while the run is running.
    let late = store
        .set_run_quiet(&run.id, Some("2026-11-02T11:00:00.000Z"))
        .await
        .unwrap();
    assert_eq!(late.quiet_since, None);

    let (_, plain) = running_run(&store, "plain").await;
    store
        .set_run_quiet(&plain.id, Some("2026-11-02T10:00:00.000Z"))
        .await
        .unwrap();
    let done = store
        .finish_run(&plain.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    assert_eq!(done.quiet_since, None);
    assert!(!done.quiet_stopped);
}

#[tokio::test]
async fn a_restart_clears_the_mark() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = running_run(&store, "restarted").await;
    store
        .set_run_quiet(&run.id, Some("2026-11-02T10:00:00.000Z"))
        .await
        .unwrap();
    store.recover_interrupted().await.unwrap();
    let after = store.get_run(&run.id).await.unwrap();
    assert_eq!(after.status, RunStatus::Failed);
    assert_eq!(after.error.as_deref(), Some("interrupted"));
    assert_eq!(after.quiet_since, None);
    assert_eq!(store.dot_counts(&dot).await.unwrap().0, 0);
}

#[tokio::test]
async fn the_quiet_limit_a_run_started_with_is_kept() {
    let (_dir, store) = temp_store().await;
    let (_, run) = running_run(&store, "limited").await;
    assert_eq!(run.quiet_limit_secs, None);
    store.set_run_quiet_limit(&run.id, Some(600)).await.unwrap();
    assert_eq!(
        store.get_run(&run.id).await.unwrap().quiet_limit_secs,
        Some(600)
    );
}
