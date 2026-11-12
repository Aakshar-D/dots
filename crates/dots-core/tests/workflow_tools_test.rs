//! `start_workflow` (teams spec section 7): every preset allows `Workflow`, a rule names it
//! alone, migration 0015 gives it to stored policies; a chat run of a member that a chat
//! trigger names lists the workflows it may start and starts one, on both engines. A run of an
//! open agent step reports its outcome and asks for a send-back, and a delegate step's run
//! starts a member, on both engines.

mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{init_repo, spec, wait_status};
use dots_core::model::{
    Dot, DotSpec, EngineKind, MemberSpec, NewRun, ProjectSpec, RunStatus, TeamSpec, TriggerKind,
    WaitingOn, WorkflowRun, WorkflowRunStatus, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::policy::{Action, Policy, Preset, Rule, APP_TOOLS, WORKFLOW_RULE, WORKFLOW_TOOL};
use dots_core::store::Store;
use dots_core::team_tools::{self, StepTools};
use dots_core::workflow::{
    BranchOn, BranchPath, Budget, Definition, Delegate, Step, StepKind, Trigger,
};
use dots_core::workflows::STEP_CLOSED;
use dots_core::{Config, Runtime};
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tempfile::TempDir;

#[test]
fn workflow_follows_memory_asked_by_sandboxed_and_custom_and_the_cli_never_sees_it() {
    assert_eq!(WORKFLOW_TOOL, "Workflow");
    for preset in Preset::ALL {
        let p = Policy::preset(*preset);
        // The app rules come after it (app library spec section 6).
        let tail: Vec<&str> = p.rules[p.rules.len() - 6..]
            .iter()
            .map(|r| r.tool.as_str())
            .collect();
        assert_eq!(
            tail,
            [
                "Memory",
                "Workflow",
                "App",
                "ProposeApp",
                "Instructions",
                "MakeDots"
            ],
            "{preset:?}"
        );
        let want = match preset {
            Preset::Sandboxed | Preset::Custom => Action::Ask,
            Preset::ReadOnly | Preset::Trusted => Action::Allow,
        };
        assert_eq!(p.resolve("Workflow", &json!({})), want, "{preset:?}");
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

/// The `Workflow` input of each workflow tool, as `team_tools::Tool::for_input` reads it.
fn workflow_calls() -> [(&'static str, Value); 4] {
    [
        (
            "start_workflow",
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix it."}),
        ),
        ("start_member", json!({"member": "dev", "task": "Fix it."})),
        ("report_outcome", json!({"outcome": "fixed"})),
        (
            "send_back",
            json!({"to_step": "Build", "note": "Use the light icon."}),
        ),
    ]
}

/// `report_outcome` and `send_back` never ask (the person's decision, 2026-11-04): under
/// Sandboxed and Custom, under the person's own `Workflow: ask` and under a default of ask,
/// they are allowed, while `start_workflow` and `start_member` still ask. A `Workflow: deny` or
/// a default of deny refuses all four.
#[test]
fn report_outcome_and_send_back_never_ask_and_a_deny_refuses_every_workflow_tool() {
    let asking = |name: &str| matches!(name, "start_workflow" | "start_member");
    for preset in [Preset::Sandboxed, Preset::Custom] {
        let p = Policy::preset(preset);
        for (name, input) in workflow_calls() {
            let want = if asking(name) {
                Action::Ask
            } else {
                Action::Allow
            };
            assert_eq!(p.resolve(WORKFLOW_TOOL, &input), want, "{preset:?} {name}");
        }
    }
    let mut own = Policy::preset(Preset::Trusted);
    own.rules.push(Rule::new("Workflow", Action::Ask));
    let mut no_rule = Policy::preset(Preset::Custom);
    no_rule.rules.retain(|r| r.tool != "Workflow");
    assert_eq!(no_rule.default, Action::Ask);
    for p in [&own, &no_rule] {
        for (name, input) in workflow_calls() {
            let want = if asking(name) {
                Action::Ask
            } else {
                Action::Allow
            };
            assert_eq!(p.resolve(WORKFLOW_TOOL, &input), want, "{name}");
        }
    }
    let mut denies = Policy::preset(Preset::Sandboxed);
    denies.rules.push(Rule::new("Workflow", Action::Deny));
    let mut by_default = no_rule.clone();
    by_default.default = Action::Deny;
    for p in [&denies, &by_default] {
        for (name, input) in workflow_calls() {
            assert_eq!(p.resolve(WORKFLOW_TOOL, &input), Action::Deny, "{name}");
        }
    }
    // Only `Workflow`: another tool's input with an outcome still asks as before.
    let sandboxed = Policy::preset(Preset::Sandboxed);
    assert_eq!(
        sandboxed.resolve("WebFetch", &json!({"outcome": "fixed"})),
        Action::Ask
    );
}

/// Migration 0015 on a database Dots left at 0014: every stored policy whose preset is not
/// Custom gets the `Workflow` rule its preset has (Sandboxed asks, the others allow) unless a
/// rule names it, after `Memory`; 0016 then adds `App` and `ProposeApp` (app library spec
/// section 6), a Custom policy too.
#[tokio::test]
async fn migration_0015_gives_every_policy_that_is_not_custom_its_workflow_rule() {
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
        .filter(|m| m.version < 15)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0014 left it: without `Workflow`, and without the app rules 0016 adds.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| {
            r.tool != "Workflow"
                && !APP_TOOLS.contains(&r.tool.as_str())
                && r.tool != "Instructions"
                && r.tool != "MakeDots"
        });
        p
    };
    let with_apps = |mut p: Policy| {
        p.rules
            .extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        p.rules.push(Rule::new("Instructions", Action::Allow));
        p.rules.push(Rule::new("MakeDots", Action::Allow));
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
    let d1 = store.get_dot("D1").await.unwrap().spec.policy;
    assert_eq!(d1, Policy::preset(Preset::Sandboxed));
    let workflow = d1.rules.iter().position(|r| r.tool == "Workflow").unwrap();
    assert_eq!(d1.rules[workflow], Rule::new("Workflow", Action::Ask));
    assert_eq!(d1.rules[workflow - 1].tool, "Memory");
    assert_eq!(
        store.get_dot("D2").await.unwrap().spec.policy,
        Policy::preset(Preset::Trusted)
    );
    assert_eq!(
        store.get_dot("D3").await.unwrap().spec.policy,
        with_apps(denies)
    );
    assert_eq!(
        store.get_dot("D4").await.unwrap().spec.policy,
        with_apps(custom)
    );
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
    let check = Step {
        id: "check".into(),
        name: "Your check".into(),
        kind: StepKind::Check { send_back_to: None },
    };
    team_with(t, helper, vec![check]).await.0
}

/// `team_of` with the workflow's own steps; gives the team's and the workflow's ids.
async fn team_with(t: &Rt, helper: &Dot, steps: Vec<Step>) -> (String, String) {
    let team =
        t.rt.teams()
            .create(&TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
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
    let flow =
        t.rt.teams()
            .create_workflow(
                &team.id,
                &WorkflowSpec {
                    name: "Fix".into(),
                    definition: Definition {
                        trigger: Trigger::Chat {
                            starters: vec![helper.id.clone()],
                        },
                        steps,
                        budget: Budget::default(),
                    },
                    enabled: true,
                },
            )
            .await
            .unwrap();
    (team.id, flow.id)
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
    // Trusted allows `start_workflow`; the Sandboxed preset asks (see the test of that).
    s.policy = Policy::preset(Preset::Trusted);
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
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
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
    let runs = t.rt.workflows().list(&Default::default()).await.unwrap();
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
    s.policy = Policy::preset(Preset::Trusted);
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
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
    let runs = t.rt.workflows().list(&Default::default()).await.unwrap();
    assert_eq!(runs.len(), 1, "{runs:#?}");
    assert_eq!(runs[0].origin_run_id.as_deref(), Some(chat.id.as_str()));
    assert_eq!(runs[0].task, "Fix the icon.");
    t.rt.shutdown();
}

/// A chat run that carries Dots' report to a maker (a wake) or a maker's job for a dot it made
/// (dots-make-dots design section 8) holds words a model wrote, not the person's: it may not
/// start workflows, though a chat trigger names its dot.
#[tokio::test]
async fn a_wake_or_a_job_run_may_not_start_workflows() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![text("Woken."), text("Job done.")]).await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    team_of(&t, &helper).await;
    let payloads = [
        json!({"message": "\"Fixer\" finished the job.", "wake_id": "W1"}),
        json!({"message": "Fix the icon.", "job_id": "J1", "job_from": "Researcher"}),
    ];
    for (n, payload) in payloads.into_iter().enumerate() {
        let mut new = NewRun::new(&helper.id, TriggerKind::Chat);
        new.payload = Some(payload);
        let run = t.rt.store().create_run(&new).await.unwrap();
        wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
        let reqs = llm.requests();
        assert_eq!(reqs.len(), n + 1);
        let last = &reqs[n];
        assert!(
            !tool_names(last).contains(&"start_workflow".to_string()),
            "{n}"
        );
        assert!(
            !prompt(last).contains("## Workflows you can start"),
            "{}",
            prompt(last)
        );
    }
    t.rt.shutdown();
}

fn workflow_spec(name: &str, trigger: Trigger) -> WorkflowSpec {
    WorkflowSpec {
        name: name.into(),
        definition: Definition {
            trigger,
            steps: vec![Step {
                id: "check".into(),
                name: "Your check".into(),
                kind: StepKind::Check { send_back_to: None },
            }],
            budget: Budget::default(),
        },
        enabled: true,
    }
}

#[tokio::test]
async fn the_answer_to_a_workflow_steps_question_may_not_start_a_workflow() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        call("c1", "ask", json!({"question": "Which icon set?"})),
        text("Asked."),
        text("Thanks."),
    ])
    .await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    let (_, flow) = team_with(
        &t,
        &helper,
        vec![Step {
            id: "dev".into(),
            name: "Fix it".into(),
            kind: StepKind::Agent {
                member: helper.id.clone(),
                job: "Fix.".into(),
                can_send_back_to: vec![],
                outcomes: vec![],
                delegate: None,
            },
        }],
    )
    .await;
    let project =
        t.rt.store()
            .get_team_project_id(&flow, "Dots")
            .await
            .unwrap();
    t.rt.workflows()
        .start(dots_core::workflows::StartRequest {
            workflow_id: flow,
            project_id: project,
            task: "Fix the icon.".into(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            test: false,
        })
        .await
        .unwrap();
    let question = loop {
        let open =
            t.rt.store()
                .list_messages(Some(&helper.id))
                .await
                .unwrap()
                .into_iter()
                .find(|m| m.answered_at.is_none());
        if let Some(q) = open {
            break q;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    // The step's own run has no such tool, though the helper is a starter.
    let first = llm.wait_requests(1, 30).await;
    assert!(!tool_names(&first[0]).contains(&"start_workflow".to_string()));
    wait_status(
        t.rt.store(),
        question.run_id.as_deref().unwrap(),
        RunStatus::Succeeded,
        30,
    )
    .await;
    let answer = t.rt.reply(&question.id, "The light one.").await.unwrap();
    assert!(answer.workflow_step_run_id.is_some());
    wait_status(t.rt.store(), &answer.id, RunStatus::Succeeded, 30).await;
    let last = llm.requests().last().unwrap().clone();
    assert!(!tool_names(&last).contains(&"start_workflow".to_string()));
    assert!(!prompt(&last).contains("## Workflows you can start"));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_call_held_for_approval_is_refused_once_the_member_stops_being_a_starter() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        call(
            "c1",
            "start_workflow",
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}),
        ),
        text("Done."),
    ])
    .await;
    let mut s = local_spec(&t, &llm.endpoint(), "helper");
    s.policy.rules.push(Rule::new("Workflow", Action::Ask));
    s.approval_wait_secs = 30;
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
    let team = team_of(&t, &helper).await;
    let mut events = t.rt.subscribe();
    let chat =
        t.rt.chat(&helper.id, "Please fix the icon.", None)
            .await
            .unwrap();
    let approval = loop {
        if let dots_core::events::RuntimeEvent::ApprovalRequested { approval } =
            events.recv().await.unwrap()
        {
            break approval;
        }
    };
    assert_eq!(approval.tool, "Workflow");
    // While it waits, the workflow stops naming the helper.
    let flow = t.rt.teams().list_workflows(&team).await.unwrap().remove(0);
    t.rt.teams()
        .update_workflow(&flow.id, &workflow_spec("Fix", Trigger::Manual))
        .await
        .unwrap();
    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    assert_eq!(
        tool_result(&llm.requests()[1], "c1"),
        "There is no workflow named \"Fix\" that you may start."
    );
    assert!(t
        .rt
        .workflows()
        .list(&Default::default())
        .await
        .unwrap()
        .is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn an_approval_that_outlives_the_wait_still_starts_the_workflow_on_the_resume_run() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        call(
            "c1",
            "start_workflow",
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}),
        ),
        text("Queued."),
        text("Done after approval."),
    ])
    .await;
    let mut s = local_spec(&t, &llm.endpoint(), "helper");
    s.policy.rules.push(Rule::new("Workflow", Action::Ask));
    s.approval_wait_secs = 0;
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
    team_of(&t, &helper).await;
    let chat =
        t.rt.chat(&helper.id, "Please fix the icon.", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::AwaitingApproval, 30).await;
    let approval =
        t.rt.store()
            .list_pending_approvals()
            .await
            .unwrap()
            .remove(0);
    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    let child = loop {
        let runs = t.rt.store().list_runs(Some(&helper.id), 10).await.unwrap();
        if let Some(c) = runs
            .into_iter()
            .find(|r| r.trigger == dots_core::model::TriggerKind::Resume)
        {
            break c;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    wait_status(t.rt.store(), &child.id, RunStatus::Succeeded, 30).await;
    let runs = t.rt.workflows().list(&Default::default()).await.unwrap();
    assert_eq!(runs.len(), 1, "{runs:#?}");
    assert_eq!(runs[0].task, "Fix the icon.");
    t.rt.shutdown();
}

#[tokio::test]
async fn a_reply_in_the_chat_of_a_step_that_has_ended_is_refused() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![text("Fixed.")]).await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    let (_, flow) = team_with(
        &t,
        &helper,
        vec![Step {
            id: "dev".into(),
            name: "Fix it".into(),
            kind: StepKind::Agent {
                member: helper.id.clone(),
                job: "Fix.".into(),
                can_send_back_to: vec![],
                outcomes: vec![],
                delegate: None,
            },
        }],
    )
    .await;
    let project =
        t.rt.store()
            .get_team_project_id(&flow, "Dots")
            .await
            .unwrap();
    let wr =
        t.rt.workflows()
            .start(dots_core::workflows::StartRequest {
                workflow_id: flow,
                project_id: project,
                task: "Fix the icon.".into(),
                trigger: WorkflowTrigger::Manual,
                origin_run_id: None,
                test: false,
            })
            .await
            .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let wr = loop {
        let wr = t.rt.workflows().get(&wr.id).await.unwrap();
        if wr.status == WorkflowRunStatus::Succeeded {
            break wr;
        }
        assert!(std::time::Instant::now() < deadline, "{wr:#?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let run =
        t.rt.store()
            .runs_of_step_run(&wr.steps[0].id)
            .await
            .unwrap()
            .remove(0);
    let refused =
        t.rt.chat(&helper.id, "And the tooltip?", Some(&run.id))
            .await;
    assert!(
        matches!(&refused, Err(dots_core::Error::Conflict(m)) if m == dots_core::workflows::ENDED_STEP_THREAD),
        "{refused:?}"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_chat_starts_a_workflow_on_a_project_once_until_that_run_ends() {
    let t = start(json!([])).await;
    let start_call = |id: &str| {
        call(
            id,
            "start_workflow",
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}),
        )
    };
    let llm = Llm::start(vec![
        start_call("c1"),
        start_call("c2"),
        text("Started once."),
        start_call("c3"),
        text("Started again."),
    ])
    .await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    team_of(&t, &helper).await;
    let chat =
        t.rt.chat(&helper.id, "Please fix the icon.", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert_eq!(
        tool_result(&reqs[1], "c1"),
        "Workflow \"Fix\" started on \"Dots\"."
    );
    assert_eq!(
        tool_result(&reqs[2], "c2"),
        "Not started: \"Fix\" is already running on \"Dots\" from this chat."
    );
    let runs = t.rt.workflows().list(&Default::default()).await.unwrap();
    assert_eq!(runs.len(), 1, "{runs:#?}");
    // Once that run has ended, the same chat starts it again.
    t.rt.workflows().stop(&runs[0].id).await.unwrap();
    let reply =
        t.rt.chat(&helper.id, "Again, please.", Some(&chat.id))
            .await
            .unwrap();
    wait_status(t.rt.store(), &reply.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert_eq!(
        tool_result(reqs.last().unwrap(), "c3"),
        "Workflow \"Fix\" started on \"Dots\"."
    );
    assert_eq!(
        t.rt.workflows()
            .list(&Default::default())
            .await
            .unwrap()
            .len(),
        2
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_chat_of_a_dot_whose_policy_denies_workflow_is_not_offered_the_tool() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![text("Hello.")]).await;
    let mut s = local_spec(&t, &llm.endpoint(), "helper");
    s.policy.rules.push(Rule::new("Workflow", Action::Deny));
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
    team_of(&t, &helper).await;
    let chat = t.rt.chat(&helper.id, "Hi.", None).await.unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let first = llm.requests()[0].clone();
    assert!(!tool_names(&first).contains(&"start_workflow".to_string()));
    assert!(!prompt(&first).contains("## Workflows you can start"));
    t.rt.shutdown();
}

/// The plain Sandboxed preset asks before a chat starts a workflow, and still offers the tool.
#[tokio::test]
async fn a_sandboxed_dot_is_offered_start_workflow_and_asks_before_it_starts_one() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        call(
            "c1",
            "start_workflow",
            json!({"workflow": "Fix", "project": "Dots", "task": "Fix the icon."}),
        ),
        text("Done."),
    ])
    .await;
    let mut s = local_spec(&t, &llm.endpoint(), "helper");
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 30;
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
    team_of(&t, &helper).await;
    let mut events = t.rt.subscribe();
    let chat =
        t.rt.chat(&helper.id, "Please fix the icon.", None)
            .await
            .unwrap();
    let approval = loop {
        if let dots_core::events::RuntimeEvent::ApprovalRequested { approval } =
            events.recv().await.unwrap()
        {
            break approval;
        }
    };
    assert_eq!(approval.tool, "Workflow");
    assert!(tool_names(&llm.requests()[0]).contains(&"start_workflow".to_string()));
    assert!(t
        .rt
        .workflows()
        .list(&Default::default())
        .await
        .unwrap()
        .is_empty());
    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    assert_eq!(
        t.rt.workflows()
            .list(&Default::default())
            .await
            .unwrap()
            .len(),
        1
    );
    t.rt.shutdown();
}

