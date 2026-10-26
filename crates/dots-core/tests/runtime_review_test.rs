mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{init_repo, spec, wait_status};
use dots_core::model::{DotOverview, DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::{Config, Error, Runtime};
use llm::{call, text, Llm};
use serde_json::json;
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    repo: PathBuf,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, repo, rt }
}

fn schedule(v: serde_json::Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

/// A Claude dot (fake CLI) working directly in a plain folder.
fn folder_spec(t: &Rt, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

/// A local dot working in a worktree of the test repo.
fn worktree_spec(t: &Rt, llm: &Llm, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.repo.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "m".into();
    s
}

/// Runs a local worktree dot that writes `new.txt`, so its worktree is kept for review.
async fn changed_worktree_run(t: &Rt, name: &str) -> dots_core::model::Run {
    let llm = Llm::start(vec![
        call(
            "c1",
            "write_file",
            json!({"file_path": "new.txt", "content": "x\n"}),
        ),
        text("Wrote new.txt."),
    ])
    .await;
    let dot =
        t.rt.create_dot(worktree_spec(t, &llm, name), None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await
}

#[tokio::test]
async fn dot_overview_shows_active_and_last_runs_and_the_next_fire() {
    let next_fire = |row: &DotOverview| row.schedule_state.as_ref()?.next_fire.clone();
    let t = start().await;
    let mut s = folder_spec(&t, "sched");
    s.schedule = Some(Schedule::cron("0 0 9 * * *"));
    let dot = t.rt.create_dot(s, None).await.unwrap();
    let mut s = folder_spec(&t, "daily");
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "daily", "time": "07:15"}}),
    ));
    let daily = t.rt.create_dot(s, None).await.unwrap();
    let idle =
        t.rt.create_dot(folder_spec(&t, "idle"), None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;

    let rows = t.rt.dot_overview().await.unwrap();
    let row = rows.iter().find(|r| r.dot.id == dot.id).unwrap();
    assert!(row.active_run.is_none());
    assert_eq!(
        row.last_run.as_ref().map(|r| r.id.as_str()),
        Some(run.id.as_str())
    );
    let next = next_fire(row).expect("next fire");
    assert!(next.contains("T09:00:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == daily.id).unwrap();
    let next = next_fire(row).expect("next fire");
    assert!(next.contains("T07:15:00"), "{next}");
    let row = rows.iter().find(|r| r.dot.id == idle.id).unwrap();
    assert!(row.last_run.is_none() && row.schedule_state.is_none());

    // A run waiting on its model shows as active.
    let llm = Llm::start(vec![text("late").set_delay(Duration::from_secs(5))]).await;
    let mut s = folder_spec(&t, "slow");
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "m".into();
    let slow = t.rt.create_dot(s, None).await.unwrap();
    let r = t.rt.run_now(&slow.id).await.unwrap();
    let rows = t.rt.dot_overview().await.unwrap();
    let row = rows.iter().find(|x| x.dot.id == slow.id).unwrap();
    assert_eq!(
        row.active_run.as_ref().map(|x| x.id.as_str()),
        Some(r.id.as_str())
    );
    t.rt.cancel_run(&r.id).await.unwrap();
    t.rt.shutdown();
}

#[test]
fn preview_schedule_lists_the_next_fire_times() {
    let preview = |v: serde_json::Value, n| Runtime::preview_schedule(&schedule(v), n);
    let times = Runtime::preview_schedule(&Schedule::cron("0 30 9 * * Mon"), 3).unwrap();
    assert_eq!(times.len(), 3);
    assert!(times.iter().all(|t| t.contains("T09:30:00")), "{times:?}");
    assert!(times[0] < times[1] && times[1] < times[2]);
    assert_eq!(
        Runtime::preview_schedule(&Schedule::cron("0 0 * * * *"), 50)
            .unwrap()
            .len(),
        10
    );
    assert!(matches!(
        Runtime::preview_schedule(&Schedule::cron("every day"), 3),
        Err(Error::Invalid(_))
    ));

    // The builder's kinds, within their bounds.
    let daily = json!({"rule": {"kind": "daily", "time": "07:15"}, "until": {"count": 2}});
    let times = preview(daily, 3).unwrap();
    assert_eq!(times.len(), 2, "{times:?}");
    assert!(times.iter().all(|t| t.contains("T07:15:00")), "{times:?}");
    // An every schedule as the UI sends it, without an anchor, starts now.
    let before = chrono::Local::now();
    let times = preview(json!({"rule": {"kind": "every", "minutes": 90}}), 2).unwrap();
    let first = chrono::DateTime::parse_from_rfc3339(&times[0]).unwrap();
    let gap = first.with_timezone(&chrono::Local) - before;
    assert!(
        gap > chrono::TimeDelta::minutes(89) && gap <= chrono::TimeDelta::minutes(91),
        "{times:?}"
    );
    // An every schedule sent back with its anchor keeps its phase.
    let anchored = json!({"rule": {"kind": "every", "minutes": 60,
        "anchor": "2020-01-01T00:20:00Z"}});
    let times = preview(anchored, 2).unwrap();
    assert!(times.iter().all(|t| t.contains(":20:00")), "{times:?}");
    // Refused like `DotSpec::validate`, and like a create or a change of the schedule when no
    // slot is left, so the editor can say why before a save.
    let no_day = json!({"rule": {"kind": "weekly", "days": [], "time": "09:00"}});
    assert!(matches!(preview(no_day, 3), Err(Error::Invalid(_))));
    let never =
        |v| matches!(preview(v, 3), Err(Error::Invalid(m)) if m == "this schedule would never run");
    assert!(never(
        json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}})
    ));
    assert!(never(json!({"rule": {"kind": "daily", "time": "09:00"},
        "until": {"date": "2020-01-01T08:00"}})));
}

