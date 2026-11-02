//! The stuck-run watchdog in the runner (stuck-run watchdog spec sections 3 and 4.2): a run
//! quiet for its limit is marked and announced, output or Keep waiting clears the mark, a
//! pending approval pauses the clock, and a run still quiet at twice the limit is stopped.
//!
//! One limit-second lasts 10 ms here, so a 60 s limit warns at 600 ms and stops at 1.2 s.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{Dot, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

const STOPPED: &str = "stopped: no output for 2 min";

fn text(t: &str) -> Step {
    Step::Emit(EngineEvent::AssistantText { text: t.into() })
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

fn runner_with(store: &Store, bus: &Bus, dir: &TempDir, engine: Arc<dyn Engine>) -> Arc<Runner> {
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine);
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_millis(10),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    runner
}

/// A folder dot with the given quiet limit and time limit.
async fn folder_dot(
    store: &Store,
    dir: &TempDir,
    name: &str,
    quiet: Option<u64>,
    timeout_secs: u64,
    approval_wait_secs: u64,
) -> Dot {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.quiet_limit_secs = quiet;
    s.timeout_secs = timeout_secs;
    s.approval_wait_secs = approval_wait_secs;
    store.create_dot(&s, None, "default").await.unwrap()
}

async fn quiet_dot(store: &Store, dir: &TempDir, name: &str) -> Dot {
    folder_dot(store, dir, name, Some(60), 3600, 240).await
}

async fn manual(runner: &Runner, dot: &Dot) -> String {
    runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
        .id
}

/// The next `RunQuiet` for `run_id` (its `quiet_since`) and the run row announced with it,
/// waiting up to `secs`.
async fn next_quiet(
    events: &mut Receiver<RuntimeEvent>,
    run_id: &str,
    secs: u64,
) -> (Option<String>, Option<u64>, Run) {
    let wait = async {
        let mut quiet = None;
        loop {
            match events.recv().await.unwrap() {
                RuntimeEvent::RunQuiet {
                    run_id: id,
                    quiet_since,
                    quiet_limit_secs,
                    ..
                } if id == run_id => quiet = Some((quiet_since, quiet_limit_secs)),
                RuntimeEvent::RunUpdated { run } if run.id == run_id => {
                    if let Some((since, limit)) = quiet.take() {
                        return (since, limit, run);
                    }
                }
                _ => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(secs), wait)
        .await
        .expect("no RunQuiet in time")
}

/// Every `RunQuiet` on the bus so far, for `run_id`.
fn quiet_events(events: &mut Receiver<RuntimeEvent>, run_id: &str) -> usize {
    let mut n = 0;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, RuntimeEvent::RunQuiet { run_id: ref id, .. } if id == run_id) {
            n += 1;
        }
    }
    n
}

#[tokio::test]
async fn a_quiet_run_is_marked_and_announced() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let mut events = bus.subscribe();
    let engine = Arc::new(ScriptedEngine::new(vec![
        text("working"),
        Step::WaitForCancel,
    ]));
    let runner = runner_with(&store, &bus, &dir, engine);
    // A 120 s limit: the mark at 1.2 s, the stop not before 2.4 s, so the cancel has room.
    let dot = folder_dot(&store, &dir, "quiet", Some(120), 3600, 240).await;
    let id = manual(&runner, &dot).await;

    let (since, limit, run) = next_quiet(&mut events, &id, 5).await;
    assert!(since.is_some());
    assert_eq!(limit, Some(120));
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.quiet_since, since);
    assert_eq!(run.quiet_limit_secs, Some(120));

    runner.cancel(&id).await.unwrap();
    let run = wait_status(&store, &id, RunStatus::Cancelled, 10).await;
    assert_eq!(run.quiet_since, None);
    assert!(!run.quiet_stopped);
}

#[tokio::test]
async fn output_clears_the_mark() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let mut events = bus.subscribe();
    let engine = Arc::new(ScriptedEngine::new(vec![
        Step::Sleep(Duration::from_millis(1800)),
        text("still here"),
        Step::WaitForCancel,
    ]));
    let runner = runner_with(&store, &bus, &dir, engine);
    // A 120 s limit: the mark at 1.2 s, the output at 1.8 s, the stop not before 2.4 s, so a
    // slow machine has room on both sides.
    let dot = folder_dot(&store, &dir, "talks", Some(120), 3600, 240).await;
    let id = manual(&runner, &dot).await;

    let (since, _, _) = next_quiet(&mut events, &id, 5).await;
    assert!(since.is_some());
    let (since, limit, run) = next_quiet(&mut events, &id, 5).await;
    assert_eq!(since, None);
    assert_eq!(limit, Some(120));
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.quiet_since, None);

    runner.cancel(&id).await.unwrap();
    wait_status(&store, &id, RunStatus::Cancelled, 10).await;
}

#[tokio::test]
async fn an_unanswered_quiet_run_is_stopped_at_twice_the_limit() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(ScriptedEngine::new(vec![Step::WaitForCancel]));
    let runner = runner_with(&store, &bus, &dir, engine);
    let dot = quiet_dot(&store, &dir, "silent").await;
    let id = manual(&runner, &dot).await;

    let run = wait_status(&store, &id, RunStatus::Failed, 10).await;
    assert_eq!(run.error.as_deref(), Some(STOPPED));
    assert!(run.quiet_stopped);
    assert_eq!(run.quiet_since, None);
}

