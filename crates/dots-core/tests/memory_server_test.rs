//! The memory tools on the Claude engine (dot memory spec section 3): Dots' own `dots` MCP
//! server lists `remember`, `forget` and `recall`, checks each call against the dot's policy
//! under `Memory`, and performs it for the run's dot; `--allowedTools` names them. Driven
//! through `fake-claude`.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::checkins::CheckIns;
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::followups::FollowUps;
use dots_core::memory::{Memories, DUPLICATE, NOTHING, PERSONS};
use dots_core::model::{
    Dot, DotSpec, EngineKind, MemorySource, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::SystemClock;
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
    memories: Memories,
    base: String,
    args_out: PathBuf,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, and the local server.
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
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
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
    let follow_ups = FollowUps::new(store.clone(), bus.clone(), Arc::new(SystemClock));
    let checkins = CheckIns::new(store.clone(), bus.clone(), follow_ups);
    let memories = checkins.memories().clone();
    runner.set_checkins(checkins);
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner: runner.clone(),
            hub,
        },
        CancellationToken::new(),
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        dir,
        store,
        runner,
        memories,
        base,
        args_out,
    }
}

/// A Claude dot in the test folder, under the Sandboxed preset unless `edit` changes it.
async fn claude_dot(e: &Env, name: &str, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec(name);
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 0;
    edit(&mut s);
    e.store.create_dot(&s).await.unwrap()
}

/// A run of `dot` that the test calls the `dots` server for, and its secret. It never starts.
async fn registered(e: &Env, dot: &Dot) -> (Run, String) {
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();
    let secret = e.runner.register_secret(&run.id);
    (run, secret)
}

/// The answer of Dots' own server to `body`, for the run of `secret`.
async fn rpc(e: &Env, secret: &str, body: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{}/mcp", e.base))
        .bearer_auth(secret)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Calls the `dots` server's tool `name` for the run of `secret`.
async fn call(e: &Env, secret: &str, name: &str, arguments: Value) -> Value {
    rpc(
        e,
        secret,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": name, "arguments": arguments}}),
    )
    .await
}

fn text(answer: &Value) -> &str {
    answer["result"]["content"][0]["text"].as_str().unwrap()
}

/// The text of a call that was not performed.
fn refused(answer: &Value) -> String {
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    text(answer).to_string()
}

#[tokio::test]
async fn dots_own_server_serves_the_memory_tools_for_the_runs_dot() {
    let e = env(json!([])).await;
    let dot = claude_dot(&e, "invoices", |_| {}).await;
    let other = claude_dot(&e, "other", |_| {}).await;
    let theirs = e
        .memories
        .add(&other.id, "Another dot's fact.")
        .await
        .unwrap();
    let persons = e.memories.add(&dot.id, "Sends on Fridays.").await.unwrap();
    let (run, secret) = registered(&e, &dot).await;

    let listed = rpc(
        &e,
        &secret,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "approve",
            "notify",
            "ask",
            "follow_up",
            "remember",
            "forget",
            "recall"
        ]
    );
    assert_eq!(
        listed["result"]["tools"][4]["inputSchema"]["required"],
        json!(["text"])
    );

    let saved = call(
        &e,
        &secret,
        "remember",
        json!({"text": "Prefers invoices as PDF."}),
    )
    .await;
    assert!(saved["result"].get("isError").is_none(), "{saved}");
    let list = e.memories.list(&dot.id).await.unwrap();
    let memory = list.iter().find(|m| m.source == MemorySource::Dot).unwrap();
    assert_eq!(text(&saved), format!("Remembered {}.", memory.id));
    assert_eq!(memory.run_id.as_deref(), Some(run.id.as_str()));
    let again = call(
        &e,
        &secret,
        "remember",
        json!({"text": "PREFERS invoices as PDF."}),
    )
    .await;
    assert_eq!(refused(&again), DUPLICATE);
    // A text that is not a string is no memory.
    let odd = call(&e, &secret, "remember", json!({"text": 7})).await;
    assert_eq!(refused(&odd), "A memory is 1 to 500 characters.");

    let found = call(&e, &secret, "recall", json!({"query": "pdf"})).await;
    assert_eq!(
        text(&found),
        format!("[{}] Prefers invoices as PDF.", memory.id)
    );
    let none = call(&e, &secret, "recall", json!({"query": "another"})).await;
    assert_eq!(text(&none), NOTHING);
    assert_eq!(
        refused(&call(&e, &secret, "forget", json!({"id": persons.id})).await),
        PERSONS
    );
    assert_eq!(
        refused(&call(&e, &secret, "forget", json!({"id": theirs.id})).await),
        format!("No memory {}.", theirs.id)
    );
    let forgot = call(&e, &secret, "forget", json!({"id": memory.id})).await;
    assert_eq!(text(&forgot), format!("Forgot {}.", memory.id));
    assert_eq!(e.memories.list(&dot.id).await.unwrap(), vec![persons]);
    assert_eq!(e.memories.list(&other.id).await.unwrap(), vec![theirs]);
}

