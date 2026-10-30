//! Placing and moving dots (accounts design §6.3): a dot is made in a workspace of its account,
//! moved only within the account, and a team's members share its account and workspace.
//! Personal has workspaces of its own, which the person adds, renames and deletes. The Inbox
//! lists teams whose members are outside the team's scope.

mod common;

use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, sign, sign_roster, token, ACME, ACME_NAME, ISSUED,
    START, TOKEN,
};
use common::spec;
use dots_core::accounts::identity;
use dots_core::model::{MemberSpec, TeamSpec};
use dots_core::vault::org::Orgs;
use dots_core::vault::MemorySecrets;
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    _dir: TempDir,
    rt: Runtime,
    bao: FakeOpenBao,
    keys: KeyPair,
}

impl Rt {
    async fn start() -> Rt {
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.clock = FakeClock::new(at(START));
        cfg.secrets = Some(Arc::new(MemorySecrets::new()));
        cfg.claude_path = Some(std::path::PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        let rt = Runtime::start(cfg).await.unwrap();
        Rt {
            _dir: dir,
            rt,
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

    /// Serves a member list at `seq` with the workspaces Default and Payments, an owner who is
    /// not this PC, and this PC as `role` with the workspace entries `mine`.
    async fn serve_list(&self, seq: u64, role: &str, mine: Value) {
        let workspaces = json!([
            {"id": "default", "name": "Default"},
            {"id": "payments", "name": "Payments"}
        ]);
        self.serve_workspaces(seq, role, mine, workspaces).await;
    }

    /// `serve_list` with the workspaces `workspaces`.
    async fn serve_workspaces(&self, seq: u64, role: &str, mine: Value, workspaces: Value) {
        let members = json!([
            member(
                "01OWNER",
                "owner@acme.com",
                "owner",
                &public_key(&keys()),
                json!({"default": "admin"})
            ),
            member("01ME", "me@acme.com", role, &self.me().await, mine),
        ]);
        let text = json!({
            "v": 1, "org_id": ACME, "issued_at": "2026-10-28T09:00:00Z", "seq": seq,
            "workspaces": workspaces,
            "members": members
        })
        .to_string();
        let mut extra = json!({"v": 2, "roster": "required", "admins": []});
        extra["plain"] = json!(["DOTS_TEST_LEVEL"]);
        extra["offline_hours"] = json!(0);
        let signed = sign(&self.keys, &policy(ACME, ISSUED, extra));
        self.bao
            .put_env("secret", "dots", json!({"DOTS_TEST_LEVEL": "x"}));
        self.bao.put_policy("secret", "dots", &signed);
        self.bao
            .put_roster("secret", "dots", &sign_roster(&self.keys, &text));
    }

    /// Joins Acme with this PC as `role` and `mine`.
    async fn join_as(&self, role: &str, mine: Value) {
        self.serve_list(1, role, mine).await;
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        self.orgs().join(&code, Some(token())).await.unwrap();
    }

    async fn become_(&self, seq: u64, role: &str, mine: Value) {
        self.serve_list(seq, role, mine).await;
        self.rt.refresh_org(ACME).await.unwrap();
    }

    async fn personal_dot(&self, name: &str, workspace: Option<&str>) -> String {
        self.rt
            .create_dot(spec(name), None, workspace.map(String::from))
            .await
            .unwrap()
            .id
    }
}

fn member(id: &str, email: &str, role: &str, key: &str, workspaces: Value) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": workspaces, "added_at": "2026-10-28T09:00:00Z"
    })
}

fn team(name: &str, workspace: &str, dots: &[&str]) -> TeamSpec {
    TeamSpec {
        name: name.into(),
        description: String::new(),
        members: dots
            .iter()
            .map(|d| MemberSpec {
                dot_id: d.to_string(),
                role: String::new(),
            })
            .collect(),
        org_id: None,
        workspace_id: workspace.into(),
    }
}

fn invalid<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::Invalid(text)) => text,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn not_found<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::NotFound(text)) => text,
        other => panic!("expected NotFound, got {other:?}"),
    }
}

fn refused<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::NotAllowed(text)) => text,
        other => panic!("expected NotAllowed, got {other:?}"),
    }
}

const VIEWER_PAYMENTS: &str =
    "Acme Engineering: a Viewer in the Payments workspace can't create or change dots. Ask an Admin or Owner.";

