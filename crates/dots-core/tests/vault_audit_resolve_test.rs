//! A record for each resolution attempt in an org (env vault spec §10): a run records every
//! name it resolves for a consumer and the first it cannot, with the dot and the run; a Library
//! check in the account records with neither; a Personal dot records nothing. No record holds
//! a value.

mod common;

use common::orgs::{OrgsHarness, ACME};
use dots_core::accounts::NotMember;
use dots_core::vault::audit::writer::Usage;
use dots_core::vault::audit::{AuditDot, AuditRecord, Event};
use dots_core::vault::{Consumer, DotScope, EntryKind, Grants, Unresolved, Vault};
use serde_json::{json, Value};

const API_KEY: &str = "org-secret-api-key-61f0d3";
const LEVEL: &str = "org-plain-level-2c7a90";
const PERSONAL: &str = "personal-secret-only-4e8b17";

fn entries() -> Value {
    json!({"API_KEY": API_KEY, "LOG_LEVEL": LEVEL, "DOTS_AUDIT_TOKEN": "audit-token-1d3f"})
}

/// Acme with `extra` in its policy, `LOG_LEVEL` plain and audit on; Personal holds `IN_PERSONAL`.
async fn setup(extra: Value) -> (OrgsHarness, Vault) {
    let h = OrgsHarness::new().await;
    let mut extra = extra;
    extra["plain"] = json!(["LOG_LEVEL"]);
    extra["audit"] =
        json!({"endpoint": "https://audit.acme.test/dots", "token_entry": "DOTS_AUDIT_TOKEN"});
    h.join_acme(entries(), extra).await;
    let vault = h.vault();
    vault
        .add("IN_PERSONAL", EntryKind::Secret, PERSONAL)
        .await
        .unwrap();
    (h, vault)
}

