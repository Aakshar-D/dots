//! Account roles enforced on what a person does directly (members spec §6.3): a dot of an
//! account is created, changed, run and its approvals answered only by a role that may, and the
//! refusal names the account, the role and what it can't do. The account serves a signed member
//! list from the fake OpenBao; Personal is always Owner and an account with no member list acts
//! as Member. The orgs keep no offline copy (`offline_hours: 0`).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::apps::write_app;
use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, roster, sign, sign_roster, token, ACME, ACME_NAME,
    ISSUED, START, TOKEN,
};
use common::{spec, wait_status};
use dots_core::accounts::{identity, Access, Membership, Permission};
use dots_core::apps::call::Actor;
use dots_core::apps::propose::{AppProposal, Decision};
use dots_core::apps::source::read_folder;
use dots_core::library::AppOrigin;
use dots_core::model::{
    DotSpec, FolderConfig, HoldReason, MemberSpec, NewRun, NewWorkflowRun, ProjectSpec, RunStatus,
    TeamSpec, TriggerKind, WatcherConfig, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::vault::org::{OrgStatus, Orgs};
use dots_core::vault::{DotScope, Grants, MemorySecrets};
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::workflows::StartRequest;
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePool;
use tempfile::TempDir;

const LEVEL: &str = "org-plain-level-roles-7d21";
const WAITING_TEXT: &str =
    "Acme Engineering: you aren't on its member list yet. Send your Dots ID to an Admin.";

fn viewer_cant(action: &str) -> String {
    format!("Acme Engineering: a Viewer can't {action}. Ask an Admin or Owner.")
}

struct Rt {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
    bao: FakeOpenBao,
    keys: KeyPair,
    secrets: Arc<MemorySecrets>,
}

impl Rt {
    async fn start() -> Rt {
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let secrets = Arc::new(MemorySecrets::new());
        Rt::on(tempfile::tempdir().unwrap(), secrets, bao, keys()).await
    }

    /// A runtime on the data folder in `dir`, with `secrets`, as a restart of Dots finds them.
    async fn on(dir: TempDir, secrets: Arc<MemorySecrets>, bao: FakeOpenBao, keys: KeyPair) -> Rt {
        let args_out = dir.path().join("args.json");
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.clock = FakeClock::new(at(START));
        cfg.secrets = Some(secrets.clone());
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        cfg.claude_env.push((
            "FAKE_CLAUDE_ARGS_OUT".into(),
            args_out.to_string_lossy().to_string(),
        ));
        let rt = Runtime::start(cfg).await.unwrap();
        Rt {
            dir,
            rt,
            args_out,
            bao,
            keys,
            secrets,
        }
    }

    async fn runs(&self, dot_id: &str) -> usize {
        self.rt
            .store()
            .list_runs(Some(dot_id), 100)
            .await
            .unwrap()
            .len()
    }

    fn orgs(&self) -> &Orgs {
        self.rt.vault().orgs().expect("the runtime's orgs")
    }

    /// This PC's Dots ID, made when the runtime started.
    async fn me(&self) -> String {
        identity(self.rt.store()).await.unwrap().public_key
    }

    /// Serves Acme's `DOTS_TEST_LEVEL` under a policy with `extra`, and `signed_roster`.
    fn serve(&self, extra: Value, signed_roster: Option<&str>) {
        let mut extra = extra;
        extra["plain"] = json!(["DOTS_TEST_LEVEL"]);
        extra["offline_hours"] = json!(0);
        let signed = sign(&self.keys, &policy(ACME, ISSUED, extra));
        self.bao
            .put_env("secret", "dots", json!({"DOTS_TEST_LEVEL": LEVEL}));
        self.bao.put_policy("secret", "dots", &signed);
        match signed_roster {
            Some(text) => self.bao.put_roster("secret", "dots", text),
            None => self.bao.remove("secret", "dots/roster"),
        }
    }

    /// Serves a member list at `seq` with an owner who is not this PC and, with `Some(role)`,
    /// this PC as `role`.
    async fn serve_list(&self, seq: u64, role: Option<&str>) {
        let mut members = vec![member("01OWNER", "owner@acme.com", "owner", &stranger())];
        if let Some(role) = role {
            members.push(member("01ME", "me@acme.com", role, &self.me().await));
        }
        let text = roster(ACME, seq, Value::Array(members));
        let required = json!({"v": 2, "roster": "required", "admins": []});
        self.serve(required, Some(&sign_roster(&self.keys, &text)));
    }

    async fn join(&self) {
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        self.orgs().join(&code, Some(token())).await.unwrap();
    }

    /// Joins Acme with this PC on its member list as `role`.
    async fn join_as(&self, role: &str) {
        self.serve_list(1, Some(role)).await;
        self.join().await;
    }

    /// Serves the list again at `seq` with this PC as `role`, and fetches it.
    async fn become_(&self, seq: u64, role: Option<&str>) {
        self.serve_list(seq, role).await;
        self.rt.refresh_org(ACME).await.unwrap();
    }

    /// A folder dot named `triage` whose shell is granted `DOTS_TEST_LEVEL`.
    fn triage(&self) -> DotSpec {
        let mut s = spec("triage");
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.variables = Grants([("shell".to_string(), vec!["DOTS_TEST_LEVEL".to_string()])].into());
        s
    }

    /// Runs the dot now and waits for it to succeed; the level its CLI was given.
    async fn run(&self, dot_id: &str) -> String {
        let run = self.rt.run_now(dot_id).await.unwrap();
        wait_status(self.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
        let rec: Value =
            serde_json::from_str(&std::fs::read_to_string(&self.args_out).unwrap()).unwrap();
        rec["env"]["DOTS_TEST_LEVEL"].as_str().unwrap().to_string()
    }
}

fn member(id: &str, email: &str, role: &str, key: &str) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

fn stranger() -> String {
    public_key(&keys())
}

/// The text of a `NotAllowed` refusal; anything else fails the test.
fn refused<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::NotAllowed(text)) => text,
        other => panic!("expected NotAllowed, got {other:?}"),
    }
}