#[tokio::test]
async fn create_dot_in_workspace() {
    let t = Rt::start().await;
    let ops = t.rt.personal_workspace_add("Ops").await.unwrap();
    assert_eq!(ops.id, "ops");
    let in_ops = t.personal_dot("a", Some("ops")).await;
    let plain = t.personal_dot("b", None).await;
    assert_eq!(
        t.rt.store().get_dot(&in_ops).await.unwrap().workspace_id,
        "ops"
    );
    assert_eq!(
        t.rt.store().get_dot(&plain).await.unwrap().workspace_id,
        "default"
    );
    assert_eq!(
        not_found(t.rt.create_dot(spec("c"), None, Some("nope".into())).await),
        "No workspace 'nope' in Personal."
    );

    // In an account: a workspace of it, and only where the role allows.
    t.join_as("member", json!({"default": "member", "payments": "viewer"}))
        .await;
    let dot =
        t.rt.create_dot(spec("pay"), Some(ACME.into()), Some("default".into()))
            .await
            .unwrap();
    assert_eq!(dot.workspace_id, "default");
    assert_eq!(
        not_found(
            t.rt.create_dot(spec("x"), Some(ACME.into()), Some("ops".into()))
                .await
        ),
        "No workspace 'ops' in Acme Engineering."
    );
    assert_eq!(
        refused(
            t.rt.create_dot(spec("y"), Some(ACME.into()), Some("payments".into()))
                .await
        ),
        VIEWER_PAYMENTS
    );
    t.become_(
        2,
        "member",
        json!({"default": "member", "payments": "member"}),
    )
    .await;
    let paid =
        t.rt.create_dot(spec("y"), Some(ACME.into()), Some("payments".into()))
            .await
            .unwrap();
    assert_eq!(paid.workspace_id, "payments");
    t.rt.shutdown();
}

#[tokio::test]
async fn move_respects_team_and_account() {
    let t = Rt::start().await;
    t.rt.personal_workspace_add("Ops").await.unwrap();
    let a = t.personal_dot("alpha", None).await;
    let moved = t.rt.move_dot(&a, "ops").await.unwrap();
    assert_eq!(moved.workspace_id, "ops");
    assert_eq!(t.rt.store().get_dot(&a).await.unwrap().workspace_id, "ops");
    assert_eq!(
        not_found(t.rt.move_dot(&a, "nope").await),
        "No workspace 'nope' in Personal."
    );

    // A dot in a team stays where the team is.
    let b = t.personal_dot("beta", None).await;
    t.rt.teams()
        .create(&team("crew", "default", &[&b]))
        .await
        .unwrap();
    assert_eq!(
        invalid(t.rt.move_dot(&b, "ops").await),
        "beta is in the team crew. Take it out of the team first."
    );

    // Another account's workspace is not one of this dot's.
    t.join_as("member", json!({"default": "member", "payments": "member"}))
        .await;
    let acme =
        t.rt.create_dot(spec("pay"), Some(ACME.into()), None)
            .await
            .unwrap();
    assert_eq!(
        not_found(t.rt.move_dot(&acme.id, "ops").await),
        "No workspace 'ops' in Acme Engineering."
    );
    let there = t.rt.move_dot(&acme.id, "payments").await.unwrap();
    assert_eq!(there.workspace_id, "payments");
    t.rt.shutdown();
}

