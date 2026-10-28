//! An org joined on this PC, as the `orgs` table keeps it (env vault spec §5.2): the settings of
//! its join code and what the last good fetch accepted. It holds no secrets.

use std::collections::BTreeMap;

use super::join::SourceSettings;
use crate::accounts::{Membership, Role, WorkspaceRole};

/// One row of `orgs`. `store/orgs.rs` reads and writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgRow {
    /// The join code's `org_id`; `join::check_org_id` holds for it.
    pub id: String,
    /// The org's name as the person sees it, "Acme Engineering".
    pub name: String,
    pub source: SourceSettings,
    /// The minisign public key the org's policy is signed with, the bare `RW…` base64 line.
    pub policy_key: String,
    /// The `issued_at` of the last policy accepted, as its text. A policy issued before it is
    /// refused (§6.4). `None` before the first.
    pub last_issued_at: Option<String>,
    /// When the last good fetch was, RFC 3339 UTC. `None` before the first.
    pub last_fetch_at: Option<String>,
    /// When the org was joined, RFC 3339 UTC.
    pub joined_at: String,
    /// This PC's place on the account's member list.
    pub member: MemberRow,
}

/// This PC's place on an account's signed member list, as the `orgs` row keeps it (members
/// spec §6.4). All but `membership` are empty until a list names this PC; `last_roster_seq`
/// stays once a list was accepted, for the rollback check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberRow {
    pub membership: Membership,
    /// This PC's member on the last accepted list, while `active`.
    pub member_id: Option<String>,
    pub role: Option<Role>,
    /// The member's workspaces and roles there, while `active`.
    pub workspaces: BTreeMap<String, WorkspaceRole>,
    /// The `seq` of the last list accepted: an older one is refused.
    pub last_roster_seq: Option<u64>,
    /// The Dots ID this PC was `active` under. It stays while `waiting` because that ID
    /// changed, so the account can say so.
    pub key: Option<String>,
    /// The `roster_hash` of the last list accepted. The same list fetched again is known by it,
    /// and is not checked against the current admins again (§6.1).
    pub roster_hash: Option<String>,
}

impl MemberRow {
    /// Waiting because this PC's Dots ID changed since it was on the list.
    pub fn id_changed(&self) -> bool {
        self.membership == Membership::Waiting && self.key.is_some()
    }
}