#[tokio::test]
async fn viewer_cannot_create_edit_run_or_answer() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
    let approval =
        t.rt.store()
            .create_approval(&run.id, "Bash", &json!({"command": "ls"}))
            .await
            .unwrap();
    t.become_(2, Some("viewer")).await;
    let edit = viewer_cant("create or change dots");
    // No queued run is claimed, so nothing but the calls below changes the approval and runs.
    t.rt.hold_runs(true).await;

    assert_eq!(
        refused(t.rt.create_dot(t.triage(), Some(ACME.into())).await),
        edit
    );
    let mut changed = dot.spec.clone();
    changed.instructions = "something else".into();
    assert_eq!(refused(t.rt.update_dot(&dot.id, changed).await), edit);
    assert_eq!(refused(t.rt.set_dot_enabled(&dot.id, false).await), edit);
    assert_eq!(refused(t.rt.regenerate_webhook_token(&dot.id).await), edit);
    assert_eq!(refused(t.rt.delete_dot(&dot.id).await), edit);
    // Nothing changed.
    let stored = t.rt.store().get_dot(&dot.id).await.unwrap();
    assert_eq!(stored.spec, dot.spec);
    assert!(!stored.has_webhook_token);

    let run_text = viewer_cant("run dots");
    assert_eq!(refused(t.rt.run_now(&dot.id).await), run_text);
    assert_eq!(refused(t.rt.chat(&dot.id, "hello", None).await), run_text);
    // No run was enqueued.
    assert_eq!(t.runs(&dot.id).await, 1);

    let answer = viewer_cant("answer approvals");
    assert_eq!(
        refused(t.rt.decide_approval(&approval.id, true, None, None).await),
        answer
    );
    assert_eq!(
        refused(t.rt.decide_run_approvals(&run.id, true, None).await),
        answer
    );
    let still = t.rt.store().get_approval(&approval.id).await.unwrap();
    assert_eq!(still, approval);

    let mcp = viewer_cant("add MCP servers");
    let scope = DotScope::Org(ACME.into());
    assert_eq!(
        refused(
            t.rt.library()
                .add_confirmed_origin("srv", &scope, "https://mcp.acme.com")
                .await
        ),
        mcp
    );
    assert_eq!(
        refused(t.rt.library().accept("srv", &scope, "fp").await),
        mcp
    );

    // `require` answers the same for what the app checks itself.
    assert_eq!(
        refused(t.rt.require(Some(ACME), Permission::ManageMembers).await),
        viewer_cant("manage members")
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn member_can_create_and_run() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    let mut changed = dot.spec.clone();
    changed.instructions = "something else".into();
    t.rt.update_dot(&dot.id, changed).await.unwrap();
    assert_eq!(t.run(&dot.id).await, LEVEL);
    // A Member does not manage the account.
    assert_eq!(
        refused(t.rt.require(Some(ACME), Permission::ManageMembers).await),
        "Acme Engineering: a Member can't manage members. Ask an Admin or Owner."
    );
    t.rt.delete_dot(&dot.id).await.unwrap();
    t.rt.shutdown();
}

