//! What each account role may do (members spec §6.3), checked here in core and never only in
//! the UI. Personal is always Owner; an account with no member list (`legacy`) acts as Member;
//! a PC that waits to be added, or was removed, may do nothing in that account (§6.4). On a dot
//! or team, the member's role in its workspace narrows the account role (§6.3).

use super::membership::{Membership, NotMember};
use super::roster::{Role, WorkspaceRole};
use crate::model::{Dot, Team};
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
/// member list, or one whose member list it is not on. On a dot or team, also a workspace of
/// the account the member list does not put it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Owner,
    Role(Role),
    Legacy,
    Waiting,
    Removed,
    /// An account Member or Viewer with no entry for the workspace (named here): it may only
    /// clean up the workspace's dots, like a PC waiting or removed (§6.3 "does not see it").
    OutsideWorkspace {
        workspace: String,
    },
}

impl Access {
    pub fn allows(&self, p: Permission) -> bool {
        match self {
            Self::Owner => true,
            Self::Role(role) => can(*role, p),
            Self::Legacy => can(Role::Member, p),
            Self::Waiting | Self::Removed | Self::OutsideWorkspace { .. } => false,
        }
    }

    /// The role a refusal names: Legacy acts as Member.
    fn role(&self) -> Option<Role> {
        match self {
            Self::Owner => Some(Role::Owner),
            Self::Role(role) => Some(*role),
            Self::Legacy => Some(Role::Member),
            Self::Waiting | Self::Removed | Self::OutsideWorkspace { .. } => None,
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
    Err(Error::NotAllowed(
        refusal(orgs, org_id, None, &access, p).await,
    ))
}

/// This PC's access to the workspace `workspace_id` of `org_id` (members spec §6.3): an Owner or
/// Admin of the account has its role in every workspace; a Member has Member rights where its
/// entry is `admin` or `member` and Viewer rights where it is `viewer`; a Viewer has Viewer
/// rights where it has an entry. Without an entry, a Member or Viewer is outside the workspace.
/// Personal, an account with no member list, and a PC not on the list are not narrowed.
pub fn access_in(orgs: Option<&Orgs>, org_id: Option<&str>, workspace_id: &str) -> Access {
    let account = access(orgs, org_id);
    let (Access::Role(role @ (Role::Member | Role::Viewer)), Some(org_id)) = (&account, org_id)
    else {
        return account;
    };
    let entry = orgs
        .and_then(|orgs| orgs.my_workspaces(org_id))
        .and_then(|mine| mine.get(workspace_id).copied());
    match (role, entry) {
        (_, None) => Access::OutsideWorkspace {
            workspace: workspace_name(orgs, org_id, workspace_id),
        },
        (Role::Member, Some(WorkspaceRole::Admin | WorkspaceRole::Member)) => {
            Access::Role(Role::Member)
        }
        (_, Some(_)) => Access::Role(Role::Viewer),
    }
}

/// This PC's access to `dot`: `access_in` its account and workspace.
pub fn access_dot(orgs: Option<&Orgs>, dot: &Dot) -> Access {
    access_in(orgs, dot.org_id.as_deref(), &dot.workspace_id)
}

/// `require` in the workspace `workspace_id` of `org_id` (`access_in`). A role the workspace
/// narrowed is named with the workspace; a workspace this PC isn't in says so.
pub async fn require_in(
    orgs: Option<&Orgs>,
    org_id: Option<&str>,
    workspace_id: &str,
    p: Permission,
) -> Result<()> {
    let access = access_in(orgs, org_id, workspace_id);
    if access.allows(p) {
        return Ok(());
    }
    // The workspace narrowed the role only when the account role alone allows `p`.
    let narrowed = self::access(orgs, org_id).allows(p).then_some(workspace_id);
    Err(Error::NotAllowed(
        refusal(orgs, org_id, narrowed, &access, p).await,
    ))
}

/// `require` on `dot`, in its account and workspace.
pub async fn require_dot(orgs: Option<&Orgs>, dot: &Dot, p: Permission) -> Result<()> {
    require_in(orgs, dot.org_id.as_deref(), &dot.workspace_id, p).await
}

/// `require` on `team`, in its account and workspace.
pub async fn require_team(orgs: Option<&Orgs>, team: &Team, p: Permission) -> Result<()> {
    require_in(orgs, team.org_id.as_deref(), &team.workspace_id, p).await
}

/// The name the member list gives the workspace `workspace_id` of `org_id`, or its id when it
/// gives none.
pub fn workspace_name(orgs: Option<&Orgs>, org_id: &str, workspace_id: &str) -> String {
    orgs.and_then(|orgs| orgs.workspace_name(org_id, workspace_id))
        .unwrap_or_else(|| workspace_id.to_string())
}

/// The account's name for the texts: the org's name, or its id when it is not joined.
async fn account_name(orgs: Option<&Orgs>, org_id: &str) -> String {
    match orgs {
        Some(orgs) => orgs.summary(org_id).await.map(|s| s.name),
        None => None,
    }
    .unwrap_or_else(|| org_id.to_string())
}

/// Why `access` may not do `p` in `org_id`, in plain words naming the account: the role (with
/// the workspace that narrowed it, `narrowed`), the workspace this PC isn't in, or why it is
/// not a member.
async fn refusal(
    orgs: Option<&Orgs>,
    org_id: Option<&str>,
    narrowed: Option<&str>,
    access: &Access,
    p: Permission,
) -> String {
    // Only an account refuses: Personal is Owner.
    let org_id = org_id.unwrap_or_default();
    let account = account_name(orgs, org_id).await;
    if let Access::OutsideWorkspace { workspace } = access {
        return format!(
            "{account}: you aren't in the {workspace} workspace. Ask an Admin or Owner to add you."
        );
    }
    match (access.role(), narrowed) {
        (Some(role), Some(workspace_id)) => format!(
            "{account}: a {} in the {} workspace can't {}. Ask an Admin or Owner.",
            role.label(),
            workspace_name(orgs, org_id, workspace_id),
            p.action()
        ),
        (Some(role), None) => format!(
            "{account}: a {} can't {}. Ask an Admin or Owner.",
            role.label(),
            p.action()
        ),
        // The org's own reason names a changed Dots ID too.
        (None, _) => orgs
            .and_then(|orgs| orgs.not_member(org_id))
            .unwrap_or(match access {
                Access::Removed => NotMember::Removed,
                _ => NotMember::Waiting,
            })
            .text(&account),
    }
}
