//! A workspace's optional `use` rules narrow which account entries its dots may use (members
//! spec §6.3): after the account's own, and never wider.

mod common;

use common::orgs::{OrgsHarness, ACME};
use dots_core::store::WorkspaceRow;
use dots_core::vault::org::{may_use_in, UseRules};
use dots_core::vault::{Consumer, DotScope, Grants, Unresolved, Vault};
use serde_json::json;

fn rules(allow: &[&str], deny: &[&str]) -> UseRules {
    UseRules {
        allow: allow.iter().map(|s| s.to_string()).collect(),
        deny: deny.iter().map(|s| s.to_string()).collect(),
    }
}

/// Acme joined with the entries `STRIPE_KEY`, `API_KEY` and `DB_URL` under a policy of `extra`,
/// and the workspace `pay` with `use` rules `workspace`.
async fn setup(extra: serde_json::Value, workspace: Option<UseRules>) -> (OrgsHarness, Vault) {
    let h = OrgsHarness::new().await;
    h.join_acme(
        json!({"STRIPE_KEY": "s-1", "API_KEY": "a-1", "DB_URL": "d-1"}),
        extra,
    )
    .await;
    h.store
        .add_workspace(&WorkspaceRow {
            scope: ACME.into(),
            id: "pay".into(),
            name: "Payments".into(),
            use_rules: workspace,
            created_at: "2026-10-30T00:00:00Z".into(),
        })
        .await
        .unwrap();
    let vault = h.vault();
    (h, vault)
}

/// `name` resolved for a dot of the workspace `pay`: its value, or why not and the text.
async fn resolve(vault: &Vault, name: &str) -> Result<String, (Unresolved, String)> {
    let consumer = Consumer::Shell;
    let grants = Grants([(consumer.key(), vec![name.to_string()])].into());
    let names = vec![name.to_string()];
    match vault
        .resolve_in(
            &DotScope::Org(ACME.into()),
            "pay",
            &grants,
            &consumer,
            &names,
        )
        .await
        .unwrap()
    {
        Ok(resolved) => Ok(resolved.get(name).unwrap()),
        Err(missing) => {
            let text = missing.text("triage");
            Err((missing.why, text))
        }
    }
}

#[tokio::test]
async fn workspace_use_narrows_policy() {
    let (_h, vault) = setup(
        json!({"shell_secrets": true}),
        Some(rules(&["STRIPE_*"], &[])),
    )
    .await;
    assert_eq!(resolve(&vault, "STRIPE_KEY").await.unwrap(), "s-1");
    assert_eq!(
        resolve(&vault, "API_KEY").await.unwrap_err(),
        (
            Unresolved::DeniedByWorkspace("Payments".into()),
            "the Payments workspace doesn't allow API_KEY".into()
        )
    );
}

#[tokio::test]
async fn workspace_deny_adds_to_policy_deny() {
    let extra = json!({"shell_secrets": true, "use": {"deny": ["DB_*"]}});
    let (_h, vault) = setup(extra, Some(rules(&["*"], &["API_*"]))).await;
    assert_eq!(resolve(&vault, "STRIPE_KEY").await.unwrap(), "s-1");
    assert!(matches!(
        resolve(&vault, "API_KEY").await.unwrap_err().0,
        Unresolved::DeniedByWorkspace(_)
    ));
    assert_eq!(
        resolve(&vault, "DB_URL").await.unwrap_err().0,
        Unresolved::DeniedByPolicy
    );
}

#[tokio::test]
async fn no_workspace_use_changes_nothing() {
    let (_h, vault) = setup(json!({"shell_secrets": true}), None).await;
    for (name, value) in [("STRIPE_KEY", "s-1"), ("API_KEY", "a-1"), ("DB_URL", "d-1")] {
        assert_eq!(resolve(&vault, name).await.unwrap(), value);
    }
}

#[tokio::test]
async fn policy_deny_still_wins() {
    // The workspace allows API_KEY, but the account's policy denies it: the policy's own words.
    let extra = json!({"shell_secrets": true, "use": {"deny": ["API_*"]}});
    let (_h, vault) = setup(extra, Some(rules(&["*"], &[]))).await;
    let (why, text) = resolve(&vault, "API_KEY").await.unwrap_err();
    assert_eq!(why, Unresolved::DeniedByPolicy);
    assert_eq!(
        text,
        "API_KEY: Acme Engineering's policy does not allow it (dot 'triage')"
    );
    assert_eq!(resolve(&vault, "STRIPE_KEY").await.unwrap(), "s-1");
}

#[test]
fn may_use_in_needs_both_to_allow() {
    let policy = rules(&["*"], &["SECRET_*"]);
    assert!(may_use_in(&policy, None, "ANY"));
    assert!(!may_use_in(&policy, None, "secret_x"));
    let ws = rules(&["A_*", "SECRET_*"], &["A_PRIVATE"]);
    assert!(may_use_in(&policy, Some(&ws), "a_one"));
    assert!(!may_use_in(&policy, Some(&ws), "A_PRIVATE"));
    assert!(!may_use_in(&policy, Some(&ws), "SECRET_X"));
    assert!(!may_use_in(&policy, Some(&ws), "B_ONE"));
}
