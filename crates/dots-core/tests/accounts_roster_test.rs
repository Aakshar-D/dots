//! The signed member list (members spec §6.1, §6.2): who may sign it, and each rule it keeps.

use std::io::Cursor;

use dots_core::accounts::{
    parse_roster, roster_envelope, verify_roster, Role, RosterError, WorkspaceRole,
};
use minisign::KeyPair;
use serde_json::{json, Value};

const ORG: &str = "acme-eng";

fn keys() -> KeyPair {
    KeyPair::generate_unencrypted_keypair().unwrap()
}

fn sign(keys: &KeyPair, roster: &str) -> String {
    let signature = minisign::sign(Some(&keys.pk), &keys.sk, Cursor::new(roster), None, None)
        .unwrap()
        .to_string();
    roster_envelope(roster, &signature)
}

fn member(id: &str, email: &str, role: &str, key: &str) -> Value {
    json!({
        "id": id, "name": "Asha Rao", "email": email, "role": role, "keys": [key],
        "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

/// A valid roster for ORG at `seq` 7, as JSON.
fn roster_json() -> Value {
    json!({
        "v": 1, "org_id": ORG, "issued_at": "2026-10-28T09:00:00Z", "seq": 7,
        "workspaces": [
            {"id": "default", "name": "Default"},
            {"id": "payments", "name": "Payments", "use": {"allow": ["STRIPE_*"], "deny": []}}
        ],
        "members": [member("01A", "asha@acme.com", "owner", "RWQowner0000000000")]
    })
}

fn invalid(roster: &Value) -> String {
    match parse_roster(&roster.to_string(), ORG) {
        Err(RosterError::Invalid(why)) => why,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn parses_a_valid_roster() {
    let roster = parse_roster(&roster_json().to_string(), ORG).unwrap();
    assert_eq!(roster.seq, 7);
    assert_eq!(roster.members[0].role, Role::Owner);
    assert_eq!(
        roster.members[0].workspaces["default"],
        WorkspaceRole::Member
    );
    assert_eq!(
        roster.workspaces[1].use_rules.as_ref().unwrap().allow,
        ["STRIPE_*"]
    );
}

#[test]
fn accepts_roster_signed_by_root() {
    let root = keys();
    let text = roster_json().to_string();
    let accepted = verify_roster(&sign(&root, &text), &root.pk.to_base64(), &[], ORG).unwrap();
    assert_eq!(accepted.text, text);
    assert_eq!(accepted.signer, root.pk.to_base64());
    assert_eq!(accepted.roster.org_id, ORG);
}

#[test]
fn accepts_roster_signed_by_admin_key() {
    let (root, other, admin) = (keys(), keys(), keys());
    let admins = vec![other.pk.to_base64(), admin.pk.to_base64()];
    let accepted = verify_roster(
        &sign(&admin, &roster_json().to_string()),
        &root.pk.to_base64(),
        &admins,
        ORG,
    )
    .unwrap();
    assert_eq!(accepted.signer, admin.pk.to_base64());
}

#[test]
fn refuses_roster_signed_by_stranger() {
    let (root, admin, stranger) = (keys(), keys(), keys());
    let err = verify_roster(
        &sign(&stranger, &roster_json().to_string()),
        &root.pk.to_base64(),
        &[admin.pk.to_base64()],
        ORG,
    )
    .unwrap_err();
    assert_eq!(err, RosterError::Signature);
}

#[test]
fn refuses_tampered_text() {
    let root = keys();
    let text = roster_json().to_string();
    let signed = sign(&root, &text);
    let envelope: Value = serde_json::from_str(&signed).unwrap();
    let tampered = text.replace("\"seq\":7", "\"seq\":9");
    assert_ne!(tampered, text);
    let forged = roster_envelope(&tampered, envelope["signature"].as_str().unwrap());
    let err = verify_roster(&forged, &root.pk.to_base64(), &[], ORG).unwrap_err();
    assert_eq!(err, RosterError::Signature);
}

#[test]
fn refuses_an_unreadable_envelope() {
    let root = keys();
    let err = verify_roster("nope", &root.pk.to_base64(), &[], ORG).unwrap_err();
    assert!(matches!(err, RosterError::Envelope(_)));
    let bad_sig = roster_envelope("{}", "not a signature");
    let err = verify_roster(&bad_sig, &root.pk.to_base64(), &[], ORG).unwrap_err();
    assert!(matches!(err, RosterError::Envelope(_)));
}

#[test]
fn wrong_org() {
    let root = keys();
    let err = verify_roster(
        &sign(&root, &roster_json().to_string()),
        &root.pk.to_base64(),
        &[],
        "other-org",
    )
    .unwrap_err();
    assert_eq!(
        err,
        RosterError::WrongOrg {
            expected: "other-org".into(),
            found: ORG.into()
        }
    );
}

#[test]
fn seq_must_grow() {
    let roster = parse_roster(&roster_json().to_string(), ORG).unwrap();
    assert!(roster.check_seq(None).is_ok());
    assert!(roster.check_seq(Some(6)).is_ok());
    assert_eq!(
        roster.check_seq(Some(7)).unwrap_err(),
        RosterError::RolledBack { last: 7, found: 7 }
    );
    let mut five = roster.clone();
    five.seq = 5;
    let err = five.check_seq(Some(7)).unwrap_err();
    assert_eq!(err, RosterError::RolledBack { last: 7, found: 5 });
    assert_eq!(
        err.to_string(),
        "this member list (#5) is older than the one already accepted (#7)"
    );
    let mut eight = roster;
    eight.seq = 8;
    assert!(eight.check_seq(Some(7)).is_ok());
}

#[test]
fn needs_a_default_workspace() {
    let mut r = roster_json();
    r["workspaces"] = json!([{"id": "payments", "name": "Payments"}]);
    r["members"][0]["workspaces"] = json!({});
    assert_eq!(invalid(&r), "the member list needs a 'default' workspace");
}

#[test]
fn workspace_ids_are_checked() {
    let mut r = roster_json();
    r["workspaces"][1]["id"] = json!("Bad_Id");
    assert_eq!(
        invalid(&r),
        "workspace id 'Bad_Id' must be lowercase letters, digits and dashes, up to 32"
    );
    for bad in ["-lead", "", &"a".repeat(33)] {
        r["workspaces"][1]["id"] = json!(bad);
        assert!(invalid(&r).starts_with("workspace id "), "{bad}");
    }
    for good in ["a", "0-x", &"a".repeat(32)] {
        r["workspaces"][1]["id"] = json!(good);
        assert!(parse_roster(&r.to_string(), ORG).is_ok(), "{good}");
    }
}

#[test]
fn a_member_cannot_name_a_missing_workspace() {
    let mut r = roster_json();
    r["members"][0]["workspaces"] = json!({"default": "member", "nope": "viewer"});
    assert_eq!(
        invalid(&r),
        "member asha@acme.com names workspace 'nope', which isn't on the list"
    );
}

#[test]
fn a_key_belongs_to_one_member() {
    let mut r = roster_json();
    r["members"].as_array_mut().unwrap().push(member(
        "01B",
        "ben@acme.com",
        "member",
        "RWQowner0000000000",
    ));
    assert_eq!(invalid(&r), "key RWQowner… is listed for two members");
}

#[test]
fn needs_an_owner() {
    let mut r = roster_json();
    r["members"][0]["role"] = json!("admin");
    assert_eq!(invalid(&r), "the member list needs at least one Owner");
}

#[test]
fn unknown_fields_fail_closed_at_every_level() {
    let mut r = roster_json();
    r["x"] = json!(1);
    assert_eq!(invalid(&r), "unknown field 'x'");

    let mut r = roster_json();
    r["members"][0]["x"] = json!(1);
    assert_eq!(invalid(&r), "unknown field 'x'");

    let mut r = roster_json();
    r["workspaces"][0]["x"] = json!(1);
    assert_eq!(invalid(&r), "unknown field 'x'");
}

#[test]
fn an_unknown_role_or_version_is_refused() {
    let mut r = roster_json();
    r["members"][0]["role"] = json!("boss");
    assert!(invalid(&r).contains("boss"));
    let mut r = roster_json();
    r["v"] = json!(2);
    assert!(invalid(&r).contains("version 2 is not supported"));
}

#[test]
fn member_with_key_finds_member() {
    let mut r = roster_json();
    r["members"].as_array_mut().unwrap().push(member(
        "01B",
        "ben@acme.com",
        "member",
        "RWQben000000000000",
    ));
    let roster = parse_roster(&r.to_string(), ORG).unwrap();
    assert_eq!(
        roster
            .member_with_key("RWQben000000000000")
            .map(|m| m.email.as_str()),
        Some("ben@acme.com")
    );
    assert!(roster.member_with_key("RWQnobody").is_none());
}
