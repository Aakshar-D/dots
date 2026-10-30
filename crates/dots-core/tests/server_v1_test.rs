//! The `/v1` routes (accounts spec §10): a per-workspace API key lists, triggers and reads the
//! dots and runs of its own workspace only, as far as the person's role there allows at the
//! time of the request. The account serves a signed member list with a `payments` workspace
//! from the fake OpenBao; requests go to the runtime's own server.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, sign, sign_roster, token, ACME, ACME_NAME, ISSUED,
    START, TOKEN,
};
use common::spec;
use dots_core::accounts::{identity, PERSONAL};
use dots_core::model::{DotSpec, NewRun, TriggerKind, WorkspaceMode};
use dots_core::store::ApiKeyRow;
use dots_core::vault::org::Orgs;
use dots_core::vault::MemorySecrets;
use dots_core::{Config, Runtime};
use minisign::KeyPair;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

const BAD_KEY: &str = "That API key isn't valid. Make a new one in Dots.";
const OUTSIDE: &str =
    "Acme Engineering: you aren't in the Payments workspace. Ask an Admin or Owner to add you.";
const NOT_FOUND: &str = "Not found.";

struct Rt {
    dir: TempDir,
    rt: Runtime,
    bao: FakeOpenBao,
    keys: KeyPair,
    base: String,
    http: reqwest::Client,
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
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        let rt = Runtime::start(cfg).await.unwrap();
        let base = format!("http://127.0.0.1:{}", rt.port());
        Rt {
            dir,
            rt,
            bao,
            keys: keys(),
            base,
            http: reqwest::Client::new(),
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
        let me = member("01ME", "me@acme.com", role, &self.me().await, mine);
        self.serve_members(seq, Some(me)).await;
    }