/// An agent step of `member` with the outcomes it may report and the steps it may send back to.
fn agent_step(id: &str, name: &str, member: &Dot, outcomes: &[&str], back: &[&str]) -> Step {
    Step {
        id: id.into(),
        name: name.into(),
        kind: StepKind::Agent {
            member: member.id.clone(),
            job: format!("{name}."),
            can_send_back_to: back.iter().map(|s| s.to_string()).collect(),
            outcomes: outcomes.iter().map(|s| s.to_string()).collect(),
            delegate: None,
        },
    }
}

/// A team of `members` with the project "Dots" and a manual workflow "Steps" of `steps`;
/// starts a run of it.
async fn run_steps(t: &Rt, members: &[&Dot], steps: Vec<Step>) -> WorkflowRun {
    let team =
        t.rt.teams()
            .create(&TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
                name: "Dots management".into(),
                description: String::new(),
                members: members
                    .iter()
                    .map(|d| MemberSpec {
                        dot_id: d.id.clone(),
                        role: format!("{} agent", d.spec.name),
                    })
                    .collect(),
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
    let flow =
        t.rt.teams()
            .create_workflow(
                &team.id,
                &WorkflowSpec {
                    name: "Steps".into(),
                    definition: Definition {
                        trigger: Trigger::Manual,
                        steps,
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
            task: "Fix the icon.".into(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            test: false,
        })
        .await
        .unwrap()
}

/// The workflow run once `done` holds of it, within 30 s.
async fn until(t: &Rt, id: &str, done: impl Fn(&WorkflowRun) -> bool) -> WorkflowRun {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let wr = t.rt.workflows().get(id).await.unwrap();
        if done(&wr) {
            return wr;
        }
        assert!(std::time::Instant::now() < deadline, "{wr:#?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

const WORKFLOW_TOOLS: [&str; 4] = [
    "start_workflow",
    "start_member",
    "report_outcome",
    "send_back",
];

/// The workflow tools among the request's tools.
fn workflow_tools(req: &Value) -> Vec<String> {
    tool_names(req)
        .into_iter()
        .filter(|n| WORKFLOW_TOOLS.contains(&n.as_str()))
        .collect()
}

/// The local engine: a step with outcomes reports one, the run that carries the person's answer
/// to its question (a later run of its thread) still has the tool, the last report counts, and
/// the branch after it takes that outcome's path. A step without outcomes or send-backs has no
/// workflow tool.
#[tokio::test]
async fn a_local_step_reports_its_outcome_and_the_branch_after_it_takes_that_path() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        text("Built."),
        call("c1", "ask", json!({"question": "Light or dark icon?"})),
        text("Asked."),
        call("c2", "report_outcome", json!({"outcome": "maybe"})),
        call("c3", "report_outcome", json!({"outcome": "approved"})),
        call("c4", "report_outcome", json!({"outcome": " Changes "})),
        text("Needs changes."),
        text("Fixed."),
    ])
    .await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    let route = Step {
        id: "route".into(),
        name: "Route".into(),
        kind: StepKind::Branch {
            on: BranchOn::Outcome,
            paths: vec![
                BranchPath {
                    label: "approved".into(),
                    steps: vec![Step {
                        id: "check".into(),
                        name: "Your check".into(),
                        kind: StepKind::Check { send_back_to: None },
                    }],
                },
                BranchPath {
                    label: "changes".into(),
                    steps: vec![agent_step("fix", "Fix it", &helper, &[], &[])],
                },
            ],
            otherwise: vec![],
        },
    };
    let wr = run_steps(
        &t,
        &[&helper],
        vec![
            agent_step("build", "Build it", &helper, &[], &[]),
            agent_step(
                "review",
                "Review it",
                &helper,
                &["approved", "changes"],
                &[],
            ),
            route,
        ],
    )
    .await;
    let question = loop {
        let open =
            t.rt.store()
                .list_messages(Some(&helper.id))
                .await
                .unwrap()
                .into_iter()
                .find(|m| m.answered_at.is_none());
        if let Some(q) = open {
            break q;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    wait_status(
        t.rt.store(),
        question.run_id.as_deref().unwrap(),
        RunStatus::Succeeded,
        30,
    )
    .await;
    let answer = t.rt.reply(&question.id, "The light one.").await.unwrap();
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let reqs = llm.requests();
    // The build step has no workflow tool; the review step only `report_outcome`, in its first
    // run and in the run that carries the answer.
    assert!(
        workflow_tools(&reqs[0]).is_empty(),
        "{:?}",
        tool_names(&reqs[0])
    );
    assert!(!prompt(&reqs[0]).contains("### Workflow tools"));
    assert_eq!(workflow_tools(&reqs[1]), ["report_outcome"]);
    assert_eq!(workflow_tools(&reqs[3]), ["report_outcome"]);
    let review = prompt(&reqs[1]);
    assert!(
        review.contains(
            "### Workflow tools\n\n- Report how the step came out with `report_outcome`, one of: \"approved\", \"changes\". The workflow goes on by the outcome you report.\n\nWhen you are done"
        ),
        "{review}"
    );
    assert_eq!(
        tool_result(&reqs[4], "c2"),
        "\"maybe\" is not an outcome of this step; use one of: \"approved\", \"changes\"."
    );
    assert_eq!(tool_result(&reqs[5], "c3"), "Outcome recorded.");
    assert_eq!(tool_result(&reqs[6], "c4"), "Outcome recorded.");
    // The last report counts, as the step declares it; the branch took its path.
    let ran: Vec<&str> = wr.steps.iter().map(|s| s.step_id.as_str()).collect();
    assert_eq!(ran, ["build", "review", "route", "fix"]);
    assert_eq!(wr.steps[1].outcome.as_deref(), Some("changes"));
    assert_eq!(
        wr.steps[1].handoff.as_ref().unwrap().outcome.as_deref(),
        Some("changes")
    );
    assert_eq!(wr.steps[2].decision, Some(json!({"path": "changes"})));
    assert_eq!(
        answer.workflow_step_run_id.as_deref(),
        Some(wr.steps[1].id.as_str())
    );
    t.rt.shutdown();
}

/// The local engine: `send_back` names an earlier step by name without case and needs a note;
/// the last call is recorded on the step run. Once the step has ended, its runs have no
/// workflow tools, and a call made with the tools a run had records nothing.
#[tokio::test]
async fn a_local_step_asks_for_a_send_back_and_a_closed_step_records_nothing() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        text("Built."),
        call(
            "c1",
            "send_back",
            json!({"to_step": "Nowhere", "note": "x"}),
        ),
        call(
            "c2",
            "send_back",
            json!({"to_step": "build it", "note": "  "}),
        ),
        call(
            "c3",
            "send_back",
            json!({"to_step": "Build it", "note": "First try."}),
        ),
        call(
            "c4",
            "send_back",
            json!({"to_step": "BUILD IT", "note": " Use the light icon. ", "extra": 1}),
        ),
        text("Sent back."),
    ])
    .await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    let wr = run_steps(
        &t,
        &[&helper],
        vec![
            agent_step("build", "Build it", &helper, &[], &[]),
            agent_step("review", "Review it", &helper, &[], &["build"]),
        ],
    )
    .await;
    // The review's send-back is performed when its step ends: "Build it" runs again.
    let wr = until(&t, &wr.id, |wr| {
        wr.send_backs == 1
            && wr
                .steps
                .iter()
                .any(|s| s.step_id == "build" && s.attempt == 2)
    })
    .await;
    assert_eq!(wr.send_back.as_ref().map(|s| s.to.as_str()), Some("build"));
    let reqs = llm.requests();
    assert_eq!(workflow_tools(&reqs[1]), ["send_back"]);
    let review = prompt(&reqs[1]);
    assert!(
        review.contains(
            "### Workflow tools\n\n- If the work must be redone in an earlier step, send it back with `send_back` to one of: \"Build it\", with a note that says what to change.\n"
        ),
        "{review}"
    );
    let answers: Vec<String> = (1..=4)
        .map(|i| tool_result(&reqs[i + 1], &format!("c{i}")))
        .collect();
    let sent = "Sent back to \"Build it\". End your turn now: the work goes back when you finish.";
    assert_eq!(
        answers,
        [
            "You can send work back only to: \"Build it\".",
            "Give a note of 1 to 8,000 characters that says what to change.",
            sent,
            sent,
        ]
    );
    let step_run = &wr.steps[1];
    assert_eq!(
        step_run.send_back,
        Some(json!({"to": "build", "note": "Use the light icon."}))
    );
    assert_eq!(step_run.outcome, None);
    // The step has ended: a run of its thread has no workflow tools, and a call made with
    // those its run had is refused.
    let run =
        t.rt.store()
            .runs_of_step_run(&step_run.id)
            .await
            .unwrap()
            .remove(0);
    assert_eq!(team_tools::for_run(t.rt.store(), &run).await.unwrap(), None);
    let step = wr.definition.step("review").unwrap();
    let tools = StepTools::of(&step_run.id, step, &wr.definition).unwrap();
    let refused =
        t.rt.workflows()
            .call_send_back(
                &run.id,
                &tools,
                &json!({"to_step": "Build it", "note": "Again."}),
            )
            .await;
    assert_eq!(refused, Err(STEP_CLOSED.to_string()));
    let refused =
        t.rt.workflows()
            .call_report_outcome(&run.id, &tools, &json!({"outcome": "x"}))
            .await;
    assert_eq!(refused, Err(STEP_CLOSED.to_string()));
    let after = t.rt.store().get_step_run(&step_run.id).await.unwrap();
    assert_eq!(after.send_back, step_run.send_back);
    assert_eq!(after.outcome, None);
    t.rt.shutdown();
}

/// The Claude engine: Dots' own server lists a step's tools and performs them, the CLI may call
/// them without asking (Dots checks each call itself), and the prompt names them as the CLI's
/// model knows them.
#[tokio::test]
async fn a_claude_step_reports_and_sends_back_through_dots_own_server() {
    let t = start(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "report_outcome",
                      "arguments": {"outcome": "approved"}}},
        {"mcp_call": {"server": "dots", "tool": "send_back",
                      "arguments": {"to_step": "build it", "note": "Again."}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]))
    .await;
    let llm = Llm::start(vec![text("Built.")]).await;
    let builder =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "builder"), None, None)
            .await
            .unwrap();
    let mut s = spec("reviewer");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.policy = Policy::preset(Preset::Trusted);
    let reviewer = t.rt.create_dot(s, None, None).await.unwrap();
    let wr = run_steps(
        &t,
        &[&builder, &reviewer],
        vec![
            agent_step("build", "Build it", &builder, &[], &[]),
            agent_step("review", "Review it", &reviewer, &["approved"], &["build"]),
        ],
    )
    .await;
    // The review's send-back is performed when its step ends: "Build it" runs again.
    let wr = until(&t, &wr.id, |wr| {
        wr.send_backs == 1
            && wr
                .steps
                .iter()
                .any(|s| s.step_id == "build" && s.attempt == 2)
    })
    .await;
    assert_eq!(wr.send_back.as_ref().map(|s| s.to.as_str()), Some("build"));
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    let allowed: Vec<&str> = args
        .iter()
        .skip_while(|a| *a != "--allowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .map(String::as_str)
        .filter(|a| {
            WORKFLOW_TOOLS
                .iter()
                .any(|t| *a == format!("mcp__dots__{t}"))
        })
        .collect();
    assert_eq!(
        allowed,
        ["mcp__dots__report_outcome", "mcp__dots__send_back"],
        "{args:?}"
    );
    let prompt = recorded["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("with `mcp__dots__report_outcome`, one of: \"approved\".")
            && prompt.contains("send it back with `mcp__dots__send_back` to one of: \"Build it\","),
        "{prompt}"
    );
    let step_run = &wr.steps[1];
    let run =
        t.rt.store()
            .runs_of_step_run(&step_run.id)
            .await
            .unwrap()
            .remove(0);
    let events = t.rt.store().list_events(&run.id, 0).await.unwrap();
    let listed = events.iter().find(|e| e.kind == "raw").unwrap();
    let listed: Value = serde_json::from_str(listed.data["line"].as_str().unwrap()).unwrap();
    let listed: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|n| WORKFLOW_TOOLS.contains(n))
        .collect();
    assert_eq!(listed, ["report_outcome", "send_back"]);
    let results: Vec<(&str, bool)> = events
        .iter()
        .filter(|e| e.kind == "tool_result")
        .map(|e| {
            (
                e.data["output"].as_str().unwrap(),
                e.data["is_error"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            ("Outcome recorded.", false),
            (
                "Sent back to \"Build it\". End your turn now: the work goes back when you finish.",
                false
            ),
        ]
    );
    assert_eq!(step_run.outcome.as_deref(), Some("approved"));
    assert_eq!(
        step_run.send_back,
        Some(json!({"to": "build", "note": "Again."}))
    );
    t.rt.shutdown();
}

/// The local engine: a step that declares neither outcomes nor send-backs is offered neither
/// tool, and a call of either is refused and records nothing.
#[tokio::test]
async fn a_local_step_not_offered_report_outcome_or_send_back_is_refused_both() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        call("c1", "report_outcome", json!({"outcome": "done"})),
        call(
            "c2",
            "send_back",
            json!({"to_step": "Build it", "note": "Again."}),
        ),
        text("Done."),
    ])
    .await;
    let helper =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "helper"), None, None)
            .await
            .unwrap();
    let wr = run_steps(
        &t,
        &[&helper],
        vec![agent_step("review", "Review it", &helper, &[], &[])],
    )
    .await;
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let reqs = llm.requests();
    assert!(
        workflow_tools(&reqs[0]).is_empty(),
        "{:?}",
        tool_names(&reqs[0])
    );
    assert_eq!(
        tool_result(&reqs[1], "c1"),
        "report_outcome is not available in this run"
    );
    assert_eq!(
        tool_result(&reqs[2], "c2"),
        "send_back is not available in this run"
    );
    let ran: Vec<&str> = wr.steps.iter().map(|s| s.step_id.as_str()).collect();
    assert_eq!(ran, ["review"]);
    assert_eq!(wr.steps[0].outcome, None);
    assert_eq!(wr.steps[0].send_back, None);
    assert_eq!(wr.send_backs, 0);
    assert_eq!(wr.send_back, None);
    t.rt.shutdown();
}

