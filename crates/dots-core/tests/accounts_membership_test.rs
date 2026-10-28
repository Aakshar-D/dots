//! This PC's place on an account's signed member list (members spec §6.1, §6.4, §6.6): after
//! every accepted fetch the `Orgs` service works out `legacy`, `waiting`, `active` or
//! `removed`, keeps it in the `orgs` row, and keeps the accepted list in the offline copy. The
//! fetches go to the fake OpenBao, and the policies and lists are signed with minisign.

mod common;

use chrono::Duration;
use common::orgs::{
    keys, policy, public_key, roster, sign, sign_roster, token, OrgsHarness, ACME, ACME_NAME,
    ISSUED,
};
use common::spec;
use dots_core::accounts::{
    ensure_identity, verify_roster, Membership, NotMember, Role, WorkspaceRole,
};
use dots_core::vault::org::{read_offline, OrgLookup, OrgStatus};
use dots_core::vault::{GrantState, Grants, SecretStore};
use dots_core::Error;
use minisign::KeyPair;
use serde_json::{json, Value};
use zeroize::Zeroizing;

const API_KEY: &str = "org-secret-member-key-5b81d2";
/// A policy issued after `ISSUED`, for the account's switch to a member list.
const LATER: &str = "2026-10-22T08:30:00Z";
const WAITING_TEXT: &str =
    "Acme Engineering: you aren't on its member list yet. Send your Dots ID to an Admin.";

fn entries() -> Value {
    json!({"API_KEY": API_KEY})
}

/// Policy v2 asking for a member list, signed by the admins `admins` too.
fn required(admins: &[String]) -> Value {
    json!({"v": 2, "roster": "required", "admins": admins})
}

/// A member of the list with the Dots ID `key`.
fn member(id: &str, email: &str, role: &str, key: &str) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

/// This PC's Dots ID, made as the runtime makes it at start.
async fn me(h: &OrgsHarness) -> String {
    ensure_identity(&h.store, h.secrets.as_ref())
        .await
        .unwrap()
        .public_key
}

/// Someone else's Dots ID.
fn stranger() -> String {
    public_key(&keys())
}

/// A list at `seq`: an owner who is not this PC, then `others`.
fn list(seq: u64, others: &[Value]) -> String {
    let mut members = vec![member("01OWNER", "owner@acme.com", "owner", &stranger())];
    members.extend(others.iter().cloned());
    roster(ACME, seq, Value::Array(members))
}

/// Serves Acme's entries, its policy issued at `issued_at` with `extra`, and `signed_roster`
/// (none when `None`).
fn serve(h: &OrgsHarness, issued_at: &str, extra: Value, signed_roster: Option<&str>) {
    let signed = sign(&h.keys, &policy(ACME, issued_at, extra));
    h.serve("dots", entries(), &signed);
    match signed_roster {
        Some(text) => h.bao.put_roster("secret", "dots", text),
        None => h.bao.remove("secret", "dots/roster"),
    }
}

async fn join(h: &OrgsHarness) {
    h.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
}

/// Joins with this PC on a list at seq 1 as an Admin; the list's text and its signed text.
async fn join_active(h: &OrgsHarness) -> (String, String) {
    let mine = me(h).await;
    let text = list(1, &[member("01ME", "me@acme.com", "admin", &mine)]);
    let signed = sign_roster(&h.keys, &text);
    serve(h, ISSUED, required(&[]), Some(&signed));
    join(h).await;
    (text, signed)
}

fn value() -> OrgLookup {
    OrgLookup::Value {
        value: Zeroizing::new(API_KEY.into()),
        is_plain: false,
    }
}