#[tokio::test]
async fn a_thread_and_its_events_can_be_read_from_any_of_its_runs() {
    let t = start().await;
    let dot =
        t.rt.create_dot(folder_spec(&t, "chatty"), None)
            .await
            .unwrap();
    let first = t.rt.chat(&dot.id, "hi", None).await.unwrap();
    wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 15).await;
    let second = t.rt.chat(&dot.id, "more", Some(&first.id)).await.unwrap();
    wait_status(t.rt.store(), &second.id, RunStatus::Succeeded, 15).await;

    for id in [&first.id, &second.id] {
        let ids: Vec<String> =
            t.rt.run_thread(id)
                .await
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect();
        assert_eq!(ids, vec![first.id.clone(), second.id.clone()]);
    }
    let all = t.rt.list_run_events(&first.id, 0).await.unwrap();
    assert!(!all.is_empty());
    let after = t.rt.list_run_events(&first.id, all[0].seq).await.unwrap();
    assert_eq!(after.len(), all.len() - 1);
    assert!(matches!(
        t.rt.list_run_events("nope", 0).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        t.rt.run_thread("nope").await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(t.rt.list_runs(Some(&dot.id), 1).await.unwrap().len(), 1);
    assert_eq!(
        t.rt.get_run(&second.id).await.unwrap().parent_run_id,
        Some(first.id)
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn the_inbox_lists_failures_and_kept_worktrees_until_reviewed() {
    let t = start().await;
    // A plain success in a folder only notifies.
    let ok = t.rt.create_dot(folder_spec(&t, "ok"), None).await.unwrap();
    let r = t.rt.run_now(&ok.id).await.unwrap();
    wait_status(t.rt.store(), &r.id, RunStatus::Succeeded, 15).await;
    // A failure (here: the workdir is gone) is listed.
    let mut s = folder_spec(&t, "bad");
    s.workdir = t.dir.path().join("missing").to_string_lossy().to_string();
    let bad = t.rt.create_dot(s, None).await.unwrap();
    let failed = t.rt.run_now(&bad.id).await.unwrap();
    wait_status(t.rt.store(), &failed.id, RunStatus::Failed, 15).await;
    // A worktree with changes is listed.
    let kept = changed_worktree_run(&t, "writer").await;

    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.approvals.is_empty());
    let mut ids: Vec<&str> = inbox
        .review_runs
        .iter()
        .map(|r| r.run.id.as_str())
        .collect();
    ids.sort();
    let mut want = vec![failed.id.as_str(), kept.id.as_str()];
    want.sort();
    assert_eq!(ids, want);
    let row = inbox
        .review_runs
        .iter()
        .find(|r| r.run.id == kept.id)
        .unwrap();
    assert_eq!(row.dot_name, "writer");
    // The kept run has something to discard; the failed one has no worktree on disk.
    assert!(row.worktree_exists);
    let gone = inbox
        .review_runs
        .iter()
        .find(|r| r.run.id == failed.id)
        .unwrap();
    assert!(!gone.worktree_exists);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 2);

    let reviewed = t.rt.mark_reviewed(&failed.id).await.unwrap();
    assert!(reviewed.reviewed_at.is_some());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_pending_approval_is_listed_with_its_dot() {
    let t = start().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": "git push"})),
        text("Done."),
    ])
    .await;
    let mut s = worktree_spec(&t, &llm, "pusher");
    s.approval_wait_secs = 30;
    let dot = t.rt.create_dot(s, None).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let inbox = loop {
        let inbox = t.rt.inbox().await.unwrap();
        if !inbox.approvals.is_empty() {
            break inbox;
        }
        assert!(std::time::Instant::now() < deadline, "no approval appeared");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let a = &inbox.approvals[0];
    assert_eq!(
        (a.dot_id.as_str(), a.dot_name.as_str()),
        (dot.id.as_str(), "pusher")
    );
    assert_eq!(a.approval.run_id, run.id);
    assert_eq!(a.approval.tool, "Bash");
    assert_eq!(t.rt.approvals_for_run(&run.id).await.unwrap().len(), 1);
    // Marking an active thread reviewed is refused.
    assert!(matches!(
        t.rt.mark_reviewed(&run.id).await,
        Err(Error::Conflict(_))
    ));
    t.rt.decide_approval(&a.approval.id, false, None, None)
        .await
        .unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}
