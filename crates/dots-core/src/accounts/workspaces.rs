//! Workspaces (accounts design §6.3): their ids, made from a name and never changed after, and
//! which of an account's this PC may see.

use serde::Serialize;

use super::membership::Membership;
use super::permissions::{access, Permission};
use super::roster::{Role, WorkspaceRole};
use super::{DEFAULT_WORKSPACE, PERSONAL};
use crate::store::Store;
use crate::vault::org::{Orgs, UseRules};
use crate::Result;

/// The longest a workspace id may be (`^[a-z0-9][a-z0-9-]{0,31}$`).
const MAX_ID: usize = 32;

/// The id for a new workspace called `name`: its lower-case letters and digits, words joined by
/// `-` (`Payments Team` is `payments-team`), `workspace` when the name has none. A taken id
/// (`taken`, which includes `default`) gets `-2`, `-3`, ... after it. Never over 32 characters:
/// the name is cut short to make room.
pub fn workspace_id_for(name: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = if base.is_empty() { "workspace" } else { &base };
    let is_taken = |id: &str| id == DEFAULT_WORKSPACE || taken.iter().any(|t| t == id);
    let fit = |room: usize| base[..base.len().min(room)].trim_end_matches('-');
    if !is_taken(fit(MAX_ID)) {
        return fit(MAX_ID).to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            format!("{}{suffix}", fit(MAX_ID - suffix.len()))
        })
        .find(|id| !is_taken(id))
        .expect("an unused number exists")
}

/// A workspace as this PC sees it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WorkspaceView {
    /// `personal`, or the id of the org the workspace belongs to.
    pub scope: String,
    pub id: String,
    pub name: String,
    pub use_rules: Option<UseRules>,
    /// This PC's role in the workspace: its entry on the member list (an Owner or Admin of the
    /// account is `Admin` in every workspace). `None` in Personal and in an account with no
    /// member list.
    pub my_role: Option<WorkspaceRole>,
    /// Whether this PC may rename, add or delete workspaces here.
    pub can_manage: bool,
}

/// The workspaces of `scope` this PC may see (§6.3). Personal and an account with no member
/// list: all of them. An active member: all as an Owner or Admin, otherwise only those the
/// member list puts it in. A PC waiting to be added, removed, or in an account not joined: none.
pub async fn visible_workspaces(
    store: &Store,
    orgs: &Orgs,
    scope: &str,
) -> Result<Vec<WorkspaceView>> {
    let org = (scope != PERSONAL).then_some(scope);
    let (membership, role) = match org {
        None => (Membership::Legacy, Some(Role::Owner)),
        Some(id) => match orgs.access(id) {
            Some(place) => place,
            None => return Ok(Vec::new()),
        },
    };
    let mine = org
        .and_then(|id| orgs.my_workspaces(id))
        .unwrap_or_default();
    let sees_all = match membership {
        Membership::Waiting | Membership::Removed => return Ok(Vec::new()),
        Membership::Legacy => true,
        Membership::Active => matches!(role, Some(Role::Owner | Role::Admin)),
    };
    let can_manage = access(Some(orgs), org).allows(Permission::ManageWorkspaces);
    let my_role = |id: &str| match (membership, role) {
        (Membership::Active, Some(Role::Owner | Role::Admin)) => Some(WorkspaceRole::Admin),
        (Membership::Active, _) => mine.get(id).copied(),
        _ => None,
    };
    Ok(store
        .list_workspaces(scope)
        .await?
        .into_iter()
        .filter(|w| sees_all || mine.contains_key(&w.id))
        .map(|w| WorkspaceView {
            my_role: my_role(&w.id),
            scope: w.scope,
            name: w.name,
            use_rules: w.use_rules,
            can_manage,
            id: w.id,
        })
        .collect())
}