#[tokio::test]
async fn v1_policy_is_legacy() {
    let h = OrgsHarness::new().await;
    me(&h).await;
    // A list that leaves this PC out is ignored under a v1 policy.
    let text = list(1, &[]);
    serve(&h, ISSUED, json!({}), Some(&sign_roster(&h.keys, &text)));
    join(&h).await;

    let summary = h.orgs.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Legacy);
    assert_eq!(summary.role, None);
    assert_eq!(summary.roster_seq, None);
    assert_eq!(summary.roster_error, None);
    assert_eq!(summary.member_count, None);
    assert_eq!(h.orgs.access(ACME), Some((Membership::Legacy, None)));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.member.membership, Membership::Legacy);
    assert_eq!(row.member.last_roster_seq, None);
    assert_eq!(h.orgs.access("beta"), None);
}

#[tokio::test]
async fn required_and_on_list_is_active_with_role() {
    let h = OrgsHarness::new().await;
    let (text, signed) = join_active(&h).await;
    // What the harness signs passes the real check.
    let accepted = verify_roster(&signed, &public_key(&h.keys), &[], ACME).unwrap();
    assert_eq!(accepted.text, text);

    let summary = h.orgs.summary(ACME).await.unwrap();
    assert_eq!(summary.status, OrgStatus::Connected);
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Admin));
    assert_eq!(summary.roster_seq, Some(1));
    assert_eq!(summary.member_count, Some(2));
    assert_eq!(summary.roster_error, None);
    assert_eq!(
        h.orgs.access(ACME),
        Some((Membership::Active, Some(Role::Admin)))
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());

    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.member.membership, Membership::Active);
    assert_eq!(row.member.member_id.as_deref(), Some("01ME"));
    assert_eq!(row.member.role, Some(Role::Admin));
    assert_eq!(
        row.member.workspaces.get("default"),
        Some(&WorkspaceRole::Member)
    );
    assert_eq!(row.member.last_roster_seq, Some(1));
    // The accepted signed list is kept beside the policy.
    let copy = read_offline(&h.vault_dir(), h.protector.as_ref(), ACME)
        .unwrap()
        .unwrap();
    assert_eq!(copy.roster, Some(signed));
}

#[tokio::test]
async fn required_not_on_list_first_time_is_waiting_nothing_wiped() {
    let h = OrgsHarness::new().await;
    me(&h).await;
    serve(&h, ISSUED, json!({}), None);
    join(&h).await;
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());

    // The owner switches to a member list before this PC is on it.
    let text = list(1, &[]);
    serve(&h, LATER, required(&[]), Some(&sign_roster(&h.keys, &text)));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Waiting);
    assert_eq!(summary.status, OrgStatus::Connected);
    assert_eq!(summary.role, None);
    assert_eq!(summary.roster_seq, Some(1));
    assert!(h.copy_path(ACME).exists(), "the copy is kept");
    assert_eq!(
        h.orgs.lookup(ACME, "API_KEY").await,
        OrgLookup::NotMember(NotMember::Waiting)
    );
    assert!(h.orgs.entries(ACME).await.unwrap().is_empty());

    // A required list that is missing, never accepted: waiting too, with why.
    let h = OrgsHarness::new().await;
    me(&h).await;
    serve(&h, ISSUED, required(&[]), None);
    join(&h).await;
    let summary = h.orgs.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Waiting);
    assert_eq!(summary.roster_seq, None);
    assert!(summary.roster_error.is_some());
    assert!(h.copy_path(ACME).exists());
}

#[tokio::test]
async fn missing_identity_while_active_stays_active() {
    let h = OrgsHarness::new().await;
    join_active(&h).await;
    // The Dots ID can't be read when the next list is evaluated, and the list no longer names
    // it: nothing changes, and the summary says why.
    h.store
        .set_marker("identity_public_key", &Value::Null, false)
        .await
        .unwrap();
    let text = list(2, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Admin));
    assert_eq!(summary.roster_seq, Some(1));
    assert!(summary
        .roster_error
        .as_deref()
        .is_some_and(|e| e.starts_with("this PC's Dots ID can't be read")));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());
    assert!(h.copy_path(ACME).exists(), "nothing is wiped");

    // A first join with no Dots ID at all is waiting as well.
    let h = OrgsHarness::new().await;
    let text = list(1, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    join(&h).await;
    assert_eq!(
        h.orgs.summary(ACME).await.unwrap().membership,
        Membership::Waiting
    );
}

