mod common;

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, Local, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    AttentionRun, Dot, DotSpec, NeedsAttention, NewRun, PendingRetry, Run, RunStatus,
    ScheduleChange, ScheduleState, ScheduleStatus, ScheduleView, TriggerKind, WorkspaceMode,
};
use dots_core::schedule::Schedule;
use dots_core::store::{Store, DB_FILE};
use dots_core::util::{now, timestamp};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

fn config(dir: &TempDir) -> Config {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg
}

async fn start(dir: &TempDir) -> Runtime {
    Runtime::start(config(dir)).await.unwrap()
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
        .create_dot(folder_spec(&dir, "a", Some(every(600))), None)
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
        .create_dot(folder_spec(&dir, "plain", None), None)
        .await
        .unwrap();
    assert_eq!(row(&rt, &plain).await, None);
    assert!(updated(&mut rx).is_empty());

    // A disabled dot gets its row too; it is not processed while the dot is disabled.
    let mut s = folder_spec(&dir, "off", Some(every(600)));
    s.enabled = false;
    let since = now();
    let dot = rt.create_dot(s, None).await.unwrap();
    assert_fresh(&rt, &dot, &since).await;
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    rt.delete_dot(&dot.id).await.unwrap();
    assert_eq!(row(&rt, &dot).await, None);
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    rt.delete_dot(&plain.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    rt.shutdown();
}

/// The runtime writes the times of a schedule's row from its clock: at start, where it makes
/// the row of a dot upgraded by migration 0005, on a change of the dot, and on a resume after
/// a pause, which does not catch up the slots of the pause.
#[tokio::test]
async fn the_runtime_takes_the_times_of_its_rows_from_its_clock() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = DateTime::parse_from_rfc3339("2030-01-07T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    // A dot from before the upgrade: a schedule, no row.
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let store = Store::open(&data.join(DB_FILE)).await.unwrap();
    let old = store
        .create_dot(&folder_spec(&dir, "old", Some(every(600))), None)
        .await
        .unwrap();
    store.close().await;

    let clock = FakeClock::new(t0);
    let mut cfg = config(&dir);
    cfg.clock = clock.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    assert_eq!(row(&rt, &old).await.unwrap().last_slot_at, timestamp(t0));

    let dot = rt
        .create_dot(folder_spec(&dir, "new", Some(every(600))), None)
        .await
        .unwrap();
    assert_eq!(row(&rt, &dot).await.unwrap().last_slot_at, timestamp(t0));

    rt.set_schedules_paused(true).await.unwrap();
    let resumed = t0 + TimeDelta::hours(30);
    clock.set(resumed);
    rt.set_schedules_paused(false).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while row(&rt, &dot).await.unwrap().last_slot_at != timestamp(resumed) {
        assert!(
            std::time::Instant::now() < deadline,
            "the resume did not move the row"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        row(&rt, &old).await.unwrap().last_slot_at,
        timestamp(resumed)
    );
    assert!(rt.list_runs(None, 10).await.unwrap().is_empty());
    rt.shutdown();
}

/// A scheduled run of `dot` that failed with `error`, set as the schedule's "Needs
/// attention" (as the scheduler sets it).
async fn flag(rt: &Runtime, dot: &Dot, error: &str) -> Run {
    let run = rt
        .store()
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let run = rt
        .store()
        .finish_run(&run.id, RunStatus::Failed, None, Some(error))
        .await
        .unwrap();
    let change = ScheduleChange {
        attention_run_id: Some(Some(run.id.clone())),
        ..Default::default()
    };
    rt.store()
        .update_schedule_state(&dot.id, &change, Utc::now())
        .await
        .unwrap()
        .unwrap();
    run
}

fn attention(dot: &Dot, run: &Run) -> NeedsAttention {
    NeedsAttention {
        dot_id: dot.id.clone(),
        dot_name: dot.spec.name.clone(),
        run_id: run.id.clone(),
        error: run.error.clone(),
        failed_at: run.ended_at.clone().unwrap(),
    }
}

/// Scheduler v2 spec §8: the Inbox lists the schedules that need attention, newest failure
/// first, and counts them; `dismiss_schedule_attention` clears a flag, quietly when there is
/// none, and refuses a dot without a schedule.
#[tokio::test]
async fn attention_is_listed_in_the_inbox_until_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let rt = start(&dir).await;
    let older = rt
        .create_dot(folder_spec(&dir, "older", Some(every(600))), None)
        .await
        .unwrap();
    let newer = rt
        .create_dot(folder_spec(&dir, "newer", Some(every(600))), None)
        .await
        .unwrap();
    let plain = rt
        .create_dot(folder_spec(&dir, "plain", None), None)
        .await
        .unwrap();
    let first = flag(&rt, &older, "model server down").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = flag(&rt, &newer, "claude: not found").await;

    let inbox = rt.inbox().await.unwrap();
    assert_eq!(
        inbox.attention,
        [attention(&newer, &second), attention(&older, &first)]
    );
    assert_eq!(
        rt.inbox_count().await.unwrap(),
        inbox.approvals.len() + inbox.review_runs.len() + 2
    );

    let mut rx = rt.subscribe();
    rt.dismiss_schedule_attention(&older.id).await.unwrap();
    assert_eq!(row(&rt, &older).await.unwrap().attention_run_id, None);
    assert_eq!(updated(&mut rx), [older.id.as_str()]);
    assert_eq!(
        rt.inbox().await.unwrap().attention,
        [attention(&newer, &second)]
    );

    // No flag: nothing to do, and no error.
    rt.dismiss_schedule_attention(&older.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    // No schedule row: not found.
    for id in [plain.id.as_str(), "no-such-dot"] {
        assert!(matches!(
            rt.dismiss_schedule_attention(id).await,
            Err(Error::NotFound(_))
        ));
    }
    rt.shutdown();
}

/// Every hour from 2030-01-07T09:00Z, bounded by `until`.
fn hourly(until: serde_json::Value) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": 60, "anchor": "2030-01-07T09:00:00Z"},
        "catch_up": true,
        "until": until
    }))
    .unwrap()
}