/// The Claude engine: Dots' own server neither lists nor performs `report_outcome` and
/// `send_back` for a step that does not offer them, and nothing is recorded.
#[tokio::test]
async fn a_claude_step_not_offered_report_outcome_or_send_back_is_refused_both() {
    let t = start(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "report_outcome",
                      "arguments": {"outcome": "done"}}},
        {"mcp_call": {"server": "dots", "tool": "send_back",
                      "arguments": {"to_step": "Review it", "note": "Again."}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]))
    .await;
    let mut s = spec("reviewer");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.policy = Policy::preset(Preset::Trusted);
    let reviewer = t.rt.create_dot(s, None, None).await.unwrap();
    let wr = run_steps(
        &t,
        &[&reviewer],
        vec![agent_step("review", "Review it", &reviewer, &[], &[])],
    )
    .await;
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        !args.iter().any(|a| WORKFLOW_TOOLS
            .iter()
            .any(|t| *a == format!("mcp__dots__{t}"))),
        "{args:?}"
    );
    assert!(!recorded["prompt"]
        .as_str()
        .unwrap()
        .contains("### Workflow tools"));
    let step_run = &wr.steps[0];
    let run =
        t.rt.store()
            .runs_of_step_run(&step_run.id)
            .await
            .unwrap()
            .remove(0);
    let events = t.rt.store().list_events(&run.id, 0).await.unwrap();
    let listed = events.iter().find(|e| e.kind == "raw").unwrap();
    let listed: Value = serde_json::from_str(listed.data["line"].as_str().unwrap()).unwrap();
    let listed: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|n| WORKFLOW_TOOLS.contains(n))
        .collect();
    assert!(listed.is_empty(), "{listed:?}");
    // Both calls were answered as unknown tools (an error, so no text), not performed.
    let results: Vec<&str> = events
        .iter()
        .filter(|e| e.kind == "tool_result")
        .map(|e| e.data["output"].as_str().unwrap())
        .collect();
    assert_eq!(results, ["", ""]);
    assert_eq!(step_run.outcome, None);
    assert_eq!(step_run.send_back, None);
    assert_eq!(wr.send_backs, 0);
    assert_eq!(wr.steps.len(), 1);
    t.rt.shutdown();
}

