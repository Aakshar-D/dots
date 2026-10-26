//! Resolving a dot's variables in its org (env vault spec §8): the grant first, then the org's
//! state and its `use` rules, then its `personal` mode, then the shell's secrets rule. A
//! Personal dot reads Personal only, as before.

mod common;

use chrono::Duration;
use common::fake_openbao::Failure;
use common::orgs::{
    keys, policy, sign, token, OrgsHarness, Scripted, ACME, ACME_NAME, ISSUED, TOKEN,
};
use common::{spec, temp_store};
use dots_core::vault::org::OrgStatus;
use dots_core::vault::{Consumer, DotScope, EntryKind, Grants, Missing, Unresolved, Vault};
use dots_core::Error;
use serde_json::{json, Value};

const DOT: &str = "triage";

const ORG_BOTH: &str = "org-secret-both-3f9a1c";
const ORG_ONLY: &str = "org-secret-only-b27e44";
const ORG_PLAIN: &str = "org-plain-level-5d71e0";
const ORG_ROTATED: &str = "org-secret-rotated-0e8b37";
const PERSONAL_BOTH: &str = "personal-secret-both-91d0aa";
const PERSONAL_ONLY: &str = "personal-secret-only-6c5e12";
const PERSONAL_PLAIN: &str = "personal-plain-region-2a4f";
const ACME_SHARED: &str = "acme-secret-shared-71c0d4";
const ACME_OWN: &str = "acme-secret-own-e93b20";
const BETA_SHARED: &str = "beta-secret-shared-4a6f81";
const BETA_OWN: &str = "beta-secret-own-0d52c7";

/// Every value a test serves or stores, which no error or `Debug` may hold.
const VALUES: &[&str] = &[
    ORG_BOTH,
    ORG_ONLY,
    ORG_PLAIN,
    ORG_ROTATED,
    PERSONAL_BOTH,
    PERSONAL_ONLY,
    PERSONAL_PLAIN,
    ACME_SHARED,
    ACME_OWN,
    BETA_SHARED,
    BETA_OWN,
    TOKEN,
];

