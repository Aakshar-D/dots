//! The status of a dot's grants (env vault spec §9, the dot editor's Variables): each name
//! granted to each consumer, resolved in the dot's scope by the resolver's own steps, with
//! where its value comes from and why it does not resolve. No value is in the answer, no
//! Personal secret's value is read, and the org is not fetched.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::Duration;
use common::fake_openbao::Failure;
use common::orgs::{keys, policy, sign, OrgsHarness, ACME, TOKEN};
use common::spec;
use dots_core::library::{LibraryConfig, McpServerConfig};
use dots_core::model::Dot;
use dots_core::vault::org::{OrgStatus, PersonalMode, UseRules};
use dots_core::vault::{
    Consumer, DotScope, EntryKind, GrantState, GrantStatus, Grants, MemorySecrets, SecretStore,
    ValueSource, Vault,
};
use serde_json::{json, Value};
use zeroize::Zeroizing;

const ORG_TOKEN: &str = "org-secret-gh-token-7c41e9";
const ORG_PROD: &str = "org-secret-prod-db-0b5d22";
const ORG_LEVEL: &str = "org-plain-level-e83f10";
const PERSONAL_KEY: &str = "personal-secret-key-4a90cd";
const PERSONAL_REGION: &str = "personal-plain-region-61b7";

/// Every value a test serves or stores, which no answer may hold.
const VALUES: &[&str] = &[
    ORG_TOKEN,
    ORG_PROD,
    ORG_LEVEL,
    PERSONAL_KEY,
    PERSONAL_REGION,
    TOKEN,
];

