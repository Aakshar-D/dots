mod common;
mod llm;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::{
    Dot, EngineKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Policy, Preset};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use llm::{call, calls, completion, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use wiremock::ResponseTemplate;

struct Env {
    _dir: TempDir,
    ws: PathBuf,
    store: Store,
    runner: Arc<Runner>,
}

async fn env() -> Env {
    let (dir, store) = temp_store().await;
    // The workspace gets its own folder: the store's database lives in `dir`.
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("README.md"), "hello\n").unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus,
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        _dir: dir,
        ws,
        store,
        runner,
    }
}

async fn local_dot(e: &Env, llm: &Llm, preset: Preset) -> Dot {
    let mut s = spec("loco");
    s.workdir = e.ws.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "test-model".into();
    s.policy = Policy::preset(preset);
    e.store.create_dot(&s).await.unwrap()
}

async fn run(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

async fn events(e: &Env, run_id: &str, kind: &str) -> Vec<RunEventRecord> {
    e.store
        .list_events(run_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == kind)
        .collect()
}

#[tokio::test]
async fn reads_a_file_then_finishes() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "read_file", json!({"file_path": "README.md"})),
        text("The README says hello."),
    ])
    .await;
    let dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    let r = run(&e, &dot).await;
    let done = wait_status(&e.store, &r.id, RunStatus::Succeeded, 10).await;
    assert_eq!(done.summary.as_deref(), Some("The README says hello."));
    assert_eq!(done.session_id.as_deref(), Some(r.id.as_str()));
    assert_eq!((done.tokens_in, done.tokens_out), (20, 10));

    let reqs = llm.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0]["model"], "test-model");
    assert_eq!(reqs[0]["stream"], false);
    assert_eq!(reqs[0]["tools"].as_array().unwrap().len(), 7);
    let first = messages(&reqs[0]);
    assert_eq!(first[0]["role"], "system");
    assert_eq!(first[1]["role"], "user");
    assert!(first[1]["content"]
        .as_str()
        .unwrap()
        .contains("Do the thing."));
    assert_eq!(tool_result(&reqs[1], "c1"), "hello\n");

    let call_ev = &events(&e, &r.id, "tool_call").await[0];
    assert_eq!(call_ev.data["tool"], "Read");
    assert_eq!(call_ev.data["input"], json!({"file_path": "README.md"}));
    let roles: Vec<Value> = events(&e, &r.id, "message")
        .await
        .iter()
        .map(|ev| ev.data["message"]["role"].clone())
        .collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "tool", "assistant"]
    );
}

#[tokio::test]
async fn denied_and_malformed_calls_are_reported_to_the_model() {
    let e = env().await;
    let llm = Llm::start(vec![
        calls(&[
            (
                "w",
                "write_file",
                json!({"file_path": "x.txt", "content": "x"}),
            ),
            ("u", "fly", json!({})),
            ("o", "read_file", json!({"file_path": "../secret"})),
        ]),
        completion(json!({"role": "assistant", "content": "", "tool_calls": [
            {"id": "bad", "type": "function",
             "function": {"name": "read_file", "arguments": "{not json"}},
            {"type": "function",
             "function": {"name": "list_dir", "arguments": {"path": "."}}}
        ]})),
        text("gave up"),
    ])
    .await;
    let dot = local_dot(&e, &llm, Preset::ReadOnly).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 10).await;
    let reqs = llm.requests();
    assert!(tool_result(&reqs[1], "w").contains("Denied by the policy"));
    assert!(tool_result(&reqs[1], "u").contains("unknown tool 'fly'"));
    assert!(tool_result(&reqs[1], "o").contains(".."));
    assert!(!e.ws.join("x.txt").exists());
    assert!(tool_result(&reqs[2], "bad").contains("not valid JSON"));
    assert_eq!(tool_result(&reqs[2], "call_2_1"), "README.md");
    let results = events(&e, &r.id, "tool_result").await;
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].data["is_error"], true);
}

#[tokio::test]
async fn max_turns_fails_the_run() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("a", "list_dir", json!({})),
        call("b", "list_dir", json!({})),
    ])
    .await;
    let mut dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    dot.spec.max_turns = 2;
    e.store.update_dot(&dot.id, &dot.spec).await.unwrap();
    let r = run(&e, &dot).await;
    let failed = wait_status(&e.store, &r.id, RunStatus::Failed, 10).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("error_max_turns: stopped after 2 turns")
    );
    assert_eq!(llm.requests().len(), 2);
}

#[tokio::test]
async fn endpoint_failures_fail_the_run_with_the_reason() {
    let e = env().await;
    let llm = Llm::start(vec![
        ResponseTemplate::new(404).set_body_json(json!({"error": "model not loaded"}))
    ])
    .await;
    let dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    let r = run(&e, &dot).await;
    let failed = wait_status(&e.store, &r.id, RunStatus::Failed, 10).await;
    let err = failed.error.unwrap();
    assert!(
        err.contains("404") && err.contains("model not loaded"),
        "{err}"
    );
}

#[tokio::test]
async fn cancel_during_a_request_cancels_the_run() {
    let e = env().await;
    let slow = text("late").set_delay(Duration::from_secs(30));
    let llm = Llm::start(vec![slow]).await;
    let dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    let r = run(&e, &dot).await;
    llm.wait_requests(1, 10).await;
    e.runner.cancel(&r.id).await.unwrap();
    wait_status(&e.store, &r.id, RunStatus::Cancelled, 5).await;
}

#[tokio::test]
async fn ask_parks_the_call_and_the_run_awaits_approval() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": "git log -1"})),
        text("Waiting for approval."),
    ])
    .await;
    let mut dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    dot.spec.approval_wait_secs = 0;
    e.store.update_dot(&dot.id, &dot.spec).await.unwrap();
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::AwaitingApproval, 10).await;
    let approvals = e.store.approvals_for_run(&r.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "Bash");
    assert_eq!(approvals[0].input, json!({"command": "git log -1"}));
    let reqs = llm.requests();
    assert!(tool_result(&reqs[1], "c1").contains("Queued for human approval"));
}
