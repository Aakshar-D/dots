mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::model::{Dot, DotSpec, NewRun, ScheduleView, TriggerKind, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::store::DB_FILE;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use tempfile::TempDir;

const T0: &str = "2030-01-07T09:00:00.000Z";
const T1: &str = "2030-01-07T10:00:00.000Z";
const T2: &str = "2030-01-07T11:00:00.000Z";
const T3: &str = "2030-01-07T12:00:00.000Z";

async fn start(dir: &TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    // No dispatcher may claim the runs these tests create.
    rt.hold_runs(true).await;
    rt
}

/// A dot whose first slot is hours away, so nothing fires during a test.
async fn dot(rt: &Runtime, dir: &TempDir) -> Dot {
    let schedule: Schedule = serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": 600},
        "catch_up": true,
        "until": null
    }))
    .unwrap();
    let mut s: DotSpec = spec("skipper");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.schedule = Some(schedule);
    rt.create_dot(s, None, None).await.unwrap()
}

async fn pool(dir: &TempDir) -> SqlitePool {
    SqlitePoolOptions::new()
        .connect(&format!(
            "sqlite://{}",
            dir.path().join("data").join(DB_FILE).display()
        ))
        .await
        .unwrap()
}

/// A scheduled run of the dot, queued at `at`.
async fn scheduled_run(rt: &Runtime, pool: &SqlitePool, dot: &Dot, at: &str) {
    let run = rt
        .store()
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET queued_at = ? WHERE id = ?")
        .bind(at)
        .bind(&run.id)
        .execute(pool)
        .await
        .unwrap();
}

async fn skip(pool: &SqlitePool, dot: &Dot, slot_at: &str) {
    sqlx::query(
        "INSERT INTO schedule_skips (id, dot_id, slot_at, reason, run_id, created_at) \
         VALUES (?, ?, ?, 'busy', NULL, ?)",
    )
    .bind(format!("skip-{slot_at}"))
    .bind(&dot.id)
    .bind(slot_at)
    .bind(slot_at)
    .execute(pool)
    .await
    .unwrap();
}

async fn view(rt: &Runtime, dot: &Dot) -> ScheduleView {
    rt.dot_overview()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.dot.id == dot.id)
        .unwrap()
        .schedule_state
        .expect("a schedule view")
}

#[tokio::test]
async fn the_view_shows_the_last_skip_and_how_many_since_the_last_run() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let pool = pool(&dir).await;
    let dot = dot(&rt, &dir).await;
    scheduled_run(&rt, &pool, &dot, T0).await;
    skip(&pool, &dot, T1).await;
    skip(&pool, &dot, T2).await;
    let v = view(&rt, &dot).await;
    assert_eq!(v.last_skipped_at.as_deref(), Some(T2));
    assert_eq!(v.skipped_since_run, 2);

    scheduled_run(&rt, &pool, &dot, T3).await;
    let v = view(&rt, &dot).await;
    assert_eq!(v.skipped_since_run, 0);
    assert_eq!(v.last_skipped_at.as_deref(), Some(T2));
    rt.shutdown();
}

#[tokio::test]
async fn no_skips_gives_none_and_zero() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let v = view(&rt, &dot).await;
    assert_eq!(v.last_skipped_at, None);
    assert_eq!(v.skipped_since_run, 0);
    rt.shutdown();
}

#[tokio::test]
async fn schedule_skips_are_newest_first_and_limited() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let pool = pool(&dir).await;
    let dot = dot(&rt, &dir).await;
    let slots: Vec<String> = (10..15)
        .map(|h| format!("2030-01-07T{h}:00:00.000Z"))
        .collect();
    for slot in &slots {
        skip(&pool, &dot, slot).await;
    }
    let newest: Vec<String> = rt
        .schedule_skips(&dot.id, Some(3))
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.slot_at)
        .collect();
    assert_eq!(
        newest,
        vec![slots[4].clone(), slots[3].clone(), slots[2].clone()]
    );
    assert_eq!(
        rt.schedule_skips(&dot.id, Some(500)).await.unwrap().len(),
        5
    );
    assert_eq!(rt.schedule_skips(&dot.id, None).await.unwrap().len(), 5);
    assert!(matches!(
        rt.schedule_skips("nope", None).await,
        Err(Error::NotFound(_))
    ));
    rt.shutdown();
}
