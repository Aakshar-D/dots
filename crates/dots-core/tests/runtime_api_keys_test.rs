//! API keys per workspace (accounts spec §10): made, listed and revoked in the runtime behind the
//! workspace permission rule; gone with their workspace or account.

mod common;

use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, sign, sign_roster, token, ACME, ACME_NAME, ISSUED,
    START, TOKEN,
};
use dots_core::accounts::api_keys::{key_prefix, looks_like_key, ApiKeyView};
use dots_core::accounts::{identity, PERSONAL};
use dots_core::util::token_hash;
use dots_core::vault::org::Orgs;
use dots_core::vault::MemorySecrets;
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
use tempfile::TempDir;

const OUTSIDE: &str =
    "Acme Engineering: you aren't in the Payments workspace. Ask an Admin or Owner to add you.";

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

    /// Serves a member list at `seq` with the workspaces `workspaces` (ids; `payments` is
    /// called Payments), an owner who is not this PC, and this PC as `role` with `mine`.
    async fn serve_list(&self, seq: u64, workspaces: &[&str], role: &str, mine: Value) {
        let list: Vec<Value> = workspaces
            .iter()
            .map(|id| {
                let name = if *id == "payments" {
                    "Payments"
                } else {
                    "Default"
                };
                json!({"id": id, "name": name})
            })
            .collect();
        let text = json!({
            "v": 1, "org_id": ACME, "issued_at": "2026-10-28T09:00:00Z", "seq": seq,
            "workspaces": list,
            "members": [
                member("01OWNER", "owner@acme.com", "owner", &public_key(&keys()),
                       json!({"default": "admin"})),
                member("01ME", "me@acme.com", role, &self.me().await, mine),
            ]
        })
        .to_string();
        let policy_text = sign(
            &self.keys,
            &policy(
                ACME,
                ISSUED,
                json!({"v": 2, "roster": "required", "admins": [], "offline_hours": 0, "plain": ["DOTS_TEST_LEVEL"]}),
            ),
        );
        self.bao
            .put_env("secret", "dots", json!({"DOTS_TEST_LEVEL": "level"}));
        self.bao.put_policy("secret", "dots", &policy_text);
        self.bao
            .put_roster("secret", "dots", &sign_roster(&self.keys, &text));
    }

    /// Joins Acme as a Member of Default and Payments.
    async fn join(&self) {
        self.serve_list(
            1,
            &["default", "payments"],
            "member",
            json!({"default": "member", "payments": "member"}),
        )
        .await;
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        self.orgs().join(&code, Some(token())).await.unwrap();
    }

    async fn become_(&self, seq: u64, workspaces: &[&str], role: &str, mine: Value) {
        self.serve_list(seq, workspaces, role, mine).await;
        self.rt.refresh_org(ACME).await.unwrap();
    }
}

fn member(id: &str, email: &str, role: &str, key: &str, workspaces: Value) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": workspaces, "added_at": "2026-10-28T09:00:00Z"
    })
}

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

fn names(keys: Vec<ApiKeyView>) -> Vec<String> {
    let mut n: Vec<String> = keys.into_iter().map(|k| k.name).collect();
    n.sort();
    n
}

#[tokio::test]
async fn create_shows_the_secret_once_and_lists_without_it() {
    let t = Rt::start().await;
    let made =
        t.rt.create_api_key(PERSONAL, "default", "ci")
            .await
            .unwrap();
    assert!(looks_like_key(&made.secret));
    assert_eq!(made.key.prefix, key_prefix(&made.secret));
    assert_eq!(made.key.name, "ci");
    assert_eq!(made.key.workspace_name, "Default");
    assert_eq!(made.key.revoked_at, None);

    let listed = t.rt.api_keys(PERSONAL).await.unwrap();
    assert_eq!(listed, vec![made.key.clone()]);
    assert!(!serde_json::to_string(&listed)
        .unwrap()
        .contains(&made.secret));

    let row = t.rt.store().api_key(&made.key.id).await.unwrap().unwrap();
    assert_eq!(row.hash, token_hash(&made.secret));
    assert_ne!(row.hash, made.secret);
    // The key's text is in no file Dots wrote.
    t.rt.shutdown();
    for file in files_under(t._dir.path()) {
        let bytes = std::fs::read(&file).unwrap();
        assert!(
            !bytes
                .windows(made.secret.len())
                .any(|w| w == made.secret.as_bytes()),
            "{}",
            file.display()
        );
    }
}

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[tokio::test]
async fn personal_keys_allowed() {
    let t = Rt::start().await;
    let ws = t.rt.personal_workspace_add("Billing").await.unwrap();
    assert_eq!(ws.refusals.create_api_keys, None);
    let made =
        t.rt.create_api_key(PERSONAL, &ws.id, "billing bot")
            .await
            .unwrap();
    assert_eq!(made.key.workspace_name, "Billing");
    let revoked = t.rt.revoke_api_key(&made.key.id).await.unwrap();
    assert!(revoked.revoked_at.is_some());
    // Revoking again keeps the first time.
    let again = t.rt.revoke_api_key(&made.key.id).await.unwrap();
    assert_eq!(again.revoked_at, revoked.revoked_at);
    assert!(matches!(
        t.rt.revoke_api_key("nope").await,
        Err(Error::NotFound(text)) if text == "That API key isn't on this PC any more."
    ));
    t.rt.shutdown();
}