/// The plain Sandboxed preset does not ask before a step's run reports its outcome or sends
/// the work back (the person's decision, 2026-11-04): both are recorded at once, and no
/// approval is asked for.
#[tokio::test]
async fn a_sandboxed_step_reports_its_outcome_and_sends_back_without_asking() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![
        text("Built."),
        call("c1", "report_outcome", json!({"outcome": "done"})),
        call(
            "c2",
            "send_back",
            json!({"to_step": "Build it", "note": "Use the light icon."}),
        ),
        text("Sent back."),
        text("Built again."),
        text("Fine now."),
    ])
    .await;
    let mut s = local_spec(&t, &llm.endpoint(), "helper");
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 30;
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
    let mut events = t.rt.subscribe();
    let wr = run_steps(
        &t,
        &[&helper],
        vec![
            agent_step("build", "Build it", &helper, &[], &[]),
            agent_step("review", "Review it", &helper, &["done"], &["build"]),
        ],
    )
    .await;
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let reqs = llm.requests();
    assert_eq!(tool_result(&reqs[2], "c1"), "Outcome recorded.");
    assert_eq!(
        tool_result(&reqs[3], "c2"),
        "Sent back to \"Build it\". End your turn now: the work goes back when you finish."
    );
    let review = wr
        .steps
        .iter()
        .find(|s| s.step_id == "review" && s.attempt == 1)
        .unwrap();
    assert_eq!(review.outcome.as_deref(), Some("done"));
    assert_eq!(
        review.send_back,
        Some(json!({"to": "build", "note": "Use the light icon."}))
    );
    assert_eq!(wr.send_backs, 1);
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(
                event,
                dots_core::events::RuntimeEvent::ApprovalRequested { .. }
            ),
            "{event:?}"
        );
    }
    t.rt.shutdown();
}