fn assert_clean(text: &str) {
    for value in VALUES {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

fn github() -> Consumer {
    Consumer::Mcp("github".into())
}

fn acme() -> DotScope {
    DotScope::Org(ACME.into())
}

/// The org's entries: `BOTH` (Personal has it too), `IN_ORG`, and the plain `LOG_LEVEL`.
fn org_entries() -> Value {
    json!({"BOTH": ORG_BOTH, "IN_ORG": ORG_ONLY, "LOG_LEVEL": ORG_PLAIN})
}

/// Acme joined with `org_entries` and a policy of `extra` (with `LOG_LEVEL` plain), and
/// Personal holding `BOTH`, `IN_PERSONAL` (secrets) and the plain `REGION`.
async fn setup(extra: Value) -> (OrgsHarness, Vault) {
    let h = OrgsHarness::new().await;
    let mut extra = extra;
    extra["plain"] = json!(["LOG_LEVEL"]);
    h.join_acme(org_entries(), extra).await;
    let vault = h.vault();
    vault
        .add("BOTH", EntryKind::Secret, PERSONAL_BOTH)
        .await
        .unwrap();
    vault
        .add("IN_PERSONAL", EntryKind::Secret, PERSONAL_ONLY)
        .await
        .unwrap();
    vault
        .add("REGION", EntryKind::Plain, PERSONAL_PLAIN)
        .await
        .unwrap();
    (h, vault)
}

/// Resolves `name`, granted to `consumer`, in `scope`: its value, or what is missing. Neither
/// side's `Debug`, nor the missing text, holds a value.
async fn resolve(
    vault: &Vault,
    scope: &DotScope,
    consumer: &Consumer,
    name: &str,
) -> Result<String, Missing> {
    let grants = Grants([(consumer.key(), vec![name.to_string()])].into());
    let names = vec![name.to_string()];
    let resolved = vault
        .resolve(scope, &grants, consumer, &names)
        .await
        .unwrap();
    match resolved {
        Ok(resolved) => {
            assert_clean(&format!("{resolved:?}"));
            Ok(resolved.get(name).expect("the asked name"))
        }
        Err(missing) => {
            assert_clean(&format!("{missing:?}"));
            assert_clean(&missing.text(DOT));
            Err(missing)
        }
    }
}

/// What is missing for `name`, as its reason and its text.
async fn missing(
    vault: &Vault,
    scope: &DotScope,
    consumer: &Consumer,
    name: &str,
) -> (Unresolved, String) {
    let missing = resolve(vault, scope, consumer, name)
        .await
        .expect_err("a missing variable");
    assert_eq!(missing.name, name);
    assert_eq!(&missing.consumer, consumer);
    (missing.why.clone(), missing.text(DOT))
}

// The `personal` modes

#[tokio::test]
async fn under_deny_only_the_orgs_entries_resolve() {
    let (_h, vault) = setup(json!({"personal": "deny"})).await;
    let scope = acme();
    assert_eq!(
        resolve(&vault, &scope, &github(), "BOTH").await.unwrap(),
        ORG_BOTH
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
    assert_eq!(
        missing(&vault, &scope, &github(), "IN_PERSONAL").await,
        (
            Unresolved::NotDefined,
            "IN_PERSONAL: not in Acme Engineering (dot 'triage', mcp:github)".into()
        )
    );
}

#[tokio::test]
async fn under_fallback_the_orgs_entry_wins_else_personal() {
    let (_h, vault) = setup(json!({"personal": "fallback"})).await;
    let scope = acme();
    assert_eq!(
        resolve(&vault, &scope, &github(), "BOTH").await.unwrap(),
        ORG_BOTH
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "IN_PERSONAL")
            .await
            .unwrap(),
        PERSONAL_ONLY
    );
    assert_eq!(
        missing(&vault, &scope, &github(), "NOWHERE").await,
        (
            Unresolved::NotDefined,
            "NOWHERE: not in Acme Engineering (dot 'triage', mcp:github)".into()
        )
    );
}

#[tokio::test]
async fn under_override_personal_wins_else_the_orgs_entry() {
    let (_h, vault) = setup(json!({"personal": "override"})).await;
    let scope = acme();
    assert_eq!(
        resolve(&vault, &scope, &github(), "BOTH").await.unwrap(),
        PERSONAL_BOTH
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "IN_PERSONAL")
            .await
            .unwrap(),
        PERSONAL_ONLY
    );
    assert_eq!(
        missing(&vault, &scope, &github(), "NOWHERE").await.0,
        Unresolved::NotDefined
    );
}

// The grant and `use`

#[tokio::test]
async fn a_name_not_granted_is_refused_before_anything_else() {
    let (h, vault) = setup(json!({"personal": "override"})).await;
    let grants = Grants([("mcp:github".to_string(), vec!["BOTH".to_string()])].into());
    let names = vec!["IN_ORG".to_string()];
    let refused = vault
        .resolve(&acme(), &grants, &github(), &names)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.why, Unresolved::NotGranted);
    assert_eq!(
        refused.text(DOT),
        "IN_ORG: not granted to mcp:github in dot 'triage'"
    );

    // Even an org that is gone: the grant comes first.
    h.orgs.remove(ACME).await.unwrap();
    let refused = vault
        .resolve(&acme(), &grants, &github(), &names)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.why, Unresolved::NotGranted);
}

#[tokio::test]
async fn use_deny_wins_over_allow_wherever_the_value_would_come_from() {
    let h = OrgsHarness::new().await;
    let extra = json!({
        "personal": "override",
        "use": {"allow": ["PROD_*", "*"], "deny": ["PROD_*"]},
    });
    h.join_acme(json!({"PROD_DB": ORG_BOTH, "DEV_DB": ORG_ONLY}), extra)
        .await;
    let vault = h.vault();
    vault
        .add("PROD_TOKEN", EntryKind::Secret, PERSONAL_ONLY)
        .await
        .unwrap();
    let scope = acme();
    assert_eq!(
        missing(&vault, &scope, &github(), "PROD_DB").await,
        (
            Unresolved::DeniedByPolicy,
            "PROD_DB: Acme Engineering's policy does not allow it (dot 'triage')".into()
        )
    );
    // In another case, and for a name only Personal has, which `override` would let win.
    assert_eq!(
        missing(&vault, &scope, &github(), "prod_db").await.0,
        Unresolved::DeniedByPolicy
    );
    assert_eq!(
        missing(&vault, &scope, &github(), "PROD_TOKEN").await.0,
        Unresolved::DeniedByPolicy
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "DEV_DB").await.unwrap(),
        ORG_ONLY
    );
}