fn row_at(last_slot_at: &str, fired_count: u32) -> ScheduleState {
    ScheduleState {
        dot_id: "d".into(),
        rule_key: String::new(),
        status: ScheduleStatus::Active,
        last_slot_at: last_slot_at.into(),
        fired_count,
        watch_run_id: None,
        retry: None,
        attention_run_id: None,
        updated_at: last_slot_at.into(),
    }
}

fn utc(t: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc)
}

/// Scheduler v2 spec §8: what the Dots screen is told about a schedule. The next fire time is
/// the first slot after now, none once nothing more fires; times are in the view's zone.
#[test]
fn a_schedule_view_says_what_comes_next() {
    let now = utc("2030-01-07T10:30:00Z");
    let s = hourly(json!(null));
    let view = ScheduleView::new(
        &row_at("2030-01-07T10:00:00.000Z", 1),
        &s,
        true,
        None,
        now,
        &Utc,
    );
    assert_eq!(
        view,
        ScheduleView {
            status: ScheduleStatus::Active,
            fired_count: 1,
            next_fire: Some("2030-01-07T11:00:00+00:00".into()),
            retry_at: None,
            retry_attempt: None,
            attention: None,
        }
    );
    // A slot the pass has not handled yet is past: it is not the next one.
    let view = ScheduleView::new(
        &row_at("2030-01-07T09:00:00.000Z", 0),
        &s,
        true,
        None,
        now,
        &Utc,
    );
    assert_eq!(view.next_fire.as_deref(), Some("2030-01-07T11:00:00+00:00"));

    // A waiting retry and the attention, in the view's zone.
    let mut row = row_at("2030-01-07T10:00:00.000Z", 1);
    row.retry = Some(PendingRetry {
        slot_at: "2030-01-07T10:00:00.000Z".into(),
        attempt: 2,
        at: "2030-01-07T10:31:00.000Z".into(),
    });
    let flag = AttentionRun {
        run_id: "r1".into(),
        error: Some("down".into()),
    };
    let plus_one = FixedOffset::east_opt(3600).unwrap();
    let view = ScheduleView::new(&row, &s, true, Some(flag.clone()), now, &plus_one);
    assert_eq!(
        (
            view.next_fire.as_deref(),
            view.retry_at.as_deref(),
            view.retry_attempt
        ),
        (
            Some("2030-01-07T12:00:00+01:00"),
            Some("2030-01-07T11:31:00+01:00"),
            Some(2)
        )
    );
    assert_eq!(view.attention, Some(flag.clone()));

    // A disabled dot fires and retries nothing; its flag stays.
    let view = ScheduleView::new(&row, &s, false, Some(flag.clone()), now, &Utc);
    assert_eq!(
        (
            view.next_fire,
            view.retry_at,
            view.retry_attempt,
            view.attention
        ),
        (None, None, None, Some(flag))
    );
    // Neither does a schedule whose count is reached (its last outcome may still be pending),
    // nor a completed or expired one.
    let counted = hourly(json!({"count": 1}));
    let view = ScheduleView::new(
        &row_at("2030-01-07T10:00:00.000Z", 1),
        &counted,
        true,
        None,
        now,
        &Utc,
    );
    assert_eq!(
        (view.status, view.next_fire),
        (ScheduleStatus::Active, None)
    );
    for status in [ScheduleStatus::Completed, ScheduleStatus::Expired] {
        let mut row = row_at("2030-01-07T10:00:00.000Z", 1);
        row.status = status;
        let view = ScheduleView::new(&row, &s, true, None, now, &Utc);
        assert_eq!((view.status, view.next_fire), (status, None));
    }
}