#[tokio::test]
async fn active_then_dropped_is_removed_and_wiped() {
    let h = OrgsHarness::new().await;
    let mine = me(&h).await;
    join_active(&h).await;
    assert!(h.copy_path(ACME).exists());

    let text = list(2, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Removed);
    assert_eq!(summary.status_text, "Removed from Acme Engineering");
    assert_eq!(summary.role, None);
    assert_eq!(summary.copy_until, None);
    assert!(!h.copy_path(ACME).exists(), "the copy is wiped");
    assert_eq!(
        h.orgs.lookup(ACME, "API_KEY").await,
        OrgLookup::NotMember(NotMember::Removed)
    );
    assert_eq!(h.orgs.access(ACME), Some((Membership::Removed, None)));
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.member.membership, Membership::Removed);
    assert_eq!(row.member.role, None);

    // It stays removed until the person joins again: after a restart, and when a later list
    // names this PC again.
    let restarted = h.service();
    restarted.load().await.unwrap();
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Removed);
    assert_eq!(summary.status_text, "Removed from Acme Engineering");
    let text = list(3, &[member("01ME", "me@acme.com", "member", &mine)]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Removed);
    assert_eq!(
        restarted.lookup(ACME, "API_KEY").await,
        OrgLookup::NotMember(NotMember::Removed)
    );

    // Leaving and joining again starts over.
    restarted.remove(ACME).await.unwrap();
    restarted.join(&h.acme_code(), Some(token())).await.unwrap();
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Member));
}

#[tokio::test]
async fn bad_roster_after_accepted_keeps_membership_and_reports() {
    let h = OrgsHarness::new().await;
    let (_, signed) = join_active(&h).await;

    // Signed by someone who is neither the owner nor an admin, and leaving this PC out.
    let forged = sign_roster(&keys(), &list(2, &[]));
    serve(&h, ISSUED, required(&[]), Some(&forged));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Admin));
    assert_eq!(summary.roster_seq, Some(1));
    assert_eq!(summary.member_count, Some(2));
    assert_eq!(
        summary.roster_error.as_deref(),
        Some("the member list is not signed by the owner or an admin")
    );
    assert_eq!(
        summary.status_text,
        "connected; member list not accepted: the member list is not signed by the owner or an \
         admin"
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());
    // The new copy keeps the last accepted list.
    let copy = read_offline(&h.vault_dir(), h.protector.as_ref(), ACME)
        .unwrap()
        .unwrap();
    assert_eq!(copy.roster, Some(signed));

    // A list that is gone: the same.
    serve(&h, ISSUED, required(&[]), None);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert!(summary.roster_error.is_some());
    assert!(h.copy_path(ACME).exists());

    // After a restart the kept list is checked again and still counts.
    let restarted = h.service();
    restarted.load().await.unwrap();
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.member_count, Some(2));
    assert_eq!(summary.roster_error, None);

    // The list goes with the copy when the copy runs out (offline_hours, 24 by default).
    h.advance(Duration::hours(25));
    h.bao
        .fail_next(common::fake_openbao::Failure::Status(503), 1);
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(summary.status, OrgStatus::Expired);
}