fn assert_clean(text: &str) {
    for value in VALUES {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

/// A secret store that counts the values read from it.
#[derive(Debug, Default)]
struct Counting {
    inner: MemorySecrets,
    gets: AtomicUsize,
}

impl SecretStore for Counting {
    fn put(&self, key: &str, value: &str) -> dots_core::Result<()> {
        self.inner.put(key, value)
    }
    fn get(&self, key: &str) -> dots_core::Result<Option<Zeroizing<String>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(key)
    }
    fn delete(&self, key: &str) -> dots_core::Result<()> {
        self.inner.delete(key)
    }
    fn keys(&self, prefix: &str) -> dots_core::Result<Vec<String>> {
        self.inner.keys(prefix)
    }
}

/// The org's entries: the secrets `GITHUB_TOKEN` and `PROD_DB`, and the plain `LOG_LEVEL`.
fn org_entries() -> Value {
    json!({"GITHUB_TOKEN": ORG_TOKEN, "PROD_DB": ORG_PROD, "LOG_LEVEL": ORG_LEVEL})
}

struct Setup {
    h: OrgsHarness,
    vault: Vault,
    secrets: Arc<Counting>,
}

/// Acme joined with `org_entries` under a policy of `extra` (`LOG_LEVEL` plain), and Personal
/// holding the secret `API_KEY` and the plain `REGION`, in a store that counts reads.
async fn setup(extra: Value) -> Setup {
    let h = OrgsHarness::new().await;
    let mut extra = extra;
    extra["plain"] = json!(["LOG_LEVEL"]);
    h.join_acme(org_entries(), extra).await;
    let secrets = Arc::new(Counting::default());
    let vault = Vault::new(h.store.clone(), secrets.clone()).with_orgs(h.orgs.clone());
    vault
        .add("API_KEY", EntryKind::Secret, PERSONAL_KEY)
        .await
        .unwrap();
    vault
        .add("REGION", EntryKind::Plain, PERSONAL_REGION)
        .await
        .unwrap();
    // A server that reads no variable, so ticking it grants nothing beyond what a test gives.
    let github = McpServerConfig::Stdio {
        command: "gh-mcp".into(),
        args: Vec::new(),
        cwd: None,
        env: Default::default(),
    };
    h.store
        .add_library_item("github", &LibraryConfig::McpServer(github), None)
        .await
        .unwrap();
    Setup { h, vault, secrets }
}

impl Setup {
    /// A dot named `triage` in `org`, using the server `github`, granted `grants` (consumer
    /// key, names).
    async fn dot(&self, org: Option<&str>, grants: &[(&str, &[&str])]) -> Dot {
        let mut s = spec("triage");
        s.uses.mcp_servers = vec!["github".into()];
        s.variables = Grants(
            grants
                .iter()
                .map(|(key, names)| {
                    (
                        key.to_string(),
                        names.iter().map(|n| n.to_string()).collect(),
                    )
                })
                .collect(),
        );
        self.h.store.create_dot(&s, org).await.unwrap()
    }

    /// The statuses of `dot`, none of which holds a value, read without reading a Personal
    /// secret's value.
    async fn statuses(&self, dot: &Dot) -> Vec<GrantStatus> {
        let gets = self.secrets.gets.load(Ordering::SeqCst);
        let statuses = self.vault.grant_statuses(dot).await.unwrap();
        assert_eq!(
            self.secrets.gets.load(Ordering::SeqCst),
            gets,
            "no value was read"
        );
        assert_clean(&format!("{statuses:?}"));
        assert_clean(&serde_json::to_string(&statuses).unwrap());
        statuses
    }
}

/// A status that resolves.
fn ok(consumer: &str, name: &str, source: ValueSource, secret: bool) -> GrantStatus {
    GrantStatus {
        consumer: consumer.into(),
        name: name.into(),
        status: GrantState::Ok,
        text: None,
        source: Some(source),
        secret,
    }
}

/// A status that does not resolve, with its text.
fn not(consumer: &str, name: &str, status: GrantState, text: &str) -> GrantStatus {
    GrantStatus {
        consumer: consumer.into(),
        name: name.into(),
        status,
        text: Some(text.into()),
        source: None,
        secret: false,
    }
}

/// Each status agrees with what `resolve` gives for that grant: the same names resolve, and
/// the same text is given for those that don't.
async fn agrees_with_resolve(vault: &Vault, dot: &Dot, statuses: &[GrantStatus]) {
    let scope = DotScope::of(dot);
    for status in statuses {
        let consumer = Consumer::parse(&status.consumer).unwrap();
        let names = vec![status.name.clone()];
        let resolved = vault
            .resolve(&scope, &dot.spec.variables, &consumer, &names)
            .await
            .unwrap();
        match resolved {
            Ok(_) => assert_eq!(status.status, GrantState::Ok, "{status:?}"),
            Err(missing) => {
                assert_eq!(
                    status.text,
                    Some(missing.text(&dot.spec.name)),
                    "{status:?}"
                )
            }
        }
    }
}

#[tokio::test]
async fn a_grant_resolves_from_the_org_or_from_personal_under_fallback() {
    let t = setup(json!({"personal": "fallback"})).await;
    let dot = t
        .dot(
            Some(ACME),
            &[
                ("mcp:github", &["GITHUB_TOKEN", "API_KEY", "REGION"]),
                ("shell", &["LOG_LEVEL", "region"]),
            ],
        )
        .await;
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses,
        [
            ok("mcp:github", "GITHUB_TOKEN", ValueSource::Org, true),
            ok("mcp:github", "API_KEY", ValueSource::Personal, true),
            ok("mcp:github", "REGION", ValueSource::Personal, false),
            ok("shell", "LOG_LEVEL", ValueSource::Org, false),
            // As granted, in any case.
            ok("shell", "region", ValueSource::Personal, false),
        ]
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;
}

#[tokio::test]
async fn under_override_a_name_both_have_comes_from_personal() {
    let t = setup(json!({"personal": "override", "shell_secrets": true})).await;
    // The org's summary tells the editor both rules, for its name pickers.
    let summary = t.h.orgs.summary(ACME).await.unwrap();
    assert_eq!(summary.personal_mode, Some(PersonalMode::Override));
    assert!(summary.shell_secrets);
    t.vault
        .add("LOG_LEVEL", EntryKind::Plain, PERSONAL_REGION)
        .await
        .unwrap();
    let dot = t
        .dot(Some(ACME), &[("shell", &["LOG_LEVEL", "GITHUB_TOKEN"])])
        .await;
    assert_eq!(
        t.statuses(&dot).await,
        [
            ok("shell", "LOG_LEVEL", ValueSource::Personal, false),
            // `shell_secrets: true`: the shell may have the org's secret.
            ok("shell", "GITHUB_TOKEN", ValueSource::Org, true),
        ]
    );
}

#[tokio::test]
async fn each_reason_a_grant_does_not_resolve_has_its_status_and_text() {
    let t = setup(json!({
        "personal": "fallback",
        "use": {"allow": ["*"], "deny": ["PROD_*"]},
    }))
    .await;
    // The summary gives the editor the `use` globs, so its shell picker leaves out PROD_*.
    let summary = t.h.orgs.summary(ACME).await.unwrap();
    assert_eq!(
        summary.use_rules,
        Some(UseRules {
            allow: vec!["*".into()],
            deny: vec!["PROD_*".into()],
        })
    );
    let dot = t
        .dot(
            Some(ACME),
            &[
                ("mcp:github", &["NOWHERE", "PROD_DB"]),
                ("shell", &["GITHUB_TOKEN", "API_KEY", "LOG_LEVEL"]),
            ],
        )
        .await;
    let statuses = t.statuses(&dot).await;
    let forbidden = |name: &str, source: ValueSource| GrantStatus {
        source: Some(source),
        secret: true,
        ..not(
            "shell",
            name,
            GrantState::ShellSecretsForbidden,
            &format!("{name}: Acme Engineering does not allow secrets in the shell (dot 'triage')"),
        )
    };
    assert_eq!(
        statuses,
        [
            not(
                "mcp:github",
                "NOWHERE",
                GrantState::NotDefined,
                "NOWHERE: not in Acme Engineering (dot 'triage', mcp:github)"
            ),
            not(
                "mcp:github",
                "PROD_DB",
                GrantState::DeniedByPolicy,
                "PROD_DB: Acme Engineering's policy does not allow it (dot 'triage')"
            ),
            // A secret of the org's, and a Personal one `fallback` would give.
            forbidden("GITHUB_TOKEN", ValueSource::Org),
            forbidden("API_KEY", ValueSource::Personal),
            // A plain value is no secret.
            ok("shell", "LOG_LEVEL", ValueSource::Org, false),
        ]
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;
}

/// Serves Acme's entries under a policy signed with a key that is not the org's.
fn serve_badly_signed(h: &OrgsHarness) {
    let signed = sign(&keys(), &policy(ACME, "2026-10-22T10:00:00Z", json!({})));
    h.serve("dots", org_entries(), &signed);
}

#[tokio::test]
async fn an_org_that_gives_no_values_is_the_status_of_every_grant() {
    let t = setup(json!({"personal": "override", "offline_hours": 0})).await;
    let dot = t
        .dot(Some(ACME), &[("mcp:github", &["GITHUB_TOKEN", "REGION"])])
        .await;
    let each = |status: GrantState, text: &str| {
        vec![
            not("mcp:github", "GITHUB_TOKEN", status, text),
            not("mcp:github", "REGION", status, text),
        ]
    };

    t.h.bao.fail_next(Failure::Status(503), 1);
    assert_eq!(
        t.h.orgs.refresh(ACME).await.unwrap().status,
        OrgStatus::Offline { until: None }
    );
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses,
        each(
            GrantState::OrgOffline,
            "Acme Engineering: offline, with no saved copy"
        )
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;

    serve_badly_signed(&t.h);
    let OrgStatus::PolicyInvalid { error } = t.h.orgs.refresh(ACME).await.unwrap().status else {
        panic!("the policy is refused");
    };
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses,
        each(
            GrantState::PolicyInvalid,
            &format!("Acme Engineering: policy invalid: {error}")
        )
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;

    t.h.bao.revoke_token(TOKEN);
    assert_eq!(
        t.h.orgs.refresh(ACME).await.unwrap().status,
        OrgStatus::Revoked
    );
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses,
        each(GrantState::OrgRevoked, "Acme Engineering: access revoked")
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;

    t.h.orgs.remove(ACME).await.unwrap();
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses,
        each(
            GrantState::OrgRemoved,
            "dot 'triage': its org 'acme-eng' was removed"
        )
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;
}

#[tokio::test]
async fn an_expired_copy_is_the_status_of_every_grant() {
    let t = setup(json!({"personal": "fallback", "offline_hours": 2})).await;
    let dot = t.dot(Some(ACME), &[("shell", &["LOG_LEVEL"])]).await;
    t.h.bao.fail_next(Failure::Status(503), 1);
    t.h.orgs.refresh(ACME).await.unwrap();
    t.h.advance(Duration::hours(2));
    assert_eq!(
        t.statuses(&dot).await,
        [not(
            "shell",
            "LOG_LEVEL",
            GrantState::OrgExpired,
            "Acme Engineering: secrets expired offline; reconnect to refresh"
        )]
    );
}

#[tokio::test]
async fn the_status_fetches_nothing_even_when_the_org_is_stale() {
    let t = setup(json!({"personal": "deny"})).await;
    let dot = t
        .dot(Some(ACME), &[("mcp:github", &["GITHUB_TOKEN"])])
        .await;
    assert_eq!(t.h.env_reads("dots"), 1);
    // Past `refresh_minutes` (15): a run's start would fetch now.
    t.h.advance(Duration::minutes(16));
    assert_eq!(
        t.statuses(&dot).await,
        [ok("mcp:github", "GITHUB_TOKEN", ValueSource::Org, true)]
    );
    assert_eq!(t.h.env_reads("dots"), 1, "no fetch");
}

#[tokio::test]
async fn a_personal_dot_shows_personal_only() {
    let t = setup(json!({"personal": "fallback"})).await;
    let dot = t
        .dot(None, &[("shell", &["API_KEY", "GITHUB_TOKEN", "REGION"])])
        .await;
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses,
        [
            // A Personal dot's shell may have a Personal secret.
            ok("shell", "API_KEY", ValueSource::Personal, true),
            not(
                "shell",
                "GITHUB_TOKEN",
                GrantState::NotDefined,
                "GITHUB_TOKEN: not in Personal (dot 'triage', shell)"
            ),
            ok("shell", "REGION", ValueSource::Personal, false),
        ]
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;

    // A secret whose value is gone from the store does not resolve, as at a run.
    t.secrets.inner.delete("personal/API_KEY").unwrap();
    let statuses = t.statuses(&dot).await;
    assert_eq!(
        statuses[0],
        not(
            "shell",
            "API_KEY",
            GrantState::NotDefined,
            "API_KEY: not in Personal (dot 'triage', shell)"
        )
    );
    agrees_with_resolve(&t.vault, &dot, &statuses).await;
}

#[tokio::test]
async fn a_dot_with_no_grants_has_no_status() {
    let t = setup(json!({})).await;
    let dot = t.dot(Some(ACME), &[]).await;
    assert_eq!(t.statuses(&dot).await, []);
}

#[test]
fn the_states_serialize_in_snake_case() {
    let names: Vec<Value> = GrantState::ALL
        .iter()
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "ok",
            "not_granted",
            "not_defined",
            "denied_by_policy",
            "shell_secrets_forbidden",
            "org_offline",
            "org_expired",
            "org_revoked",
            "policy_invalid",
            "org_removed",
            "not_member"
        ]
    );
    assert_eq!(
        serde_json::to_value(ValueSource::Personal).unwrap(),
        json!("personal")
    );
}