#[tokio::test]
async fn member_grant_change_needs_grant_permission_ok_for_member() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    let mut regranted = dot.spec.clone();
    regranted.variables = Grants(
        [(
            "shell".to_string(),
            vec!["DOTS_TEST_LEVEL".to_string(), "API_KEY".to_string()],
        )]
        .into(),
    );
    let saved = t.rt.update_dot(&dot.id, regranted.clone()).await.unwrap();
    assert_eq!(saved.spec.variables, regranted.variables);
    t.rt.shutdown();
}

#[tokio::test]
async fn removed_run_refused_with_removed_text() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    // A new list leaves out this PC, which was on the last one: removed.
    t.become_(2, None).await;
    let removed = "Acme Engineering: you were removed from it.";
    assert_eq!(refused(t.rt.run_now(&dot.id).await), removed);
    match t.rt.vault().before_run(&dot).await {
        Err(Error::NotAllowed(got)) => assert_eq!(got, removed),
        other => panic!("expected NotAllowed, got {other:?}"),
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn waiting_run_refused_with_waiting_text() {
    let t = Rt::start().await;
    // Joined before the account asked for a member list: legacy.
    t.serve(json!({}), None);
    t.join().await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    // The account now asks for a list that leaves this PC out: waiting, never removed.
    t.serve_list(1, None).await;
    t.rt.refresh_org(ACME).await.unwrap();

    assert_eq!(refused(t.rt.run_now(&dot.id).await), WAITING_TEXT);
    assert_eq!(
        refused(t.rt.chat(&dot.id, "hello", None).await),
        WAITING_TEXT
    );
    match t.rt.vault().before_run(&dot).await {
        Err(Error::NotAllowed(got)) => assert_eq!(got, WAITING_TEXT),
        other => panic!("expected NotAllowed, got {other:?}"),
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn personal_always_allowed() {
    let t = Rt::start().await;
    // A Viewer of an account is still Owner of Personal.
    t.join_as("viewer").await;
    let mut s = t.triage();
    s.variables = Grants::default();
    let dot = t.rt.create_dot(s, None).await.unwrap();
    let mut changed = dot.spec.clone();
    changed.instructions = "something else".into();
    t.rt.update_dot(&dot.id, changed).await.unwrap();
    t.rt.regenerate_webhook_token(&dot.id).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    for p in [Permission::ManageMembers, Permission::SignRoster] {
        t.rt.require(None, p).await.unwrap();
    }
    t.rt.delete_dot(&dot.id).await.unwrap();
    t.rt.shutdown();
}

#[tokio::test]
async fn legacy_account_acts_as_member() {
    let t = Rt::start().await;
    t.serve(json!({}), None);
    t.join().await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    assert_eq!(t.run(&dot.id).await, LEVEL);
    t.rt.require(Some(ACME), Permission::GrantEntries)
        .await
        .unwrap();
    assert_eq!(
        refused(t.rt.require(Some(ACME), Permission::ManageWorkspaces).await),
        "Acme Engineering: a Member can't manage workspaces. Ask an Admin or Owner."
    );
    t.rt.shutdown();
}

/// A workflow of one agent step, done by `member`.
fn flow(member: &str) -> WorkflowSpec {
    WorkflowSpec {
        name: "fix".into(),
        definition: Definition {
            trigger: Trigger::Manual,
            steps: vec![Step {
                id: "a".into(),
                name: "Work".into(),
                kind: StepKind::Agent {
                    member: member.into(),
                    job: "Fix it.".into(),
                },
            }],
            budget: Budget::default(),
        },
        enabled: true,
    }
}

#[tokio::test]
async fn viewer_cannot_answer_or_change_through_the_other_doors() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    let store = t.rt.store();
    // What a Member made while it could: a run that asked a question and proposed an app, a
    // memory, a watcher, and a team with a project, a workflow and a workflow run.
    t.rt.hold_runs(true).await;
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let question = store
        .add_question(&dot.id, &run.id, "Which branch?", None)
        .await
        .unwrap()
        .unwrap();
    let proposal: AppProposal = serde_json::from_value(json!({
        "id": "proposal-1", "dot_id": dot.id, "run_id": run.id, "root_run_id": run.root_run_id,
        "app_name": "notes", "app_id": null, "base_version": null, "note": "Built it.",
        "summary": {
            "title": "Notes", "description": "", "tools": [], "permissions": [],
            "tests": {"results": []}, "files": 0, "bytes": 0, "migrations": [],
            "update_from": null
        },
        "status": "pending", "answer": null, "answer_run_id": null,
        "created_at": START, "decided_at": null
    }))
    .unwrap();
    store.add_app_proposal(&proposal).await.unwrap();
    let memory = t.rt.memory_add(&dot.id, "Use main.").await.unwrap();
    let folder = t.dir.path().to_string_lossy().to_string();
    let watcher =
        t.rt.watcher_add(
            &dot.id,
            "inbox",
            WatcherConfig::Folder(FolderConfig::new(&folder)),
        )
        .await
        .unwrap();
    let team_spec = TeamSpec {
        name: "crew".into(),
        description: String::new(),
        members: vec![MemberSpec {
            dot_id: dot.id.clone(),
            role: String::new(),
        }],
        org_id: Some(ACME.into()),
    };
    let team = t.rt.teams().create(&team_spec).await.unwrap();
    let project_spec = ProjectSpec {
        name: "repo".into(),
        repo_path: folder.clone(),
        default_branch: None,
        remote: "origin".into(),
    };
    // Added in the store: the folder is not a git repo, which the store does not check.
    let project = store.add_project(&team.id, &project_spec).await.unwrap();
    let workflow =
        t.rt.teams()
            .create_workflow(&team.id, &flow(&dot.id))
            .await
            .unwrap();
    let wr = store
        .create_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id.clone(),
            team_id: team.id.clone(),
            definition: workflow.spec.definition.clone(),
            project_id: project.id.clone(),
            project: project_spec.clone(),
            task: "Fix it.".into(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            test: false,
        })
        .await
        .unwrap();
    t.become_(2, Some("viewer")).await;
    let team = t.rt.teams().get(&team.id).await.unwrap();
    let edit = viewer_cant("create or change dots");
    let answer = viewer_cant("answer approvals");
    let run_text = viewer_cant("run dots");

    // Answers.
    assert_eq!(refused(t.rt.reply(&question.id, "main").await), answer);
    assert!(store.get_message(&question.id).await.unwrap().waits());
    assert_eq!(
        refused(
            t.rt.decide_app_proposal(&proposal.id, Decision::Discard)
                .await
        ),
        answer
    );
    assert_eq!(
        store.get_app_proposal(&proposal.id).await.unwrap(),
        proposal
    );
    assert_eq!(t.runs(&dot.id).await, 1);

    // Memories and watchers.
    assert_eq!(refused(t.rt.memory_add(&dot.id, "More.").await), edit);
    assert_eq!(
        refused(t.rt.memory_update(&memory.id, Some("Other."), None).await),
        edit
    );
    assert_eq!(refused(t.rt.memory_delete(&memory.id).await), edit);
    assert_eq!(store.list_memories(&dot.id).await.unwrap(), vec![memory]);
    let config = WatcherConfig::Folder(FolderConfig::new(&folder));
    assert_eq!(
        refused(t.rt.watcher_add(&dot.id, "more", config.clone()).await),
        edit
    );
    assert_eq!(
        refused(t.rt.watcher_update(&watcher.id, "renamed", config).await),
        edit
    );
    assert_eq!(
        refused(t.rt.watcher_set_enabled(&watcher.id, false).await),
        edit
    );
    assert_eq!(refused(t.rt.watcher_resume(&watcher.id).await), edit);
    assert_eq!(refused(t.rt.watcher_delete(&watcher.id).await), edit);
    assert_eq!(store.list_watchers(&dot.id).await.unwrap(), vec![watcher]);

    // Teams, projects and workflows.
    let teams = t.rt.teams();
    assert_eq!(refused(teams.create(&team_spec).await), edit);
    let mut renamed = team_spec.clone();
    renamed.name = "renamed".into();
    assert_eq!(refused(teams.update(&team.id, &renamed).await), edit);
    assert_eq!(refused(teams.delete(&team.id).await), edit);
    assert_eq!(
        refused(teams.add_project(&team.id, &project_spec).await),
        edit
    );
    assert_eq!(
        refused(teams.update_project(&project.id, &project_spec).await),
        edit
    );
    assert_eq!(refused(teams.delete_project(&project.id).await), edit);
    assert_eq!(
        refused(teams.create_workflow(&team.id, &flow(&dot.id)).await),
        edit
    );
    assert_eq!(
        refused(teams.update_workflow(&workflow.id, &flow(&dot.id)).await),
        edit
    );
    assert_eq!(refused(teams.delete_workflow(&workflow.id).await), edit);
    assert_eq!(refused(t.rt.workflows().delete_team(&team.id).await), edit);
    assert_eq!(teams.get(&team.id).await.unwrap(), team);
    assert_eq!(
        teams.list_workflows(&team.id).await.unwrap(),
        vec![workflow.clone()]
    );

    // Workflow runs.
    let start = StartRequest {
        workflow_id: workflow.id.clone(),
        project_id: project.id.clone(),
        task: "Fix it again.".into(),
        trigger: WorkflowTrigger::Manual,
        origin_run_id: None,
        test: false,
    };
    let workflows = t.rt.workflows();
    let before = store
        .list_workflow_runs(Some(&team.id), 100)
        .await
        .unwrap()
        .len();
    assert_eq!(refused(workflows.start(start).await), run_text);
    assert_eq!(refused(workflows.retry_step(&wr.id).await), run_text);
    assert_eq!(refused(workflows.add_budget(&wr.id).await), run_text);
    assert_eq!(refused(workflows.approve_check(&wr.id, None).await), answer);
    assert_eq!(
        store
            .list_workflow_runs(Some(&team.id), 100)
            .await
            .unwrap()
            .len(),
        before
    );

    // A worktree of the account's run.
    assert_eq!(refused(t.rt.discard_worktree(&run.id).await), run_text);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_damaged_org_acts_as_waiting() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    // The org's saved settings no longer read: on the next start it loads as damaged.
    let Rt {
        dir,
        rt,
        bao,
        keys,
        secrets,
        ..
    } = t;
    assert!(rt.close(std::time::Duration::from_secs(10)).await);
    let db = dir.path().join("data").join("dots.db");
    let pool = SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    sqlx::query("UPDATE orgs SET source = ? WHERE id = ?")
        .bind(json!({"kind": "dropbox", "folder": "x"}).to_string())
        .bind(ACME)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let t = Rt::on(dir, secrets, bao, keys).await;
    assert_eq!(t.rt.access(Some(ACME)), Access::Waiting);
    // Its summary says the same as its access: waiting, with the waiting words.
    let summary = t.orgs().summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Waiting);
    assert_eq!(summary.not_member_text.as_deref(), Some(WAITING_TEXT));
    assert_eq!(refused(t.rt.run_now(&dot.id).await), WAITING_TEXT);
    match t.rt.vault().before_run(&dot).await {
        Err(Error::NotAllowed(got)) => assert_eq!(got, WAITING_TEXT),
        other => panic!("expected NotAllowed, got {other:?}"),
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn scheduled_run_held_for_role_then_runs_when_role_returns() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    t.become_(2, Some("viewer")).await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
            .await
            .unwrap();
    t.rt.hold_runs(false).await;
    // A few dispatch passes later the run is still queued, listed as held for the role.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        t.rt.store().get_run(&run.id).await.unwrap().status,
        RunStatus::Queued
    );
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.held.len(), 1);
    assert_eq!(inbox.held[0].run_id, run.id);
    assert_eq!(
        inbox.held[0].reason,
        HoldReason::Role {
            account: ACME_NAME.into(),
            text: "Acme Engineering: your role no longer allows running dots".into(),
        }
    );
    // It is not a usage hold.
    let limits = t.rt.limits_get().await.unwrap();
    assert!(limits.held_run_ids.is_empty() && !limits.holding);
    // The role returns: the run goes on the next dispatch.
    t.become_(3, Some("member")).await;
    t.rt.hold_runs(false).await;
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    assert!(t.rt.inbox().await.unwrap().held.is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn release_refused_for_role_hold() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    t.become_(2, Some("viewer")).await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
            .await
            .unwrap();
    assert_eq!(
        refused(t.rt.run_release_hold(&run.id).await),
        "this run waits for Acme Engineering to let you run dots again"
    );
    assert_eq!(
        t.rt.store().get_run(&run.id).await.unwrap().status,
        RunStatus::Queued
    );
    t.rt.shutdown();
}

