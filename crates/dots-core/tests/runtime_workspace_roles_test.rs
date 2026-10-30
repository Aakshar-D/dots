//! Workspace roles (members spec §6.3): an account Member's role in a workspace narrows what it
//! may do with that workspace's dots and teams; an Owner or Admin keeps its role everywhere; an
//! account Viewer stays a Viewer; a workspace the member isn't in refuses everything but the
//! cleanup actions. Legacy accounts and Personal are not narrowed. The account serves a signed
//! member list with a `payments` workspace from the fake OpenBao.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, sign, sign_roster, token, ACME, ACME_NAME, ISSUED,
    START, TOKEN,
};
use common::{spec, wait_status};
use dots_core::accounts::{identity, DotAccess};
use dots_core::model::{
    DotSpec, HoldReason, NewRun, RunStatus, TeamSpec, TriggerKind, WorkspaceMode,
};
use dots_core::store::WorkspaceRow;
use dots_core::vault::org::Orgs;
use dots_core::vault::{Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
use tempfile::TempDir;

const LEVEL: &str = "org-plain-level-workspaces-5c83";
const OUTSIDE: &str =
    "Acme Engineering: you aren't in the Payments workspace. Ask an Admin or Owner to add you.";

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

    /// Serves a member list at `seq` with the workspaces Default and Payments, an owner who is
    /// not this PC, and this PC as `role` with the workspace entries `mine`.
    async fn serve_list(&self, seq: u64, role: &str, mine: Value) {
        let members = json!([
            member(
                "01OWNER",
                "owner@acme.com",
                "owner",
                &stranger(),
                json!({"default": "admin"})
            ),
            member("01ME", "me@acme.com", role, &self.me().await, mine),
        ]);
        let text = json!({
            "v": 1, "org_id": ACME, "issued_at": "2026-10-28T09:00:00Z", "seq": seq,
            "workspaces": [
                {"id": "default", "name": "Default"},
                {"id": "payments", "name": "Payments"}
            ],
            "members": members
        })
        .to_string();
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

    /// Joins Acme as a Member of both workspaces, makes a dot there, and moves it to Payments.
    async fn payments_dot(&self) -> String {
        self.serve_list(
            1,
            "member",
            json!({"default": "member", "payments": "member"}),
        )
        .await;
        self.join().await;
        let dot = self
            .rt
            .create_dot(self.triage(), Some(ACME.into()), None)
            .await
            .unwrap();
        self.rt
            .store()
            .set_dot_workspace(&dot.id, "payments")
            .await
            .unwrap();
        dot.id
    }

    /// Serves the list again at `seq` with this PC as `role` and `mine`, and fetches it.
    async fn become_(&self, seq: u64, role: &str, mine: Value) {
        self.serve_list(seq, role, mine).await;
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

    /// A spec of the dot with other instructions.
    async fn changed(&self, dot_id: &str) -> DotSpec {
        let mut s = self.rt.store().get_dot(dot_id).await.unwrap().spec;
        s.instructions = "something else".into();
        s
    }
}

fn member(id: &str, email: &str, role: &str, key: &str, workspaces: Value) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": workspaces, "added_at": "2026-10-28T09:00:00Z"
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

fn payments_viewer_cant(action: &str) -> String {
    format!(
        "Acme Engineering: a Viewer in the Payments workspace can't {action}. Ask an Admin or Owner."
    )
}

#[tokio::test]
async fn workspace_viewer_cannot_run_and_schedules_hold() {
    let t = Rt::start().await;
    let dot = t.payments_dot().await;
    t.become_(
        2,
        "member",
        json!({"default": "member", "payments": "viewer"}),
    )
    .await;
    t.rt.hold_runs(true).await;

    let run_text = payments_viewer_cant("run dots");
    assert_eq!(refused(t.rt.run_now(&dot).await), run_text);
    assert_eq!(refused(t.rt.chat(&dot, "hello", None).await), run_text);
    let stored = t.rt.store().get_dot(&dot).await.unwrap();
    match t.rt.vault().before_run(&stored).await {
        Err(Error::NotAllowed(got)) => assert_eq!(got, run_text),
        other => panic!("expected NotAllowed, got {other:?}"),
    }
    assert_eq!(
        refused(t.rt.update_dot(&dot, t.changed(&dot).await).await),
        payments_viewer_cant("create or change dots")
    );

    // A scheduled run waits, held with the workspace's words.
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Schedule))
            .await
            .unwrap();
    t.rt.hold_runs(false).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        t.rt.store().get_run(&run.id).await.unwrap().status,
        RunStatus::Queued
    );
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.held.len(), 1);
    assert_eq!(
        inbox.held[0].reason,
        HoldReason::Role {
            account: ACME_NAME.into(),
            text:
                "Acme Engineering: your role in the Payments workspace doesn't allow running dots"
                    .into(),
        }
    );
    // Member in Payments again: the run goes.
    t.become_(
        3,
        "member",
        json!({"default": "member", "payments": "member"}),
    )
    .await;
    t.rt.hold_runs(false).await;
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn workspace_member_can_edit_and_run() {
    let t = Rt::start().await;
    let dot = t.payments_dot().await;
    // An `admin` entry gives an account Member no more than Member rights.
    t.become_(2, "member", json!({"payments": "admin"})).await;
    t.rt.update_dot(&dot, t.changed(&dot).await).await.unwrap();
    assert_eq!(t.run(&dot).await, LEVEL);
    t.rt.shutdown();
}

