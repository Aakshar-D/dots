//! "Ask the dot to fix this" (result check spec section 4): the concerns go back to the dot as
//! the person's next message in the same conversation, only when the person asks.

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::model::{Dot, DotSpec, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::result_check::{ResultCheck, Verdict};
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
    let mut s: DotSpec = spec("writer");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    rt.create_dot(s, None, None).await.unwrap()
}

/// A chat message the dot answered, with a session to continue.
async fn answered(rt: &Runtime, new: NewRun) -> Run {
    let run = rt.store().create_run(&new).await.unwrap();
    rt.store().claim_run(&run.id).await.unwrap();
    rt.store().set_session(&run.id, "sess-1").await.unwrap();
    rt.store()
        .mark_delivered(&run.id, chrono::Utc::now())
        .await
        .unwrap();
    rt.store()
        .finish_run(
            &run.id,
            RunStatus::Succeeded,
            Some("Here are the notes."),
            None,
        )
        .await
        .unwrap();
    rt.store().get_run(&run.id).await.unwrap()
}

fn message(dot: &str, text: &str) -> NewRun {
    let mut new = NewRun::new(dot, TriggerKind::Chat);
    new.payload = Some(json!({ "message": text, "typed": true }));
    new
}

fn flagged() -> ResultCheck {
    ResultCheck::Done {
        model: "haiku".into(),
        verdict: Verdict::HasConcerns,
        concerns: vec!["The breaking change is missing.".into()],
        confidence: Some(0.7),
    }
}

#[tokio::test]
async fn the_concerns_go_back_as_the_next_message_of_the_conversation() {
    let dir = TempDir::new().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = answered(&rt, message(&dot.id, "Write the release notes.")).await;
    rt.store()
        .set_result_check(&run.id, Some(&flagged()))
        .await
        .unwrap();

    let fix = rt.ask_to_fix(&run.id).await.unwrap();
    assert_eq!(fix.trigger, TriggerKind::Chat);
    assert_eq!(fix.status, RunStatus::Queued);
    assert_eq!(fix.parent_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(fix.root_run_id, run.root_run_id);
    let text = fix.payload.as_ref().unwrap()["message"].as_str().unwrap();
    assert!(text.contains("- The breaking change is missing."));
    assert!(text.contains("fix what needs fixing"));
    // The fix is itself a message the dot will answer: no loop, nothing sent by itself.
    assert_eq!(
        rt.store()
            .list_thread(&run.root_run_id)
            .await
            .unwrap()
            .len(),
        2
    );

    // While the dot works on it, a second ask is refused as any message would be.
    assert!(matches!(
        rt.ask_to_fix(&run.id).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn nothing_to_fix_is_refused() {
    let dir = TempDir::new().unwrap();
    let rt = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let run = answered(&rt, message(&dot.id, "hi")).await;
    assert!(matches!(
        rt.ask_to_fix(&run.id).await,
        Err(Error::Invalid(_))
    ));
    let good = ResultCheck::Done {
        model: "haiku".into(),
        verdict: Verdict::LooksGood,
        concerns: vec![],
        confidence: None,
    };
    rt.store()
        .set_result_check(&run.id, Some(&good))
        .await
        .unwrap();
    assert!(matches!(
        rt.ask_to_fix(&run.id).await,
        Err(Error::Invalid(_))
    ));
}