/// A phase 1 org whose token's ACL grants only `env` and `policy`: OpenBao refuses the roster
/// with a 403. That is no roster, not a revoked token: the legacy account joins, stays
/// connected on refresh, and keeps its values and its offline copy.
#[tokio::test]
async fn a_legacy_org_whose_roster_read_is_refused_stays_connected() {
    let t = Rt::start().await;
    t.bao.deny("secret", "dots/roster");
    let signed = sign(
        &t.keys,
        &policy(ACME, ISSUED, json!({"plain": ["DOTS_TEST_LEVEL"]})),
    );
    t.bao
        .put_env("secret", "dots", json!({"DOTS_TEST_LEVEL": LEVEL}));
    t.bao.put_policy("secret", "dots", &signed);
    t.join().await;
    t.rt.refresh_org(ACME).await.unwrap();
    assert!(t
        .bao
        .calls()
        .contains(&"GET /v1/secret/data/dots/roster".to_string()));

    let summary = t.orgs().summary(ACME).await.unwrap();
    assert_eq!(
        summary.status,
        OrgStatus::Connected,
        "{}",
        summary.status_text
    );
    assert_eq!(summary.membership, Membership::Legacy);
    assert_eq!(summary.roster_error, None);
    assert!(summary.copy_until.is_some(), "the offline copy is kept");
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    assert_eq!(t.run(&dot.id).await, LEVEL);
    t.rt.shutdown();
}