#[tokio::test]
async fn move_needs_member_rights_in_both() {
    let t = Rt::start().await;
    t.join_as("member", json!({"default": "member", "payments": "member"}))
        .await;
    let dot =
        t.rt.create_dot(spec("pay"), Some(ACME.into()), None)
            .await
            .unwrap()
            .id;

    // A Viewer in the target: refused there.
    t.become_(
        2,
        "member",
        json!({"default": "member", "payments": "viewer"}),
    )
    .await;
    assert_eq!(
        refused(t.rt.move_dot(&dot, "payments").await),
        VIEWER_PAYMENTS
    );
    // Not in the target at all.
    t.become_(3, "member", json!({"default": "member"})).await;
    assert_eq!(
        refused(t.rt.move_dot(&dot, "payments").await),
        "Acme Engineering: you aren't in the Payments workspace. Ask an Admin or Owner to add you."
    );
    // Member in the target, but a Viewer in the source.
    t.become_(
        4,
        "member",
        json!({"default": "viewer", "payments": "member"}),
    )
    .await;
    assert_eq!(
        refused(t.rt.move_dot(&dot, "payments").await),
        "Acme Engineering: a Viewer in the Default workspace can't create or change dots. Ask an Admin or Owner."
    );
    // Member in both.
    t.become_(
        5,
        "member",
        json!({"default": "member", "payments": "member"}),
    )
    .await;
    assert_eq!(
        t.rt.move_dot(&dot, "payments").await.unwrap().workspace_id,
        "payments"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn team_members_share_workspace() {
    let t = Rt::start().await;
    t.rt.personal_workspace_add("Ops").await.unwrap();
    let a = t.personal_dot("alpha", Some("ops")).await;
    let b = t.personal_dot("beta", None).await;
    assert_eq!(
        invalid(t.rt.teams().create(&team("crew", "ops", &[&a, &b])).await),
        "dot 'beta' is in another workspace than the team"
    );
    let made =
        t.rt.teams()
            .create(&team("crew", "ops", &[&a]))
            .await
            .unwrap();
    assert_eq!(made.workspace_id, "ops");
    assert_eq!(
        invalid(
            t.rt.teams()
                .update(&made.id, &team("crew", "ops", &[&a, &b]))
                .await
        ),
        "dot 'beta' is in another workspace than the team"
    );
    // The workspace is fixed once the team is made, like its account.
    let kept =
        t.rt.teams()
            .update(&made.id, &team("crew", "default", &[&a]))
            .await
            .unwrap();
    assert_eq!(kept.workspace_id, "ops");
    assert_eq!(
        not_found(t.rt.teams().create(&team("other", "nope", &[])).await),
        "No workspace 'nope' in Personal."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn personal_workspaces_add_rename_delete_moves_dots() {
    let t = Rt::start().await;
    let ws =
        t.rt.personal_workspace_add("  Side Projects ")
            .await
            .unwrap();
    assert_eq!(
        (ws.id.as_str(), ws.name.as_str()),
        ("side-projects", "Side Projects")
    );
    assert!(ws.can_manage && ws.my_role.is_none());
    // A second with the same name gets another id.
    assert_eq!(
        t.rt.personal_workspace_add("Side Projects")
            .await
            .unwrap()
            .id,
        "side-projects-2"
    );
    assert_eq!(
        t.rt.personal_workspace_add("Default").await.unwrap().id,
        "default-2"
    );
    assert!(invalid(t.rt.personal_workspace_add("   ").await).contains("name"));

    t.rt.personal_workspace_rename("side-projects", "Hobby")
        .await
        .unwrap();
    let names: Vec<String> =
        t.rt.workspaces("personal")
            .await
            .unwrap()
            .into_iter()
            .map(|w| format!("{}={}", w.id, w.name))
            .collect();
    assert_eq!(
        names,
        [
            "default=Default",
            "default-2=Default",
            "side-projects=Hobby",
            "side-projects-2=Side Projects"
        ]
    );
    assert!(matches!(
        t.rt.personal_workspace_rename("nope", "X").await,
        Err(Error::NotFound(_))
    ));

    let dot = t.personal_dot("a", Some("side-projects")).await;
    t.rt.personal_workspace_delete("side-projects")
        .await
        .unwrap();
    assert_eq!(
        t.rt.store().get_dot(&dot).await.unwrap().workspace_id,
        "default"
    );
    assert!(matches!(
        t.rt.personal_workspace_delete("side-projects").await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        invalid(t.rt.personal_workspace_delete("default").await),
        "The Default workspace can't be deleted."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn inbox_lists_team_scope_issues() {
    let t = Rt::start().await;
    assert!(t.rt.inbox().await.unwrap().team_scope.is_empty());
    t.rt.personal_workspace_add("Ops").await.unwrap();
    let a = t.personal_dot("alpha", None).await;
    let b = t.personal_dot("beta", None).await;
    let crew =
        t.rt.teams()
            .create(&team("crew", "default", &[&a, &b]))
            .await
            .unwrap();
    // Data from before the rule: a member dot placed elsewhere.
    t.rt.store().set_dot_workspace(&b, "ops").await.unwrap();

    let issues = t.rt.inbox().await.unwrap().team_scope;
    assert_eq!(issues.len(), 1);
    // It waits for the person like the rest of the Inbox.
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    let issue = &issues[0];
    assert_eq!(
        (
            issue.team_id.as_str(),
            issue.team_name.as_str(),
            issue.dot_id.as_str(),
            issue.dot_name.as_str()
        ),
        (crew.id.as_str(), "crew", b.as_str(), "beta")
    );
    assert_eq!(
        issue.reason,
        "beta is in another workspace than the crew team."
    );

    t.rt.store().set_dot_workspace(&b, "default").await.unwrap();
    assert!(t.rt.inbox().await.unwrap().team_scope.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn leaving_puts_the_accounts_dots_in_default_and_a_rejoin_keeps_them_there() {
    let t = Rt::start().await;
    t.join_as("member", json!({"default": "member", "payments": "member"}))
        .await;
    let dot =
        t.rt.create_dot(spec("pay"), Some(ACME.into()), Some("payments".into()))
            .await
            .unwrap();
    let crew =
        t.rt.teams()
            .create(&TeamSpec {
                name: "crew".into(),
                description: String::new(),
                members: Vec::new(),
                org_id: Some(ACME.into()),
                workspace_id: "payments".into(),
            })
            .await
            .unwrap();

    t.rt.remove_org(ACME).await.unwrap();
    let left = t.rt.store().get_dot(&dot.id).await.unwrap();
    assert_eq!(left.org_id.as_deref(), Some(ACME));
    assert_eq!(left.workspace_id, "default");
    assert_eq!(
        t.rt.store().get_team(&crew.id).await.unwrap().workspace_id,
        "default"
    );

    // A dot left before leaving moved it, still in Payments, is moved when the account is
    // joined again on a list without Payments: the mirror never saw that workspace.
    t.rt.store()
        .set_dot_workspace(&dot.id, "payments")
        .await
        .unwrap();
    t.serve_workspaces(
        1,
        "member",
        json!({"default": "member"}),
        json!([{"id": "default", "name": "Default"}]),
    )
    .await;
    let code = code(ACME, ACME_NAME, openbao(&t.bao.address(), "dots"), &t.keys);
    t.orgs().join(&code, Some(token())).await.unwrap();
    assert_eq!(
        t.rt.store().get_dot(&dot.id).await.unwrap().workspace_id,
        "default"
    );
    t.rt.shutdown();
}
