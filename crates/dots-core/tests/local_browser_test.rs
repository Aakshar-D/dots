//! Browser use on the local engine (browser spec sections 2.2, 3.5 and 3.6), with a scripted
//! endpoint, `fake-browser` as Edge and `fake-mcp` as Playwright: the 25 functions offered to a
//! dot with the browser and none to one without, calls decided by the gate, password typing
//! refused before Playwright, and a late approval that Dots does not perform.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
use common::fake_mcp::FakeMcp;
use common::{spec, temp_store, wait_status};
use dots_core::approvals::{ApprovalHub, DecideEffect};
use dots_core::browser::gate::PASSWORD_REFUSAL;
use dots_core::browser::{Browsers, LATE_APPROVAL};
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::{
    ApplyState, Dot, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::settings::Settings;
use dots_core::store::Store;
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Dropped in this order: the fakes quit, then the rest, then the folder.
struct Env {
    edges: FakeEdges,
    playwright: FakeMcp,
    store: Store,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    ws: PathBuf,
    _dir: TempDir,
}

async fn env() -> Env {
    let (dir, store) = temp_store().await;
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine =
        LocalEngine::with_retry_delays(store.clone(), hub.clone(), vec![Duration::ZERO; 2]);
    hub.set_self_apply([EngineKind::Local]);
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
            plugins_dir: dir.path().join("plugins"),
        },
    );
    store
        .put_settings(&Settings {
            edge_path: Some(exe()),
            ..Settings::default()
        })
        .await
        .unwrap();
    let root = dir.path().join("browser");
    let playwright = FakeMcp::new(cart_script());
    // No server listens on port 1: `fake-mcp` never connects to its door.
    runner.set_browsers(Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        1,
    ));
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        edges: FakeEdges::new(&root),
        playwright,
        store,
        hub,
        runner,
        ws,
        _dir: dir,
    }
}

/// A local dot on `llm`, with the browser on or off, under `rules`; asking parks at once. Its
/// Edge opens on the cart page.
async fn local_dot(e: &Env, llm: &Llm, browser: bool, rules: &[(&str, Action)]) -> Dot {
    let mut s = spec("loco");
    s.workdir = e.ws.to_string_lossy().into_owned();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "test-model".into();
    s.approval_wait_secs = 0;
    s.browser = browser;
    s.policy
        .rules
        .extend(rules.iter().map(|(tool, action)| Rule::new(tool, *action)));
    let dot = e.store.create_dot(&s, None, "default").await.unwrap();
    e.edges.first_pages(&dot.id, &[CART]);
    dot
}

async fn run(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

/// The function names of a recorded request.
fn functions(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

fn called(e: &Env) -> Vec<String> {
    e.playwright
        .received("tools/call")
        .iter()
        .map(|m| m["params"]["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn the_local_engine_offers_the_browser_and_its_gate_decides() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "mcp__browser__browser_snapshot", json!({})),
        call("c2", "mcp__browser__browser_click", json!({"target": "e6"})),
        text("done"),
    ])
    .await;
    let dot = local_dot(&e, &llm, true, &[("Browser(shop.com)", Action::Allow)]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    let names = functions(&reqs[0]);
    assert_eq!(
        names
            .iter()
            .filter(|n| n.starts_with("mcp__browser__"))
            .count(),
        25
    );
    assert!(names.contains(&"mcp__browser__browser_click".to_string()));
    let clicked = tool_result(&reqs[2], "c2");
    assert!(clicked.starts_with("Dots: Clicked \"Home\"\n"), "{clicked}");
    assert_eq!(called(&e), ["browser_snapshot", "browser_click"]);
    assert!(e.edges.gone_within(&dot.id, Duration::from_secs(10)).await);
}

#[tokio::test]
async fn a_dot_without_the_browser_has_no_browser_tools() {
    let e = env().await;
    let llm = Llm::start(vec![text("done")]).await;
    let dot = local_dot(&e, &llm, false, &[]).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 30).await;
    assert!(functions(&llm.requests()[0])
        .iter()
        .all(|n| !n.starts_with("mcp__browser__")));
    assert!(e.edges.starts(&dot.id).is_empty());
}

#[tokio::test]
async fn password_typing_is_refused_before_playwright() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "mcp__browser__browser_snapshot", json!({})),
        call(
            "c2",
            "mcp__browser__browser_type",
            json!({"target": "e13", "text": "hunter2"}),
        ),
        text("done"),
    ])
    .await;
    let everything = [
        ("Browser(*)", Action::Allow),
        ("BrowserRisky(*)", Action::Allow),
    ];
    let dot = local_dot(&e, &llm, true, &everything).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 30).await;
    assert_eq!(tool_result(&llm.requests()[2], "c2"), PASSWORD_REFUSAL);
    assert_eq!(called(&e), ["browser_snapshot"]);
    assert!(e.store.approvals_for_run(&r.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_late_browser_approval_is_not_performed_by_dots() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "mcp__browser__browser_snapshot", json!({})),
        call(
            "c2",
            "mcp__browser__browser_click",
            json!({"target": "e16"}),
        ),
        text("Waiting for the approval."),
        text("I will take a snapshot first."),
    ])
    .await;
    let dot = local_dot(&e, &llm, true, &[("Browser(shop.com)", Action::Allow)]).await;
    let parent = run(&e, &dot).await;
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 30).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert!(
        approval.self_apply,
        "a local dot's late approvals are Dots' to perform"
    );
    let decided = e.hub.decide(&approval.id, true, None).await.unwrap();
    assert!(matches!(decided, DecideEffect::Parked(_)));
    e.runner.maybe_resume(&parent.id).await.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let child = loop {
        let runs = e.store.list_runs(Some(&dot.id), 10).await.unwrap();
        if let Some(r) = runs
            .into_iter()
            .find(|r| r.parent_run_id.as_deref() == Some(parent.id.as_str()))
        {
            break r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no resume run appeared"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 30).await;
    let skipped = e.store.get_approval(&approval.id).await.unwrap();
    assert_eq!(skipped.apply_state, Some(ApplyState::Skipped));
    let last = llm.requests().pop().unwrap();
    assert!(
        messages(&last).iter().any(|m| m["role"] == "user"
            && m["content"]
                .as_str()
                .is_some_and(|c| c.contains(LATE_APPROVAL))),
        "{last:#}"
    );
    assert_eq!(called(&e), ["browser_snapshot"]);
}
