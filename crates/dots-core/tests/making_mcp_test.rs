//! The tools for making dots on the Claude engine (dots-make-dots design sections 5 and 7 to
//! 10): Dots' own `dots` MCP server lists `make_dot`, `change_dot`, `delete_dot`, `give_job` and
//! `list_made_dots` for a run of a maker, and for no other run; checks each call against the
//! dot's policy under `MakeDots`; and has the makers' service perform it. `--allowedTools` names
//! them, so the CLI does not ask `approve` first. Driven through `fake-claude`: a script that
//! waits holds a run open while the test calls the server as that run's CLI would.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{init_repo, spec, wait_status};
use dots_core::library::Uses;
use dots_core::making::jobs::gave_text;
use dots_core::making::manage::{made_dots_text, ChangeDot};
use dots_core::making::reach::{Wanted, WantedPermissions};
use dots_core::making::service::MakeDot;
use dots_core::making::tools::{offered, Tool};
use dots_core::model::{
    Dot, DotSpec, EngineKind, Making, MemberSpec, ProjectSpec, Run, RunStatus, TeamSpec,
    WorkflowRun, WorkflowRunStatus, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::policy::{Action, GuardKind, Preset, Rule};
use dots_core::vault::{Grants, MemorySecrets};
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::{Config, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

/// The five tools as the `dots` server lists them, in its order.
const NAMES: [&str; 5] = [
    "make_dot",
    "change_dot",
    "delete_dot",
    "give_job",
    "list_made_dots",
];

/// The five tools as the `claude` CLI names them.
const CLI_NAMES: [&str; 5] = [
    "mcp__dots__make_dot",
    "mcp__dots__change_dot",
    "mcp__dots__delete_dot",
    "mcp__dots__give_job",
    "mcp__dots__list_made_dots",
];

/// What `make_dot` tells the run of a dot made at once, with no schedule and no first job.
const MADE: &str =
    "Made the dot \"Blog Watcher\". It is on and ready; give it a job with give_job.";

struct T {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
}

/// A runtime whose Claude engine is `fake-claude` playing `script`.
async fn start(script: Value) -> T {
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.new_dot_root = dir.path().join("Dots");
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    cfg.max_concurrent_runs = 4;
    cfg.claude_env = vec![
        (
            "FAKE_CLAUDE_SCRIPT".into(),
            script_path.to_string_lossy().to_string(),
        ),
        (
            "FAKE_CLAUDE_ARGS_OUT".into(),
            args_out.to_string_lossy().to_string(),
        ),
    ];
    let rt = Runtime::start(cfg).await.unwrap();
    T { dir, rt, args_out }
}

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

fn success() -> Value {
    json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                    "usage": {"input_tokens": 1, "output_tokens": 1}}})
}

/// A script whose run stays open for a minute, or until the test cancels it: the test calls
/// the `dots` server meanwhile, with the run's own secret.
fn held() -> Value {
    json!([init(), {"sleep_ms": 60_000}, success()])
}

fn mcp_call(tool: &str, arguments: Value) -> Value {
    json!({"mcp_call": {"server": "dots", "tool": tool, "arguments": arguments}})
}

/// A Claude dot named `name` that may make dots, in a folder of its own, under the Sandboxed
/// preset. A call that asks parks at once.
async fn create(t: &T, name: &str, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let folder = t.dir.path().join("work").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let mut s = spec(name);
    s.workdir = folder.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.making.on = true;
    s.approval_wait_secs = 0;
    edit(&mut s);
    t.rt.create_dot(s, None, None).await.unwrap()
}

/// What a started CLI was given: how it reaches the `dots` server as its run, and its
/// arguments.
struct Cli {
    url: String,
    auth: String,
    args: Vec<String>,
}

