//! What each account role may do (members spec §6.3), checked here in core and never only in
//! the UI. Personal is always Owner; an account with no member list (`legacy`) acts as Member;
//! a PC that waits to be added, or was removed, may do nothing in that account (§6.4).

use super::membership::{Membership, NotMember};
use super::roster::Role;
use crate::vault::org::Orgs;
use crate::{Error, Result};

/// One row of the §6.3 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    View,
    EditDots,
    RunDots,
    AnswerApprovals,
    GrantEntries,
    AddMcp,
    CreateApiKeys,
    ManageMembers,
    ManageWorkspaces,
    SignRoster,
}

impl Permission {
    /// What the permission lets a person do, as a refusal names it.
    pub fn action(&self) -> &'static str {
        match self {
            Self::View => "see this account's dots",
            Self::EditDots => "create or change dots",
            Self::RunDots => "run dots",
            Self::AnswerApprovals => "answer approvals",
            Self::GrantEntries => "give secrets to dots",
            Self::AddMcp => "add MCP servers",
            Self::CreateApiKeys => "create API keys",
            Self::ManageMembers => "manage members",
            Self::ManageWorkspaces => "manage workspaces",
            Self::SignRoster => "sign the member list",
        }
    }
}

/// Whether `role` has `p` (§6.3). `SignRoster` is true for an Admin: whether the Admin's key is
/// in the policy's `admins` is checked where a list is accepted.
pub fn can(role: Role, p: Permission) -> bool {
    match p {
        Permission::View => true,
        Permission::EditDots
        | Permission::RunDots
        | Permission::AnswerApprovals
        | Permission::GrantEntries
        | Permission::AddMcp
        | Permission::CreateApiKeys => role != Role::Viewer,
        Permission::ManageMembers | Permission::ManageWorkspaces | Permission::SignRoster => {
            matches!(role, Role::Owner | Role::Admin)
        }
    }
}

/// This PC's standing in a scope: Personal, an account it holds a role in, an account with no
/// member list, or one whose member list it is not on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Owner,
    Role(Role),
    Legacy,
    Waiting,
    Removed,
}

impl Access {
    pub fn allows(&self, p: Permission) -> bool {
        match self {
            Self::Owner => true,
            Self::Role(role) => can(*role, p),
            Self::Legacy => can(Role::Member, p),
            Self::Waiting | Self::Removed => false,
        }
    }

    /// The role a refusal names: Legacy acts as Member.
    fn role(&self) -> Option<Role> {
        match self {
            Self::Owner => Some(Role::Owner),
            Self::Role(role) => Some(*role),
            Self::Legacy => Some(Role::Member),
            Self::Waiting | Self::Removed => None,
        }
    }
}

/// This PC's access to `org_id` (`None` is Personal). An org that is not joined is left to the
/// checks that refuse a dot of a removed org (env vault spec §6.4), so it acts as Legacy here:
/// a dot of it can still be deleted. An active member without a role fails closed, as waiting.
pub fn access(orgs: Option<&Orgs>, org_id: Option<&str>) -> Access {
    let Some(org_id) = org_id else {
        return Access::Owner;
    };
    match orgs.and_then(|orgs| orgs.access(org_id)) {
        None | Some((Membership::Legacy, _)) => Access::Legacy,
        Some((Membership::Active, Some(role))) => Access::Role(role),
        Some((Membership::Active, None)) | Some((Membership::Waiting, _)) => Access::Waiting,
        Some((Membership::Removed, _)) => Access::Removed,
    }
}

/// `Ok` when this PC may do `p` in `org_id`, otherwise `Error::NotAllowed` with the account's
/// name (the org's name) and the role, or why this PC is not a member.
pub async fn require(orgs: Option<&Orgs>, org_id: Option<&str>, p: Permission) -> Result<()> {
    let access = access(orgs, org_id);
    if access.allows(p) {
        return Ok(());
    }
    // Only an account refuses: Personal is Owner.
    let org_id = org_id.unwrap_or_default();
    let account = match orgs {
        Some(orgs) => orgs.summary(org_id).await.map(|s| s.name),
        None => None,
    }
    .unwrap_or_else(|| org_id.to_string());
    let text = match access.role() {
        Some(role) => format!(
            "{account}: a {} can't {}. Ask an Admin or Owner.",
            role.label(),
            p.action()
        ),
        // The org's own reason names a changed Dots ID too.
        None => orgs
            .and_then(|orgs| orgs.not_member(org_id))
            .unwrap_or(match access {
                Access::Removed => NotMember::Removed,
                _ => NotMember::Waiting,
            })
            .text(&account),
    };
    Err(Error::NotAllowed(text))
}