// The shell's secrets

#[tokio::test]
async fn the_shell_gets_no_secret_while_shell_secrets_is_false() {
    let (_h, vault) = setup(json!({"personal": "fallback", "shell_secrets": false})).await;
    let scope = acme();
    let shell = Consumer::Shell;
    // An org secret, and a Personal secret that `fallback` reaches.
    assert_eq!(
        missing(&vault, &scope, &shell, "IN_ORG").await,
        (
            Unresolved::ShellSecretsForbidden,
            "IN_ORG: Acme Engineering does not allow secrets in the shell (dot 'triage')".into()
        )
    );
    assert_eq!(
        missing(&vault, &scope, &shell, "IN_PERSONAL").await.0,
        Unresolved::ShellSecretsForbidden
    );
    // A plain org entry and a plain Personal one are not secrets.
    assert_eq!(
        resolve(&vault, &scope, &shell, "LOG_LEVEL").await.unwrap(),
        ORG_PLAIN
    );
    assert_eq!(
        resolve(&vault, &scope, &shell, "REGION").await.unwrap(),
        PERSONAL_PLAIN
    );
    // The rule is the shell's: a server still gets the secret.
    assert_eq!(
        resolve(&vault, &scope, &github(), "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
}

#[tokio::test]
async fn the_shell_gets_secrets_when_shell_secrets_is_true() {
    let (_h, vault) = setup(json!({"personal": "fallback", "shell_secrets": true})).await;
    let scope = acme();
    let shell = Consumer::Shell;
    assert_eq!(
        resolve(&vault, &scope, &shell, "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
    assert_eq!(
        resolve(&vault, &scope, &shell, "IN_PERSONAL")
            .await
            .unwrap(),
        PERSONAL_ONLY
    );
}

#[tokio::test]
async fn names_compare_without_regard_to_case() {
    let (_h, vault) = setup(json!({"personal": "fallback"})).await;
    let scope = acme();
    assert_eq!(
        resolve(&vault, &scope, &github(), "in_org").await.unwrap(),
        ORG_ONLY
    );
    assert_eq!(
        resolve(&vault, &scope, &github(), "In_Personal")
            .await
            .unwrap(),
        PERSONAL_ONLY
    );
    // `plain` matches in any case too, so the shell gets it.
    assert_eq!(
        resolve(&vault, &scope, &Consumer::Shell, "log_level")
            .await
            .unwrap(),
        ORG_PLAIN
    );
    assert_eq!(
        missing(&vault, &scope, &Consumer::Shell, "both").await.0,
        Unresolved::ShellSecretsForbidden
    );
}

// The org's state

#[tokio::test]
async fn an_offline_org_resolves_from_its_copy() {
    let (h, vault) = setup(json!({"personal": "deny"})).await;
    h.bao.fail_next(Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert!(matches!(
        summary.status,
        OrgStatus::Offline { until: Some(_) }
    ));
    assert_eq!(
        resolve(&vault, &acme(), &github(), "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
}

#[tokio::test]
async fn an_offline_org_with_no_copy_gives_nothing() {
    let (h, vault) = setup(json!({"personal": "override", "offline_hours": 0})).await;
    h.bao.fail_next(Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.status, OrgStatus::Offline { until: None });
    let offline = (
        Unresolved::OrgOffline,
        "Acme Engineering: offline, with no saved copy".to_string(),
    );
    assert_eq!(missing(&vault, &acme(), &github(), "IN_ORG").await, offline);
    // The org's state blocks every name, even one `override` would take from Personal.
    assert_eq!(missing(&vault, &acme(), &github(), "BOTH").await, offline);
}

#[tokio::test]
async fn an_expired_copy_gives_nothing() {
    let (h, vault) = setup(json!({"personal": "fallback", "offline_hours": 2})).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    h.advance(Duration::hours(2));
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_ORG").await,
        (
            Unresolved::OrgExpired,
            "Acme Engineering: secrets expired offline; reconnect to refresh".into()
        )
    );
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_PERSONAL").await.0,
        Unresolved::OrgExpired
    );
}

#[tokio::test]
async fn a_revoked_org_gives_nothing() {
    let (h, vault) = setup(json!({"personal": "override"})).await;
    h.bao.revoke_token(TOKEN);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.status, OrgStatus::Revoked);
    let revoked = (
        Unresolved::OrgRevoked,
        "Acme Engineering: access revoked".to_string(),
    );
    assert_eq!(missing(&vault, &acme(), &github(), "IN_ORG").await, revoked);
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_PERSONAL").await,
        revoked
    );
}

/// Serves Acme's entries, rotated, under a policy signed with a key that is not the org's.
fn serve_badly_signed(h: &OrgsHarness, extra: Value) {
    let signed = sign(&keys(), &policy(ACME, "2026-10-22T10:00:00Z", extra));
    h.serve("dots", json!({"IN_ORG": ORG_ROTATED}), &signed);
}

#[tokio::test]
async fn an_invalid_policy_keeps_the_last_good_copy() {
    let (h, vault) = setup(json!({"personal": "deny"})).await;
    serve_badly_signed(&h, json!({}));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert!(matches!(summary.status, OrgStatus::PolicyInvalid { .. }));
    assert_eq!(
        resolve(&vault, &acme(), &github(), "IN_ORG").await.unwrap(),
        ORG_ONLY
    );
}

#[tokio::test]
async fn an_invalid_policy_with_no_copy_gives_nothing() {
    let (h, vault) = setup(json!({"personal": "fallback", "offline_hours": 0})).await;
    serve_badly_signed(&h, json!({"offline_hours": 0}));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    let OrgStatus::PolicyInvalid { error } = summary.status else {
        panic!("{summary:?}");
    };
    assert!(!error.is_empty());
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_ORG").await,
        (
            Unresolved::PolicyInvalid(error.clone()),
            format!("Acme Engineering: policy invalid: {error}")
        )
    );
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_PERSONAL").await.0,
        Unresolved::PolicyInvalid(error)
    );
}

#[tokio::test]
async fn a_removed_org_gives_nothing() {
    let (h, vault) = setup(json!({"personal": "override"})).await;
    h.orgs.remove(ACME).await.unwrap();
    let removed = (
        Unresolved::OrgRemoved,
        "dot 'triage': its org 'acme-eng' was removed".to_string(),
    );
    assert_eq!(missing(&vault, &acme(), &github(), "IN_ORG").await, removed);
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_PERSONAL").await,
        removed
    );
    let gone = resolve(&vault, &acme(), &github(), "IN_ORG")
        .await
        .unwrap_err();
    assert_eq!(gone.org.as_deref(), Some(ACME));
}