#[tokio::test]
async fn dots_own_server_checks_each_memory_call_against_the_dots_policy() {
    let e = env(json!([])).await;
    let denying = claude_dot(&e, "denying", |s| {
        s.policy.rules.insert(0, Rule::new("Memory", Action::Deny));
    })
    .await;
    let asking = claude_dot(&e, "asking", |s| {
        s.policy.rules.push(Rule::new("Memory", Action::Ask));
    })
    .await;

    let (_, secret) = registered(&e, &denying).await;
    let denied = call(&e, &secret, "recall", json!({})).await;
    assert!(
        refused(&denied).contains("Denied by the policy of dot 'denying'"),
        "{denied}"
    );
    // An ask rule wins over the preset's allow: with no wait, the call parks on an approval
    // named `Memory`, and nothing is saved.
    let (asked_in, secret) = registered(&e, &asking).await;
    let parked = call(&e, &secret, "remember", json!({"text": "Ship on Mondays."})).await;
    assert!(
        refused(&parked).starts_with("Queued for human approval #"),
        "{parked}"
    );
    let approvals = e.store.approvals_for_run(&asked_in.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "Memory");
    assert_eq!(approvals[0].input, json!({"text": "Ship on Mondays."}));
    assert!(e.memories.list(&asking.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_claude_run_may_call_the_memory_tools_without_asking_the_cli() {
    let e = env(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "remember",
                      "arguments": {"text": "Prefers invoices as PDF."}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]))
    .await;
    let dot = claude_dot(&e, "claude", |_| {}).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&e.args_out).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    let allowed: Vec<String> = args
        .iter()
        .skip_while(|a| *a != "--allowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    for tool in [
        "mcp__dots__remember",
        "mcp__dots__forget",
        "mcp__dots__recall",
    ] {
        assert!(allowed.contains(&tool.to_string()), "{args:?}");
    }
    assert!(!allowed.contains(&"Memory".to_string()), "{args:?}");
    let list = e.memories.list(&dot.id).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].text, "Prefers invoices as PDF.");
    assert_eq!(list[0].run_id.as_deref(), Some(run.id.as_str()));
}

/// The final review: Dots' server keeps only the called tool's own argument, so the person
/// approves exactly the tool that will run, and an approval for one memory tool cannot be
/// redeemed by another with the same arguments.
#[tokio::test]
async fn a_memory_call_is_checked_with_its_own_argument_only() {
    let e = env(json!([])).await;
    let asking = claude_dot(&e, "asking", |s| {
        s.policy.rules.push(Rule::new("Memory", Action::Ask));
    })
    .await;
    let (run, secret) = registered(&e, &asking).await;
    let mixed = json!({"text": "Prefers PDF", "id": "01HXAAAAAAAAAAAAAAAAAAAAAA", "query": "x"});
    call(&e, &secret, "remember", mixed.clone()).await;
    call(&e, &secret, "forget", mixed.clone()).await;
    call(&e, &secret, "recall", mixed).await;
    let inputs: Vec<Value> = e
        .store
        .approvals_for_run(&run.id)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.input)
        .collect();
    assert_eq!(
        inputs,
        [
            json!({"text": "Prefers PDF"}),
            json!({"id": "01HXAAAAAAAAAAAAAAAAAAAAAA"}),
            json!({"query": "x"}),
        ]
    );
}
