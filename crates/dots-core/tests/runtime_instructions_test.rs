//! The person decides instructions a dot proposed (conversational chat spec section 4): the
//! Inbox lists the waiting ones, approving saves them unless they changed since, and the chat
//! continues with what the person decided.

mod common;

use std::path::PathBuf;

use common::{spec, wait_status};
use dots_core::instructions::{InstructionStatus, APPROVED, DECIDED_ALREADY, REJECTED, STALE};
use dots_core::model::{DotSpec, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start(script: Option<Value>) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    if let Some(script) = script {
        let p = dir.path().join("script.json");
        std::fs::write(&p, script.to_string()).unwrap();
        cfg.claude_env
            .push(("FAKE_CLAUDE_SCRIPT".into(), p.to_string_lossy().to_string()));
    }
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

fn folder_spec(t: &Rt, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

/// A dot whose chat ended after it proposed `Do the thing twice.`, and the proposal's id.
async fn proposed(t: &Rt, name: &str) -> (String, Run, String) {
    let dot = t.rt.create_dot(folder_spec(t, name), None).await.unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    let chat = wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 15).await;
    t.rt.checkins()
        .instructions()
        .propose(&chat.id, "Do the thing twice.", "Twice")
        .await
        .unwrap();
    let id =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap()[0]
            .id
            .clone();
    (dot.id, chat, id)
}

fn message(run: &Run) -> &str {
    run.payload.as_ref().unwrap()["message"].as_str().unwrap()
}

#[tokio::test]
async fn approving_saves_the_instructions_and_continues_the_chat() {
    let t = start(None).await;
    let (dot_id, chat, id) = proposed(&t, "shopper").await;
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.instructions.len(), 1);
    assert_eq!(inbox.instructions[0].proposal.id, id);
    assert_eq!(inbox.instructions[0].dot_name, "shopper");
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    let (decided, run) = t.rt.decide_instruction_proposal(&id, true).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Approved);
    assert_eq!(decided.answer_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(
        t.rt.get_dot(&dot_id).await.unwrap().spec.instructions,
        "Do the thing twice."
    );
    // The chat goes on in the dot's session, with Dots' own words: not a message the person
    // typed.
    assert_eq!(run.trigger, TriggerKind::Chat);
    assert_eq!(run.parent_run_id.as_deref(), Some(chat.id.as_str()));
    assert_eq!(run.root_run_id, chat.id);
    assert_eq!(run.session_id, chat.session_id);
    assert_eq!(message(&run), APPROVED);
    assert!(!dots_core::prompt::is_typed(&run));
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    assert!(t.rt.inbox().await.unwrap().instructions.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    let again =
        t.rt.decide_instruction_proposal(&id, false)
            .await
            .unwrap_err();
    assert!(matches!(again, Error::Conflict(m) if m == DECIDED_ALREADY));
}

#[tokio::test]
async fn rejecting_keeps_the_instructions_and_tells_the_dot() {
    let t = start(None).await;
    let (dot_id, _chat, id) = proposed(&t, "shopper").await;
    let (decided, run) = t.rt.decide_instruction_proposal(&id, false).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Rejected);
    assert_eq!(message(&run), REJECTED);
    assert_eq!(
        t.rt.get_dot(&dot_id).await.unwrap().spec.instructions,
        "Do the thing."
    );
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
}

#[tokio::test]
async fn instructions_edited_since_the_proposal_are_not_overwritten() {
    let t = start(None).await;
    let (dot_id, _chat, id) = proposed(&t, "shopper").await;
    let mut edited = t.rt.get_dot(&dot_id).await.unwrap().spec;
    edited.instructions = "Do the other thing.".into();
    t.rt.update_dot(&dot_id, edited).await.unwrap();
    let err =
        t.rt.decide_instruction_proposal(&id, true)
            .await
            .unwrap_err();
    assert!(matches!(err, Error::Conflict(m) if m == STALE));
    assert_eq!(
        t.rt.get_dot(&dot_id).await.unwrap().spec.instructions,
        "Do the other thing."
    );
    let p = t.rt.store().get_instruction_proposal(&id).await.unwrap();
    assert_eq!(p.status, InstructionStatus::Pending);
    // The card still waits: the person may reject it.
    let (decided, _) = t.rt.decide_instruction_proposal(&id, false).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Rejected);
}

#[tokio::test]
async fn approving_instructions_that_already_say_the_proposal_continues_the_chat() {
    let t = start(None).await;
    let (dot_id, chat, id) = proposed(&t, "shopper").await;
    // The person typed the proposal's text into the editor before approving.
    let mut edited = t.rt.get_dot(&dot_id).await.unwrap().spec;
    edited.instructions = "Do the thing twice.\n".into();
    let saved = t.rt.update_dot(&dot_id, edited).await.unwrap();
    let (decided, run) = t.rt.decide_instruction_proposal(&id, true).await.unwrap();
    assert_eq!(decided.status, InstructionStatus::Approved);
    assert_eq!(decided.answer_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(run.parent_run_id.as_deref(), Some(chat.id.as_str()));
    assert_eq!(message(&run), APPROVED);
    // Applied already: they are not saved again.
    let dot = t.rt.get_dot(&dot_id).await.unwrap();
    assert_eq!(dot.spec.instructions, "Do the thing twice.\n");
    assert_eq!(dot.updated_at, saved.updated_at);
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
}

#[tokio::test]
async fn a_chat_that_is_still_working_is_not_continued() {
    let t = start(Some(json!([{"sleep_ms": 3000}]))).await;
    let dot =
        t.rt.create_dot(folder_spec(&t, "busy"), None)
            .await
            .unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Running, 15).await;
    t.rt.checkins()
        .instructions()
        .propose(&chat.id, "Do the thing twice.", "Twice")
        .await
        .unwrap();
    let id =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap()[0]
            .id
            .clone();
    let err =
        t.rt.decide_instruction_proposal(&id, true)
            .await
            .unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("still working on this chat")),
        "{err}"
    );
    assert_eq!(
        t.rt.get_dot(&dot.id).await.unwrap().spec.instructions,
        "Do the thing."
    );
    t.rt.shutdown();
}
