mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{init_repo, spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::{new_bus, Bus};
use dots_core::model::{Dot, EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

fn runner_with(
    store: &Store,
    bus: &Bus,
    dir: &TempDir,
    engine: Arc<dyn Engine>,
    max: usize,
) -> Arc<Runner> {
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine);
    Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: max,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
        },
    )
}

async fn folder_dot(store: &Store, dir: &TempDir, name: &str) -> Dot {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    store.create_dot(&s).await.unwrap()
}

async fn manual(runner: &Runner, dot: &Dot) -> String {
    runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn successful_run_records_events_and_succeeds() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(ScriptedEngine::new(vec![
        Step::Emit(EngineEvent::SessionStarted {
            session_id: "s1".into(),
        }),
        Step::Emit(EngineEvent::AssistantText {
            text: "working".into(),
        }),
        Step::Emit(EngineEvent::Usage {
            tokens_in: 10,
            tokens_out: 5,
        }),
        finished(),
    ]));
    let runner = runner_with(&store, &bus, &dir, engine, 2);
    let mut events = bus.subscribe();
    runner.spawn_dispatcher(CancellationToken::new());
    let dot = folder_dot(&store, &dir, "ok").await;
    let id = manual(&runner, &dot).await;
    let run = wait_status(&store, &id, RunStatus::Succeeded, 5).await;
    assert_eq!(run.summary.as_deref(), Some("done"));
    assert_eq!(run.session_id.as_deref(), Some("s1"));
    assert_eq!((run.tokens_in, run.tokens_out), (10, 5));
    assert_eq!(
        run.workspace_path.as_deref(),
        Some(dot.spec.workdir.as_str())
    );
    let kinds: Vec<String> = store
        .list_events(&id, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        ["session_started", "assistant_text", "usage", "finished"]
    );
    let mut saw_run_event = false;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, dots_core::events::RuntimeEvent::RunEvent { .. }) {
            saw_run_event = true;
        }
    }
    assert!(saw_run_event);
}

#[tokio::test]
async fn engine_failure_and_missing_result_fail_the_run() {
    // Separate stores: two dispatchers on one store would race for each other's runs.
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let failing = runner_with(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![Step::Emit(EngineEvent::Failed {
            error: "boom".into(),
        })])),
        2,
    );
    failing.spawn_dispatcher(CancellationToken::new());
    let a = folder_dot(&store, &dir, "a").await;
    let id = manual(&failing, &a).await;
    assert_eq!(
        wait_status(&store, &id, RunStatus::Failed, 5)
            .await
            .error
            .as_deref(),
        Some("boom")
    );

    let (dir2, store2) = temp_store().await;
    let silent = runner_with(
        &store2,
        &bus,
        &dir2,
        Arc::new(ScriptedEngine::new(vec![])),
        2,
    );
    silent.spawn_dispatcher(CancellationToken::new());
    let b = folder_dot(&store2, &dir2, "b").await;
    let id = manual(&silent, &b).await;
    let run = wait_status(&store2, &id, RunStatus::Failed, 5).await;
    assert_eq!(run.error.as_deref(), Some("engine exited without a result"));
}

#[tokio::test]
async fn timeout_fails_run_and_frees_slot() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![Step::Hang])),
        1,
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let mut s = spec("hang");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.timeout_secs = 1;
    let hang = store.create_dot(&s).await.unwrap();
    let other = folder_dot(&store, &dir, "other").await;
    let hung = manual(&runner, &hang).await;
    let next = manual(&runner, &other).await;
    let run = wait_status(&store, &hung, RunStatus::Failed, 5).await;
    assert_eq!(run.error.as_deref(), Some("timeout after 1s"));
    // The slot is freed even though the engine never closed its channel.
    wait_status(&store, &next, RunStatus::Running, 5).await;
}

#[tokio::test]
async fn user_cancel_of_running_and_queued_runs() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![Step::WaitForCancel])),
        1,
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let a = folder_dot(&store, &dir, "a").await;
    let b = folder_dot(&store, &dir, "b").await;
    let running = manual(&runner, &a).await;
    wait_status(&store, &running, RunStatus::Running, 5).await;
    let queued = manual(&runner, &b).await;
    let cancelled_queued = runner.cancel(&queued).await.unwrap();
    assert_eq!(cancelled_queued.status, RunStatus::Cancelled);
    runner.cancel(&running).await.unwrap();
    wait_status(&store, &running, RunStatus::Cancelled, 5).await;
    assert!(matches!(
        runner.cancel(&running).await,
        Err(dots_core::Error::Conflict(_))
    ));
}