/// The CLI of the run whose prompt holds `message`, once it has started.
async fn connect(t: &T, message: &str) -> Cli {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let recorded = std::fs::read_to_string(&t.args_out)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .filter(|r| r["prompt"].as_str().is_some_and(|p| p.contains(message)));
        if let Some(recorded) = recorded {
            let entry = &recorded["mcp_config"]["mcpServers"]["dots"];
            return Cli {
                url: entry["url"].as_str().unwrap().to_string(),
                auth: entry["headers"]["Authorization"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                args: serde_json::from_value(recorded["args"].clone()).unwrap(),
            };
        }
        assert!(Instant::now() < deadline, "no CLI started for {message:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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

/// The answer of Dots' own server to `body`, for the run of `cli`.
async fn rpc(cli: &Cli, body: Value) -> Value {
    reqwest::Client::new()
        .post(&cli.url)
        .header("Authorization", &cli.auth)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The tools the `dots` server lists for the run of `cli`.
async fn tools(cli: &Cli) -> Vec<Value> {
    let listed = rpc(
        cli,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    listed["result"]["tools"].as_array().unwrap().clone()
}

fn names(tools: &[Value]) -> Vec<&str> {
    tools.iter().map(|t| t["name"].as_str().unwrap()).collect()
}

/// Calls the `dots` server's tool `name` for the run of `cli`.
async fn call(cli: &Cli, name: &str, arguments: Value) -> Value {
    rpc(
        cli,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": name, "arguments": arguments}}),
    )
    .await
}

/// The text of a tool result that is no error.
fn done(answer: &Value) -> &str {
    assert!(answer["result"].get("isError").is_none(), "{answer}");
    answer["result"]["content"][0]["text"].as_str().unwrap()
}

/// The text of a tool result that is an error.
fn refused(answer: &Value) -> &str {
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    answer["result"]["content"][0]["text"].as_str().unwrap()
}

/// The arguments of a `make_dot` for a dot named `name`, with nothing but what a new dot needs.
fn asks(name: &str) -> Value {
    json!({
        "name": name,
        "instructions": "Read the three blogs and list what is new.",
        "purpose": "Reads the feeds"
    })
}

/// The dot named `name` that `maker` made, if it exists.
async fn made_dot(t: &T, maker: &Dot, name: &str) -> Option<Dot> {
    t.rt.store().made_dot_named(&maker.id, name).await.unwrap()
}

/// The names of all dots, sorted.
async fn dot_names(t: &T) -> Vec<String> {
    let mut names: Vec<String> =
        t.rt.store()
            .list_dots()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.spec.name)
            .collect();
    names.sort();
    names
}

/// Ends the runs the test held open, and the runtime.
async fn finish(t: &T, runs: &[&Run]) {
    for run in runs {
        let _ = t.rt.cancel_run(&run.id).await;
    }
    t.rt.shutdown();
}

/// The resume run of `parent`, once the runner made it.
async fn resume_child(t: &T, parent: &Run) -> Run {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let runs =
            t.rt.store()
                .list_runs(Some(&parent.dot_id), 10)
                .await
                .unwrap();
        if let Some(r) = runs
            .into_iter()
            .find(|r| r.parent_run_id.as_deref() == Some(parent.id.as_str()))
        {
            return r;
        }
        assert!(Instant::now() < deadline, "no resume run appeared");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn keys(object: &Value) -> Vec<String> {
    let mut keys: Vec<String> = object.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn the_schemas_name_the_fields_of_the_asks_one_for_one() {
    let every = Wanted {
        name: Some("Blog Watcher".into()),
        instructions: Some("Read the blogs.".into()),
        model: Some("haiku".into()),
        folder: Some("blogs".into()),
        permissions: Some(WantedPermissions::Preset(Preset::ReadOnly)),
        uses: Some(Uses::default()),
        variables: Some(Grants::default()),
        browser: Some(false),
        use_user_settings: Some(false),
        max_turns: Some(10),
        timeout_secs: Some(600),
        schedule: Some(None),
    };
    let make = MakeDot {
        purpose: "Reads the feeds".into(),
        wanted: every.clone(),
        first_job: Some("Read them now.".into()),
    };
    let change = ChangeDot {
        dot: "Blog Watcher".into(),
        purpose: Some("Reads the feeds".into()),
        wanted: every,
    };
    let make = serde_json::to_value(&make).unwrap();
    let change = serde_json::to_value(&change).unwrap();
    assert_eq!(keys(&Tool::MakeDot.properties()), keys(&make));
    assert_eq!(keys(&Tool::ChangeDot.properties()), keys(&change));
    assert_eq!(keys(&Tool::DeleteDot.properties()), ["dot"]);
    assert_eq!(keys(&Tool::GiveJob.properties()), ["dot", "message"]);
    assert!(keys(&Tool::ListMadeDots.properties()).is_empty());
    for tool in [Tool::MakeDot, Tool::ChangeDot] {
        let properties = tool.properties();
        assert_eq!(
            keys(&properties["uses"]["properties"]),
            keys(&make["uses"]),
            "{tool:?}"
        );
        // The words a model may write, and each is one the ask reads.
        assert_eq!(
            properties["permissions"]["enum"],
            json!(["maker", "read-only", "sandboxed", "trusted"])
        );
        assert_eq!(properties["schedule"]["anyOf"][0]["enum"], json!(["none"]));
        assert_eq!(properties["browser"]["type"], "boolean");
        assert_eq!(properties["max_turns"]["type"], "integer");
        assert_eq!(properties["timeout_secs"]["type"], "integer");
        assert_eq!(properties["folder"]["type"], "string");
    }
    for word in ["maker", "read-only", "sandboxed", "trusted"] {
        let mut ask = asks("Blog Watcher");
        ask["permissions"] = json!(word);
        serde_json::from_value::<MakeDot>(ask).unwrap();
    }
    for schedule in [
        json!("none"),
        json!({"rule": {"kind": "daily", "time": "09:00"}}),
        json!({"rule": {"kind": "weekly", "days": ["mon"], "time": "09:00"}, "until": {"count": 4}}),
    ] {
        let mut ask = asks("Blog Watcher");
        ask["schedule"] = schedule;
        serde_json::from_value::<MakeDot>(ask).unwrap();
    }
    let limits = Making {
        on: true,
        per_run: 2,
        total: 7,
    };
    for (tool, cli_name) in Tool::ALL.into_iter().zip(CLI_NAMES) {
        assert_eq!(tool.model_name(EngineKind::Claude), cli_name);
        assert!(cli_name.len() <= 64);
        assert_eq!(Tool::from_name(tool.name()), Some(tool));
        let schema = tool.mcp_schema(&limits);
        assert_eq!(schema["name"], tool.name());
        assert!(
            schema["description"].as_str().unwrap().len() > 40,
            "{schema}"
        );
        assert_eq!(schema["inputSchema"]["type"], "object");
        assert_eq!(schema["inputSchema"]["additionalProperties"], false);
        assert_eq!(schema["inputSchema"]["properties"], tool.properties());
        // What a call must give is something the tool has, described for the model.
        for required in tool.required() {
            let property = &schema["inputSchema"]["properties"][required];
            assert!(property["description"].is_string(), "{tool:?} {required}");
        }
    }
    // `make_dot` names the maker's own limits.
    let description = Tool::MakeDot.description(&limits);
    assert!(description.contains("2 in one run"), "{description}");
    assert!(description.contains("7 kept"), "{description}");
}

#[tokio::test]
async fn only_a_makers_run_is_offered_the_five_tools() {
    let t = start(held()).await;
    // The same dot with the switch off.
    let plain = create(&t, "Plain", |s| s.making.on = false).await;
    let plain_chat = t.rt.chat(&plain.id, "hello plain", None).await.unwrap();
    let plain_cli = connect(&t, "hello plain").await;
    let plain_tools = tools(&plain_cli).await;
    let plain_allowed = values_of(&plain_cli.args, "--allowedTools");
    for name in NAMES {
        assert!(!names(&plain_tools).contains(&name), "{plain_tools:?}");
    }
    assert!(
        !plain_cli
            .args
            .iter()
            .any(|a| NAMES.iter().any(|name| a.contains(name))),
        "{:?}",
        plain_cli.args
    );

    let maker = create(&t, "Researcher", |_| {}).await;
    let maker_chat = t.rt.chat(&maker.id, "hello maker", None).await.unwrap();
    let maker_cli = connect(&t, "hello maker").await;
    let maker_tools = tools(&maker_cli).await;
    // Everything the other run has, as it has it, and then the five.
    assert_eq!(maker_tools.len(), plain_tools.len() + 5, "{maker_tools:?}");
    assert_eq!(maker_tools[..plain_tools.len()], plain_tools[..]);
    let five = &maker_tools[plain_tools.len()..];
    assert_eq!(names(five), NAMES);
    for (tool, listed) in Tool::ALL.into_iter().zip(five) {
        assert_eq!(listed, &tool.mcp_schema(&maker.spec.making));
        assert!(!listed["inputSchema"]["properties"].is_null());
    }
    assert_eq!(
        five[0]["inputSchema"]["required"],
        json!(["name", "instructions", "purpose"])
    );
    let maker_allowed = values_of(&maker_cli.args, "--allowedTools");
    for name in CLI_NAMES {
        assert!(
            maker_allowed.contains(&name.to_string()),
            "{maker_allowed:?}"
        );
        assert!(
            !plain_allowed.contains(&name.to_string()),
            "{plain_allowed:?}"
        );
    }
    // The policy name is Dots' own: the CLI is never given it.
    assert!(!maker_cli.args.contains(&"MakeDots".to_string()));
    let without: Vec<String> = maker_allowed
        .iter()
        .filter(|a| !CLI_NAMES.contains(&a.as_str()))
        .cloned()
        .collect();
    assert_eq!(without, plain_allowed);

    // A maker whose policy denies `MakeDots` is not offered them either.
    let denied = create(&t, "Denied", |s| {
        s.policy
            .rules
            .insert(0, Rule::new("MakeDots", Action::Deny));
    })
    .await;
    let denied_chat = t.rt.chat(&denied.id, "hello denied", None).await.unwrap();
    let denied_cli = connect(&t, "hello denied").await;
    assert_eq!(tools(&denied_cli).await, plain_tools);
    let denied_allowed = values_of(&denied_cli.args, "--allowedTools");
    for name in CLI_NAMES {
        assert!(!denied_allowed.contains(&name.to_string()));
    }

    // A call by name from a run that was not offered the tool is refused as any tool the
    // server does not have, and makes nothing.
    for cli in [&plain_cli, &denied_cli] {
        let forged = call(cli, "make_dot", asks("Blog Watcher")).await;
        assert_eq!(forged["error"]["code"], -32602, "{forged}");
        assert_eq!(forged["error"]["message"], "unknown tool", "{forged}");
        for name in ["change_dot", "delete_dot", "give_job"] {
            let forged = call(cli, name, json!({"dot": "Researcher", "message": "Go."})).await;
            assert_eq!(forged["error"]["code"], -32602, "{name}: {forged}");
        }
        let forged = call(cli, "list_made_dots", json!({})).await;
        assert_eq!(forged["error"]["code"], -32602, "{forged}");
    }
    assert_eq!(dot_names(&t).await, ["Denied", "Plain", "Researcher"]);
    finish(&t, &[&plain_chat, &maker_chat, &denied_chat]).await;
}

/// A team of `member` with the project "Dots" and a manual workflow "Steps" of one agent step
/// of that member; starts a run of it.
async fn run_step(t: &T, member: &Dot) -> WorkflowRun {
    let team =
        t.rt.teams()
            .create(&TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
                name: "Dots management".into(),
                description: String::new(),
                members: vec![MemberSpec {
                    dot_id: member.id.clone(),
                    role: "researcher".into(),
                }],
            })
            .await
            .unwrap();
    let repo = t.dir.path().join("repo");
    init_repo(&repo);
    t.rt.teams()
        .add_project(
            &team.id,
            &ProjectSpec {
                name: "Dots".into(),
                repo_path: repo.to_string_lossy().to_string(),
                default_branch: None,
                remote: "origin".into(),
            },
        )
        .await
        .unwrap();
    let step = Step {
        id: "research".into(),
        name: "Research it".into(),
        kind: StepKind::Agent {
            member: member.id.clone(),
            job: "Research it.".into(),
            can_send_back_to: Vec::new(),
            outcomes: Vec::new(),
            delegate: None,
        },
    };
    let flow =
        t.rt.teams()
            .create_workflow(
                &team.id,
                &WorkflowSpec {
                    name: "Steps".into(),
                    definition: Definition {
                        trigger: Trigger::Manual,
                        steps: vec![step],
                        budget: Budget::default(),
                    },
                    enabled: true,
                },
            )
            .await
            .unwrap();
    let project =
        t.rt.store()
            .get_team_project_id(&flow.id, "Dots")
            .await
            .unwrap();
    t.rt.workflows()
        .start(dots_core::workflows::StartRequest {
            workflow_id: flow.id,
            project_id: project,
            task: "Watch three blogs.".into(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            test: false,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_workflow_steps_run_of_a_maker_is_not_offered_the_tools() {
    let t = start(json!([
        init(),
        {"mcp_list": "dots"},
        mcp_call("make_dot", asks("Blog Watcher")),
        success()
    ]))
    .await;
    let maker = create(&t, "Researcher", |_| {}).await;
    let started = run_step(&t, &maker).await;
    let deadline = Instant::now() + Duration::from_secs(60);
    let ended = loop {
        let wr = t.rt.workflows().get(&started.id).await.unwrap();
        if wr.status == WorkflowRunStatus::Succeeded {
            break wr;
        }
        assert!(Instant::now() < deadline, "{wr:#?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let run =
        t.rt.store()
            .runs_of_step_run(&ended.steps[0].id)
            .await
            .unwrap()
            .remove(0);
    assert_eq!(run.dot_id, maker.id);

    // The CLI was not given the names, the server did not list the tools, and the call by name
    // was answered as an unknown tool (an error, so no text) and made nothing.
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    for name in CLI_NAMES {
        assert!(!args.contains(&name.to_string()), "{args:?}");
    }
    let events = t.rt.store().list_events(&run.id, 0).await.unwrap();
    let listed = events.iter().find(|e| e.kind == "raw").unwrap();
    let listed: Value = serde_json::from_str(listed.data["line"].as_str().unwrap()).unwrap();
    let listed: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(listed.contains(&"notify"), "{listed:?}");
    for name in NAMES {
        assert!(!listed.contains(&name), "{listed:?}");
    }
    let results: Vec<&str> = events
        .iter()
        .filter(|e| e.kind == "tool_result")
        .map(|e| e.data["output"].as_str().unwrap())
        .collect();
    assert_eq!(results, [""]);
    assert_eq!(dot_names(&t).await, ["Researcher"]);

    // The offer follows what the makers' service would refuse a run for.
    let mut chat = run.clone();
    chat.workflow_step_run_id = None;
    chat.status = RunStatus::Running;
    assert!(offered(&maker, &chat));
    let mut step = chat.clone();
    step.workflow_step_run_id = run.workflow_step_run_id.clone();
    assert!(step.workflow_step_run_id.is_some());
    assert!(!offered(&maker, &step));
    let mut off = maker.clone();
    off.spec.making.on = false;
    assert!(!offered(&off, &chat));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_makers_run_makes_changes_lists_jobs_and_deletes_through_dots_own_server() {
    let t = start(held()).await;
    // Without "Always ask before sending or deleting", so its `delete_dot` does not ask.
    let maker = create(&t, "Researcher", |s| s.policy.ask_send_delete = false).await;
    let chat =
        t.rt.chat(&maker.id, "watch three blogs", None)
            .await
            .unwrap();
    let cli = connect(&t, "watch three blogs").await;

    // Within reach: made at once, and the result is the service's text.
    let made = call(&cli, "make_dot", asks("Blog Watcher")).await;
    assert_eq!(done(&made), MADE);
    let dot = made_dot(&t, &maker, "Blog Watcher").await.expect("made");
    let by = dot.made.clone().unwrap();
    assert_eq!(by.by_dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(by.run_id, chat.id);
    assert_eq!(by.purpose, "Reads the feeds");

    // Beyond reach: nothing is made, the request waits, and the run is told to carry on.
    let mut more = asks("Browser Dot");
    more["browser"] = json!(true);
    let waiting = call(&cli, "make_dot", more).await;
    let told = done(&waiting);
    assert!(
        told.starts_with("Not made yet: \"Browser Dot\" needs the person's say-so"),
        "{told}"
    );
    assert!(told.contains("The browser"), "{told}");
    assert!(
        told.contains("you will be woken with their answer"),
        "{told}"
    );
    assert!(made_dot(&t, &maker, "Browser Dot").await.is_none());
    let requests =
        t.rt.store()
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].name, "Browser Dot");
    assert_eq!(requests[0].run_id, chat.id);

    // An argument the tool does not have is refused in a sentence, and nothing is made.
    let mut sneaky = asks("Sneaky");
    sneaky["can_make_dots"] = json!(true);
    let told = call(&cli, "make_dot", sneaky).await;
    let told = refused(&told);
    assert!(
        told.starts_with("make_dot cannot read its arguments: unknown field `can_make_dots`"),
        "{told}"
    );
    assert!(
        told.ends_with("Send only the arguments the tool lists."),
        "{told}"
    );
    let told = call(
        &cli,
        "change_dot",
        json!({"dot": "Blog Watcher", "engine": "local"}),
    )
    .await;
    let told = refused(&told);
    assert!(
        told.starts_with("change_dot cannot read its arguments: unknown field `engine`"),
        "{told}"
    );
    let told = call(&cli, "make_dot", json!({"name": "No Purpose"})).await;
    assert!(refused(&told).contains("missing field `purpose`"), "{told}");
    assert!(made_dot(&t, &maker, "Sneaky").await.is_none());
    // No call so far asked the person anything.
    assert!(t
        .rt
        .store()
        .approvals_for_run(&chat.id)
        .await
        .unwrap()
        .is_empty());

    // `list_made_dots` is the service's list in the service's words.
    let listed = call(&cli, "list_made_dots", json!({})).await;
    let lines = t.rt.makers().list_made_dots(&chat.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(
        done(&listed),
        made_dots_text("Researcher", &lines, &requests)
    );
    assert!(done(&listed).contains("Blog Watcher"), "{listed}");
    assert!(done(&listed).contains("Browser Dot"), "{listed}");

    // `change_dot` changes what it names.
    let changed = call(
        &cli,
        "change_dot",
        json!({"dot": "Blog Watcher", "purpose": "Reads four feeds", "max_turns": 10}),
    )
    .await;
    assert!(
        done(&changed).starts_with("Changed \"Blog Watcher\": "),
        "{changed}"
    );
    let dot = t.rt.store().get_dot(&dot.id).await.unwrap();
    assert_eq!(dot.spec.max_turns, 10);
    assert_eq!(dot.made.clone().unwrap().purpose, "Reads four feeds");

    // A second dot, deleted again.
    assert!(done(&call(&cli, "make_dot", asks("Second")).await).contains("\"Second\""));
    let deleted = call(&cli, "delete_dot", json!({"dot": "Second"})).await;
    assert!(done(&deleted).contains("Second"), "{deleted}");
    assert!(made_dot(&t, &maker, "Second").await.is_none());

    // `give_job` returns at once, and the made dot has a run.
    let gave = call(
        &cli,
        "give_job",
        json!({"dot": "Blog Watcher", "message": "Read the feeds now."}),
    )
    .await;
    assert_eq!(done(&gave), gave_text("Blog Watcher"));
    let jobs = t.rt.store().list_runs(Some(&dot.id), 10).await.unwrap();
    assert_eq!(jobs.len(), 1);

    // What the service refuses is a tool error with the service's sentence: a dot with a run
    // going is not deleted, and a dot the maker did not make is not its to delete.
    let busy = call(&cli, "delete_dot", json!({"dot": "Blog Watcher"})).await;
    assert!(refused(&busy).contains("Blog Watcher"), "{busy}");
    assert!(made_dot(&t, &maker, "Blog Watcher").await.is_some());
    let not_made = call(&cli, "delete_dot", json!({"dot": "Researcher"})).await;
    assert!(!refused(&not_made).is_empty());
    assert!(!refused(&not_made).contains("invalid input"), "{not_made}");

    // A deny rule the person adds while the run goes on refuses the next call, as it refuses
    // any tool.
    let mut narrowed = t.rt.store().get_dot(&maker.id).await.unwrap().spec;
    narrowed
        .policy
        .rules
        .insert(0, Rule::new("MakeDots", Action::Deny));
    t.rt.update_dot(&maker.id, narrowed).await.unwrap();
    let denied = call(&cli, "list_made_dots", json!({})).await;
    assert!(
        refused(&denied).contains("Denied by the policy of dot 'Researcher'"),
        "{denied}"
    );
    let denied = call(&cli, "make_dot", asks("Third")).await;
    assert!(
        refused(&denied).contains("Denied by the policy of dot 'Researcher'"),
        "{denied}"
    );
    assert!(made_dot(&t, &maker, "Third").await.is_none());
    finish(&t, &[&chat, &jobs[0]]).await;
}

#[tokio::test]
async fn a_rule_that_asks_parks_the_call_and_the_approved_dot_is_made() {
    let t = start(json!([
        init(),
        mcp_call("make_dot", asks("Blog Watcher")),
        success()
    ]))
    .await;
    // The ask rule wins over the preset's allow, as for Dots' other own tools.
    let maker = create(&t, "Researcher", |s| {
        s.policy.rules.push(Rule::new("MakeDots", Action::Ask));
    })
    .await;
    let chat =
        t.rt.chat(&maker.id, "watch three blogs", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::AwaitingApproval, 30).await;
    let approval =
        t.rt.store()
            .approvals_for_run(&chat.id)
            .await
            .unwrap()
            .remove(0);
    // The card names the policy name, and its input the tool and what it was called with.
    assert_eq!(approval.tool, "MakeDots");
    assert_eq!(
        approval.input,
        json!({"tool": "make_dot", "input": asks("Blog Watcher")})
    );
    assert!(made_dot(&t, &maker, "Blog Watcher").await.is_none());

    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    let child = resume_child(&t, &chat).await;
    let message = child.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        message.contains("you may now use mcp__dots__make_dot with the same input"),
        "{message}"
    );
    // The resumed run calls again, and the grant lets the call through.
    wait_status(t.rt.store(), &child.id, RunStatus::Succeeded, 30).await;
    let dot = made_dot(&t, &maker, "Blog Watcher").await.expect("made");
    assert_eq!(dot.made.unwrap().run_id, child.id);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_makers_run_on_the_claude_engine_makes_a_dot() {
    let t = start(json!([
        init(),
        {"mcp_list": "dots"},
        mcp_call("make_dot", asks("Blog Watcher")),
        success()
    ]))
    .await;
    let maker = create(&t, "Researcher", |_| {}).await;
    let chat =
        t.rt.chat(&maker.id, "watch three blogs", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;

    let events = t.rt.store().list_events(&chat.id, 0).await.unwrap();
    let of = |kind: &str| -> Vec<Value> {
        events
            .iter()
            .filter(|ev| ev.kind == kind)
            .map(|ev| ev.data.clone())
            .collect()
    };
    let listed: Value = serde_json::from_str(of("raw")[0]["line"].as_str().unwrap()).unwrap();
    let listed: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    assert_eq!(listed[listed.len() - 5..], NAMES);
    assert_eq!(of("tool_call")[0]["tool"], "mcp__dots__make_dot");
    let result = &of("tool_result")[0];
    assert_eq!(result["output"], MADE);
    assert_eq!(result["is_error"], false);

    let dot = made_dot(&t, &maker, "Blog Watcher").await.expect("made");
    let by = dot.made.clone().unwrap();
    assert_eq!(by.by_dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(by.by_name, "Researcher");
    assert_eq!(by.run_id, chat.id);
    assert!(dot.spec.enabled);
    assert!(!dot.spec.making.on);
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "Researcher"]);
    t.rt.shutdown();
}

#[tokio::test]
async fn deleting_a_made_dot_asks_first_while_the_maker_always_asks_before_deleting() {
    let t = start(held()).await;
    // Every preset starts with "Always ask before sending or deleting" on.
    let maker = create(&t, "Researcher", |_| {}).await;
    assert!(maker.spec.policy.ask_send_delete);
    let chat =
        t.rt.chat(&maker.id, "watch three blogs", None)
            .await
            .unwrap();
    let cli = connect(&t, "watch three blogs").await;
    // Making, changing and listing do not ask.
    assert_eq!(
        done(&call(&cli, "make_dot", asks("Blog Watcher")).await),
        MADE
    );
    done(
        &call(
            &cli,
            "change_dot",
            json!({"dot": "Blog Watcher", "max_turns": 10}),
        )
        .await,
    );
    done(&call(&cli, "list_made_dots", json!({})).await);
    assert!(t
        .rt
        .store()
        .approvals_for_run(&chat.id)
        .await
        .unwrap()
        .is_empty());

    // Deleting asks: the call parks on an approval, and the dot stays.
    let parked = call(&cli, "delete_dot", json!({"dot": "Blog Watcher"})).await;
    refused(&parked);
    let approvals = t.rt.store().approvals_for_run(&chat.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "MakeDots");
    assert_eq!(
        approvals[0].input,
        json!({"tool": "delete_dot", "input": {"dot": "Blog Watcher"}})
    );
    let guard =
        t.rt.approval_guard(&approvals[0].id)
            .await
            .unwrap()
            .expect("a guarded call");
    assert_eq!(guard.kind, GuardKind::Delete);
    assert_eq!(
        guard.summary,
        "delete the dot \"Blog Watcher\" and its history"
    );
    assert!(guard.always_asks);
    assert!(made_dot(&t, &maker, "Blog Watcher").await.is_some());
    finish(&t, &[&chat]).await;
}
