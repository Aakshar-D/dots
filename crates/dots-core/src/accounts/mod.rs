//! Accounts: the Dots ID of this install, and (later) the members and workspaces built on it
//! (accounts design §5).

mod identity;
mod membership;
pub mod roster;
mod scope;

pub use identity::{
    ensure_identity, identity, set_profile, Identity, IDENTITY_SECRET, MAX_EMAIL, MAX_PROFILE_NAME,
};
pub use membership::{Membership, NotMember};
pub use roster::{
    open_accepted_roster, parse_roster, roster_envelope, roster_hash, verify_roster, Accepted,
    Member, Role, Roster, RosterError, Workspace, WorkspaceRole, ROSTER_ENVELOPE_VERSION,
    ROSTER_VERSION,
};
pub use scope::{current_scope, set_current_scope, CurrentScope, DEFAULT_WORKSPACE, PERSONAL};