fn grants(consumer: &str, names: &[&str]) -> Grants {
    Grants(
        [(
            consumer.to_string(),
            names.iter().map(|n| n.to_string()).collect(),
        )]
        .into(),
    )
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

fn usage() -> Usage {
    Usage::run("01J9DOT", "Triage", "01J9RUN")
}

/// The records after the join's `fetched`, as (event, name, consumer, source, reason).
async fn resolutions(h: &OrgsHarness) -> Vec<AuditRecord> {
    let batch = h.store.audit_batch(ACME, 500).await.unwrap();
    for (_, line) in &batch {
        for value in [API_KEY, LEVEL, PERSONAL] {
            assert!(!line.contains(value), "{line} holds {value}");
        }
    }
    batch
        .iter()
        .map(|(_, line)| AuditRecord::parse(line).unwrap())
        .filter(|r| r.event != Event::Fetched)
        .collect()
}

type Row = (
    Event,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn row(r: &AuditRecord) -> Row {
    (
        r.event,
        r.name.clone(),
        r.consumer.clone(),
        r.source.clone(),
        r.reason.clone(),
    )
}

fn s(text: &str) -> Option<String> {
    Some(text.to_string())
}

#[tokio::test]
async fn a_run_records_each_name_it_resolves_and_the_first_it_cannot() {
    let (h, vault) = setup(json!({})).await;
    let scope = DotScope::Org(ACME.into());
    let consumer = Consumer::Mcp("github".into());
    let granted = grants("mcp:github", &["LOG_LEVEL", "API_KEY", "MISSING", "AFTER"]);
    let asked = names(&["LOG_LEVEL", "API_KEY", "MISSING", "AFTER"]);
    let missing = vault
        .resolve_in(&scope, "default", &granted, &consumer, &asked, &usage())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.why, Unresolved::NotDefined);

    let records = resolutions(&h).await;
    assert_eq!(
        records.iter().map(row).collect::<Vec<_>>(),
        vec![
            (
                Event::Resolved,
                s("LOG_LEVEL"),
                s("mcp:github"),
                s("openbao"),
                None
            ),
            (
                Event::Resolved,
                s("API_KEY"),
                s("mcp:github"),
                s("openbao"),
                None
            ),
            (
                Event::Denied,
                s("MISSING"),
                s("mcp:github"),
                s("openbao"),
                s("not_defined")
            ),
        ]
    );
    for r in &records {
        assert_eq!(
            r.dot,
            Some(AuditDot {
                id: "01J9DOT".into(),
                name: "Triage".into()
            })
        );
        assert_eq!(r.run_id.as_deref(), Some("01J9RUN"));
    }
    // History lists them, newest first.
    let shown: Vec<Option<String>> = h
        .store
        .audit_history(ACME, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(shown, vec![s("MISSING"), s("API_KEY"), s("LOG_LEVEL")]);
}

#[tokio::test]
async fn a_personal_value_the_policy_lets_in_is_recorded_as_personal() {
    let (h, vault) = setup(json!({"personal": "fallback"})).await;
    let scope = DotScope::Org(ACME.into());
    let granted = grants("shell", &["IN_PERSONAL"]);
    vault
        .resolve_in(
            &scope,
            "default",
            &granted,
            &Consumer::Shell,
            &names(&["LOG_LEVEL"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap_err();
    let granted = grants("mcp:x", &["IN_PERSONAL"]);
    vault
        .resolve_in(
            &scope,
            "default",
            &granted,
            &Consumer::Mcp("x".into()),
            &names(&["IN_PERSONAL"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        resolutions(&h).await.iter().map(row).collect::<Vec<_>>(),
        vec![
            (
                Event::Denied,
                s("LOG_LEVEL"),
                s("shell"),
                s("openbao"),
                s("not_granted")
            ),
            (
                Event::Resolved,
                s("IN_PERSONAL"),
                s("mcp:x"),
                s("personal"),
                None
            ),
        ]
    );
}

#[tokio::test]
async fn a_secret_kept_from_the_shell_is_denied_with_its_code() {
    let (h, vault) = setup(json!({})).await;
    let scope = DotScope::Org(ACME.into());
    let granted = grants("shell", &["API_KEY"]);
    vault
        .resolve_in(
            &scope,
            "default",
            &granted,
            &Consumer::Shell,
            &names(&["API_KEY"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        resolutions(&h).await.iter().map(row).collect::<Vec<_>>(),
        vec![(
            Event::Denied,
            s("API_KEY"),
            s("shell"),
            s("openbao"),
            s("shell_secrets_forbidden")
        )]
    );
}

#[tokio::test]
async fn a_personal_dot_writes_no_record() {
    let (h, vault) = setup(json!({})).await;
    let granted = grants("shell", &["IN_PERSONAL"]);
    vault
        .resolve_in(
            &DotScope::Personal,
            "default",
            &granted,
            &Consumer::Shell,
            &names(&["IN_PERSONAL"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolutions(&h).await, vec![]);
    assert_eq!(
        h.store.audit_history("personal", 100).await.unwrap(),
        vec![]
    );
}

#[tokio::test]
async fn a_check_in_the_account_records_with_no_dot_or_run() {
    let (h, vault) = setup(json!({})).await;
    let scope = DotScope::Org(ACME.into());
    vault
        .check_values(
            &scope,
            &Consumer::Mcp("github".into()),
            &names(&["API_KEY"]),
        )
        .await
        .unwrap()
        .unwrap();
    let records = resolutions(&h).await;
    assert_eq!(
        records.iter().map(row).collect::<Vec<_>>(),
        vec![(
            Event::Resolved,
            s("API_KEY"),
            s("mcp:github"),
            s("openbao"),
            None
        )]
    );
    assert_eq!(
        (records[0].dot.clone(), records[0].run_id.clone()),
        (None, None)
    );
}

#[test]
fn every_reason_has_its_code() {
    let codes: Vec<&str> = [
        Unresolved::NotGranted,
        Unresolved::NotDefined,
        Unresolved::DeniedByPolicy,
        Unresolved::DeniedByWorkspace("ops".into()),
        Unresolved::ShellSecretsForbidden,
        Unresolved::OrgOffline,
        Unresolved::OrgExpired,
        Unresolved::OrgRevoked,
        Unresolved::PolicyInvalid("x".into()),
        Unresolved::OrgRemoved,
        Unresolved::NotMember(NotMember::Waiting),
    ]
    .iter()
    .map(Unresolved::code)
    .collect();
    assert_eq!(
        codes,
        vec![
            "not_granted",
            "not_defined",
            "denied_by_policy",
            "denied_by_workspace",
            "shell_secrets_forbidden",
            "org_offline",
            "org_expired",
            "org_revoked",
            "policy_invalid",
            "org_removed",
            "not_member",
        ]
    );
}

/// The shell asks `orgs` for `API_KEY`; why it is refused.
async fn refused(h: &OrgsHarness, orgs: &dots_core::vault::org::Orgs) -> Unresolved {
    let vault = Vault::new(h.store.clone(), h.secrets.clone()).with_orgs(orgs.clone());
    vault
        .resolve_in(
            &DotScope::Org(ACME.into()),
            "default",
            &grants("shell", &["API_KEY"]),
            &Consumer::Shell,
            &names(&["API_KEY"]),
            &usage(),
        )
        .await
        .unwrap()
        .unwrap_err()
        .why
}

/// A half-done Remove leaves the account damaged for the session: a run against it is refused
/// as `policy_invalid`, and that attempt is recorded and queued like any other.
#[tokio::test]
async fn a_half_removed_account_still_records_each_attempt() {
    let (h, _vault) = setup(json!({})).await;
    let copy = h.copy_path(ACME);
    std::fs::remove_file(&copy).unwrap();
    std::fs::create_dir(&copy).unwrap();
    std::fs::write(copy.join("held"), "x").unwrap();
    h.orgs.remove(ACME).await.unwrap_err();

    assert!(matches!(
        refused(&h, &h.orgs).await,
        Unresolved::PolicyInvalid(_)
    ));
    let records = resolutions(&h).await;
    assert_eq!(
        records.iter().map(row).collect::<Vec<_>>(),
        vec![(
            Event::Denied,
            s("API_KEY"),
            s("shell"),
            s("openbao"),
            s("policy_invalid")
        )]
    );
    assert_eq!(records[0].seq, 2);
    let history = h.store.audit_history(ACME, 100).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, dots_core::store::HistoryState::Queued);
    std::fs::remove_dir_all(&copy).unwrap();
}

/// An account whose stored settings don't read still records a run's refused attempt, queued
/// by its last accepted target; its source kind can't be read, so `source` is null.
#[tokio::test]
async fn an_account_with_damaged_settings_still_records_each_attempt() {
    let (h, _vault) = setup(json!({})).await;
    let pool = sqlx::sqlite::SqlitePool::connect(&format!(
        "sqlite://{}",
        h.data.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE orgs SET source = ? WHERE id = ?")
        .bind(json!({"kind": "dropbox", "folder": "x"}).to_string())
        .bind(ACME)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let restarted = h.service();
    restarted.load().await.unwrap();

    assert!(matches!(
        refused(&h, &restarted).await,
        Unresolved::PolicyInvalid(_)
    ));
    let records = resolutions(&h).await;
    assert_eq!(
        records.iter().map(row).collect::<Vec<_>>(),
        vec![(
            Event::Denied,
            s("API_KEY"),
            s("shell"),
            None,
            s("policy_invalid")
        )]
    );
    assert_eq!(records[0].seq, 2);
}
