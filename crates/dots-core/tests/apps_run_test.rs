//! A run's apps (app library spec section 6): the tools of the apps a dot uses, offered on both
//! engines as `<app>__<tool>`, checked under the policy name `App`, called as the run's dot, and
//! named in the prompt's "Apps" section.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::apps::{expenses, write_app};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::apps::call::Actor;
use dots_core::apps::run::{attach, PROMPT_INTRO};
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::checkins::CheckIns;
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::followups::FollowUps;
use dots_core::library::AppOrigin;
use dots_core::model::{
    ApplyState, Dot, DotSpec, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::run_tools::RunTools;
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::SystemClock;
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use dots_core::{Config, Runtime};
use llm::{call as llm_call, messages, text as llm_text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const SECTION_LOCAL: &str = "- Expenses (expenses): Logs expenses and totals them by month. \
    Tools: expenses__add_expense (writes), expenses__list_expenses, expenses__month_total\n";

async fn install(apps: &Apps) {
    let src = tempfile::tempdir().unwrap();
    write_app(src.path(), &expenses());
    apps.install(read_folder(src.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
}

fn expense(item: &str, amount: f64) -> Value {
    json!({"item": item, "amount": amount, "on": "2026-10-22"})
}

struct Env {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
    apps: Apps,
    base: String,
    args_out: PathBuf,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, the local server, and the
/// Expenses app installed.
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
    runner.set_checkins(CheckIns::new(store.clone(), bus.clone(), follow_ups));
    let apps = Apps::new(store.clone(), bus.clone(), dir.path().join("apps"));
    install(&apps).await;
    runner.set_apps(apps.clone());
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
        apps,
        base,
        args_out,
    }
}

/// A Claude dot in the test folder that uses Expenses, under the Sandboxed preset unless `edit`
/// changes it.
async fn claude_dot(e: &Env, name: &str, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec(name);
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 0;
    s.uses.apps = vec!["expenses".into()];
    edit(&mut s);
    e.store.create_dot(&s, None, "default").await.unwrap()
}

/// A run of `dot` with its apps open, that the test calls the `dots` server for, and its secret.
/// It never starts.
async fn registered(e: &Env, dot: &Dot) -> (Run, String) {
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();
    let mut notices = Vec::new();
    let tools = attach(
        Some(&e.apps),
        dot,
        &run.id,
        RunTools::default(),
        &mut notices,
    )
    .await
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    e.runner.register_tools(&run.id, Arc::new(tools));
    let secret = e.runner.register_secret(&run.id);
    (run, secret)
}

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

async fn listed(e: &Env, secret: &str) -> Vec<Value> {
    let answer = rpc(
        e,
        secret,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    answer["result"]["tools"].as_array().unwrap().clone()
}

/// The app tools among `tools`: the names with `__`.
fn names(tools: &[Value]) -> Vec<String> {
    tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .filter(|n| n.contains("__"))
        .collect()
}

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

fn refused(answer: &Value) -> String {
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    text(answer).to_string()
}

/// The items of October's expenses, as the person's screen would list them.
async fn month(apps: &Apps) -> Vec<String> {
    let rows = apps
        .call(
            "expenses",
            "list_expenses",
            json!({"month": "2026-10"}),
            Actor::Person,
        )
        .await
        .unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["item"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn dots_own_server_serves_a_runs_app_tools_and_calls_them_as_its_dot() {
    let e = env(json!([])).await;
    let dot = claude_dot(&e, "builder", |_| {}).await;
    let other = claude_dot(&e, "other", |s| s.uses.apps.clear()).await;
    let (run, secret) = registered(&e, &dot).await;

    let tools = listed(&e, &secret).await;
    assert_eq!(
        names(&tools),
        [
            "expenses__add_expense",
            "expenses__list_expenses",
            "expenses__month_total"
        ]
    );
    let add = tools
        .iter()
        .find(|t| t["name"] == "expenses__add_expense")
        .unwrap();
    assert_eq!(add["description"], "Log an expense");
    assert_eq!(
        add["inputSchema"]["required"],
        json!(["item", "amount", "on"])
    );

    let saved = call(&e, &secret, "expenses__add_expense", expense("Coffee", 4.5)).await;
    assert!(saved["result"].get("isError").is_none(), "{saved}");
    assert!(text(&saved).starts_with("{\"id\":\""), "{saved}");
    let total = call(
        &e,
        &secret,
        "expenses__month_total",
        json!({"month": "2026-10"}),
    )
    .await;
    assert_eq!(text(&total), "4.5");
    // The input is checked against the tool's schema before the code runs.
    let bad = call(&e, &secret, "expenses__add_expense", json!({"item": "Tea"})).await;
    let said = refused(&bad);
    assert!(said.starts_with("expenses__add_expense: "), "{said}");
    assert!(said.contains("amount is required"), "{said}");
    assert_eq!(month(&e.apps).await, ["Coffee"]);

    // The writes are logged with the dot and its run; the read is not.
    let (item, _) = e.apps.get("expenses").await.unwrap();
    let logged = e.store.app_calls(&item.id, 10).await.unwrap();
    let tools_logged: Vec<&str> = logged.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools_logged, ["add_expense", "add_expense"]);
    let ok = logged.iter().find(|c| c.error.is_none()).unwrap();
    assert_eq!(
        ok.actor,
        Actor::Dot {
            id: dot.id.clone(),
            name: "builder".into(),
            run_id: run.id.clone()
        }
    );
    assert_eq!(ok.run_id.as_deref(), Some(run.id.as_str()));

    // A dot that does not use the app has no such tools.
    let (_, theirs) = registered(&e, &other).await;
    assert!(names(&listed(&e, &theirs).await).is_empty());
    let unknown = call(&e, &theirs, "expenses__add_expense", expense("Ink", 1.0)).await;
    assert_eq!(unknown["error"]["message"], "unknown tool", "{unknown}");
    assert_eq!(month(&e.apps).await, ["Coffee"]);
}

#[tokio::test]
async fn an_app_rule_hides_what_it_denies_and_parks_what_it_asks_about() {
    let e = env(json!([])).await;
    let reader = claude_dot(&e, "reader", |s| {
        s.policy
            .rules
            .insert(0, Rule::new("App(expenses.write)", Action::Deny));
    })
    .await;
    let none = claude_dot(&e, "none", |s| {
        s.policy
            .rules
            .insert(0, Rule::new("App(expenses)", Action::Deny));
    })
    .await;
    let asking = claude_dot(&e, "asking", |s| {
        s.policy
            .rules
            .push(Rule::new("App(expenses.write)", Action::Ask));
    })
    .await;

    let (_, secret) = registered(&e, &reader).await;
    assert_eq!(
        names(&listed(&e, &secret).await),
        ["expenses__list_expenses", "expenses__month_total"]
    );
    let (_, secret) = registered(&e, &none).await;
    assert!(names(&listed(&e, &secret).await).is_empty());

    // An ask rule wins over the preset's allow: with no wait, the write parks on an approval
    // named `App` whose input says which app and tool, and nothing is written. A read does not
    // ask.
    let (asked_in, secret) = registered(&e, &asking).await;
    let parked = call(&e, &secret, "expenses__add_expense", expense("Rent", 900.0)).await;
    assert!(
        refused(&parked).starts_with("Queued for human approval #"),
        "{parked}"
    );
    let approvals = e.store.approvals_for_run(&asked_in.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "App");
    assert_eq!(
        approvals[0].input,
        json!({"app": "expenses", "tool": "add_expense", "writes": true,
               "input": expense("Rent", 900.0)})
    );
    assert!(month(&e.apps).await.is_empty());
    let read = call(
        &e,
        &secret,
        "expenses__month_total",
        json!({"month": "2026-10"}),
    )
    .await;
    assert_eq!(text(&read), "0");
}

#[tokio::test]
async fn a_claude_run_calls_its_app_tools_without_asking_the_cli_and_its_prompt_names_them() {
    let e = env(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "expenses__add_expense",
                      "arguments": {"item": "Coffee", "amount": 4.5, "on": "2026-10-22"}}},
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
        "mcp__dots__expenses__add_expense",
        "mcp__dots__expenses__list_expenses",
        "mcp__dots__expenses__month_total",
    ] {
        assert!(allowed.contains(&tool.to_string()), "{args:?}");
    }
    assert!(!allowed.iter().any(|a| a.starts_with("App")), "{args:?}");
    let prompt = recorded["prompt"].as_str().unwrap();
    assert!(
        prompt.contains(&format!(
            "\n## Apps\n\n{PROMPT_INTRO}- Expenses (expenses): Logs expenses and totals them by \
             month. Tools: mcp__dots__expenses__add_expense (writes), \
             mcp__dots__expenses__list_expenses, mcp__dots__expenses__month_total\n"
        )),
        "{prompt}"
    );
    assert_eq!(month(&e.apps).await, ["Coffee"]);
}

#[tokio::test]
async fn an_app_whose_code_cannot_be_read_is_left_out_of_the_run_with_a_notice() {
    let e = env(json!([])).await;
    let dot = claude_dot(&e, "builder", |_| {}).await;
    let (item, config) = e.apps.get("expenses").await.unwrap();
    std::fs::write(
        e.apps.code_dir(&item.id, config.version).join("app.json"),
        "{",
    )
    .unwrap();
    let mut notices = Vec::new();
    let tools = attach(Some(&e.apps), &dot, "r1", RunTools::default(), &mut notices)
        .await
        .unwrap();
    assert!(tools.apps().is_none());
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].starts_with("app expenses: "), "{notices:?}");
    assert!(notices[0].contains("app.json"), "{notices:?}");
}

#[tokio::test]
async fn a_dot_without_apps_gets_no_apps_section() {
    let e = env(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]))
    .await;
    let dot = claude_dot(&e, "plain", |s| s.uses.apps.clear()).await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&e.args_out).unwrap()).unwrap();
    assert!(!recorded["prompt"].as_str().unwrap().contains("## Apps"));
    let args = recorded["args"].to_string();
    assert!(!args.contains("expenses__"), "{args}");
}

