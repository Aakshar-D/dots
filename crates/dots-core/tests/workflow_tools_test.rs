//! `start_workflow` (teams spec section 7): every preset allows `Workflow`, a rule names it
//! alone, migration 0013 gives it to stored policies; a chat run of a member that a chat
//! trigger names lists the workflows it may start and starts one, on both engines.

mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{init_repo, spec, wait_status};
use dots_core::model::{
    Dot, DotSpec, EngineKind, MemberSpec, ProjectSpec, RunStatus, TeamSpec, WaitingOn,
    WorkflowRunStatus, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule, WORKFLOW_RULE, WORKFLOW_TOOL};
use dots_core::store::Store;
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::{Config, Runtime};
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tempfile::TempDir;

#[test]
fn every_preset_allows_workflow_last_and_the_cli_never_sees_it() {
    assert_eq!(WORKFLOW_TOOL, "Workflow");
    for preset in Preset::ALL {
        let p = Policy::preset(*preset);
        let tail: Vec<&str> = p.rules[p.rules.len() - 2..]
            .iter()
            .map(|r| r.tool.as_str())
            .collect();
        assert_eq!(tail, ["Memory", "Workflow"], "{preset:?}");
        assert_eq!(
            p.resolve("Workflow", &json!({})),
            Action::Allow,
            "{preset:?}"
        );
        let (allow, deny) = p.cli_lists();
        assert!(
            !allow.iter().chain(&deny).any(|t| t == "Workflow"),
            "{preset:?}"
        );
    }
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.push(Rule::new("Workflow", Action::Ask));
    assert_eq!(p.resolve("Workflow", &json!({})), Action::Ask);
    p.rules
        .push(Rule::new("Workflow(start_workflow)", Action::Deny));
    let e = p.validate().unwrap_err().to_string();
    assert!(e.contains(WORKFLOW_RULE), "{e}");
}

/// Migration 0013 on a database Dots left at 0012: every stored policy whose preset is not
/// Custom gets a `Workflow` allow rule unless a rule names it, after `Memory`.
#[tokio::test]
async fn migration_0013_gives_every_policy_that_is_not_custom_its_workflow_rule() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 13)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| r.tool != "Workflow");
        p
    };
    let mut denies = old(Preset::Sandboxed);
    denies.rules.push(Rule::new("Workflow", Action::Deny));
    let custom = old(Preset::Custom);
    for (id, name, policy) in [
        ("D1", "sandboxed", old(Preset::Sandboxed)),
        ("D2", "trusted", old(Preset::Trusted)),
        ("D3", "denies", denies.clone()),
        ("D4", "custom", custom.clone()),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(serde_json::to_string(&policy).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        store.get_dot("D1").await.unwrap().spec.policy,
        Policy::preset(Preset::Sandboxed)
    );
    assert_eq!(
        store.get_dot("D2").await.unwrap().spec.policy,
        Policy::preset(Preset::Trusted)
    );
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, denies);
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom);
    store.close().await;
}

struct Rt {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
}

/// A runtime whose Claude engine is `fake-claude` playing `script`.
async fn start(script: Value) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
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
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt, args_out }
}

/// A team with `helper`, a project, and a workflow "Fix" whose chat trigger names the helper
/// and whose one step is the person's check.
async fn team_of(t: &Rt, helper: &Dot) -> String {
    let team =
        t.rt.teams()
            .create(&TeamSpec {
                name: "Dots management".into(),
                description: String::new(),
                members: vec![MemberSpec {
                    dot_id: helper.id.clone(),
                    role: "user helper".into(),
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
    t.rt.teams()
        .create_workflow(
            &team.id,
            &WorkflowSpec {
                name: "Fix".into(),
                definition: Definition {
                    trigger: Trigger::Chat {
                        starters: vec![helper.id.clone()],
                    },
                    steps: vec![Step {
                        id: "check".into(),
                        name: "Your check".into(),
                        kind: StepKind::Check {},
                    }],
                    budget: Budget::default(),
                },
                enabled: true,
            },
        )
        .await
        .unwrap();
    team.id
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
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

fn prompt(req: &Value) -> String {
    messages(req)
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_local_helper_starts_the_teams_workflow_from_a_chat() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        call(
            "c1",
            "start_workflow",
            json!({"workflow": "Deploy", "project": "Dots", "task": "x"}),
        ),
        call(
            "c2",
            "start_workflow",
            json!({"workflow": "Fix", "project": "Other", "task": "x"}),
        ),
        call(
            "c3",
            "start_workflow",
            json!({"workflow": "fix", "project": "dots", "task": " "}),
        ),
        call(
            "c4",
            "start_workflow",
            json!({"workflow": "Dots management/Fix", "project": "Dots",
                   "task": "The tray icon is invisible in dark mode."}),
        ),
        text("Started it."),
        text("A plain run."),
    ])
    .await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"))
            .await
            .unwrap();
    team_of(&t, &helper).await;
    let chat =
        t.rt.chat(&helper.id, "Please fix the tray icon.", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(tool_names(&reqs[0]).contains(&"start_workflow".to_string()));
    let first = prompt(&reqs[0]);
    assert!(
        first.contains(
            "## Workflows you can start\n\n- \"Fix\" of the team \"Dots management\": projects \"Dots\"\n\nStart one with `start_workflow` when the person asks for work it covers.\n"
        ),
        "{first}"
    );
    let answers: Vec<String> = (1..=4)
        .map(|i| tool_result(&reqs[i], &format!("c{i}")))
        .collect();
    assert_eq!(
        answers,
        [
            "There is no workflow named \"Deploy\" that you may start.",
            "The team \"Dots management\" has no project named \"Other\".",
            "Not started: the task must be 1 to 8,000 characters.",
            "Workflow \"Fix\" started on \"Dots\".",
        ]
    );
    let runs = t.rt.workflows().list(None, 10).await.unwrap();
    assert_eq!(runs.len(), 1);
    let wr = &runs[0];
    assert_eq!(wr.trigger, WorkflowTrigger::Chat);
    assert_eq!(wr.origin_run_id.as_deref(), Some(chat.id.as_str()));
    assert_eq!(wr.task, "The tray icon is invisible in dark mode.");
    // Its one step is the check, so it waits for the person.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let wr = t.rt.workflows().get(&wr.id).await.unwrap();
        if wr.status == WorkflowRunStatus::Waiting {
            assert_eq!(wr.waiting_on, Some(WaitingOn::Check));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{wr:#?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // A run that is not a chat has no such tool.
    let plain = t.rt.run_now(&helper.id).await.unwrap();
    wait_status(t.rt.store(), &plain.id, RunStatus::Succeeded, 30).await;
    let last = llm.requests().last().unwrap().clone();
    assert!(!tool_names(&last).contains(&"start_workflow".to_string()));
    assert!(!prompt(&last).contains("## Workflows you can start"));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_helper_starts_it_through_dots_own_server() {
    let t = start(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "start_workflow",
                      "arguments": {"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]))
    .await;
    let mut s = spec("helper");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let helper = t.rt.create_dot(s).await.unwrap();
    team_of(&t, &helper).await;
    let chat =
        t.rt.chat(&helper.id, "Please fix the icon.", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        args.contains(&"mcp__dots__start_workflow".to_string()),
        "{args:?}"
    );
    let runs = t.rt.workflows().list(None, 10).await.unwrap();
    assert_eq!(runs.len(), 1, "{runs:#?}");
    assert_eq!(runs[0].origin_run_id.as_deref(), Some(chat.id.as_str()));
    assert_eq!(runs[0].task, "Fix the icon.");
    t.rt.shutdown();
}
