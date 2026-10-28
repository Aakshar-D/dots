//! Account roles enforced on what a person does directly (members spec §6.3): a dot of an
//! account is created, changed, run and its approvals answered only by a role that may, and the
//! refusal names the account, the role and what it can't do. The account serves a signed member
//! list from the fake OpenBao; Personal is always Owner and an account with no member list acts
//! as Member. The orgs keep no offline copy (`offline_hours: 0`).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, roster, sign, sign_roster, token, ACME, ACME_NAME,
    ISSUED, START, TOKEN,
};
use common::{spec, wait_status};
use dots_core::accounts::{identity, Permission};
use dots_core::model::{DotSpec, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::vault::org::Orgs;
use dots_core::vault::{DotScope, Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
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
}

impl Rt {
    async fn start() -> Rt {
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let dir = tempfile::tempdir().unwrap();
        let args_out = dir.path().join("args.json");
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.clock = FakeClock::new(at(START));
        cfg.secrets = Some(Arc::new(MemorySecrets::new()));
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
            keys: keys(),
        }
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

    let answer = viewer_cant("answer approvals");
    assert_eq!(
        refused(t.rt.decide_approval(&approval.id, true, None, None).await),
        answer
    );
    assert_eq!(
        refused(t.rt.decide_run_approvals(&run.id, true, None).await),
        answer
    );

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
