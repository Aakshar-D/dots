//! Edits to the member list (members spec §6.2): each keeps every rule `parse_roster` checks.

use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use dots_core::accounts::edit::{apply, Change};
use dots_core::accounts::{parse_roster, Role, Roster, WorkspaceRole};
use dots_core::Error;
use minisign::KeyPair;
use serde_json::json;

const ORG: &str = "acme-eng";
const OWNER_TEXT: &str =
    "Owners can only be changed with the account owner's key, using dots-admin.";

fn dots_id() -> String {
    KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk
        .to_base64()
}

fn member(id: &str, name: &str, email: &str, role: &str, keys: &[&str]) -> serde_json::Value {
    json!({
        "id": id, "name": name, "email": email, "role": role, "keys": keys,
        "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

/// A list with an Owner (`boss@acme.com`) and a Member (`asha@acme.com`, Asha Rao), at seq 7.
fn roster(owner_key: &str, asha_key: &str) -> Roster {
    let text = json!({
        "v": 1, "org_id": ORG, "issued_at": "2026-10-28T09:00:00Z", "seq": 7,
        "workspaces": [{"id": "default", "name": "Default"}],
        "members": [
            member("m-owner", "Boss", "boss@acme.com", "owner", &[owner_key]),
            member("m-asha", "Asha Rao", "asha@acme.com", "member", &[asha_key]),
        ]
    })
    .to_string();
    parse_roster(&text, ORG).unwrap()
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 12, 30, 5).unwrap()
}

fn add(email: &str, key: &str, role: Role) -> Change {
    Change::AddKey {
        name: "New Person".into(),
        email: email.into(),
        key: key.into(),
        role,
        workspaces: BTreeMap::from([("default".to_string(), WorkspaceRole::Member)]),
    }
}

fn not_allowed(result: dots_core::Result<(Roster, String)>) {
    match result {
        Err(Error::NotAllowed(text)) => assert_eq!(text, OWNER_TEXT),
        other => panic!("expected NotAllowed, got {other:?}"),
    }
}

#[test]
fn add_new_member_bumps_seq() {
    let (owner, asha, ben) = (dots_id(), dots_id(), dots_id());
    let before = roster(&owner, &asha);
    let (after, _) = apply(&before, &add("ben@acme.com", &ben, Role::Viewer), now()).unwrap();
    assert_eq!(after.seq, 8);
    assert_eq!(after.issued_at, "2026-10-29T12:30:05Z");
    assert_eq!(after.members.len(), 3);
    let new = after.member_with_key(&ben).unwrap();
    assert_eq!(new.email, "ben@acme.com");
    assert_eq!(new.role, Role::Viewer);
    assert_eq!(new.added_at, "2026-10-29T12:30:05Z");
    assert_eq!(new.workspaces["default"], WorkspaceRole::Member);
    assert!(new.id.starts_with("m-"));
    assert_eq!(before.seq, 7);
}

#[test]
fn add_key_to_existing_email_keeps_role() {
    let (owner, asha, second) = (dots_id(), dots_id(), dots_id());
    let before = roster(&owner, &asha);
    let (after, _) = apply(&before, &add("asha@acme.com", &second, Role::Viewer), now()).unwrap();
    assert_eq!(after.members.len(), 2);
    let m = after.member_with_key(&second).unwrap();
    assert_eq!(m.id, "m-asha");
    assert_eq!(m.role, Role::Member);
    assert_eq!(m.keys, [asha, second]);
    assert_eq!(m.workspaces["default"], WorkspaceRole::Member);
}

#[test]
fn key_already_listed_is_refused() {
    let (owner, asha) = (dots_id(), dots_id());
    let before = roster(&owner, &asha);
    match apply(&before, &add("other@acme.com", &asha, Role::Member), now()) {
        Err(Error::Conflict(text)) => {
            assert_eq!(text, "This Dots ID is already on the member list.")
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
}

#[test]
fn owner_entries_are_locked() {
    let (owner, asha, extra) = (dots_id(), dots_id(), dots_id());
    let before = roster(&owner, &asha);
    not_allowed(apply(
        &before,
        &Change::SetRole {
            member_id: "m-asha".into(),
            role: Role::Owner,
        },
        now(),
    ));
    not_allowed(apply(
        &before,
        &Change::SetRole {
            member_id: "m-owner".into(),
            role: Role::Admin,
        },
        now(),
    ));
    not_allowed(apply(
        &before,
        &Change::Remove {
            member_id: "m-owner".into(),
        },
        now(),
    ));
    not_allowed(apply(
        &before,
        &add("boss@acme.com", &extra, Role::Member),
        now(),
    ));
    not_allowed(apply(
        &before,
        &add("new@acme.com", &extra, Role::Owner),
        now(),
    ));
    not_allowed(apply(
        &before,
        &Change::RemoveKey {
            member_id: "m-owner".into(),
            key: owner,
        },
        now(),
    ));
}

#[test]
fn remove_last_key_removes_member() {
    let (owner, asha) = (dots_id(), dots_id());
    let before = roster(&owner, &asha);
    let (after, _) = apply(
        &before,
        &Change::RemoveKey {
            member_id: "m-asha".into(),
            key: asha,
        },
        now(),
    )
    .unwrap();
    assert_eq!(after.members.len(), 1);
    assert_eq!(after.members[0].id, "m-owner");
}

#[test]
fn unknown_member_is_not_found() {
    let (owner, asha) = (dots_id(), dots_id());
    let before = roster(&owner, &asha);
    for change in [
        Change::SetRole {
            member_id: "m-zed".into(),
            role: Role::Admin,
        },
        Change::Remove {
            member_id: "m-zed".into(),
        },
        Change::RemoveKey {
            member_id: "m-zed".into(),
            key: asha,
        },
    ] {
        match apply(&before, &change, now()) {
            Err(Error::NotFound(text)) => assert_eq!(text, "No member 'm-zed' on the list."),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }
}

#[test]
fn summaries_read_plainly() {
    let (owner, asha, ben, second) = (dots_id(), dots_id(), dots_id(), dots_id());
    let before = roster(&owner, &asha);
    let summary = |roster: &Roster, change: Change| apply(roster, &change, now()).unwrap().1;
    assert_eq!(
        summary(&before, add("ben@acme.com", &ben, Role::Member)),
        "Add ben@acme.com as Member"
    );
    assert_eq!(
        summary(&before, add("asha@acme.com", &second, Role::Member)),
        "Add a PC for Asha Rao"
    );
    assert_eq!(
        summary(
            &before,
            Change::SetRole {
                member_id: "m-asha".into(),
                role: Role::Viewer
            }
        ),
        "Change Asha Rao to Viewer"
    );
    assert_eq!(
        summary(
            &before,
            Change::Remove {
                member_id: "m-asha".into()
            }
        ),
        "Remove Asha Rao"
    );
    let (two_pcs, _) = apply(&before, &add("asha@acme.com", &second, Role::Member), now()).unwrap();
    assert_eq!(
        summary(
            &two_pcs,
            Change::RemoveKey {
                member_id: "m-asha".into(),
                key: second
            }
        ),
        "Remove one PC of Asha Rao"
    );
}
