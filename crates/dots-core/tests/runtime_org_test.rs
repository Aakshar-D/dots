//! A run of a dot in an org (env vault spec §6.4 and §8), through the runtime and the fake
//! claude CLI: the org must still be joined, and a stale org is fetched before the run
//! resolves its variables. The orgs keep no offline copy (`offline_hours: 0`), so nothing here
//! writes a protected file.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::Duration;
use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, sign, sops_at, token, ACME, ACME_NAME, START, TOKEN,
};
use common::{spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::model::{DotSpec, RunStatus, WorkspaceMode};
use dots_core::vault::org::{OrgStatus, Orgs};
use dots_core::vault::{GrantState, Grants, MemorySecrets, ValueSource};
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
        let clock = FakeClock::new(at(START));
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.clock = clock.clone();
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
            clock,
            bao,
            keys,
            secrets,
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
        self.orgs()
            .join(&self.acme_code(), Some(token()))
            .await
            .unwrap();
    }

    fn acme_code(&self) -> String {
        code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        )
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

    // The person's own run is refused before it is queued.
    match t.rt.run_now(&dot.id).await {
        Err(Error::Conflict(text)) => assert_eq!(text, REMOVED),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert!(t
        .rt
        .store()
        .list_runs(Some(&dot.id), 10)
        .await
        .unwrap()
        .is_empty());

    // A webhook's run fails at its start.
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

/// Waits (at most 10 real seconds) for `check` to hold.
async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..1000 {
        if check() {
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    panic!("{what} did not happen");
}

/// The runtime's org commands keep one timer per joined org: a join starts it, an Update and a
/// Refresh keep it, a remove stops it, and the runtime's shutdown stops every one. Each change
/// is published as `OrgUpdated`.
#[tokio::test]
async fn the_runtime_keeps_one_timer_per_joined_org() {
    let t = Rt::start().await;
    let mut events = t.rt.subscribe();
    let timers = t.rt.org_timers();
    t.serve(LEVEL_ONE);
    t.rt.join_org(&t.acme_code(), Some(token())).await.unwrap();
    assert_eq!(timers.running(), [ACME]);
    assert_eq!(t.env_reads(), 1);
    let updated = |event: Result<RuntimeEvent, _>| match event {
        Ok(RuntimeEvent::OrgUpdated { org_id }) => Some(org_id),
        _ => None,
    };
    assert_eq!(updated(events.try_recv()).as_deref(), Some(ACME));

    // Its timer fetches after `refresh_minutes`.
    t.clock.set(at(START) + Duration::minutes(15));
    eventually("the timer's fetch", || t.env_reads() == 2).await;

    t.rt.update_org(ACME, &t.acme_code(), None).await.unwrap();
    t.rt.refresh_org(ACME).await.unwrap();
    assert_eq!(t.env_reads(), 4);
    assert_eq!(timers.running(), [ACME]);

    t.rt.remove_org(ACME).await.unwrap();
    assert!(timers.running().is_empty());
    let mut said = Vec::new();
    while let Some(org_id) = updated(events.try_recv()) {
        said.push(org_id);
    }
    assert!(
        said.len() >= 3 && said.iter().all(|id| id == ACME),
        "{said:?}"
    );
    t.clock.set(at(START) + Duration::hours(2));
    tokio::time::sleep(StdDuration::from_millis(200)).await;
    assert_eq!(t.env_reads(), 4, "a removed org is not fetched");
    assert!(matches!(
        t.rt.remove_org(ACME).await,
        Err(Error::NotFound(_))
    ));

    t.rt.join_org(&t.acme_code(), Some(token())).await.unwrap();
    assert_eq!(timers.running(), [ACME]);
    t.rt.shutdown();
    eventually("the timers' end", || timers.running().is_empty()).await;
}

/// A restarted runtime fetches every joined org once, and starts its timer.
#[tokio::test]
async fn a_restarted_runtime_fetches_each_org_and_starts_its_timer() {
    let t = Rt::start().await;
    t.join_acme().await;
    assert!(t.rt.close(StdDuration::from_secs(10)).await);
    let Rt {
        dir,
        rt,
        bao,
        keys,
        secrets,
        ..
    } = t;
    drop(rt);
    let t = Rt::on(dir, secrets, bao, keys).await;
    let timers = t.rt.org_timers();
    eventually("the start-up fetch", || {
        t.env_reads() == 2 && !timers.starting_up()
    })
    .await;
    assert_eq!(timers.running(), [ACME]);
    t.rt.shutdown();
}

/// A SOPS join that waits for the admin to add this PC's key joins nothing: the runtime starts
/// no timer for it and says nothing of it.
#[tokio::test]
async fn a_join_that_waits_for_the_admin_starts_no_timer_and_sends_no_event() {
    let t = Rt::start().await;
    let timers = t.rt.org_timers();
    eventually("the start-up fetch", || !timers.starting_up()).await;
    let mut events = t.rt.subscribe();
    // A file encrypted to other keys: this PC's new key is not among its recipients.
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sops/good.enc.json")
        .display()
        .to_string();
    let sops_code = code(ACME, ACME_NAME, sops_at(&file), &t.keys);
    t.rt.orgs().sops_identity(ACME).await.unwrap();

    let waiting = t.rt.join_org(&sops_code, None).await.unwrap();
    assert_eq!(waiting.status, OrgStatus::Pending);
    assert!(timers.running().is_empty());
    tokio::time::sleep(StdDuration::from_millis(200)).await;
    assert!(events.try_recv().is_err(), "no event for a join that waits");
    assert!(!t.orgs().is_joined(ACME));

    // Its refresh time comes and goes with nothing fetched or said.
    t.clock.set(at(START) + Duration::minutes(15));
    tokio::time::sleep(StdDuration::from_millis(200)).await;
    assert!(timers.running().is_empty());
    assert!(events.try_recv().is_err());
    t.rt.shutdown();
}

/// The dot editor's status of a dot's variables (`dot_variables`) is read without the fetch a
/// run's start makes of a stale org, and holds no value. A dot whose org was removed gets the
/// reason for each grant.
#[tokio::test]
async fn the_status_of_a_dots_variables_fetches_nothing_and_holds_no_value() {
    let t = Rt::start().await;
    t.join_acme().await;
    let dot =
        t.rt.create_dot(t.triage(), Some(ACME.into()))
            .await
            .unwrap();
    assert_eq!(t.env_reads(), 1);
    // Stale: a run's start would fetch now.
    t.clock
        .set(at(START) + Duration::minutes(15) + Duration::seconds(1));
    let statuses = t.rt.dot_variables(&dot.id).await.unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(
        (statuses[0].consumer.as_str(), statuses[0].name.as_str()),
        ("shell", "DOTS_TEST_LEVEL")
    );
    assert_eq!(statuses[0].status, GrantState::Ok);
    assert_eq!(statuses[0].source, Some(ValueSource::Org));
    assert!(!statuses[0].secret);
    let json = serde_json::to_string(&statuses).unwrap();
    assert!(!json.contains(LEVEL_ONE), "{json}");
    assert_eq!(t.env_reads(), 1, "no fetch");

    t.orgs().remove(ACME).await.unwrap();
    let statuses = t.rt.dot_variables(&dot.id).await.unwrap();
    assert_eq!(statuses[0].status, GrantState::OrgRemoved);
    assert_eq!(statuses[0].text.as_deref(), Some(REMOVED));
    assert!(matches!(
        t.rt.dot_variables("01J0NODOT").await,
        Err(Error::NotFound(_))
    ));
}
