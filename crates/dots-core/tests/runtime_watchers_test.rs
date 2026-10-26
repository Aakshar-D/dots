//! The person's side of watchers (watchers spec sections 7 and 8): the runtime's six methods, the
//! Inbox list and counts, the one pause switch, and the acting span a real run gives its dot.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    FolderConfig, PausedReason, PausedWatcher, WatcherConfig, WatcherKind, WorkspaceMode,
};
use dots_core::watchers::NAME_RULE;
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

pub struct Rt {
    pub dir: TempDir,
    pub rt: Runtime,
    pub args_out: PathBuf,
}

/// A runtime on the fake CLI, running `script` (its default when `None`).
pub async fn start(script: Option<Value>) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    if let Some(script) = script {
        let p = dir.path().join("script.json");
        std::fs::write(&p, script.to_string()).unwrap();
        cfg.claude_env
            .push(("FAKE_CLAUDE_SCRIPT".into(), p.to_string_lossy().to_string()));
    }
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt, args_out }
}

/// A folder-mode dot working in `<temp>/work`, which exists.
pub async fn folder_dot(t: &Rt, name: &str) -> dots_core::model::Dot {
    let work = t.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut s = spec(name);
    s.workdir = work.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    t.rt.create_dot(s).await.unwrap()
}

fn folder(t: &Rt, name: &str) -> WatcherConfig {
    let path = t.dir.path().join(name);
    std::fs::create_dir_all(&path).unwrap();
    WatcherConfig::Folder(FolderConfig::new(&path.to_string_lossy()))
}

fn refusal(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

#[tokio::test]
async fn the_person_adds_lists_changes_switches_and_deletes_watchers() {
    let t = start(None).await;
    let dot = folder_dot(&t, "invoices").await;
    let mut events = t.rt.subscribe();
    let w =
        t.rt.watcher_add(&dot.id, " Invoices folder ", folder(&t, "in"))
            .await
            .unwrap();
    assert_eq!(w.name, "Invoices folder");
    assert_eq!(t.rt.watchers_list(&dot.id).await.unwrap(), vec![w.clone()]);
    let renamed =
        t.rt.watcher_update(&w.id, "Bills", folder(&t, "in"))
            .await
            .unwrap();
    assert_eq!(renamed.name, "Bills");
    assert!(
        !t.rt
            .watcher_set_enabled(&w.id, false)
            .await
            .unwrap()
            .enabled
    );
    // Not paused by Dots: a resume changes nothing and says nothing.
    assert_eq!(
        t.rt.watcher_resume(&w.id).await.unwrap().paused_reason,
        None
    );
    t.rt.watcher_delete(&w.id).await.unwrap();
    assert!(t.rt.watchers_list(&dot.id).await.unwrap().is_empty());
    let gone = t.rt.watcher_delete(&w.id).await.unwrap_err();
    assert!(matches!(gone, Error::NotFound(_)), "{gone:?}");
    let unknown =
        t.rt.watcher_add("NODOT", "x", folder(&t, "in"))
            .await
            .unwrap_err();
    assert!(matches!(unknown, Error::NotFound(_)), "{unknown:?}");
    assert_eq!(
        refusal(
            t.rt.watcher_add(&dot.id, " ", folder(&t, "in"))
                .await
                .unwrap_err()
        ),
        NAME_RULE
    );

    let mut updated = 0;
    while let Ok(event) = events.try_recv() {
        if let RuntimeEvent::WatcherUpdated { dot_id, watcher_id } = event {
            assert_eq!(
                (dot_id.as_str(), watcher_id.as_str()),
                (dot.id.as_str(), w.id.as_str())
            );
            updated += 1;
        }
    }
    // Added, changed, switched off, deleted.
    assert_eq!(updated, 4);
}

#[tokio::test]
async fn a_watcher_paused_by_dots_waits_in_the_inbox_and_counts_for_its_dot() {
    let t = start(None).await;
    let dot = folder_dot(&t, "invoices").await;
    let w =
        t.rt.watcher_add(&dot.id, "Invoices folder", folder(&t, "in"))
            .await
            .unwrap();
    let error = "The folder C:/in no longer exists.";
    t.rt.watchers()
        .pause(&w.id, PausedReason::Error, Some(error))
        .await
        .unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.watchers.len(), 1);
    let PausedWatcher {
        watcher_id,
        dot_name,
        kind,
        reason,
        error: said,
        ..
    } = &inbox.watchers[0];
    assert_eq!(
        (
            watcher_id.as_str(),
            dot_name.as_str(),
            *kind,
            *reason,
            said.as_deref()
        ),
        (
            w.id.as_str(),
            "invoices",
            WatcherKind::Folder,
            PausedReason::Error,
            Some(error)
        )
    );
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    let row =
        t.rt.dot_overview()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.dot.id == dot.id)
            .unwrap();
    assert_eq!(row.needs_you, 1);
    assert_eq!(t.rt.unread_total().await.unwrap(), 1);

    t.rt.watcher_resume(&w.id).await.unwrap();
    assert!(t.rt.inbox().await.unwrap().watchers.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
}