/// The plain Sandboxed preset still asks before a delegate step's run starts a member: the
/// member starts once the person allows it.
#[tokio::test]
async fn a_sandboxed_lead_asks_before_it_starts_a_member() {
    let t = start(json!([])).await;
    let lead_llm = Llm::start(vec![
        call(
            "c1",
            "start_member",
            json!({"member": "dev", "task": "Make the icon light."}),
        ),
        text("Started."),
        text("All in."),
    ])
    .await;
    let dev_llm = Llm::start(vec![text("Made it light.")]).await;
    let mut s = local_spec(&t, &lead_llm.endpoint(), "lead");
    s.policy = Policy::preset(Preset::Sandboxed);
    s.approval_wait_secs = 30;
    let lead = t.rt.create_dot(s, None, None).await.unwrap();
    let dev =
        t.rt.create_dot(local_spec(&t, &dev_llm.endpoint(), "dev"), None, None)
            .await
            .unwrap();
    let mut events = t.rt.subscribe();
    let wr = run_steps(&t, &[&lead, &dev], vec![delegate_step(&lead, &dev)]).await;
    let approval = loop {
        if let dots_core::events::RuntimeEvent::ApprovalRequested { approval } =
            events.recv().await.unwrap()
        {
            break approval;
        }
    };
    assert_eq!(approval.tool, "Workflow");
    assert_eq!(
        approval.input,
        json!({"member": "dev", "task": "Make the icon light."})
    );
    assert!(dev_llm.requests().is_empty());
    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    assert_eq!(
        tool_result(&lead_llm.requests()[1], "c1"),
        "Started \"dev\"."
    );
    assert_eq!(wr.steps[0].handoff.as_ref().unwrap().members.len(), 1);
    t.rt.shutdown();
}