#[tokio::test]
async fn older_seq_is_refused_keeps_previous() {
    let h = OrgsHarness::new().await;
    let mine = me(&h).await;
    let text = list(5, &[member("01ME", "me@acme.com", "viewer", &mine)]);
    let signed = sign_roster(&h.keys, &text);
    serve(&h, ISSUED, required(&[]), Some(&signed));
    join(&h).await;
    assert_eq!(h.orgs.summary(ACME).await.unwrap().roster_seq, Some(5));

    // The same list again is fine.
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.roster_error, None);
    assert_eq!(summary.membership, Membership::Active);

    // An older list that leaves this PC out is refused: no removal.
    let older = list(4, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &older)),
    );
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Viewer));
    assert_eq!(summary.roster_seq, Some(5));
    assert_eq!(
        summary.roster_error.as_deref(),
        Some("this member list (#4) is older than the one already accepted (#5)")
    );
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.member.last_roster_seq, Some(5));
}

#[tokio::test]
async fn admin_signed_roster_accepted_stranger_refused() {
    let h = OrgsHarness::new().await;
    let mine = me(&h).await;
    let admin: KeyPair = keys();
    let admins = vec![public_key(&admin)];
    let text = list(1, &[member("01ME", "me@acme.com", "member", &mine)]);
    serve(
        &h,
        ISSUED,
        required(&admins),
        Some(&sign_roster(&admin, &text)),
    );
    join(&h).await;
    let summary = h.orgs.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Member));
    assert_eq!(summary.roster_error, None);

    // A newer list from a key the policy doesn't name, which drops this PC, is refused.
    let forged = sign_roster(&keys(), &list(2, &[]));
    serve(&h, ISSUED, required(&admins), Some(&forged));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.roster_seq, Some(1));
    assert_eq!(
        summary.roster_error.as_deref(),
        Some("the member list is not signed by the owner or an admin")
    );
}

#[tokio::test]
async fn old_offline_copy_without_roster_loads() {
    let h = OrgsHarness::new().await;
    serve(&h, ISSUED, json!({}), None);
    join(&h).await;
    // A copy as phase 1 wrote it: no `roster` field.
    let signed = sign(&h.keys, &policy(ACME, ISSUED, json!({})));
    let fetched_at = common::orgs::stamp(h.now());
    let plain = json!({
        "v": 1, "fetched_at": fetched_at, "entries": entries(), "policy": signed
    })
    .to_string();
    let sealed = h
        .protector
        .protect(plain.as_bytes(), ACME.as_bytes())
        .unwrap();
    std::fs::write(h.copy_path(ACME), sealed).unwrap();

    let copy = read_offline(&h.vault_dir(), h.protector.as_ref(), ACME)
        .unwrap()
        .unwrap();
    assert_eq!(copy.roster, None);
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(restarted.lookup(ACME, "API_KEY").await, value());
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Legacy);
    assert!(matches!(
        summary.status,
        OrgStatus::Offline { until: Some(_) }
    ));
}

#[tokio::test]
async fn waiting_dot_run_refused_with_message() {
    let h = OrgsHarness::new().await;
    me(&h).await;
    let text = list(1, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    join(&h).await;
    let mut s = spec("triage");
    s.variables = Grants(
        [("shell".to_string(), vec!["API_KEY".to_string()])]
            .into_iter()
            .collect(),
    );
    let dot = h.store.create_dot(&s, Some(ACME)).await.unwrap();
    let vault = h.vault();

    match vault.before_run(&dot).await {
        Err(Error::Other(text)) => assert_eq!(text, WAITING_TEXT),
        other => panic!("expected the run refused, got {other:?}"),
    }
    let statuses = vault.grant_statuses(&dot).await.unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].status, GrantState::NotMember);
    assert_eq!(statuses[0].text.as_deref(), Some(WAITING_TEXT));
}

