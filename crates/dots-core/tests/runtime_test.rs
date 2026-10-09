mod common;

use std::path::PathBuf;

use common::{spec, wait_status};
use dots_core::model::{DotSpec, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
}

async fn start(script: Option<Value>, claude: Option<PathBuf>) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path =
        Some(claude.unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_fake-claude"))));
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

fn folder_spec(dir: &TempDir, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

#[tokio::test]
async fn webhook_to_success_end_to_end() {
    let t = start(None, None).await;
    let dot = t.rt.create_dot(folder_spec(&t.dir, "hook")).await.unwrap();
    let url = t.rt.webhook_url(&dot);
    assert!(url.starts_with(&format!("http://127.0.0.1:{}/dots/", t.rt.port())));
    let resp: Value = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&dot.webhook_token)
        .json(&json!({"event": "build_failed"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let run = wait_status(
        t.rt.store(),
        resp["run_id"].as_str().unwrap(),
        RunStatus::Succeeded,
        15,
    )
    .await;
    assert_eq!(run.summary.as_deref(), Some("fake done"));
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    assert!(rec["prompt"].as_str().unwrap().contains("build_failed"));
    t.rt.shutdown();
}

#[tokio::test]
async fn run_now_then_chat_follow_up_resumes_session() {
    let t = start(None, None).await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "chatty"))
            .await
            .unwrap();
    let manual = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &manual.id, RunStatus::Succeeded, 15).await;

    let first = t.rt.chat(&dot.id, "status?", None).await.unwrap();
    let first = wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 15).await;
    let follow =
        t.rt.chat(&dot.id, "and now?", Some(&first.id))
            .await
            .unwrap();
    assert_eq!(follow.parent_run_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(follow.session_id.as_deref(), Some("fake-session"));
    wait_status(t.rt.store(), &follow.id, RunStatus::Succeeded, 15).await;
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    assert_eq!(rec["prompt"], "and now?");

    assert!(matches!(
        t.rt.chat(&dot.id, "  ", None).await,
        Err(Error::Invalid(_))
    ));
    let other = t.rt.create_dot(folder_spec(&t.dir, "other")).await.unwrap();
    assert!(matches!(
        t.rt.chat(&other.id, "x", Some(&first.id)).await,
        Err(Error::Invalid(_))
    ));
    t.rt.shutdown();
}

#[tokio::test]
async fn decide_approval_resumes_parked_run() {
    let script = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"approve": {"tool_name": "Bash", "input": {"command": "git push"}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    let t = start(Some(script), None).await;
    let mut s = folder_spec(&t.dir, "gated");
    s.approval_wait_secs = 0;
    let dot = t.rt.create_dot(s).await.unwrap();
    let parent = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &parent.id, RunStatus::AwaitingApproval, 15).await;
    let approval =
        t.rt.store()
            .list_pending_approvals()
            .await
            .unwrap()
            .remove(0);
    t.rt.decide_approval(&approval.id, true, None)
        .await
        .unwrap();
    let mut child_id = None;
    for _ in 0..100 {
        let runs = t.rt.store().list_runs(Some(&dot.id), 10).await.unwrap();
        if let Some(c) = runs.iter().find(|r| r.trigger == TriggerKind::Resume) {
            child_id = Some(c.id.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let child_id = child_id.expect("resume run");
    wait_status(t.rt.store(), &child_id, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn start_recovers_interrupted_runs() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let run_id = {
        let store = Store::open(&data.join("dots.db")).await.unwrap();
        let dot = store.create_dot(&spec("crashy")).await.unwrap();
        let run = store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        store.claim_run(&run.id).await.unwrap();
        run.id
    };
    let mut cfg = Config::new(data);
    cfg.port = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    let run = rt.store().get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("interrupted"));
    rt.shutdown();
}

#[tokio::test]
async fn delete_with_active_runs_conflicts() {
    let script = json!([{"sleep_ms": 3000}]);
    let t = start(Some(script), None).await;
    let dot = t.rt.create_dot(folder_spec(&t.dir, "busy")).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    assert!(matches!(
        t.rt.delete_dot(&dot.id).await,
        Err(Error::Conflict(_))
    ));
    t.rt.cancel_run(&run.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Cancelled, 15).await;
    t.rt.delete_dot(&dot.id).await.unwrap();
    t.rt.shutdown();
}

#[tokio::test]
async fn missing_claude_fails_runs_cleanly() {
    let t = start(None, Some(PathBuf::from("Z:/nope/claude.exe"))).await;
    assert!(t.rt.claude_program().is_none());
    let dot = t.rt.create_dot(folder_spec(&t.dir, "nocli")).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    let failed = wait_status(t.rt.store(), &run.id, RunStatus::Failed, 15).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("claude CLI not found: install Claude Code or set Config.claude_path")
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn delete_with_awaiting_run_conflicts_and_keeps_rows() {
    let t = start(None, None).await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "waiting"))
            .await
            .unwrap();
    let store = t.rt.store();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.claim_run(&run.id).await.unwrap();
    store
        .finish_run(&run.id, RunStatus::AwaitingApproval, None, None)
        .await
        .unwrap();
    store
        .create_approval(&run.id, "Bash", &json!({"command": "x"}))
        .await
        .unwrap();
    assert!(matches!(
        t.rt.delete_dot(&dot.id).await,
        Err(Error::Conflict(_))
    ));
    store.get_run(&run.id).await.unwrap();
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);
    assert!(matches!(
        t.rt.delete_dot("nope").await,
        Err(Error::NotFound(_))
    ));
    t.rt.shutdown();
}

#[tokio::test]
async fn chat_follow_up_requires_finished_parent_with_session() {
    let t = start(None, None).await;
    let dot = t.rt.create_dot(folder_spec(&t.dir, "ghost")).await.unwrap();
    let queued =
        t.rt.store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
    let res = t.rt.chat(&dot.id, "hi", Some(&queued.id)).await;
    assert!(matches!(res, Err(Error::Conflict(_))), "{res:?}");
    t.rt.shutdown();
}

#[tokio::test]
async fn update_dot_workspace_change_conflicts_while_runs_are_active() {
    // Seed the parked run before start so the dispatcher can never claim it first.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (dot, run) = {
        let store = Store::open(&data.join("dots.db")).await.unwrap();
        let dot = store
            .create_dot(&folder_spec(&dir, "moving"))
            .await
            .unwrap();
        let run = store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        assert!(store.claim_run(&run.id).await.unwrap());
        store
            .finish_run(&run.id, RunStatus::AwaitingApproval, None, None)
            .await
            .unwrap();
        // Still pending, so the startup resume scan leaves the run parked.
        store
            .create_approval(&run.id, "Bash", &json!({"command": "git push"}))
            .await
            .unwrap();
        (dot, run)
    };
    let mut cfg = Config::new(data);
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    let t = Rt {
        args_out: dir.path().join("args.json"),
        dir,
        rt,
    };
    let store = t.rt.store();
    assert_eq!(
        store.get_run(&run.id).await.unwrap().status,
        RunStatus::AwaitingApproval
    );

    let mut moved = dot.spec.clone();
    moved.workdir = t.dir.path().join("elsewhere").to_string_lossy().to_string();
    let res = t.rt.update_dot(&dot.id, moved.clone()).await;
    assert!(matches!(res, Err(Error::Conflict(_))), "{res:?}");
    let mut remoded = dot.spec.clone();
    remoded.workspace_mode = WorkspaceMode::Worktree;
    let res = t.rt.update_dot(&dot.id, remoded).await;
    assert!(matches!(res, Err(Error::Conflict(_))), "{res:?}");
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec, dot.spec);

    // Other fields stay editable.
    let mut tweaked = dot.spec.clone();
    tweaked.model = "haiku".into();
    t.rt.update_dot(&dot.id, tweaked).await.unwrap();

    t.rt.cancel_run(&run.id).await.unwrap();
    t.rt.update_dot(&dot.id, moved).await.unwrap();
    t.rt.shutdown();
}

