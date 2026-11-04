//! Sending a failed message again by hand (crash resend spec section 3.6).

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::model::{Dot, DotSpec, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

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
    let mut s: DotSpec = spec("talker");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    rt.create_dot(s, None, None).await.unwrap()
}

/// A run of `new` that ended as `status`.
async fn ended(rt: &Runtime, new: NewRun, status: RunStatus) -> Run {
    let run = rt.store().create_run(&new).await.unwrap();
    rt.store().claim_run(&run.id).await.unwrap();
    // Its engine had started on it: the message got there.
    rt.store()
        .mark_delivered(&run.id, chrono::Utc::now())
        .await
        .unwrap();
    rt.store()
        .finish_run(&run.id, status, None, Some("interrupted"))
        .await
        .unwrap();
    rt.store().get_run(&run.id).await.unwrap()
}

fn message(dot: &str, text: &str) -> NewRun {
    let mut new = NewRun::new(dot, TriggerKind::Chat);
    new.payload = Some(json!({ "message": text, "typed": true }));
    new
}

#[tokio::test]
async fn a_failed_message_is_sent_again() {
    let dir = TempDir::new().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let failed = ended(&rt, message(&dot.id, "hi"), RunStatus::Failed).await;

    let again = rt.resend(&failed.id).await.unwrap();
    assert_eq!(again.status, RunStatus::Queued);
    assert_eq!(again.trigger, TriggerKind::Chat);
    assert_eq!(again.retry_of.as_deref(), Some(failed.id.as_str()));
    assert_eq!(again.root_run_id, failed.root_run_id);
    // Sent by the person: no note that Dots resent it.
    assert_eq!(again.payload, failed.payload);

    // The topic now works on it: sending the failed one again is refused.
    assert!(matches!(
        rt.resend(&failed.id).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn only_a_failed_message_that_ends_its_topic_is_sent_again() {
    let dir = TempDir::new().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let done = ended(&rt, message(&dot.id, "ok"), RunStatus::Succeeded).await;
    assert!(matches!(rt.resend(&done.id).await, Err(Error::Conflict(_))));
    let manual = ended(
        &rt,
        NewRun::new(&dot.id, TriggerKind::Manual),
        RunStatus::Failed,
    )
    .await;
    assert!(matches!(
        rt.resend(&manual.id).await,
        Err(Error::Conflict(_))
    ));

    // A failed message with a newer run after it in its topic.
    let failed = ended(&rt, message(&dot.id, "first"), RunStatus::Failed).await;
    let mut later = message(&dot.id, "second");
    later.root_run_id = Some(failed.root_run_id.clone());
    later.parent_run_id = Some(failed.id.clone());
    ended(&rt, later, RunStatus::Succeeded).await;
    assert!(matches!(
        rt.resend(&failed.id).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn a_message_that_never_got_there_is_left_to_retry() {
    let dir = TempDir::new().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = rt
        .store()
        .create_run(&message(&dot.id, "hi"))
        .await
        .unwrap();
    rt.store().claim_run(&run.id).await.unwrap();
    rt.store()
        .finish_run(&run.id, RunStatus::Failed, None, Some("engine: boom"))
        .await
        .unwrap();
    // Not delivered: its Retry (delivery tracking) sends it again, not Send again.
    assert!(matches!(rt.resend(&run.id).await, Err(Error::Conflict(_))));
}

#[tokio::test]
async fn files_go_with_a_message_sent_again() {
    let dir = TempDir::new().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    rt.store()
        .insert_staged_attachment("att1", &dot.id, "a.txt", 3)
        .await
        .unwrap();
    let mut new = message(&dot.id, "see the file");
    new.payload.as_mut().unwrap()["attachments"] =
        json!([{ "id": "att1", "name": "a.txt", "size": 3 }]);
    let run = rt
        .store()
        .create_chat_run(&new, &[("att1".into(), "a.txt".into())], None, None)
        .await
        .unwrap();
    rt.store().claim_run(&run.id).await.unwrap();
    rt.store()
        .mark_delivered(&run.id, chrono::Utc::now())
        .await
        .unwrap();
    rt.store()
        .finish_run(&run.id, RunStatus::Failed, None, Some("interrupted"))
        .await
        .unwrap();

    let again = rt.resend(&run.id).await.unwrap();
    assert!(rt
        .store()
        .run_attachments(&run.id)
        .await
        .unwrap()
        .is_empty());
    let files = rt.store().run_attachments(&again.id).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "att1");
}
