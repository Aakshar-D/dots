mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::{ApprovalHub, DecideEffect};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::new_bus;
use dots_core::model::{Dot, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
}

async fn env(steps: Vec<Step>) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(
        EngineKind::Claude,
        Arc::new(ScriptedEngine::with_gate(steps, hub.clone())),
    );
    let runner = Runner::new(
        store.clone(),
        bus,
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        dir,
        store,
        hub,
        runner,
    }
}

async fn parking_dot(e: &Env) -> Dot {
    let mut s = spec("resumer");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.approval_wait_secs = 0;
    e.store.create_dot(&s).await.unwrap()
}

/// What Runtime::decide_approval does (Task 14).
async fn decide(e: &Env, id: &str, approved: bool, note: Option<&str>) -> Option<Run> {
    match e
        .hub
        .decide(id, approved, note.map(String::from))
        .await
        .unwrap()
    {
        DecideEffect::Parked(a) => e.runner.maybe_resume(&a.run_id).await.unwrap(),
        DecideEffect::Live(_) => None,
    }
}

fn ask(cmd: &str) -> Step {
    Step::Ask {
        tool: "Bash".into(),
        input: json!({"command": cmd, "description": "x"}),
    }
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

fn message(run: &Run) -> String {
    run.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn approve_after_park_resumes_child_with_grant() {
    let e = env(vec![
        Step::Emit(EngineEvent::SessionStarted {
            session_id: "s1".into(),
        }),
        ask("git push"),
        finished(),
    ])
    .await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 5).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);

    let child = decide(&e, &approval.id, true, None)
        .await
        .expect("child run");
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
    assert_eq!(child.trigger, TriggerKind::Resume);
    assert_eq!(child.parent_run_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.root_run_id, parent.id);
    assert_eq!(child.session_id.as_deref(), Some("s1"));
    assert!(message(&child).contains(&format!("Approval #{} granted", approval.id)));
    assert_eq!(
        child.payload.as_ref().unwrap()["approvals"],
        json!([approval.id])
    );

    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    let outputs: Vec<Value> = e
        .store
        .list_events(&child.id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_result")
        .map(|ev| ev.data["output"].clone())
        .collect();
    assert_eq!(outputs, vec![json!("allowed")]);
}

#[tokio::test]
async fn deny_with_note_resumes_with_the_note() {
    let e = env(vec![ask("git push"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 5).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    let child = decide(&e, &approval.id, false, Some("open a PR instead"))
        .await
        .expect("child run");
    assert!(message(&child).contains("Reviewer note: open a PR instead"));
    // No grant was created, so the replayed script parks again.
    wait_status(&e.store, &child.id, RunStatus::AwaitingApproval, 5).await;
}

#[tokio::test]
async fn plain_deny_closes_the_parent_without_a_child() {
    let e = env(vec![ask("git push"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 5).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert!(decide(&e, &approval.id, false, None).await.is_none());
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn late_decision_while_running_resumes_after_finish() {
    let e = env(vec![
        ask("git push"),
        Step::Sleep(Duration::from_millis(400)),
        finished(),
    ])
    .await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let approval = loop {
        let list = e.store.approvals_for_run(&parent.id).await.unwrap();
        if let Some(a) = list.into_iter().find(|a| a.parked) {
            break a;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Running
    );
    assert!(
        decide(&e, &approval.id, true, None).await.is_none(),
        "run still running"
    );

    let runs = loop {
        let runs = e.store.list_runs(Some(&dot.id), 10).await.unwrap();
        if runs.len() == 2 {
            break runs;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let child = runs.iter().find(|r| r.id != parent.id).unwrap();
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
}

#[tokio::test]
async fn two_parked_approvals_resume_once_after_both_are_decided() {
    let e = env(vec![ask("git push"), ask("git push --tags"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 5).await;
    let approvals = e.store.approvals_for_run(&parent.id).await.unwrap();
    assert_eq!(approvals.len(), 2);
    assert!(decide(&e, &approvals[0].id, true, None).await.is_none());
    let child = decide(&e, &approvals[1].id, true, None)
        .await
        .expect("child run");
    assert_eq!(
        child.payload.as_ref().unwrap()["approvals"],
        json!([approvals[0].id, approvals[1].id])
    );
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 2);
}
