mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::model::{DotSpec, EngineKind, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use llm::{call, messages, text, tool_result, Llm};
use serde_json::json;
use tempfile::TempDir;
use wiremock::ResponseTemplate;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

fn local_spec(t: &Rt, llm: &Llm, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{}/v1", llm.endpoint()));
    s.model = "test-model".into();
    s
}

#[cfg(windows)]
const ECHO: &str = "Write-Output approved-ok";
#[cfg(not(windows))]
const ECHO: &str = "echo approved-ok";

#[tokio::test]
async fn live_approval_lets_the_command_run() {
    let t = start().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": ECHO})),
        text("ran it"),
    ])
    .await;
    let mut s = local_spec(&t, &llm, "live");
    s.approval_wait_secs = 30;
    let dot = t.rt.create_dot(s).await.unwrap();
    let mut events = t.rt.subscribe();
    let r = t.rt.run_now(&dot.id).await.unwrap();
    let approval = loop {
        if let RuntimeEvent::ApprovalRequested { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    assert_eq!(approval.tool, "Bash");
    t.rt.decide_approval(&approval.id, true, None)
        .await
        .unwrap();
    wait_status(t.rt.store(), &r.id, RunStatus::Succeeded, 30).await;
    assert_eq!(tool_result(&llm.requests()[1], "c1").trim(), "approved-ok");
    t.rt.shutdown();
}

#[tokio::test]
async fn parked_approval_resumes_and_the_grant_allows_one_retry() {
    let t = start().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": ECHO})),
        text("Queued; nothing else to do."),
        call("c2", "shell", json!({"command": ECHO})),
        text("Done after approval."),
    ])
    .await;
    let mut s = local_spec(&t, &llm, "parked");
    s.approval_wait_secs = 0;
    let dot = t.rt.create_dot(s).await.unwrap();
    let parent = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &parent.id, RunStatus::AwaitingApproval, 10).await;
    let approval =
        t.rt.store()
            .list_pending_approvals()
            .await
            .unwrap()
            .remove(0);
    t.rt.decide_approval(&approval.id, true, None)
        .await
        .unwrap();

    let mut child = None;
    for _ in 0..200 {
        let runs = t.rt.store().list_runs(Some(&dot.id), 10).await.unwrap();
        if let Some(c) = runs.into_iter().find(|r| r.trigger == TriggerKind::Resume) {
            child = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let child = child.expect("resume run");
    let child = wait_status(t.rt.store(), &child.id, RunStatus::Succeeded, 30).await;
    assert_eq!(child.session_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.summary.as_deref(), Some("Done after approval."));

    let reqs = llm.requests();
    assert_eq!(reqs.len(), 4);
    let resumed = messages(&reqs[2]);
    let roles: Vec<&str> = resumed
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "tool", "assistant", "user"]
    );
    assert!(resumed[3]["content"]
        .as_str()
        .unwrap()
        .contains("Queued for human approval"));
    assert!(resumed[5]["content"].as_str().unwrap().contains("granted"));
    assert_eq!(tool_result(&reqs[3], "c2").trim(), "approved-ok");
    // The retry used the grant: no second approval was asked.
    assert!(t
        .rt
        .store()
        .approvals_for_run(&child.id)
        .await
        .unwrap()
        .is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn chat_follow_up_continues_the_conversation() {
    let t = start().await;
    let llm = Llm::start(vec![text("hello there"), text("still here")]).await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm, "chatty"))
            .await
            .unwrap();
    let first = t.rt.chat(&dot.id, "hi", None).await.unwrap();
    let first = wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 10).await;
    let follow = t.rt.chat(&dot.id, "again?", Some(&first.id)).await.unwrap();
    wait_status(t.rt.store(), &follow.id, RunStatus::Succeeded, 10).await;

    let sent = messages(&llm.requests()[1]);
    let contents: Vec<&str> = sent
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(sent.len(), 4);
    assert!(contents[1].contains("hi"));
    assert_eq!(contents[2], "hello there");
    assert_eq!(contents[3], "again?");
    t.rt.shutdown();
}

#[tokio::test]
async fn test_endpoint_checks_for_a_well_formed_tool_call() {
    let t = start().await;
    let good = Llm::start(vec![call("p", "echo", json!({"text": "ping"}))]).await;
    let msg =
        t.rt.test_endpoint(&good.endpoint(), "test-model")
            .await
            .unwrap();
    assert!(msg.contains("well-formed"), "{msg}");
    assert_eq!(good.requests()[0]["tools"][0]["function"]["name"], "echo");

    let chatty = Llm::start(vec![text("ping!")]).await;
    let err =
        t.rt.test_endpoint(&chatty.endpoint(), "m")
            .await
            .unwrap_err();
    assert!(err.to_string().contains("instead of a tool call"), "{err}");

    let broken = Llm::start(vec![call("p", "echo", json!({"nope": 1}))]).await;
    let err =
        t.rt.test_endpoint(&broken.endpoint(), "m")
            .await
            .unwrap_err();
    assert!(err.to_string().contains("malformed"), "{err}");

    let down = Llm::start(vec![
        ResponseTemplate::new(400).set_body_string("no such model")
    ])
    .await;
    let err = t.rt.test_endpoint(&down.endpoint(), "m").await.unwrap_err();
    assert!(err.to_string().contains("no such model"), "{err}");

    let err =
        t.rt.test_endpoint("https://example.com", "m")
            .await
            .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    t.rt.shutdown();
}