#[tokio::test]
async fn a_vault_with_no_orgs_resolves_an_org_scope_as_removed() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    vault
        .add("IN_PERSONAL", EntryKind::Secret, PERSONAL_ONLY)
        .await
        .unwrap();
    assert_eq!(
        missing(&vault, &acme(), &github(), "IN_PERSONAL").await,
        (
            Unresolved::OrgRemoved,
            "dot 'triage': its org 'acme-eng' was removed".into()
        )
    );
    assert_eq!(
        resolve(&vault, &DotScope::Personal, &github(), "IN_PERSONAL")
            .await
            .unwrap(),
        PERSONAL_ONLY
    );
}

// Personal dots and Library-time reads

#[tokio::test]
async fn a_personal_dot_reads_personal_only() {
    let (_h, vault) = setup(json!({"personal": "fallback"})).await;
    let scope = DotScope::Personal;
    assert_eq!(
        resolve(&vault, &scope, &github(), "BOTH").await.unwrap(),
        PERSONAL_BOTH
    );
    assert_eq!(
        missing(&vault, &scope, &github(), "IN_ORG").await,
        (
            Unresolved::NotDefined,
            "IN_ORG: not in Personal (dot 'triage', mcp:github)".into()
        )
    );
    // No org rule applies: the shell gets a Personal secret.
    assert_eq!(
        resolve(&vault, &scope, &Consumer::Shell, "IN_PERSONAL")
            .await
            .unwrap(),
        PERSONAL_ONLY
    );
    // What the Library reads outside a run is Personal too.
    let names = vec!["IN_ORG".to_string(), "BOTH".to_string()];
    let personal = vault.personal_values(&names).await.unwrap();
    assert!(!personal.contains("IN_ORG"));
    assert_eq!(personal.get("BOTH").as_deref(), Some(PERSONAL_BOTH));
}

