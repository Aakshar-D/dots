//! Edits to the member list (members spec §6.2), shared by Dots and `dots-admin`: add a person
//! or a PC, change a role, remove a person or a PC. Pure: a roster in, a roster out.
//!
//! Dots never changes an Owner (only the owner's key may sign that, and only `dots-admin` holds
//! it), so [`apply`] refuses any change that touches one. `dots-admin` uses
//! [`edit_members`] with `allow_owner_changes`, and bumps `seq` only when it signs.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::roster::{parse_roster, Member, Role, Roster, WorkspaceRole, ROSTER_VERSION};
use crate::vault::org::key_fingerprint;
use crate::Error;

/// Why Dots refuses an edit that touches an Owner.
pub const OWNER_LOCKED: &str =
    "Owners can only be changed with the account owner's key, using dots-admin.";

/// What a person asks for on the Members tab, as it crosses to the service. `Approve` carries
/// the request's text; the service turns it into [`Change::AddKey`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RosterEdit {
    Approve {
        request: String,
        role: Option<Role>,
        confirm_no_invite: bool,
    },
    SetRole {
        member_id: String,
        role: Role,
    },
    Remove {
        member_id: String,
    },
    RemoveKey {
        member_id: String,
        key: String,
    },
}

/// One change to the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A new PC. If `email` is already listed it joins that person (their role and workspaces
    /// stay); otherwise a new member is added with `role` and `workspaces`.
    AddKey {
        name: String,
        email: String,
        key: String,
        role: Role,
        workspaces: BTreeMap<String, WorkspaceRole>,
    },
    SetRole {
        member_id: String,
        role: Role,
    },
    Remove {
        member_id: String,
    },
    /// One PC of a person. Their last key removes them.
    RemoveKey {
        member_id: String,
        key: String,
    },
}

/// A member id from their first key, which no other member shares: the first 12 hex digits of
/// the key's fingerprint, with a number added if the list already has it.
pub fn new_member_id(roster: &Roster, key: &str) -> String {
    let hex: String = key_fingerprint(key)
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(12)
        .collect::<String>()
        .to_lowercase();
    let base = format!("m-{hex}");
    let mut id = base.clone();
    let mut n = 1;
    while roster.members.iter().any(|m| m.id == id) {
        n += 1;
        id = format!("{base}-{n}");
    }
    id
}

/// The role a person gets in the `default` workspace unless told otherwise: their account
/// role, with Owner counting as Admin.
pub fn default_workspace_role(role: Role) -> WorkspaceRole {
    match role {
        Role::Owner | Role::Admin => WorkspaceRole::Admin,
        Role::Member => WorkspaceRole::Member,
        Role::Viewer => WorkspaceRole::Viewer,
    }
}

/// Applies `change` as Dots does: Owners are never touched. Returns the new list, with `seq`
/// one higher and `issued_at` set to `now`, and a one-line summary. The list is serialized and
/// parsed again, so every §6.2 rule holds.
pub fn apply(
    roster: &Roster,
    change: &Change,
    now: DateTime<Utc>,
) -> crate::Result<(Roster, String)> {
    let (mut edited, summary) = edit_members(roster, change, now, false)?;
    edited.seq = roster.seq + 1;
    edited.issued_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    Ok((reparsed(&edited)?, summary))
}

/// Applies `change` to the members only: `seq` and `issued_at` are left alone (only signing
/// bumps them in `dots-admin`). With `allow_owner_changes` false an Owner is never touched.
pub fn edit_members(
    roster: &Roster,
    change: &Change,
    now: DateTime<Utc>,
    allow_owner_changes: bool,
) -> crate::Result<(Roster, String)> {
    let mut next = roster.clone();
    let locked = |owner: bool| {
        if owner && !allow_owner_changes {
            Err(Error::NotAllowed(OWNER_LOCKED.into()))
        } else {
            Ok(())
        }
    };
    let summary = match change {
        Change::AddKey {
            name,
            email,
            key,
            role,
            workspaces,
        } => {
            if roster.member_with_key(key).is_some() {
                return Err(Error::Conflict(
                    "This Dots ID is already on the member list.".into(),
                ));
            }
            if let Some(member) = next.members.iter_mut().find(|m| &m.email == email) {
                locked(member.role == Role::Owner)?;
                member.keys.push(key.clone());
                format!("Add a PC for {}", member.name)
            } else {
                locked(*role == Role::Owner)?;
                let member = Member {
                    id: new_member_id(roster, key),
                    name: name.clone(),
                    email: email.clone(),
                    role: *role,
                    keys: vec![key.clone()],
                    workspaces: workspaces.clone(),
                    added_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
                };
                next.members.push(member);
                format!("Add {email} as {}", role.label())
            }
        }
        Change::SetRole { member_id, role } => {
            let member = find(&mut next, member_id)?;
            locked(member.role == Role::Owner || *role == Role::Owner)?;
            member.role = *role;
            format!("Change {} to {}", member.name, role.label())
        }
        Change::Remove { member_id } => {
            let member = find(&mut next, member_id)?.clone();
            locked(member.role == Role::Owner)?;
            next.members.retain(|m| m.id != *member_id);
            format!("Remove {}", member.name)
        }
        Change::RemoveKey { member_id, key } => {
            let member = find(&mut next, member_id)?;
            locked(member.role == Role::Owner)?;
            let Some(at) = member.keys.iter().position(|k| k == key) else {
                return Err(Error::NotFound(format!(
                    "{} does not list that Dots ID.",
                    member.name
                )));
            };
            member.keys.remove(at);
            let name = member.name.clone();
            if member.keys.is_empty() {
                next.members.retain(|m| m.id != *member_id);
                format!("Remove {name}")
            } else {
                format!("Remove one PC of {name}")
            }
        }
    };
    Ok((next, summary))
}

fn find<'a>(roster: &'a mut Roster, id: &str) -> crate::Result<&'a mut Member> {
    roster
        .members
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| Error::NotFound(format!("No member '{id}' on the list.")))
}

/// The roster written as a roster file and read back by `parse_roster`.
fn reparsed(roster: &Roster) -> crate::Result<Roster> {
    #[derive(Serialize)]
    struct File<'a> {
        v: u32,
        #[serde(flatten)]
        roster: &'a Roster,
    }
    let text = serde_json::to_string(&File {
        v: ROSTER_VERSION,
        roster,
    })?;
    parse_roster(&text, &roster.org_id).map_err(|e| Error::Invalid(e.to_string()))
}
