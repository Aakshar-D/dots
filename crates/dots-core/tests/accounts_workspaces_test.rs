//! An account's workspaces are mirrored from its accepted member list on every fetch, each PC
//! sees only the ones it may, and the current scope can pick one (accounts design §6.3, §9).
//! The lists come from the fake OpenBao, signed with minisign.

mod common;

use common::orgs::{keys, policy, public_key, sign, sign_roster, token, OrgsHarness, ACME, ISSUED};
use common::spec;
use dots_core::accounts::{
    current_scope, ensure_identity, set_current_scope, visible_workspaces, CurrentScope,
    WorkspaceRole,
};
use dots_core::Error;
use serde_json::{json, Value};

/// A list at `seq` with the workspaces `default` and `payments` (the latter limited to `PAY_*`
/// entries), an owner who is not this PC, and this PC as `role` with `mine` workspace entries.
fn list(seq: u64, workspaces: Value, role: &str, mine: Value, key: &str) -> String {
    json!({
        "v": 1, "org_id": ACME, "issued_at": "2026-10-28T09:00:00Z", "seq": seq,
        "workspaces": workspaces,
        "members": [
            {"id": "01OWNER", "name": "o", "email": "owner@acme.com", "role": "owner",
             "keys": [public_key(&keys())], "workspaces": {"default": "admin"},
             "added_at": "2026-10-28T09:00:00Z"},
            {"id": "01ME", "name": "me", "email": "me@acme.com", "role": role,
             "keys": [key], "workspaces": mine, "added_at": "2026-10-28T09:00:00Z"}
        ]
    })
    .to_string()
}

fn both() -> Value {
    json!([
        {"id": "default", "name": "Default"},
        {"id": "payments", "name": "Payments", "use": {"allow": ["PAY_*"], "deny": ["PAY_ROOT"]}}
    ])
}

fn only_default() -> Value {
    json!([{"id": "default", "name": "Default"}])
}

fn serve(h: &OrgsHarness, text: &str) {
    let policy = policy(ACME, ISSUED, json!({"v": 2, "roster": "required"}));
    h.serve("dots", json!({"A": "1"}), &sign(&h.keys, &policy));
    h.bao
        .put_roster("secret", "dots", &sign_roster(&h.keys, text));
}

async fn me(h: &OrgsHarness) -> String {
    ensure_identity(&h.store, h.secrets.as_ref())
        .await
        .unwrap()
        .public_key
}

/// Joins as `role` with `mine` entries on a list with both workspaces.
async fn joined(role: &str, mine: Value) -> OrgsHarness {
    let h = OrgsHarness::new().await;
    let key = me(&h).await;
    serve(&h, &list(1, both(), role, mine, &key));
    h.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    h
}

async fn ids(h: &OrgsHarness) -> Vec<String> {
    visible_workspaces(&h.store, &h.orgs, ACME)
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect()
}

#[tokio::test]
async fn accepted_list_mirrors_workspaces() {
    let h = joined("member", json!({"default": "member", "payments": "member"})).await;
    let rows = h.store.list_workspaces(ACME).await.unwrap();
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.name.as_str()))
        .collect();
    assert_eq!(got, [("default", "Default"), ("payments", "Payments")]);
    let rules = rows[1].use_rules.clone().unwrap();
    assert_eq!(rules.allow, ["PAY_*"]);
    assert_eq!(rules.deny, ["PAY_ROOT"]);
    assert!(rows[0].use_rules.is_none());

    let views = visible_workspaces(&h.store, &h.orgs, ACME).await.unwrap();
    assert_eq!(views[1].my_role, Some(WorkspaceRole::Member));
    assert!(!views[1].can_manage);
}

#[tokio::test]
async fn refused_list_does_not_mirror() {
    let h = joined("member", json!({"default": "member", "payments": "member"})).await;
    // A list nobody signed: refused, so the rows stay.
    let key = me(&h).await;
    let text = list(
        2,
        only_default(),
        "member",
        json!({"default": "member"}),
        &key,
    );
    let other = keys();
    h.bao
        .put_roster("secret", "dots", &sign_roster(&other, &text));
    h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(h.store.list_workspaces(ACME).await.unwrap().len(), 2);
}

#[tokio::test]
async fn vanished_workspace_moves_dots_to_default() {
    let h = joined("member", json!({"default": "member", "payments": "member"})).await;
    let dot = h
        .store
        .create_dot(&spec("pay"), Some(ACME), "payments")
        .await
        .unwrap();
    set_current_scope(&h.store, &h.orgs, ACME, "payments")
        .await
        .unwrap();
    assert_eq!(
        current_scope(&h.store, &h.orgs).await.unwrap().workspace,
        "payments"
    );

    let key = me(&h).await;
    serve(
        &h,
        &list(
            2,
            only_default(),
            "member",
            json!({"default": "member"}),
            &key,
        ),
    );
    h.orgs.refresh(ACME).await.unwrap();

    assert_eq!(
        h.store.get_dot(&dot.id).await.unwrap().workspace_id,
        "default"
    );
    assert_eq!(h.store.list_workspaces(ACME).await.unwrap().len(), 1);
    assert_eq!(
        current_scope(&h.store, &h.orgs).await.unwrap(),
        CurrentScope {
            account: ACME.into(),
            workspace: "default".into()
        }
    );
}

#[tokio::test]
async fn member_sees_only_their_workspaces() {
    let h = joined("member", json!({"default": "member"})).await;
    assert_eq!(ids(&h).await, ["default"]);
    let views = visible_workspaces(&h.store, &h.orgs, ACME).await.unwrap();
    assert_eq!(views[0].my_role, Some(WorkspaceRole::Member));
    assert!(!views[0].can_manage);

    // Added to payments by the next list.
    let key = me(&h).await;
    serve(
        &h,
        &list(
            2,
            both(),
            "member",
            json!({"default": "member", "payments": "viewer"}),
            &key,
        ),
    );
    h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(ids(&h).await, ["default", "payments"]);
}

#[tokio::test]
async fn admin_sees_all() {
    let h = joined("admin", json!({"default": "member"})).await;
    assert_eq!(ids(&h).await, ["default", "payments"]);
    let views = visible_workspaces(&h.store, &h.orgs, ACME).await.unwrap();
    assert_eq!(views[1].my_role, Some(WorkspaceRole::Admin));
    assert!(views[1].can_manage);
    assert!(views[1].use_rules.is_some());
}

#[tokio::test]
async fn personal_sees_its_rows_and_manages() {
    let h = OrgsHarness::new().await;
    let views = visible_workspaces(&h.store, &h.orgs, "personal")
        .await
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].my_role, None);
    assert!(views[0].can_manage);
}

#[tokio::test]
async fn scope_refuses_hidden_workspace() {
    let h = joined("member", json!({"default": "member"})).await;
    let err = set_current_scope(&h.store, &h.orgs, ACME, "payments")
        .await
        .unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == "no workspace 'payments' in this account"));
    let err = set_current_scope(&h.store, &h.orgs, ACME, "nope")
        .await
        .unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == "no workspace 'nope' in this account"));
    set_current_scope(&h.store, &h.orgs, ACME, "default")
        .await
        .unwrap();
}

#[tokio::test]
async fn scope_picks_a_visible_workspace() {
    let h = joined("admin", json!({"default": "member"})).await;
    let set = set_current_scope(&h.store, &h.orgs, ACME, "payments")
        .await
        .unwrap();
    assert_eq!(set.workspace, "payments");
    assert_eq!(current_scope(&h.store, &h.orgs).await.unwrap(), set);
}