    /// Serves a member list at `seq` with the owner and, when given, this PC's entry.
    async fn serve_members(&self, seq: u64, me: Option<Value>) {
        let mut members = vec![member(
            "01OWNER",
            "owner@acme.com",
            "owner",
            &public_key(&keys()),
            json!({"default": "admin"}),
        )];
        members.extend(me);
        let text = json!({
            "v": 1, "org_id": ACME, "issued_at": "2026-10-28T09:00:00Z", "seq": seq,
            "workspaces": [
                {"id": "default", "name": "Default"},
                {"id": "payments", "name": "Payments"}
            ],
            "members": members,
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

    async fn become_(&self, seq: u64, role: &str, mine: Value) {
        self.serve_list(seq, role, mine).await;
        self.rt.refresh_org(ACME).await.unwrap();
    }

    /// A folder dot called `name`, in `org_id` (None: Personal) and `workspace`.
    async fn dot(&self, name: &str, org_id: Option<&str>, workspace: &str) -> String {
        let mut s: DotSpec = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        let dot = self
            .rt
            .create_dot(s, org_id.map(String::from), None)
            .await
            .unwrap();
        if workspace != "default" {
            self.rt
                .store()
                .set_dot_workspace(&dot.id, workspace)
                .await
                .unwrap();
        }
        dot.id
    }

    /// A new key for `workspace` of `scope`; the key text.
    async fn key(&self, scope: &str, workspace: &str) -> String {
        self.rt
            .create_api_key(scope, workspace, "ci")
            .await
            .unwrap()
            .secret
    }

    async fn get(&self, path: &str, key: Option<&str>) -> (StatusCode, Value) {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some(k) = key {
            req = req.bearer_auth(k);
        }
        answer(req.send().await.unwrap()).await
    }

    async fn trigger(&self, dot_id: &str, key: &str, body: &str) -> (StatusCode, Value) {
        let r = self
            .http
            .post(format!("{}/v1/dots/{dot_id}/trigger", self.base))
            .bearer_auth(key)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        answer(r).await
    }

    async fn runs_of(&self, dot_id: &str) -> usize {
        self.rt
            .store()
            .list_runs(Some(dot_id), 50)
            .await
            .unwrap()
            .len()
    }
}

async fn answer(r: reqwest::Response) -> (StatusCode, Value) {
    let status = r.status();
    let text = r.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn member(id: &str, email: &str, role: &str, key: &str, workspaces: Value) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": workspaces, "added_at": "2026-10-28T09:00:00Z"
    })
}

fn error(text: &str) -> Value {
    json!({ "error": text })
}

fn ids(body: &Value) -> Vec<String> {
    let mut ids: Vec<String> = body["dots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn list_dots_in_the_keys_workspace_only() {
    let t = Rt::start().await;
    t.join().await;
    let pay = t.dot("pay", Some(ACME), "payments").await;
    let pay2 = t.dot("pay2", Some(ACME), "payments").await;
    let _acme_default = t.dot("acme-default", Some(ACME), "default").await;
    let mine = t.dot("mine", None, "default").await;
    let key = t.key(ACME, "payments").await;

    let (status, body) = t.get("/v1/dots", Some(&key)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut want = vec![pay.clone(), pay2];
    want.sort();
    assert_eq!(ids(&body), want);
    let one = body["dots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == pay.as_str())
        .unwrap();
    assert_eq!(
        one,
        &json!({
            "id": pay, "name": "pay", "enabled": true, "org_id": ACME, "workspace_id": "payments"
        })
    );

    let personal = t.key(PERSONAL, "default").await;
    let (status, body) = t.get("/v1/dots", Some(&personal)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&body), vec![mine.clone()]);
    assert_eq!(body["dots"][0]["org_id"], Value::Null);
    t.rt.shutdown();
}

#[tokio::test]
async fn trigger_queues_a_run_and_returns_its_id() {
    let t = Rt::start().await;
    let dot = t.dot("hook", None, "default").await;
    let key = t.key(PERSONAL, "default").await;
    let (status, body) = t.trigger(&dot, &key, r#"{"case": 7}"#).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let run =
        t.rt.store()
            .get_run(body["run_id"].as_str().unwrap())
            .await
            .unwrap();
    assert_eq!(run.dot_id, dot);
    assert_eq!(run.trigger, TriggerKind::Api);
    assert_eq!(run.payload, Some(json!({"case": 7})));

    // An empty or blank body is no payload.
    for blank in ["", "  \n"] {
        let (status, body) = t.trigger(&dot, &key, blank).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let run =
            t.rt.store()
                .get_run(body["run_id"].as_str().unwrap())
                .await
                .unwrap();
        assert_eq!(run.payload, None);
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn read_run_status() {
    let t = Rt::start().await;
    let dot = t.dot("d", None, "default").await;
    let key = t.key(PERSONAL, "default").await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Manual))
            .await
            .unwrap();
    let (status, body) = t.get(&format!("/v1/runs/{}", run.id), Some(&key)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = t.rt.store().get_run(&run.id).await.unwrap();
    assert_eq!(
        body,
        json!({
            "id": run.id, "dot_id": dot, "status": stored.status.as_str(),
            "queued_at": stored.queued_at, "started_at": stored.started_at,
            "ended_at": stored.ended_at, "summary": stored.summary, "error": stored.error
        })
    );

    let (status, body) = t.get("/v1/runs/nope", Some(&key)).await;
    assert_eq!((status, body), (StatusCode::NOT_FOUND, error(NOT_FOUND)));
    let (status, body) = t.trigger("nope", &key, "").await;
    assert_eq!((status, body), (StatusCode::NOT_FOUND, error(NOT_FOUND)));
    t.rt.shutdown();
}

#[tokio::test]
async fn key_is_limited_to_its_workspace() {
    let t = Rt::start().await;
    t.join().await;
    let acme_default = t.dot("acme-default", Some(ACME), "default").await;
    let mine = t.dot("mine", None, "default").await;
    let pay = t.dot("pay", Some(ACME), "payments").await;
    let key = t.key(ACME, "payments").await;
    let other_run =
        t.rt.store()
            .create_run(&NewRun::new(&acme_default, TriggerKind::Manual))
            .await
            .unwrap();
    let personal_run =
        t.rt.store()
            .create_run(&NewRun::new(&mine, TriggerKind::Manual))
            .await
            .unwrap();

    // Another workspace of the same account, and another account (Personal).
    for dot in [&acme_default, &mine] {
        let before = t.runs_of(dot).await;
        let (status, body) = t.trigger(dot, &key, r#"{"x": 1}"#).await;
        assert_eq!((status, body), (StatusCode::NOT_FOUND, error(NOT_FOUND)));
        assert_eq!(t.runs_of(dot).await, before, "nothing queued");
    }
    for run in [&other_run.id, &personal_run.id] {
        let (status, body) = t.get(&format!("/v1/runs/{run}"), Some(&key)).await;
        assert_eq!((status, body), (StatusCode::NOT_FOUND, error(NOT_FOUND)));
    }

    // A Personal key does not reach the account's dot of the same workspace id.
    let personal = t.key(PERSONAL, "default").await;
    let before = t.runs_of(&acme_default).await;
    let (status, body) = t.trigger(&acme_default, &personal, "").await;
    assert_eq!((status, body), (StatusCode::NOT_FOUND, error(NOT_FOUND)));
    assert_eq!(t.runs_of(&acme_default).await, before);

    // Its own workspace works.
    let (status, _) = t.trigger(&pay, &key, "").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    t.rt.shutdown();
}

#[tokio::test]
async fn bad_keys_are_401() {
    let t = Rt::start().await;
    let dot = t.dot("d", None, "default").await;
    let good = t.key(PERSONAL, "default").await;
    let want = (StatusCode::UNAUTHORIZED, error(BAD_KEY));

    // A missing or garbled header.
    assert_eq!(t.get("/v1/dots", None).await, want);
    let r = t
        .http
        .get(format!("{}/v1/dots", t.base))
        .header("authorization", format!("Basic {good}"))
        .send()
        .await
        .unwrap();
    assert_eq!(answer(r).await, want);
    assert_eq!(t.get("/v1/dots", Some("garbage")).await, want);

    // One changed character.
    let mut changed = good.clone().into_bytes();
    let last = changed.len() - 1;
    changed[last] = if changed[last] == b'a' { b'b' } else { b'a' };
    let changed = String::from_utf8(changed).unwrap();
    assert_eq!(t.get("/v1/dots", Some(&changed)).await, want);
    assert_eq!(t.trigger(&dot, &changed, "").await, want);

    // A key whose workspace is not there, though not revoked.
    let orphan = dots_core::accounts::api_keys::new_api_key();
    t.rt.store()
        .add_api_key(&ApiKeyRow {
            id: "orphan".into(),
            scope: PERSONAL.into(),
            workspace_id: "gone".into(),
            name: "orphan".into(),
            prefix: dots_core::accounts::api_keys::key_prefix(&orphan),
            hash: dots_core::util::token_hash(&orphan),
            created_at: dots_core::util::now(),
            last_used_at: None,
            revoked_at: None,
        })
        .await
        .unwrap();
    assert_eq!(t.get("/v1/dots", Some(&orphan)).await, want);

    // A key of an org that is not joined on this PC, though its row and workspace remain.
    let ghost = dots_core::accounts::api_keys::new_api_key();
    t.rt.store()
        .add_workspace(&dots_core::store::WorkspaceRow {
            scope: "ghost-org".into(),
            id: "default".into(),
            name: "Default".into(),
            use_rules: None,
            created_at: dots_core::util::now(),
        })
        .await
        .unwrap();
    t.rt.store()
        .add_api_key(&ApiKeyRow {
            id: "ghost".into(),
            scope: "ghost-org".into(),
            workspace_id: "default".into(),
            name: "ghost".into(),
            prefix: dots_core::accounts::api_keys::key_prefix(&ghost),
            hash: dots_core::util::token_hash(&ghost),
            created_at: dots_core::util::now(),
            last_used_at: None,
            revoked_at: None,
        })
        .await
        .unwrap();
    assert_eq!(t.get("/v1/dots", Some(&ghost)).await, want);

    // A deleted workspace's key.
    let ws = t.rt.personal_workspace_add("Billing").await.unwrap();
    let billing = t.key(PERSONAL, &ws.id).await;
    assert_eq!(t.get("/v1/dots", Some(&billing)).await.0, StatusCode::OK);
    t.rt.personal_workspace_delete(&ws.id).await.unwrap();
    assert_eq!(t.get("/v1/dots", Some(&billing)).await, want);

    // A revoked key.
    let made = t.rt.create_api_key(PERSONAL, "default", "r").await.unwrap();
    assert_eq!(
        t.get("/v1/dots", Some(&made.secret)).await.0,
        StatusCode::OK
    );
    t.rt.revoke_api_key(&made.key.id).await.unwrap();
    assert_eq!(t.get("/v1/dots", Some(&made.secret)).await, want);
    assert_eq!(t.trigger(&dot, &made.secret, "").await, want);
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Manual))
            .await
            .unwrap();
    assert_eq!(
        t.get(&format!("/v1/runs/{}", run.id), Some(&made.secret))
            .await,
        want
    );

    // The good key still works.
    assert_eq!(t.get("/v1/dots", Some(&good)).await.0, StatusCode::OK);
    t.rt.shutdown();
}

#[tokio::test]
async fn role_change_refuses_existing_key() {
    let t = Rt::start().await;
    t.join().await;
    let dot = t.dot("pay", Some(ACME), "payments").await;
    let key = t.key(ACME, "payments").await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Manual))
            .await
            .unwrap();
    let run_path = format!("/v1/runs/{}", run.id);

    // Member -> Viewer in Payments: reading still works, triggering does not.
    t.become_(
        2,
        "member",
        json!({"default": "member", "payments": "viewer"}),
    )
    .await;
    let before = t.runs_of(&dot).await;
    let (status, body) = t.trigger(&dot, &key, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body,
        error(
            "Acme Engineering: a Viewer in the Payments workspace can't run dots. Ask an Admin or Owner."
        )
    );
    assert_eq!(t.runs_of(&dot).await, before, "nothing queued");
    assert_eq!(t.get("/v1/dots", Some(&key)).await.0, StatusCode::OK);
    assert_eq!(t.get(&run_path, Some(&key)).await.0, StatusCode::OK);

    // Removed from the workspace: everything is refused with the outside text.
    t.become_(3, "member", json!({"default": "member"})).await;
    let outside = (StatusCode::FORBIDDEN, error(OUTSIDE));
    assert_eq!(t.trigger(&dot, &key, "").await, outside);
    assert_eq!(t.get("/v1/dots", Some(&key)).await, outside);
    assert_eq!(t.get(&run_path, Some(&key)).await, outside);
    assert_eq!(t.runs_of(&dot).await, before, "nothing queued");
    t.rt.shutdown();
}

#[tokio::test]
async fn disabled_dot_is_409() {
    let t = Rt::start().await;
    let dot = t.dot("d", None, "default").await;
    let mut s = t.rt.store().get_dot(&dot).await.unwrap().spec;
    s.enabled = false;
    t.rt.store().update_dot(&dot, &s).await.unwrap();
    let key = t.key(PERSONAL, "default").await;
    let (status, body) = t.trigger(&dot, &key, "").await;
    assert_eq!(
        (status, body),
        (StatusCode::CONFLICT, error("dot is disabled"))
    );
    assert_eq!(t.runs_of(&dot).await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn last_used_is_updated() {
    let t = Rt::start().await;
    let made =
        t.rt.create_api_key(PERSONAL, "default", "ci")
            .await
            .unwrap();
    let row = || async { t.rt.store().api_key(&made.key.id).await.unwrap().unwrap() };
    assert_eq!(row().await.last_used_at, None);
    assert_eq!(
        t.get("/v1/dots", Some(&made.secret)).await.0,
        StatusCode::OK
    );
    assert!(row().await.last_used_at.is_some());
    t.rt.shutdown();
}

#[tokio::test]
async fn bad_json_is_400() {
    let t = Rt::start().await;
    let dot = t.dot("d", None, "default").await;
    let key = t.key(PERSONAL, "default").await;
    let (status, body) = t.trigger(&dot, &key, "{not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("body must be JSON: "),
        "{body}"
    );
    assert_eq!(t.runs_of(&dot).await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn removed_from_the_account_refuses_existing_key() {
    let t = Rt::start().await;
    t.join().await;
    let dot = t.dot("d", Some(ACME), "default").await;
    let key = t.key(ACME, "default").await;
    assert_eq!(t.get("/v1/dots", Some(&key)).await.0, StatusCode::OK);

    // The member list no longer has this PC.
    t.serve_members(2, None).await;
    t.rt.refresh_org(ACME).await.unwrap();
    let removed = (
        StatusCode::FORBIDDEN,
        error("Acme Engineering: you were removed from it."),
    );
    assert_eq!(t.get("/v1/dots", Some(&key)).await, removed);
    assert_eq!(t.trigger(&dot, &key, "").await, removed);
    assert_eq!(t.runs_of(&dot).await, 0, "nothing queued");
    t.rt.shutdown();
}

#[tokio::test]
async fn oversized_body_is_checked_after_the_key() {
    let t = Rt::start().await;
    let dot = t.dot("d", None, "default").await;
    let key = t.key(PERSONAL, "default").await;
    let big = format!("\"{}\"", "a".repeat(dots_core::server::MAX_WEBHOOK_BODY));
    // No key: 401, not 413.
    let r = t
        .http
        .post(format!("{}/v1/dots/{dot}/trigger", t.base))
        .body(big.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(answer(r).await, (StatusCode::UNAUTHORIZED, error(BAD_KEY)));
    // With a key: 413 in JSON.
    let (status, body) = t.trigger(&dot, &key, &big).await;
    assert_eq!(
        (status, body),
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            error("body must be 256 KB or less")
        )
    );
    assert_eq!(t.runs_of(&dot).await, 0);
    t.rt.shutdown();
}
