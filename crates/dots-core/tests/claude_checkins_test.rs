//! The check-in tools on the Claude engine (check-ins spec section 2): Dots' own `dots` MCP
//! server lists `notify`, `ask` and `follow_up`, checks each call against the dot's policy
//! under `Notify`, `Ask` and `FollowUp`, and performs it; `--allowedTools` names them, so the
//! CLI does not ask `approve` first. Driven through `fake-claude`.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::{ApprovalHub, DecideEffect};
use dots_core::checkins::{local_label, CheckIns, ASKED, ASKED_ALREADY, SENT};
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::followups::{FollowUps, TOO_FAR, TOO_MANY, TOO_SOON};
use dots_core::model::{
    Dot, DotSpec, EngineKind, MessageKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind,
    WorkspaceMode,
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
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    base: String,
    args_out: PathBuf,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, the local server, and
/// the check-ins on the PC's clock. Without `dispatch` nothing runs.
async fn env(script: Value, dispatch: bool) -> Env {
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
    runner.set_checkins(CheckIns::new(store.clone(), bus.clone(), follow_ups));
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner: runner.clone(),
            hub: hub.clone(),
        },
        CancellationToken::new(),
    );
    if dispatch {
        runner.spawn_dispatcher(CancellationToken::new());
    }
    Env {
        dir,
        store,
        hub,
        runner,
        base,
        args_out,
    }
}

/// A Claude dot in the test folder, under the Sandboxed preset unless `edit` changes it.
async fn claude_dot(e: &Env, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec("checker");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 0;
    edit(&mut s);
    e.store.create_dot(&s).await.unwrap()
}