/// Two orgs with the same names and other values, a dot in each and a Personal dot: each sees
/// its own org's value (or Personal's, where its mode lets Personal win), never the other
/// org's, under every `personal` mode.
#[tokio::test]
async fn two_orgs_with_the_same_names_never_mix_under_any_personal_mode() {
    for mode in ["deny", "fallback", "override"] {
        let h = OrgsHarness::new().await;
        let extra = json!({"personal": mode});
        let acme_policy = sign(&h.keys, &policy(ACME, ISSUED, extra.clone()));
        h.serve(
            "dots",
            json!({"SHARED": ACME_SHARED, "ACME_OWN": ACME_OWN}),
            &acme_policy,
        );
        h.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
        let beta_policy = sign(&h.keys, &policy("beta", ISSUED, extra));
        h.serve(
            "beta",
            json!({"SHARED": BETA_SHARED, "BETA_OWN": BETA_OWN}),
            &beta_policy,
        );
        h.orgs
            .join(&h.code_at("beta", "Beta", "beta"), Some(token()))
            .await
            .unwrap();
        let vault = h.vault();
        vault
            .add("SHARED", EntryKind::Secret, PERSONAL_BOTH)
            .await
            .unwrap();
        let store = &h.store;
        let in_acme = store
            .create_dot(&spec("in-acme"), Some(ACME))
            .await
            .unwrap();
        let in_beta = store
            .create_dot(&spec("in-beta"), Some("beta"))
            .await
            .unwrap();
        let mine = store.create_dot(&spec("mine"), None).await.unwrap();
        let (acme, beta, personal) = (
            DotScope::of(&in_acme),
            DotScope::of(&in_beta),
            DotScope::of(&mine),
        );
        // Under `override` Personal's value wins in both orgs; otherwise each org's own.
        let shared = |own: &'static str| {
            if mode == "override" {
                PERSONAL_BOTH
            } else {
                own
            }
        };
        let gh = github();
        assert_eq!(
            resolve(&vault, &acme, &gh, "SHARED").await.unwrap(),
            shared(ACME_SHARED),
            "{mode}"
        );
        assert_eq!(
            resolve(&vault, &beta, &gh, "SHARED").await.unwrap(),
            shared(BETA_SHARED),
            "{mode}"
        );
        assert_eq!(
            resolve(&vault, &personal, &gh, "SHARED").await.unwrap(),
            PERSONAL_BOTH,
            "{mode}"
        );
        assert_eq!(
            resolve(&vault, &acme, &gh, "ACME_OWN").await.unwrap(),
            ACME_OWN
        );
        assert_eq!(
            resolve(&vault, &beta, &gh, "BETA_OWN").await.unwrap(),
            BETA_OWN
        );
        // A name only the other org has is in neither this org nor Personal.
        for (scope, name) in [
            (&acme, "BETA_OWN"),
            (&beta, "ACME_OWN"),
            (&personal, "ACME_OWN"),
            (&personal, "BETA_OWN"),
        ] {
            assert_eq!(
                missing(&vault, scope, &gh, name).await.0,
                Unresolved::NotDefined,
                "{mode}: {name} in {scope:?}"
            );
        }
    }
}