// --- the local engine ---

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    install(rt.apps()).await;
    Rt { dir, rt }
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
    s.uses.apps = vec!["expenses".into()];
    s
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_local_dot_calls_its_app_tools_as_built_in_tools_named_app() {
    let t = start().await;
    let llm = Llm::start(vec![
        llm_call("c1", "expenses__add_expense", expense("Coffee", 4.5)),
        llm_call("c2", "expenses__month_total", json!({"month": "2026-10"})),
        llm_call(
            "c3",
            "expenses__add_expense",
            json!({"item": "", "amount": 1, "on": "2026-10-22"}),
        ),
        llm_text("Done."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "builder"), None, None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    let names = tool_names(&reqs[0]);
    for name in [
        "expenses__add_expense",
        "expenses__list_expenses",
        "expenses__month_total",
    ] {
        assert!(names.iter().any(|n| n == name), "{names:?}");
    }
    let add = reqs[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["function"]["name"] == "expenses__add_expense")
        .unwrap();
    assert_eq!(
        add["function"]["parameters"]["required"],
        json!(["item", "amount", "on"])
    );
    let prompt = messages(&reqs[0])
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        prompt.contains(&format!("\n## Apps\n\n{PROMPT_INTRO}{SECTION_LOCAL}")),
        "{prompt}"
    );
    assert!(tool_result(&reqs[1], "c1").starts_with("{\"id\":\""));
    assert_eq!(tool_result(&reqs[2], "c2"), "4.5");
    let bad = tool_result(&reqs[3], "c3");
    assert!(bad.contains("at least 1 characters"), "{bad}");
    assert_eq!(month(t.rt.apps()).await, ["Coffee"]);
    // The transcript names each call by the policy name, with the app and tool in its input.
    let calls: Vec<Value> =
        t.rt.store()
            .list_events(&run.id, 0)
            .await
            .unwrap()
            .into_iter()
            .filter(|ev| ev.kind == "tool_call")
            .map(|ev| ev.data)
            .collect();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c["tool"] == "App"), "{calls:?}");
    assert_eq!(
        calls[1]["input"],
        json!({"app": "expenses", "tool": "month_total", "writes": false,
               "input": {"month": "2026-10"}})
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn an_app_rule_that_asks_parks_the_call_and_the_decision_is_carried_out() {
    let t = start().await;
    let llm = Llm::start(vec![
        llm_call("c1", "expenses__add_expense", expense("Rent", 900.0)),
        llm_text("Queued."),
        llm_text("Done after approval."),
        llm_call("c2", "expenses__add_expense", expense("Gift", 50.0)),
        llm_text("Queued again."),
        llm_text("Not added."),
    ])
    .await;
    let mut s = local_spec(&t, &llm.endpoint(), "asking");
    s.approval_wait_secs = 0;
    s.policy
        .rules
        .push(Rule::new("App(expenses.write)", Action::Ask));
    let dot = t.rt.create_dot(s, None, None).await.unwrap();

    // Approved: Dots performs the parked call in the resume run, as the dot.
    let parent = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &parent.id, RunStatus::AwaitingApproval, 20).await;
    let approval =
        t.rt.store()
            .approvals_for_run(&parent.id)
            .await
            .unwrap()
            .remove(0);
    assert_eq!(approval.tool, "App");
    assert_eq!(approval.input["tool"], "add_expense");
    assert!(month(t.rt.apps()).await.is_empty());
    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    loop {
        let a = t.rt.store().get_approval(&approval.id).await.unwrap();
        if a.apply_state == Some(ApplyState::Applied) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(month(t.rt.apps()).await, ["Rent"]);
    let resumed =
        t.rt.store()
            .list_runs(Some(&dot.id), 10)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.trigger == TriggerKind::Resume)
            .unwrap();
    wait_status(t.rt.store(), &resumed.id, RunStatus::Succeeded, 30).await;

    // Denied with a note: the resume run names the tool as the model knows it.
    let parent = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &parent.id, RunStatus::AwaitingApproval, 20).await;
    let approval =
        t.rt.store()
            .approvals_for_run(&parent.id)
            .await
            .unwrap()
            .remove(0);
    t.rt.decide_approval(&approval.id, false, Some("No gifts.".into()), None)
        .await
        .unwrap();
    let denied = loop {
        let runs = t.rt.store().list_runs(Some(&dot.id), 10).await.unwrap();
        if let Some(r) = runs
            .into_iter()
            .find(|r| r.parent_run_id.as_deref() == Some(parent.id.as_str()))
        {
            break r;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    wait_status(t.rt.store(), &denied.id, RunStatus::Succeeded, 30).await;
    let told = denied.payload.unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        told.contains(&format!(
            "- Approval #{} for expenses__add_expense was denied. Reviewer note: No gifts.",
            approval.id
        )),
        "{told}"
    );
    assert_eq!(month(t.rt.apps()).await, ["Rent"]);
    t.rt.shutdown();
}