/// `dot_overview` gives each dot with a schedule the view of its row, with the error of the
/// run that needs attention, at the runtime clock's time; a dot without a schedule has none.
#[tokio::test]
async fn the_overview_shows_where_each_schedule_stands() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = utc("2030-01-07T09:00:00Z");
    let mut cfg = config(&dir);
    cfg.clock = FakeClock::new(t0);
    let rt = Runtime::start(cfg).await.unwrap();
    let on = rt
        .create_dot(folder_spec(&dir, "on", Some(hourly(json!(null)))), None)
        .await
        .unwrap();
    let mut s = folder_spec(&dir, "off", Some(hourly(json!(null))));
    s.enabled = false;
    let off = rt.create_dot(s, None).await.unwrap();
    let plain = rt
        .create_dot(folder_spec(&dir, "plain", None), None)
        .await
        .unwrap();
    let failed = flag(&rt, &on, "model server down").await;
    let change = ScheduleChange {
        retry: Some(Some(PendingRetry {
            slot_at: timestamp(t0),
            attempt: 2,
            at: timestamp(t0 + TimeDelta::minutes(5)),
        })),
        ..Default::default()
    };
    rt.store()
        .update_schedule_state(&on.id, &change, t0)
        .await
        .unwrap()
        .unwrap();

    let rows = rt.dot_overview().await.unwrap();
    let view = |dot: &Dot| {
        rows.iter()
            .find(|r| r.dot.id == dot.id)
            .unwrap()
            .schedule_state
            .clone()
    };
    let local = |t: DateTime<Utc>| t.with_timezone(&Local).to_rfc3339();
    assert_eq!(
        view(&on),
        Some(ScheduleView {
            status: ScheduleStatus::Active,
            fired_count: 0,
            next_fire: Some(local(t0 + TimeDelta::hours(1))),
            retry_at: Some(local(t0 + TimeDelta::minutes(5))),
            retry_attempt: Some(2),
            attention: Some(AttentionRun {
                run_id: failed.id,
                error: Some("model server down".into()),
            }),
        })
    );
    assert_eq!(
        view(&off).map(|v| (v.status, v.next_fire)),
        Some((ScheduleStatus::Active, None))
    );
    assert_eq!(view(&plain), None);
    rt.shutdown();
}