#[tokio::test]
async fn outside_workspace_refuses_but_cleanup() {
    let t = Rt::start().await;
    let dot = t.payments_dot().await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Manual))
            .await
            .unwrap();
    let approval =
        t.rt.store()
            .create_approval(&run.id, "Bash", &json!({"command": "ls"}))
            .await
            .unwrap();
    t.rt.hold_runs(true).await;
    t.become_(2, "member", json!({"default": "member"})).await;

    assert_eq!(refused(t.rt.run_now(&dot).await), OUTSIDE);
    assert_eq!(
        refused(t.rt.update_dot(&dot, t.changed(&dot).await).await),
        OUTSIDE
    );
    assert_eq!(
        refused(t.rt.decide_approval(&approval.id, true, None, None).await),
        OUTSIDE
    );
    assert_eq!(refused(t.rt.set_dot_enabled(&dot, true).await), OUTSIDE);
    // A team placed there is refused the same.
    let team = TeamSpec {
        name: "crew".into(),
        description: String::new(),
        members: Vec::new(),
        org_id: Some(ACME.into()),
        workspace_id: "payments".into(),
    };
    assert_eq!(refused(t.rt.teams().create(&team).await), OUTSIDE);

    // Cleaning up stays allowed.
    let off = t.rt.set_dot_enabled(&dot, false).await.unwrap();
    assert!(!off.spec.enabled);
    t.rt.store().cancel_if_inactive(&run.id).await.unwrap();
    t.rt.delete_dot(&dot).await.unwrap();
    assert!(t.rt.store().get_dot(&dot).await.is_err());
    t.rt.shutdown();
}

