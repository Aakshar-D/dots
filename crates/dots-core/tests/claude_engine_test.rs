mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::{ApprovalHub, DecideEffect};
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{Dot, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct E2e {
    dir: TempDir,
    store: Store,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    bus: Bus,
    args_out: PathBuf,
    scratch: PathBuf,
}

async fn e2e(script: Value, program: Option<PathBuf>) -> E2e {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let args_out = dir.path().join("args.json");
    let scratch = dir.path().join("scratch");
    let program = program.unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let engine = ClaudeEngine::new(program, scratch.clone())
        .with_env("FAKE_CLAUDE_SCRIPT", &script_path.to_string_lossy())
        .with_env("FAKE_CLAUDE_ARGS_OUT", &args_out.to_string_lossy());
    let listener = bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: format!("http://127.0.0.1:{port}/mcp"),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_secs(2),
        },
    );
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner: runner.clone(),
            hub: hub.clone(),
        },
        CancellationToken::new(),
    );
    runner.spawn_dispatcher(CancellationToken::new());
    E2e {
        dir,
        store,
        hub,
        runner,
        bus,
        args_out,
        scratch,
    }
}

async fn folder_dot(e: &E2e, wait: u64) -> Dot {
    let mut s = spec("cli");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.approval_wait_secs = wait;
    e.store.create_dot(&s).await.unwrap()
}

fn recorded_args(e: &E2e) -> Value {
    serde_json::from_str(&std::fs::read_to_string(&e.args_out).unwrap()).unwrap()
}

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

fn success(text: &str) -> Value {
    json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": text,
                    "usage": {"input_tokens": 3, "output_tokens": 2}}})
}

#[tokio::test]
async fn happy_path_streams_events_and_passes_prompt_on_stdin() {
    let e = e2e(
        json!([init(),
               {"emit": {"type": "assistant", "message": {"content": [{"type": "text", "text": "$PROMPT"}]}}},
               success("fake done")]),
        None,
    )
    .await;
    let dot = folder_dot(&e, 600).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let done = wait_status(&e.store, &run.id, RunStatus::Succeeded, 15).await;
    assert_eq!(done.summary.as_deref(), Some("fake done"));
    assert_eq!(done.session_id.as_deref(), Some("fake-session"));
    assert_eq!((done.tokens_in, done.tokens_out), (3, 2));

    let events = e.store.list_events(&run.id, 0).await.unwrap();
    let text = events
        .iter()
        .find(|ev| ev.kind == "assistant_text")
        .unwrap();
    assert!(text.data["text"]
        .as_str()
        .unwrap()
        .starts_with("Do the thing."));

    let rec = recorded_args(&e);
    let args: Vec<String> = serde_json::from_value(rec["args"].clone()).unwrap();
    assert!(args
        .windows(2)
        .any(|w| w == ["--setting-sources", ""]));
    assert!(!args.iter().any(|a| a.contains("Do the thing")));
    assert_eq!(rec["mcp_tool_timeout"], "630000");
    assert_eq!(PathBuf::from(rec["cwd"].as_str().unwrap()), e.dir.path());
    assert!(
        std::fs::read_dir(&e.scratch).unwrap().next().is_none(),
        "mcp config not cleaned up"
    );
}

#[tokio::test]
async fn live_approval_round_trip() {
    let e = e2e(
        json!([init(),
               {"approve": {"tool_name": "Bash", "input": {"command": "git push"}}},
               success("pushed")]),
        None,
    )
    .await;
    let dot = folder_dot(&e, 10).await;
    let mut events = e.bus.subscribe();
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let approval = loop {
        if let RuntimeEvent::ApprovalRequested { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    assert_eq!(approval.tool, "Bash");
    assert!(matches!(
        e.hub.decide(&approval.id, true, None).await.unwrap(),
        DecideEffect::Live(_)
    ));
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 15).await;
    let result = e
        .store
        .list_events(&run.id, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|ev| ev.kind == "tool_result")
        .unwrap();
    assert_eq!(result.data["output"], "ok");
    assert_eq!(result.data["is_error"], false);
}

#[tokio::test]
async fn parked_approval_resumes_with_the_same_session() {
    let e = e2e(
        json!([init(),
               {"approve": {"tool_name": "Bash", "input": {"command": "git push"}}},
               success("finished other work")]),
        None,
    )
    .await;
    let dot = folder_dot(&e, 0).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 15).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    let effect = e.hub.decide(&approval.id, true, None).await.unwrap();
    let DecideEffect::Parked(a) = effect else {
        panic!("expected parked")
    };
    let _ = e.runner.maybe_resume(&a.run_id).await.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let child = loop {
        let runs = e.store.list_runs(Some(&dot.id), 10).await.unwrap();
        if let Some(r) = runs.into_iter().find(|r| {
            r.trigger == TriggerKind::Resume
                && r.parent_run_id.as_deref() == Some(parent.id.as_str())
        }) {
            break r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no resume child appeared"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 15).await;
    let args: Vec<String> = serde_json::from_value(recorded_args(&e)["args"].clone()).unwrap();
    assert!(args.windows(2).any(|w| w == ["--resume", "fake-session"]));
    let ok = e
        .store
        .list_events(&child.id, 0)
        .await
        .unwrap()
        .into_iter()
        .any(|ev| ev.kind == "tool_result" && ev.data["output"] == "ok");
    assert!(ok, "grant should allow the retried call");
}

#[tokio::test]
async fn nonzero_exit_without_result_reports_stderr() {
    let e = e2e(json!([{"stderr": "not logged in"}, {"exit": 3}]), None).await;
    let dot = folder_dot(&e, 600).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let failed = wait_status(&e.store, &run.id, RunStatus::Failed, 15).await;
    let err = failed.error.unwrap();
    assert!(err.contains("code 3"), "{err}");
    assert!(err.contains("not logged in"), "{err}");
}

#[tokio::test]
async fn cancel_kills_the_process() {
    let e = e2e(json!([init(), {"sleep_ms": 60000}, success("never")]), None).await;
    let dot = folder_dot(&e, 600).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    loop {
        let evs = e.store.list_events(&run.id, 0).await.unwrap();
        if evs.iter().any(|ev| ev.kind == "session_started") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pid = recorded_args(&e)["pid"].as_u64().unwrap();
    assert!(
        process_alive(pid),
        "fake-claude should be running before cancel"
    );
    e.runner.cancel(&run.id).await.unwrap();
    wait_status(&e.store, &run.id, RunStatus::Cancelled, 10).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while process_alive(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "process {pid} still alive after cancel"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(windows)]
fn process_alive(pid: u64) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .any(|w| w == pid.to_string())
}

#[cfg(unix)]
fn process_alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn missing_program_fails_the_run() {
    let e = e2e(
        json!([]),
        Some(PathBuf::from("Z:/definitely/not/claude.exe")),
    )
    .await;
    let dot = folder_dot(&e, 600).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let failed = wait_status(&e.store, &run.id, RunStatus::Failed, 15).await;
    assert!(failed.error.unwrap().contains("failed to start"));
}