/// Under `required`, a roster the token may not read is no member list: the PC waits, and the
/// account says why.
#[tokio::test]
async fn a_required_org_whose_roster_read_is_refused_waits_and_says_why() {
    let t = Rt::start().await;
    t.bao.deny("secret", "dots/roster");
    t.serve_list(1, Some("member")).await;
    t.join().await;
    let summary = t.orgs().summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Waiting);
    assert_eq!(
        summary.roster_error.as_deref(),
        Some(
            "the account has no member list: can't read DOTS_ROSTER at secret/dots/roster \
             (HTTP 403)"
        )
    );
    t.rt.shutdown();
}

/// A webhook of a dot whose account does not let this PC run dots is refused when it is
/// called, with the refusal's words: no run is queued to fire later.
#[tokio::test]
async fn a_webhook_is_refused_when_the_role_cannot_run_dots() {
    let t = Rt::start().await;
    t.join_as("member").await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    let token = t.rt.regenerate_webhook_token(&dot.id).await.unwrap();
    let url = t.rt.webhook_url(&dot).unwrap();
    let call = || {
        reqwest::Client::new()
            .post(&url)
            .bearer_auth(&token)
            .json(&json!({"event": "build_failed"}))
            .send()
    };

    t.become_(2, Some("viewer")).await;
    let resp = call().await.unwrap();
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], viewer_cant("run dots"));

    t.become_(3, None).await;
    let resp = call().await.unwrap();
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Acme Engineering: you were removed from it.");
    assert_eq!(t.runs(&dot.id).await, 0);
    t.rt.shutdown();
}