async fn start(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

/// A run of `dot` that the test calls the `dots` server for, and its secret. It never starts.
async fn registered(e: &Env, dot: &Dot) -> (Run, String) {
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let secret = e.runner.register_secret(&run.id);
    (run, secret)
}

/// What `fake-claude` recorded of its last start.
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

fn mcp_call(server: &str, tool: &str, arguments: Value) -> Value {
    json!({"mcp_call": {"server": server, "tool": tool, "arguments": arguments}})
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

#[tokio::test]
async fn a_claude_run_checks_in_through_dots_own_server() {
    let e = env(
        json!([init(),
               {"mcp_list": "dots"},
               mcp_call("dots", "notify", json!({"text": "Half way there."})),
               mcp_call("dots", "ask", json!({"question": "Ship it?", "choices": ["yes", "no"]})),
               mcp_call("dots", "ask", json!({"question": "Really?"})),
               mcp_call("dots", "follow_up", json!({"note": "Check the build.", "in": "2h"})),
               mcp_call("dots", "notify", json!({"text": "  "})),
               success()]),
        true,
    )
    .await;
    let dot = claude_dot(&e, |_| {}).await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let args: Vec<String> = serde_json::from_value(recorded(&e)["args"].clone()).unwrap();
    let allowed = values_of(&args, "--allowedTools");
    for tool in [
        "mcp__dots__notify",
        "mcp__dots__ask",
        "mcp__dots__follow_up",
    ] {
        assert!(allowed.contains(&tool.to_string()), "{args:?}");
    }
    for name in ["Notify", "Ask", "FollowUp"] {
        assert!(!allowed.contains(&name.to_string()), "{args:?}");
    }
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(
        listed["tools"],
        json!([
            "approve",
            "notify",
            "ask",
            "follow_up",
            "remember",
            "forget",
            "recall"
        ])
    );
    let calls: Vec<Value> = events(&e, &run.id, "tool_call")
        .await
        .into_iter()
        .map(|ev| ev.data["tool"].clone())
        .collect();
    assert_eq!(
        calls,
        [
            json!("mcp__dots__notify"),
            json!("mcp__dots__ask"),
            json!("mcp__dots__ask"),
            json!("mcp__dots__follow_up"),
            json!("mcp__dots__notify"),
        ]
    );
    let pending = e
        .store
        .list_pending_follow_ups(Some(&dot.id))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].from_run_id, run.id);
    let due = DateTime::parse_from_rfc3339(&pending[0].due_at)
        .unwrap()
        .with_timezone(&Utc);
    let results: Vec<(String, bool)> = events(&e, &run.id, "tool_result")
        .await
        .into_iter()
        .map(|ev| {
            (
                ev.data["output"].as_str().unwrap().to_string(),
                ev.data["is_error"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            (SENT.to_string(), false),
            (ASKED.to_string(), false),
            (ASKED_ALREADY.to_string(), true),
            (
                format!(
                    "Follow-up {} scheduled for {}.",
                    pending[0].id,
                    local_label(due)
                ),
                false
            ),
            ("`text` must not be blank.".to_string(), true),
        ]
    );
    let messages = e.store.list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(messages.len(), 2);
    let question = messages
        .iter()
        .find(|m| m.kind == MessageKind::Question)
        .unwrap();
    assert_eq!(question.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(
        question.choices,
        Some(vec!["yes".to_string(), "no".to_string()])
    );
}

#[tokio::test]
async fn dots_own_server_checks_each_check_in_against_the_dots_policy() {
    let e = env(json!([]), false).await;
    let plain = claude_dot(&e, |_| {}).await;
    let denying = claude_dot(&e, |s| {
        s.name = "denying".into();
        s.policy.rules.insert(0, Rule::new("Notify", Action::Deny));
    })
    .await;
    let asking = claude_dot(&e, |s| {
        s.name = "asking".into();
        // deny > allow > ask: the preset's bare `FollowUp` allow would win over an ask rule.
        s.policy.rules.retain(|r| r.tool != "FollowUp");
        s.policy.rules.push(Rule::new("FollowUp", Action::Ask));
    })
    .await;

    let (_, plain_secret) = registered(&e, &plain).await;
    let listed = rpc(
        &e,
        &plain_secret,
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
        listed["result"]["tools"][3]["inputSchema"]["required"],
        json!(["note"])
    );
    let sent = call(&e, &plain_secret, "notify", json!({"text": "Hello."})).await;
    assert_eq!(text(&sent), SENT);
    assert!(sent["result"].get("isError").is_none(), "{sent}");

    // The dot's own rule decides: a `Notify` deny rule denies the call, and nothing is stored.
    let (_, denying_secret) = registered(&e, &denying).await;
    let denied = call(&e, &denying_secret, "notify", json!({"text": "Hello."})).await;
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert!(
        text(&denied).contains("Denied by the policy of dot 'denying'"),
        "{denied}"
    );
    assert!(e
        .store
        .list_messages(Some(&denying.id))
        .await
        .unwrap()
        .is_empty());

    // An ask rule asks: with no wait, the call parks on an approval and nothing is scheduled.
    let (asked_in, asking_secret) = registered(&e, &asking).await;
    let parked = call(
        &e,
        &asking_secret,
        "follow_up",
        json!({"note": "Later.", "in": "1h"}),
    )
    .await;
    assert_eq!(parked["result"]["isError"], true, "{parked}");
    let approvals = e.store.approvals_for_run(&asked_in.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "FollowUp");
    assert!(e
        .store
        .list_pending_follow_ups(Some(&asking.id))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn dots_own_server_answers_a_broken_rule_with_its_sentence() {
    let e = env(json!([]), false).await;
    let dot = claude_dot(&e, |_| {}).await;
    let (_, secret) = registered(&e, &dot).await;
    let refused = |answer: &Value| -> String {
        assert_eq!(answer["result"]["isError"], true, "{answer}");
        text(answer).to_string()
    };
    let far = (Utc::now() + TimeDelta::days(31)).to_rfc3339();
    assert_eq!(
        refused(&call(&e, &secret, "follow_up", json!({"note": "n", "in": "4m"})).await),
        TOO_SOON
    );
    assert_eq!(
        refused(&call(&e, &secret, "follow_up", json!({"note": "n", "at": &far})).await),
        TOO_FAR
    );
    assert_eq!(
        refused(
            &call(
                &e,
                &secret,
                "follow_up",
                json!({"note": "n", "in": "2h", "at": &far})
            )
            .await
        ),
        "Give exactly one of `at` or `in`."
    );
    assert_eq!(
        refused(
            &call(
                &e,
                &secret,
                "ask",
                json!({"question": "q", "choices": ["a", "b", "c", "d", "e", "f"]})
            )
            .await
        ),
        "`choices` must be a list of 1 to 5 strings."
    );
    assert_eq!(
        refused(&call(&e, &secret, "notify", json!({"text": "x".repeat(2_001)})).await),
        "`text` must be at most 2,000 characters."
    );
    for later in ["1h", "2h", "3h"] {
        let ok = call(&e, &secret, "follow_up", json!({"note": "n", "in": later})).await;
        assert!(ok["result"].get("isError").is_none(), "{ok}");
    }
    assert_eq!(
        refused(&call(&e, &secret, "follow_up", json!({"note": "n", "in": "4h"})).await),
        TOO_MANY
    );
    // Arguments that are not an object are a protocol error, as for `context`.
    let bad = rpc(
        &e,
        &secret,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "notify", "arguments": "hi"}}),
    )
    .await;
    assert_eq!(bad["error"]["code"], -32602, "{bad}");
}

#[tokio::test]
async fn a_parked_check_in_resumes_with_the_name_the_model_knows() {
    let e = env(
        json!([
            init(),
            mcp_call("dots", "follow_up", json!({"note": "Later.", "in": "1h"})),
            success()
        ]),
        true,
    )
    .await;
    let dot = claude_dot(&e, |s| {
        s.policy.rules.retain(|r| r.tool != "FollowUp");
        s.policy.rules.push(Rule::new("FollowUp", Action::Ask));
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::AwaitingApproval, 20).await;
    let approval = e.store.approvals_for_run(&run.id).await.unwrap().remove(0);
    assert_eq!(approval.tool, "FollowUp");
    assert!(matches!(
        e.hub.decide(&approval.id, true, None).await.unwrap(),
        DecideEffect::Parked(_)
    ));
    e.runner.maybe_resume(&run.id).await.unwrap();
    let child = resume_child(&e, &run).await;
    let message = child.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        message.contains("you may now use mcp__dots__follow_up with the same input"),
        "{message}"
    );
    // The resumed run calls again, and the grant lets the call through.
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await;
    assert_eq!(
        e.store
            .list_pending_follow_ups(Some(&dot.id))
            .await
            .unwrap()
            .len(),
        1
    );
}