#[tokio::test]
async fn start_resumes_awaiting_runs_with_parked_decisions() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (dot_id, parent_id) = {
        let store = Store::open(&data.join("dots.db")).await.unwrap();
        let mut s = spec("restarted");
        s.workdir = dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        let dot = store.create_dot(&s).await.unwrap();
        let run = store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        store.claim_run(&run.id).await.unwrap();
        store.set_session(&run.id, "s-before-crash").await.unwrap();
        store
            .finish_run(&run.id, RunStatus::AwaitingApproval, None, None)
            .await
            .unwrap();
        let a = store
            .create_approval(&run.id, "Bash", &json!({"command": "git push"}))
            .await
            .unwrap();
        // Decided and parked, but the process died before the resume was created.
        store
            .decide_approval_with_parked(&a.id, true, None, true)
            .await
            .unwrap();
        (dot.id, run.id)
    };
    let mut cfg = Config::new(data);
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    let runs = rt.store().list_runs(Some(&dot_id), 10).await.unwrap();
    let child = runs
        .iter()
        .find(|r| r.trigger == TriggerKind::Resume)
        .expect("resume child created at start");
    assert_eq!(child.parent_run_id.as_deref(), Some(parent_id.as_str()));
    assert_eq!(child.session_id.as_deref(), Some("s-before-crash"));
    assert_eq!(
        rt.store().get_run(&parent_id).await.unwrap().status,
        RunStatus::Succeeded
    );
    rt.shutdown();
}