#[tokio::test]
async fn summary_carries_membership_and_role() {
    let h = OrgsHarness::new().await;
    join_active(&h).await;
    let summary = h.orgs.summary(ACME).await.unwrap();
    let json = serde_json::to_value(&summary).unwrap();
    assert_eq!(json["name"], ACME_NAME);
    assert_eq!(json["membership"], "active");
    assert_eq!(json["role"], "admin");
    assert_eq!(json["roster_seq"], 1);
    assert_eq!(json["roster_error"], Value::Null);
    assert_eq!(json["member_count"], 2);

    let legacy = OrgsHarness::new().await;
    serve(&legacy, ISSUED, json!({}), None);
    join(&legacy).await;
    let json = serde_json::to_value(legacy.orgs.summary(ACME).await.unwrap()).unwrap();
    assert_eq!(json["membership"], "legacy");
    assert_eq!(json["role"], Value::Null);
    assert_eq!(json["roster_seq"], Value::Null);
    assert_eq!(json["member_count"], Value::Null);
}

/// Policy v2 asking for a member list, with `more` fields over it.
fn required_with(admins: &[String], more: Value) -> Value {
    let mut policy = required(admins);
    for (key, value) in more.as_object().unwrap() {
        policy[key] = value.clone();
    }
    policy
}

/// Makes a new Dots ID for this PC, as `ensure_identity` does when the secret was lost.
async fn regenerate_identity(h: &OrgsHarness) -> String {
    let old = me(h).await;
    h.secrets.delete("identity").unwrap();
    let new = me(h).await;
    assert_ne!(new, old);
    new
}

#[tokio::test]
async fn admin_dropped_from_admins_keeps_the_accepted_list() {
    let h = OrgsHarness::new().await;
    let mine = me(&h).await;
    let admin = keys();
    let text = list(3, &[member("01ME", "me@acme.com", "member", &mine)]);
    let signed = sign_roster(&admin, &text);
    serve(&h, ISSUED, required(&[public_key(&admin)]), Some(&signed));
    join(&h).await;
    assert_eq!(h.orgs.summary(ACME).await.unwrap().roster_error, None);

    // The owner re-signs the policy without that admin; the list it signed stays accepted.
    serve(&h, LATER, required(&[]), Some(&signed));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.roster_error, None);
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.roster_seq, Some(3));

    let restarted = h.service();
    restarted.load().await.unwrap();
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(summary.roster_error, None);
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.member_count, Some(2));
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(summary.roster_error, None);
    assert_eq!(summary.membership, Membership::Active);

    // A new list from the dropped admin is refused.
    let newer = list(4, &[member("01ME", "me@acme.com", "viewer", &mine)]);
    serve(&h, LATER, required(&[]), Some(&sign_roster(&admin, &newer)));
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(
        summary.roster_error.as_deref(),
        Some("the member list is not signed by the owner or an admin")
    );
    assert_eq!(summary.role, Some(Role::Member));
}

#[tokio::test]
async fn regenerated_identity_waits_for_the_new_id_nothing_wiped() {
    let h = OrgsHarness::new().await;
    join_active(&h).await;
    let new = regenerate_identity(&h).await;

    // The same list: the old ID is on it, the new one is not.
    let summary = h.orgs.refresh(ACME).await.unwrap();
    let changed = "Acme Engineering: this PC's Dots ID changed. Send the new one to an Admin.";
    assert_eq!(summary.membership, Membership::Waiting);
    assert_eq!(summary.status_text, changed);
    assert_eq!(summary.role, None);
    assert!(h.copy_path(ACME).exists(), "nothing is wiped");
    assert_eq!(
        h.orgs.lookup(ACME, "API_KEY").await,
        OrgLookup::NotMember(NotMember::IdChanged)
    );
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.member.membership, Membership::Waiting);
    assert_eq!(row.member.member_id.as_deref(), Some("01ME"));
    let dot = h
        .store
        .create_dot(&spec("triage"), Some(ACME))
        .await
        .unwrap();
    match h.vault().before_run(&dot).await {
        Err(Error::Other(text)) => assert_eq!(text, changed),
        other => panic!("expected the run refused, got {other:?}"),
    }

    // A restart says the same.
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(restarted.summary(ACME).await.unwrap().status_text, changed);

    // The admin swaps in the new ID for the same member: active again.
    let text = list(2, &[member("01ME", "me@acme.com", "admin", &new)]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(summary.role, Some(Role::Admin));
    assert_eq!(restarted.lookup(ACME, "API_KEY").await, value());
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.member.key.as_deref(), Some(new.as_str()));
}