/// A step of a dot whose policy denies `Workflow` outright has neither the tools nor their
/// lines in its prompt.
#[tokio::test]
async fn a_step_of_a_dot_whose_policy_denies_workflow_has_no_workflow_tools() {
    let t = start(json!([])).await;
    let llm = Llm::start(vec![text("Done.")]).await;
    let mut s = local_spec(&t, &llm.endpoint(), "helper");
    s.policy.rules.push(Rule::new("Workflow", Action::Deny));
    let helper = t.rt.create_dot(s, None, None).await.unwrap();
    let wr = run_steps(
        &t,
        &[&helper],
        vec![agent_step("review", "Review it", &helper, &["done"], &[])],
    )
    .await;
    until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let first = llm.requests()[0].clone();
    assert!(
        workflow_tools(&first).is_empty(),
        "{:?}",
        tool_names(&first)
    );
    assert!(!prompt(&first).contains("### Workflow tools"));
    t.rt.shutdown();
}

/// A delegate step of `lead` that may start `member`, one at a time and once in all.
fn delegate_step(lead: &Dot, member: &Dot) -> Step {
    Step {
        id: "split".into(),
        name: "Split it".into(),
        kind: StepKind::Agent {
            member: lead.id.clone(),
            job: "Split it.".into(),
            can_send_back_to: vec![],
            outcomes: vec![],
            delegate: Some(Delegate {
                may_start: vec![member.id.clone()],
                at_once: 1,
                total: 1,
            }),
        },
    }
}

