//! The current account and workspace (accounts design §9), through the runtime: personal until
//! a joined org is chosen, and personal again when that org is removed.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{at, code, keys, openbao, policy, sign, token, ACME, ACME_NAME, START, TOKEN};
use dots_core::accounts::{CurrentScope, DEFAULT_WORKSPACE, PERSONAL};
use dots_core::vault::MemorySecrets;
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::json;
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
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        let rt = Runtime::start(cfg).await.unwrap();
        Rt {
            _dir: dir,
            rt,
            bao,
            keys: keys(),
        }
    }

    async fn join_acme(&self) {
        let extra = json!({"plain": ["DOTS_TEST_LEVEL"], "offline_hours": 0});
        let signed = sign(&self.keys, &policy(ACME, "2026-10-22T08:00:00Z", extra));
        self.bao
            .put_env("secret", "dots", json!({"DOTS_TEST_LEVEL": "one"}));
        self.bao.put_policy("secret", "dots", &signed);
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        self.rt.orgs().join(&code, Some(token())).await.unwrap();
    }
}

fn personal() -> CurrentScope {
    CurrentScope {
        account: PERSONAL.into(),
        workspace: DEFAULT_WORKSPACE.into(),
    }
}

#[tokio::test]
async fn default_is_personal() {
    let t = Rt::start().await;
    assert_eq!(t.rt.current_scope().await.unwrap(), personal());
}

#[tokio::test]
async fn set_and_read_back_joined_org() {
    let t = Rt::start().await;
    t.join_acme().await;
    let set = t.rt.set_current_scope(ACME, "default").await.unwrap();
    assert_eq!(set.account, ACME);
    assert_eq!(t.rt.current_scope().await.unwrap(), set);
    let back = t.rt.set_current_scope(PERSONAL, "default").await.unwrap();
    assert_eq!(back, personal());
}

#[tokio::test]
async fn unknown_account_refused() {
    let t = Rt::start().await;
    let err =
        t.rt.set_current_scope("ghost", "default")
            .await
            .unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == "no account 'ghost' on this PC"));
    assert_eq!(t.rt.current_scope().await.unwrap(), personal());
}

#[tokio::test]
async fn other_workspace_refused() {
    let t = Rt::start().await;
    let err = t.rt.set_current_scope(PERSONAL, "web").await.unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == "no workspace 'web' in this account"));
}

#[tokio::test]
async fn falls_back_after_org_removed() {
    let t = Rt::start().await;
    t.join_acme().await;
    t.rt.set_current_scope(ACME, "default").await.unwrap();
    t.rt.orgs().remove(ACME).await.unwrap();
    assert_eq!(t.rt.current_scope().await.unwrap().account, PERSONAL);
}