#[tokio::test]
async fn regenerated_identity_and_member_gone_is_removed() {
    let h = OrgsHarness::new().await;
    join_active(&h).await;
    regenerate_identity(&h).await;
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Waiting);
    // The admin takes the member off: the old ID and the member are both gone.
    let text = list(2, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Removed);
    assert!(!h.copy_path(ACME).exists());
}

#[tokio::test]
async fn equal_seq_with_another_text_after_restart_without_copy_is_refused() {
    let h = OrgsHarness::new().await;
    let mine = me(&h).await;
    let no_copy = required_with(&[], json!({"offline_hours": 0}));
    let text = list(3, &[member("01ME", "me@acme.com", "admin", &mine)]);
    let signed = sign_roster(&h.keys, &text);
    serve(&h, ISSUED, no_copy.clone(), Some(&signed));
    join(&h).await;
    assert!(!h.copy_path(ACME).exists());

    let restarted = h.service();
    restarted.load().await.unwrap();
    // The same list again is known by its hash.
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(summary.roster_error, None);
    assert_eq!(summary.role, Some(Role::Admin));

    // Another list of the same seq is refused.
    let other = list(3, &[member("01ME", "me@acme.com", "viewer", &mine)]);
    serve(&h, ISSUED, no_copy, Some(&sign_roster(&h.keys, &other)));
    let summary = restarted.refresh(ACME).await.unwrap();
    assert_eq!(
        summary.roster_error.as_deref(),
        Some("this member list (#3) is older than the one already accepted (#3)")
    );
    assert_eq!(summary.role, Some(Role::Admin));
}

#[tokio::test]
async fn active_then_policy_off_is_legacy() {
    let h = OrgsHarness::new().await;
    join_active(&h).await;
    serve(&h, LATER, json!({"v": 2}), None);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Legacy);
    assert_eq!(summary.role, None);
    assert_eq!(summary.member_count, None);
    assert_eq!(h.orgs.access(ACME), Some((Membership::Legacy, None)));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());
}

#[tokio::test]
async fn unreachable_or_invalid_while_active_is_never_removed() {
    let h = OrgsHarness::new().await;
    join_active(&h).await;
    h.bao
        .fail_next(common::fake_openbao::Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert!(matches!(summary.status, OrgStatus::Offline { .. }));
    assert_eq!(summary.membership, Membership::Active);

    // A policy signed by another key is refused; the list is not looked at.
    let forged = sign(&keys(), &policy(ACME, LATER, required(&[])));
    h.serve("dots", entries(), &forged);
    h.bao
        .put_roster("secret", "dots", &sign_roster(&h.keys, &list(2, &[])));
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert!(matches!(summary.status, OrgStatus::PolicyInvalid { .. }));
    assert_eq!(summary.membership, Membership::Active);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value());
    assert!(h.copy_path(ACME).exists());
}

#[tokio::test]
async fn denied_while_waiting_stays_waiting() {
    let h = OrgsHarness::new().await;
    me(&h).await;
    let text = list(1, &[]);
    serve(
        &h,
        ISSUED,
        required(&[]),
        Some(&sign_roster(&h.keys, &text)),
    );
    join(&h).await;
    h.bao.revoke_token(common::orgs::TOKEN);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(summary.membership, Membership::Waiting);
    assert_ne!(summary.status, OrgStatus::Revoked);
    assert!(h.copy_path(ACME).exists(), "nothing is wiped");
    assert_eq!(
        h.orgs.lookup(ACME, "API_KEY").await,
        OrgLookup::NotMember(NotMember::Waiting)
    );
}