#[tokio::test]
async fn concurrency_limit_and_one_run_per_dot() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(ScriptedEngine::new(vec![
        Step::Sleep(Duration::from_millis(400)),
        finished(),
    ]));
    let runner = runner_with(&store, &bus, &dir, engine, 2);
    runner.spawn_dispatcher(CancellationToken::new());
    let a = folder_dot(&store, &dir, "a").await;
    let b = folder_dot(&store, &dir, "b").await;
    let c = folder_dot(&store, &dir, "c").await;
    let a1 = manual(&runner, &a).await;
    let a2 = manual(&runner, &a).await;
    let b1 = manual(&runner, &b).await;
    let c1 = manual(&runner, &c).await;
    // Sample until every run has finished: never more than two running, never a1 and a2
    // together, and the limit is actually reached at some point.
    let ids = [a1.clone(), a2.clone(), b1, c1];
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut saw_two = false;
    loop {
        let runs = store.list_runs(None, 10).await.unwrap();
        let running: Vec<&String> = runs
            .iter()
            .filter(|r| r.status == RunStatus::Running)
            .map(|r| &r.id)
            .collect();
        assert!(running.len() <= 2, "{running:?}");
        assert!(!(running.contains(&&a1) && running.contains(&&a2)));
        saw_two |= running.len() == 2;
        let done = runs
            .iter()
            .filter(|r| ids.contains(&r.id) && r.status == RunStatus::Succeeded)
            .count();
        if done == ids.len() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{runs:#?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saw_two, "the concurrency limit was never reached");
}

#[tokio::test]
async fn missing_engine_fails_with_clear_error() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![finished()])),
        1,
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let mut s = spec("local");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some("http://127.0.0.1:11434".into());
    let dot = store.create_dot(&s).await.unwrap();
    let id = manual(&runner, &dot).await;
    let run = wait_status(&store, &id, RunStatus::Failed, 5).await;
    assert_eq!(
        run.error.as_deref(),
        Some("invalid input: engine 'local' is not available")
    );

    // A worktree-mode dot with a missing engine must not leave a worktree behind.
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let mut w = spec("local-wt");
    w.workdir = repo.to_string_lossy().to_string();
    w.engine = EngineKind::Local;
    w.endpoint_url = Some("http://127.0.0.1:11434".into());
    let wt_dot = store.create_dot(&w).await.unwrap();
    let id = manual(&runner, &wt_dot).await;
    wait_status(&store, &id, RunStatus::Failed, 5).await;
    assert!(!dir.path().join("wt").join(&id).exists());
}

#[tokio::test]
async fn parked_approval_leaves_run_awaiting() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = Arc::new(ScriptedEngine::with_gate(
        vec![
            Step::Ask {
                tool: "Bash".into(),
                input: json!({"command": "git push"}),
            },
            finished(),
        ],
        hub,
    ));
    let runner = runner_with(&store, &bus, &dir, engine, 1);
    runner.spawn_dispatcher(CancellationToken::new());
    let mut s = spec("ask");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.approval_wait_secs = 0;
    let dot = store.create_dot(&s).await.unwrap();
    let id = manual(&runner, &dot).await;
    let run = wait_status(&store, &id, RunStatus::AwaitingApproval, 5).await;
    assert_eq!(run.summary.as_deref(), Some("done"));
    assert!(store.has_pending_for_run(&id).await.unwrap());
}

#[tokio::test]
async fn unchanged_worktree_is_removed_after_success() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![finished()])),
        1,
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let mut s = spec("wt");
    s.workdir = repo.to_string_lossy().to_string();
    let dot = store.create_dot(&s).await.unwrap();
    let id = manual(&runner, &dot).await;
    let run = wait_status(&store, &id, RunStatus::Succeeded, 10).await;
    let path = run.workspace_path.unwrap();
    assert!(path.ends_with(&id));
    assert!(!std::path::Path::new(&path).exists());
}

#[tokio::test]
async fn recover_delegates_to_store() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(&store, &bus, &dir, Arc::new(ScriptedEngine::new(vec![])), 1);
    let dot = folder_dot(&store, &dir, "r").await;
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.claim_run(&run.id).await.unwrap();
    assert_eq!(runner.recover().await.unwrap(), vec![run.id]);
}

// ---- final review: I3 / M1 ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_right_after_enqueue_always_succeeds() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(
        &store,
        &bus,
        &dir,
        Arc::new(ScriptedEngine::new(vec![Step::WaitForCancel])),
        4,
    );
    runner.spawn_dispatcher(CancellationToken::new());
    for i in 0..60u64 {
        let dot = folder_dot(&store, &dir, &format!("c{i}")).await;
        let id = manual(&runner, &dot).await;
        if i % 3 != 0 {
            tokio::time::sleep(Duration::from_micros(200 * (i % 20))).await;
        }
        // No "starting; try again" window: queued, claimed or running all cancel.
        runner.cancel(&id).await.unwrap();
        wait_status(&store, &id, RunStatus::Cancelled, 5).await;
    }
}

struct PanicEngine;

#[async_trait::async_trait]
impl Engine for PanicEngine {
    async fn start(
        &self,
        _ctx: dots_core::engine::RunContext,
    ) -> dots_core::Result<tokio::sync::mpsc::Receiver<EngineEvent>> {
        panic!("engine exploded");
    }
}

#[tokio::test]
async fn panicking_engine_fails_the_run_and_frees_the_slot() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let runner = runner_with(&store, &bus, &dir, Arc::new(PanicEngine), 1);
    runner.spawn_dispatcher(CancellationToken::new());
    let a = folder_dot(&store, &dir, "a").await;
    let b = folder_dot(&store, &dir, "b").await;
    let first = manual(&runner, &a).await;
    let second = manual(&runner, &b).await;
    for id in [first, second] {
        let run = wait_status(&store, &id, RunStatus::Failed, 5).await;
        assert_eq!(
            run.error.as_deref(),
            Some("internal error: engine task panicked")
        );
    }
    assert!(!runner.is_active("anything"));
}