/// A run's start refreshes a stale org under the org's lock. A Remove that was waiting for
/// that lock goes through before the refresh gets it, and the run must not start then.
#[tokio::test]
async fn a_run_whose_org_is_removed_while_it_waits_to_refresh_does_not_start() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let signed = sign(&h.keys, &policy(ACME, ISSUED, json!({})));
    scripted.serve(&[("BOTH", ORG_BOTH)], &signed);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    scripted.entered().await;
    let vault = Vault::new(h.store.clone(), h.secrets.clone()).with_orgs(orgs.clone());
    let dot = h.store.create_dot(&spec(DOT), Some(ACME)).await.unwrap();

    // A Refresh holds the org's lock while its source answers.
    let gate = scripted.hold();
    let refreshing = tokio::spawn({
        let orgs = orgs.clone();
        async move { orgs.refresh(ACME).await }
    });
    scripted.entered().await;
    // A Remove waits for the lock, then the run's start does: the org is still joined then.
    let remove = orgs.remove(ACME);
    tokio::pin!(remove);
    assert!(futures_util::poll!(&mut remove).is_pending());
    let start = vault.before_run(&dot);
    tokio::pin!(start);
    assert!(futures_util::poll!(&mut start).is_pending());

    gate.add_permits(1);
    refreshing.await.unwrap().unwrap();
    remove.await.unwrap();
    match start.await {
        Err(Error::Other(text)) => {
            assert_eq!(text, "dot 'triage': its org 'acme-eng' was removed")
        }
        other => panic!("expected the run refused, got {other:?}"),
    }
}

#[tokio::test]
async fn a_dots_scope_is_its_org() {
    let h = OrgsHarness::new().await;
    let personal = h.store.create_dot(&spec("mine"), None).await.unwrap();
    let in_org = h
        .store
        .create_dot(&spec("theirs"), Some(ACME))
        .await
        .unwrap();
    assert_eq!(DotScope::of(&personal), DotScope::Personal);
    assert_eq!(DotScope::of(&in_org), DotScope::Org(ACME.into()));
}

#[test]
fn every_missing_text_is_exact() {
    let text = |name: &str, consumer: Consumer, why: Unresolved, org: Option<&str>| {
        Missing {
            name: name.into(),
            consumer,
            why,
            org: org.map(str::to_string),
        }
        .text("D")
    };
    let acme = Some(ACME_NAME);
    let x = || Consumer::Mcp("x".into());
    let cases = [
        (
            text("NAME", x(), Unresolved::NotGranted, acme),
            "NAME: not granted to mcp:x in dot 'D'",
        ),
        (
            text("NAME", x(), Unresolved::NotDefined, acme),
            "NAME: not in Acme Engineering (dot 'D', mcp:x)",
        ),
        (
            text("NAME", x(), Unresolved::NotDefined, None),
            "NAME: not in Personal (dot 'D', mcp:x)",
        ),
        (
            text("NAME", x(), Unresolved::DeniedByPolicy, acme),
            "NAME: Acme Engineering's policy does not allow it (dot 'D')",
        ),
        (
            text(
                "NAME",
                Consumer::Shell,
                Unresolved::ShellSecretsForbidden,
                acme,
            ),
            "NAME: Acme Engineering does not allow secrets in the shell (dot 'D')",
        ),
        (
            text("NAME", x(), Unresolved::OrgExpired, acme),
            "Acme Engineering: secrets expired offline; reconnect to refresh",
        ),
        (
            text("NAME", x(), Unresolved::OrgRevoked, acme),
            "Acme Engineering: access revoked",
        ),
        (
            text(
                "NAME",
                x(),
                Unresolved::PolicyInvalid("signature does not match".into()),
                acme,
            ),
            "Acme Engineering: policy invalid: signature does not match",
        ),
        (
            text("NAME", x(), Unresolved::OrgOffline, acme),
            "Acme Engineering: offline, with no saved copy",
        ),
        (
            text("NAME", x(), Unresolved::OrgRemoved, Some(ACME)),
            "dot 'D': its org 'acme-eng' was removed",
        ),
    ];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}