/// A PC waiting for, or removed from, the member list may still clean up the account's dots:
/// switch them off or delete them. Switching one on or changing it stays refused.
#[tokio::test]
async fn waiting_or_removed_may_switch_off_and_delete_but_not_change() {
    let t = Rt::start().await;
    // Joined before the account asked for a member list: legacy.
    t.serve(json!({}), None);
    t.join().await;
    let one =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    let mut other = t.triage();
    other.name = "tidy".into();
    let two = t.rt.create_dot(other, Some(ACME.into())).await.unwrap();
    // The account now asks for a list that leaves this PC out: waiting.
    t.serve_list(1, None).await;
    t.rt.refresh_org(ACME).await.unwrap();
    assert_eq!(t.rt.access(Some(ACME)), Access::Waiting);

    let mut changed = one.spec.clone();
    changed.instructions = "something else".into();
    assert_eq!(
        refused(t.rt.update_dot(&one.id, changed).await),
        WAITING_TEXT
    );
    let off = t.rt.set_dot_enabled(&one.id, false).await.unwrap();
    assert!(!off.spec.enabled);
    assert_eq!(
        refused(t.rt.set_dot_enabled(&one.id, true).await),
        WAITING_TEXT
    );
    t.rt.delete_dot(&one.id).await.unwrap();

    // Added, then left off the next list: removed, with the same cleanup.
    t.become_(2, Some("member")).await;
    t.become_(3, None).await;
    assert_eq!(t.rt.access(Some(ACME)), Access::Removed);
    let removed = "Acme Engineering: you were removed from it.";
    assert_eq!(refused(t.rt.set_dot_enabled(&two.id, true).await), removed);
    t.rt.set_dot_enabled(&two.id, false).await.unwrap();
    t.rt.delete_dot(&two.id).await.unwrap();
    assert!(t.rt.store().get_dot(&two.id).await.is_err());
    t.rt.shutdown();
}

