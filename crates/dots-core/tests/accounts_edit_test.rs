//! Edits to the member list (members spec §6.2): each keeps every rule `parse_roster` checks.

use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use dots_core::accounts::edit::{apply, edit_members, Change};
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

#[test]
fn edit_members_with_owner_changes_allowed_touches_neither_seq_nor_issued_at() {
    let (owner, asha) = (dots_id(), dots_id());
    let before = roster(&owner, &asha);
    let (after, _) = edit_members(
        &before,
        &Change::SetRole {
            member_id: "m-owner".into(),
            role: Role::Owner,
        },
        now(),
        true,
    )
    .unwrap();
    // An Owner's own entry may be changed when the root key signs.
    let (after, _) = edit_members(
        &after,
        &Change::SetRole {
            member_id: "m-owner".into(),
            role: Role::Admin,
        },
        now(),
        true,
    )
    .unwrap();
    assert_eq!(after.member_with_key(&owner).unwrap().role, Role::Admin);
    assert_eq!(after.seq, before.seq);
    assert_eq!(after.issued_at, before.issued_at);
}

// ---- workspaces ---------------------------------------------------------------------------

fn roles(pairs: &[(&str, WorkspaceRole)]) -> BTreeMap<String, WorkspaceRole> {
    pairs.iter().map(|(id, r)| (id.to_string(), *r)).collect()
}

fn with_payments(owner: &str, asha: &str) -> Roster {
    let (roster, _) = apply(
        &roster(owner, asha),
        &Change::AddWorkspace {
            name: "Payments Team".into(),
        },
        now(),
    )
    .unwrap();
    roster
}

#[test]
fn add_rename_delete_workspace() {
    let (owner, asha) = (dots_id(), dots_id());
    let before = roster(&owner, &asha);
    let (added, summary) = apply(
        &before,
        &Change::AddWorkspace {
            name: "Payments Team".into(),
        },
        now(),
    )
    .unwrap();
    assert_eq!(summary, "Add the Payments Team workspace");
    assert_eq!(added.seq, 8);
    let ids: Vec<_> = added.workspaces.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["default", "payments-team"]);
    assert_eq!(added.workspaces[1].name, "Payments Team");

    let (renamed, summary) = apply(
        &added,
        &Change::RenameWorkspace {
            id: "payments-team".into(),
            name: "Payments".into(),
        },
        now(),
    )
    .unwrap();
    assert_eq!(summary, "Rename Payments Team to Payments");
    assert_eq!(renamed.workspaces[1].id, "payments-team");
    assert_eq!(renamed.workspaces[1].name, "Payments");

    let (deleted, summary) = apply(
        &renamed,
        &Change::DeleteWorkspace {
            id: "payments-team".into(),
        },
        now(),
    )
    .unwrap();
    assert_eq!(summary, "Delete the Payments workspace");
    assert_eq!(deleted.workspaces.len(), 1);

    let missing = Change::RenameWorkspace {
        id: "nope".into(),
        name: "X".into(),
    };
    assert!(matches!(
        apply(&before, &missing, now()),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn delete_default_is_refused() {
    let (owner, asha) = (dots_id(), dots_id());
    let result = apply(
        &roster(&owner, &asha),
        &Change::DeleteWorkspace {
            id: "default".into(),
        },
        now(),
    );
    assert!(matches!(result, Err(Error::Invalid(_))), "{result:?}");
}

#[test]
fn delete_removes_member_entries() {
    let (owner, asha) = (dots_id(), dots_id());
    let list = with_payments(&owner, &asha);
    let (list, _) = edit_members(
        &list,
        &Change::SetWorkspaces {
            member_id: "m-asha".into(),
            workspaces: roles(&[
                ("default", WorkspaceRole::Member),
                ("payments-team", WorkspaceRole::Admin),
            ]),
        },
        now(),
        false,
    )
    .unwrap();
    assert_eq!(list.members[1].workspaces.len(), 2);
    let (after, _) = apply(
        &list,
        &Change::DeleteWorkspace {
            id: "payments-team".into(),
        },
        now(),
    )
    .unwrap();
    assert_eq!(
        after.members[1].workspaces,
        roles(&[("default", WorkspaceRole::Member)])
    );
}

#[test]
fn set_workspaces_needs_default() {
    let (owner, asha) = (dots_id(), dots_id());
    let list = with_payments(&owner, &asha);
    let result = apply(
        &list,
        &Change::SetWorkspaces {
            member_id: "m-asha".into(),
            workspaces: roles(&[("payments-team", WorkspaceRole::Member)]),
        },
        now(),
    );
    assert!(matches!(result, Err(Error::Invalid(_))), "{result:?}");
    let unlisted = apply(
        &list,
        &Change::SetWorkspaces {
            member_id: "m-asha".into(),
            workspaces: roles(&[
                ("default", WorkspaceRole::Member),
                ("ghost", WorkspaceRole::Member),
            ]),
        },
        now(),
    );
    assert!(unlisted.is_err());
    let (ok, summary) = apply(
        &list,
        &Change::SetWorkspaces {
            member_id: "m-asha".into(),
            workspaces: roles(&[
                ("default", WorkspaceRole::Viewer),
                ("payments-team", WorkspaceRole::Member),
            ]),
        },
        now(),
    )
    .unwrap();
    assert_eq!(summary, "Change Asha Rao's workspaces");
    assert_eq!(ok.members[1].workspaces.len(), 2);
}

#[test]
fn owner_workspaces_locked() {
    let (owner, asha) = (dots_id(), dots_id());
    let list = roster(&owner, &asha);
    let change = Change::SetWorkspaces {
        member_id: "m-owner".into(),
        workspaces: roles(&[("default", WorkspaceRole::Viewer)]),
    };
    not_allowed(apply(&list, &change, now()));
    let (edited, _) = edit_members(&list, &change, now(), true).unwrap();
    assert_eq!(
        edited.members[0].workspaces["default"],
        WorkspaceRole::Viewer
    );
}
