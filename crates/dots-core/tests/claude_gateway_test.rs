//! The Claude engine's MCP servers, skills and plugins (MCP/skills spec sections 5 and 7): the
//! gateway `POST /mcp/s/<server>` that checks each call, and the run's `--mcp-config`,
//! `--allowedTools` and `--plugin-dir`, driven through `fake-claude` and `fake-mcp`.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store, wait_status};
use dots_core::approvals::{parked_message, ApprovalHub, DecideEffect};
use dots_core::engine::claude::ClaudeEngine;
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus};
use dots_core::library::{
    ContextConfig, LibraryConfig, LibraryItem, McpServerConfig, PluginConfig, SkillConfig,
};
use dots_core::library_service::LibraryService;
use dots_core::model::{
    ApprovalStatus, Dot, DotSpec, EngineKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind,
    WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use dots_core::vault::{DotScope, Grants, Vault};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    bus: Bus,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
    lib: LibraryService,
    base: String,
    args_out: PathBuf,
}

/// A runner with the Claude engine on `fake-claude` playing `script`, and the local server.
/// Without `dispatch` nothing runs: a test opens a run's tools itself.
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
        lib: LibraryService::new(store.clone(), bus.clone()),
        dir,
        store,
        bus,
        hub,
        runner,
        base,
        args_out,
    }
}

/// Adds `config` as the MCP server `name`, its tool list accepted as it is now.
async fn add_server(e: &Env, name: &str, config: McpServerConfig) -> (LibraryItem, String) {
    let probe = e.lib.probe(&config).await.unwrap();
    let item = e
        .lib
        .add(
            name,
            LibraryConfig::McpServer(config),
            Some(&probe.fingerprint),
        )
        .await
        .unwrap();
    (item, probe.fingerprint)
}

/// A Claude dot in the test folder, under the Sandboxed preset unless `edit` changes it.
async fn claude_dot(e: &Env, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut s = spec("gw");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 0;
    edit(&mut s);
    e.store.create_dot(&s, None, "default").await.unwrap()
}

async fn start(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

/// What `fake-claude` recorded of its last start: `args`, `mcp_config`, `plugin_dirs`, ...
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
async fn a_claude_run_reaches_its_servers_through_the_gateway_only() {
    let e = env(
        json!([init(),
               {"mcp_list": "files"},
               mcp_call("files", "read", json!({"q": "a"})),
               success()]),
        true,
    )
    .await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    add_server(&e, "files", fake.stdio()).await;
    let starts = || {
        fake.log()
            .iter()
            .filter(|r| r.get("start").is_some())
            .count()
    };
    let before = starts();
    let dot = claude_dot(&e, |s| s.uses.mcp_servers = vec!["files".into()]).await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let rec = recorded(&e);
    let args: Vec<String> = serde_json::from_value(rec["args"].clone()).unwrap();
    assert!(
        values_of(&args, "--allowedTools").contains(&"mcp__files__*".to_string()),
        "{args:?}"
    );
    let servers = &rec["mcp_config"]["mcpServers"];
    assert_eq!(servers.as_object().unwrap().len(), 2, "{servers}");
    assert_eq!(
        servers["files"],
        json!({"type": "http", "url": format!("{}/mcp/s/files", e.base),
               "headers": {"Authorization": servers["dots"]["headers"]["Authorization"]}})
    );
    // Only the run's own session started the server: the CLI reached it through Dots.
    assert_eq!(starts(), before + 1);

    // The CLI saw the session's tools, and its call reached the server.
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(listed["tools"], json!(["read", "write"]));
    let calls = events(&e, &run.id, "tool_call").await;
    assert_eq!(calls[0].data["tool"], "mcp__files__read");
    let result = &events(&e, &run.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], false);
    assert!(
        result["output"].as_str().unwrap().starts_with("read:"),
        "{result}"
    );
    let received = fake.received("tools/call");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0]["params"]["arguments"], json!({"q": "a"}));
    // The run's tools are gone with it.
    assert!(e.runner.run_tools(&run.id).is_none());
    assert!(fake.gone_within(Duration::from_secs(10)).await);
}

