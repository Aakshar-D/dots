//! A run of a dot in an org (env vault spec §6.4 and §8), through the runtime and the fake
//! claude CLI: the org must still be joined, and a stale org is fetched before the run
//! resolves its variables. The orgs keep no offline copy (`offline_hours: 0`), so nothing here
//! writes a protected file.

mod common;

use std::path::PathBuf;

use chrono::Duration;
use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{at, code, keys, openbao, policy, sign, token, ACME, ACME_NAME, START, TOKEN};
use common::{spec, wait_status};
use dots_core::model::{DotSpec, RunStatus, WorkspaceMode};
use dots_core::vault::org::Orgs;
use dots_core::vault::Grants;
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
use tempfile::TempDir;

const REMOVED: &str = "dot 'triage': its org 'acme-eng' was removed";
const LEVEL_ONE: &str = "org-plain-level-one-4e1c";
const LEVEL_TWO: &str = "org-plain-level-two-93ab";

struct Rt {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
    clock: std::sync::Arc<FakeClock>,
    bao: FakeOpenBao,
    keys: KeyPair,
}

impl Rt {
    async fn start() -> Rt {
        let dir = tempfile::tempdir().unwrap();
        let args_out = dir.path().join("args.json");
        let clock = FakeClock::new(at(START));
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.clock = clock.clone();
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        cfg.claude_env.push((
            "FAKE_CLAUDE_ARGS_OUT".into(),
            args_out.to_string_lossy().to_string(),
        ));
        let rt = Runtime::start(cfg).await.unwrap();
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        Rt {
            dir,
            rt,
            args_out,
            clock,
            bao,
            keys: keys(),
        }
    }

    fn orgs(&self) -> &Orgs {
        self.rt.vault().orgs().expect("the runtime's orgs")
    }

    /// Serves Acme's `DOTS_TEST_LEVEL`, plain, with no offline copy.
    fn serve(&self, level: &str) {
        let extra = json!({"plain": ["DOTS_TEST_LEVEL"], "offline_hours": 0});
        let signed = sign(&self.keys, &policy(ACME, "2026-10-22T08:00:00Z", extra));
        self.bao
            .put_env("secret", "dots", json!({"DOTS_TEST_LEVEL": level}));
        self.bao.put_policy("secret", "dots", &signed);
    }

    async fn join_acme(&self) {
        self.serve(LEVEL_ONE);
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        self.orgs().join(&code, Some(token())).await.unwrap();
    }

    fn env_reads(&self) -> usize {
        self.bao.count("/v1/secret/data/dots/env")
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

#[tokio::test]
async fn a_run_of_a_dot_whose_org_was_removed_fails_at_start_and_spawns_nothing() {
    let t = Rt::start().await;
    t.join_acme().await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    t.orgs().remove(ACME).await.unwrap();

    let run = t.rt.run_now(&dot.id).await.unwrap();
    let run = wait_status(t.rt.store(), &run.id, RunStatus::Failed, 15).await;
    assert_eq!(run.error.as_deref(), Some(REMOVED));

    // A webhook's run takes the same path.
    let token = t.rt.regenerate_webhook_token(&dot.id).await.unwrap();
    let resp: Value = reqwest::Client::new()
        .post(t.rt.webhook_url(&dot).unwrap())
        .bearer_auth(&token)
        .json(&json!({"event": "build_failed"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hooked = resp["run_id"].as_str().unwrap();
    let hooked = wait_status(t.rt.store(), hooked, RunStatus::Failed, 15).await;
    assert_eq!(hooked.error.as_deref(), Some(REMOVED));

    // The CLI never started.
    assert!(!t.args_out.exists());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_run_start_fetches_a_stale_org_once_and_not_a_fresh_one() {
    let t = Rt::start().await;
    t.join_acme().await;
    assert_eq!(t.env_reads(), 1);
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();

    // Fresh: the run uses what the join fetched, even after the value changed at the source.
    assert_eq!(t.run(&dot.id).await, LEVEL_ONE);
    t.serve(LEVEL_TWO);
    assert_eq!(t.run(&dot.id).await, LEVEL_ONE);
    assert_eq!(t.env_reads(), 1);

    // Stale after `refresh_minutes`: the next start fetches first, once.
    t.clock
        .set(at(START) + Duration::minutes(15) + Duration::seconds(1));
    assert_eq!(t.run(&dot.id).await, LEVEL_TWO);
    assert_eq!(t.env_reads(), 2);
    assert_eq!(t.run(&dot.id).await, LEVEL_TWO);
    assert_eq!(t.env_reads(), 2);
    t.rt.shutdown();
}

#[tokio::test]
async fn saving_a_dot_whose_org_was_removed_is_refused() {
    let t = Rt::start().await;
    t.join_acme().await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    t.orgs().remove(ACME).await.unwrap();

    let mut changed = dot.spec.clone();
    changed.instructions = "something else".into();
    let refused = t.rt.update_dot(&dot.id, changed).await.unwrap_err();
    assert!(
        matches!(&refused, Error::Conflict(text) if text == REMOVED),
        "{refused:?}"
    );
    let kept = t.rt.store().get_dot(&dot.id).await.unwrap();
    assert_eq!(kept.spec, dot.spec);

    // A Personal dot saves as before.
    let mut mine = t.triage();
    mine.name = "mine".into();
    let mine = t.rt.create_dot(mine, None).await.unwrap();
    t.rt.update_dot(&mine.id, mine.spec.clone()).await.unwrap();
    t.rt.shutdown();
}
