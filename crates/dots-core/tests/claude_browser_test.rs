//! Browser use on the Claude engine (browser spec sections 2.2 and 3.6), driven through
//! `fake-claude`, with `fake-browser` as Edge and `fake-mcp` as Playwright: the run's
//! `--mcp-config` and `--allowedTools`, its calls through the gateway route `/mcp/s/browser`,
//! a late approval that is skipped and told while its rule is saved, and a Library server
//! named `browser`.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::{grant_key, ApprovalHub, DecideEffect};
use dots_core::browser::{Browsers, LATE_APPROVAL};
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::library::LibraryConfig;
use dots_core::library_service::LibraryService;
use dots_core::model::{
    ApplyState, ApprovalStatus, Dot, DotSpec, EngineKind, NewRun, Run, RunEventRecord, RunStatus,
    TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::server::{bind, serve, ServerState};
use dots_core::settings::Settings;
use dots_core::store::Store;
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
    lib: LibraryService,
    base: String,
    args_out: PathBuf,
    dir: TempDir,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, the local server, and
/// the dots' browsers on `fake-browser` and `fake-mcp`.
async fn env(script: Value) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let args_out = dir.path().join("args.json");
    let engine = ClaudeEngine::new(
        PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")),
        dir.path().join("scratch"),
    )
    .with_env("FAKE_CLAUDE_SCRIPT", &script_path.to_string_lossy())
    .with_env("FAKE_CLAUDE_ARGS_OUT", &args_out.to_string_lossy());
    let listener = bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: format!("{base}/mcp"),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_secs(2),
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
    runner.set_browsers(Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        port,
    ));
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
    Env {
        edges: FakeEdges::new(&root),
        playwright,
        lib: LibraryService::new(store.clone(), bus),
        store,
        hub,
        runner,
        base,
        args_out,
        dir,
    }
}

/// A Claude dot with the browser on, in the test folder; asking parks at once. Its Edge opens
/// on the cart page.
async fn browser_dot(e: &Env, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec("shopper");
    s.workdir = e.dir.path().to_string_lossy().into_owned();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.browser = true;
    s.approval_wait_secs = 0;
    edit(&mut s);
    let dot = e.store.create_dot(&s).await.unwrap();
    e.edges.first_pages(&dot.id, &[CART]);
    dot
}

async fn start(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

/// What `fake-claude` recorded of its last start: `args`, `mcp_config`, `prompt`, ...
fn recorded(e: &Env) -> Value {
    serde_json::from_str(&std::fs::read_to_string(&e.args_out).unwrap()).unwrap()
}

/// The values after `flag`, up to the next flag.
fn values_of(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .skip_while(|a| *a != flag)
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect()
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

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

fn success() -> Value {
    json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                    "usage": {"input_tokens": 1, "output_tokens": 1}}})
}

fn browser_call(tool: &str, arguments: Value) -> Value {
    json!({"mcp_call": {"server": "browser", "tool": tool, "arguments": arguments}})
}