/// Runs a dot whose script writes through `files`: the call asks and parks. Returns the parked
/// run and its approval's id.
async fn parked_write(e: &Env, fake: &FakeMcp) -> (Run, String) {
    let dot = claude_dot(e, |s| s.uses.mcp_servers = vec!["files".into()]).await;
    let parent = start(e, &dot).await;
    wait_status(&e.store, &parent.id, RunStatus::AwaitingApproval, 20).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approval.tool, "mcp__files__write");
    assert_eq!(approval.input, json!({"q": "w"}));
    let result = &events(e, &parent.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], true);
    assert_eq!(result["output"], parked_message(&approval.id));
    assert!(fake.received("tools/call").is_empty(), "asked, not sent");
    (parent, approval.id)
}

#[tokio::test]
async fn a_writing_call_asks_and_its_retry_goes_through_with_the_grant() {
    let e = env(
        json!([
            init(),
            mcp_call("files", "write", json!({"q": "w"})),
            success()
        ]),
        true,
    )
    .await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let (_, fingerprint) = add_server(&e, "files", fake.stdio()).await;
    let (parent, approval) = parked_write(&e, &fake).await;
    assert_eq!(
        e.store
            .get_approval(&approval)
            .await
            .unwrap()
            .mcp_fingerprint,
        Some(fingerprint)
    );
    let DecideEffect::Parked(_) = e.hub.decide(&approval, true, None).await.unwrap() else {
        panic!("expected parked")
    };
    e.runner.maybe_resume(&parent.id).await.unwrap();
    let child = resume_child(&e, &parent).await;
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await;
    let result = &events(&e, &child.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], false, "{result}");
    assert!(result["output"].as_str().unwrap().starts_with("write:"));
    assert_eq!(fake.received("tools/call").len(), 1);
}

#[tokio::test]
async fn a_grant_does_not_hold_once_the_servers_accepted_list_changed() {
    let e = env(
        json!([
            init(),
            mcp_call("files", "write", json!({"q": "w"})),
            success()
        ]),
        true,
    )
    .await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let (item, _) = add_server(&e, "files", fake.stdio()).await;
    let (parent, approval) = parked_write(&e, &fake).await;
    // The person accepts a new list before the retry: `write` now says something else.
    let mut write = tool("write", false);
    write["description"] = json!("Writes, and also deletes.");
    fake.rewrite(json!({"tools": [tool("read", true), write]}));
    let seen = e.lib.check(&item.id, &DotScope::Personal).await.unwrap();
    e.lib
        .accept(
            &item.id,
            &DotScope::Personal,
            seen.seen_fingerprint.as_deref().unwrap(),
        )
        .await
        .unwrap();
    e.hub.decide(&approval, true, None).await.unwrap();
    e.runner.maybe_resume(&parent.id).await.unwrap();
    let child = resume_child(&e, &parent).await;
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 20).await;
    let result = &events(&e, &child.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], true);
    assert_eq!(
        result["output"],
        "mcp__files__write: not performed, because the tool changed since it was approved."
    );
    assert!(fake.received("tools/call").is_empty());
    // Nothing was asked again.
    assert!(e
        .store
        .approvals_for_run(&child.id)
        .await
        .unwrap()
        .is_empty());
}

