//! A harness for the `Orgs` service (env vault spec §6): a store and secrets in memory, the test
//! protector, a clock the test moves by hand, a fake OpenBao, and an org key that signs real
//! policies with minisign. `Scripted` is a source that answers what a test tells it to, for
//! the cases a real server can't be made to give.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dots_core::store::Store;
use dots_core::vault::org::{
    envelope, http_client_with_timeout, BaoAuth, Fetched, JoinCode, OpenBaoSettings, OrgSummary,
    Orgs, Protector, SopsSettings, Source, SourceCreds, SourceError, SourceFactory, SourceSettings,
    TestProtector, OFFLINE_DIR,
};
use dots_core::vault::MemorySecrets;
use minisign::KeyPair;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::{Notify, Semaphore};
use zeroize::Zeroizing;

use super::clock::FakeClock;
use super::fake_openbao::FakeOpenBao;

pub const ACME: &str = "acme-eng";
pub const ACME_NAME: &str = "Acme Engineering";
/// The person's OpenBao token, which the fake accepts.
pub const TOKEN: &str = "hvs.org-test-token-51c2e0";
/// Where the harness's clock stands until a test moves it.
pub const START: &str = "2026-10-22T09:00:00Z";
/// The `issued_at` of the policies the harness serves.
pub const ISSUED: &str = "2026-10-22T08:00:00Z";

pub fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

/// A time as the service writes it: RFC 3339, UTC, milliseconds.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A new org key pair, as `dots-admin keygen` makes one.
pub fn keys() -> KeyPair {
    KeyPair::generate_unencrypted_keypair().unwrap()
}

/// The join code's `policy_key`: the bare `RW…` line.
pub fn public_key(keys: &KeyPair) -> String {
    keys.pk.to_base64()
}

/// The signed text of `policy`, as `dots-admin policy sign` writes it.
pub fn sign(keys: &KeyPair, policy: &str) -> String {
    let signature = minisign::sign(Some(&keys.pk), &keys.sk, Cursor::new(policy), None, None)
        .unwrap()
        .to_string();
    envelope(policy, &signature)
}

/// A policy for `org_id` issued at `issued_at`: the defaults, with `extra`'s fields over them.
pub fn policy(org_id: &str, issued_at: &str, extra: Value) -> String {
    let mut policy = json!({"v": 1, "org_id": org_id, "issued_at": issued_at});
    for (key, value) in extra.as_object().unwrap() {
        policy[key] = value.clone();
    }
    policy.to_string()
}

pub fn token() -> SourceCreds {
    SourceCreds::Token {
        token: Zeroizing::new(TOKEN.into()),
    }
}

/// OpenBao with token sign-in, the `secret` mount and `path`.
pub fn openbao(address: &str, path: &str) -> SourceSettings {
    SourceSettings::OpenBao(OpenBaoSettings {
        address: address.into(),
        mount: "secret".into(),
        path: path.into(),
        auth: BaoAuth::Token,
        auth_mount: None,
        namespace: None,
    })
}

pub fn sops_at(location: &str) -> SourceSettings {
    SourceSettings::Sops(SopsSettings {
        location: location.into(),
    })
}

/// The join code an admin hands out for `org_id`.
pub fn code(org_id: &str, name: &str, source: SourceSettings, keys: &KeyPair) -> String {
    JoinCode {
        v: 1,
        org_id: org_id.into(),
        name: name.into(),
        source,
        policy_key: public_key(keys),
    }
    .encode()
}

/// One service on a data folder of its own, with its fake OpenBao and org key.
pub struct OrgsHarness {
    pub data: TempDir,
    pub store: Store,
    pub secrets: Arc<MemorySecrets>,
    pub protector: Arc<dyn Protector>,
    pub clock: Arc<FakeClock>,
    pub bao: FakeOpenBao,
    pub keys: KeyPair,
    pub orgs: Orgs,
}

impl OrgsHarness {
    pub async fn new() -> OrgsHarness {
        OrgsHarness::with_protector(Arc::new(TestProtector)).await
    }

    pub async fn with_protector(protector: Arc<dyn Protector>) -> OrgsHarness {
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(&data.path().join("dots.db")).await.unwrap();
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let secrets = Arc::new(MemorySecrets::new());
        let clock = FakeClock::new(at(START));
        let orgs = service(&store, &secrets, &protector, &data, &clock);
        OrgsHarness {
            data,
            store,
            secrets,
            protector,
            clock,
            bao,
            keys: keys(),
            orgs,
        }
    }

    /// Another service on the same data, secrets and clock, as after a restart. It has loaded
    /// nothing yet.
    pub fn service(&self) -> Orgs {
        service(
            &self.store,
            &self.secrets,
            &self.protector,
            &self.data,
            &self.clock,
        )
    }

    /// `data_dir/vault`.
    pub fn vault_dir(&self) -> PathBuf {
        self.data.path().join(OFFLINE_DIR)
    }