/// The text of the last message the person's side sent in a recorded request.
fn last_user(req: &Value) -> String {
    messages(req)
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// The local engine: a delegate step's run is offered `start_member`, its call starts the
/// member, whose run has no workflow tool, and the lead's thread is continued with the
/// member's result.
#[tokio::test]
async fn a_local_lead_starts_a_member_and_is_told_how_it_went() {
    let t = start(json!([])).await;
    let lead_llm = Llm::start(vec![
        call(
            "c1",
            "start_member",
            json!({"member": "DEV", "task": "Make the icon light."}),
        ),
        text("Started."),
        text("All in."),
    ])
    .await;
    let dev_llm = Llm::start(vec![text("Made it light.")]).await;
    let lead =
        t.rt.create_dot(local_spec(&t, &lead_llm.endpoint(), "lead"), None, None)
            .await
            .unwrap();
    let dev =
        t.rt.create_dot(local_spec(&t, &dev_llm.endpoint(), "dev"), None, None)
            .await
            .unwrap();
    let wr = run_steps(&t, &[&lead, &dev], vec![delegate_step(&lead, &dev)]).await;
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let reqs = lead_llm.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(workflow_tools(&reqs[0]), ["start_member"]);
    let first = prompt(&reqs[0]);
    assert!(
        first.contains(
            "### Workflow tools\n\n- Start other members of the team on parts of the work with \
             `start_member`, one of: \"dev\". At most 1 may run at once, and 1 in all. End your \
             turn once you have started them: you are told when they finish.\n"
        ),
        "{first}"
    );
    assert_eq!(tool_result(&reqs[1], "c1"), "Started \"dev\".");
    // The member's run has no workflow tool, and its prompt names its part.
    let member = dev_llm.requests().remove(0);
    assert!(
        workflow_tools(&member).is_empty(),
        "{:?}",
        tool_names(&member)
    );
    let part = prompt(&member);
    assert!(
        part.contains("### Your part\n\nMake the icon light.")
            && !part.contains("### Workflow tools"),
        "{part}"
    );
    // The lead goes on in its thread, with its tool, told of the member.
    assert_eq!(workflow_tools(&reqs[2]), ["start_member"]);
    assert_eq!(
        last_user(&reqs[2]),
        "The members you started have finished:\n\n#### dev: Make the icon light.\n\n\
         Status: succeeded\nBranch: none\n\nMade it light.\n\n\
         You may start no more members: end with your summary of the step."
    );
    let handoff = wr.steps[0].handoff.clone().unwrap();
    assert_eq!(handoff.summary, "All in.");
    assert_eq!(handoff.members.len(), 1);
    assert_eq!(handoff.members[0].summary, "Made it light.");
    t.rt.shutdown();
}

/// The Claude engine: Dots' own server lists `start_member` to a delegate step's run and
/// performs it, the CLI may call it without asking, and a call past `total` (the continued
/// run plays the same script) is refused.
#[tokio::test]
async fn a_claude_lead_starts_a_member_through_dots_own_server() {
    let t = start(json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "start_member",
                      "arguments": {"member": "dev", "task": "Make it."}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]))
    .await;
    let llm = Llm::start(vec![text("Made it.")]).await;
    let dev =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "dev"), None, None)
            .await
            .unwrap();
    let mut s = spec("lead");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.policy = Policy::preset(Preset::Trusted);
    let lead = t.rt.create_dot(s, None, None).await.unwrap();
    let wr = run_steps(&t, &[&lead, &dev], vec![delegate_step(&lead, &dev)]).await;
    let wr = until(&t, &wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    let allowed: Vec<&str> = args
        .iter()
        .skip_while(|a| *a != "--allowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .map(String::as_str)
        .filter(|a| {
            WORKFLOW_TOOLS
                .iter()
                .any(|t| *a == format!("mcp__dots__{t}"))
        })
        .collect();
    assert_eq!(allowed, ["mcp__dots__start_member"], "{args:?}");
    // The args are the continued run's: its prompt is the member's result.
    let told = recorded["prompt"].as_str().unwrap();
    assert!(
        told.contains("#### dev: Make it.\n\nStatus: succeeded\nBranch: none\n\nMade it.\n\n"),
        "{told}"
    );
    let runs =
        t.rt.store()
            .runs_of_step_run(&wr.steps[0].id)
            .await
            .unwrap();
    let members = t.rt.store().step_members(&wr.steps[0].id).await.unwrap();
    let own: Vec<_> = runs
        .iter()
        .filter(|r| !members.iter().any(|m| m.run_id == r.root_run_id))
        .collect();
    assert_eq!(own.len(), 2);
    let mut answers = Vec::new();
    for run in &own {
        let events = t.rt.store().list_events(&run.id, 0).await.unwrap();
        let listed = events.iter().find(|e| e.kind == "raw").unwrap();
        let listed: Value = serde_json::from_str(listed.data["line"].as_str().unwrap()).unwrap();
        let listed: Vec<&str> = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|n| WORKFLOW_TOOLS.contains(n))
            .collect();
        assert_eq!(listed, ["start_member"]);
        answers.extend(events.iter().filter(|e| e.kind == "tool_result").map(|e| {
            (
                e.data["output"].as_str().unwrap().to_string(),
                e.data["is_error"].as_bool().unwrap(),
            )
        }));
    }
    assert_eq!(
        answers,
        [
            ("Started \"dev\".".to_string(), false),
            (
                "Not started: you have started 1 members, the most this step allows.".to_string(),
                true
            ),
        ]
    );
    assert_eq!(members.len(), 1);
    t.rt.shutdown();
}