fn write_file(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[tokio::test]
async fn a_claude_run_gets_its_plugin_folders_and_reports_plugin_errors() {
    let e = env(
        json!([{"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION",
                         "plugin_errors": [{"plugin": "forms", "error": "agents/x.md: bad"}]}},
               success()]),
        true,
    )
    .await;
    let skill = e.dir.path().join("lib").join("pdf");
    write_file(
        &skill.join("SKILL.md"),
        "---\nname: pdf\ndescription: Read PDFs.\nallowed-tools: Bash\n---\nBody.\n",
    );
    let fake = FakeMcp::new(json!({"tools": [tool("fill", false)]}));
    let plugin = e.dir.path().join("lib").join("forms");
    write_file(
        &plugin.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "forms"}"#,
    );
    write_file(
        &plugin.join("skills").join("fill").join("SKILL.md"),
        "---\ndescription: Fill forms.\n---\nFill.\n",
    );
    let McpServerConfig::Stdio { command, args, .. } = fake.stdio() else {
        unreachable!()
    };
    write_file(
        &plugin.join(".mcp.json"),
        &json!({"mcpServers": {"files": {"command": command, "args": args}}}).to_string(),
    );
    e.lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig {
                path: skill.to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    e.lib
        .add(
            "forms",
            LibraryConfig::Plugin(PluginConfig {
                path: plugin.to_string_lossy().into_owned(),
                installed_id: None,
                run_hooks: false,
            }),
            None,
        )
        .await
        .unwrap();
    let dot = claude_dot(&e, |s| {
        s.uses.skills = vec!["pdf".into()];
        s.uses.plugins = vec!["forms".into()];
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let rec = recorded(&e);
    let args: Vec<String> = serde_json::from_value(rec["args"].clone()).unwrap();
    let dirs: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--plugin-dir")
        .map(|w| w[1].clone())
        .collect();
    let seen: Vec<String> = rec["plugin_dirs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(dirs, seen);
    assert_eq!(dirs.len(), 2);
    assert!(dirs[0].ends_with("dots-skills"), "{dirs:?}");
    assert!(dirs[1].ends_with("forms"), "{dirs:?}");
    assert_eq!(
        rec["plugin_dirs"][0]["files"],
        json!([".claude-plugin/plugin.json", "skills/pdf/SKILL.md"])
    );
    // The plugin's copy has no `.mcp.json`: its server goes through the gateway.
    assert_eq!(
        rec["plugin_dirs"][1]["files"],
        json!([".claude-plugin/plugin.json", "skills/fill/SKILL.md"])
    );
    let servers = &rec["mcp_config"]["mcpServers"];
    assert_eq!(
        servers["plugin-forms-files"]["url"],
        format!("{}/mcp/s/plugin-forms-files", e.base)
    );
    let allowed = values_of(&args, "--allowedTools");
    assert!(allowed.contains(&"mcp__plugin-forms-files__*".into()));
    // Claude Code's own prefix for the plugin's server is never allowed: a CLI that loads the
    // plugin from the person's settings and starts its server itself is not let past the check.
    assert!(
        allowed.iter().all(|t| !t.starts_with("mcp__plugin_")),
        "{allowed:?}"
    );
    // The folders go with the run.
    assert!(dirs.iter().all(|d| !Path::new(d).exists()));

    let notices: Vec<Value> = events(&e, &run.id, "notice")
        .await
        .into_iter()
        .map(|ev| ev.data["text"].clone())
        .collect();
    assert!(
        notices.contains(&json!("plugin forms: agents/x.md: bad")),
        "{notices:?}"
    );
}

async fn rpc(e: &Env, path: &str, secret: Option<&str>, body: Value) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{}{path}", e.base))
        .json(&body);
    if let Some(s) = secret {
        req = req.bearer_auth(s);
    }
    req.send().await.unwrap()
}

async fn call(e: &Env, secret: &str, tool: &str, arguments: Value) -> Value {
    rpc(
        e,
        "/mcp/s/files",
        Some(secret),
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
               "params": {"name": tool, "arguments": arguments}}),
    )
    .await
    .json()
    .await
    .unwrap()
}