/// Review Focus 1: the engine ignores the stop, the grace runs out, and the ending still names
/// the watchdog.
#[tokio::test]
async fn a_run_that_ignores_the_stop_still_ends_with_the_watchdogs_reason() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(ScriptedEngine::new(vec![Step::Hang]));
    let runner = runner_with(&store, &bus, &dir, engine);
    let dot = quiet_dot(&store, &dir, "hung").await;
    let id = manual(&runner, &dot).await;

    let run = wait_status(&store, &id, RunStatus::Failed, 10).await;
    assert_eq!(run.error.as_deref(), Some(STOPPED));
    assert!(run.quiet_stopped);
}

#[tokio::test]
async fn keep_waiting_restarts_the_clock() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let mut events = bus.subscribe();
    let engine = Arc::new(ScriptedEngine::new(vec![Step::WaitForCancel]));
    let runner = runner_with(&store, &bus, &dir, engine);
    // A 120 s limit (1.2 s): a second of slack between the mark and the stop.
    let dot = folder_dot(&store, &dir, "patient", Some(120), 3600, 240).await;
    let id = manual(&runner, &dot).await;

    let (since, _, _) = next_quiet(&mut events, &id, 5).await;
    assert!(since.is_some());
    let t0 = std::time::Instant::now();
    runner.keep_waiting(&id).await.unwrap();
    let (since, _, run) = next_quiet(&mut events, &id, 5).await;
    assert_eq!(since, None);
    assert_eq!(run.quiet_since, None);

    // The clock starts again at the press: the next mark comes a full limit after it. (Had only
    // the mark gone, the timer would already be past due and mark the run again at once.)
    let (since, _, _) = next_quiet(&mut events, &id, 5).await;
    assert!(since.is_some(), "a second mark comes a full limit later");
    assert!(
        t0.elapsed() >= Duration::from_millis(1080),
        "the run was marked again {:?} after Keep waiting",
        t0.elapsed()
    );
    assert_eq!(store.get_run(&id).await.unwrap().status, RunStatus::Running);

    runner.cancel(&id).await.unwrap();
    wait_status(&store, &id, RunStatus::Cancelled, 10).await;
}

#[tokio::test]
async fn a_pending_approval_pauses_the_watchdog() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let mut events = bus.subscribe();
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
    let runner = runner_with(&store, &bus, &dir, engine);
    let dot = folder_dot(&store, &dir, "asks", Some(120), 3600, 30).await;
    let id = manual(&runner, &dot).await;

    wait_status(&store, &id, RunStatus::Running, 5).await;
    // The approval is asked for live: wait until it is pending before timing the pause.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !store.has_pending_for_run(&id).await.unwrap() {
        assert!(std::time::Instant::now() < deadline, "no approval asked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Past the stop (2.4 s) a run left quiet would be gone.
    tokio::time::sleep(Duration::from_millis(3000)).await;

    let run = store.get_run(&id).await.unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.quiet_since, None);
    assert_eq!(quiet_events(&mut events, &id), 0);

    runner.cancel(&id).await.unwrap();
    wait_status(&store, &id, RunStatus::Cancelled, 10).await;
}

#[tokio::test]
async fn the_watchdog_off_never_marks() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let mut events = bus.subscribe();
    let engine = Arc::new(ScriptedEngine::new(vec![
        Step::Sleep(Duration::from_secs(2)),
        finished(),
    ]));
    let runner = runner_with(&store, &bus, &dir, engine);
    let dot = folder_dot(&store, &dir, "off", Some(0), 3600, 240).await;
    let id = manual(&runner, &dot).await;

    let run = wait_status(&store, &id, RunStatus::Succeeded, 10).await;
    assert_eq!(run.quiet_limit_secs, None);
    assert!(!run.quiet_stopped);
    assert_eq!(quiet_events(&mut events, &id), 0);
}

#[tokio::test]
async fn the_time_limit_firing_first_keeps_its_reason() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(ScriptedEngine::new(vec![Step::Hang]));
    let runner = runner_with(&store, &bus, &dir, engine);
    let dot = folder_dot(&store, &dir, "slow", Some(86400), 1, 240).await;
    let id = manual(&runner, &dot).await;

    let run = wait_status(&store, &id, RunStatus::Failed, 10).await;
    assert_eq!(run.error.as_deref(), Some("timeout after 1s"));
    assert!(!run.quiet_stopped);
}

#[tokio::test]
async fn a_flooding_engine_still_hits_the_time_limit() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    // Output every poll, and cancellation ignored: only the time limit and the grace
    // period can end this run.
    let engine = Arc::new(ScriptedEngine::new(vec![Step::Flood]));
    let runner = runner_with(&store, &bus, &dir, engine);
    let dot = folder_dot(&store, &dir, "flood", Some(86400), 1, 240).await;
    let id = manual(&runner, &dot).await;

    let run = wait_status(&store, &id, RunStatus::Failed, 15).await;
    assert_eq!(run.error.as_deref(), Some("timeout after 1s"));
    assert!(!run.quiet_stopped);
}

#[tokio::test]
async fn keep_waiting_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    // No run of this test may start: the runs below are moved by hand.
    rt.hold_runs(true).await;
    let store = rt.store();

    assert!(matches!(
        rt.keep_waiting("no-such-run").await,
        Err(Error::NotFound(_))
    ));

    let mut s = spec("held");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = store.create_dot(&s, None, "default").await.unwrap();

    let done = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store.claim_run(&done.id).await.unwrap());
    store
        .finish_run(&done.id, RunStatus::Succeeded, Some("done"), None)
        .await
        .unwrap();
    assert!(matches!(
        rt.keep_waiting(&done.id).await,
        Err(Error::Conflict(_))
    ));

    let running = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store.claim_run(&running.id).await.unwrap());
    let err = rt.keep_waiting(&running.id).await.unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("is not quiet")),
        "{err:?}"
    );
}