#[tokio::test]
async fn pausing_schedules_pauses_watchers_and_a_deleted_dot_takes_its_watchers() {
    let t = start(None).await;
    t.rt.set_schedules_paused(true).await.unwrap();
    assert!(t.rt.watchers().is_paused());
    t.rt.set_schedules_paused(false).await.unwrap();
    assert!(!t.rt.watchers().is_paused());

    let dot = folder_dot(&t, "gone").await;
    let w =
        t.rt.watcher_add(&dot.id, "In", folder(&t, "in"))
            .await
            .unwrap();
    t.rt.delete_dot(&dot.id).await.unwrap();
    let e = t.rt.watcher_set_enabled(&w.id, false).await.unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

#[tokio::test]
async fn a_dot_acts_while_its_run_is_held_and_five_seconds_after_it_ends() {
    let t = start(Some(json!([
        {"sleep_ms": 800},
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false,
                  "result": "done", "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ])))
    .await;
    let dot = folder_dot(&t, "actor").await;
    let acting = t.rt.watchers().acting().clone();
    assert!(!acting.is_acting(&dot.id, Utc::now()));
    let run = t.rt.run_now(&dot.id).await.unwrap();
    common::wait_status(
        t.rt.store(),
        &run.id,
        dots_core::model::RunStatus::Running,
        20,
    )
    .await;
    assert!(acting.is_acting(&dot.id, Utc::now()));
    common::wait_status(
        t.rt.store(),
        &run.id,
        dots_core::model::RunStatus::Succeeded,
        20,
    )
    .await;
    // The runner lets go just after the status is written.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(acting.is_acting(&dot.id, Utc::now()));
    tokio::time::sleep(Duration::from_millis(5500)).await;
    assert!(!acting.is_acting(&dot.id, Utc::now()));
}

#[tokio::test]
async fn a_dot_that_writes_into_its_watched_folder_does_not_wake_itself() {
    let t = start(Some(json!([
        {"write": {"path": "report.pdf", "text": "done"}},
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false,
                  "result": "filed", "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ])))
    .await;
    let dot = folder_dot(&t, "filer").await;
    let work = t.dir.path().join("work");
    let w =
        t.rt.watcher_add(
            &dot.id,
            "Work folder",
            WatcherConfig::Folder(FolderConfig::new(&work.to_string_lossy())),
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !t.rt.watchers().running().contains(&w.id) {
        assert!(
            std::time::Instant::now() < deadline,
            "the watcher did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::fs::write(work.join("in.pdf"), b"%PDF").unwrap();
    let watcher_runs = || async {
        t.rt.list_runs(Some(&dot.id), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.trigger == dots_core::model::TriggerKind::Watcher)
            .collect::<Vec<_>>()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let runs = watcher_runs().await;
        if runs.len() == 1 && runs[0].status == dots_core::model::RunStatus::Succeeded {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{runs:#?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(work.join("report.pdf").is_file());
    // The settle (2 s), the 5 seconds after the run, and some.
    tokio::time::sleep(Duration::from_secs(9)).await;
    assert_eq!(watcher_runs().await.len(), 1);
}