#[tokio::test]
async fn the_gateway_answers_a_running_runs_open_servers_and_checks_each_call() {
    let e = env(json!([]), false).await;
    let fake = FakeMcp::new(json!({"tools": [
        tool("read", true), tool("write", false), tool("drop", false)]}));
    add_server(&e, "files", fake.stdio()).await;
    let dot = claude_dot(&e, |s| {
        s.uses.mcp_servers = vec!["files".into()];
        s.policy.rules.push(Rule {
            tool: "mcp__files__drop".into(),
            action: Action::Deny,
        });
    })
    .await;
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let (tools, notices) = RunTools::open(OpenRequest {
        store: &e.store,
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        scope: &DotScope::Personal,
        workspace_id: "default",
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    let tools = Arc::new(tools);
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, tools.clone());
    let s = Some(secret.as_str());

    // The run's secret, and a server the run has open.
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    assert_eq!(
        rpc(&e, "/mcp/s/files", None, ping.clone()).await.status(),
        401
    );
    assert_eq!(
        rpc(&e, "/mcp/s/files", Some("made-up"), ping.clone())
            .await
            .status(),
        401
    );
    assert_eq!(rpc(&e, "/mcp/s/other", s, ping.clone()).await.status(), 404);
    let other = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let no_tools = e.runner.register_secret(&other.id);
    assert_eq!(
        rpc(&e, "/mcp/s/files", Some(&no_tools), ping.clone())
            .await
            .status(),
        404
    );
    let get = reqwest::Client::new()
        .get(format!("{}/mcp/s/files", e.base))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 405);

    let init: Value = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                          "clientInfo": {"name": "claude-code", "version": "x"}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "files");
    let note = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(note.status(), 202);
    let list: Value = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["read", "write", "drop"]);
    assert_eq!(
        list["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );

    // A read-only tool runs; the server's result comes back as it is.
    let read = call(&e, &secret, "read", json!({"q": "a"})).await;
    assert_eq!(read["result"]["content"][0]["type"], "text");
    assert!(read["result"].get("isError").is_none(), "{read}");
    // A rule of the dot denies; nothing reaches the server.
    let drop = call(&e, &secret, "drop", json!({})).await;
    assert_eq!(
        drop["result"],
        json!({"content": [{"type": "text",
               "text": "Denied by the policy of dot 'gw': mcp__files__drop is not permitted."}],
               "isError": true})
    );
    // A writing tool asks, and asking parks at once.
    let write = call(&e, &secret, "write", json!({"q": "w"})).await;
    let approval = e.store.approvals_for_run(&run.id).await.unwrap().remove(0);
    assert_eq!(approval.tool, "mcp__files__write");
    assert_eq!(
        approval.mcp_fingerprint,
        tools.server("files").unwrap().accepted_fingerprint()
    );
    assert_eq!(write["result"]["isError"], true);
    assert_eq!(
        write["result"]["content"][0]["text"],
        parked_message(&approval.id)
    );
    let received: Vec<Value> = fake
        .received("tools/call")
        .into_iter()
        .map(|m| m["params"]["name"].clone())
        .collect();
    assert_eq!(received, [json!("read")]);

    let bad = call(&e, &secret, "read", json!("not an object")).await;
    assert_eq!(bad["error"]["code"], -32602);
    let unknown: Value = rpc(
        &e,
        "/mcp/s/files",
        s,
        json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);

    // The CLI's `approve` tool leaves a gateway tool to the gateway, which checks every call.
    let approve: Value = rpc(
        &e,
        "/mcp",
        s,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "approve", "arguments": {
                   "tool_name": "mcp__files__write", "input": {"q": "x"}}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    let decision: Value =
        serde_json::from_str(approve["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        decision,
        json!({"behavior": "allow", "updatedInput": {"q": "x"}})
    );
    assert_eq!(e.store.approvals_for_run(&run.id).await.unwrap().len(), 1);

    tools.close().await;
}

/// `approve` for a tool: the decision JSON it answers.
async fn approve(e: &Env, secret: &str, tool: &str) -> Value {
    let reply: Value = rpc(
        e,
        "/mcp",
        Some(secret),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "approve", "arguments": {"tool_name": tool, "input": {}}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn under_trusted_a_writing_gateway_tool_asks_and_so_does_a_tool_outside_the_gateway() {
    let e = env(json!([]), false).await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    add_server(&e, "files", fake.stdio()).await;
    let dot = claude_dot(&e, |s| {
        s.policy = Policy::preset(Preset::Trusted);
        s.uses.mcp_servers = vec!["files".into()];
        s.policy.rules.push(Rule {
            tool: "mcp__allowed__*".into(),
            action: Action::Allow,
        });
        s.policy.rules.push(Rule {
            tool: "mcp__blocked__*".into(),
            action: Action::Deny,
        });
    })
    .await;
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let (tools, _) = RunTools::open(OpenRequest {
        store: &e.store,
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        scope: &DotScope::Personal,
        workspace_id: "default",
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));

    // The gateway tool: a read runs, a write asks and parks.
    let read = call(&e, &secret, "read", json!({})).await;
    assert!(read["result"].get("isError").is_none(), "{read}");
    let write = call(&e, &secret, "write", json!({"q": "w"})).await;
    assert_eq!(write["result"]["isError"], true);
    assert_eq!(e.store.approvals_for_run(&run.id).await.unwrap().len(), 1);
    assert!(fake.received("tools/call").len() == 1);

    // A tool of a server the CLI reached by itself is never decided by Trusted's default allow.
    let other = approve(&e, &secret, "mcp__other__write").await;
    assert_eq!(other["behavior"], "deny", "{other}");
    let approvals = e.store.approvals_for_run(&run.id).await.unwrap();
    assert_eq!(approvals.len(), 2);
    assert_eq!(approvals[1].tool, "mcp__other__write");
    // An explicit allow rule still allows, an explicit deny rule still denies, asking nothing.
    assert_eq!(
        approve(&e, &secret, "mcp__allowed__x").await["behavior"],
        "allow"
    );
    assert_eq!(
        approve(&e, &secret, "mcp__blocked__x").await["behavior"],
        "deny"
    );
    assert_eq!(e.store.approvals_for_run(&run.id).await.unwrap().len(), 2);
    e.runner.run_tools(&run.id).unwrap().close().await;
}

/// A plugin's server under Claude Code's own name for it (`mcp__plugin_<plugin>_<server>__*`, a
/// server the CLI started itself from the person's settings) is not a gateway tool: `approve`
/// decides it by the MCP check, even under Trusted. The gateway's name passes.
#[tokio::test]
async fn a_plugin_server_under_claude_codes_own_name_is_checked_by_approve() {
    let e = env(json!([]), false).await;
    let fake = FakeMcp::new(json!({"tools": [tool("fill", false)]}));
    let plugin = e.dir.path().join("lib").join("forms");
    write_file(
        &plugin.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "forms"}"#,
    );
    let McpServerConfig::Stdio { command, args, .. } = fake.stdio() else {
        unreachable!()
    };
    write_file(
        &plugin.join(".mcp.json"),
        &json!({"mcpServers": {"files": {"command": command, "args": args}}}).to_string(),
    );
    e.lib
        .add(
            "forms",
            LibraryConfig::Plugin(PluginConfig {
                path: plugin.to_string_lossy().into_owned(),
                installed_id: None,
                run_hooks: false,
            }),
            None,
        )
        .await
        .unwrap();
    let dot = claude_dot(&e, |s| {
        s.policy = Policy::preset(Preset::Trusted);
        s.uses.plugins = vec!["forms".into()];
    })
    .await;
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let (tools, _) = RunTools::open(OpenRequest {
        store: &e.store,
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        scope: &DotScope::Personal,
        workspace_id: "default",
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    assert!(tools.server("plugin-forms-files").is_some());
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));

    let gateway = approve(&e, &secret, "mcp__plugin-forms-files__fill").await;
    assert_eq!(gateway["behavior"], "allow", "{gateway}");
    assert!(e.store.approvals_for_run(&run.id).await.unwrap().is_empty());
    let own = approve(&e, &secret, "mcp__plugin_forms_files__fill").await;
    assert_eq!(own["behavior"], "deny", "{own}");
    let approvals = e.store.approvals_for_run(&run.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "mcp__plugin_forms_files__fill");
    e.runner.run_tools(&run.id).unwrap().close().await;
}

/// Adds the context collection `company`: a folder with `style.md`.
async fn add_company(e: &Env) {
    let kb = e.dir.path().join("kb");
    write_file(&kb.join("style.md"), "# House style\nShort sentences.\n");
    e.lib
        .add(
            "company",
            LibraryConfig::Context(ContextConfig {
                path: kb.to_string_lossy().into_owned(),
                description: "How we work".into(),
                every_dot: false,
            }),
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_claude_run_reads_its_collections_through_dots_own_server() {
    let e = env(
        json!([init(),
               {"mcp_list": "dots"},
               mcp_call("dots", "context", json!({"collection": "company", "path": "style.md"})),
               success()]),
        true,
    )
    .await;
    add_company(&e).await;
    let dot = claude_dot(&e, |s| {
        s.policy = Policy::preset(Preset::ReadOnly);
        s.uses.context = vec!["company".into()];
    })
    .await;
    let run = start(&e, &dot).await;
    wait_status(&e.store, &run.id, RunStatus::Succeeded, 20).await;

    let args: Vec<String> = serde_json::from_value(recorded(&e)["args"].clone()).unwrap();
    let allowed = values_of(&args, "--allowedTools");
    assert!(
        allowed.contains(&"mcp__dots__context".to_string()),
        "{args:?}"
    );
    assert!(!allowed.contains(&"Context".to_string()), "{args:?}");
    let listed = events(&e, &run.id, "raw").await;
    let listed: Value = serde_json::from_str(listed[0].data["line"].as_str().unwrap()).unwrap();
    assert_eq!(listed["tools"], json!(["approve", "context"]));
    let calls = events(&e, &run.id, "tool_call").await;
    assert_eq!(calls[0].data["tool"], "mcp__dots__context");
    let result = &events(&e, &run.id, "tool_result").await[0].data;
    assert_eq!(result["is_error"], false, "{result}");
    assert_eq!(result["output"], "# House style\nShort sentences.");
}

#[tokio::test]
async fn dots_own_server_offers_context_only_with_collections_and_checks_each_call() {
    let e = env(json!([]), false).await;
    add_company(&e).await;
    let dot = claude_dot(&e, |s| s.uses.context = vec!["company".into()]).await;
    let bare = claude_dot(&e, |s| s.name = "bare".into()).await;
    let denying = claude_dot(&e, |s| {
        s.name = "denying".into();
        s.uses.context = vec!["company".into()];
        s.policy.rules.insert(0, Rule::new("Context", Action::Deny));
    })
    .await;
    let mut secrets = Vec::new();
    for d in [&dot, &bare, &denying] {
        let run = e
            .store
            .create_run(&NewRun::new(&d.id, TriggerKind::Manual))
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let (tools, _) = RunTools::open(OpenRequest {
            store: &e.store,
            bus: &e.bus,
            run_id: &run.id,
            uses: &d.spec.uses,
            workspace: e.dir.path(),
            plugins_dir: None,
            cancel: &cancel,
            vault: &Vault::in_memory(e.store.clone()),
            scope: &DotScope::Personal,
            workspace_id: "default",
            grants: &Grants::default(),
            dot_name: "gateway",
        })
        .await
        .unwrap();
        let secret = e.runner.register_secret(&run.id);
        e.runner.register_tools(&run.id, Arc::new(tools));
        secrets.push(secret);
    }
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let names = |v: Value| -> Vec<String> {
        v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let with: Value = rpc(&e, "/mcp", Some(&secrets[0]), list.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(names(with.clone()), ["approve", "context"]);
    assert_eq!(
        with["result"]["tools"][1]["inputSchema"]["required"],
        json!(["collection"])
    );
    let without: Value = rpc(&e, "/mcp", Some(&secrets[1]), list)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(names(without), ["approve"]);

    let context = |args: Value| {
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "context", "arguments": args}})
    };
    let ok: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[0]),
        context(json!({"collection": "company"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(
        ok["result"]["content"][0]["text"],
        "- style.md — House style: Short sentences."
    );
    assert!(ok["result"].get("isError").is_none(), "{ok}");
    // The dot's own rule decides: a `Context` deny rule denies the call.
    let denied: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[2]),
        context(json!({"collection": "company"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    let text = denied["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Denied by the policy of dot 'denying'"),
        "{denied}"
    );
    assert!(!text.contains("style.md"), "{denied}");
    let unknown: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[0]),
        context(json!({"collection": "x"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(unknown["result"]["isError"], true);
    assert!(
        unknown["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Collections: company"),
        "{unknown}"
    );
    // A run without collections has no `context` tool.
    let none: Value = rpc(
        &e,
        "/mcp",
        Some(&secrets[1]),
        context(json!({"collection": "company"})),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(none["error"]["code"], -32602, "{none}");
}

#[tokio::test]
async fn dots_own_server_caps_the_text_of_an_error_like_the_local_tool() {
    let e = env(json!([]), false).await;
    let kb = e.dir.path().join("kb");
    write_file(
        &kb.join("style.md"),
        "# House style
",
    );
    let names: Vec<String> = (0..900)
        .map(|i| format!("{}-{i:03}", "c".repeat(34)))
        .collect();
    for name in &names {
        e.lib
            .add(
                name,
                LibraryConfig::Context(ContextConfig {
                    path: kb.to_string_lossy().into_owned(),
                    description: String::new(),
                    every_dot: false,
                }),
                None,
            )
            .await
            .unwrap();
    }
    let dot = claude_dot(&e, |s| s.uses.context = names.clone()).await;
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let (tools, _) = RunTools::open(OpenRequest {
        store: &e.store,
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        scope: &DotScope::Personal,
        workspace_id: "default",
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": {"name": "context", "arguments": {"collection": "nope"}}});
    let reply: Value = rpc(&e, "/mcp", Some(&secret), body)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("unknown collection"), "{text}");
    // The same cap as the local tool: 30 KB, then its "[truncated: n more bytes]" note.
    assert!(text.contains("[truncated: "), "{}", text.len());
    assert!(text.len() <= 30 * 1024 + 64, "{}", text.len());
}

#[tokio::test]
async fn a_context_call_that_asks_waits_for_the_decision_and_parks_when_none_comes() {
    let e = env(json!([]), false).await;
    add_company(&e).await;
    let dot = claude_dot(&e, |s| {
        s.uses.context = vec!["company".into()];
        s.approval_wait_secs = 1;
        // deny > allow > ask: the preset's bare `Context` allow would win over an ask rule.
        s.policy.rules.retain(|r| r.tool != "Context");
        s.policy.rules.push(Rule::new("Context", Action::Ask));
    })
    .await;
    let run = e
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let (tools, _) = RunTools::open(OpenRequest {
        store: &e.store,
        bus: &e.bus,
        run_id: &run.id,
        uses: &dot.spec.uses,
        workspace: e.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
        vault: &Vault::in_memory(e.store.clone()),
        scope: &DotScope::Personal,
        workspace_id: "default",
        grants: &Grants::default(),
        dot_name: "gateway",
    })
    .await
    .unwrap();
    let secret = e.runner.register_secret(&run.id);
    e.runner.register_tools(&run.id, Arc::new(tools));
    let call = |id: u64| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": "context",
                          "arguments": {"collection": "company", "path": "style.md"}}})
    };

    // A live approval reads the collection and returns it.
    let approver = async {
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let pending = e.store.approvals_for_run(&run.id).await.unwrap();
                // The waiter is inserted after the row, so wait for both.
                if let Some(a) = pending.iter().find(|a| a.status == ApprovalStatus::Pending) {
                    if e.hub.waiter_count() > 0 {
                        return a.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the approval row and its waiter never appeared");
        assert_eq!(found.tool, "Context");
        assert_eq!(
            found.input,
            json!({"collection": "company", "path": "style.md"})
        );
        let DecideEffect::Live(_) = e.hub.decide(&found.id, true, None).await.unwrap() else {
            panic!("expected a live decision")
        };
    };
    let (granted, ()) = tokio::join!(rpc(&e, "/mcp", Some(&secret), call(1)), approver);
    let granted: Value = granted.json().await.unwrap();
    assert!(granted["result"].get("isError").is_none(), "{granted}");
    assert_eq!(
        granted["result"]["content"][0]["text"],
        "# House style
Short sentences."
    );

    // With no decision inside the wait, the call is parked and the document stays unread.
    let parked: Value = rpc(&e, "/mcp", Some(&secret), call(2))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(parked["result"]["isError"], true, "{parked}");
    let text = parked["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Queued for human approval #"), "{parked}");
    assert!(!text.contains("Short sentences"), "{parked}");
    let approvals = e.store.approvals_for_run(&run.id).await.unwrap();
    let last = approvals.last().unwrap();
    assert_eq!(last.tool, "Context");
    assert!(last.parked, "{last:?}");
    assert_eq!(last.input["collection"], "company");
    assert_eq!(text, parked_message(&last.id));
}