/// The resume run of `parent`, once the runner made it.
async fn resume_child(e: &Env, parent: &Run) -> Run {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let runs = e.store.list_runs(Some(&parent.dot_id), 10).await.unwrap();
        if let Some(r) = runs
            .into_iter()
            .find(|r| r.parent_run_id.as_deref() == Some(parent.id.as_str()))
        {
            return r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no resume run appeared"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn called(e: &Env) -> Vec<String> {
    e.playwright
        .received("tools/call")
        .iter()
        .map(|m| m["params"]["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_claude_run_reaches_the_browser_through_the_gateway() {
    let e = env(json!([
        init(),
        {"mcp_list": "browser"},
        browser_call("browser_snapshot", json!({})),
        browser_call("browser_click", json!({"target": "e6"})),
        success()
    ]))
    .await;
    let dot = browser_dot(&e, |s| {
        s.policy
            .rules
            .push(Rule::new("Browser(shop.com)", Action::Allow))
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 30).await;

    let rec = recorded(&e);
    let args: Vec<String> = serde_json::from_value(rec["args"].clone()).unwrap();
    let allowed = values_of(&args, "--allowedTools");
    assert!(allowed.contains(&"mcp__browser__*".to_string()), "{args:?}");
    assert!(
        values_of(&args, "--disallowedTools")
            .iter()
            .all(|t| !t.starts_with("Browser")),
        "{args:?}"
    );
    let servers = &rec["mcp_config"]["mcpServers"];
    assert_eq!(servers.as_object().unwrap().len(), 2, "{servers}");
    assert_eq!(
        servers["browser"],
        json!({"type": "http", "url": format!("{}/mcp/s/browser", e.base),
               "headers": {"Authorization": servers["dots"]["headers"]["Authorization"]}})
    );
    // The CLI saw the pinned list, and its calls went through the gate to Playwright.
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(listed["tools"].as_array().unwrap().len(), 25);
    assert_eq!(listed["tools"][0], "browser_close");
    let results = events(&e, &run.id, "tool_result").await;
    let clicked = results[1].data["output"].as_str().unwrap();
    assert!(clicked.starts_with("Dots: Clicked \"Home\"\n"), "{clicked}");
    assert_eq!(called(&e), ["browser_snapshot", "browser_click"]);
    // The run's end closed its Playwright server and its Edge.
    assert!(e.playwright.gone_within(Duration::from_secs(10)).await);
    assert!(e.edges.gone_within(&dot.id, Duration::from_secs(10)).await);
}

#[tokio::test]
async fn a_late_browser_approval_is_skipped_and_the_dot_is_told() {
    let e = env(json!([
        init(),
        browser_call("browser_snapshot", json!({})),
        browser_call("browser_click", json!({"target": "e16"})),
        success()
    ]))
    .await;
    let dot = browser_dot(&e, |_| {}).await;
    let parent = start(&e, &dot).await;
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 30).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approval.tool, "mcp__browser__browser_click");
    // Approved after the run parked, with "always allow on shop.com".
    let decided = e
        .hub
        .decide_with_rule(
            &approval.id,
            true,
            None,
            Some(Rule::new("Browser(shop.com)", Action::Allow)),
        )
        .await
        .unwrap();
    assert!(matches!(decided, DecideEffect::Parked(_)));
    e.runner.maybe_resume(&parent.id).await.unwrap();
    let child = resume_child(&e, &parent).await;
    let message = child.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        message.contains(&format!(
            "- Approval #{} for a browser action: {LATE_APPROVAL}",
            approval.id
        )),
        "{message}"
    );
    assert!(!message.contains("granted"), "{message}");
    let skipped = e.store.get_approval(&approval.id).await.unwrap();
    assert_eq!(skipped.status, ApprovalStatus::Approved);
    assert_eq!(skipped.apply_state, Some(ApplyState::Skipped));
    assert!(skipped.resolved);
    assert_eq!(
        e.store
            .take_mcp_grant(
                &parent.root_run_id,
                &approval.tool,
                &grant_key(&approval.tool, &approval.input)
            )
            .await
            .unwrap(),
        None,
        "no grant: the click is never retried by itself"
    );
    // The rule chosen with it is saved.
    assert!(e
        .store
        .get_dot(&dot.id)
        .await
        .unwrap()
        .spec
        .policy
        .rules
        .contains(&Rule::new("Browser(shop.com)", Action::Allow)));
    // The resumed run is told; its own click of "Pay now" is a new call, which asks again.
    wait_status(&e.store, &child.id, RunStatus::AwaitingApproval, 30).await;
    assert_eq!(recorded(&e)["prompt"].as_str().unwrap(), message);
    assert!(!called(&e).contains(&"browser_click".to_string()));
}

#[tokio::test]
async fn a_library_server_named_browser_leaves_the_built_in_out() {
    let e = env(json!([init(), success()])).await;
    let fake = FakeMcp::new(json!({"tools": [tool("look", true)]}));
    let probe = e.lib.probe(&fake.stdio()).await.unwrap();
    e.lib
        .add(
            "browser",
            LibraryConfig::McpServer(fake.stdio()),
            Some(&probe.fingerprint),
        )
        .await
        .unwrap();
    let dot = browser_dot(&e, |s| s.uses.mcp_servers = vec!["browser".into()]).await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 30).await;
    let notices: Vec<String> = events(&e, &run.id, "notice")
        .await
        .iter()
        .map(|ev| ev.data["text"].as_str().unwrap().to_string())
        .collect();
    assert!(
        notices.contains(
            &"browser: an MCP server this dot uses is named browser, so the built-in browser \
              is left out of this run; rename that server in the Library to use both"
                .to_string()
        ),
        "{notices:?}"
    );
    assert!(e.edges.starts(&dot.id).is_empty());
}

/// Browser spec section 14: Dots' server has the doors' route, and a secret that opens no door
/// is refused with 404, WebSocket or not.
#[tokio::test]
async fn the_server_refuses_a_door_without_its_secret() {
    let e = env(json!([init(), success()])).await;
    let ws = e.base.replace("http://", "ws://");
    match tokio_tungstenite::connect_async(format!("{ws}/cdp/not-a-secret")).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(answer)) => {
            assert_eq!(answer.status().as_u16(), 404)
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
    let plain = reqwest::get(format!("{}/cdp/not-a-secret", e.base))
        .await
        .unwrap();
    assert_eq!(plain.status().as_u16(), 404);
}