const STARTER_MANIFEST: &str = r#"{
  "name": "chores",
  "title": "Chores",
  "description": "Chores dots do",
  "tools": [
    {
      "name": "assign",
      "description": "Give a chore to a dot",
      "input": { "type": "object", "properties": { "dot": { "type": "string" } }, "required": ["dot"] },
      "writes": true
    }
  ],
  "permissions": ["start_dots"]
}"#;

const STARTER_LOGIC: &str = r#"
export function assign({ dot }, ctx) {
  return ctx.dots.start(dot, "Do the chore", "chore");
}
"#;

/// An app's start of a dot whose account does not let this PC run dots queues nothing: it is
/// refused when it would be queued, not created to fail at its start.
#[tokio::test]
async fn an_app_start_is_refused_when_the_role_cannot_run_dots() {
    let t = Rt::start().await;
    let src = t.dir.path().join("chores");
    write_app(
        &src,
        &[
            ("app.json", STARTER_MANIFEST.to_string()),
            ("logic.js", STARTER_LOGIC.to_string()),
            (
                "ui/index.html",
                "<!doctype html><title>Chores</title>\n".to_string(),
            ),
        ],
    );
    t.rt.apps()
        .install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    t.rt.apps()
        .set_can_start_dots("chores", true)
        .await
        .unwrap();
    t.join_as("member").await;
    let mut s = t.triage();
    s.uses.apps = vec!["chores".into()];
    let dot = t.rt.create_dot(s, Some(ACME.into())).await.unwrap();
    t.become_(2, Some("viewer")).await;

    let id =
        t.rt.apps()
            .call("chores", "assign", json!({"dot": dot.id}), Actor::Person)
            .await
            .unwrap();
    let id = id.as_str().unwrap();
    assert!(matches!(
        t.rt.store().get_run(id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(t.runs(&dot.id).await, 0);
    t.rt.shutdown();
}