#[tokio::test]
async fn account_admin_is_admin_everywhere() {
    let t = Rt::start().await;
    let dot = t.payments_dot().await;
    // No Payments entry, and only `viewer` in Default: an Admin is Admin in both.
    t.become_(2, "admin", json!({"default": "viewer"})).await;
    t.rt.update_dot(&dot, t.changed(&dot).await).await.unwrap();
    assert_eq!(t.run(&dot).await, LEVEL);
    let other =
        t.rt.create_dot(
            {
                let mut s = t.triage();
                s.name = "tidy".into();
                s
            },
            Some(ACME.into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(other.workspace_id, "default");
    t.rt.shutdown();
}

#[tokio::test]
async fn account_viewer_stays_viewer_with_workspace_admin_entry() {
    let t = Rt::start().await;
    let dot = t.payments_dot().await;
    t.become_(2, "viewer", json!({"payments": "admin"})).await;
    t.rt.hold_runs(true).await;
    assert_eq!(
        refused(t.rt.run_now(&dot).await),
        "Acme Engineering: a Viewer can't run dots. Ask an Admin or Owner."
    );
    assert_eq!(
        refused(t.rt.update_dot(&dot, t.changed(&dot).await).await),
        "Acme Engineering: a Viewer can't create or change dots. Ask an Admin or Owner."
    );
    // And without an entry, the workspace is closed to it.
    t.become_(3, "viewer", json!({"default": "viewer"})).await;
    assert_eq!(refused(t.rt.run_now(&dot).await), OUTSIDE);
    t.rt.shutdown();
}

#[tokio::test]
async fn legacy_and_personal_unchanged() {
    let t = Rt::start().await;
    // An account with no member list: workspaces narrow nothing.
    t.serve(json!({}), None);
    t.join().await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()), None)
            .await
            .unwrap();
    // The mirror knows the workspace (it has no `use` rules): a workspace the mirror lacks
    // refuses every org name, and an account with no member list has only Default of its own.
    t.rt.store()
        .add_workspace(&WorkspaceRow {
            scope: ACME.into(),
            id: "payments".into(),
            name: "Payments".into(),
            use_rules: None,
            created_at: "2026-10-28T09:00:00Z".into(),
        })
        .await
        .unwrap();
    t.rt.store()
        .set_dot_workspace(&dot.id, "payments")
        .await
        .unwrap();
    t.rt.update_dot(&dot.id, t.changed(&dot.id).await)
        .await
        .unwrap();
    assert_eq!(t.run(&dot.id).await, LEVEL);

    // Personal: always Owner, whatever workspace a dot is in.
    let mut s = t.triage();
    s.name = "home".into();
    s.variables = Grants::default();
    let mine = t.rt.create_dot(s, None, None).await.unwrap();
    t.rt.store()
        .set_dot_workspace(&mine.id, "side-projects")
        .await
        .unwrap();
    t.rt.update_dot(&mine.id, t.changed(&mine.id).await)
        .await
        .unwrap();
    let run = t.rt.run_now(&mine.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    t.rt.delete_dot(&mine.id).await.unwrap();
    t.rt.shutdown();
}

#[tokio::test]
async fn create_dot_in_a_workspace_needs_rights_there() {
    let t = Rt::start().await;
    // New dots go to Default for now: a `viewer` entry there refuses them.
    t.serve_list(
        1,
        "member",
        json!({"default": "viewer", "payments": "member"}),
    )
    .await;
    t.join().await;
    assert_eq!(
        refused(t.rt.create_dot(t.triage(), Some(ACME.into()), None).await),
        "Acme Engineering: a Viewer in the Default workspace can't create or change dots. Ask an Admin or Owner."
    );
    // No entry in Default: it is closed.
    t.become_(2, "member", json!({"payments": "member"})).await;
    assert_eq!(
        refused(t.rt.create_dot(t.triage(), Some(ACME.into()), None).await),
        "Acme Engineering: you aren't in the Default workspace. Ask an Admin or Owner to add you."
    );
    // Member there: allowed.
    t.become_(3, "member", json!({"default": "member"})).await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()), None)
            .await
            .unwrap();
    assert_eq!(dot.workspace_id, "default");
    t.rt.shutdown();
}

#[tokio::test]
async fn dot_access_names_what_the_role_refuses() {
    let t = Rt::start().await;
    let dot = t.payments_dot().await;
    assert_eq!(t.rt.dot_access(&dot).await.unwrap(), DotAccess::default());

    t.become_(
        2,
        "member",
        json!({"default": "member", "payments": "viewer"}),
    )
    .await;
    assert_eq!(
        t.rt.dot_access(&dot).await.unwrap(),
        DotAccess {
            edit: Some(payments_viewer_cant("create or change dots")),
            run: Some(payments_viewer_cant("run dots")),
            answer: Some(payments_viewer_cant("answer approvals")),
        }
    );
    // The workspace's own refusals say the same.
    let views = t.rt.workspaces(ACME).await.unwrap();
    let payments = views.iter().find(|w| w.id == "payments").unwrap();
    assert_eq!(payments.refusals, t.rt.dot_access(&dot).await.unwrap());

    t.become_(3, "member", json!({"default": "member"})).await;
    let outside = Some(OUTSIDE.to_string());
    assert_eq!(
        t.rt.dot_access(&dot).await.unwrap(),
        DotAccess {
            edit: outside.clone(),
            run: outside.clone(),
            answer: outside,
        }
    );
    assert!(matches!(
        t.rt.dot_access("ghost").await,
        Err(Error::NotFound(_))
    ));
    t.rt.shutdown();
}
