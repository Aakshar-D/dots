mod common;

use std::path::PathBuf;

use chrono::Utc;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, ScheduleChange, ScheduleState, ScheduleStatus, WorkspaceMode,
};
use dots_core::schedule::Schedule;
use dots_core::util::now;
use dots_core::{Config, Runtime};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

async fn start(dir: &TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    Runtime::start(cfg).await.unwrap()
}

/// Every `minutes`, as the editor sends it: without an anchor. The first slot is hours away,
/// so nothing fires during a test.
fn every(minutes: u32) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": minutes},
        "catch_up": true,
        "until": null
    }))
    .unwrap()
}

fn folder_spec(dir: &TempDir, name: &str, schedule: Option<Schedule>) -> DotSpec {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.schedule = schedule;
    s
}

/// The dots named by the `ScheduleUpdated` events published since the last call.
fn updated(rx: &mut Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::ScheduleUpdated { dot_id } = ev {
            ids.push(dot_id);
        }
    }
    ids
}

async fn row(rt: &Runtime, dot: &Dot) -> Option<ScheduleState> {
    rt.store().get_schedule_state(&dot.id).await.unwrap()
}

/// Marks the row completed behind the runtime's back, so that a reset shows.
async fn complete(rt: &Runtime, dot: &Dot) {
    let change = ScheduleChange {
        status: Some(ScheduleStatus::Completed),
        ..Default::default()
    };
    rt.store()
        .update_schedule_state(&dot.id, &change, Utc::now())
        .await
        .unwrap()
        .unwrap();
}

async fn status(rt: &Runtime, dot: &Dot) -> ScheduleStatus {
    row(rt, dot).await.expect("a row").status
}

/// The row is as a reset at or after `since` leaves it, for the dot's stored schedule.
async fn assert_fresh(rt: &Runtime, dot: &Dot, since: &str) {
    let row = row(rt, dot).await.expect("a row");
    assert_eq!(row.rule_key, dot.spec.schedule.as_ref().unwrap().rule_key());
    assert_eq!(row.status, ScheduleStatus::Active);
    assert!(
        row.last_slot_at.as_str() >= since,
        "{row:?} is older than {since}"
    );
    assert_eq!(row.updated_at, row.last_slot_at);
    assert_eq!(row.fired_count, 0);
    assert_eq!(
        (row.watch_run_id, row.retry, row.attention_run_id),
        (None, None, None)
    );
}

#[tokio::test]
async fn the_schedule_row_follows_the_dot() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let mut rx = rt.subscribe();

    // Created with a schedule: a fresh row.
    let since = now();
    let dot = rt
        .create_dot(folder_spec(&dir, "a", Some(every(600))))
        .await
        .unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // A save that leaves the schedule as it is leaves the row alone. The editor sends no
    // anchor; the core keeps the stored one, so the rule key is the same.
    complete(&rt, &dot).await;
    let mut edit = dot.spec.clone();
    edit.instructions = "Do another thing.".into();
    edit.schedule = Some(every(600));
    rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());

    // A changed schedule resets it.
    edit.schedule = Some(every(720));
    let since = now();
    let dot = rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // Disabling leaves it alone; enabling resets it; enabling an enabled dot does not.
    complete(&rt, &dot).await;
    rt.set_dot_enabled(&dot.id, false).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());
    let since = now();
    rt.set_dot_enabled(&dot.id, true).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    complete(&rt, &dot).await;
    rt.set_dot_enabled(&dot.id, true).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());

    // So does enabling it in the editor.
    edit.enabled = false;
    rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_eq!(status(&rt, &dot).await, ScheduleStatus::Completed);
    assert!(updated(&mut rx).is_empty());
    edit.enabled = true;
    let since = now();
    rt.update_dot(&dot.id, edit.clone()).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // The schedule removed: no row.
    edit.schedule = None;
    rt.update_dot(&dot.id, edit).await.unwrap();
    assert_eq!(row(&rt, &dot).await, None);
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    rt.shutdown();
}

#[tokio::test]
async fn only_a_dot_with_a_schedule_has_a_row_and_it_goes_with_the_dot() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let mut rx = rt.subscribe();

    let plain = rt
        .create_dot(folder_spec(&dir, "plain", None))
        .await
        .unwrap();
    assert_eq!(row(&rt, &plain).await, None);
    assert!(updated(&mut rx).is_empty());

    // A disabled dot gets its row too; it is not processed while the dot is disabled.
    let mut s = folder_spec(&dir, "off", Some(every(600)));
    s.enabled = false;
    let since = now();
    let dot = rt.create_dot(s).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    rt.delete_dot(&dot.id).await.unwrap();
    assert_eq!(row(&rt, &dot).await, None);
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    rt.delete_dot(&plain.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    rt.shutdown();
}