    /// Where the offline copy of `org_id` is.
    pub fn copy_path(&self, org_id: &str) -> PathBuf {
        self.vault_dir().join(format!("{org_id}.bin"))
    }

    pub fn now(&self) -> DateTime<Utc> {
        dots_core::scheduler::Clock::now(self.clock.as_ref())
    }

    /// Moves the clock on by `by`.
    pub fn advance(&self, by: Duration) {
        self.clock.set(self.now() + by);
    }

    /// Serves `entries` and the signed `policy` at `secret/<path>` of the fake.
    pub fn serve(&self, path: &str, entries: Value, signed: &str) {
        self.bao.put_env("secret", path, entries);
        self.bao.put_policy("secret", path, signed);
    }

    /// Serves Acme's `entries` at `secret/dots`, with a policy issued at `ISSUED` and `extra`.
    pub fn serve_acme(&self, entries: Value, extra: Value) {
        let signed = sign(&self.keys, &policy(ACME, ISSUED, extra));
        self.serve("dots", entries, &signed);
    }

    /// Acme's join code: the fake at `secret/dots`, and the harness's key.
    pub fn acme_code(&self) -> String {
        self.code_at(ACME, ACME_NAME, "dots")
    }

    /// The join code of `org_id` at `secret/<path>` of the fake, with the harness's key.
    pub fn code_at(&self, org_id: &str, name: &str, path: &str) -> String {
        code(org_id, name, openbao(&self.bao.address(), path), &self.keys)
    }

    /// Serves Acme and joins it with the token.
    pub async fn join_acme(&self, entries: Value, extra: Value) -> OrgSummary {
        self.serve_acme(entries, extra);
        self.orgs
            .join(&self.acme_code(), Some(token()))
            .await
            .unwrap()
    }

    /// How many times the fake was asked for `secret/<path>/env`.
    pub fn env_reads(&self, path: &str) -> usize {
        self.bao.count(&format!("/v1/secret/data/{path}/env"))
    }
}

fn service(
    store: &Store,
    secrets: &Arc<MemorySecrets>,
    protector: &Arc<dyn Protector>,
    data: &TempDir,
    clock: &Arc<FakeClock>,
) -> Orgs {
    Orgs::new(
        store.clone(),
        secrets.clone(),
        protector.clone(),
        data.path().join(OFFLINE_DIR),
        http_client_with_timeout(StdDuration::from_secs(5)),
        clock.clone(),
    )
}

/// A source that answers what the test sets, after a gate the test may hold shut. It records
/// the sign-in details of every fetch.
#[derive(Clone, Default)]
pub struct Scripted(Arc<ScriptedState>);

#[derive(Default)]
struct ScriptedState {
    answer: Mutex<Option<Result<Fetched, SourceError>>>,
    creds: Mutex<Vec<SourceCreds>>,
    gate: Mutex<Option<Arc<Semaphore>>>,
    entered: Notify,
}

impl Scripted {
    pub fn new() -> Scripted {
        Scripted::default()
    }

    /// What every fetch gives from now on.
    pub fn answer(&self, answer: Result<Fetched, SourceError>) {
        *self.0.answer.lock().unwrap() = Some(answer);
    }

    /// Every fetch gives `entries` and the signed `policy`.
    pub fn serve(&self, entries: &[(&str, &str)], signed: &str) {
        self.answer(Ok(Fetched {
            entries: entries
                .iter()
                .map(|(k, v)| (k.to_string(), Zeroizing::new(v.to_string())))
                .collect(),
            policy: signed.to_string(),
        }));
    }

    /// The sign-in details each fetch was given, in order.
    pub fn creds(&self) -> Vec<SourceCreds> {
        self.0.creds.lock().unwrap().clone()
    }

    /// Shuts the gate: each fetch from now on waits for a permit added to the semaphore.
    pub fn hold(&self) -> Arc<Semaphore> {
        let gate = Arc::new(Semaphore::new(0));
        *self.0.gate.lock().unwrap() = Some(gate.clone());
        gate
    }

    /// Returns once a fetch has begun (or had begun since the last call).
    pub async fn entered(&self) {
        tokio::time::timeout(StdDuration::from_secs(10), self.0.entered.notified())
            .await
            .expect("no fetch began");
    }

    /// Every org's source is this one.
    pub fn factory(&self) -> SourceFactory {
        let this = self.clone();
        Arc::new(move |_: &SourceSettings| Box::new(this.clone()) as Box<dyn Source>)
    }
}

#[async_trait]
impl Source for Scripted {
    async fn fetch(&self, creds: &SourceCreds) -> Result<Fetched, SourceError> {
        self.0.creds.lock().unwrap().push(creds.clone());
        self.0.entered.notify_one();
        let gate = self.0.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
        self.0
            .answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Err(SourceError::Unreachable("no answer is set".into())))
    }
}