#[tokio::test]
async fn name_rules() {
    let t = Rt::start().await;
    let text = "Give the key a name (up to 64 characters).";
    for bad in ["", "   ", &"x".repeat(65)] {
        match t.rt.create_api_key(PERSONAL, "default", bad).await {
            Err(Error::Invalid(got)) => assert_eq!(got, text),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }
    let made =
        t.rt.create_api_key(PERSONAL, "default", "  ci  ")
            .await
            .unwrap();
    assert_eq!(made.key.name, "ci");
    t.rt.create_api_key(PERSONAL, "default", &"x".repeat(64))
        .await
        .unwrap();
    match t.rt.create_api_key(PERSONAL, "nope", "ci").await {
        Err(Error::NotFound(got)) => assert_eq!(got, "No workspace 'nope' in Personal."),
        other => panic!("expected NotFound, got {other:?}"),
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn viewer_cannot_create_but_can_revoke() {
    let t = Rt::start().await;
    t.join().await;
    let made = t.rt.create_api_key(ACME, "payments", "ci").await.unwrap();
    t.become_(
        2,
        &["default", "payments"],
        "member",
        json!({"default": "member", "payments": "viewer"}),
    )
    .await;

    let text = payments_viewer_cant("create API keys");
    assert_eq!(
        refused(t.rt.create_api_key(ACME, "payments", "again").await),
        text
    );
    // The list still shows the key.
    let listed = t.rt.api_keys(ACME).await.unwrap();
    assert_eq!(listed.len(), 1);
    // Revoking only reduces access, so any role may do it.
    let revoked = t.rt.revoke_api_key(&made.key.id).await.unwrap();
    assert!(revoked.revoked_at.is_some());
    let row = t.rt.store().api_key(&made.key.id).await.unwrap().unwrap();
    assert!(row.revoked_at.is_some());
    let views = t.rt.workspaces(ACME).await.unwrap();
    let payments = views.iter().find(|w| w.id == "payments").unwrap();
    assert_eq!(
        payments.refusals.create_api_keys.as_deref(),
        Some(text.as_str())
    );
    let default = views.iter().find(|w| w.id == "default").unwrap();
    assert_eq!(default.refusals.create_api_keys, None);
    t.rt.shutdown();
}

#[tokio::test]
async fn outside_workspace_cannot_create() {
    let t = Rt::start().await;
    t.join().await;
    let made = t.rt.create_api_key(ACME, "payments", "ci").await.unwrap();
    t.become_(
        2,
        &["default", "payments"],
        "member",
        json!({"default": "member"}),
    )
    .await;
    assert_eq!(
        refused(t.rt.create_api_key(ACME, "payments", "again").await),
        OUTSIDE
    );
    // Revoking only reduces access, so even a key out of sight may be revoked.
    assert!(t.rt.revoke_api_key(&made.key.id).await.is_ok());
    t.rt.shutdown();
}

#[tokio::test]
async fn list_shows_only_visible_workspaces() {
    let t = Rt::start().await;
    t.join().await;
    t.rt.create_api_key(ACME, "payments", "pay").await.unwrap();
    t.rt.create_api_key(ACME, "default", "def").await.unwrap();
    t.rt.create_api_key(PERSONAL, "default", "mine")
        .await
        .unwrap();
    assert_eq!(names(t.rt.api_keys(ACME).await.unwrap()), ["def", "pay"]);
    assert_eq!(names(t.rt.api_keys(PERSONAL).await.unwrap()), ["mine"]);

    // Out of Payments: its key is not listed, though the key and workspace still exist.
    t.become_(
        2,
        &["default", "payments"],
        "member",
        json!({"default": "member"}),
    )
    .await;
    assert_eq!(names(t.rt.api_keys(ACME).await.unwrap()), ["def"]);
    // An account not joined has none.
    assert!(t.rt.api_keys("not-joined").await.unwrap().is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn workspace_gone_revokes_keys() {
    let t = Rt::start().await;
    // Deleting a Personal workspace revokes its keys, and only its keys.
    let ws = t.rt.personal_workspace_add("Billing").await.unwrap();
    let billing = t.rt.create_api_key(PERSONAL, &ws.id, "b").await.unwrap();
    let kept = t.rt.create_api_key(PERSONAL, "default", "d").await.unwrap();
    t.rt.personal_workspace_delete(&ws.id).await.unwrap();
    let row = |id: String| {
        let store = t.rt.store().clone();
        async move { store.api_key(&id).await.unwrap().unwrap() }
    };
    assert!(row(billing.key.id.clone()).await.revoked_at.is_some());
    assert!(row(kept.key.id.clone()).await.revoked_at.is_none());

    // A workspace the member list drops revokes its keys.
    t.join().await;
    let pay = t.rt.create_api_key(ACME, "payments", "p").await.unwrap();
    let def = t.rt.create_api_key(ACME, "default", "d").await.unwrap();
    t.become_(2, &["default"], "member", json!({"default": "member"}))
        .await;
    assert!(row(pay.key.id.clone()).await.revoked_at.is_some());
    assert!(row(def.key.id.clone()).await.revoked_at.is_none());

    // Leaving the account deletes the account's keys, not Personal's.
    t.rt.remove_org(ACME).await.unwrap();
    assert!(t.rt.store().api_keys(ACME).await.unwrap().is_empty());
    assert!(t.rt.store().api_key(&def.key.id).await.unwrap().is_none());
    assert_eq!(t.rt.store().api_keys(PERSONAL).await.unwrap().len(), 2);
    t.rt.shutdown();
}
